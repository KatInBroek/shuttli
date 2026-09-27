use super::*;
use crate::content::payload;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use shuttli_model::sync::{ClipboardStamp, Format};
use shuttli_ports::sync::{ClipboardValue, Payload};
use std::{
    io::{BufRead, BufReader, Read, Write},
    process::{Child, Command, Stdio},
    sync::mpsc::{Receiver, SyncSender, sync_channel},
    time::Duration,
};

pub struct MacClipboard {
    child: Child,
    requests: SyncSender<Value>,
    responses: Receiver<Result<Value>>,
    cache: Option<ClipboardValue>,
}
impl MacClipboard {
    pub fn open() -> Result<Self> {
        use std::os::unix::fs::PermissionsExt;
        let path = crate::files::data_dir()?.join("shuttli-pasteboard");
        let bytes = include_bytes!(concat!(env!("OUT_DIR"), "/shuttli-pasteboard"));
        crate::files::atomic_write(&path, bytes)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
        let mut child = Command::new(path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| e.to_string())?;
        let mut input = child.stdin.take().ok_or("missing helper input")?;
        let mut output = BufReader::new(child.stdout.take().ok_or("missing helper output")?);
        let (requests, incoming) = sync_channel::<Value>(1);
        let (outgoing, responses) = sync_channel(1);
        // All pipe I/O runs on one worker; a hung OS pasteboard cannot stall the agent.
        std::thread::spawn(move || {
            while let Ok(request) = incoming.recv() {
                let result = (|| -> Result<Value> {
                    serde_json::to_writer(&mut input, &request).map_err(|e| e.to_string())?;
                    input.write_all(b"\n").map_err(|e| e.to_string())?;
                    input.flush().map_err(|e| e.to_string())?;
                    let mut line = String::new();
                    output
                        .by_ref()
                        .take(12 * 1024 * 1024 + 1)
                        .read_line(&mut line)
                        .map_err(|e| e.to_string())?;
                    if line.len() > 12 * 1024 * 1024 {
                        return Err("pasteboard helper response too large".into());
                    }
                    let value: Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
                    if let Some(error) = value["error"].as_str() {
                        return Err(error.into());
                    }
                    Ok(value)
                })();
                if outgoing.send(result).is_err() {
                    break;
                }
            }
        });
        Ok(Self {
            child,
            requests,
            responses,
            cache: None,
        })
    }
    fn call(&mut self, value: Value) -> Result<Value> {
        self.requests
            .try_send(value)
            .map_err(|_| "pasteboard helper unavailable")?;
        match self.responses.recv_timeout(Duration::from_secs(5)) {
            Ok(result) => result,
            Err(_) => {
                let _ = self.child.kill();
                Err("pasteboard helper timeout; restart agent to reconnect".into())
            }
        }
    }
    fn read_os(&mut self) -> Result<ClipboardValue> {
        let v = self.call(json!({"op":"read"}))?;
        let data = if let Some(encoded) = v["data"].as_str() {
            let f = match v["format"].as_str() {
                Some("text") => Format::Text,
                Some("png") => Format::Png,
                _ => return Err("unsupported pasteboard type".into()),
            };
            Some(payload(
                f,
                STANDARD.decode(encoded).map_err(|e| e.to_string())?,
            )?)
        } else {
            None
        };
        Ok(ClipboardValue {
            stamp: ClipboardStamp {
                generation: v["generation"].as_u64().ok_or("missing changeCount")?,
                digest: data.as_ref().map_or([0; 32], |p| p.meta.digest),
                sensitive: v["sensitive"].as_bool().unwrap_or(true),
            },
            payload: data,
        })
    }
}
impl Clipboard for MacClipboard {
    fn description(&self) -> &str {
        "macOS NSPasteboard (changeCount; conservative equal-content suppression)"
    }
    fn read(&mut self) -> Result<ClipboardValue> {
        let v = self.call(json!({"op":"inspect"}))?;
        if let Some(c) = &self.cache {
            if Some(c.stamp.generation) == v["generation"].as_u64() {
                return Ok(c.clone());
            }
        }
        let c = self.read_os()?;
        self.cache = Some(c.clone());
        Ok(c)
    }
    fn write(
        &mut self,
        p: &Payload,
        authorization: shuttli_core::sync::WriteAuthorization,
    ) -> Result<ClipboardValue> {
        if authorization.metadata() != &p.meta {
            return Err("write content differs from core authorization".into());
        }
        let expected = authorization.baseline();

        self.call(json!({"op":"write","generation":expected.generation,"format":p.meta.format,"data":STANDARD.encode(&p.data)}))?;
        self.cache = None;
        let c = self.read_os()?;
        if c.stamp.digest != p.meta.digest {
            return Err("pasteboard readback mismatch".into());
        }
        self.cache = Some(c.clone());
        Ok(c)
    }
}
impl Drop for MacClipboard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
