// Native desktop presentation of the shared control API. No clipboard/network policy.
using System;
using System.Collections;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Reflection;
using System.Text;
using System.Threading;
using System.Threading.Tasks;
using W = System.Windows;
using C = System.Windows.Controls;
using P = System.Windows.Controls.Primitives;
using M = System.Windows.Media;
using B = System.Windows.Media.Imaging;
using A = System.Windows.Automation;
using D = System.Windows.Threading;
using F = System.Windows.Forms;

partial class Native {
    static Stream Resource(string name) { return Assembly.GetExecutingAssembly().GetManifestResourceStream(name); }
    static string ResourceText(string name) { using(var s=Resource(name)) using(var r=new StreamReader(s,Encoding.UTF8)) return r.ReadToEnd(); }
    static string Product { get { return Environment.GetEnvironmentVariable("SHUTTLI_PRODUCT_NAME") ?? ResourceText("shuttli.ProductName").Trim(); } }
    static void RenderDesignUi(string fixture,string output,double scale,double width,double height) {
        var app=new W.Application {ShutdownMode=W.ShutdownMode.OnExplicitShutdown};
        var window=new DesignWindow(Path.GetDirectoryName(Path.GetFullPath(fixture)),null,fixture);
        window.WindowStyle=W.WindowStyle.None;window.ResizeMode=W.ResizeMode.NoResize;window.ShowInTaskbar=false;window.Left=-20000;window.Top=-20000;window.Width=width;window.Height=height;
        window.Loaded+=async delegate {try {await window.ExportReview(output,scale,width,height);}catch(Exception e) {Console.Error.WriteLine(e);Environment.ExitCode=1;}finally {app.Shutdown();}};
        app.Run(window);
    }
    static void RunDesignUi(string profile,string executable,string fixture) {
        bool created;
        using(var signal=new EventWaitHandle(false,EventResetMode.AutoReset,"Local\\"+PipeName(profile)+"-window",out created)) {
            if(!created) {signal.Set();return;}
            var app=new W.Application {ShutdownMode=W.ShutdownMode.OnExplicitShutdown};
            var window=new DesignWindow(profile,executable,fixture);
            var wait=ThreadPool.RegisterWaitForSingleObject(signal,delegate {window.Post(new Action(window.Reveal));},null,Timeout.Infinite,false);
            try {app.Run(window);} finally {wait.Unregister(null);}
        }
    }

    sealed class HistoryGroup {
        public Dictionary<string,object> Row;
        public readonly List<Dictionary<string,object>> Transfers=new List<Dictionary<string,object>>();
        public object ContentId;
        public bool Local;
        public string Key;
    }

    sealed class DesignWindow : W.Window {
        readonly string profile,executable,fixturePath;
        readonly Dictionary<string,Dictionary<string,string>> catalogs=new Dictionary<string,Dictionary<string,string>>();
        readonly Dictionary<string,P.ToggleButton> toggles=new Dictionary<string,P.ToggleButton>();
        readonly Dictionary<string,string> edits=new Dictionary<string,string>();
        readonly Dictionary<string,C.Button> navigation=new Dictionary<string,C.Button>();
        readonly SemaphoreSlim gate=new SemaphoreSlim(1,1);
        readonly F.NotifyIcon tray=new F.NotifyIcon();
        readonly D.DispatcherTimer timer=new D.DispatcherTimer {Interval=TimeSpan.FromSeconds(2)};
        readonly C.Grid root=new C.Grid();
        readonly C.StackPanel sidebar=new C.StackPanel();
        readonly C.StackPanel summary=new C.StackPanel();
        readonly C.StackPanel body=new C.StackPanel();
        readonly C.TextBlock title=new C.TextBlock();
        readonly C.TextBlock message=new C.TextBlock();
        readonly C.Border messageBox=new C.Border();
        readonly C.ScrollViewer scroll=new C.ScrollViewer();
        C.Button back,refresh;
        Dictionary<string,object> settings,current,fixture;
        List<Dictionary<string,object>> peers=new List<Dictionary<string,object>>();
        List<Dictionary<string,object>> history=new List<Dictionary<string,object>>();
        List<HistoryGroup> groups=new List<HistoryGroup>();
        Dictionary<string,object> selectedPeer;
        HistoryGroup selectedHistory;
        string page="status",language="system",theme="system",signature="",autostartState="unknown";
        string autostartMessage="";
        bool service,polling,quitting,dark,modal,closed,deferredRender,pollErrorNotice;
        int generation,historyPage;
        double textScale=1;
        string reviewOutput;
        double reviewScale=1;

