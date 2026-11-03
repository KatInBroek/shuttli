//! Windows composition root. The native helper owns ACL-protected session pipes.
use shuttli_adapters::{
    clipboard, files, identity, network::TailscaleNetwork, platform::NativePlatform,
};
use shuttli_api::control::{Answer, ControlApi, ControlRequest};
use shuttli_application::service::Service;
use std::{
    io::{BufRead, BufReader, Read, Write},
    path::PathBuf,
    process::Stdio,
    sync::mpsc,
    time::{Duration, Instant},
};

struct Remote {
    dir: PathBuf,
}
impl ControlApi for Remote {
    fn request(&mut self, request: ControlRequest) -> Answer {
        fn call(dir: &std::path::Path, request: &ControlRequest) -> Result<Answer, String> {
            let mut child = shuttli_adapters::windows::command()?
                .arg("client")
                .arg(dir)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|e| e.to_string())?;
            let mut input = child.stdin.take().ok_or("missing control input")?;
            serde_json::to_writer(&mut input, request).map_err(|e| e.to_string())?;
            input.write_all(b"\n").map_err(|e| e.to_string())?;
            drop(input);
            // The helper bounds connection, frame size and read/write durations.
            let output = child.wait_with_output().map_err(|e| e.to_string())?;
            if !output.status.success() {
                return Err(format!(
                    "agent unavailable: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ));
            }
            serde_json::from_slice(&output.stdout).map_err(|_| "invalid agent response".into())
        }
        call(&self.dir, &request).unwrap_or_else(|message| Answer::Error { message })
    }
}
pub fn run() -> Result<u8, String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let dir = files::data_dir()?;
    if args.first().map(String::as_str) == Some("daemon") {
        return daemon(dir);
    }
    if args.first().map(String::as_str) == Some("ui") {
        return ui(dir);
    }
    let output = shuttli_cli::control::run(&args, &mut Remote { dir });
    print!("{}", output.text);
    Ok(output.exit_code)
}
fn ui(dir: PathBuf) -> Result<u8, String> {
    let status = shuttli_adapters::windows::command()?
        .arg("ui")
        .arg(dir)
        .arg(std::env::current_exe().map_err(|e| e.to_string())?)
        .status()
        .map_err(|e| e.to_string())?;
    Ok(if status.success() { 0 } else { 1 })
}
fn daemon(dir: PathBuf) -> Result<u8, String> {
    let _lock = files::instance_lock(&dir)?;
    let identity = identity::load(&dir)?;
    let id = identity.id;
    let epoch = shuttli_adapters::random_epoch()?;
    let service = shuttli_runtime::block_on(Service::new(
        id,
        epoch,
        shuttli_runtime::worker("shuttli-clipboard", clipboard::open()?)?,
        shuttli_runtime::worker(
            "shuttli-store",
            Box::new(shuttli_adapters::storage::SqlStore::open(&dir)?)
                as Box<dyn shuttli_ports::sync::Store>,
        )?,
        Box::new(TailscaleNetwork::open(identity, epoch)?),
        shuttli_runtime::worker(
            "shuttli-platform",
            Box::<NativePlatform>::default() as Box<dyn shuttli_ports::sync::Platform>,
        )?,
    ))?;
    let mut ipc = shuttli_adapters::windows::command()?
        .arg("server")
        .arg(&dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| e.to_string())?;
    let mut input = ipc.stdin.take().ok_or("missing IPC input")?;
    let mut output = BufReader::new(ipc.stdout.take().ok_or("missing IPC output")?);
    let (tx, rx) = mpsc::sync_channel::<(u64, ControlRequest)>(8);
    let host = std::thread::current();
    std::thread::spawn(move || {
        loop {
            let mut line = String::new();
            if output
                .by_ref()
                .take(512 * 1024 + 1)
                .read_line(&mut line)
                .ok()
                .is_none_or(|n| n == 0)
                || line.len() > 512 * 1024
            {
                break;
            }
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
                break;
            };
            let (Some(id), Ok(request)) = (
                value["id"].as_u64(),
                serde_json::from_value(value["request"].clone()),
            ) else {
                break;
            };
            if tx.send((id, request)).is_err() {
                break;
            }
            host.unpark();
        }
    });
    let (reply_tx, reply_rx) = mpsc::channel::<(u64, Answer)>();
    let mut tasks: Vec<shuttli_runtime::Task<'_>> = Vec::new();
    let ticking = std::cell::Cell::new(false);
    let mut next = Instant::now();
    eprintln!(
        "{} Windows agent ready; user/session named pipe available",
        shuttli_brand::NAME
    );
    let result = (|| {
        loop {
            if ipc.try_wait().map_err(|e| e.to_string())?.is_some() {
                return Err("native IPC helper exited".into());
            }
            while let Ok((id, request)) = rx.try_recv() {
                if tasks.len() >= 16 {
                    reply_tx
                        .send((
                            id,
                            Answer::Error {
                                message: "agent busy".into(),
                            },
                        ))
                        .map_err(|e| e.to_string())?;
                    continue;
                }
                let service = &service;
                let reply_tx = reply_tx.clone();
                tasks.push(Box::pin(async move {
                    let _ = reply_tx.send((id, service.request(request).await));
                }));
            }
            if !service.stopping() && Instant::now() >= next && !ticking.get() {
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
            while let Ok((id, answer)) = reply_rx.try_recv() {
                serde_json::to_writer(&mut input, &serde_json::json!({"id":id,"answer":answer}))
                    .map_err(|e| e.to_string())?;
                input
                    .write_all(b"\n")
                    .and_then(|_| input.flush())
                    .map_err(|e| e.to_string())?;
            }
            if service.stopping() {
                break;
            }
            std::thread::park_timeout(Duration::from_millis(20));
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        while !tasks.is_empty() && Instant::now() < deadline {
            shuttli_runtime::poll_tasks(&mut tasks);
            std::thread::park_timeout(Duration::from_millis(10));
        }
        drop(tasks);
        shuttli_runtime::block_on(service.cleanup())?;
        // Give the bounded pipe writer time to deliver the final quit response.
        std::thread::sleep(Duration::from_millis(100));
        Ok(0)
    })();
    let _ = ipc.kill();
    let _ = ipc.wait();
    result
}
