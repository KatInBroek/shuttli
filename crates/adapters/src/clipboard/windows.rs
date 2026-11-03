use super::*;
use crate::content::payload;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use shuttli_model::sync::{ClipboardStamp, Format};
use shuttli_ports::sync::{ClipboardValue, Payload};
use std::{
    io::{BufRead, BufReader, Read, Write},
    process::{Child, Stdio},
    sync::mpsc,
    time::Duration,
};

pub struct WindowsClipboard {
    child: Child,
    requests: mpsc::SyncSender<Value>,
    responses: mpsc::Receiver<Result<Value>>,
    cache: Option<ClipboardValue>,
}
impl WindowsClipboard {
    pub fn open() -> Result<Self> {
        let mut child = crate::windows::command()?
            .arg("clipboard")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| e.to_string())?;
        let mut input = child.stdin.take().ok_or("missing clipboard input")?;
        let mut output = BufReader::new(child.stdout.take().ok_or("missing clipboard output")?);
        let (requests, incoming) = mpsc::sync_channel::<Value>(1);
        let (outgoing, responses) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            while let Ok(request) = incoming.recv() {
                let result = (|| {
                    serde_json::to_writer(&mut input, &request).map_err(|e| e.to_string())?;
                    input
                        .write_all(b"\n")
                        .and_then(|_| input.flush())
                        .map_err(|e| e.to_string())?;
                    let mut line = String::new();
                    output
                        .by_ref()
                        .take(12 * 1024 * 1024 + 1)
                        .read_line(&mut line)
                        .map_err(|e| e.to_string())?;
                    if line.len() > 12 * 1024 * 1024 {
                        return Err("clipboard response too large".into());
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
    fn call(&mut self, request: Value) -> Result<Value> {
        self.requests
            .try_send(request)
            .map_err(|_| "clipboard helper unavailable")?;
        match self.responses.recv_timeout(Duration::from_secs(5)) {
            Ok(result) => result,
            Err(_) => {
                let _ = self.child.kill();
                Err("clipboard helper timeout; restart agent".into())
            }
        }
    }
    fn read_os(&mut self) -> Result<ClipboardValue> {
        let value = self.call(json!({"op":"read"}))?;
        let data = if let Some(data) = value["data"].as_str() {
            let format = match value["format"].as_str() {
                Some("text") => Format::Text,
                Some("png") => Format::Png,
                _ => return Err("unsupported clipboard format".into()),
            };
            Some(payload(
                format,
                STANDARD.decode(data).map_err(|e| e.to_string())?,
            )?)
        } else {
            None
        };
        Ok(ClipboardValue {
            stamp: ClipboardStamp {
                generation: value["generation"]
                    .as_u64()
                    .ok_or("missing clipboard sequence")?,
                digest: data.as_ref().map_or([0; 32], |p| p.meta.digest),
                sensitive: value["sensitive"].as_bool().unwrap_or(true),
            },
            payload: data,
        })
    }
}
impl Clipboard for WindowsClipboard {
    fn description(&self) -> &str {
        "Windows native clipboard (format listener; sequence and independent OS readback)"
    }
    fn read(&mut self) -> Result<ClipboardValue> {
        let value = self.call(json!({"op":"inspect"}))?;
        if let Some(cache) = &self.cache {
            if Some(cache.stamp.generation) == value["generation"].as_u64() {
                return Ok(cache.clone());
            }
        }
        let result = self.read_os()?;
        self.cache = Some(result.clone());
        Ok(result)
    }
    fn write(
        &mut self,
        p: &Payload,
        authorization: shuttli_core::sync::WriteAuthorization,
    ) -> Result<ClipboardValue> {
        if authorization.metadata() != &p.meta {
            return Err("content differs from core authorization".into());
        }
        // Validate before entering any native write, including pixel and encoded limits.
        let checked = payload(p.meta.format, p.data.to_vec())?;
        if checked.meta != p.meta {
            return Err("invalid clipboard metadata".into());
        }
        self.call(
            json!({"op":"write", "generation":authorization.baseline().generation,
            "format":p.meta.format, "data":STANDARD.encode(&p.data)}),
        )?;
        self.cache = None;
        let value = self.read_os()?;
        if value.stamp.digest != p.meta.digest {
            return Err("Windows clipboard readback mismatch".into());
        }
        self.cache = Some(value.clone());
        Ok(value)
    }
}
impl Drop for WindowsClipboard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
