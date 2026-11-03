// Windows handles live here; portable policy remains in the Rust application.
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Drawing;
using System.Drawing.Imaging;
using System.IO;
using System.IO.Pipes;
using System.Runtime.InteropServices;
using System.Security.AccessControl;
using System.Security.Principal;
using System.Text;
using System.Threading;
using System.Threading.Tasks;
using System.Web.Script.Serialization;
using System.Windows.Forms;

partial class Native {
    const int MaxBody = 12 * 1024 * 1024, MaxBytes = 8 * 1024 * 1024;
    static readonly UTF8Encoding Utf8 = new UTF8Encoding(false, true);
    static JavaScriptSerializer Json() { return new JavaScriptSerializer { MaxJsonLength = MaxBody, RecursionLimit = 64 }; }
    static Dictionary<string,object> Obj(string line) { return Json().Deserialize<Dictionary<string,object>>(line); }
    static string Encode(object value) { return Json().Serialize(value); }
    static object Get(Dictionary<string,object> d, string k) { object v; return d.TryGetValue(k, out v) ? v : null; }
    static string Text(Dictionary<string,object> d, string k) { return Convert.ToString(Get(d,k)); }
    static void Output(object o) { Console.WriteLine(Encode(o)); Console.Out.Flush(); }
    static SecurityIdentifier User { get { return WindowsIdentity.GetCurrent().User; } }
    static int Session { get { return Process.GetCurrentProcess().SessionId; } }
    [DllImport("kernel32.dll", SetLastError=true)] static extern bool GetNamedPipeClientSessionId(IntPtr h, out uint id);
    [DllImport("kernel32.dll", SetLastError=true)] static extern bool GetNamedPipeServerSessionId(IntPtr h, out uint id);
    [DllImport("user32.dll")] static extern bool OpenClipboard(IntPtr hwnd);
    [DllImport("user32.dll")] static extern bool CloseClipboard();
    [DllImport("user32.dll")] static extern bool EmptyClipboard();
    [DllImport("user32.dll")] static extern uint GetClipboardSequenceNumber();
    [DllImport("user32.dll")] static extern bool IsClipboardFormatAvailable(uint f);
    [DllImport("user32.dll")] static extern IntPtr GetClipboardData(uint f);
    [DllImport("user32.dll", SetLastError=true)] static extern IntPtr SetClipboardData(uint f, IntPtr p);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern uint RegisterClipboardFormat(string name);
    [DllImport("user32.dll")] static extern bool AddClipboardFormatListener(IntPtr hwnd);
    [DllImport("user32.dll")] static extern bool RemoveClipboardFormatListener(IntPtr hwnd);
    [DllImport("kernel32.dll")] static extern UIntPtr GlobalSize(IntPtr h);
    [DllImport("kernel32.dll")] static extern IntPtr GlobalLock(IntPtr h);
    [DllImport("kernel32.dll")] static extern bool GlobalUnlock(IntPtr h);
    [DllImport("kernel32.dll")] static extern IntPtr GlobalAlloc(uint flags, UIntPtr size);
    [DllImport("kernel32.dll")] static extern IntPtr GlobalFree(IntPtr h);

