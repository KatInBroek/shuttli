//! Foreground-only mobile transport. The caller supplies a Tailscale IPv4
//! address; no VPN/private API or public-interface listener is opened here.
use crate::{
    HistoryActivity, MobileHistory,
    peers::{Candidate, PeerDirectory},
};
use shuttli_identity::{Identity, device_id};
use shuttli_model::{
    mobile::Capabilities,
    sync::{DeviceId, EventId, Format, Metadata},
};
use shuttli_protocol::{FrameV2, Hello, decode_hello, encode_hello};
use std::{
    collections::{BTreeMap, VecDeque},
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{Semaphore, mpsc, watch},
    time::timeout,
};
use tokio_rustls::{TlsAcceptor, TlsConnector};

const PORT: u16 = 45987;
type Result<T> = std::result::Result<T, String>;

pub fn tailscale_ipv4(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    octets[0] == 100 && (64..=127).contains(&octets[1])
}

pub struct MobileTransport {
    epoch: [u8; 16],
    stop: watch::Sender<bool>,
    commands: mpsc::Sender<SendCommand>,
    thread: Option<std::thread::JoinHandle<()>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendState {
    Queued,
    Sending,
    Applied,
    Failed,
    Unknown,
}

#[derive(Clone)]
pub struct SendCommand {
    pub event: EventId,
    pub target: DeviceId,
    pub metadata: Metadata,
    pub body: Arc<[u8]>,
}

pub type SendResults = Arc<Mutex<BTreeMap<(EventId, DeviceId), SendState>>>;

impl MobileTransport {
    pub fn start(
        bind_ip: Ipv4Addr,
        identity: Identity,
        name: String,
        history: Arc<Mutex<MobileHistory>>,
        peers: Arc<Mutex<PeerDirectory>>,
        results: SendResults,
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
        let mut epoch = [0; 16];
        getrandom::getrandom(&mut epoch).map_err(|e| e.to_string())?;
        let (stop, receiver) = watch::channel(false);
        let (commands, command_rx) = mpsc::channel(16);
        let context = SessionContext {
            identity: Arc::new(identity),
            name,
            epoch,
            history,
            peers,
            results,
            routes: Arc::new(tokio::sync::Mutex::new(BTreeMap::new())),
            stop: receiver.clone(),
        };
        let thread = std::thread::Builder::new()
            .name("shuttli-mobile".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .max_blocking_threads(2)
                    .build()
                    .expect("mobile runtime");
                runtime.block_on(run_listener(listener, context, command_rx, receiver));
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            epoch,
            stop,
            commands,
            thread: Some(thread),
        })
    }

    pub fn epoch(&self) -> [u8; 16] {
        self.epoch
    }

