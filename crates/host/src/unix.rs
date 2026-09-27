use shuttli_adapters::{
    clipboard, files, identity::Identity, network::TailscaleNetwork, platform::NativePlatform,
    storage::SqlStore,
};
use shuttli_api::control::{Answer, ControlApi, ControlRequest};
use shuttli_application::service::Service;
use std::{
    io::{Read, Write},
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
    },
    path::PathBuf,
    sync::mpsc,
    time::{Duration, Instant},
};
const MAX_REQUEST: u64 = 256 * 1024;
struct Remote {
    path: PathBuf,
}
impl ControlApi for Remote {
    fn request(&mut self, r: ControlRequest) -> Answer {
        fn call(path: &std::path::Path, r: ControlRequest) -> Result<Answer, String> {
            let mut s = match UnixStream::connect(path) {
                Ok(stream) => stream,
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
                    ) && path.with_file_name("stopped").is_file() =>
                {
                    return Ok(Answer::Stopped);
                }
                Err(_) => return Err("agent is not running; start `shuttli daemon`".into()),
            };
            s.set_read_timeout(Some(Duration::from_secs(12)))
                .map_err(|e| e.to_string())?;
            s.set_write_timeout(Some(Duration::from_secs(3)))
                .map_err(|e| e.to_string())?;
            let b = serde_json::to_vec(&r).map_err(|e| e.to_string())?;
            if b.len() > MAX_REQUEST as usize {
                return Err("request too large".into());
            }
            s.write_all(&b).map_err(|e| e.to_string())?;
            s.shutdown(std::net::Shutdown::Write)
                .map_err(|e| e.to_string())?;
            let mut b = Vec::new();
            s.take(12 * 1024 * 1024 + 1)
                .read_to_end(&mut b)
                .map_err(|e| e.to_string())?;
            if b.len() > 12 * 1024 * 1024 {
                return Err("response too large".into());
            }
            serde_json::from_slice(&b).map_err(|_| "invalid agent response".into())
        }
        call(&self.path, r).unwrap_or_else(|message| Answer::Error { message })
    }
}
pub fn run() -> Result<u8, String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let dir = files::data_dir()?;
    #[cfg(target_os = "macos")]
    if args.is_empty()
        && std::env::current_exe()
            .ok()
            .is_some_and(|p| p.to_string_lossy().contains(".app/Contents/MacOS/"))
    {
        return ui();
    }
    if args.first().map(String::as_str) == Some("daemon") {
        return daemon(dir);
    }
    if args.first().map(String::as_str) == Some("ui") {
        return ui();
    }
    let out = shuttli_cli::control::run(
        &args,
        &mut Remote {
            path: dir.join("control.sock"),
        },
    );
    print!("{}", out.text);
    Ok(out.exit_code)
}
#[cfg(target_os = "linux")]
struct PresentationPlatform {
    native: NativePlatform,
    profile: PathBuf,
}
#[cfg(target_os = "linux")]
impl shuttli_ports::sync::Platform for PresentationPlatform {
    fn notify(&mut self, title: &str, body: &str) {
        let (title, body) = shuttli_native_ui::i18n::notice(&self.profile, title, body);
        self.native.notify(&title, &body);
    }
    fn autostart(
        &mut self,
        enabled: Option<bool>,
    ) -> Result<shuttli_model::sync::AutostartStatus, String> {
        self.native.autostart(enabled)
    }
}
type ControlMessage = (ControlRequest, mpsc::SyncSender<Answer>);
fn serve_control(
    mut stream: UnixStream,
    sender: &mpsc::SyncSender<ControlMessage>,
    host: &std::thread::Thread,
) -> Result<(), String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    (&mut stream)
        .take(MAX_REQUEST + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_REQUEST as usize {
        return Err("request too large".into());
    }
    let answer = match serde_json::from_slice::<ControlRequest>(&bytes) {
        Ok(request) => {
            let (reply, result) = mpsc::sync_channel(1);
            if sender.try_send((request, reply)).is_err() {
                Answer::Error {
                    message: "agent busy".into(),
                }
            } else {
                host.unpark();
                result
                    .recv_timeout(Duration::from_secs(10))
                    .unwrap_or_else(|_| Answer::Error {
                        message: "operation timed out; inspect current status before retrying"
                            .into(),
                    })
            }
        }
        Err(_) => Answer::Error {
            message: "invalid request".into(),
        },
    };
    serde_json::to_writer(&mut stream, &answer).map_err(|e| e.to_string())
}
fn daemon(dir: PathBuf) -> Result<u8, String> {
    let _lock = files::instance_lock(&dir)?;
    match std::fs::remove_file(dir.join("stopped")) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.to_string()),
    }
    let identity = Identity::load(&dir)?;
    let id = identity.id;
    // Random restart epoch from the OS provider; no epoch is compared by magnitude.
    let epoch = shuttli_adapters::random_epoch()?;
    let store = SqlStore::open(&dir)?;
    let clipboard = clipboard::open()?;
    let network = TailscaleNetwork::open(identity, epoch)?;
    #[cfg(target_os = "linux")]
    let platform = Box::new(PresentationPlatform {
        native: NativePlatform::default(),
        profile: dir.clone(),
    });
    #[cfg(not(target_os = "linux"))]
    let platform = Box::<NativePlatform>::default();
    let service = shuttli_runtime::block_on(Service::new(
        id,
        epoch,
        shuttli_runtime::worker("shuttli-clipboard", clipboard)?,
        shuttli_runtime::worker(
            "shuttli-store",
            Box::new(store) as Box<dyn shuttli_ports::sync::Store>,
        )?,
        Box::new(network),
        shuttli_runtime::worker(
            "shuttli-platform",
            platform as Box<dyn shuttli_ports::sync::Platform>,
        )?,
    ))?;
    let path = dir.join("control.sock");
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| e.to_string())?;
    }
    let listener = UnixListener::bind(&path).map_err(|e| e.to_string())?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| e.to_string())?;
    let (tx, rx) = mpsc::sync_channel::<(ControlRequest, mpsc::SyncSender<Answer>)>(8);
    // Four bounded IPC readers prevent a slow request from blocking pause/status.
    let listener = std::sync::Arc::new(listener);
    let host_thread = std::thread::current();
    let connections = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    for _ in 0..4 {
        let listener = listener.clone();
        let tx = tx.clone();
        let host_thread = host_thread.clone();
        let connections = connections.clone();
        std::thread::Builder::new()
            .name("shuttli-control".into())
            .spawn(move || {
                for stream in listener.incoming().flatten() {
                    connections.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    let _ = serve_control(stream, &tx, &host_thread);
                    connections.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                    host_thread.unpark();
                }
            })
            .map_err(|e| e.to_string())?;
    }
    drop(tx);
    eprintln!(
        "{} agent ready; local control socket available",
        shuttli_brand::NAME
    );
    #[cfg(target_os = "linux")]
    let mut tray = match start_tray(&dir) {
        Ok(child) => Some(child),
        Err(error) => {
            eprintln!(
                "{} tray unavailable: {error}; CLI and window remain available",
                shuttli_brand::NAME
            );
            None
        }
    };
    let mut next = Instant::now();
    let ticking = std::cell::Cell::new(false);
    let mut tasks: Vec<shuttli_runtime::Task<'_>> = Vec::new();
    loop {
        #[cfg(target_os = "linux")]
        if let Some(child) = tray.as_mut() {
            if let Ok(Some(status)) = child.try_wait() {
                eprintln!(
                    "{} tray exited ({status}); CLI and window remain available",
                    shuttli_brand::NAME
                );
                tray = None;
            }
        }
        while let Ok((request, reply)) = rx.try_recv() {
            if tasks.len() >= 16 {
                let _ = reply.send(Answer::Error {
                    message: "agent busy".into(),
                });
                continue;
            }
            let service = &service;
            tasks.push(Box::pin(async move {
                let _ = reply.send(service.request(request).await);
            }));
        }
        if Instant::now() >= next && !ticking.get() {
            ticking.set(true);
            let service = &service;
            let ticking = &ticking;
            tasks.push(Box::pin(async move {
                service.tick().await;
                ticking.set(false);
            }));
            next = Instant::now() + Duration::from_millis(500);
        }
        shuttli_runtime::poll_tasks(&mut tasks);
        if service.stopping() {
            break;
        }
        let wait = if ticking.get() {
            Duration::from_millis(500)
        } else {
            next.saturating_duration_since(Instant::now())
        };
        std::thread::park_timeout(wait);
    }
    // Stop accepting new clients, finish already-started effects and flush the
    // quit reply before process exit. Preferences and login registration persist.
    files::atomic_write(&dir.join("stopped"), b"explicit quit\n")?;
    std::fs::remove_file(&path).map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(10);
    while !tasks.is_empty() && Instant::now() < deadline {
        while let Ok((_, reply)) = rx.try_recv() {
            let _ = reply.send(Answer::Error {
                message: "agent is stopping".into(),
            });
        }
        shuttli_runtime::poll_tasks(&mut tasks);
        std::thread::park_timeout(Duration::from_millis(10));
    }
    drop(tasks);
    let mut cleanup: Vec<shuttli_runtime::Task<'_>> = vec![Box::pin(async {
        if let Err(error) = service.cleanup().await {
            eprintln!("history cleanup: {error}");
        }
    })];
    while (!cleanup.is_empty() || connections.load(std::sync::atomic::Ordering::SeqCst) != 0)
        && Instant::now() < deadline
    {
        while let Ok((_, reply)) = rx.try_recv() {
            let _ = reply.send(Answer::Error {
                message: "agent is stopping".into(),
            });
        }
        shuttli_runtime::poll_tasks(&mut cleanup);
        std::thread::park_timeout(Duration::from_millis(10));
    }
    #[cfg(target_os = "linux")]
    if let Some(mut child) = tray {
        let _ = child.kill();
        let _ = child.wait();
    }
    Ok(0)
}

