//! Bounded TLS 1.3 transport. Identity is checked again before any OFFER.
use crate::{
    content::payload,
    discovery,
    identity::{Identity, device_id},
};
use serde::{Deserialize, Serialize};
use shuttli_core::sync::Publication;
use shuttli_model::sync::*;
use shuttli_ports::sync::{Network, NetworkEvent, Payload, Result};
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
#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum Frame {
    Hello {
        version: u16,
        name: String,
        epoch: [u8; 16],
    },
    Offer {
        event: EventId,
        meta: Metadata,
    },
    Ready,
    Applied,
    Error {
        message: String,
    },
    Select {
        initiator: DeviceId,
    },
    Status {
        event: EventId,
    },
    Receipt {
        event: EventId,
        state: Option<DeliveryState>,
    },
    Poll,
    Idle,
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
    }
}
fn hello_name(f: Frame) -> Result<(String, [u8; 16])> {
    match f {
        Frame::Hello {
            version: 1,
            name,
            epoch,
        } if name.len() <= 256 => Ok((name, epoch)),
        _ => Err("incompatible HELLO".into()),
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
    events: mpsc::SyncSender<NetworkEvent>,
    revision: Arc<AtomicU64>,
    sessions: Mutex<HashMap<DeviceId, Session>>,
    pending: Mutex<HashMap<DeviceId, Outbound>>,
    queries: Mutex<VecDeque<Query>>,
    budget: Mutex<(Instant, u32)>,
    receive_slots: Semaphore,
}
pub struct TailscaleNetwork {
    events: mpsc::Receiver<NetworkEvent>,
    send: tokio::sync::mpsc::Sender<Outbound>,
    revision: Arc<AtomicU64>,
    refresh: Arc<tokio::sync::Notify>,
    queries: tokio::sync::mpsc::Sender<Query>,
    ready: Arc<AtomicBool>,
}
impl TailscaleNetwork {
    pub fn open(identity: Identity, epoch: [u8; 16]) -> Result<Self> {
        let (events_tx, events) = mpsc::sync_channel(64);
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
                    if let Some(out)=pending.remove(&id){report(&shared,&out,Err("queued transfer expired or permission changed".into()));}
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
                    report(&shared,&out,Err("outgoing peer capacity reached".into()));continue;
                }
                if let Some(old)=pending.insert(target,out){report(&shared,&old,Err("superseded by a newer copy".into()));}
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
    info: PeerInfo,
    initiator: DeviceId,
    id: DeviceId,
    epoch: [u8; 16],
) -> Result<Arc<std::sync::atomic::AtomicBool>> {
    let live = Arc::new(std::sync::atomic::AtomicBool::new(true));
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
async fn connect(s: Arc<Shared>, address: SocketAddr) -> Result<()> {
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
    let (name, remote_epoch) = timeout(Duration::from_secs(5), async {
        write_frame(
            &mut tls,
            &Frame::Hello {
                version: 1,
                name: s.name.clone(),
                epoch: s.epoch,
            },
        )
        .await?;
        hello_name(read_frame(&mut tls).await?)
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
    )
    .await?;
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
                report(
                    &s,
                    &out,
                    r.clone()
                        .map_err(|e| format!("outcome unknown: {e}"))
                        .and_then(TransferResult::receipt),
                );
                r?;
            }
            write_frame(&mut tls, &Frame::Poll).await?;
            match timeout(DEADLINE, read_frame(&mut tls))
                .await
                .map_err(|_| "session idle timeout")??
            {
                Frame::Idle => {}
                Frame::Status { event } => answer_query(&s, &mut tls, id, event).await?,
                Frame::Offer { event, meta } => {
                    timeout(DEADLINE, receive_transfer(&s, &mut tls, id, event, meta))
                        .await
                        .map_err(|_| "receive timed out")??
                }
                _ => return Err("unexpected poll response".into()),
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        Ok(())
    }
    .await;
    disconnected(&s, id, &live).await;
    result
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
    let (name, remote_epoch) = timeout(Duration::from_secs(5), async {
        let name = hello_name(read_frame(&mut tls).await?)?;
        write_frame(
            &mut tls,
            &Frame::Hello {
                version: 1,
                name: s.name.clone(),
                epoch: s.epoch,
            },
        )
        .await?;
        Ok::<_, String>(name)
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
    )
    .await?;
    let result = async {
        while live.load(Ordering::SeqCst) {
            match timeout(DEADLINE, read_frame(&mut tls))
                .await
                .map_err(|_| "session idle timeout")??
            {
                Frame::Offer { event, meta } => {
                    timeout(DEADLINE, receive_transfer(&s, &mut tls, id, event, meta))
                        .await
                        .map_err(|_| "receive timed out")??
                }
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
                        report(
                            &s,
                            &out,
                            r.clone()
                                .map_err(|e| format!("outcome unknown: {e}"))
                                .and_then(TransferResult::receipt),
                        );
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
            let _ = s.events.try_send(NetworkEvent::Peer(v.peer));
        }
    }
}
fn report(s: &Shared, out: &Outbound, result: Result<()>) {
    let (state, detail) = match result {
        Ok(()) => (
            DeliveryState::Applied,
            "remote OS readback and durable receipt completed".into(),
        ),
        Err(e) => {
            let state = if e.starts_with("outcome unknown:") {
                DeliveryState::Unknown
            } else if e.starts_with("superseded") {
                DeliveryState::Superseded
            } else if e.starts_with("queued transfer") {
                DeliveryState::Cancelled
            } else {
                DeliveryState::Failed
            };
            (state, e)
        }
    };
    let _ = s.events.try_send(NetworkEvent::Delivery {
        event: out.permit.event(),
        peer: hex::encode(out.permit.target()),
        state,
        detail,
    });
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
                let state = state.unwrap_or(DeliveryState::Unknown);
                let _ = s.events.try_send(NetworkEvent::Delivery {
                    event,
                    peer: hex::encode(query.peer),
                    state,
                    detail: if state == DeliveryState::Applied {
                        "durable receiver receipt recovered without reapplying clipboard"
                    } else {
                        "receiver has no confirmed applied receipt"
                    }
                    .into(),
                });
                Ok(())
            }
            _ => Err("invalid receipt response".into()),
        }
    })
    .await
    .unwrap_or_else(|_| Err("receipt query timeout".into()));
    if result.is_err() && query.attempts < 2 {
        query.attempts += 1;
        enqueue_query(s, query).await;
    }
    result
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
impl TransferResult {
    fn receipt(self) -> Result<()> {
        match self {
            Self::Applied => Ok(()),
            Self::Rejected(e) => Err(e),
        }
    }
}
async fn send_transfer(
    s: &Shared,
    tls: &mut impl Duplex,
    out: &Outbound,
) -> Result<TransferResult> {
    let p = &out.permit;
    let check = || {
        if s.revision.load(Ordering::SeqCst) == p.policy_revision() {
            Ok(())
        } else {
            Err("send permission changed".to_string())
        }
    };
    check()?;
    write_frame(
        tls,
        &Frame::Offer {
            event: p.event(),
            meta: p.metadata().clone(),
        },
    )
    .await?;
    match read_frame(tls).await? {
        Frame::Ready => {}
        Frame::Error { message } => {
            return Ok(TransferResult::Rejected(safe_remote_error(&message)));
        }
        _ => return Err("expected READY".into()),
    }
    for chunk in out.payload.data.chunks(65536) {
        check()?;
        tls.write_all(chunk).await.map_err(|e| e.to_string())?;
    }
    tls.flush().await.map_err(|e| e.to_string())?;
    match read_frame(tls).await? {
        Frame::Applied => Ok(TransferResult::Applied),
        Frame::Error { message } => Ok(TransferResult::Rejected(safe_remote_error(&message))),
        _ => Err("missing APPLIED receipt".into()),
    }
}
struct ReceiveGuard {
    events: mpsc::SyncSender<NetworkEvent>,
    event: EventId,
    peer: String,
    active: bool,
}
impl Drop for ReceiveGuard {
    fn drop(&mut self) {
        if self.active {
            let _ = self.events.try_send(NetworkEvent::Delivery {
                event: self.event,
                peer: self.peer.clone(),
                state: DeliveryState::Unknown,
                detail: "receive interrupted before a confirmed completion".into(),
            });
        }
    }
}
async fn receive_transfer(
    s: &Shared,
    tls: &mut impl Duplex,
    id: DeviceId,
    event: EventId,
    meta: Metadata,
) -> Result<()> {
    let mut guard = ReceiveGuard {
        events: s.events.clone(),
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
            write_frame(
                tls,
                &Frame::Error {
                    message: "global receive capacity reached".into(),
                },
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
            write_frame(tls, &Frame::Error { message: e }).await?;
            return Ok(());
        }
    };
    write_frame(tls, &Frame::Ready).await?;
    let mut bytes = vec![0; meta.size as usize];
    for chunk in bytes.chunks_mut(65536) {
        if s.revision.load(Ordering::SeqCst) != ticket.policy_revision() {
            return Err("receive permission changed".into());
        }
        tls.read_exact(chunk).await.map_err(|e| e.to_string())?;
    }
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
    write_frame(
        tls,
        &match result {
            Ok(()) => Frame::Applied,
            Err(e) => Frame::Error { message: e },
        },
    )
    .await
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test(flavor = "current_thread")]
    async fn offline_start_does_not_queue_content_and_keeps_latest_permission_revision() {
        let (_, events) = mpsc::sync_channel(1);
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
    #[test]
    fn unknown_fields_and_versions_are_rejected() {
        assert!(
            serde_json::from_str::<Frame>(
                r#"{"type":"hello","version":1,"name":"n","required":"unknown"}"#
            )
            .is_err()
        );
        assert!(
            hello_name(Frame::Hello {
                version: 2,
                epoch: [0; 16],
                name: "x".into()
            })
            .is_err()
        );
    }
}