    pub fn enqueue(&self, command: SendCommand) -> bool {
        self.commands.try_send(command).is_ok()
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
    context: SessionContext,
    mut commands: mpsc::Receiver<SendCommand>,
    mut stop: watch::Receiver<bool>,
) {
    let Ok(listener) = TcpListener::from_std(listener) else {
        return;
    };
    let slots = Arc::new(Semaphore::new(16));
    let mut refresh = tokio::time::interval(Duration::from_secs(15));
    let mut attempts = BTreeMap::<DeviceId, Instant>::new();
    loop {
        tokio::select! {
            _ = stop.changed() => { if *stop.borrow() { break; } }
            command = commands.recv() => {
                let Some(command) = command else { break; };
                let route = context.routes.lock().await.get(&command.target).cloned();
                if route.is_none_or(|route| route.try_send(command.clone()).is_err()) {
                    if let Ok(mut results) = context.results.lock() {
                        results.insert((command.event, command.target), SendState::Failed);
                    }
                }
            }
            _ = refresh.tick() => {
                let candidates = {
                    let Ok(mut directory) = context.peers.lock() else { continue; };
                    directory.expire(now_ms());
                    directory.candidates()
                };
                let mut launched = 0;
                for candidate in candidates {
                    if launched >= 4 { break; }
                    if attempts.get(&candidate.hint.id).is_some_and(|last| last.elapsed() < Duration::from_secs(30)) { continue; }
                    attempts.insert(candidate.hint.id, Instant::now());
                    launched += 1;
                    let context = context.clone();
                    tokio::spawn(async move { let _ = connect_candidate(candidate, context).await; });
                }
                attempts.retain(|_, last| last.elapsed() < Duration::from_secs(60));
            }
            incoming = listener.accept() => {
                let Ok((stream, address)) = incoming else { continue; };
                if !matches!(address.ip(), IpAddr::V4(ip) if tailscale_ipv4(ip)) { continue; }
                let Ok(slot) = slots.clone().try_acquire_owned() else { continue; };
                let context = context.clone();
                tokio::spawn(async move {
                    let _slot = slot;
                    let _ = accept_desktop(stream, address, context).await;
                });
            }
        }
    }
}

#[derive(Clone)]
struct SessionContext {
    identity: Arc<Identity>,
    name: String,
    epoch: [u8; 16],
    history: Arc<Mutex<MobileHistory>>,
    peers: Arc<Mutex<PeerDirectory>>,
    results: SendResults,
    routes: Arc<tokio::sync::Mutex<BTreeMap<DeviceId, mpsc::Sender<SendCommand>>>>,
    stop: watch::Receiver<bool>,
}

async fn accept_desktop(
    stream: TcpStream,
    address: SocketAddr,
    context: SessionContext,
) -> Result<()> {
    let mut tls = timeout(
        Duration::from_secs(5),
        TlsAcceptor::from(context.identity.server.clone()).accept(stream),
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
            name: context.name.clone(),
            epoch: context.epoch,
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
    context
        .peers
        .lock()
        .map_err(|_| "peer directory unavailable")?
        .observed_direct(remote_id, remote_name, endpoint, Capabilities::desktop())
        .map_err(|e| format!("{e:?}"))?;
    let generation = context
        .history
        .lock()
        .map_err(|_| "history unavailable")?
        .generation();
    let (sender, commands) = mpsc::channel(8);
    context
        .routes
        .lock()
        .await
        .insert(remote_id, sender.clone());
    let result = mobile_session(
        &mut tls,
        remote_id,
        remote_epoch,
        generation,
        &context,
        commands,
    )
    .await;
    finish_session(&context, remote_id, generation, &sender).await;
    result
}

async fn connect_candidate(candidate: Candidate, context: SessionContext) -> Result<()> {
    let address: SocketAddr = candidate
        .hint
        .endpoint
        .parse()
        .map_err(|_| "invalid hinted endpoint")?;
    if !matches!(address.ip(), IpAddr::V4(ip) if tailscale_ipv4(ip)) {
        return Err("invalid hinted network".into());
    }
    let stream = timeout(Duration::from_secs(5), TcpStream::connect(address))
        .await
        .map_err(|_| "connection timeout")?
        .map_err(|e| e.to_string())?;
    let server_name = rustls::pki_types::ServerName::try_from("shuttli.local")
        .map_err(|_| "invalid server name")?;
    let mut tls = timeout(
        Duration::from_secs(5),
        TlsConnector::from(context.identity.client.clone()).connect(server_name, stream),
    )
    .await
    .map_err(|_| "TLS timeout")?
    .map_err(|e| e.to_string())?;
    let remote_id = device_id(
        tls.get_ref()
            .1
            .peer_certificates()
            .and_then(|certs| certs.first())
            .ok_or("missing hinted identity")?
            .as_ref(),
    )?;
    if remote_id != candidate.hint.id {
        return Err("hinted identity mismatch".into());
    }
    write_hello(
        &mut tls,
        &Hello::V2 {
            name: context.name.clone(),
            epoch: context.epoch,
            capabilities: Capabilities::pull_only(),
        },
    )
    .await?;
    let (remote_name, remote_epoch, capabilities) =
        match timeout(Duration::from_secs(5), read_hello(&mut tls))
            .await
            .map_err(|_| "HELLO timeout")??
        {
            Hello::V2 {
                name,
                epoch,
                capabilities,
            } => (name, epoch, capabilities),
            Hello::V1 { .. } => return Err("peer does not support history pull".into()),
        };
    if !capabilities.history_pull {
        return Err("peer does not support history pull".into());
    }
    write_frame(
        &mut tls,
        &FrameV2::Select {
            initiator: context.identity.id,
        },
    )
    .await?;
    if !matches!(read_frame(&mut tls).await?, FrameV2::Select { initiator } if initiator == context.identity.id)
    {
        return Err("session selection mismatch".into());
    }
    context
        .peers
        .lock()
        .map_err(|_| "peer directory unavailable")?
        .observed_direct(remote_id, remote_name, address.to_string(), capabilities)
        .map_err(|e| format!("{e:?}"))?;
    let generation = context
        .history
        .lock()
        .map_err(|_| "history unavailable")?
        .generation();
    let (sender, commands) = mpsc::channel(8);
    context
        .routes
        .lock()
        .await
        .insert(remote_id, sender.clone());
    let result = mobile_session(
        &mut tls,
        remote_id,
        remote_epoch,
        generation,
        &context,
        commands,
    )
    .await;
    finish_session(&context, remote_id, generation, &sender).await;
    result
}

/// Only the currently registered session may invalidate this source's state.
async fn finish_session(
    context: &SessionContext,
    source: DeviceId,
    generation: u64,
    sender: &mpsc::Sender<SendCommand>,
) {
    let mut routes = context.routes.lock().await;
    if routes
        .get(&source)
        .is_none_or(|current| !current.same_channel(sender))
    {
        return;
    }
    routes.remove(&source);
    if let Ok(mut history) = context.history.lock() {
        history.source_activity(generation, source, HistoryActivity::Failed, None);
    }
    if let Ok(mut results) = context.results.lock() {
        for ((_, target), state) in results.iter_mut() {
            if *target == source {
                *state = match *state {
                    SendState::Queued => SendState::Failed,
                    SendState::Sending => SendState::Unknown,
                    terminal => terminal,
                };
            }
        }
    }
    if let Ok(mut directory) = context.peers.lock() {
        directory.disconnected(source);
    }
}

async fn mobile_session<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    source: DeviceId,
    source_epoch: [u8; 16],
    generation: u64,
    context: &SessionContext,
    mut commands: mpsc::Receiver<SendCommand>,
) -> Result<()> {
    let history = &context.history;
    let peers = &context.peers;
    let mut stop = context.stop.clone();
    let mut pending_list = false;
    let mut pending_body: Option<EventId> = None;
    let mut body_queue = VecDeque::new();
    let mut next_cursor = None;
    let mut body_budget = crate::MAX_SESSION_BYTES;
    let mut refresh = tokio::time::interval(Duration::from_secs(15));
    refresh.tick().await;
    let mut refresh_needed = true;
    let mut request_started = Instant::now();
    let mut frame_reader = FrameReader::default();
    let mut deferred_command: Option<SendCommand> = None;
    loop {
        if !pending_list && pending_body.is_none() && frame_reader.is_idle() {
            if let Some(command) = deferred_command.take() {
                let outcome = if command.target != source {
                    SendState::Failed
                } else {
                    match timeout(
                        Duration::from_secs(20),
                        send_offer(stream, &command, context),
                    )
                    .await
                    {
                        Ok(Ok(state)) => state,
                        _ => SendState::Unknown,
                    }
                };
                if let Ok(mut results) = context.results.lock() {
                    results.insert((command.event, command.target), outcome);
                }
                if outcome == SendState::Unknown {
                    return Err("outgoing transfer outcome unknown".into());
                }
            }
        }
        if refresh_needed && !pending_list && pending_body.is_none() && can_query(context, source) {
            refresh_needed = false;
            body_budget = crate::MAX_SESSION_BYTES;
            history
                .lock()
                .map_err(|_| "history unavailable")?
                .source_activity(generation, source, HistoryActivity::Updating, None);
            write_frame(
                stream,
                &FrameV2::HistoryListRequest {
                    cursor: None,
                    limit: 20,
                },
            )
            .await?;
            pending_list = true;
            request_started = Instant::now();
        }
        tokio::select! {
            _ = stop.changed() => { if *stop.borrow() { return Ok(()); } }
            command = commands.recv(), if !pending_list && pending_body.is_none() && deferred_command.is_none() => {
                deferred_command = command;
            }
            _ = refresh.tick() => {
                refresh_needed = true;
                if (pending_list || pending_body.is_some()) && request_started.elapsed() > Duration::from_secs(30) {
                    return Err("history response timeout".into());
                }
            }
            frame = frame_reader.read(stream) => {
                match frame? {
                    FrameV2::PeerList { revision, peers: hints } => {
                        peers.lock().map_err(|_| "peer directory unavailable")?
                            .accept_hints(source, shuttli_model::mobile::PeerList { revision, peers: hints }, now_ms())
                            .map_err(|e| format!("{e:?}"))?;
                    }
                    FrameV2::HistoryListResponse { source_epoch: page_epoch, revision, items, next } if pending_list => {
                        if page_epoch != source_epoch { return Err("history source epoch mismatch".into()); }
                        pending_list = false;
                        if !can_query(context, source) {
                            body_queue.clear();
                            next_cursor = None;
                            continue;
                        }
                        next_cursor = next;
                        let page = shuttli_model::mobile::HistoryListResponse { source_epoch: page_epoch, revision, items, next };
                        let wants_body = history.lock().map_err(|_| "history unavailable")?.wants_body();
                        let candidates: Vec<_> = page.items.iter().filter(|i| i.body_available && wants_body)
                            .map(|i| (i.event, i.metadata.size)).collect();
                        {
                            let mut cache = history.lock().map_err(|_| "history unavailable")?;
                            match cache.merge_page(generation, source, page, now_ms()) {
                                Ok(()) => {},
                                Err(crate::HistoryError::Disabled) => { body_queue.clear(); next_cursor = None; continue; }
                                Err(error) => return Err(format!("{error:?}")),
                            }
                            for (event, size) in candidates {
                                if cache.body_for_explicit_copy(event).is_none() && size <= body_budget as u64 {
                                    body_budget -= size as usize;
                                    body_queue.push_back(event);
                                }
                            }
                        }
                        if !can_query(context, source) { body_queue.clear(); next_cursor = None; }
                        if let Some(event) = body_queue.pop_front() {
                            write_frame(stream, &FrameV2::HistoryGet { event }).await?;
                            pending_body = Some(event);
                            request_started = Instant::now();
                            history.lock().map_err(|_| "history unavailable")?.source_activity(generation, source, HistoryActivity::Receiving, Some(event));
                        } else if let Some(cursor) = next_cursor.take() {
                            write_frame(stream, &FrameV2::HistoryListRequest { cursor: Some(cursor), limit: 20 }).await?;
                            pending_list = true;
                            request_started = Instant::now();
                            history.lock().map_err(|_| "history unavailable")?.source_activity(generation, source, HistoryActivity::Updating, None);
                        } else {
                            history.lock().map_err(|_| "history unavailable")?.source_activity(generation, source, HistoryActivity::Updated, None);
                        }
                    }
                    FrameV2::HistoryBody { event, metadata } if pending_body == Some(event) => {
                        if metadata.size > 8 * 1024 * 1024 { return Err("oversized history body".into()); }
                        let mut bytes = vec![0; metadata.size as usize];
                        timeout(Duration::from_secs(20), stream.read_exact(&mut bytes))
                            .await.map_err(|_| "history body timeout")?.map_err(|e| e.to_string())?;
                        if can_query(context, source) && history.lock().map_err(|_| "history unavailable")?.wants_body() {
                            if history.lock().map_err(|_| "history unavailable")?.timeline()
                                .iter().find(|row| row.summary.event == event)
                                .is_none_or(|row| row.summary.metadata != metadata) {
                                return Err("history body metadata mismatch".into());
                            }
                            history.lock().map_err(|_| "history unavailable")?
                                .cache_body(generation, event, bytes).map_err(|e| format!("{e:?}"))?;
                        } else {
                            body_queue.clear();
                            next_cursor = None;
                        }
                        pending_body = None;
                        if !can_query(context, source) { body_queue.clear(); next_cursor = None; }
                        if let Some(event) = body_queue.pop_front() {
                            write_frame(stream, &FrameV2::HistoryGet { event }).await?;
                            pending_body = Some(event);
                            request_started = Instant::now();
                            history.lock().map_err(|_| "history unavailable")?.source_activity(generation, source, HistoryActivity::Receiving, Some(event));
                        } else if let Some(cursor) = next_cursor.take() {
                            write_frame(stream, &FrameV2::HistoryListRequest { cursor: Some(cursor), limit: 20 }).await?;
                            pending_list = true;
                            request_started = Instant::now();
                            history.lock().map_err(|_| "history unavailable")?.source_activity(generation, source, HistoryActivity::Updating, None);
                        } else {
                            history.lock().map_err(|_| "history unavailable")?.source_activity(generation, source, HistoryActivity::Updated, None);
                        }
                    }
                    FrameV2::HistoryChanged { .. } => {
                        refresh_needed = true;
                    }
                    FrameV2::Error { code } => {
                        let activity = if code == "history_denied" {
                            HistoryActivity::Denied
                        } else { HistoryActivity::Failed };
                        history.lock().map_err(|_| "history unavailable")?.source_activity(generation, source, activity, None);
                        if code == "history_cursor_stale" { refresh_needed = true; }
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

fn can_query(context: &SessionContext, source: DeviceId) -> bool {
    let permitted = context.peers.lock().ok().is_some_and(|directory| {
        directory
            .direct()
            .iter()
            .any(|peer| peer.id == source && peer.online && peer.directions.receive)
    });
    permitted
        && context
            .history
            .lock()
            .ok()
            .is_some_and(|history| history.query_enabled())
}

fn permitted_to_send(context: &SessionContext, target: DeviceId) -> bool {
    context.peers.lock().ok().is_some_and(|directory| {
        directory
            .direct()
            .iter()
            .any(|peer| peer.id == target && peer.online && peer.directions.send)
    })
}

async fn offer_reply<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    source: DeviceId,
    context: &SessionContext,
) -> Result<FrameV2> {
    loop {
        match read_frame(stream).await? {
            FrameV2::PeerList { revision, peers } => {
                context
                    .peers
                    .lock()
                    .map_err(|_| "peer directory unavailable")?
                    .accept_hints(
                        source,
                        shuttli_model::mobile::PeerList { revision, peers },
                        now_ms(),
                    )
                    .map_err(|e| format!("{e:?}"))?;
            }
            FrameV2::HistoryChanged { .. } => {}
            response @ (FrameV2::Ready | FrameV2::Applied | FrameV2::Error { .. }) => {
                return Ok(response);
            }
            _ => return Err("unexpected transfer response".into()),
        }
    }
}

async fn send_offer<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    command: &SendCommand,
    context: &SessionContext,
) -> Result<SendState> {
    if command.event.origin != context.identity.id
        || command.event.epoch != context.epoch
        || command.event.seq == 0
        || !permitted_to_send(context, command.target)
        || command.metadata.size != command.body.len() as u64
        || command.metadata.size == 0
        || command.metadata.size > 8 * 1024 * 1024
        || matches!(command.metadata.format, Format::Text) && command.metadata.size > 1024 * 1024
        || shuttli_content::canonical_digest(command.metadata.format, &command.body).ok()
            != Some(command.metadata.digest)
    {
        return Ok(SendState::Failed);
    }
    write_frame(
        stream,
        &FrameV2::Offer {
            event: command.event,
            meta: command.metadata.clone(),
        },
    )
    .await?;
    match offer_reply(stream, command.target, context).await? {
        FrameV2::Ready => {}
        FrameV2::Error { .. } => return Ok(SendState::Failed),
        _ => return Err("missing READY".into()),
    }
    if let Ok(mut results) = context.results.lock() {
        results.insert((command.event, command.target), SendState::Sending);
    }
    for chunk in command.body.chunks(65_536) {
        if !permitted_to_send(context, command.target) {
            return Err("send permission changed".into());
        }
        stream.write_all(chunk).await.map_err(|e| e.to_string())?;
    }
    stream.flush().await.map_err(|e| e.to_string())?;
    match offer_reply(stream, command.target, context).await? {
        FrameV2::Applied => Ok(SendState::Applied),
        FrameV2::Error { .. } => Ok(SendState::Failed),
        _ => Err("missing APPLIED".into()),
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

/// Keep partial control bytes across timer/command branches of select!.
#[derive(Default)]
struct FrameReader {
    header: [u8; 4],
    header_len: usize,
    bytes: Vec<u8>,
    body_len: usize,
}
impl FrameReader {
    fn is_idle(&self) -> bool {
        self.header_len == 0
    }
    async fn read(&mut self, stream: &mut (impl AsyncRead + Unpin)) -> Result<FrameV2> {
        while self.header_len < 4 {
            let n = stream
                .read(&mut self.header[self.header_len..])
                .await
                .map_err(|e| e.to_string())?;
            if n == 0 {
                return Err("history connection closed".into());
            }
            self.header_len += n;
        }
        if self.bytes.is_empty() {
            let len = u32::from_be_bytes(self.header) as usize;
            if len == 0 || len > shuttli_protocol::MAX_CONTROL_FRAME_BYTES {
                return Err("invalid v2 frame size".into());
            }
            self.bytes.resize(len, 0);
        }
        while self.body_len < self.bytes.len() {
            let n = stream
                .read(&mut self.bytes[self.body_len..])
                .await
                .map_err(|e| e.to_string())?;
            if n == 0 {
                return Err("history connection closed".into());
            }
            self.body_len += n;
        }
        let bytes = std::mem::take(&mut self.bytes);
        self.header_len = 0;
        self.body_len = 0;
        FrameV2::decode(&bytes).map_err(str::to_owned)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HistoryMode;
    use shuttli_model::{
        mobile::HistorySummary,
        sync::{Format, Metadata},
    };

    #[tokio::test(flavor = "current_thread")]
    async fn partial_control_frame_survives_cancelled_read_future() {
        let (mut remote, mut local) = tokio::io::duplex(1024);
        let bytes = FrameV2::HistoryChanged { revision: 7 }.encode().unwrap();
        let mut packet = (bytes.len() as u32).to_be_bytes().to_vec();
        packet.extend(bytes);
        let mut reader = FrameReader::default();
        remote.write_all(&packet[..2]).await.unwrap();
        assert!(
            timeout(Duration::from_millis(5), reader.read(&mut local))
                .await
                .is_err()
        );
        assert!(!reader.is_idle());
        remote.write_all(&packet[2..6]).await.unwrap();
        assert!(
            timeout(Duration::from_millis(5), reader.read(&mut local))
                .await
                .is_err()
        );
        remote.write_all(&packet[6..]).await.unwrap();
        assert!(matches!(
            reader.read(&mut local).await.unwrap(),
            FrameV2::HistoryChanged { revision: 7 }
        ));
        assert!(reader.is_idle());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn session_cleanup_preserves_replacement_and_settles_only_its_source() {
        let source = [2; 32];
        let event = EventId {
            origin: [1; 32],
            epoch: [1; 16],
            seq: 1,
        };
        let history = Arc::new(Mutex::new(MobileHistory::default()));
        let generation = history.lock().unwrap().enter_foreground();
        history.lock().unwrap().source_activity(
            generation,
            source,
            HistoryActivity::Receiving,
            Some(event),
        );
        let (_, stop) = watch::channel(false);
        let context = SessionContext {
            identity: Arc::new(Identity::generate().unwrap().0),
            name: "Phone".into(),
            epoch: [1; 16],
            history: history.clone(),
            peers: Arc::new(Mutex::new(PeerDirectory::new([1; 32]))),
            results: Arc::new(Mutex::new(BTreeMap::new())),
            routes: Arc::new(tokio::sync::Mutex::new(BTreeMap::new())),
            stop,
        };
        let (old, _) = mpsc::channel(8);
        let (current, _) = mpsc::channel(8);
        context.routes.lock().await.insert(source, current.clone());
        finish_session(&context, source, generation, &old).await;
        assert_eq!(
            history.lock().unwrap().source(source).unwrap().receiving,
            Some(event)
        );
        let queued = EventId { seq: 2, ..event };
        let applied = EventId { seq: 3, ..event };
        context.results.lock().unwrap().extend([
            ((event, source), SendState::Sending),
            ((queued, source), SendState::Queued),
            ((applied, source), SendState::Applied),
            ((event, [3; 32]), SendState::Queued),
        ]);
        finish_session(&context, source, generation, &current).await;
        assert!(!context.routes.lock().await.contains_key(&source));
        let freshness = history.lock().unwrap().source(source).unwrap();
        assert_eq!(freshness.activity, HistoryActivity::Failed);
        assert_eq!(freshness.receiving, None);
        let results = context.results.lock().unwrap();
        assert_eq!(results[&(event, source)], SendState::Unknown);
        assert_eq!(results[&(queued, source)], SendState::Failed);
        assert_eq!(results[&(applied, source)], SendState::Applied);
        assert_eq!(results[&(event, [3; 32])], SendState::Queued);
    }

    #[test]
    fn listener_only_accepts_tailnet_ipv4() {
        assert!(tailscale_ipv4("100.64.0.1".parse().unwrap()));
        assert!(tailscale_ipv4("100.127.255.254".parse().unwrap()));
        assert!(!tailscale_ipv4("100.128.0.1".parse().unwrap()));
        assert!(!tailscale_ipv4("192.168.1.2".parse().unwrap()));
    }

    #[test]
    fn changed_hint_cannot_query_after_receive_or_history_is_disabled() {
        let (identity, _) = Identity::generate().unwrap();
        let source = [2; 32];
        let history = Arc::new(Mutex::new(MobileHistory::default()));
        history.lock().unwrap().enter_foreground();
        let peers = Arc::new(Mutex::new(PeerDirectory::new(identity.id)));
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
        let (_, stop) = watch::channel(false);
        let context = SessionContext {
            identity: Arc::new(identity),
            name: "Phone".into(),
            epoch: [4; 16],
            history: history.clone(),
            peers: peers.clone(),
            results: Arc::new(Mutex::new(BTreeMap::new())),
            routes: Arc::new(tokio::sync::Mutex::new(BTreeMap::new())),
            stop,
        };
        assert!(can_query(&context, source));
        peers
            .lock()
            .unwrap()
            .directions(
                source,
                crate::peers::Directions {
                    send: false,
                    receive: false,
                },
            )
            .unwrap();
        assert!(!can_query(&context, source));
        peers
            .lock()
            .unwrap()
            .directions(source, crate::peers::Directions::default())
            .unwrap();
        history.lock().unwrap().set_mode(HistoryMode::Off);
        assert!(!can_query(&context, source));
        history.lock().unwrap().set_mode(HistoryMode::Status);
        assert!(can_query(&context, source));
        history.lock().unwrap().set_limit(0);
        assert!(!can_query(&context, source));
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
            digest: shuttli_content::canonical_digest(Format::Text, bytes).unwrap(),
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
        let (command_sender, commands) = mpsc::channel(8);
        let context = SessionContext {
            identity: Arc::new(Identity::generate().unwrap().0),
            name: "Phone".into(),
            epoch: [4; 16],
            history: history.clone(),
            peers: peers.clone(),
            results: Arc::new(Mutex::new(BTreeMap::new())),
            routes: Arc::new(tokio::sync::Mutex::new(BTreeMap::new())),
            stop: receiver,
        };
        let session = tokio::spawn(async move {
            mobile_session(&mut phone, source, epoch, generation, &context, commands).await
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
        write_frame(&mut desktop, &FrameV2::HistoryChanged { revision: 1 })
            .await
            .unwrap();
        write_frame(&mut desktop, &FrameV2::HistoryChanged { revision: 2 })
            .await
            .unwrap();
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
        assert!(matches!(
            timeout(Duration::from_secs(1), read_frame(&mut desktop))
                .await
                .unwrap()
                .unwrap(),
            FrameV2::HistoryListRequest { cursor: None, .. }
        ));
        write_frame(
            &mut desktop,
            &FrameV2::HistoryListResponse {
                source_epoch: epoch,
                revision: 2,
                items: vec![],
                next: None,
            },
        )
        .await
        .unwrap();
        timeout(Duration::from_secs(1), async {
            while history.lock().unwrap().source(source).unwrap().activity
                != HistoryActivity::Updated
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(history.lock().unwrap().timeline().len(), 1);
        write_frame(&mut desktop, &FrameV2::HistoryChanged { revision: 3 })
            .await
            .unwrap();
        assert!(matches!(
            read_frame(&mut desktop).await.unwrap(),
            FrameV2::HistoryListRequest { .. }
        ));
        write_frame(
            &mut desktop,
            &FrameV2::Error {
                code: "history_denied".into(),
            },
        )
        .await
        .unwrap();
        timeout(Duration::from_secs(1), async {
            while history.lock().unwrap().source(source).unwrap().activity
                != HistoryActivity::Denied
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(history.lock().unwrap().body_available(event));
        stop.send(true).unwrap();
        assert!(session.await.unwrap().is_ok());
        drop(command_sender);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn explicit_offer_requires_send_consent_and_applied_reply() {
        let (identity, _) = Identity::generate().unwrap();
        let source = [2; 32];
        let epoch = [4; 16];
        let event = EventId {
            origin: identity.id,
            epoch,
            seq: 1,
        };
        let bytes: Arc<[u8]> = Arc::from(&b"manual copy"[..]);
        let command = SendCommand {
            event,
            target: source,
            metadata: Metadata {
                format: Format::Text,
                size: bytes.len() as u64,
                digest: shuttli_content::canonical_digest(Format::Text, &bytes).unwrap(),
            },
            body: bytes.clone(),
        };
        let peers = Arc::new(Mutex::new(PeerDirectory::new(identity.id)));
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
        let (_, stop) = watch::channel(false);
        let context = SessionContext {
            identity: Arc::new(identity),
            name: "Phone".into(),
            epoch,
            history: Arc::new(Mutex::new(MobileHistory::default())),
            peers: peers.clone(),
            results: Arc::new(Mutex::new(BTreeMap::new())),
            routes: Arc::new(tokio::sync::Mutex::new(BTreeMap::new())),
            stop,
        };
        let (mut phone, mut desktop) = tokio::io::duplex(1024);
        assert_eq!(
            send_offer(&mut phone, &command, &context).await.unwrap(),
            SendState::Failed
        );
        peers
            .lock()
            .unwrap()
            .directions(
                source,
                crate::peers::Directions {
                    send: true,
                    receive: true,
                },
            )
            .unwrap();
        let task = tokio::spawn(async move { send_offer(&mut phone, &command, &context).await });
        assert!(
            matches!(read_frame(&mut desktop).await.unwrap(), FrameV2::Offer { event: offered, .. } if offered == event)
        );
        write_frame(&mut desktop, &FrameV2::Ready).await.unwrap();
        let mut body = vec![0; bytes.len()];
        desktop.read_exact(&mut body).await.unwrap();
        assert_eq!(body, &*bytes);
        write_frame(&mut desktop, &FrameV2::Applied).await.unwrap();
        assert_eq!(task.await.unwrap().unwrap(), SendState::Applied);
    }
}