    static void SecureDir(string path) {
        var d = new DirectoryInfo(path);
        if ((d.Attributes & FileAttributes.ReparsePoint) != 0) throw new Exception("Profile cannot be a reparse point");
        var acl = d.GetAccessControl();
        if (!User.Equals(acl.GetOwner(typeof(SecurityIdentifier)))) throw new Exception("Profile owner differs from current user");
        acl.SetAccessRuleProtection(true, false);
        foreach (FileSystemAccessRule rule in acl.GetAccessRules(true, false, typeof(SecurityIdentifier))) acl.RemoveAccessRuleSpecific(rule);
        acl.AddAccessRule(new FileSystemAccessRule(User, FileSystemRights.FullControl, InheritanceFlags.ContainerInherit | InheritanceFlags.ObjectInherit, PropagationFlags.None, AccessControlType.Allow));
        d.SetAccessControl(acl);
    }
    static string PipeName(string profile) {
        using (var hash = System.Security.Cryptography.SHA256.Create())
            return "shuttli-" + Session + "-" + BitConverter.ToString(hash.ComputeHash(Utf8.GetBytes(User.Value + "|" + Path.GetFullPath(profile).TrimEnd('\\').ToUpperInvariant()))).Replace("-", "");
    }
    static void Exact(Stream s, byte[] b) {
        int offset = 0;
        while (offset < b.Length) {
            var task = s.ReadAsync(b, offset, b.Length-offset);
            if (!task.Wait(12000)) throw new IOException("Pipe read timed out");
            int n = task.Result; if(n == 0) throw new EndOfStreamException(); offset += n;
        }
    }
    static string Receive(Stream s, int limit) {
        var header = new byte[4]; Exact(s,header); int size = BitConverter.ToInt32(header,0);
        if(size < 1 || size > limit) throw new IOException("Pipe frame too large");
        var b = new byte[size]; Exact(s,b); return Utf8.GetString(b);
    }
    static void Send(Stream s, string value, int limit) {
        byte[] b = Utf8.GetBytes(value); if(b.Length > limit) throw new IOException("Pipe frame too large");
        var header = BitConverter.GetBytes(b.Length);
        if(!s.WriteAsync(header,0,4).Wait(12000) || !s.WriteAsync(b,0,b.Length).Wait(12000)) throw new IOException("Pipe write timed out");
        s.Flush();
    }
    static string Call(string profile, string request) {
        using(var pipe = new NamedPipeClientStream(".",PipeName(profile),PipeDirection.InOut,PipeOptions.Asynchronous,TokenImpersonationLevel.Identification)) {
            pipe.Connect(2500);
            uint session;
            if(!GetNamedPipeServerSessionId(pipe.SafePipeHandle.DangerousGetHandle(),out session) || session != Session) throw new IOException("Pipe server session mismatch");
            var owner = pipe.GetAccessControl().GetOwner(typeof(SecurityIdentifier));
            if(!User.Equals(owner)) throw new IOException("Pipe server owner mismatch");
            Send(pipe,request,256*1024); return Receive(pipe,MaxBody);
        }
    }
    static readonly object OutputLock = new object();
    static readonly Dictionary<long,TaskCompletionSource<string>> Pending = new Dictionary<long,TaskCompletionSource<string>>();
    static long nextId;
    static void Server(string profile) {
        var security = new PipeSecurity(); security.SetAccessRuleProtection(true,false);
        security.SetOwner(User);
        security.AddAccessRule(new PipeAccessRule(new SecurityIdentifier(WellKnownSidType.NetworkSid,null),PipeAccessRights.FullControl,AccessControlType.Deny));
        security.AddAccessRule(new PipeAccessRule(User,PipeAccessRights.FullControl,AccessControlType.Allow));
        for(int i=0;i<4;i++) new Thread(delegate() {
            while(true) {
                using(var pipe = new NamedPipeServerStream(PipeName(profile),PipeDirection.InOut,4,PipeTransmissionMode.Byte,PipeOptions.Asynchronous,4096,4096,security)) {
                    try {
                        pipe.WaitForConnection(); uint session;
                        if(!GetNamedPipeClientSessionId(pipe.SafePipeHandle.DangerousGetHandle(),out session) || session != Session) continue;
                        if(!new NTAccount(pipe.GetImpersonationUserName()).Translate(typeof(SecurityIdentifier)).Equals(User)) continue;
                        string request = Receive(pipe,256*1024); Obj(request);
                        long id = Interlocked.Increment(ref nextId); var reply = new TaskCompletionSource<string>();
                        lock(Pending) Pending.Add(id,reply);
                        try {
                            lock(OutputLock) Output(new {id=id,request=Obj(request)});
                            if(!reply.Task.Wait(12000)) throw new IOException("Agent response timed out");
                            Send(pipe,reply.Task.Result,MaxBody);
                        } finally { lock(Pending) Pending.Remove(id); }
                    } catch(Exception) { /* A malformed/slow client cannot stop other readers. */ }
                }
            }
        }) { IsBackground=true }.Start();
        string line;
        while((line=Console.ReadLine()) != null) {
            var v=Obj(line); long id=Convert.ToInt64(v["id"]); TaskCompletionSource<string> reply;
            lock(Pending) if(Pending.TryGetValue(id,out reply)) reply.TrySetResult(Encode(v["answer"]));
        }
    }

