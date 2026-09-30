//! Bounded TLS 1.3 transport. Identity is checked again before any OFFER.
use crate::{
    content::payload,
    discovery,
    identity::{Identity, device_id},
};
use shuttli_core::sync::Publication;
use shuttli_model::mobile::{Capabilities, PeerHint, PeerList};
use shuttli_model::sync::*;
use shuttli_ports::sync::{Network, NetworkEvent, Payload, Result};
use shuttli_protocol::{FrameV2, Hello, decode_hello, encode_hello};
use shuttli_transport::{read_frame as read_v2_frame, write_frame as write_v2_frame};
use std::{
    collections::{HashMap, VecDeque},
    net::{IpAddr, SocketAddr},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{Mutex, Semaphore},
    time::timeout,
};
use tokio_rustls::{TlsAcceptor, TlsConnector};
const PORT: u16 = 45987;
const DEADLINE: Duration = Duration::from_secs(20);
use shuttli_protocol::FrameV1 as Frame;
async fn write_hello(s: &mut (impl AsyncWrite + Unpin), hello: &Hello) -> Result<()> {
    let bytes = encode_hello(hello).map_err(str::to_owned)?;
    s.write_u32(bytes.len() as u32)
        .await
        .map_err(|e| e.to_string())?;
    s.write_all(&bytes).await.map_err(|e| e.to_string())?;
    s.flush().await.map_err(|e| e.to_string())
}
async fn read_hello(s: &mut (impl AsyncRead + Unpin)) -> Result<Hello> {
    let len = s.read_u32().await.map_err(|e| e.to_string())? as usize;
    if len == 0 || len > shuttli_protocol::MAX_CONTROL_FRAME_BYTES {
        return Err("invalid hello size".into());
    }
    let mut bytes = vec![0; len];
    s.read_exact(&mut bytes).await.map_err(|e| e.to_string())?;
    decode_hello(&bytes).map_err(str::to_owned)
}
/// Time out idle polls before consuming bytes. Once a frame starts, finish it
/// within the transfer deadline or close the session rather than retry mid-frame.
async fn read_v2_frame_or_idle(
    stream: &mut (impl AsyncRead + Unpin),
    idle: Duration,
) -> Result<Option<FrameV2>> {
    let mut first = [0u8; 1];
    match timeout(idle, stream.read(&mut first)).await {
        Err(_) => return Ok(None),
        Ok(Err(error)) => return Err(error.to_string()),
        Ok(Ok(0)) => return Err("history connection closed".into()),
        Ok(Ok(_)) => {}
    }
    let mut prefixed = std::io::Cursor::new(first).chain(stream);
    timeout(DEADLINE, read_v2_frame(&mut prefixed))
        .await
        .map_err(|_| "v2 frame timed out")?
        .map(Some)
}

