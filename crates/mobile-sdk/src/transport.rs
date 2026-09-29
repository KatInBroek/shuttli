//! Foreground-only mobile transport. The caller supplies a Tailscale IPv4
//! address; no VPN/private API or public-interface listener is opened here.
use crate::{MobileHistory, peers::PeerDirectory};
use shuttli_identity::{Identity, device_id};
use shuttli_model::{
    mobile::Capabilities,
    sync::{DeviceId, EventId},
};
use shuttli_protocol::{FrameV2, Hello, decode_hello, encode_hello};
use std::{
    collections::VecDeque,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{Semaphore, watch},
    time::timeout,
};
use tokio_rustls::TlsAcceptor;

const PORT: u16 = 45987;
type Result<T> = std::result::Result<T, String>;

pub fn tailscale_ipv4(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    octets[0] == 100 && (64..=127).contains(&octets[1])
}

pub struct MobileTransport {
    stop: watch::Sender<bool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl MobileTransport {
    pub fn start(
        bind_ip: Ipv4Addr,
        identity: Identity,
        name: String,
        history: Arc<Mutex<MobileHistory>>,
        peers: Arc<Mutex<PeerDirectory>>,
    ) -> Result<Self> {
        if !tailscale_ipv4(bind_ip)
            || name.is_empty()
            || name.len() > 256
            || name.chars().any(char::is_control)
        {
            return Err("invalid foreground listener configuration".into());
        }
        let listener = std::net::TcpListener::bind((bind_ip, PORT)).map_err(|e| e.to_string())?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let (stop, receiver) = watch::channel(false);
        let thread = std::thread::Builder::new()
            .name("shuttli-mobile".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .max_blocking_threads(2)
                    .build()
                    .expect("mobile runtime");
                runtime.block_on(run_listener(
                    listener, identity, name, history, peers, receiver,
                ));
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            stop,
            thread: Some(thread),
        })
    }

    pub fn stop(&mut self) {
        let _ = self.stop.send(true);
        // The listener and sessions observe stop asynchronously. Avoid joining
        // a network thread on a native UI/lifecycle thread.
        self.thread.take();
    }
}

impl Drop for MobileTransport {
    fn drop(&mut self) {
        self.stop();
    }
}

async fn run_listener(
    listener: std::net::TcpListener,
    identity: Identity,
    name: String,
    history: Arc<Mutex<MobileHistory>>,
    peers: Arc<Mutex<PeerDirectory>>,
    mut stop: watch::Receiver<bool>,
) {
    let Ok(listener) = TcpListener::from_std(listener) else {
        return;
    };
    let identity = Arc::new(identity);
    let slots = Arc::new(Semaphore::new(16));
    loop {
        tokio::select! {
            _ = stop.changed() => { if *stop.borrow() { break; } }
            incoming = listener.accept() => {
                let Ok((stream, address)) = incoming else { continue; };
                if !matches!(address.ip(), IpAddr::V4(ip) if tailscale_ipv4(ip)) { continue; }
                let Ok(slot) = slots.clone().try_acquire_owned() else { continue; };
                let identity = identity.clone();
                let history = history.clone();
                let peers = peers.clone();
                let name = name.clone();
                let stop = stop.clone();
                tokio::spawn(async move {
                    let _slot = slot;
                    let _ = accept_desktop(stream, address, identity, name, history, peers, stop).await;
                });
            }
        }
    }
}

async fn accept_desktop(
    stream: TcpStream,
    address: SocketAddr,
    identity: Arc<Identity>,
    name: String,
    history: Arc<Mutex<MobileHistory>>,
    peers: Arc<Mutex<PeerDirectory>>,
    stop: watch::Receiver<bool>,
) -> Result<()> {
    let mut tls = timeout(
        Duration::from_secs(5),
        TlsAcceptor::from(identity.server.clone()).accept(stream),
    )
    .await
    .map_err(|_| "TLS timeout")?
    .map_err(|e| e.to_string())?;
    let remote_id = device_id(
        tls.get_ref()
            .1
            .peer_certificates()
            .and_then(|certs| certs.first())
            .ok_or("missing desktop identity")?
            .as_ref(),
    )?;
    let remote = timeout(Duration::from_secs(5), read_hello(&mut tls))
        .await
        .map_err(|_| "HELLO timeout")??;
    let (remote_name, remote_epoch) = match remote {
        Hello::V1 { name, epoch } | Hello::V2 { name, epoch, .. } => (name, epoch),
    };
    write_hello(
        &mut tls,
        &Hello::V2 {
            name,
            epoch: [0; 16],
            capabilities: Capabilities::pull_only(),
        },
    )
    .await?;
    let selected = read_frame(&mut tls).await?;
    if !matches!(selected, FrameV2::Select { initiator } if initiator == remote_id) {
        return Err("session selection mismatch".into());
    }
    write_frame(
        &mut tls,
        &FrameV2::Select {
            initiator: remote_id,
        },
    )
    .await?;
    let endpoint = SocketAddr::new(address.ip(), PORT).to_string();
    peers
        .lock()
        .map_err(|_| "peer directory unavailable")?
        .observed_direct(remote_id, remote_name, endpoint, Capabilities::desktop())
        .map_err(|e| format!("{e:?}"))?;
    let generation = history
        .lock()
        .map_err(|_| "history unavailable")?
        .generation();
    let result = mobile_session(
        &mut tls,
        remote_id,
        remote_epoch,
        generation,
        history,
        peers.clone(),
        stop,
    )
    .await;
    if let Ok(mut directory) = peers.lock() {
        directory.disconnected(remote_id);
    }
    result
}

