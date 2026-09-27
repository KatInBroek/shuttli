use crate::files::atomic_write;
use shuttli_ports::sync::{Platform, Result};
use std::{
    path::PathBuf,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
#[derive(Default)]
pub struct NativePlatform {
    notifier: Option<Notifier>,
}
type NoticeState = (bool, Option<(String, String)>);
struct Notifier {
    shared: std::sync::Arc<(std::sync::Mutex<NoticeState>, std::sync::Condvar)>,
}
impl Notifier {
    fn new() -> Self {
        let shared = std::sync::Arc::new((
            std::sync::Mutex::<NoticeState>::new((false, None)),
            std::sync::Condvar::new(),
        ));
        let worker = shared.clone();
        std::thread::spawn(move || {
            loop {
                let (lock, wake) = &*worker;
                let mut state = lock.lock().expect("notification lock");
                while !state.0 && state.1.is_none() {
                    state = wake.wait(state).expect("notification lock");
                }
                if state.0 {
                    return;
                }
                let (title, body) = state.1.take().expect("pending notification");
                drop(state);
                show_notice(&title, &body);
                std::thread::sleep(Duration::from_millis(150));
            }
        });
        Self { shared }
    }
    fn send(&self, title: String, body: String) {
        let (lock, wake) = &*self.shared;
        lock.lock().expect("notification lock").1 = Some((title, body));
        wake.notify_one();
    }
}
impl Drop for Notifier {
    fn drop(&mut self) {
        let (lock, wake) = &*self.shared;
        lock.lock().expect("notification lock").0 = true;
        wake.notify_one();
    }
}
fn show_notice(title: &str, body: &str) {
    let escaped = body
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    let mut command = if cfg!(target_os = "macos") {
        let mut c = Command::new("osascript");
        c.args(["-e","on run argv\n display notification (item 2 of argv) with title (item 1 of argv)\nend run",title,body]);
        c
    } else {
        let mut c = Command::new("notify-send");
        c.args([
            "--app-name",
            shuttli_brand::NAME,
            "--expire-time=3000",
            "--replace-id=45987",
            title,
            &escaped,
        ]);
        c
    };
    command.stdout(Stdio::null()).stderr(Stdio::null());
    if let Ok(mut child) = command.spawn() {
        let until = Instant::now() + Duration::from_secs(3);
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if Instant::now() < until => std::thread::sleep(Duration::from_millis(50)),
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break;
                }
            }
        }
    }
}
fn home() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or("HOME unavailable".into())
}
fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
impl Platform for NativePlatform {
    fn notify(&mut self, title: &str, body: &str) {
        self.notifier
            .get_or_insert_with(Notifier::new)
            .send(title.into(), body.into());
    }
    fn autostart(&mut self, enabled: Option<bool>) -> Result<String> {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let exe = exe.to_str().ok_or("non-Unicode application path")?;
        if cfg!(target_os = "linux") {
            let dir = std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .unwrap_or(home()?.join(".config"))
                .join("autostart");
            let path = dir.join("shuttli.desktop");
            if let Some(on) = enabled {
                if on {
                    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                    let escaped = exe
                        .replace('\\', "\\\\")
                        .replace('"', "\\\"")
                        .replace('`', "\\`")
                        .replace('$', "\\$")
                        .replace('%', "%%");
                    atomic_write(&path,format!("[Desktop Entry]\nType=Application\nName={}\nExec=\"{escaped}\" daemon\nTerminal=false\nX-GNOME-Autostart-enabled=true\n", shuttli_brand::desktop_name()).as_bytes())?;
                } else if path.exists() {
                    std::fs::remove_file(&path).map_err(|e| e.to_string())?;
                }
            }
            let data = std::fs::read_to_string(path).unwrap_or_default();
            Ok(if data.is_empty()
                || data
                    .lines()
                    .any(|l| l == "Hidden=true" || l == "X-GNOME-Autostart-enabled=false")
            {
                "disabled"
            } else {
                "enabled (current user's graphical login)"
            }
            .into())
        } else if cfg!(target_os = "macos") {
            let dir = home()?.join("Library/LaunchAgents");
            let path = dir.join("org.shuttli.agent.plist");
            let uid = crate::discovery::command_output("id", &["-u"])?;
            let uid = std::str::from_utf8(&uid).map_err(|_| "invalid uid")?.trim();
            let domain = format!("gui/{uid}");
            let label = format!("{domain}/org.shuttli.agent");
            if let Some(on) = enabled {
                if on {
                    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                    atomic_write(&path,format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\"><plist version=\"1.0\"><dict><key>Label</key><string>org.shuttli.agent</string><key>ProgramArguments</key><array><string>{}</string><string>daemon</string></array><key>RunAtLoad</key><true/></dict></plist>",xml(exe)).as_bytes())?;
                    let _ = crate::discovery::command_output("launchctl", &["enable", &label]);
                    // Register for future login; avoid spawning a second instance now.
                } else if path.exists() {
                    std::fs::remove_file(&path).map_err(|e| e.to_string())?;
                }
            }
            if !path.exists() {
                return Ok("disabled".into());
            }
            if let Ok(b) =
                crate::discovery::command_output("launchctl", &["print-disabled", &domain])
            {
                let text = String::from_utf8_lossy(&b);
                if text
                    .lines()
                    .any(|l| l.contains("org.shuttli.agent") && l.contains("true"))
                {
                    return Ok("requires_user_action (disabled by macOS)".into());
                }
            }
            Ok("enabled (registered for next graphical login)".into())
        } else {
            Err("autostart unsupported on this platform".into())
        }
    }
}