#[cfg(target_os = "linux")]
fn start_tray(dir: &std::path::Path) -> Result<std::process::Child, String> {
    write_locale_assets(dir)?;
    let script = dir.join("shuttli-tray.py");
    files::atomic_write(&script, shuttli_native_ui::TRAY_CLIENT.as_bytes())?;
    files::atomic_write(
        &dir.join("tray_model.py"),
        shuttli_native_ui::TRAY_MODEL.as_bytes(),
    )?;
    std::process::Command::new("/usr/bin/python3")
        .arg(script)
        .arg(dir.join("control.sock"))
        .arg(std::env::current_exe().map_err(|e| e.to_string())?)
        .arg(std::process::id().to_string())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())
}
#[cfg(target_os = "linux")]
fn write_locale_assets(dir: &std::path::Path) -> Result<(), String> {
    files::atomic_write(
        &dir.join("control_client.py"),
        shuttli_native_ui::CONTROL_CLIENT.as_bytes(),
    )?;
    files::atomic_write(
        &dir.join("i18n.py"),
        shuttli_native_ui::I18N_CLIENT.as_bytes(),
    )?;
    files::atomic_write(
        &dir.join("brand.py"),
        shuttli_native_ui::BRAND_CLIENT.as_bytes(),
    )?;
    files::atomic_write(
        &dir.join("product-name.txt"),
        shuttli_brand::NAME.as_bytes(),
    )?;
    files::atomic_write(
        &dir.join("tray_icons.py"),
        shuttli_native_ui::TRAY_ICONS.as_bytes(),
    )?;
    let locales = dir.join("locales");
    std::fs::create_dir_all(&locales).map_err(|e| e.to_string())?;
    for (code, json) in shuttli_native_ui::LOCALES {
        files::atomic_write(&locales.join(format!("{code}.json")), json.as_bytes())?;
    }
    Ok(())
}
fn ui() -> Result<u8, String> {
    let dir = files::data_dir()?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut remote = Remote {
        path: dir.join("control.sock"),
    };
    if matches!(
        remote.request(ControlRequest {
            version: shuttli_api::control::VERSION,
            action: shuttli_api::control::Action::Status
        }),
        Answer::Error { .. } | Answer::Stopped
    ) {
        std::process::Command::new(&exe)
            .arg("daemon")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| e.to_string())?;
        for _ in 0..20 {
            if dir.join("control.sock").exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }
    #[cfg(target_os = "linux")]
    {
        write_locale_assets(&dir)?;
        for (name, content) in [
            ("ui_model.py", shuttli_native_ui::UI_MODEL),
            ("style.css", shuttli_native_ui::UI_STYLE),
            ("mark.svg", shuttli_native_ui::UI_MARK),
        ] {
            files::atomic_write(&dir.join(name), content.as_bytes())?;
        }
        let path = dir.join("shuttli-ui.py");
        files::atomic_write(&path, shuttli_native_ui::GTK_CLIENT.as_bytes())?;
        let status = std::process::Command::new("/usr/bin/python3")
            .arg(path)
            .arg(dir.join("control.sock"))
            .status()
            .map_err(|e| e.to_string())?;
        Ok(if status.success() { 0 } else { 1 })
    }
    #[cfg(target_os = "macos")]
    {
        let path = dir.join("shuttli-ui");
        files::atomic_write(&path, shuttli_native_ui::MACOS_CLIENT)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
        let mut command = std::process::Command::new(path);
        command.arg(exe);
        if std::env::args().any(|a| a == "--background") {
            command.arg("--background");
        }
        let status = command.status().map_err(|e| e.to_string())?;
        Ok(if status.success() { 0 } else { 1 })
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Err("native UI unavailable".into())
    }
}
