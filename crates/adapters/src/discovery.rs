//! Tailscale discovery only supplies addresses; it cannot authorize a device.
use serde_json::Value;
use shuttli_ports::sync::Result;
use std::{
    io::Read,
    net::IpAddr,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
#[derive(Clone, Debug)]
pub struct Tailnet {
    pub own: IpAddr,
    pub name: String,
    pub peers: Vec<IpAddr>,
}
pub fn command_output(program: &str, args: &[&str]) -> Result<Vec<u8>> {
    let mut child = Command::new(program)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    let stdout = child.stdout.take().ok_or("missing command output")?;
    let reader = std::thread::spawn(move || {
        let mut b = Vec::new();
        stdout
            .take(4 * 1024 * 1024 + 1)
            .read_to_end(&mut b)
            .map(|_| b)
    });
    let until = Instant::now() + Duration::from_secs(5);
    let success = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s.success(),
            Ok(None) if Instant::now() < until => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break false;
            }
        }
    };
    let bytes = reader
        .join()
        .map_err(|_| "command reader failed")?
        .map_err(|e| e.to_string())?;
    if !success || bytes.len() > 4 * 1024 * 1024 {
        return Err("command unavailable, timed out, or response too large".into());
    }
    Ok(bytes)
}
pub fn snapshot() -> Result<Tailnet> {
    let program = std::env::var("SHUTTLI_TAILSCALE").unwrap_or_else(|_| {
        if cfg!(target_os = "macos")
            && std::path::Path::new("/Applications/Tailscale.app/Contents/MacOS/Tailscale").exists()
        {
            "/Applications/Tailscale.app/Contents/MacOS/Tailscale".into()
        } else {
            "tailscale".into()
        }
    });
    let bytes = command_output(&program, &["status", "--json"])?;
    parse(&bytes)
}
fn parse(bytes: &[u8]) -> Result<Tailnet> {
    let v: Value = serde_json::from_slice(bytes).map_err(|_| "invalid Tailscale status")?;
    if v["BackendState"] != "Running" {
        return Err("Tailscale is not running".into());
    }
    fn ipv4(v: &Value) -> Option<IpAddr> {
        v.as_array()?.iter().filter_map(|v|v.as_str()?.parse::<IpAddr>().ok()).find(|ip|matches!(ip,IpAddr::V4(a)if a.octets()[0]==100&&(64..=127).contains(&a.octets()[1])))
    }
    let own = ipv4(&v["TailscaleIPs"]).ok_or("no Tailscale IPv4 address")?;
    let peers = v["Peer"]
        .as_object()
        .map(|p| {
            p.values()
                .filter(|p| p["Online"] == true)
                .filter_map(|p| ipv4(&p["TailscaleIPs"]))
                .take(256)
                .collect()
        })
        .unwrap_or_default();
    let name = v["Self"]["HostName"]
        .as_str()
        .unwrap_or(shuttli_brand::NAME)
        .chars()
        .filter(|c| !c.is_control())
        .take(64)
        .collect();
    Ok(Tailnet { own, name, peers })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn candidates_never_include_lan_or_offline() {
        let n=parse(br#"{"BackendState":"Running","TailscaleIPs":["100.64.0.1"],"Self":{"HostName":"A"},"Peer":{"a":{"Online":true,"TailscaleIPs":["10.0.0.1"]},"b":{"Online":true,"TailscaleIPs":["100.64.0.2"]},"c":{"Online":false,"TailscaleIPs":["100.64.0.3"]}}}"#).unwrap();
        assert_eq!(n.peers.len(), 1);
    }
}