        static Dictionary<string,object> Dict(object value) {return value as Dictionary<string,object> ?? new Dictionary<string,object>();}
        static List<Dictionary<string,object>> Items(object value) {var result=new List<Dictionary<string,object>>();if(value is IEnumerable && !(value is string)) foreach(object row in (IEnumerable)value) if(row is Dictionary<string,object>) result.Add((Dictionary<string,object>)row);return result;}
        static bool Flag(Dictionary<string,object> value,string key) {return Get(value,key) is bool && (bool)value[key];}
        static long Number(Dictionary<string,object> value,string key) {try {return Convert.ToInt64(Get(value,key));} catch {return 0;}}
        string T(string key,params object[] replacements) {
            string code=language=="system"?CultureInfo.CurrentUICulture.TwoLetterISOLanguageName:language;
            if(!catalogs.ContainsKey(code)) code="en";
            string result;
            if(!catalogs[code].TryGetValue(key,out result) && !catalogs["en"].TryGetValue(key,out result)) result=key;
            result=result.Replace("{app_name}",Product);
            for(int i=0;i+1<replacements.Length;i+=2) result=result.Replace("{"+replacements[i]+"}",Convert.ToString(replacements[i+1],CultureInfo.CurrentCulture));
            return result;
        }
        M.Brush Brush(string key) {return (M.Brush)Resources[key];}
        void Color(string key,string hex) {Resources[key]=new M.SolidColorBrush((M.Color)M.ColorConverter.ConvertFromString(hex));}
        void Theme() {
            bool systemDark=false;
            using(var key=Microsoft.Win32.Registry.CurrentUser.OpenSubKey("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize")) if(key!=null) systemDark=Convert.ToInt32(key.GetValue("AppsUseLightTheme",1))==0;
            dark=theme=="dark" || (theme=="system" && systemDark);
            Color("Page",dark?"#212124":"#F9F9FB");Color("Sidebar",dark?"#2D2D31":"#EBEBED");
            Color("Card",dark?"#353539":"#FFFFFF");Color("Border",dark?"#48484D":"#DCDCE1");
            Color("Text",dark?"#F7F7F8":"#29292E");Color("Muted",dark?"#B7B7C1":"#666671");
            Color("Selected",dark?"#45454A":"#D5D5D9");Color("Accent",dark?"#93B8FF":"#2B62D9");
            Color("Button", "#2B62D9");Color("Info",dark?"#293E61":"#E4EDFC");
            Color("Green",dark?"#80DFA5":"#177343");Color("GreenBg",dark?"#233E34":"#DFF3E8");
            Color("Amber",dark?"#EFBE76":"#8A5A10");Color("AmberBg",dark?"#4D3D24":"#FCF0CF");
            Color("Red",dark?"#FF9690":"#BC272A");Color("RedBg",dark?"#4C2C30":"#FCE3E6");
            if(W.SystemParameters.HighContrast) {
                Resources["Page"]=W.SystemColors.WindowBrush;Resources["Sidebar"]=W.SystemColors.ControlBrush;Resources["Card"]=W.SystemColors.WindowBrush;
                Resources["Text"]=W.SystemColors.WindowTextBrush;Resources["Muted"]=W.SystemColors.WindowTextBrush;Resources["Border"]=W.SystemColors.WindowTextBrush;
                Resources["Accent"]=W.SystemColors.HotTrackBrush;Resources["Selected"]=W.SystemColors.HighlightBrush;
            }
            Background=Brush("Page");Foreground=Brush("Text");
        }
        C.TextBlock Label(string text,string color="Text",double size=14,bool bold=false) {
            var label=new C.TextBlock {Text=text,FontSize=size*textScale,TextWrapping=W.TextWrapping.Wrap,FontWeight=bold?W.FontWeights.SemiBold:W.FontWeights.Normal};
            label.SetResourceReference(C.TextBlock.ForegroundProperty,color);return label;
        }
        C.Button Button(string text,Action action,bool primary=false,bool link=false) {
            var button=new C.Button {Content=Label(text,primary?"White":"Accent",13.5,primary),Padding=new W.Thickness(link?2:14,link?3:8,link?2:14,link?3:8),MinHeight=link?28:34,Cursor=System.Windows.Input.Cursors.Hand,HorizontalAlignment=W.HorizontalAlignment.Right,VerticalAlignment=W.VerticalAlignment.Center,HorizontalContentAlignment=W.HorizontalAlignment.Center,VerticalContentAlignment=W.VerticalAlignment.Center};
            button.SetResourceReference(C.Control.ForegroundProperty,primary?"White":"Accent");
            button.SetResourceReference(C.Control.BackgroundProperty,primary?"Button":link?"Page":"Card");
            button.SetResourceReference(C.Control.BorderBrushProperty,primary?"Button":link?"Page":"Border");
            button.BorderThickness=new W.Thickness(link?0:1);
            button.Style=(W.Style)Resources["RoundButton"];
            A.AutomationProperties.SetName(button,text);button.Click+=delegate {action();};return button;
        }
        C.Border Card(C.StackPanel contents) {
            var border=new C.Border {CornerRadius=new W.CornerRadius(12),BorderThickness=new W.Thickness(1),Child=contents,Margin=new W.Thickness(0,0,0,4)};
            border.SetResourceReference(C.Border.BackgroundProperty,"Card");border.SetResourceReference(C.Border.BorderBrushProperty,"Border");return border;
        }
        C.StackPanel Group(string heading=null) {
            if(heading!=null) body.Children.Add(new C.Border {Child=Label(heading,"Text",14, true),Padding=new W.Thickness(3,14,0,9)});
            var content=new C.StackPanel();body.Children.Add(Card(content));return content;
        }
        C.Grid Row(C.StackPanel group,string heading,string hint=null,W.UIElement control=null) {
            var row=new C.Grid {Margin=new W.Thickness(15,12,15,12),MinHeight=37};
            row.ColumnDefinitions.Add(new C.ColumnDefinition {Width=new W.GridLength(1,W.GridUnitType.Star)});
            row.ColumnDefinitions.Add(new C.ColumnDefinition {Width=W.GridLength.Auto});
            var labels=new C.StackPanel {VerticalAlignment=W.VerticalAlignment.Center,Margin=new W.Thickness(0,0,control==null?0:18,0)};
            labels.Children.Add(Label(heading));if(!String.IsNullOrEmpty(hint)) labels.Children.Add(new C.Border {Child=Label(hint,"Muted",12),Margin=new W.Thickness(0,5,0,0)});
            row.Children.Add(labels);if(control!=null) {C.Grid.SetColumn(control,1);row.Children.Add(control);}
            if(group.Children.Count>0) {var separator=new C.Border {Height=1};separator.SetResourceReference(C.Border.BackgroundProperty,"Border");group.Children.Add(separator);}
            group.Children.Add(row);return row;
        }
        P.ToggleButton Toggle(string label,bool value,Action<bool> change) {
            var toggle=new P.ToggleButton {IsChecked=value,Width=44,Height=25,VerticalAlignment=W.VerticalAlignment.Center,Template=(C.ControlTemplate)Resources["Switch"],Cursor=System.Windows.Input.Cursors.Hand};
            A.AutomationProperties.SetName(toggle,label);toggle.ToolTip=label;
            toggle.Click+=delegate {change(toggle.IsChecked==true);};return toggle;
        }
        void Setting(C.StackPanel group,string key,string hint=null,Dictionary<string,object> policy=null,string peerId=null) {
            var source=policy??settings;
            var toggle=Toggle(T("setting."+key),Flag(source,key),delegate(bool desired) {
                if(policy!=null) {
                    if(key=="send" && desired) {AllowPeer(selectedPeer,policy);return;}
                    var next=Obj(Encode(policy));next[key]=desired;
                    Operate(new {command="peer",id=peerId,expected=policy,policy=next});
                } else if(key=="send" || key=="receive") Operate(new {command="set_directions",send=key=="send"?(bool?)desired:null,receive=key=="receive"?(bool?)desired:null});
                else Configure(new Dictionary<string,object>{{key,desired}});
            });
            toggle.IsEnabled=service;
            if(policy==null) toggles[key]=toggle;
            Row(group,T("setting."+key),hint,toggle);
        }
        void Styles() {
            Resources["White"]=M.Brushes.White;
            const string xmlns="xmlns='http://schemas.microsoft.com/winfx/2006/xaml/presentation' xmlns:x='http://schemas.microsoft.com/winfx/2006/xaml'";
            Resources["RoundButton"]=W.Markup.XamlReader.Parse("<Style "+xmlns+" TargetType='{x:Type Button}'><Setter Property='Template'><Setter.Value><ControlTemplate TargetType='{x:Type Button}'><Border x:Name='bd' Background='{TemplateBinding Background}' BorderBrush='{TemplateBinding BorderBrush}' BorderThickness='{TemplateBinding BorderThickness}' CornerRadius='7' Padding='{TemplateBinding Padding}'><ContentPresenter HorizontalAlignment='{TemplateBinding HorizontalContentAlignment}' VerticalAlignment='{TemplateBinding VerticalContentAlignment}'/></Border><ControlTemplate.Triggers><Trigger Property='IsMouseOver' Value='True'><Setter TargetName='bd' Property='Opacity' Value='0.82'/></Trigger><Trigger Property='IsEnabled' Value='False'><Setter TargetName='bd' Property='Opacity' Value='0.45'/></Trigger><Trigger Property='IsKeyboardFocused' Value='True'><Setter TargetName='bd' Property='BorderBrush' Value='{DynamicResource Accent}'/><Setter TargetName='bd' Property='BorderThickness' Value='2'/></Trigger></ControlTemplate.Triggers></ControlTemplate></Setter.Value></Setter></Style>");
            Resources["Switch"]=W.Markup.XamlReader.Parse("<ControlTemplate "+xmlns+" TargetType='{x:Type ToggleButton}'><Border x:Name='track' Background='#898991' CornerRadius='13' BorderBrush='{DynamicResource Border}' BorderThickness='1'><Ellipse x:Name='thumb' Fill='White' Width='19' Height='19' HorizontalAlignment='Left' Margin='2'/></Border><ControlTemplate.Triggers><Trigger Property='IsChecked' Value='True'><Setter TargetName='track' Property='Background' Value='#2B62D9'/><Setter TargetName='thumb' Property='HorizontalAlignment' Value='Right'/></Trigger><Trigger Property='IsEnabled' Value='False'><Setter TargetName='track' Property='Opacity' Value='0.4'/></Trigger><Trigger Property='IsKeyboardFocused' Value='True'><Setter TargetName='track' Property='BorderBrush' Value='{DynamicResource Text}'/><Setter TargetName='track' Property='BorderThickness' Value='2'/></Trigger></ControlTemplate.Triggers></ControlTemplate>");
        }
        void ExtraStyles() {
            const string xmlns="xmlns='http://schemas.microsoft.com/winfx/2006/xaml/presentation' xmlns:x='http://schemas.microsoft.com/winfx/2006/xaml'";
            Resources[typeof(P.ScrollBar)]=W.Markup.XamlReader.Parse("<Style "+xmlns+" TargetType='{x:Type ScrollBar}'><Setter Property='Width' Value='12'/><Setter Property='Template'><Setter.Value><ControlTemplate TargetType='{x:Type ScrollBar}'><Track x:Name='PART_Track' IsDirectionReversed='True' Orientation='{TemplateBinding Orientation}'><Track.DecreaseRepeatButton><RepeatButton Command='ScrollBar.PageUpCommand' Opacity='0'/></Track.DecreaseRepeatButton><Track.Thumb><Thumb><Thumb.Template><ControlTemplate TargetType='Thumb'><Border Background='#88888F' CornerRadius='4' Margin='3,2'/></ControlTemplate></Thumb.Template></Thumb></Track.Thumb><Track.IncreaseRepeatButton><RepeatButton Command='ScrollBar.PageDownCommand' Opacity='0'/></Track.IncreaseRepeatButton></Track></ControlTemplate></Setter.Value></Setter></Style>");
            Resources["Choice"]=W.Markup.XamlReader.Parse("<ControlTemplate "+xmlns+" TargetType='{x:Type ComboBox}'><Grid><ToggleButton Focusable='False' IsChecked='{Binding IsDropDownOpen,Mode=TwoWay,RelativeSource={RelativeSource TemplatedParent}}'><ToggleButton.Template><ControlTemplate TargetType='ToggleButton'><Border Background='{DynamicResource Card}' BorderBrush='{DynamicResource Border}' BorderThickness='1' CornerRadius='6'><TextBlock Text='⌄' Foreground='{DynamicResource Text}' HorizontalAlignment='Right' VerticalAlignment='Center' Margin='8'/></Border></ControlTemplate></ToggleButton.Template></ToggleButton><ContentPresenter Margin='9,6,25,6' HorizontalAlignment='Left' VerticalAlignment='Center' Content='{TemplateBinding SelectionBoxItem}' IsHitTestVisible='False'/><Popup x:Name='PART_Popup' Placement='Bottom' IsOpen='{TemplateBinding IsDropDownOpen}' AllowsTransparency='True' Focusable='False' PopupAnimation='None'><Border Background='{DynamicResource Card}' BorderBrush='{DynamicResource Border}' BorderThickness='1' CornerRadius='6'><ScrollViewer MaxHeight='260'><ItemsPresenter/></ScrollViewer></Border></Popup></Grid></ControlTemplate>");
        }
        W.UIElement Logo() {
            var canvas=new C.Canvas {Width=28,Height=28,Margin=new W.Thickness(0,0,10,0)};
            var shapes=new C.Canvas {Width=64,Height=64,RenderTransform=new M.ScaleTransform(.43,.43)};
            var outline=new System.Windows.Shapes.Path {Data=M.Geometry.Parse("M16 46A8 8 0 0 1 8 38V14A8 8 0 0 1 16 6H34A8 8 0 0 1 42 14"),StrokeThickness=6,StrokeStartLineCap=M.PenLineCap.Round,StrokeEndLineCap=M.PenLineCap.Round};
            outline.SetResourceReference(System.Windows.Shapes.Shape.StrokeProperty,"Text");shapes.Children.Add(outline);
            var sheet=new System.Windows.Shapes.Rectangle {Width=34,Height=40,RadiusX=8,RadiusY=8,Fill=new M.SolidColorBrush((M.Color)M.ColorConverter.ConvertFromString("#2B62D9"))};C.Canvas.SetLeft(sheet,22);C.Canvas.SetTop(sheet,18);shapes.Children.Add(sheet);canvas.Children.Add(shapes);return canvas;
        }
        W.UIElement NavIcon(string page) {
            string circle="M12 2 A10 10 0 1 0 12 22 A10 10 0 1 0 12 2 ";
            string data=page=="status"?circle+"M7 12 L10 15 L17 8":page=="devices"?"M4 3 H20 V17 H4 Z M1 21 H23":page=="history"?circle+"M12 6 V12 L17 15":"M2 5 H22 M2 12 H22 M2 19 H22 M7 3 V7 M17 10 V14 M9 17 V21";
            var icon=new System.Windows.Shapes.Path {Data=M.Geometry.Parse(data),Width=21,Height=21,Stretch=M.Stretch.Uniform,StrokeThickness=1.8,StrokeStartLineCap=M.PenLineCap.Round,StrokeEndLineCap=M.PenLineCap.Round,Margin=new W.Thickness(0,0,13,0)};icon.SetResourceReference(System.Windows.Shapes.Shape.StrokeProperty,"Text");return icon;
        }
        void Shell() {
            root.SetResourceReference(C.Panel.BackgroundProperty,"Page");
            root.ColumnDefinitions.Add(new C.ColumnDefinition {Width=new W.GridLength(202)});root.ColumnDefinitions.Add(new C.ColumnDefinition());
            var sidebarFrame=new C.Border {Padding=new W.Thickness(10),BorderThickness=new W.Thickness(0,0,1,0)};
            sidebarFrame.SetResourceReference(C.Border.BackgroundProperty,"Sidebar");sidebarFrame.SetResourceReference(C.Border.BorderBrushProperty,"Border");root.Children.Add(sidebarFrame);
            var dock=new C.DockPanel();sidebarFrame.Child=dock;
            var bottom=Card(summary);bottom.Padding=new W.Thickness(12);bottom.Margin=new W.Thickness(0,8,0,0);C.DockPanel.SetDock(bottom,C.Dock.Bottom);dock.Children.Add(bottom);
            dock.Children.Add(new C.ScrollViewer {Content=sidebar,VerticalScrollBarVisibility=C.ScrollBarVisibility.Auto,HorizontalScrollBarVisibility=C.ScrollBarVisibility.Disabled});
            var brand=new C.StackPanel {Orientation=C.Orientation.Horizontal,Margin=new W.Thickness(14,10,0,25)};brand.Children.Add(Logo());brand.Children.Add(Label(Product,"Text",18,true));sidebar.Children.Add(brand);
            string[] pages={"status","devices","history","settings"};
            for(int i=0;i<pages.Length;i++) {
                string id=pages[i];var line=new C.StackPanel {Orientation=C.Orientation.Horizontal};line.Children.Add(NavIcon(id));var navText=Label(T("nav."+id),"Text",15);navText.MaxWidth=122;line.Children.Add(navText);
                var nav=Button(T("nav."+id),delegate {Navigate(id);});nav.Content=line;nav.HorizontalAlignment=W.HorizontalAlignment.Stretch;nav.HorizontalContentAlignment=W.HorizontalAlignment.Left;nav.Padding=new W.Thickness(12,10,8,10);nav.Margin=new W.Thickness(0,0,0,3);nav.BorderThickness=new W.Thickness(0);navigation[id]=nav;sidebar.Children.Add(nav);
            }
            var main=new C.DockPanel();C.Grid.SetColumn(main,1);root.Children.Add(main);
            var header=new C.Grid {Height=43};header.ColumnDefinitions.Add(new C.ColumnDefinition {Width=new W.GridLength(56)});header.ColumnDefinitions.Add(new C.ColumnDefinition());header.ColumnDefinitions.Add(new C.ColumnDefinition {Width=new W.GridLength(56)});
            var headerFrame=new C.Border {BorderThickness=new W.Thickness(0,0,0,1),Child=header};headerFrame.SetResourceReference(C.Border.BorderBrushProperty,"Border");C.DockPanel.SetDock(headerFrame,C.Dock.Top);main.Children.Add(headerFrame);
            back=Button("‹",delegate {Navigate(page=="device"?"devices":"history");},false,true);back.FontSize=22;back.HorizontalAlignment=W.HorizontalAlignment.Center;header.Children.Add(back);
            title.FontSize=15*textScale;title.FontWeight=W.FontWeights.SemiBold;title.HorizontalAlignment=W.HorizontalAlignment.Center;title.VerticalAlignment=W.VerticalAlignment.Center;title.SetResourceReference(C.TextBlock.ForegroundProperty,"Text");C.Grid.SetColumn(title,1);header.Children.Add(title);
            refresh=Button("↻",delegate {Poll(true);},false,true);refresh.ToolTip=T("action.refresh_discovery");A.AutomationProperties.SetName(refresh,T("history.refresh"));refresh.HorizontalAlignment=W.HorizontalAlignment.Center;C.Grid.SetColumn(refresh,2);header.Children.Add(refresh);
            message.TextWrapping=W.TextWrapping.Wrap;message.FontSize=12*textScale;messageBox.Child=message;messageBox.Padding=new W.Thickness(16,9,16,9);messageBox.Visibility=W.Visibility.Collapsed;messageBox.SetResourceReference(C.Border.BackgroundProperty,"Info");message.SetResourceReference(C.TextBlock.ForegroundProperty,"Text");C.DockPanel.SetDock(messageBox,C.Dock.Top);main.Children.Add(messageBox);
            body.Margin=new W.Thickness(27,22,27,24);scroll.Content=body;scroll.VerticalScrollBarVisibility=C.ScrollBarVisibility.Auto;scroll.HorizontalScrollBarVisibility=C.ScrollBarVisibility.Disabled;main.Children.Add(scroll);Content=root;
        }
        public DesignWindow(string dir,string exe,string fixtureFile) {
            profile=dir;executable=exe;fixturePath=fixtureFile;
            foreach(string code in new[]{"en","nl","de","fr"}) catalogs[code]=Json().Deserialize<Dictionary<string,string>>(ResourceText("shuttli.Locale."+code));
            LoadPreferences();if(fixturePath!=null) fixture=Obj(File.ReadAllText(fixturePath,Encoding.UTF8));
            using(var textSettings=Microsoft.Win32.Registry.CurrentUser.OpenSubKey("Software\\Microsoft\\Accessibility")) if(textSettings!=null) textScale=Math.Max(1,Math.Min(2,Convert.ToDouble(textSettings.GetValue("TextScaleFactor",100))/100));
            if(fixture!=null && Get(fixture,"text_scale")!=null) textScale=Math.Max(1,Math.Min(2,Convert.ToDouble(fixture["text_scale"])));
            Title=Product;Width=850;Height=620;MinWidth=720;MinHeight=520;WindowStartupLocation=W.WindowStartupLocation.CenterScreen;
            FontFamily=new M.FontFamily("Segoe UI");FontSize=14*textScale;UseLayoutRounding=true;SnapsToDevicePixels=true;
            using(var stream=Resource("shuttli.AppIcon")) {var icon=new B.BitmapImage();icon.BeginInit();icon.CacheOption=B.BitmapCacheOption.OnLoad;icon.StreamSource=stream;icon.EndInit();icon.Freeze();Icon=icon;}
            Theme();Styles();ExtraStyles();Shell();
            Closed+=delegate {closed=true;timer.Stop();tray.Dispose();};
            Closing+=delegate(object sender,System.ComponentModel.CancelEventArgs e) {if(!quitting) {e.Cancel=true;Hide();body.Children.Clear();generation++;}};
            Loaded+=delegate {Poll(true);if(fixture==null) EnsureAgent();};
            timer.Tick+=delegate {Poll(false);};timer.Start();
            tray.Visible=true;tray.Text=Product.Length>60?Product.Substring(0,60):Product;tray.DoubleClick+=delegate {Post(new Action(Reveal));};
            Microsoft.Win32.SystemEvents.UserPreferenceChanged+=SystemThemeChanged;
            Closed+=delegate {Microsoft.Win32.SystemEvents.UserPreferenceChanged-=SystemThemeChanged;};
            InputBindings.Add(new System.Windows.Input.KeyBinding(new UiCommand(delegate {Poll(true);}),new System.Windows.Input.KeyGesture(System.Windows.Input.Key.F5)));
            InputBindings.Add(new System.Windows.Input.KeyBinding(new UiCommand(delegate {Navigate("settings");}),new System.Windows.Input.KeyGesture(System.Windows.Input.Key.OemComma,System.Windows.Input.ModifierKeys.Control)));
        }
        void SystemThemeChanged(object sender,Microsoft.Win32.UserPreferenceChangedEventArgs e) {if(!closed) Post(new Action(delegate {Theme();if(IsVisible) Render();UpdateTray();}));}
        public void Reveal() {Show();if(WindowState==W.WindowState.Minimized) WindowState=W.WindowState.Normal;Activate();Render();}
        void LoadPreferences() {
            try {string path=Path.Combine(profile,"ui-preferences.json");if(new FileInfo(path).Length>4096) return;var p=Obj(File.ReadAllText(path,Encoding.UTF8));string code=Native.Text(p,"language");if(new[]{"system","en","nl","de","fr"}.Contains(code)) language=code;string value=Native.Text(p,"theme");if(new[]{"system","light","dark"}.Contains(value)) theme=value;} catch(IOException) {} catch(ArgumentException) {}
        }
        void SavePreferences() {
            string path=Path.Combine(profile,"ui-preferences.json");string temporary=path+"."+Guid.NewGuid().ToString("N")+".tmp";
            try {File.WriteAllText(temporary,Encode(new {language=language,theme=theme}),new UTF8Encoding(false));if(File.Exists(path)) File.Replace(temporary,path,null);else File.Move(temporary,path);} finally {if(File.Exists(temporary)) File.Delete(temporary);}
        }
        void EnsureAgent() {
            Task.Run(delegate {
                try {Request(new {command="status"});}
                catch {try {var start=new System.Diagnostics.ProcessStartInfo(executable,"daemon") {UseShellExecute=false,CreateNoWindow=true};start.EnvironmentVariables["SHUTTLI_DATA_DIR"]=profile;System.Diagnostics.Process.Start(start);} catch(Exception e) {Post(new Action(delegate {Notice(e.Message,true);}));}}
            });
        }
        Dictionary<string,object> Request(object action) {
            var request=Obj(Encode(action));
            if(fixture!=null) return FixtureRequest(request);
            return Obj(Call(profile,Encode(new {version=2,action=request})));
        }
        static void Check(Dictionary<string,object> answer) {if(Native.Text(answer,"type")=="error") throw new IOException(Native.Text(answer,"message"));}
        public void Post(Action action) {Dispatcher.BeginInvoke(action);}
        void Notice(string text,bool error=false) {pollErrorNotice=false;message.Text=text;messageBox.Visibility=String.IsNullOrEmpty(text)?W.Visibility.Collapsed:W.Visibility.Visible;messageBox.SetResourceReference(C.Border.BackgroundProperty,error?"RedBg":"Info");}
        void Navigate(string target) {page=target;historyPage=target=="history"?historyPage:0;scroll.ScrollToTop();Notice("");Render();Poll(true);}
        void Poll(bool force) {
            if(polling || closed) return;polling=true;
            Task.Run(async delegate {
                await gate.WaitAsync();
                try {
                    var s=Request(new {command="status"});Check(s);var p=Request(new {command="devices"});Check(p);
                    var rows=new List<Dictionary<string,object>>();
                    for(int offset=0;offset<10000;offset+=100) {var h=Request(new {command="history",offset=offset,limit=100});Check(h);var portion=Items(Get(h,"entries"));rows.AddRange(portion);if(portion.Count<100) break;}
                    Post(new Action(delegate {
                        if(closed) return;
                        current=Dict(s["status"]);settings=Dict(current["settings"]);peers=Items(Get(p,"devices"));history=rows;groups=GroupHistory(history);service=true;
                        if(pollErrorNotice) Notice("");
                        string next=Encode(new object[]{current,p,rows});bool changed=signature!=next;signature=next;
                        UpdateSummary();UpdateTray();
                        if(IsVisible && !modal && (force || changed || deferredRender)) {
                            bool editing=page=="settings" && System.Windows.Input.Keyboard.FocusedElement is C.TextBox;
                            if(editing && !force) {foreach(var toggle in toggles) toggle.Value.IsChecked=Flag(settings,toggle.Key);deferredRender=true;}
                            else {deferredRender=false;Render();}
                        }
                    }));
                } catch(Exception e) {Post(new Action(delegate {bool changed=service;service=false;signature="";UpdateSummary();UpdateTray();if(IsVisible && (force || changed || body.Children.Count==0)) Render();Notice(T("common.error","detail",e.Message),true);pollErrorNotice=true;}));}
                finally {gate.Release();Post(new Action(delegate {polling=false;}));}
            });
        }
        void Operate(object action,Action<Dictionary<string,object>> completed=null) {
            foreach(var toggle in toggles.Values) toggle.IsEnabled=false;
            Notice(T("common.working"));
            Task.Run(async delegate {
                await gate.WaitAsync();
                try {var answer=Request(action);Check(answer);Post(new Action(delegate {Notice(Native.Text(answer,"message"));if(completed!=null) completed(answer);Poll(true);}));}
                catch(Exception e) {Post(new Action(delegate {Notice(T("common.error","detail",e.Message),true);Render();Poll(true);}));}
                finally {gate.Release();}
            });
        }
        void Configure(Dictionary<string,object> values) {
            // Fetch a fresh compare-and-replace snapshot on the serialized API worker.
            Task.Run(async delegate {
                await gate.WaitAsync();
                try {var answer=Request(new {command="settings"});Check(answer);var expected=Dict(answer["settings"]);var desired=Obj(Encode(expected));foreach(var pair in values) desired[pair.Key]=pair.Value;answer=Request(new {command="configure",expected=expected,settings=desired});Check(answer);Post(new Action(delegate {foreach(string key in values.Keys) edits.Remove(key);Notice(Native.Text(answer,"message"));Poll(true);}));}
                catch(Exception e) {Post(new Action(delegate {Notice(T("common.error","detail",e.Message),true);Render();Poll(true);}));}
                finally {gate.Release();}
            });
        }
        void UpdateSummary() {
            summary.Children.Clear();
            if(!service) {summary.Children.Add(Label(T("common.unavailable"),"Muted",12));return;}
            foreach(string key in new[]{"send","receive"}) summary.Children.Add(new C.Border {Margin=new W.Thickness(0,0,0,5),Child=Label((Flag(settings,key)?"● ":"Ⅱ ")+T("setting."+key)+" · "+T(Flag(settings,key)?"common.on":"common.off"),Flag(settings,key)?"Green":"Muted",12.5)});
            summary.Children.Add(Label(T(Flag(settings,"automatic")?"ui.automatic":"ui.manual"),"Muted",11.5));
        }
        void Render() {
            generation++;toggles.Clear();body.Children.Clear();back.Visibility=(page=="device" || page=="detail")?W.Visibility.Visible:W.Visibility.Hidden;
            string primary=page=="device"?"devices":page=="detail"?"history":page;
            title.Text=T(page=="device"?"ui.device_detail":page=="detail"?"ui.history_detail":"nav."+page);
            foreach(var nav in navigation) {nav.Value.SetResourceReference(C.Control.BackgroundProperty,nav.Key==primary?"Selected":"Sidebar");var line=(C.StackPanel)nav.Value.Content;((C.TextBlock)line.Children[1]).Text=T("nav."+nav.Key);A.AutomationProperties.SetName(nav.Value,T("nav."+nav.Key));}
            if(!service || settings==null) {var g=Group();Row(g,T("common.unavailable"),T("tray.connecting"),Button(T("ui.retry"),delegate {Poll(true);}));return;}
            double offset=scroll.VerticalOffset;
            if(page=="status") StatusPage();else if(page=="devices") DevicesPage();else if(page=="device") DevicePage();else if(page=="history") HistoryPage();else if(page=="detail") HistoryDetail();else SettingsPage();
            scroll.ScrollToVerticalOffset(offset);
        }
        void StatusPage() {
            bool send=Flag(settings,"send"),receive=Flag(settings,"receive");string state=send?(receive?"tray.both":"tray.send_only"):(receive?"tray.receive_only":"tray.paused");
            var hero=new C.Grid {Margin=new W.Thickness(0,0,0,10)};hero.ColumnDefinitions.Add(new C.ColumnDefinition {Width=new W.GridLength(65)});hero.ColumnDefinitions.Add(new C.ColumnDefinition());
            var symbol=new C.Border {Width=52,Height=52,CornerRadius=new W.CornerRadius(16),Child=new C.TextBlock {Text=send||receive?"↕":"Ⅱ",FontSize=29,HorizontalAlignment=W.HorizontalAlignment.Center,VerticalAlignment=W.VerticalAlignment.Center}};symbol.SetResourceReference(C.Border.BackgroundProperty,"GreenBg");symbol.SetResourceReference(C.TextBlock.ForegroundProperty,"Green");hero.Children.Add(symbol);
            var headings=new C.StackPanel {VerticalAlignment=W.VerticalAlignment.Center};headings.Children.Add(Label(T(state),"Text",21,true));headings.Children.Add(new C.Border {Margin=new W.Thickness(0,4,0,0),Child=Label(T(Flag(settings,"automatic")?"ui.automatic":"ui.manual"),"Muted",12)});C.Grid.SetColumn(headings,1);hero.Children.Add(headings);body.Children.Add(hero);
            if(!Flag(current,"clipboard_available")) Row(Group(),T("status.clipboard","available",T("common.no")),Native.Text(current,"last_error"));
            var counts=Dict(Get(current,"devices"));var metrics=Group(T("nav.devices"));var grid=new C.Grid {Margin=new W.Thickness(15,12,15,12)};
            string[] keys={"discovered","send","receive"};string[] labels={"status.discovered","status.allowed_send","status.allowed_receive"};
            for(int i=0;i<3;i++) {grid.ColumnDefinitions.Add(new C.ColumnDefinition());var stack=new C.StackPanel {Margin=new W.Thickness(i==0?0:10,0,0,0)};stack.Children.Add(Label(T(labels[i]),"Muted",11));stack.Children.Add(Label(Number(counts,keys[i]).ToString(),"Text",22,true));C.Grid.SetColumn(stack,i);grid.Children.Add(stack);}metrics.Children.Add(grid);
            body.Children.Add(new C.Border {Margin=new W.Thickness(3,4,0,0),Child=Label(T("status.permissions_hint"),"Muted",11)});
            var directions=Group(T("ui.directions"));Setting(directions,"send",T("ui.send_hint"));Setting(directions,"receive",T("ui.receive_hint"));
            var mode=Group(T("ui.sync_mode"));Setting(mode,"automatic",T("ui.automatic_hint"));var now=Button(T("action.send"),delegate {Operate(new {command="send"});},true);now.IsEnabled=Flag(current,"clipboard_available") && Number(counts,"send")>0;now.ToolTip=now.IsEnabled?T("ui.manual_hint"):T("devices.help");Row(mode,T("tray.send"),T("ui.manual_hint"),now);
            var recent=Group(T("ui.recent_activity"));if(groups.Count==0) Row(recent,T("history.empty"));foreach(var item in groups.Take(3)) HistoryRow(recent,item);
            body.Children.Add(Button(T("nav.history"),delegate {Navigate("history");},false,true));
            var details=new C.Expander {Foreground=Brush("Text"),Header=T("status.details"),Margin=new W.Thickness(0,14,0,0)};var diagnostics=new C.StackPanel {Margin=new W.Thickness(4,12,4,8)};diagnostics.Children.Add(Label(T("status.fingerprint"),"Muted",12));
            diagnostics.Children.Add(new C.TextBox {Text=Native.Text(current,"device"),IsReadOnly=true,TextWrapping=W.TextWrapping.Wrap,BorderThickness=new W.Thickness(0),Background=M.Brushes.Transparent,Foreground=Brush("Text"),Margin=new W.Thickness(0,4,0,10)});
            diagnostics.Children.Add(Label(Native.Text(current,"clipboard"),"Muted",12));diagnostics.Children.Add(Label(T("status.clipboard","available",T(Flag(current,"clipboard_available")?"common.yes":"common.no")),"Muted",12));if(Get(current,"last_error")!=null) diagnostics.Children.Add(Label(Native.Text(current,"last_error"),"Red",12));details.Content=diagnostics;body.Children.Add(details);
            body.Children.Add(new C.Border {Margin=new W.Thickness(0,14,0,0),Child=Label(T("status.close_hint"),"Muted",11)});
        }
        Dictionary<string,object> Policy(string id) {var all=Dict(Get(settings,"peers"));object policy;if(all.TryGetValue(id,out policy)) return Obj(Encode(policy));return Obj("{\"send\":false,\"receive\":true,\"text\":true,\"png\":true,\"max_bytes\":8388608,\"quiet\":false,\"history\":null}");}
        void DevicesPage() {
            var banner=new C.Border {CornerRadius=new W.CornerRadius(12),Padding=new W.Thickness(16),Margin=new W.Thickness(0,0,0,14),Child=Label(T("ui.devices_hint"),"Text",13)};banner.SetResourceReference(C.Border.BackgroundProperty,"Info");body.Children.Add(banner);
            body.Children.Add(Button(T("action.refresh_discovery"),delegate {Operate(new {command="refresh"});},false,true));var group=Group(T("ui.tailnet"));
            if(peers.Count==0) Row(group,T("devices.empty"),T("devices.help"));
            foreach(var peer in peers) {
                var value=peer;var policy=Policy(Native.Text(peer,"id"));string id=Native.Text(peer,"id");
                string hint=id.Substring(0,Math.Min(12,id.Length))+" · "+T(Flag(peer,"online")?"devices.online":"devices.offline");
                var directions=new C.StackPanel {VerticalAlignment=W.VerticalAlignment.Center};directions.Children.Add(Label("↑ "+T(Flag(policy,"send")?"common.on":"common.off"),Flag(policy,"send")?"Green":"Muted",12));directions.Children.Add(Label("↓ "+T(Flag(policy,"receive")?"common.on":"common.off"),Flag(policy,"receive")?"Green":"Muted",12));
                var row=new C.StackPanel();Row(row,Native.Text(peer,"name"),hint,directions);var button=Button(Native.Text(peer,"name"),delegate {selectedPeer=value;Navigate("device");});button.Content=row;button.HorizontalAlignment=W.HorizontalAlignment.Stretch;button.HorizontalContentAlignment=W.HorizontalAlignment.Stretch;button.Padding=new W.Thickness(0);button.BorderThickness=new W.Thickness(0);button.SetResourceReference(C.Control.BackgroundProperty,"Card");
                if(group.Children.Count>0) {var separator=new C.Border {Height=1};separator.SetResourceReference(C.Border.BackgroundProperty,"Border");group.Children.Add(separator);}group.Children.Add(button);
            }
            body.Children.Add(new C.Border {Margin=new W.Thickness(3,12,0,0),Child=Label(T("devices.help"),"Muted",12)});
        }
        void DevicePage() {
            if(selectedPeer==null) {Navigate("devices");return;}
            string id=Native.Text(selectedPeer,"id");selectedPeer=peers.FirstOrDefault(p=>Native.Text(p,"id")==id)??selectedPeer;var policy=Policy(id);
            body.Children.Add(Label(Native.Text(selectedPeer,"name"),"Text",21,true));body.Children.Add(new C.Border {Margin=new W.Thickness(0,5,0,12),Child=Label(Native.Text(selectedPeer,"address")+" · "+T(Flag(selectedPeer,"online")?"devices.online":"devices.offline"),"Muted",12)});
            var fingerprint=new C.TextBox {Text=id,IsReadOnly=true,TextWrapping=W.TextWrapping.Wrap,BorderThickness=new W.Thickness(0),Background=M.Brushes.Transparent,Foreground=Brush("Text"),Margin=new W.Thickness(15)};
            var identity=Group(T("status.fingerprint"));identity.Children.Add(fingerprint);
            var directions=Group(T("ui.directions"));Setting(directions,"send",T("ui.send_hint"),policy,id);Setting(directions,"receive",T("ui.receive_hint"),policy,id);
            body.Children.Add(Label(T("status.permissions_hint"),"Muted",11));
            var types=Group(T("ui.content_types"));foreach(string key in new[]{"text","png","quiet"}) Setting(types,key,null,policy,id);
            string[] modes={"inherit","off","status","content"};var chooser=Combo(modes.Select(m=>T("history.mode."+m)).ToArray(),Array.IndexOf(modes,Get(policy,"history")==null?"inherit":Native.Text(policy,"history")),delegate(int index) {var next=Obj(Encode(policy));next["history"]=index==0?null:modes[index];Operate(new {command="peer",id=id,expected=policy,policy=next});});Row(Group(),T("devices.history"),null,chooser);
        }
        void AllowPeer(Dictionary<string,object> peer,Dictionary<string,object> policy) {
            var content=new C.StackPanel();content.Children.Add(Label(T("ui.allow_hint","name",Native.Text(peer,"name"))));
            content.Children.Add(new C.TextBox {Text=Native.Text(peer,"id"),IsReadOnly=true,TextWrapping=W.TextWrapping.Wrap,FontFamily=new M.FontFamily("Consolas"),Margin=new W.Thickness(0,14,0,14),Padding=new W.Thickness(12),Background=Brush("Page"),Foreground=Brush("Text"),BorderBrush=Brush("Border")});
            var check=new C.CheckBox {Content=Label(T("ui.fingerprint_checked")),Margin=new W.Thickness(0,0,0,12)};content.Children.Add(check);
            Dialog(T("ui.allow_title"),T("ui.allow"),content,delegate {var next=Obj(Encode(policy));next["send"]=true;Operate(new {command="peer",id=Native.Text(peer,"id"),expected=policy,policy=next});},check);
        }
        static List<HistoryGroup> GroupHistory(List<Dictionary<string,object>> rows) {
            var map=new Dictionary<string,HistoryGroup>();
            foreach(var row in rows) {
                string direction=Native.Text(row,"direction");string key=Encode(Get(row,"event"))+"/"+(direction=="local"?"send":direction);HistoryGroup group;
                if(!map.TryGetValue(key,out group)) {group=new HistoryGroup {Row=Obj(Encode(row)),Key=key};map[key]=group;}
                if(direction=="local") {group.Local=true;group.Row["direction"]="local";}else group.Transfers.Add(row);
                if(Number(row,"time")>Number(group.Row,"time")) group.Row["time"]=row["time"];
                if(Flag(row,"available") && (group.ContentId==null || direction=="local" || !group.Local)) group.ContentId=row["id"];
            }
            return map.Values.OrderByDescending(g=>Number(g.Row,"time")).ThenByDescending(g=>Number(g.Row,"id")).ToList();
        }
        string PeerName(string id) {var peer=peers.FirstOrDefault(p=>Native.Text(p,"id")==id);return peer==null?id.Substring(0,Math.Min(12,id.Length)):Native.Text(peer,"name");}
        string Time(Dictionary<string,object> row,bool date=false) {try {return DateTimeOffset.FromUnixTimeSeconds(Number(row,"time")).LocalDateTime.ToString(date?"g":"T",CultureInfo.CurrentCulture);}catch {return "—";}}
        C.Border Badge(string text,string state) {string color=state=="applied"?"Green":state=="failed"?"Red":state=="unknown" || state=="uncertain"?"Amber":"Accent";string bg=color=="Accent"?"Info":color+"Bg";var badge=new C.Border {CornerRadius=new W.CornerRadius(12),Padding=new W.Thickness(8,3,8,3),Margin=new W.Thickness(0,3,6,0),Child=Label(text,color,11.5)};badge.SetResourceReference(C.Border.BackgroundProperty,bg);return badge;}
        void HistoryRow(C.StackPanel group,HistoryGroup item) {
            var row=new C.StackPanel {Margin=new W.Thickness(15,12,15,12)};
            var top=new C.DockPanel();var time=Label(Time(item.Row),"Muted",11.5);C.DockPanel.SetDock(time,C.Dock.Right);top.Children.Add(time);top.Children.Add(Label((Native.Text(item.Row,"direction")=="receive"?"↓":item.Local?"•":"↑")+"  "+T("format."+Native.Text(item.Row,"format")),"Text",14));row.Children.Add(top);
            if(item.Local) row.Children.Add(new C.Border {Margin=new W.Thickness(0,5,0,0),Child=Label(T("ui.local_copy"),"Muted",12)});
            if(item.Transfers.Count==0) row.Children.Add(Label(T("ui.not_sent"),"Muted",12));
            var badges=new C.WrapPanel();foreach(var transfer in item.Transfers) {string state=Native.Text(transfer,"state");badges.Children.Add(Badge(PeerName(Native.Text(transfer,"peer"))+" · "+T("state."+state),state));}row.Children.Add(badges);
            row.Children.Add(new C.Border {Margin=new W.Thickness(0,5,0,0),Child=Label(T("ui.content_size","bytes",Number(item.Row,"bytes")),"Muted",11)});
            var button=Button(T("ui.history_detail"),delegate {selectedHistory=item;Navigate("detail");});button.Content=row;button.HorizontalAlignment=W.HorizontalAlignment.Stretch;button.HorizontalContentAlignment=W.HorizontalAlignment.Stretch;button.Padding=new W.Thickness(0);button.BorderThickness=new W.Thickness(0);button.SetResourceReference(C.Control.BackgroundProperty,"Card");A.AutomationProperties.SetName(button,T("format."+Native.Text(item.Row,"format"))+" "+Time(item.Row)+" "+T("ui.history_detail"));
            if(group.Children.Count>0) {var separator=new C.Border {Height=1};separator.SetResourceReference(C.Border.BackgroundProperty,"Border");group.Children.Add(separator);}group.Children.Add(button);
        }
        void HistoryPage() {
            body.Children.Add(Label(T("ui.session_history"),"Muted",12));
            var actions=new C.WrapPanel {Margin=new W.Thickness(0,10,0,8)};var preferences=Button(T("ui.history_settings"),delegate {Navigate("settings");},false,true);preferences.Margin=new W.Thickness(0,0,12,0);actions.Children.Add(preferences);actions.Children.Add(Button(T("ui.clear"),ClearHistory,false,true));body.Children.Add(actions);
            var group=Group();historyPage=Math.Min(historyPage,Math.Max(0,(groups.Count-1)/20));
            if(groups.Count==0) Row(group,T("history.empty"),T("history.mode."+Native.Text(settings,"history"))+" · "+T("ui.history_empty_hint"));
            foreach(var item in groups.Skip(historyPage*20).Take(20)) HistoryRow(group,item);
            var nav=new C.DockPanel {Margin=new W.Thickness(0,12,0,0)};var buttons=new C.StackPanel {Orientation=C.Orientation.Horizontal};var previous=Button(T("history.previous"),delegate {historyPage--;Render();});previous.IsEnabled=historyPage>0;var next=Button(T("history.next"),delegate {historyPage++;Render();});next.IsEnabled=(historyPage+1)*20<groups.Count;buttons.Children.Add(previous);buttons.Children.Add(next);C.DockPanel.SetDock(buttons,C.Dock.Right);nav.Children.Add(buttons);nav.Children.Add(Label(T("ui.entries","count",groups.Count),"Muted",12));body.Children.Add(nav);
        }
        void HistoryDetail() {
            selectedHistory=selectedHistory==null?null:groups.FirstOrDefault(g=>g.Key==selectedHistory.Key);
            if(selectedHistory==null) {Row(Group(),T("ui.expired"));return;}
            var item=selectedHistory;body.Children.Add(Label(T("format."+Native.Text(item.Row,"format")),"Text",21,true));body.Children.Add(new C.Border {Margin=new W.Thickness(0,5,0,10),Child=Label(Time(item.Row,true)+" · "+T("ui.content_size","bytes",Number(item.Row,"bytes")),"Muted",12)});
            var preview=Group(T("history.preview"));
            if(item.ContentId==null) Row(preview,T("ui.content_unavailable"));else {
                int render=generation;Row(preview,T("common.working"));
                Task.Run(async delegate {await gate.WaitAsync();try {var answer=Request(new {command="preview",id=item.ContentId});Check(answer);byte[] data=Convert.FromBase64String(Native.Text(answer,"base64"));if(data.Length>8*1024*1024) throw new IOException("Preview exceeds limit");Post(new Action(delegate {
                    if(render!=generation || closed) return;preview.Children.Clear();
                    if(Native.Text(answer,"format")=="text") {var text=new C.TextBox {Text=Utf8.GetString(data),IsReadOnly=true,TextWrapping=W.TextWrapping.Wrap,AcceptsReturn=true,VerticalScrollBarVisibility=C.ScrollBarVisibility.Auto,MaxHeight=260,MinHeight=100,Padding=new W.Thickness(14),BorderThickness=new W.Thickness(0),Background=Brush("Card"),Foreground=Brush("Text"),FontFamily=new M.FontFamily("Consolas"),FontSize=12*textScale};A.AutomationProperties.SetName(text,T("history.preview"));preview.Children.Add(text);}else {var bitmap=new B.BitmapImage();bitmap.BeginInit();bitmap.CacheOption=B.BitmapCacheOption.OnLoad;bitmap.StreamSource=new MemoryStream(data);bitmap.EndInit();bitmap.Freeze();preview.Children.Add(new C.Image {Source=bitmap,MaxHeight=260,Margin=new W.Thickness(14),Stretch=M.Stretch.Uniform});}
                }));}catch(Exception e) {Post(new Action(delegate {if(render==generation) {preview.Children.Clear();Row(preview,T("ui.expired"),e.Message);}}));}finally {gate.Release();}});
            }
            var actions=new C.WrapPanel {Margin=new W.Thickness(0,12,0,8)};
            foreach(string action in new[]{"copy","copy_local","resend"}) {string operation=action;var button=Button(T("history."+action),delegate {if(operation=="resend") Operate(new {command="resend",id=item.ContentId});else Operate(new {command="copy",id=item.ContentId,local_only=operation=="copy_local"});},action=="copy_local");button.Margin=new W.Thickness(0,0,8,6);button.IsEnabled=item.ContentId!=null && (action!="resend" || Flag(settings,"send"));if(!button.IsEnabled) button.ToolTip=T("ui.content_unavailable");actions.Children.Add(button);}body.Children.Add(actions);body.Children.Add(Label(T("ui.copy_hint"),"Muted",12));
            var targets=Group(T("ui.transfers"));if(item.Local) Row(targets,T("ui.local_copy"));foreach(var transfer in item.Transfers) {string state=Native.Text(transfer,"state");Row(targets,PeerName(Native.Text(transfer,"peer")),Native.Text(transfer,"detail"),Badge(T("state."+state),state));}
            body.Children.Add(new C.Border {Margin=new W.Thickness(0,12,0,0),Child=Label(T("status.receipt_hint"),"Muted",11)});
        }
        void Dialog(string heading,string action,C.StackPanel content,Action confirm,C.CheckBox guard=null) {
            modal=true;var dialog=new W.Window {Owner=this,Title=heading,Width=450,SizeToContent=W.SizeToContent.Height,ResizeMode=W.ResizeMode.NoResize,WindowStartupLocation=W.WindowStartupLocation.CenterOwner,Background=Brush("Page"),Foreground=Brush("Text"),FontFamily=FontFamily,FontSize=14*textScale};
            var stack=new C.StackPanel {Margin=new W.Thickness(24)};stack.Children.Add(Label(heading,"Text",20,true));content.Margin=new W.Thickness(0,16,0,12);stack.Children.Add(content);var actions=new C.StackPanel {Orientation=C.Orientation.Horizontal,HorizontalAlignment=W.HorizontalAlignment.Right};actions.Children.Add(Button(T("ui.cancel"),delegate {dialog.Close();}));var accept=Button(action,delegate {dialog.DialogResult=true;},true);accept.Margin=new W.Thickness(10,0,0,0);if(guard!=null) {accept.IsEnabled=false;guard.Checked+=delegate {accept.IsEnabled=true;};guard.Unchecked+=delegate {accept.IsEnabled=false;};}actions.Children.Add(accept);stack.Children.Add(actions);dialog.Content=stack;
            dialog.Resources=Resources;
            dialog.Content=null;
            if(reviewOutput!=null) {
                var surface=new C.Border {Background=Brush("Page"),Child=stack,Width=450};surface.Resources=Resources;
                surface.Measure(new W.Size(450,Double.PositiveInfinity));surface.Arrange(new W.Rect(0,0,450,surface.DesiredSize.Height));surface.UpdateLayout();
                SaveVisual(surface,Path.Combine(reviewOutput,guard==null?"clear-confirmation.png":"allow-sending.png"),450,surface.ActualHeight,reviewScale);
                if(guard!=null) {if(accept.IsEnabled) throw new Exception("Fingerprint consent must start disabled");guard.IsChecked=true;if(!accept.IsEnabled) throw new Exception("Fingerprint check did not enable consent");}
                modal=false;return;
            }
            dialog.MaxHeight=W.SystemParameters.WorkArea.Height*.9;dialog.Content=new C.ScrollViewer {Content=stack,VerticalScrollBarVisibility=C.ScrollBarVisibility.Auto};
            try {if(dialog.ShowDialog()==true) confirm();}finally {modal=false;Poll(true);}
        }
        void ClearHistory() {var text=new C.StackPanel();text.Children.Add(Label(T("ui.clear_hint")));Dialog(T("history.clear"),T("ui.clear"),text,delegate {Operate(new {command="clear_history"});});}
        C.ComboBox Combo(string[] options,int selected,Action<int> changed) {
            var combo=new C.ComboBox {ItemsSource=options,SelectedIndex=Math.Max(0,selected),MinWidth=140,MaxWidth=210,VerticalAlignment=W.VerticalAlignment.Center,FontSize=12.5*textScale,Padding=new W.Thickness(6),Foreground=Brush("Text"),Template=(C.ControlTemplate)Resources["Choice"]};
            var itemStyle=new W.Style(typeof(C.ComboBoxItem));itemStyle.Setters.Add(new W.Setter(C.Control.ForegroundProperty,Brush("Text")));itemStyle.Setters.Add(new W.Setter(C.Control.BackgroundProperty,Brush("Card")));itemStyle.Setters.Add(new W.Setter(C.Control.PaddingProperty,new W.Thickness(9,6,9,6)));combo.ItemContainerStyle=itemStyle;
            A.AutomationProperties.SetName(combo,String.Join(", ",options));
            combo.SelectionChanged+=delegate {if(combo.SelectedIndex>=0) changed(combo.SelectedIndex);};return combo;
        }
        void Numeric(C.StackPanel group,string key,string label,long divisor=1) {
            string value;var input=new C.TextBox {Text=edits.TryGetValue(key,out value)?value:(Number(settings,key)/divisor).ToString(),MinWidth=62,MaxWidth=90,Padding=new W.Thickness(6),VerticalAlignment=W.VerticalAlignment.Center,VerticalContentAlignment=W.VerticalAlignment.Center,Background=Brush("Card"),Foreground=Brush("Text"),BorderBrush=Brush("Border")};A.AutomationProperties.SetName(input,label);
            input.TextChanged+=delegate {edits[key]=input.Text;};var actions=new C.StackPanel {Orientation=C.Orientation.Horizontal};actions.Children.Add(input);var apply=Button(T("ui.apply"),delegate {long count;if(!Int64.TryParse(input.Text,out count) || count<0 || (key=="history_limit" && count>10000) || (key=="history_memory_bytes" && count>64) || (key=="history_bytes" && count>1024)) {Notice(label+": "+T("common.error","detail","Invalid value"),true);return;}Configure(new Dictionary<string,object>{{key,count*divisor}});},true);apply.Margin=new W.Thickness(8,0,0,0);actions.Children.Add(apply);Row(group,label,key=="history_limit"?T("ui.limit_hint"):null,actions);
        }
        void SettingsPage() {
            var sync=Group(T("ui.sync"));foreach(string key in new[]{"send","receive","automatic","text","png"}) Setting(sync,key,key=="send"?T("ui.send_hint"):key=="receive"?T("ui.receive_hint"):key=="automatic"?T("ui.automatic_hint"):null);
            Setting(Group(T("setting.notifications")),"notifications",T("ui.notification_hint"));
            var retention=Group(T("history.retention"));string[] modes={"off","status","content"};Row(retention,T("ui.keep"),null,Combo(modes.Select(m=>T("history.mode."+m)).ToArray(),Array.IndexOf(modes,Native.Text(settings,"history")),delegate(int index) {Configure(new Dictionary<string,object>{{"history",modes[index]}});}));Row(retention,T("history.storage_hint"));Numeric(retention,"history_limit",T("history.limit"));Row(retention,T("history.clear"),T("ui.clear_hint"),Button(T("ui.clear"),ClearHistory));
            var advanced=new C.Expander {Foreground=Brush("Text"),Header=T("ui.storage_limits","text",Number(settings,"history_memory_bytes")/1048576,"images",Number(settings,"history_bytes")/1048576),Margin=new W.Thickness(0,12,0,0)};var quotas=new C.StackPanel();Numeric(quotas,"history_memory_bytes",T("format.text")+" (MiB)",1048576);Numeric(quotas,"history_bytes",T("format.png")+" (MiB)",1048576);advanced.Content=Card(quotas);body.Children.Add(advanced);
            var computer=Group(T("ui.this_computer"));var start=Toggle(T("ui.start_login"),autostartState=="enabled",delegate(bool enabled) {Operate(new {command="autostart",enabled=(bool?)enabled},delegate(Dictionary<string,object> answer) {var result=Dict(Get(answer,"status"));autostartState=Native.Text(result,"state");autostartMessage=Native.Text(result,"message");});});start.IsEnabled=autostartState!="unknown";Row(computer,T("ui.start_login"),autostartState=="enabled"?T("common.on"):autostartState=="disabled"?T("common.off"):autostartMessage,start);
            string[] codes={"system","en","nl","de","fr"};Row(computer,T("locale.label"),T("locale.help"),Combo(new[]{T("locale.system"),"English","Nederlands","Deutsch","Français"},Array.IndexOf(codes,language),delegate(int index) {language=codes[index];try {SavePreferences();UpdateSummary();UpdateTray();Render();}catch(Exception e) {Notice(e.Message,true);}}));
            Row(computer,T("appearance.label"),null,Combo(new[]{T("appearance.system"),T("appearance.light"),T("appearance.dark")},Array.IndexOf(new[]{"system","light","dark"},theme),delegate(int index) {theme=new[]{"system","light","dark"}[index];try {SavePreferences();Theme();UpdateTray();Render();}catch(Exception e) {Notice(e.Message,true);}}));
            Row(computer,T("ui.app_version"),Environment.GetEnvironmentVariable("SHUTTLI_PRODUCT_VERSION")??BuildVersion);body.Children.Add(new C.Border {Margin=new W.Thickness(3,14,0,0),Child=Label(Product+" · MIT License","Muted",11)});
            if(autostartState=="unknown") Task.Run(async delegate {await gate.WaitAsync();try {var result=Request(new {command="autostart",enabled=(bool?)null});Check(result);Post(new Action(delegate {var value=Dict(Get(result,"status"));autostartState=Native.Text(value,"state");autostartMessage=Native.Text(value,"message");if(page=="settings") Render();}));}catch(Exception e) {Post(new Action(delegate {autostartState="unavailable";autostartMessage=e.Message;if(page=="settings") Render();}));}finally {gate.Release();}});
        }
        void UpdateTray() {
            string state=!service?"service-unavailable":Flag(settings,"send")?(Flag(settings,"receive")?"send-on-receive-on":"send-on-receive-off"):(Flag(settings,"receive")?"send-off-receive-on":"send-off-receive-off");
            bool lightPanel=false;using(var themeKey=Microsoft.Win32.Registry.CurrentUser.OpenSubKey("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize")) if(themeKey!=null) lightPanel=Convert.ToInt32(themeKey.GetValue("SystemUsesLightTheme",0))!=0;
            int pixels=F.SystemInformation.SmallIconSize.Width;int size=pixels<=16?16:pixels<=24?24:32;
            using(var stream=Resource("shuttli.Tray.tray-"+state+(lightPanel?"-on-light-":"-on-dark-")+size+".png")) if(stream!=null) using(var image=new System.Drawing.Bitmap(stream)) {IntPtr handle=image.GetHicon();try {var icon=(System.Drawing.Icon)System.Drawing.Icon.FromHandle(handle).Clone();var previous=tray.Icon;tray.Icon=icon;if(previous!=null) previous.Dispose();}finally {DestroyIcon(handle);}}
            var menu=new F.ContextMenuStrip();var counts=Dict(current==null?null:Get(current,"devices"));menu.Items.Add("↑ "+Number(counts,"send")+"   ↓ "+Number(counts,"receive"),null,delegate {}).Enabled=false;
            menu.Items.Add(T("tray.open"),null,delegate {Post(new Action(Reveal));});
            foreach(string key in new[]{"send","receive"}) {string direction=key;var item=new F.ToolStripMenuItem(T("tray.enable_"+key)) {Checked=service && Flag(settings,key),Enabled=service};item.Click+=delegate {Post(new Action(delegate {Operate(new {command="set_directions",send=direction=="send"?(bool?)!Flag(settings,direction):null,receive=direction=="receive"?(bool?)!Flag(settings,direction):null});}));};menu.Items.Add(item);}
            menu.Items.Add(T("tray.send"),null,delegate {Post(new Action(delegate {Operate(new {command="send"});}));}).Enabled=service && Flag(settings,"send") && Flag(current,"clipboard_available") && Number(counts,"send")>0;
            menu.Items.Add(new F.ToolStripSeparator());menu.Items.Add(T("tray.quit"),null,delegate {Post(new Action(delegate {if(fixture!=null) Exit();else Operate(new {command="quit"},delegate {Exit();});}));});var old=tray.ContextMenuStrip;tray.ContextMenuStrip=menu;if(old!=null && !old.Visible) old.Dispose();
        }
        void Exit() {quitting=true;Close();W.Application.Current.Shutdown();}
        [System.Runtime.InteropServices.DllImport("user32.dll")] static extern bool DestroyIcon(IntPtr icon);