    static byte[] ReadData(uint format, int limit) {
        IntPtr h=GetClipboardData(format); if(h == IntPtr.Zero) throw new IOException("Clipboard data unavailable");
        ulong size=GlobalSize(h).ToUInt64(); if(size==0 || size>(ulong)limit) throw new IOException("Clipboard allocation exceeds limit");
        IntPtr p=GlobalLock(h); if(p==IntPtr.Zero) throw new IOException("Clipboard lock failed");
        try { var b=new byte[(int)size]; Marshal.Copy(p,b,0,b.Length); return b; } finally { GlobalUnlock(h); }
    }
    static void Put(uint format, byte[] b) {
        IntPtr h=GlobalAlloc(0x42,(UIntPtr)b.Length); if(h==IntPtr.Zero) throw new IOException("Clipboard allocation failed");
        try {
            IntPtr p=GlobalLock(h); if(p==IntPtr.Zero) throw new IOException("Clipboard memory lock failed");
            try { Marshal.Copy(b,0,p,b.Length); } finally { GlobalUnlock(h); }
            if(SetClipboardData(format,h)==IntPtr.Zero) throw new IOException("Clipboard write failed");
            h=IntPtr.Zero;
        } finally { if(h!=IntPtr.Zero) GlobalFree(h); }
    }
    static byte[] Dib(byte[] png) {
        using(var stream=new MemoryStream(png)) using(var image=new Bitmap(stream)) {
            int w=image.Width,h=image.Height; if((long)w*h>4*1024*1024 || w<1 || h<1) throw new IOException("Image dimensions exceed limit");
            byte[] b=new byte[124+w*h*4];
            using(var writer=new BinaryWriter(new MemoryStream(b))) {
                writer.Write(124);writer.Write(w);writer.Write(-h);writer.Write((short)1);writer.Write((short)32);writer.Write(3);writer.Write(w*h*4);
                writer.BaseStream.Position=40;writer.Write(0x00ff0000);writer.Write(0x0000ff00);writer.Write(0x000000ff);writer.Write(unchecked((int)0xff000000));writer.Write(0x73524742);
            }
            var data=image.LockBits(new Rectangle(0,0,w,h),ImageLockMode.ReadOnly,PixelFormat.Format32bppArgb);
            try { for(int y=0;y<h;y++) Marshal.Copy(IntPtr.Add(data.Scan0,y*data.Stride),b,124+y*w*4,w*4); } finally { image.UnlockBits(data); }
            return b;
        }
    }
    static byte[] PngFromDib(byte[] b) {
        if(b.Length<40) throw new IOException("Truncated DIB");
        int header=BitConverter.ToInt32(b,0),w=BitConverter.ToInt32(b,4),height=BitConverter.ToInt32(b,8);
        int bits=BitConverter.ToUInt16(b,14),compression=BitConverter.ToInt32(b,16);
        long h=Math.Abs((long)height);
        if(header<40 || header>b.Length || w<1 || w>16384 || h<1 || h>16384 || w*h>4*1024*1024 || BitConverter.ToUInt16(b,12)!=1 || (bits!=24 && bits!=32) || (compression!=0 && compression!=3)) throw new IOException("Unsupported DIB layout");
        if(header>=124 && (BitConverter.ToInt32(b,112)!=0 || BitConverter.ToInt32(b,116)!=0)) throw new IOException("DIB color profile unsupported");
        int offset=header; bool alpha=false;
        if(compression==3) {
            if(bits!=32 || b.Length<56) throw new IOException("Unsupported DIB masks");
            int m=header==40?40:40;
            if(BitConverter.ToUInt32(b,m)!=0x00ff0000 || BitConverter.ToUInt32(b,m+4)!=0x0000ff00 || BitConverter.ToUInt32(b,m+8)!=0x000000ff) throw new IOException("Unsupported DIB masks");
            if(header==40) offset+=12;
            else { uint a=BitConverter.ToUInt32(b,52); if(a!=0 && a!=0xff000000) throw new IOException("Unsupported alpha mask"); alpha=a!=0; }
        }
        long stride=((long)w*bits+31)/32*4;
        if(offset+stride*h>b.Length) throw new IOException("Truncated DIB pixels");
        using(var image=new Bitmap(w,(int)h,PixelFormat.Format32bppArgb)) {
            var pixels=image.LockBits(new Rectangle(0,0,w,(int)h),ImageLockMode.WriteOnly,PixelFormat.Format32bppArgb);
            try {
                var row=new byte[w*4];
                for(int y=0;y<h;y++) {
                    int src=(int)(offset+(height<0?y:h-1-y)*stride);
                    for(int x=0;x<w;x++) { int p=src+x*(bits/8);row[x*4]=b[p];row[x*4+1]=b[p+1];row[x*4+2]=b[p+2];row[x*4+3]=alpha?b[p+3]:(byte)255; }
                    Marshal.Copy(row,0,IntPtr.Add(pixels.Scan0,y*pixels.Stride),row.Length);
                }
            } finally { image.UnlockBits(pixels); }
            using(var output=new MemoryStream()) { image.Save(output,ImageFormat.Png);if(output.Length>MaxBytes) throw new IOException("Encoded image exceeds limit");return output.ToArray(); }
        }
    }
    static byte[] DecodeText(byte[] raw) {
        int end=-1;
        for(int i=0;i+1<raw.Length;i+=2) if(raw[i]==0 && raw[i+1]==0) {end=i;break;}
        if(end<0) throw new IOException("Unterminated UTF-16 clipboard text");
        string text=new UnicodeEncoding(false,false,true).GetString(raw,0,end);
        var bytes=Utf8.GetBytes(text);if(bytes.Length>1024*1024) throw new IOException("Text exceeds limit");return bytes;
    }
    class ClipboardWindow : Form {
        uint png=RegisterClipboardFormat("PNG");
        uint exclude=RegisterClipboardFormat("ExcludeClipboardContentFromMonitorProcessing");
        uint history=RegisterClipboardFormat("CanIncludeInClipboardHistory");
        uint cloud=RegisterClipboardFormat("CanUploadToCloudClipboard");
        ulong epoch; uint previous;
        public ClipboardWindow() { ShowInTaskbar=false; FormBorderStyle=FormBorderStyle.FixedToolWindow; Opacity=0; }
        ulong Sequence() { uint n=GetClipboardSequenceNumber();if(n<previous) epoch+=1UL<<32;previous=n;return epoch+n; }
        protected override void OnHandleCreated(EventArgs e) { base.OnHandleCreated(e);if(!AddClipboardFormatListener(Handle)) throw new IOException("Clipboard listener unavailable"); }
        protected override void OnHandleDestroyed(EventArgs e) { RemoveClipboardFormatListener(Handle);base.OnHandleDestroyed(e); }
        protected override void SetVisibleCore(bool visible) { base.SetVisibleCore(false); }
        bool Sensitive() {
            if(IsClipboardFormatAvailable(exclude)) return true;
            foreach(uint f in new[]{history,cloud}) if(IsClipboardFormatAvailable(f)) {
                var b=ReadData(f,16);if(b.Length<4 || BitConverter.ToInt32(b,0)==0) return true;
            }
            return false;
        }
        public object Execute(Dictionary<string,object> request) {
            string op=Text(request,"op");
            if(op=="inspect") return new {generation=Sequence()};
            byte[] write=null,dib=null;uint format=0;
            if(op=="write") {
                write=Convert.FromBase64String(Text(request,"data"));if(write.Length<1 || write.Length>MaxBytes) throw new IOException("Clipboard content exceeds limit");
                string kind=Text(request,"format");
                if(kind=="text") {
                    string text=Utf8.GetString(write);if(write.Length>1024*1024 || text.IndexOf('\0')>=0) throw new IOException("Invalid text");
                    var normalized=new StringBuilder();char last='\0';foreach(char c in text) { if(c=='\n' && last!='\r') normalized.Append('\r');normalized.Append(c);last=c; }
                    write=new UnicodeEncoding(false,false,true).GetBytes(normalized.ToString()+"\0");format=13;
                } else if(kind=="png") { dib=Dib(write);format=png; }
                else throw new IOException("Unsupported format");
            } else if(op!="read") throw new IOException("Unknown clipboard operation");
            bool opened=false;for(int i=0;i<20;i++) { if(OpenClipboard(Handle)) { opened=true;break; } Thread.Sleep(10); }
            if(!opened) throw new IOException("Clipboard busy");
            try {
                ulong generation=Sequence();
                if(op=="write") {
                    if(generation!=Convert.ToUInt64(request["generation"])) throw new IOException("Clipboard superseded");
                    if(!EmptyClipboard()) throw new IOException("Cannot empty clipboard");
                    Put(format,write);if(dib!=null) Put(17,dib);
                    return new {generation=Sequence()};
                }
                byte[] bytes=null;string type=null;bool sensitive=Sensitive();
                if(IsClipboardFormatAvailable(png)) { bytes=ReadData(png,MaxBytes);type="png"; }
                else if(IsClipboardFormatAvailable(17) || IsClipboardFormatAvailable(8)) { bytes=PngFromDib(ReadData(IsClipboardFormatAvailable(17)?17u:8u,17*1024*1024));type="png"; }
                else if(IsClipboardFormatAvailable(13)) {
                    bytes=DecodeText(ReadData(13,2*1024*1024+2));type="text";
                }
                if(Sequence()!=generation) throw new IOException("Clipboard changed during capture");
                return new {generation=generation,sensitive=sensitive,format=type,data=bytes==null || bytes.Length==0?null:Convert.ToBase64String(bytes)};
            } finally { CloseClipboard(); }
        }
    }
    static void ClipboardMain() {
        using(var window=new ClipboardWindow()) {
            IntPtr handle=window.Handle;
            new Thread(delegate() {
                string line;
                while((line=Console.ReadLine())!=null) {
                    try { var request=Obj(line);var result=window.Invoke(new Func<object>(delegate { return window.Execute(request); }));Output(result); }
                    catch(Exception e) { Output(new {error=(e.InnerException??e).Message}); }
                }
                window.BeginInvoke(new Action(delegate { Application.ExitThread(); }));
            }) {IsBackground=true}.Start();
            Application.Run();
        }
    }
    static void Assert(bool value,string name) { if(!value) throw new Exception("FAILED: "+name);Console.WriteLine("PASS: "+name); }
    static void SelfTest() {
        using(var window=new ClipboardWindow()) {
            IntPtr handle=window.Handle;
            var saved=Obj(Encode(window.Execute(Obj("{\"op\":\"read\"}"))));
            Func<Dictionary<string,object>> read=delegate {return Obj(Encode(window.Execute(Obj("{\"op\":\"read\"}"))));};
            Action<string,byte[]> write=delegate(string format,byte[] bytes) {
                var before=Obj(Encode(window.Execute(Obj("{\"op\":\"inspect\"}"))));
                window.Execute(Obj(Encode(new {op="write",generation=before["generation"],format=format,data=Convert.ToBase64String(bytes)})));
            };
            try {
                string mixed="Shuttli Windows test: 中文 😀\r\nLF\nCR\rEND";
                write("text",Utf8.GetBytes(mixed));
                var actual=read();string text=Utf8.GetString(Convert.FromBase64String(Text(actual,"data")));
                Assert(text==mixed.Replace("LF\n","LF\r\n"),"Unicode and mixed line endings preserve isolated CR");
                bool stale=false;try {window.Execute(Obj(Encode(new {op="write",generation=0,format="text",data=Convert.ToBase64String(Utf8.GetBytes("stale"))})));} catch(IOException) {stale=true;}
                Assert(stale && Text(read(),"data")==Text(actual,"data"),"stale baseline rejects before OS mutation");
                bool invalid=false;try {write("text",new byte[]{0xff});} catch(DecoderFallbackException) {invalid=true;}
                Assert(invalid,"invalid UTF-8 rejected");
                invalid=false;try {write("text",Utf8.GetBytes("a\0b"));} catch(IOException) {invalid=true;}
                Assert(invalid,"embedded NUL rejected");
                if(!OpenClipboard(handle)) throw new IOException("test clipboard busy");
                try {EmptyClipboard();Put(13,new byte[]{0,0xd8,0,0});} finally {CloseClipboard();}
                invalid=false;try {read();} catch(DecoderFallbackException) {invalid=true;}
                Assert(invalid,"invalid UTF-16 rejected");
                if(!OpenClipboard(handle)) throw new IOException("test clipboard busy");
                try {EmptyClipboard();Put(13,new byte[]{65,65,65,65,65,65,65,65,65,65,65,65,65,65,65,65,65,65,65,65,65,65,65,65,65,65,65,65,65,65,65,65});} finally {CloseClipboard();}
                invalid=false;try {DecodeText(new byte[]{65,0});} catch(IOException) {invalid=true;}
                Assert(invalid,"unterminated UTF-16 rejected");
                byte[] png;
                using(var image=new Bitmap(3,2,PixelFormat.Format32bppArgb)) using(var output=new MemoryStream()) {
                    image.SetPixel(0,0,Color.FromArgb(0,10,20,30));image.SetPixel(1,0,Color.FromArgb(128,250,120,30));image.SetPixel(2,1,Color.FromArgb(255,10,220,70));image.Save(output,ImageFormat.Png);png=output.ToArray();
                }
                write("png",png);actual=read();
                Assert(Text(actual,"format")=="png" && Text(actual,"data")==Convert.ToBase64String(png),"registered PNG independent OS readback");
                byte[] dib;
                if(!OpenClipboard(handle)) throw new IOException("test clipboard busy");
                try {dib=ReadData(17,17*1024*1024);EmptyClipboard();Put(17,dib);} finally {CloseClipboard();}
                actual=read();
                using(var expected=new Bitmap(new MemoryStream(png))) using(var received=new Bitmap(new MemoryStream(Convert.FromBase64String(Text(actual,"data"))))) {
                    bool equal=expected.Size==received.Size;
                    for(int y=0;y<2;y++) for(int x=0;x<3;x++) equal &= expected.GetPixel(x,y).ToArgb()==received.GetPixel(x,y).ToArgb();
                    Assert(equal,"DIBV5 preserves alpha, dimensions and row direction");
                }
                byte[] bottomUp24=new byte[40+12*2];
                using(var writer=new BinaryWriter(new MemoryStream(bottomUp24))) {writer.Write(40);writer.Write(3);writer.Write(2);writer.Write((short)1);writer.Write((short)24);}
                bottomUp24[40]=7;bottomUp24[41]=8;bottomUp24[42]=9;bottomUp24[52]=10;bottomUp24[53]=11;bottomUp24[54]=12;
                using(var received=new Bitmap(new MemoryStream(PngFromDib(bottomUp24)))) Assert(received.GetPixel(0,0)==Color.FromArgb(255,12,11,10) && received.GetPixel(0,1)==Color.FromArgb(255,9,8,7),"bottom-up 24-bit DIB stride and opaque pixels");
                invalid=false;try {PngFromDib(new byte[39]);} catch(IOException) {invalid=true;}Assert(invalid,"truncated DIB rejected");
                write("text",Utf8.GetBytes("sensitive"));
                if(!OpenClipboard(handle)) throw new IOException("test clipboard busy");
                try {Put(RegisterClipboardFormat("ExcludeClipboardContentFromMonitorProcessing"),new byte[]{1,0,0,0});} finally {CloseClipboard();}
                Assert((bool)read()["sensitive"],"sensitive clipboard marker honored");
                var acquired=new ManualResetEvent(false);var release=new ManualResetEvent(false);
                var holder=new Thread(delegate() {if(OpenClipboard(IntPtr.Zero)) {acquired.Set();release.WaitOne();CloseClipboard();}});holder.Start();
                if(!acquired.WaitOne(2000)) throw new IOException("Cannot establish clipboard contention");
                try {invalid=false;try {read();} catch(IOException) {invalid=true;}Assert(invalid,"clipboard contention fails within bounded retry");}
                finally {release.Set();holder.Join();}
            } finally {
                if(Get(saved,"data")!=null) {
                    write(Text(saved,"format"),Convert.FromBase64String(Text(saved,"data")));
                    if((bool)saved["sensitive"] && OpenClipboard(handle)) {try {Put(RegisterClipboardFormat("ExcludeClipboardContentFromMonitorProcessing"),new byte[]{1,0,0,0});} finally {CloseClipboard();}}
                } else if(OpenClipboard(handle)) {try {EmptyClipboard();} finally {CloseClipboard();}}
            }
        }
    }
    static void Autostart(string exe,string action) {
        const string run="Software\\Microsoft\\Windows\\CurrentVersion\\Run";
        const string approved="Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\Run";
        using(var key=Microsoft.Win32.Registry.CurrentUser.CreateSubKey(run)) {
            if(action=="on") key.SetValue("Shuttli","\""+Application.ExecutablePath+"\" launch \""+exe+"\"");
            else if(action=="off") key.DeleteValue("Shuttli",false);
            bool enabled=key.GetValue("Shuttli")!=null,blocked=false;
            using(var approval=Microsoft.Win32.Registry.CurrentUser.OpenSubKey(approved)) {
                byte[] value=approval==null?null:approval.GetValue("Shuttli") as byte[];
                blocked=value!=null && value.Length>0 && (value[0]==3 || value[0]==7);
            }
            Output(new {state=!enabled?"disabled":blocked?"requires_user_action":"enabled",message=!enabled?"Disabled":blocked?"Enable Shuttli in Windows Settings > Apps > Startup":"Enabled for the current user's next sign-in"});
        }
    }
    [STAThread] static int Main(string[] args) {
        Console.InputEncoding=Utf8;Console.OutputEncoding=new UTF8Encoding(false);
        try {
            switch(args[0]) {
                case "secure-dir": SecureDir(args[1]);break;
                case "clipboard": ClipboardMain();break;
                case "self-test": SelfTest();break;
                case "server": Server(args[1]);break;
                case "client": Console.WriteLine(Call(args[1],Console.ReadLine()));break;
                case "ui": RunDesignUi(args[1],args[2],null);break;
                case "ui-fixture": RunDesignUi(Path.GetDirectoryName(Path.GetFullPath(args[1])),null,args[1]);break;
                case "ui-render": RenderDesignUi(args[1],args[2],args.Length>3?Double.Parse(args[3],System.Globalization.CultureInfo.InvariantCulture):1,args.Length>4?Double.Parse(args[4],System.Globalization.CultureInfo.InvariantCulture):850,args.Length>5?Double.Parse(args[5],System.Globalization.CultureInfo.InvariantCulture):620);break;
                case "autostart": Autostart(args[1],args[2]);break;
                case "launch": Process.Start(new ProcessStartInfo(args[1],"daemon") {UseShellExecute=false,CreateNoWindow=true});break;
                case "notify": using(var icon=new NotifyIcon {Icon=SystemIcons.Application,Visible=true,Text="Shuttli"}) {icon.ShowBalloonTip(2500,args[1].Length>60?args[1].Substring(0,60):args[1],args[2].Length>240?args[2].Substring(0,240):args[2],ToolTipIcon.Info);var end=DateTime.UtcNow.AddSeconds(3);while(DateTime.UtcNow<end) {Application.DoEvents();Thread.Sleep(30);}}break;
                default: throw new Exception("Unknown native helper mode");
            }
            return Environment.ExitCode;
        } catch(Exception e) { Console.Error.WriteLine(e.Message);return 1; }
    }
}