async fn mobile_session<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    source: DeviceId,
    source_epoch: [u8; 16],
    generation: u64,
    history: Arc<Mutex<MobileHistory>>,
    peers: Arc<Mutex<PeerDirectory>>,
    mut stop: watch::Receiver<bool>,
) -> Result<()> {
    let mut pending_list = false;
    let mut pending_body: Option<EventId> = None;
    let mut body_queue = VecDeque::new();
    let mut next_cursor = None;
    let mut body_budget = crate::MAX_SESSION_BYTES;
    let mut refresh = tokio::time::interval(Duration::from_secs(15));
    loop {
        tokio::select! {
            _ = stop.changed() => { if *stop.borrow() { return Ok(()); } }
            _ = refresh.tick() => {
                if !pending_list && pending_body.is_none() && peers.lock().map_err(|_| "peer directory unavailable")?
                    .direct().iter().any(|p| p.id == source && p.directions.receive) {
                    body_budget = crate::MAX_SESSION_BYTES;
                    write_frame(stream, &FrameV2::HistoryListRequest { cursor: None, limit: 20 }).await?;
                    pending_list = true;
                }
            }
            frame = read_frame(stream) => {
                match frame? {
                    FrameV2::PeerList { revision, peers: hints } => {
                        peers.lock().map_err(|_| "peer directory unavailable")?
                            .accept_hints(source, shuttli_model::mobile::PeerList { revision, peers: hints }, now_ms())
                            .map_err(|e| format!("{e:?}"))?;
                    }
                    FrameV2::HistoryListResponse { source_epoch: page_epoch, revision, items, next } if pending_list => {
                        if page_epoch != source_epoch { return Err("history source epoch mismatch".into()); }
                        pending_list = false;
                        next_cursor = next;
                        let page = shuttli_model::mobile::HistoryListResponse { source_epoch: page_epoch, revision, items, next };
                        let candidates: Vec<_> = page.items.iter().filter(|i| i.body_available)
                            .map(|i| (i.event, i.metadata.size)).collect();
                        {
                            let mut cache = history.lock().map_err(|_| "history unavailable")?;
                            cache.merge_page(generation, source, page, now_ms()).map_err(|e| format!("{e:?}"))?;
                            for (event, size) in candidates {
                                if cache.body_for_explicit_copy(event).is_none() && size <= body_budget as u64 {
                                    body_budget -= size as usize;
                                    body_queue.push_back(event);
                                }
                            }
                        }
                        if let Some(event) = body_queue.pop_front() {
                            write_frame(stream, &FrameV2::HistoryGet { event }).await?;
                            pending_body = Some(event);
                        } else if let Some(cursor) = next_cursor.take() {
                            write_frame(stream, &FrameV2::HistoryListRequest { cursor: Some(cursor), limit: 20 }).await?;
                            pending_list = true;
                        }
                    }
                    FrameV2::HistoryBody { event, metadata } if pending_body == Some(event) => {
                        if metadata.size > 8 * 1024 * 1024 { return Err("oversized history body".into()); }
                        if history.lock().map_err(|_| "history unavailable")?.timeline()
                            .iter().find(|row| row.summary.event == event)
                            .is_none_or(|row| row.summary.metadata != metadata) {
                            return Err("history body metadata mismatch".into());
                        }
                        let mut bytes = vec![0; metadata.size as usize];
                        timeout(Duration::from_secs(20), stream.read_exact(&mut bytes))
                            .await.map_err(|_| "history body timeout")?.map_err(|e| e.to_string())?;
                        history.lock().map_err(|_| "history unavailable")?
                            .cache_body(generation, event, bytes).map_err(|e| format!("{e:?}"))?;
                        pending_body = None;
                        if let Some(event) = body_queue.pop_front() {
                            write_frame(stream, &FrameV2::HistoryGet { event }).await?;
                            pending_body = Some(event);
                        } else if let Some(cursor) = next_cursor.take() {
                            write_frame(stream, &FrameV2::HistoryListRequest { cursor: Some(cursor), limit: 20 }).await?;
                            pending_list = true;
                        }
                    }
                    FrameV2::HistoryChanged { .. } if !pending_list && pending_body.is_none() => {
                        write_frame(stream, &FrameV2::HistoryListRequest { cursor: None, limit: 20 }).await?;
                        pending_list = true;
                    }
                    FrameV2::Error { .. } => {
                        pending_list = false;
                        pending_body = None;
                        body_queue.clear();
                        next_cursor = None;
                    }
                    _ => return Err("unexpected mobile session frame".into()),
                }
            }
        }
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

async fn write_hello(stream: &mut (impl AsyncWrite + Unpin), hello: &Hello) -> Result<()> {
    let bytes = encode_hello(hello).map_err(str::to_owned)?;
    stream
        .write_u32(bytes.len() as u32)
        .await
        .map_err(|e| e.to_string())?;
    stream.write_all(&bytes).await.map_err(|e| e.to_string())?;
    stream.flush().await.map_err(|e| e.to_string())
}

async fn read_hello(stream: &mut (impl AsyncRead + Unpin)) -> Result<Hello> {
    let len = stream.read_u32().await.map_err(|e| e.to_string())? as usize;
    if len == 0 || len > shuttli_protocol::MAX_CONTROL_FRAME_BYTES {
        return Err("invalid hello size".into());
    }
    let mut bytes = vec![0; len];
    stream
        .read_exact(&mut bytes)
        .await
        .map_err(|e| e.to_string())?;
    decode_hello(&bytes).map_err(str::to_owned)
}

async fn write_frame(stream: &mut (impl AsyncWrite + Unpin), frame: &FrameV2) -> Result<()> {
    let bytes = frame.encode().map_err(str::to_owned)?;
    stream
        .write_u32(bytes.len() as u32)
        .await
        .map_err(|e| e.to_string())?;
    stream.write_all(&bytes).await.map_err(|e| e.to_string())?;
    stream.flush().await.map_err(|e| e.to_string())
}

async fn read_frame(stream: &mut (impl AsyncRead + Unpin)) -> Result<FrameV2> {
    let len = stream.read_u32().await.map_err(|e| e.to_string())? as usize;
    if len == 0 || len > shuttli_protocol::MAX_CONTROL_FRAME_BYTES {
        return Err("invalid v2 frame size".into());
    }
    let mut bytes = vec![0; len];
    stream
        .read_exact(&mut bytes)
        .await
        .map_err(|e| e.to_string())?;
    FrameV2::decode(&bytes).map_err(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use shuttli_model::{
        mobile::HistorySummary,
        sync::{Format, Metadata},
    };

    #[test]
    fn listener_only_accepts_tailnet_ipv4() {
        assert!(tailscale_ipv4("100.64.0.1".parse().unwrap()));
        assert!(tailscale_ipv4("100.127.255.254".parse().unwrap()));
        assert!(!tailscale_ipv4("100.128.0.1".parse().unwrap()));
        assert!(!tailscale_ipv4("192.168.1.2".parse().unwrap()));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn foreground_session_fetches_into_app_cache_without_os_copy() {
        let source = [2; 32];
        let epoch = [3; 16];
        let event = EventId {
            origin: source,
            epoch,
            seq: 1,
        };
        let bytes = b"remote copy";
        let metadata = Metadata {
            format: Format::Text,
            size: bytes.len() as u64,
            digest: Sha256::digest(bytes).into(),
        };
        let history = Arc::new(Mutex::new(MobileHistory::default()));
        let generation = history.lock().unwrap().enter_foreground();
        let peers = Arc::new(Mutex::new(PeerDirectory::new([1; 32])));
        peers
            .lock()
            .unwrap()
            .observed_direct(
                source,
                "Desktop".into(),
                "100.100.100.2:45987".into(),
                Capabilities::desktop(),
            )
            .unwrap();
        let (stop, receiver) = watch::channel(false);
        let (mut phone, mut desktop) = tokio::io::duplex(65_536);
        let cache = history.clone();
        let directory = peers.clone();
        let session = tokio::spawn(async move {
            mobile_session(
                &mut phone, source, epoch, generation, cache, directory, receiver,
            )
            .await
        });
        assert!(matches!(
            timeout(Duration::from_secs(1), read_frame(&mut desktop))
                .await
                .unwrap()
                .unwrap(),
            FrameV2::HistoryListRequest {
                cursor: None,
                limit: 20
            }
        ));
        write_frame(
            &mut desktop,
            &FrameV2::HistoryListResponse {
                source_epoch: epoch,
                revision: 1,
                items: vec![HistorySummary {
                    event,
                    metadata: metadata.clone(),
                    copied_at_ms: 1,
                    body_available: true,
                }],
                next: None,
            },
        )
        .await
        .unwrap();
        assert!(
            matches!(read_frame(&mut desktop).await.unwrap(), FrameV2::HistoryGet { event: e } if e == event)
        );
        write_frame(&mut desktop, &FrameV2::HistoryBody { event, metadata })
            .await
            .unwrap();
        desktop.write_all(bytes).await.unwrap();
        timeout(Duration::from_secs(1), async {
            loop {
                if history
                    .lock()
                    .unwrap()
                    .body_for_explicit_copy(event)
                    .is_some()
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            &*history
                .lock()
                .unwrap()
                .body_for_explicit_copy(event)
                .unwrap(),
            bytes
        );
        stop.send(true).unwrap();
        assert!(session.await.unwrap().is_ok());
    }
}