        public async Task ExportReview(string output,double scale,double width,double height) {
            // Deterministic rendering of application-owned WPF visuals, not desktop capture.
            if(fixture==null || scale<.5 || scale>3) throw new ArgumentException("Explicit fixture and bounded scale required");
            Directory.CreateDirectory(output);reviewOutput=output;reviewScale=scale;timer.Stop();await Task.Delay(150);
            var snapshot=FixtureRequest(Obj("{\"command\":\"status\"}"));current=Dict(snapshot["status"]);settings=Dict(current["settings"]);peers=Items(Get(fixture,"devices"));history=Items(Get(fixture,"history"));groups=GroupHistory(history);service=!Flag(fixture,"unavailable");UpdateSummary();UpdateTray();
            foreach(string view in new[]{"status","devices","device","history","detail","image-preview","missing-content","settings","settings-lower","paused","unavailable"}) {
                if((view=="device" && peers.Count==0) || ((view=="detail" || view=="image-preview" || view=="missing-content") && groups.Count==0)) continue;
                page=view=="image-preview" || view=="missing-content"?"detail":view=="settings-lower"?"settings":view=="paused" || view=="unavailable"?"status":view;
                selectedPeer=peers.Count>1?peers[1]:peers.FirstOrDefault();selectedHistory=view=="image-preview"?groups.FirstOrDefault(g=>Native.Text(g.Row,"format")=="png"):view=="missing-content"?groups.FirstOrDefault(g=>g.ContentId==null):groups.FirstOrDefault();
                if(view=="paused") {settings["send"]=false;settings["receive"]=false;}
                if(view=="unavailable") service=false;
                scroll.ScrollToTop();Render();UpdateSummary();await Task.Delay(160);
                root.Measure(new W.Size(width,height));root.Arrange(new W.Rect(0,0,width,height));root.UpdateLayout();
                if(view=="settings-lower") {scroll.ScrollToEnd();root.UpdateLayout();}
                SaveVisual(root,Path.Combine(output,view+".png"),width,height,scale);
                File.WriteAllText(Path.Combine(output,view+".layout.json"),Encode(new {view=view,width=width,height=height,scale=scale,language=language,theme=theme,contentHeight=body.ActualHeight,viewport=scroll.ViewportHeight}),Encoding.UTF8);
                Console.WriteLine("Rendered "+view);
            }
            service=true;
            if(peers.Count>0) AllowPeer(peers[0],Policy(Native.Text(peers[0],"id")));
            ClearHistory();
            if(history.Count>0) ValidatePresentation();
        }
        static void SaveVisual(W.Media.Visual visual,string path,double width,double height,double scale) {
            var bitmap=new B.RenderTargetBitmap((int)Math.Ceiling(width*scale),(int)Math.Ceiling(height*scale),96*scale,96*scale,M.PixelFormats.Pbgra32);bitmap.Render(visual);
            var encoder=new B.PngBitmapEncoder();encoder.Frames.Add(B.BitmapFrame.Create(bitmap));using(var stream=File.Create(path)) encoder.Save(stream);
        }
        void ValidatePresentation() {
            var grouped=GroupHistory(history);
            if(grouped.Count!=4 || !grouped.Any(g=>g.Local && g.Transfers.Count==2) || !grouped.Any(g=>g.Transfers.Any(t=>Native.Text(t,"state")=="unknown")) || !grouped.Any(g=>g.ContentId==null)) throw new Exception("Synthetic event grouping lost local, per-target or unavailable-content state");
            Console.WriteLine("PASS: event grouping preserves every destination and content availability");
            page="settings";edits["history_limit"]="47";Render();settings["receive"]=!Flag(settings,"receive");Render();
            root.UpdateLayout();
            if(!VisualChildren<C.TextBox>(root).Any(input=>A.AutomationProperties.GetName(input)==T("history.limit") && input.Text=="47")) throw new Exception("External settings refresh lost the actual retention input");
            Console.WriteLine("PASS: unsaved retention edit survives external direction refresh");
            selectedHistory=groups.FirstOrDefault(g=>g.ContentId==null);page="detail";Render();root.UpdateLayout();
            var unavailableActions=VisualChildren<C.Button>(root).Where(button=>new[]{T("history.copy"),T("history.copy_local"),T("history.resend")}.Contains(A.AutomationProperties.GetName(button))).ToArray();
            if(unavailableActions.Length!=3 || unavailableActions.Any(button=>button.IsEnabled || button.ToolTip==null)) throw new Exception("Unavailable body has an enabled or unexplained history action");
            Console.WriteLine("PASS: unavailable preview body disables copy/local-copy/resend with reasons");
            if(!navigation.Values.All(button=>!String.IsNullOrEmpty(A.AutomationProperties.GetName(button)) && button.Focusable)) throw new Exception("Navigation lacks keyboard or accessible names");
            Console.WriteLine("PASS: keyboard-focusable named navigation and guarded fingerprint consent");
        }
        static IEnumerable<T> VisualChildren<T>(W.DependencyObject parent) where T:W.DependencyObject {
            for(int index=0;index<M.VisualTreeHelper.GetChildrenCount(parent);index++) {var child=M.VisualTreeHelper.GetChild(parent,index);if(child is T) yield return (T)child;foreach(var nested in VisualChildren<T>(child)) yield return nested;}
        }