async fn write_frame(s: &mut (impl AsyncWrite + Unpin), v: &Frame) -> Result<()> {
    let b = serde_json::to_vec(v).map_err(|e| e.to_string())?;
    if b.len() > 16384 {
        return Err("frame too large".into());
    }
    s.write_u32(b.len() as u32)
        .await
        .map_err(|e| e.to_string())?;
    s.write_all(&b).await.map_err(|e| e.to_string())?;
    s.flush().await.map_err(|e| e.to_string())
}
async fn read_frame(s: &mut (impl AsyncRead + Unpin)) -> Result<Frame> {
    let n = s.read_u32().await.map_err(|e| e.to_string())? as usize;
    if n == 0 || n > 16384 {
        return Err("invalid frame size".into());
    }
    let mut b = vec![0; n];
    s.read_exact(&mut b).await.map_err(|e| e.to_string())?;
    serde_json::from_slice(&b).map_err(|_| "invalid protocol frame".into())
}
fn peer(id: DeviceId, name: String, address: SocketAddr) -> PeerInfo {
    PeerInfo {
        id: hex::encode(id),
        name: name.chars().filter(|c| !c.is_control()).take(64).collect(),
        address: address.to_string(),
        online: true,
        capabilities: Capabilities::legacy_desktop(),
    }
}
struct Outbound {
    permit: Publication,
    payload: Payload,
    queued: Instant,
}
struct Session {
    epoch: [u8; 16],
    initiator: DeviceId,
    live: Arc<std::sync::atomic::AtomicBool>,
    peer: PeerInfo,
    capabilities: Capabilities,
}
#[derive(Clone)]
struct Query {
    peer: DeviceId,
    event: EventId,
    attempts: u8,
}
struct Shared {
    epoch: [u8; 16],
    identity: Arc<Identity>,
    name: String,
    events: tokio::sync::mpsc::Sender<NetworkEvent>,
    revision: Arc<AtomicU64>,
    sessions: Mutex<HashMap<DeviceId, Session>>,
    pending: Mutex<HashMap<DeviceId, Outbound>>,
    queries: Mutex<VecDeque<Query>>,
    budget: Mutex<(Instant, u32)>,
    receive_slots: Semaphore,
}
pub struct TailscaleNetwork {
    events: tokio::sync::mpsc::Receiver<NetworkEvent>,
    send: tokio::sync::mpsc::Sender<Outbound>,
    revision: Arc<AtomicU64>,
    refresh: Arc<tokio::sync::Notify>,
    queries: tokio::sync::mpsc::Sender<Query>,
    ready: Arc<AtomicBool>,
}
impl TailscaleNetwork {
    pub fn open(identity: Identity, epoch: [u8; 16]) -> Result<Self> {
        let (events_tx, events) = tokio::sync::mpsc::channel(64);
        let (send, sends) = tokio::sync::mpsc::channel::<Outbound>(16);
        let (queries, query_rx) = tokio::sync::mpsc::channel::<Query>(32);
        let revision = Arc::new(AtomicU64::new(1));
        let refresh = Arc::new(tokio::sync::Notify::new());
        let refresh_worker = refresh.clone();
        let revision_worker = revision.clone();
        let ready = Arc::new(AtomicBool::new(false));
        let ready_worker = ready.clone();
        std::thread::Builder::new()
            .name("shuttli-network".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .max_blocking_threads(4)
                    .build()
                    .expect("runtime");
                runtime.block_on(async move {
                    // A graphical login can precede VPN readiness. Keep local
                    // clipboard/settings IPC available and retry off its thread.
                    // No clipboard bodies are queued while startup is offline.
                    let mut waiting_reported = false;
                    let (net, listener) = loop {
                        if sends.is_closed() {
                            return;
                        }
                        if let Ok(Ok(pair)) = tokio::task::spawn_blocking(listener_snapshot).await {
                            break pair;
                        }
                        if !waiting_reported {
                            eprintln!("{}: waiting for Tailscale transport; local controls remain available", shuttli_brand::NAME);
                            waiting_reported = true;
                        }
                        tokio::select! {
                            _ = tokio::time::sleep(Duration::from_secs(5)) => {},
                            _ = refresh_worker.notified() => {},
                        }
                    };
                    let shared = Arc::new(Shared {
                        epoch,
                        identity: Arc::new(identity),
                        name: net.name,
                        events: events_tx,
                        revision: revision_worker,
                        sessions: Mutex::new(HashMap::new()),
                        pending: Mutex::new(HashMap::new()),
                        queries: Mutex::new(VecDeque::new()),
                        budget: Mutex::new((Instant::now(), 0)),
                        receive_slots: Semaphore::new(2),
                    });
                    ready_worker.store(true, Ordering::SeqCst);
                    run_network(shared, listener, sends, query_rx, refresh_worker, net.peers).await;
                    ready_worker.store(false, Ordering::SeqCst);
                });
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            events,
            send,
            revision,
            refresh,
            queries,
            ready,
        })
    }
}
fn listener_snapshot() -> Result<(discovery::Tailnet, std::net::TcpListener)> {
    let net = discovery::snapshot()?;
    let listener = std::net::TcpListener::bind((net.own, PORT))
        .map_err(|e| format!("Tailscale listener: {e}"))?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    Ok((net, listener))
}
async fn run_network(
    shared: Arc<Shared>,
    listener: std::net::TcpListener,
    mut sends: tokio::sync::mpsc::Receiver<Outbound>,
    mut queries: tokio::sync::mpsc::Receiver<Query>,
    refresh: Arc<tokio::sync::Notify>,
    candidates: Vec<IpAddr>,
) {
    let listener = TcpListener::from_std(listener).expect("listener");
    let slots = Arc::new(Semaphore::new(16));
    let mut maintenance = tokio::time::interval(Duration::from_secs(1));
    let discovery = shared.clone();
    tokio::spawn(async move {
        discover(discovery, candidates, refresh).await;
    });
    let mut handshakes = (Instant::now(), 0u32);
    loop {
        tokio::select! {
            query=queries.recv()=>{
                if let Some(query)=query {
                    if query.event.origin==shared.identity.id {enqueue_query(&shared,query).await;}
                }
            },
            _=maintenance.tick()=>{
                let mut pending=shared.pending.lock().await;
                let expired:Vec<_>=pending.iter().filter(|(_,v)|v.queued.elapsed()>DEADLINE || v.permit.policy_revision()!=shared.revision.load(Ordering::SeqCst)).map(|(id,_)|*id).collect();
                for id in expired {
                    if let Some(out)=pending.remove(&id){report(&shared,&out,DeliveryState::Cancelled,"queued transfer expired or permission changed".into()).await;}
                }
            },
            incoming=listener.accept()=>{
                if let Ok((stream,address))=incoming {
                    if !tail_address(address.ip()) {continue;}
                    if handshakes.0.elapsed()>Duration::from_secs(60){handshakes=(Instant::now(),0);}
                    if handshakes.1>=120{continue;}
                    handshakes.1+=1;
                    if let Ok(slot)=slots.clone().try_acquire_owned(){
                        let state=shared.clone();
                        tokio::spawn(async move{let _slot=slot;let _=incoming_connection(state,stream,address).await;});
                    }
                }
            },
            outgoing=sends.recv()=>{
                let Some(out)=outgoing else{break};
                let target=out.permit.target();
                let mut pending=shared.pending.lock().await;
                if pending.len()>=16 && !pending.contains_key(&target){
                    report(&shared,&out,DeliveryState::Failed,"outgoing peer capacity reached".into()).await;continue;
                }
                if let Some(old)=pending.insert(target,out){report(&shared,&old,DeliveryState::Superseded,"superseded by a newer copy".into()).await;}
            }
        }
    }
}
fn tail_address(ip: IpAddr) -> bool {
    matches!(ip,IpAddr::V4(a)if a.octets()[0]==100&&(64..=127).contains(&a.octets()[1]))
}
impl Network for TailscaleNetwork {
    fn poll(&mut self) -> Option<NetworkEvent> {
        self.events.try_recv().ok()
    }
    fn send(&mut self, permit: Publication, payload: Payload) -> Result<()> {
        if !self.ready.load(Ordering::SeqCst) {
            return Err(
                "Tailscale transport is not ready; refresh discovery after network recovery".into(),
            );
        }
        if permit.metadata() != &payload.meta {
            return Err("publication content mismatch".into());
        }
        self.send
            .try_send(Outbound {
                permit,
                payload,
                queued: Instant::now(),
            })
            .map_err(|_| "outgoing queue full".into())
    }
    fn policy_revision(&mut self, r: u64) {
        self.revision.store(r, Ordering::SeqCst);
    }
    fn reconcile(&mut self, peer: DeviceId, event: EventId) -> Result<()> {
        self.queries
            .try_send(Query {
                peer,
                event,
                attempts: 0,
            })
            .map_err(|_| "receipt query queue full".into())
    }
    fn refresh(&mut self) {
        self.refresh.notify_one();
    }
}
trait Duplex: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Duplex for T {}
async fn selected(
    s: &Shared,
    tls: &mut impl Duplex,
    mut info: PeerInfo,
    initiator: DeviceId,
    id: DeviceId,
    epoch: [u8; 16],
    capabilities: Capabilities,
) -> Result<Arc<std::sync::atomic::AtomicBool>> {
    let live = Arc::new(std::sync::atomic::AtomicBool::new(true));
    info.capabilities = capabilities;
    {
        let mut sessions = s.sessions.lock().await;
        sessions.retain(|_, v| v.live.load(Ordering::SeqCst));
        if id == s.identity.id || (sessions.len() >= 16 && !sessions.contains_key(&id)) {
            return Err("session capacity or self connection".into());
        }
        if let Some(old) = sessions.get(&id) {
            if old.live.load(Ordering::SeqCst) && old.initiator <= initiator {
                return Err("preferred session already active".into());
            }
            old.live.store(false, Ordering::SeqCst);
        }
        sessions.insert(
            id,
            Session {
                epoch,
                initiator,
                live: live.clone(),
                peer: info.clone(),
                capabilities,
            },
        );
    }
    let result = async {
        write_frame(tls, &Frame::Select { initiator }).await?;
        match read_frame(tls).await? {
            Frame::Select { initiator: other } if other == initiator => Ok(()),
            _ => Err("session selection mismatch".into()),
        }
    };
    if let Err(e) = timeout(Duration::from_secs(5), result)
        .await
        .unwrap_or_else(|_| Err("selection timeout".into()))
    {
        live.store(false, Ordering::SeqCst);
        return Err(e);
    }
    s.events
        .try_send(NetworkEvent::Peer(info))
        .map_err(|_| "application busy")?;
    Ok(live)
}
#[derive(Debug)]
enum ConnectFailure {
    HelloVersion(DeviceId),
    Other(String),
}
impl From<String> for ConnectFailure {
    fn from(error: String) -> Self {
        Self::Other(error)
    }
}
impl From<&str> for ConnectFailure {
    fn from(error: &str) -> Self {
        Self::Other(error.into())
    }
}
async fn connect(s: Arc<Shared>, address: SocketAddr) -> Result<()> {
    match connect_attempt(s.clone(), address, true, None).await {
        Ok(()) => Ok(()),
        Err(ConnectFailure::HelloVersion(identity)) => {
            connect_attempt(s, address, false, Some(identity))
                .await
                .map_err(|failure| match failure {
                    ConnectFailure::Other(error) => error,
                    ConnectFailure::HelloVersion(_) => "HELLO negotiation failed".into(),
                })
        }
        Err(ConnectFailure::Other(error)) => Err(error),
    }
}
async fn connect_attempt(
    s: Arc<Shared>,
    address: SocketAddr,
    prefer_v2: bool,
    expected_id: Option<DeviceId>,
) -> std::result::Result<(), ConnectFailure> {
    let mut tls = timeout(Duration::from_secs(5), async {
        let stream = TcpStream::connect(address)
            .await
            .map_err(|e| e.to_string())?;
        stream.set_nodelay(true).map_err(|e| e.to_string())?;
        TlsConnector::from(s.identity.client.clone())
            .connect(
                rustls::pki_types::ServerName::try_from("shuttli.local")
                    .expect("constant hostname"),
                stream,
            )
            .await
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|_| "connection timed out")??;
    let id = device_id(
        tls.get_ref()
            .1
            .peer_certificates()
            .and_then(|c| c.first())
            .ok_or("missing server identity")?
            .as_ref(),
    )?;
    if expected_id.is_some_and(|expected| expected != id) {
        return Err("retry identity mismatch".into());
    }
    let (name, remote_epoch, capabilities, v2) = timeout(Duration::from_secs(5), async {
        write_hello(
            &mut tls,
            &if prefer_v2 {
                Hello::V3 {
                    name: s.name.clone(),
                    epoch: s.epoch,
                    capabilities: Capabilities::live(),
                }
            } else {
                Hello::V1 {
                    name: s.name.clone(),
                    epoch: s.epoch,
                }
            },
        )
        .await?;
        let remote = match read_hello(&mut tls).await {
            Ok(hello) => hello,
            Err(_) if prefer_v2 => return Err(ConnectFailure::HelloVersion(id)),
            Err(error) => return Err(ConnectFailure::Other(error)),
        };
        match remote {
            Hello::V1 { name, epoch } => {
                Ok::<_, ConnectFailure>((name, epoch, Capabilities::legacy_desktop(), false))
            }
            Hello::V3 {
                name,
                epoch,
                capabilities,
            } => Ok((name, epoch, capabilities, true)),
            Hello::V2 { .. } => Err(ConnectFailure::HelloVersion(id)),
        }
    })
    .await
    .map_err(|_| "HELLO timed out")??;
    let live = selected(
        &s,
        &mut tls,
        peer(id, name, address),
        s.identity.id,
        id,
        remote_epoch,
        capabilities,
    )
    .await?;
    if v2 {
        let result = serve_v2_session(&s, &mut tls, id, &live).await;
        disconnected(&s, id, &live).await;
        return result.map_err(ConnectFailure::Other);
    }
    let result = async {
        while live.load(Ordering::SeqCst) {
            if let Some(query) = take_query(&s, id).await {
                query_transfer(&s, &mut tls, query).await?;
            }
            let pending = s.pending.lock().await.remove(&id);
            if let Some(out) = pending {
                let r = timeout(DEADLINE, send_transfer(&s, &mut tls, &out))
                    .await
                    .unwrap_or_else(|_| Err("transfer timed out; outcome unknown".into()));
                if r.is_err() {
                    enqueue_query(
                        &s,
                        Query {
                            peer: out.permit.target(),
                            event: out.permit.event(),
                            attempts: 0,
                        },
                    )
                    .await;
                }
                report_transfer(&s, &out, &r).await;
                r?;
            }
            write_frame(&mut tls, &Frame::Poll).await?;
            match timeout(DEADLINE, read_frame(&mut tls))
                .await
                .map_err(|_| "session idle timeout")??
            {
                Frame::Idle => {}
                Frame::Status { event } => answer_query(&s, &mut tls, id, event).await?,
                Frame::Offer { event, meta } => timeout(
                    DEADLINE,
                    receive_transfer(&s, &mut tls, id, event, meta, false),
                )
                .await
                .map_err(|_| "receive timed out")??,
                _ => return Err("unexpected poll response".into()),
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        Ok(())
    }
    .await;
    disconnected(&s, id, &live).await;
    result.map_err(ConnectFailure::Other)
}
async fn discover(s: Arc<Shared>, mut addresses: Vec<IpAddr>, refresh: Arc<tokio::sync::Notify>) {
    let attempts = Arc::new(Semaphore::new(16));
    loop {
        for address in &addresses {
            let address = SocketAddr::new(*address, PORT);
            let connected =
                s.sessions.lock().await.values().any(|v| {
                    v.peer.address == address.to_string() && v.live.load(Ordering::SeqCst)
                });
            if !connected {
                if let Ok(slot) = attempts.clone().try_acquire_owned() {
                    let s = s.clone();
                    tokio::spawn(async move {
                        let _slot = slot;
                        let _ = connect(s, address).await;
                    });
                }
            }
        }
        tokio::select! {_=tokio::time::sleep(Duration::from_secs(30))=>{},_=refresh.notified()=>{}}
        if let Ok(Ok(net)) = tokio::task::spawn_blocking(discovery::snapshot).await {
            addresses = net.peers;
        }
    }
}
async fn incoming_connection(s: Arc<Shared>, stream: TcpStream, address: SocketAddr) -> Result<()> {
    let mut tls = timeout(
        Duration::from_secs(5),
        TlsAcceptor::from(s.identity.server.clone()).accept(stream),
    )
    .await
    .map_err(|_| "TLS timeout")?
    .map_err(|e| e.to_string())?;
    let id = device_id(
        tls.get_ref()
            .1
            .peer_certificates()
            .and_then(|c| c.first())
            .ok_or("missing client identity")?
            .as_ref(),
    )?;
    let (name, remote_epoch, capabilities, v2) = timeout(Duration::from_secs(5), async {
        let remote = read_hello(&mut tls).await?;
        let (name, epoch, capabilities, reply) = match remote {
            Hello::V1 { name, epoch } => (
                name,
                epoch,
                Capabilities::legacy_desktop(),
                Hello::V1 {
                    name: s.name.clone(),
                    epoch: s.epoch,
                },
            ),
            Hello::V3 {
                name,
                epoch,
                capabilities,
            } => (
                name,
                epoch,
                capabilities,
                Hello::V3 {
                    name: s.name.clone(),
                    epoch: s.epoch,
                    capabilities: Capabilities::live(),
                },
            ),
            Hello::V2 {
                name,
                epoch,
                capabilities,
            } => (
                name,
                epoch,
                capabilities,
                Hello::V2 {
                    name: s.name.clone(),
                    epoch: s.epoch,
                    capabilities: shuttli_model::mobile::Capabilities::desktop(),
                },
            ),
        };
        let v2 = matches!(reply, Hello::V2 { .. } | Hello::V3 { .. });
        write_hello(&mut tls, &reply).await?;
        Ok::<_, String>((name, epoch, capabilities, v2))
    })
    .await
    .map_err(|_| "HELLO timeout")??;
    let live = selected(
        &s,
        &mut tls,
        peer(id, name, SocketAddr::new(address.ip(), PORT)),
        id,
        id,
        remote_epoch,
        capabilities,
    )
    .await?;
    if v2 {
        let result = serve_v2_session(&s, &mut tls, id, &live).await;
        disconnected(&s, id, &live).await;
        return result;
    }
    let result = async {
        while live.load(Ordering::SeqCst) {
            match timeout(DEADLINE, read_frame(&mut tls))
                .await
                .map_err(|_| "session idle timeout")??
            {
                Frame::Offer { event, meta } => timeout(
                    DEADLINE,
                    receive_transfer(&s, &mut tls, id, event, meta, false),
                )
                .await
                .map_err(|_| "receive timed out")??,
                Frame::Status { event } => answer_query(&s, &mut tls, id, event).await?,
                Frame::Poll => {
                    if let Some(query) = take_query(&s, id).await {
                        query_transfer(&s, &mut tls, query).await?;
                        continue;
                    }
                    let out = s.pending.lock().await.remove(&id);
                    if let Some(out) = out {
                        let r = timeout(DEADLINE, send_transfer(&s, &mut tls, &out))
                            .await
                            .unwrap_or_else(|_| Err("transfer timed out; outcome unknown".into()));
                        if r.is_err() {
                            enqueue_query(
                                &s,
                                Query {
                                    peer: out.permit.target(),
                                    event: out.permit.event(),
                                    attempts: 0,
                                },
                            )
                            .await;
                        }
                        report_transfer(&s, &out, &r).await;
                        r?;
                    } else {
                        write_frame(&mut tls, &Frame::Idle).await?;
                    }
                }
                _ => return Err("unexpected session frame".into()),
            }
        }
        Ok(())
    }
    .await;
    disconnected(&s, id, &live).await;
    result
}
async fn disconnected(s: &Shared, id: DeviceId, live: &Arc<std::sync::atomic::AtomicBool>) {
    live.store(false, Ordering::SeqCst);
    let mut sessions = s.sessions.lock().await;
    if sessions
        .get(&id)
        .is_some_and(|v| Arc::ptr_eq(&v.live, live))
    {
        if let Some(mut v) = sessions.remove(&id) {
            v.peer.online = false;
            drop(sessions);
            let _ = s.events.send(NetworkEvent::Peer(v.peer)).await;
        }
    }
}
async fn application_reply<T: Send + 'static>(rx: mpsc::Receiver<Result<T>>) -> Result<T> {
    tokio::task::spawn_blocking(move || rx.recv_timeout(Duration::from_secs(5)))
        .await
        .map_err(|_| "application reply worker stopped")?
        .map_err(|_| "application reply timed out")?
}
async fn can_send_hints(s: &Shared, peer: DeviceId) -> Result<u64> {
    let (reply, rx) = mpsc::sync_channel(1);
    s.events
        .try_send(NetworkEvent::PeerHintsPermission { peer, reply })
        .map_err(|_| "application busy")?;
    application_reply(rx).await
}
fn history_error_code(error: &str) -> &'static str {
    match error {
        "Disabled" => "history_denied",
        "stale history cursor" => "history_cursor_stale",
        _ => "history_unavailable",
    }
}

async fn visible_history_revision(s: &Shared, peer: DeviceId) -> Result<(u64, u64)> {
    let (reply, rx) = mpsc::sync_channel(1);
    s.events
        .try_send(NetworkEvent::HistoryRevisionQuery { peer, reply })
        .map_err(|_| "application busy")?;
    application_reply(rx).await
}
async fn direct_roster(s: &Shared, receiver: DeviceId) -> Vec<PeerHint> {
    s.sessions
        .lock()
        .await
        .iter()
        .filter(|(id, session)| **id != receiver && session.live.load(Ordering::SeqCst))
        .take(shuttli_model::mobile::MAX_PEER_HINTS)
        .map(|(id, session)| PeerHint {
            id: *id,
            endpoint: session.peer.address.clone(),
            capabilities: session.capabilities,
        })
        .collect()
}
async fn serve_v2_session(
    s: &Shared,
    tls: &mut impl Duplex,
    id: DeviceId,
    live: &Arc<AtomicBool>,
) -> Result<()> {
    let capabilities = s
        .sessions
        .lock()
        .await
        .get(&id)
        .ok_or("missing v2 session")?
        .capabilities;
    let leader =
        capabilities.accept_live_offer && shuttli_transport::leads_session(s.identity.id, id);
    let mut last_roster: Option<Vec<PeerHint>> = None;
    let mut roster_revision = 0u64;
    let mut last_history_revision: Option<(u64, u64)> = None;
    let mut next_roster_check = Instant::now();
    while live.load(Ordering::SeqCst) {
        if !capabilities.accept_live_offer {
            if let Some(out) = s.pending.lock().await.remove(&id) {
                report(
                    s,
                    &out,
                    DeliveryState::Failed,
                    "peer does not accept live offers".into(),
                )
                .await;
            }
        }
        if capabilities.peer_hints && Instant::now() >= next_roster_check {
            next_roster_check = Instant::now() + Duration::from_secs(5);
            match can_send_hints(s, id).await {
                Ok(revision) if revision == s.revision.load(Ordering::SeqCst) => {
                    let roster = direct_roster(s, id).await;
                    if last_roster.as_ref() != Some(&roster) {
                        roster_revision = roster_revision.saturating_add(1);
                        write_v2_frame(
                            tls,
                            &FrameV2::from(PeerList {
                                revision: roster_revision,
                                peers: roster.clone(),
                            }),
                        )
                        .await?;
                        last_roster = Some(roster);
                    }
                }
                _ => last_roster = None,
            }
            if capabilities.history_change {
                match visible_history_revision(s, id).await {
                    Ok(current) if current.0 == s.revision.load(Ordering::SeqCst) => {
                        if last_history_revision != Some(current) {
                            write_v2_frame(
                                tls,
                                &FrameV2::HistoryChanged {
                                    revision: current.1,
                                },
                            )
                            .await?;
                            last_history_revision = Some(current);
                        }
                    }
                    _ => last_history_revision = None,
                }
            }
        }
        if leader {
            if let Some(query) = take_query(s, id).await {
                query_transfer_v2(s, tls, query).await?;
            }
            let pending = s.pending.lock().await.remove(&id);
            if let Some(out) = pending {
                send_pending_v2(s, tls, out).await?;
            }
            write_v2_frame(tls, &FrameV2::Poll).await?;
        }
        let Some(frame) = read_v2_frame_or_idle(
            tls,
            if leader {
                DEADLINE
            } else {
                Duration::from_secs(5)
            },
        )
        .await?
        else {
            if leader {
                return Err("live response timeout".into());
            }
            continue;
        };
        match frame {
            FrameV2::Idle if leader => {}
            FrameV2::Poll if capabilities.accept_live_offer && !leader => {
                if let Some(query) = take_query(s, id).await {
                    query_transfer_v2(s, tls, query).await?;
                } else {
                    let pending = s.pending.lock().await.remove(&id);
                    if let Some(out) = pending {
                        send_pending_v2(s, tls, out).await?;
                    } else {
                        write_v2_frame(tls, &FrameV2::Idle).await?;
                    }
                }
            }
            FrameV2::Status { event } => {
                answer_query_v2(s, tls, id, event).await?;
            }
            FrameV2::HistoryListRequest { cursor, limit } if capabilities.history_pull => {
                let (reply, rx) = mpsc::sync_channel(1);
                s.events
                    .try_send(NetworkEvent::HistoryListQuery {
                        peer: id,
                        cursor,
                        limit,
                        reply,
                    })
                    .map_err(|_| "application busy")?;
                match application_reply(rx).await {
                    Ok((revision, page)) if revision == s.revision.load(Ordering::SeqCst) => {
                        write_v2_frame(tls, &page.into()).await?;
                    }
                    result => {
                        write_v2_frame(
                            tls,
                            &FrameV2::Error {
                                code: result
                                    .err()
                                    .map(|error| history_error_code(&error))
                                    .unwrap_or("history_denied")
                                    .into(),
                            },
                        )
                        .await?
                    }
                }
            }
            FrameV2::HistoryGet { event } if capabilities.history_pull => {
                let (reply, rx) = mpsc::sync_channel(1);
                s.events
                    .try_send(NetworkEvent::HistoryGetQuery {
                        peer: id,
                        event,
                        reply,
                    })
                    .map_err(|_| "application busy")?;
                match application_reply(rx).await {
                    Ok((revision, payload)) if revision == s.revision.load(Ordering::SeqCst) => {
                        write_v2_frame(
                            tls,
                            &FrameV2::HistoryBody {
                                event,
                                metadata: payload.meta.clone(),
                            },
                        )
                        .await?;
                        tls.write_all(&payload.data)
                            .await
                            .map_err(|e| e.to_string())?;
                        tls.flush().await.map_err(|e| e.to_string())?;
                    }
                    result => {
                        write_v2_frame(
                            tls,
                            &FrameV2::Error {
                                code: result
                                    .err()
                                    .map(|error| history_error_code(&error))
                                    .unwrap_or("history_denied")
                                    .into(),
                            },
                        )
                        .await?
                    }
                }
            }
            FrameV2::Offer { event, meta } => {
                timeout(DEADLINE, receive_transfer(s, tls, id, event, meta, true))
                    .await
                    .map_err(|_| "receive timed out")??;
            }
            _ => return Err("unsupported v2 operation".into()),
        }
        if leader {
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
    Ok(())
}
async fn send_pending_v2(s: &Shared, tls: &mut impl Duplex, out: Outbound) -> Result<()> {
    let result = timeout(
        DEADLINE,
        shuttli_transport::send_live(
            tls,
            out.permit.event(),
            out.permit.metadata(),
            &out.payload.data,
            || s.revision.load(Ordering::SeqCst) == out.permit.policy_revision(),
            |_| Err("unexpected notice during transfer".into()),
            || {},
        ),
    )
    .await
    .unwrap_or_else(|_| Err("transfer timed out; outcome unknown".into()));
    let (state, detail) = match &result {
        Ok(shuttli_transport::SendOutcome::Applied) => (
            DeliveryState::Applied,
            "receiver confirmed its reception target".into(),
        ),
        Ok(shuttli_transport::SendOutcome::Rejected) => {
            (DeliveryState::Failed, "receiver rejected transfer".into())
        }
        Err(error) => (DeliveryState::Unknown, error.clone()),
    };
    if result.is_err() {
        enqueue_query(
            s,
            Query {
                peer: out.permit.target(),
                event: out.permit.event(),
                attempts: 0,
            },
        )
        .await;
    }
    report(s, &out, state, detail).await;
    result.map(|_| ())
}

async fn query_transfer_v2(s: &Shared, tls: &mut impl Duplex, mut query: Query) -> Result<()> {
    let result = timeout(Duration::from_secs(5), async {
        write_v2_frame(tls, &FrameV2::Status { event: query.event }).await?;
        match read_v2_frame(tls).await? {
            FrameV2::Receipt { event, state } if event == query.event => {
                Ok(state.unwrap_or(DeliveryState::Unknown))
            }
            _ => Err("invalid receipt response".into()),
        }
    })
    .await
    .unwrap_or_else(|_| Err("receipt query timeout".into()));
    match result {
        Ok(state) => s
            .events
            .send(NetworkEvent::Delivery {
                event: query.event,
                peer: hex::encode(query.peer),
                state,
                detail: if state == DeliveryState::Applied {
                    "receiver receipt recovered without repeating reception"
                } else {
                    "receiver has no confirmed receipt"
                }
                .into(),
            })
            .await
            .map_err(|_| "application stopped".into()),
        Err(error) => {
            if query.attempts < 2 {
                query.attempts += 1;
                enqueue_query(s, query).await;
            }
            Err(error)
        }
    }
}
async fn answer_query_v2(
    s: &Shared,
    tls: &mut impl Duplex,
    peer: DeviceId,
    event: EventId,
) -> Result<()> {
    if event.origin != peer {
        return Err("receipt query owner mismatch".into());
    }
    let (reply, rx) = mpsc::sync_channel(1);
    s.events
        .try_send(NetworkEvent::ReceiptQuery { peer, event, reply })
        .map_err(|_| "application busy")?;
    let state = application_reply(rx).await?;
    write_v2_frame(tls, &FrameV2::Receipt { event, state }).await
}

async fn report(s: &Shared, out: &Outbound, state: DeliveryState, detail: String) {
    // Await capacity: terminal results are never dropped while the service lives.
    // Producers are bounded by session/handshake slots and the outgoing queue.
    let _ = s
        .events
        .send(NetworkEvent::Delivery {
            event: out.permit.event(),
            peer: hex::encode(out.permit.target()),
            state,
            detail,
        })
        .await;
}
fn transfer_outcome(result: &Result<TransferResult>) -> (DeliveryState, String) {
    match result {
        Ok(TransferResult::Applied) => (
            DeliveryState::Applied,
            "receiver confirmed its reception target".into(),
        ),
        Ok(TransferResult::Rejected(message)) => (DeliveryState::Failed, message.clone()),
        Err(message) => (DeliveryState::Unknown, message.clone()),
    }
}
async fn report_transfer(s: &Shared, out: &Outbound, result: &Result<TransferResult>) {
    let (state, detail) = transfer_outcome(result);
    report(s, out, state, detail).await;
}
async fn enqueue_query(s: &Shared, query: Query) {
    let mut q = s.queries.lock().await;
    if q.len() < 64
        && !q
            .iter()
            .any(|x| x.peer == query.peer && x.event == query.event)
    {
        q.push_back(query);
    }
}
async fn take_query(s: &Shared, peer: DeviceId) -> Option<Query> {
    let mut q = s.queries.lock().await;
    let position = q.iter().position(|x| x.peer == peer)?;
    q.remove(position)
}
async fn query_transfer(s: &Shared, tls: &mut impl Duplex, mut query: Query) -> Result<()> {
    let result = timeout(Duration::from_secs(5), async {
        write_frame(tls, &Frame::Status { event: query.event }).await?;
        match read_frame(tls).await? {
            Frame::Receipt { event, state }
                if event == query.event
                    && matches!(
                        state,
                        None | Some(DeliveryState::Applied | DeliveryState::Unknown)
                    ) =>
            {
                Ok(state.unwrap_or(DeliveryState::Unknown))
            }
            _ => Err("invalid receipt response".into()),
        }
    })
    .await
    .unwrap_or_else(|_| Err("receipt query timeout".into()));
    match result {
        Ok(state) => s
            .events
            .send(NetworkEvent::Delivery {
                event: query.event,
                peer: hex::encode(query.peer),
                state,
                detail: if state == DeliveryState::Applied {
                    "durable receiver receipt recovered without reapplying clipboard"
                } else {
                    "receiver has no confirmed applied receipt"
                }
                .into(),
            })
            .await
            .map_err(|_| "application stopped".into()),
        Err(error) => {
            if query.attempts < 2 {
                query.attempts += 1;
                enqueue_query(s, query).await;
            }
            Err(error)
        }
    }
}

async fn answer_query(
    s: &Shared,
    tls: &mut impl Duplex,
    peer: DeviceId,
    event: EventId,
) -> Result<()> {
    if event.origin != peer {
        return Err("receipt query owner mismatch".into());
    }
    {
        let mut budget = s.budget.lock().await;
        if budget.0.elapsed() > Duration::from_secs(60) {
            *budget = (Instant::now(), 0)
        }
        if budget.1 >= 60 {
            return Err("receipt query rate limit".into());
        }
        budget.1 += 1;
    }
    let (tx, rx) = mpsc::sync_channel(1);
    s.events
        .try_send(NetworkEvent::ReceiptQuery {
            peer,
            event,
            reply: tx,
        })
        .map_err(|_| "application busy")?;
    let state = tokio::task::spawn_blocking(move || rx.recv_timeout(Duration::from_secs(3)))
        .await
        .map_err(|_| "application stopped")?
        .map_err(|_| "receipt lookup timeout")??;
    write_frame(tls, &Frame::Receipt { event, state }).await
}
fn safe_remote_error(message: &str) -> String {
    let text: String = message
        .chars()
        .filter(|c| !c.is_control() && !matches!(c,'\u{202a}'..='\u{202e}'|'\u{2066}'..='\u{2069}'))
        .take(160)
        .collect();
    format!("remote rejected transfer: {text}")
}
#[derive(Clone)]
enum TransferResult {
    Applied,
    Rejected(String),
}
async fn send_transfer(
    s: &Shared,
    tls: &mut impl Duplex,
    out: &Outbound,
) -> Result<TransferResult> {
    match shuttli_transport::send_live_version(
        tls,
        shuttli_transport::LiveOffer {
            event: out.permit.event(),
            metadata: out.permit.metadata(),
            body: &out.payload.data,
        },
        || s.revision.load(Ordering::SeqCst) == out.permit.policy_revision(),
        |_| Err("unexpected live notice".into()),
        || {},
        shuttli_transport::WireVersion::V1,
    )
    .await?
    {
        shuttli_transport::SendOutcome::Applied => Ok(TransferResult::Applied),
        shuttli_transport::SendOutcome::Rejected => Ok(TransferResult::Rejected(
            safe_remote_error("receiver denied transfer"),
        )),
    }
}

struct ReceiveGuard {
    permit: Option<tokio::sync::mpsc::OwnedPermit<NetworkEvent>>,
    event: EventId,
    peer: String,
    active: bool,
}
impl Drop for ReceiveGuard {
    fn drop(&mut self) {
        if self.active {
            self.permit
                .take()
                .expect("reserved completion slot")
                .send(NetworkEvent::Delivery {
                    event: self.event,
                    peer: self.peer.clone(),
                    state: DeliveryState::Unknown,
                    detail: "receive interrupted before a confirmed completion".into(),
                });
        }
    }
}
enum ReceiveReply<'a> {
    Ready,
    Applied,
    Error(&'a str),
}
async fn write_receive_reply(
    tls: &mut impl Duplex,
    v2: bool,
    reply: ReceiveReply<'_>,
) -> Result<()> {
    if v2 {
        let frame = match reply {
            ReceiveReply::Ready => FrameV2::Ready,
            ReceiveReply::Applied => FrameV2::Applied,
            ReceiveReply::Error(_) => FrameV2::Error {
                code: "offer_rejected".into(),
            },
        };
        write_v2_frame(tls, &frame).await
    } else {
        let frame = match reply {
            ReceiveReply::Ready => Frame::Ready,
            ReceiveReply::Applied => Frame::Applied,
            ReceiveReply::Error(message) => Frame::Error {
                message: message.into(),
            },
        };
        write_frame(tls, &frame).await
    }
}
async fn receive_transfer(
    s: &Shared,
    tls: &mut impl Duplex,
    id: DeviceId,
    event: EventId,
    meta: Metadata,
    v2: bool,
) -> Result<()> {
    let mut guard = ReceiveGuard {
        permit: Some(
            s.events
                .clone()
                .try_reserve_owned()
                .map_err(|_| "application busy")?,
        ),
        event,
        peer: hex::encode(id),
        active: true,
    };

    {
        let mut b = s.budget.lock().await;
        if b.0.elapsed() > Duration::from_secs(60) {
            *b = (Instant::now(), 0)
        }
        if b.1 >= 60 {
            return Err("global receive rate limit".into());
        }
        b.1 += 1;
    }
    if event.origin != id
        || meta.size > crate::content::MAX_BYTES as u64
        || !s
            .sessions
            .lock()
            .await
            .get(&id)
            .is_some_and(|v| v.epoch == event.epoch && v.live.load(Ordering::SeqCst))
    {
        return Err("invalid offer".into());
    }
    let _memory = match s.receive_slots.try_acquire() {
        Ok(slot) => slot,
        Err(_) => {
            guard.active = false;
            write_receive_reply(
                tls,
                v2,
                ReceiveReply::Error("global receive capacity reached"),
            )
            .await?;
            return Ok(());
        }
    };
    let (tx, rx) = mpsc::sync_channel(1);
    s.events
        .try_send(NetworkEvent::Offer {
            peer: id,
            event,
            meta: meta.clone(),
            reply: tx,
        })
        .map_err(|_| "application busy")?;
    let decision = tokio::task::spawn_blocking(move || rx.recv_timeout(Duration::from_secs(4)))
        .await
        .map_err(|_| "receiver stopped")?
        .map_err(|_| "application timeout")?;
    let ticket = match decision {
        Ok(t) => t,
        Err(e) => {
            write_receive_reply(tls, v2, ReceiveReply::Error(&e)).await?;
            return Ok(());
        }
    };
    write_receive_reply(tls, v2, ReceiveReply::Ready).await?;
    let bytes = shuttli_transport::receive_body(tls, &meta, || {
        s.revision.load(Ordering::SeqCst) == ticket.policy_revision()
    })
    .await?;
    if !s
        .sessions
        .lock()
        .await
        .get(&id)
        .is_some_and(|v| v.epoch == event.epoch && v.live.load(Ordering::SeqCst))
    {
        return Err("session superseded".into());
    }
    let p = payload(meta.format, bytes)?;
    if p.meta != meta {
        return Err("content integrity mismatch".into());
    }
    let (tx, rx) = mpsc::sync_channel(1);
    s.events
        .try_send(NetworkEvent::Received {
            ticket,
            payload: p,
            reply: tx,
        })
        .map_err(|_| "application busy")?;
    let result = tokio::task::spawn_blocking(move || rx.recv_timeout(Duration::from_secs(5)))
        .await
        .map_err(|_| "receiver stopped")?
        .map_err(|_| "application timeout")?;
    guard.active = false;
    match result {
        Ok(()) => write_receive_reply(tls, v2, ReceiveReply::Applied).await,
        Err(e) => write_receive_reply(tls, v2, ReceiveReply::Error(&e)).await,
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn fragmented_v2_request_survives_the_idle_poll_deadline() {
        let (mut remote, mut local) = tokio::io::duplex(1024);
        assert!(
            read_v2_frame_or_idle(&mut local, Duration::from_millis(5))
                .await
                .unwrap()
                .is_none()
        );
        let bytes = FrameV2::HistoryListRequest {
            cursor: None,
            limit: 20,
        }
        .encode()
        .unwrap();
        let mut packet = (bytes.len() as u32).to_be_bytes().to_vec();
        packet.extend(bytes);
        remote.write_all(&packet[..2]).await.unwrap();
        let sender = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            remote.write_all(&packet[2..]).await.unwrap();
        });
        assert!(matches!(
            read_v2_frame_or_idle(&mut local, Duration::from_millis(5))
                .await
                .unwrap(),
            Some(FrameV2::HistoryListRequest {
                cursor: None,
                limit: 20
            })
        ));
        sender.await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn offline_start_does_not_queue_content_and_keeps_latest_permission_revision() {
        let (_, events) = tokio::sync::mpsc::channel(1);
        let (send, mut outgoing) = tokio::sync::mpsc::channel(1);
        let (queries, _) = tokio::sync::mpsc::channel(1);
        let ready = Arc::new(AtomicBool::new(false));
        let revision = Arc::new(AtomicU64::new(1));
        let mut network = TailscaleNetwork {
            events,
            send,
            queries,
            ready: ready.clone(),
            revision: revision.clone(),
            refresh: Arc::new(tokio::sync::Notify::new()),
        };
        let mut core = shuttli_core::sync::SyncCore::new([1; 32], [1; 16], Settings::default());
        core.register([2; 32], "peer".into()).unwrap();
        let mut settings = core.settings().clone();
        settings.peers.insert(
            "peer".into(),
            PeerPolicy {
                send: true,
                ..PeerPolicy::default()
            },
        );
        core.configure(settings).unwrap();
        let body = payload(Format::Text, b"offline synthetic fixture".to_vec()).unwrap();
        let stamp = ClipboardStamp {
            generation: 1,
            digest: body.meta.digest,
            sensitive: false,
        };
        let permit = core.manual(stamp, &body.meta).unwrap().pop().unwrap();
        assert!(
            network
                .send(permit, body.clone())
                .unwrap_err()
                .contains("not ready")
        );
        assert!(outgoing.try_recv().is_err());
        network.policy_revision(9);
        assert_eq!(revision.load(Ordering::SeqCst), 9);
        ready.store(true, Ordering::SeqCst);
        // Recovery cannot resurrect the body rejected during startup. Only a
        // new explicit publication can enter the queue after readiness.
        assert!(outgoing.try_recv().is_err());
        let permit = core.manual(stamp, &body.meta).unwrap().pop().unwrap();
        network.send(permit, body.clone()).unwrap();
        assert_eq!(outgoing.try_recv().unwrap().payload.meta, body.meta);
    }
    #[test]
    fn peer_errors_cannot_inject_terminal_control_sequences() {
        let text = safe_remote_error("\x1b]52;malicious\x07\n\u{202e}error");
        assert!(!text.contains('\x1b'));
        assert!(!text.contains('\n'));
        assert!(!text.contains('\u{202e}'));
    }
    #[tokio::test(flavor = "current_thread")]
    async fn rejects_oversized_control_before_allocating() {
        let (mut a, mut b) = tokio::io::duplex(32);
        a.write_u32(16385).await.unwrap();
        assert!(read_frame(&mut b).await.is_err());
    }
    #[tokio::test(flavor = "current_thread")]
    async fn framed_hello_keeps_v1_and_v2_separate() {
        let (mut writer, mut reader) = tokio::io::duplex(1024);
        write_hello(
            &mut writer,
            &Hello::V1 {
                name: "old".into(),
                epoch: [1; 16],
            },
        )
        .await
        .unwrap();
        assert_eq!(
            read_hello(&mut reader).await.unwrap(),
            Hello::V1 {
                name: "old".into(),
                epoch: [1; 16]
            }
        );
        write_hello(
            &mut writer,
            &Hello::V2 {
                name: "phone".into(),
                epoch: [2; 16],
                capabilities: Capabilities::pull_only(),
            },
        )
        .await
        .unwrap();
        assert_eq!(
            read_hello(&mut reader).await.unwrap(),
            Hello::V2 {
                name: "phone".into(),
                epoch: [2; 16],
                capabilities: Capabilities::pull_only(),
            }
        );
    }
    #[test]
    fn unknown_fields_and_versions_are_rejected() {
        assert!(
            decode_hello(br#"{"type":"hello","version":1,"name":"n","required":"unknown"}"#)
                .is_err()
        );
        assert!(decode_hello(br#"{"type":"hello","version":4}"#).is_err());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn v2_pull_session_sends_roster_page_and_exact_body() {
        let (identity, _) = Identity::generate().unwrap();
        let desktop = identity.id;
        let phone = [9; 32];
        let (events, mut event_rx) = tokio::sync::mpsc::channel(8);
        let shared = Arc::new(Shared {
            epoch: [1; 16],
            identity: Arc::new(identity),
            name: "Desktop".into(),
            events,
            revision: Arc::new(AtomicU64::new(1)),
            sessions: Mutex::new(HashMap::new()),
            pending: Mutex::new(HashMap::new()),
            queries: Mutex::new(VecDeque::new()),
            budget: Mutex::new((Instant::now(), 0)),
            receive_slots: Semaphore::new(2),
        });
        let live = Arc::new(AtomicBool::new(true));
        shared.sessions.lock().await.insert(
            phone,
            Session {
                epoch: [2; 16],
                initiator: phone,
                live: live.clone(),
                peer: peer(
                    phone,
                    "Phone".into(),
                    "100.100.100.9:45987".parse().unwrap(),
                ),
                capabilities: Capabilities::pull_only(),
            },
        );
        let body = payload(Format::Text, b"fixture".to_vec()).unwrap();
        let event = EventId {
            origin: desktop,
            epoch: [1; 16],
            seq: 1,
        };
        let response = shuttli_model::mobile::HistoryListResponse {
            source_epoch: [1; 16],
            revision: 1,
            items: vec![shuttli_model::mobile::HistorySummary {
                event,
                metadata: body.meta.clone(),
                copied_at_ms: 1,
                body_available: true,
            }],
            next: None,
        };
        let worker = tokio::spawn(async move {
            while let Some(request) = event_rx.recv().await {
                match request {
                    NetworkEvent::PeerHintsPermission { reply, .. } => {
                        let _ = reply.send(Ok(1));
                    }
                    NetworkEvent::HistoryRevisionQuery { reply, .. } => {
                        let _ = reply.send(Ok((1, 1)));
                    }
                    NetworkEvent::HistoryListQuery { reply, .. } => {
                        let _ = reply.send(Ok((1, response.clone())));
                    }
                    NetworkEvent::HistoryGetQuery { reply, .. } => {
                        let _ = reply.send(Ok((1, body.clone())));
                    }
                    _ => panic!("unexpected application event"),
                }
            }
        });
        let (mut server, mut client) = tokio::io::duplex(65_536);
        let running = shared.clone();
        let active = live.clone();
        let server_task =
            tokio::spawn(
                async move { serve_v2_session(&running, &mut server, phone, &active).await },
            );
        assert!(
            matches!(read_v2_frame(&mut client).await.unwrap(), FrameV2::PeerList { revision: 1, peers } if peers.is_empty())
        );
        assert!(matches!(
            read_v2_frame(&mut client).await.unwrap(),
            FrameV2::HistoryChanged { revision: 1 }
        ));
        write_v2_frame(
            &mut client,
            &FrameV2::HistoryListRequest {
                cursor: None,
                limit: 20,
            },
        )
        .await
        .unwrap();
        assert!(
            matches!(read_v2_frame(&mut client).await.unwrap(), FrameV2::HistoryListResponse { items, .. } if items.len() == 1 && items[0].event == event)
        );
        write_v2_frame(&mut client, &FrameV2::HistoryGet { event })
            .await
            .unwrap();
        let length = match read_v2_frame(&mut client).await.unwrap() {
            FrameV2::HistoryBody {
                event: received,
                metadata,
            } => {
                assert_eq!(received, event);
                metadata.size as usize
            }
            _ => panic!("expected body header"),
        };
        let mut bytes = vec![0; length];
        client.read_exact(&mut bytes).await.unwrap();
        assert_eq!(bytes, b"fixture");
        server_task.abort();
        worker.abort();
    }
}

#[cfg(test)]
mod delivery_tests {
    use super::*;
    fn fixture() -> (
        Arc<Shared>,
        tokio::sync::mpsc::Receiver<NetworkEvent>,
        Outbound,
    ) {
        let random = crate::random_epoch().unwrap();
        let dir = std::env::temp_dir().join(format!("shuttli-delivery-{random:?}"));
        std::fs::create_dir_all(&dir).unwrap();
        let identity = Arc::new(crate::identity::load(&dir).unwrap());
        std::fs::remove_dir_all(dir).unwrap();
        let (events, receiver) = tokio::sync::mpsc::channel(64);
        let shared = Arc::new(Shared {
            epoch: [1; 16],
            identity,
            name: "Synthetic".into(),
            events,
            revision: Arc::new(AtomicU64::new(1)),
            sessions: Mutex::new(HashMap::new()),
            pending: Mutex::new(HashMap::new()),
            queries: Mutex::new(VecDeque::new()),
            budget: Mutex::new((Instant::now(), 0)),
            receive_slots: Semaphore::new(2),
        });
        let mut settings = Settings::default();
        settings.peers.insert(
            "peer".into(),
            PeerPolicy {
                send: true,
                ..PeerPolicy::default()
            },
        );
        let mut core = shuttli_core::sync::SyncCore::new([1; 32], [1; 16], settings);
        core.register([2; 32], "peer".into()).unwrap();
        let body = payload(Format::Text, b"synthetic".to_vec()).unwrap();
        let stamp = ClipboardStamp {
            generation: 1,
            digest: body.meta.digest,
            sensitive: false,
        };
        let permit = core.manual(stamp, &body.meta).unwrap().pop().unwrap();
        (
            shared,
            receiver,
            Outbound {
                permit,
                payload: body,
                queued: Instant::now(),
            },
        )
    }
    fn fill(shared: &Shared, n: usize) {
        for _ in 0..n {
            shared
                .events
                .try_send(NetworkEvent::Peer(PeerInfo {
                    id: "02".repeat(32),
                    name: "Synthetic".into(),
                    address: "fixture".into(),
                    online: true,
                    capabilities: Capabilities::legacy_desktop(),
                }))
                .ok()
                .unwrap();
        }
    }
    #[tokio::test(flavor = "current_thread")]
    async fn full_queue_retains_success_until_consumer_has_capacity() {
        let (shared, mut receiver, out) = fixture();
        fill(&shared, 64);
        let producer = tokio::spawn(async move {
            report_transfer(&shared, &out, &Ok(TransferResult::Applied)).await;
        });
        tokio::task::yield_now().await;
        assert!(
            !producer.is_finished(),
            "terminal result must wait for capacity"
        );
        for _ in 0..64 {
            assert!(matches!(receiver.recv().await, Some(NetworkEvent::Peer(_))));
        }
        timeout(Duration::from_secs(1), producer)
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            receiver.recv().await,
            Some(NetworkEvent::Delivery {
                state: DeliveryState::Applied,
                ..
            })
        ));
        assert!(receiver.try_recv().is_err());
    }
    #[tokio::test(flavor = "current_thread")]
    async fn receipt_recovery_survives_a_full_application_queue() {
        let (shared, mut receiver, out) = fixture();
        fill(&shared, 64);
        let query = Query {
            peer: out.permit.target(),
            event: out.permit.event(),
            attempts: 0,
        };
        let (mut client, mut remote) = tokio::io::duplex(4096);
        let producer =
            tokio::spawn(async move { query_transfer(&shared, &mut client, query).await });
        let Frame::Status { event } = read_frame(&mut remote).await.unwrap() else {
            panic!()
        };
        write_frame(
            &mut remote,
            &Frame::Receipt {
                event,
                state: Some(DeliveryState::Applied),
            },
        )
        .await
        .unwrap();
        tokio::task::yield_now().await;
        assert!(!producer.is_finished());
        for _ in 0..64 {
            receiver.recv().await.unwrap();
        }
        timeout(Duration::from_secs(1), producer)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(matches!(
            receiver.recv().await,
            Some(NetworkEvent::Delivery {
                state: DeliveryState::Applied,
                ..
            })
        ));
    }
    #[test]
    fn cancelled_receive_has_reserved_completion_capacity() {
        let (shared, mut receiver, out) = fixture();
        let permit = shared.events.clone().try_reserve_owned().unwrap();
        fill(&shared, 63);
        assert!(shared.events.clone().try_reserve_owned().is_err());
        drop(ReceiveGuard {
            permit: Some(permit),
            event: out.permit.event(),
            peer: "fixture".into(),
            active: true,
        });
        for _ in 0..63 {
            assert!(matches!(receiver.try_recv(), Ok(NetworkEvent::Peer(_))));
        }
        assert!(matches!(
            receiver.try_recv(),
            Ok(NetworkEvent::Delivery {
                state: DeliveryState::Unknown,
                ..
            })
        ));
    }
    #[test]
    fn delivery_classification_is_independent_of_error_wording() {
        for text in [
            "superseded",
            "queued transfer",
            "outcome unknown:",
            "arbitrary localized message",
        ] {
            assert_eq!(
                transfer_outcome(&Ok(TransferResult::Rejected(text.into()))).0,
                DeliveryState::Failed
            );
            assert_eq!(
                transfer_outcome(&Err(text.into())).0,
                DeliveryState::Unknown
            );
        }
    }
}
