using System;
using System.Diagnostics;
using System.IO;
using System.Reflection;

// Desktop entry point: keep console allocation out of the native window flow.
static class Launcher {
    [STAThread] static int Main() {
        try {
            string directory=Path.GetDirectoryName(Assembly.GetExecutingAssembly().Location);
            Process.Start(new ProcessStartInfo(Path.Combine(directory,"shuttli.exe"),"ui") {
                UseShellExecute=false,CreateNoWindow=true,WorkingDirectory=directory
            });
            return 0;
        } catch(Exception error) {
            System.Windows.Forms.MessageBox.Show(error.Message,"Clipboard application",System.Windows.Forms.MessageBoxButtons.OK,System.Windows.Forms.MessageBoxIcon.Error);
            return 1;
        }
    }
}