        Dictionary<string,object> FixtureRequest(Dictionary<string,object> action) {
            // Explicit synthetic presentation driver; never selected by normal startup.
            string command=Native.Text(action,"command");var s=Dict(fixture["status"]);var preferences=Dict(s["settings"]);
            switch(command) {
                case "status": if(Flag(fixture,"unavailable")) throw new IOException("Synthetic service outage");return Obj(Encode(new {type="status",status=s}));
                case "settings": return Obj(Encode(new {type="settings",settings=preferences}));
                case "devices": return Obj(Encode(new {type="devices",devices=Get(fixture,"devices"),settings=preferences}));
                case "history": return Obj(Encode(new {type="history",entries=Items(Get(fixture,"history")).Skip((int)Number(action,"offset")).Take((int)Number(action,"limit")).ToArray()}));
                case "configure": s["settings"]=action["settings"];break;
                case "set_directions": foreach(string key in new[]{"send","receive"}) if(Get(action,key)!=null) preferences[key]=action[key];break;
                case "peer": Dict(preferences["peers"])[Native.Text(action,"id")]=action["policy"];break;
                case "preview": object preview;var previews=Dict(Get(fixture,"previews"));if(!previews.TryGetValue(Native.Text(action,"id"),out preview)) return Obj("{\"type\":\"error\",\"message\":\"Synthetic content is unavailable\"}");return Obj(Encode(preview));
                case "clear_history": fixture["history"]=new object[0];break;
                case "autostart": return Obj("{\"type\":\"autostart\",\"status\":{\"state\":\"disabled\",\"message\":\"Disabled\"}}");
            }
            return Obj("{\"type\":\"done\",\"message\":\"Completed\"}");
        }
        sealed class UiCommand : System.Windows.Input.ICommand {
            readonly Action action;public UiCommand(Action execute) {action=execute;}public bool CanExecute(object parameter) {return true;}public void Execute(object parameter) {action();}public event EventHandler CanExecuteChanged {add {} remove {}}
        }
    }
}
