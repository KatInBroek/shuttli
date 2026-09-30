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
use shuttli_transport::{
    FrameReader, WireVersion, read_frame, read_live_frame, write_frame, write_live_frame,
};
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
    refresh: watch::Sender<u64>,
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
        let (refresh, refresh_rx) = watch::channel(0);
        let context = SessionContext {
            identity: Arc::new(identity),
            name,
            epoch,
            history,
            peers,
            results,
            routes: Arc::new(tokio::sync::Mutex::new(BTreeMap::new())),
            stop: receiver.clone(),
            refresh: refresh_rx,
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
            refresh,
            thread: Some(thread),
        })
    }

    pub fn epoch(&self) -> [u8; 16] {
        self.epoch
    }

    pub fn enqueue(&self, command: SendCommand) -> bool {
        self.commands.try_send(command).is_ok()
    }

    /// Coalesce refresh requests without resetting sessions or interrupting sends.
    pub fn request_refresh(&self) {
        self.refresh
            .send_modify(|revision| *revision = revision.wrapping_add(1));
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
    refresh: watch::Receiver<u64>,
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
    let legacy_pull = matches!(remote, Hello::V2 { .. });
    let (remote_name, remote_epoch, capabilities, version) = match remote {
        Hello::V1 { name, epoch } => (name, epoch, Capabilities::legacy_desktop(), WireVersion::V1),
        Hello::V2 {
            name,
            epoch,
            capabilities,
        }
        | Hello::V3 {
            name,
            epoch,
            capabilities,
        } => (name, epoch, capabilities, WireVersion::V2),
    };
    let hello = if version == WireVersion::V1 {
        Hello::V1 {
            name: context.name.clone(),
            epoch: context.epoch,
        }
    } else if legacy_pull {
        Hello::V2 {
            name: context.name.clone(),
            epoch: context.epoch,
            capabilities: Capabilities::pull_only(),
        }
    } else {
        Hello::V3 {
            name: context.name.clone(),
            epoch: context.epoch,
            capabilities: Capabilities::live(),
        }
    };
    write_hello(&mut tls, &hello).await?;
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
        .observed_direct(
            remote_id,
            remote_name,
            endpoint,
            if legacy_pull {
                Capabilities::pull_only()
            } else {
                capabilities
            },
        )
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
    let result = if version == WireVersion::V1 {
        live_v1_session(
            &mut tls,
            remote_id,
            remote_epoch,
            generation,
            &context,
            commands,
            false,
        )
        .await
    } else {
        mobile_session(
            &mut tls,
            remote_id,
            remote_epoch,
            generation,
            &context,
            commands,
        )
        .await
    };
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
    let hello = if candidate.hint.capabilities.history_pull {
        Hello::V3 {
            name: context.name.clone(),
            epoch: context.epoch,
            capabilities: Capabilities::live(),
        }
    } else {
        Hello::V1 {
            name: context.name.clone(),
            epoch: context.epoch,
        }
    };
    write_hello(&mut tls, &hello).await?;
    let (remote_name, remote_epoch, capabilities, version) =
        match timeout(Duration::from_secs(5), read_hello(&mut tls))
            .await
            .map_err(|_| "HELLO timeout")??
        {
            Hello::V3 {
                name,
                epoch,
                capabilities,
            } => (name, epoch, capabilities, WireVersion::V2),
            Hello::V2 { name, epoch, .. } => {
                (name, epoch, Capabilities::pull_only(), WireVersion::V2)
            }
            Hello::V1 { name, epoch } => {
                (name, epoch, Capabilities::legacy_desktop(), WireVersion::V1)
            }
        };
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
    let result = if version == WireVersion::V1 {
        live_v1_session(
            &mut tls,
            remote_id,
            remote_epoch,
            generation,
            &context,
            commands,
            true,
        )
        .await
    } else {
        mobile_session(
            &mut tls,
            remote_id,
            remote_epoch,
            generation,
            &context,
            commands,
        )
        .await
    };
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
    let live_protocol = context.peers.lock().ok().is_some_and(|p| {
        p.direct()
            .iter()
            .any(|peer| peer.id == source && peer.capabilities.accept_live_offer)
    });
    let leader = !live_protocol || shuttli_transport::leads_session(context.identity.id, source);
    let mut granted = leader;
    let mut pending_poll = false;
    let mut poll = tokio::time::interval(Duration::from_millis(500));
    poll.tick().await;
    let mut poll_needed = false;
    let history = &context.history;
    let peers = &context.peers;
    let mut stop = context.stop.clone();
    let mut requested = context.refresh.clone();
    let mut refresh_notifications = true;
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
        if granted
            && !pending_poll
            && !pending_list
            && pending_body.is_none()
            && frame_reader.is_idle()
        {
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
                granted = leader;
                refresh_needed = true;
                poll_needed = live_protocol;
                if outcome == SendState::Unknown {
                    return Err("outgoing transfer outcome unknown".into());
                }
            }
        }
        if granted
            && !pending_poll
            && !pending_list
            && pending_body.is_none()
            && frame_reader.is_idle()
        {
            if !can_query(context, source) {
                body_queue.clear();
                next_cursor = None;
            }
            // A live reception can fill an item that a previous page queued.
            while body_queue.front().is_some_and(|event| {
                history
                    .lock()
                    .ok()
                    .is_some_and(|h| h.body_available(*event))
            }) {
                body_queue.pop_front();
            }
            if live_protocol && leader && poll_needed {
                write_frame(stream, &FrameV2::Poll).await?;
                pending_poll = true;
                poll_needed = false;
                request_started = Instant::now();
            } else if let Some(event) = body_queue.pop_front() {
                write_frame(stream, &FrameV2::HistoryGet { event }).await?;
                pending_body = Some(event);
                request_started = Instant::now();
                granted = leader;
                history
                    .lock()
                    .map_err(|_| "history unavailable")?
                    .source_activity(generation, source, HistoryActivity::Receiving, Some(event));
            } else if let Some(cursor) = next_cursor.take() {
                write_frame(
                    stream,
                    &FrameV2::HistoryListRequest {
                        cursor: Some(cursor),
                        limit: 20,
                    },
                )
                .await?;
                pending_list = true;
                request_started = Instant::now();
                granted = leader;
            } else if refresh_needed && can_query(context, source) {
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
                granted = leader;
            } else if !leader {
                write_frame(stream, &FrameV2::Idle).await?;
                granted = false;
            }
        }
        tokio::select! {
            _ = poll.tick(), if live_protocol && leader => {
                poll_needed = true;
                if pending_poll && request_started.elapsed() > Duration::from_secs(20) {
                    return Err("live response timeout".into());
                }
            }
            notification = requested.changed(), if refresh_notifications => {
                if notification.is_ok() { refresh_needed = true; } else { refresh_notifications = false; }
            }
            _ = stop.changed() => { if *stop.borrow() { return Ok(()); } }
            command = commands.recv(), if deferred_command.is_none() => {
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
                        poll_needed = live_protocol;
                        if !can_query(context, source) {
                            body_queue.clear();
                            next_cursor = None;
                            continue;
                        }
                        next_cursor = next;
                        let page = shuttli_model::mobile::HistoryListResponse { source_epoch: page_epoch, revision, items, next };
                        let wants_body = history.lock().map_err(|_| "history unavailable")?.wants_body();
                        let candidates: Vec<_> = page.items.iter().filter(|i| i.body_available && wants_body && content_allowed(context, source, i.metadata.format))
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
                        if body_queue.is_empty() && next_cursor.is_none() {
                            history.lock().map_err(|_| "history unavailable")?.source_activity(generation, source, HistoryActivity::Updated, None);
                        }
                    }
                    FrameV2::HistoryBody { event, metadata } if pending_body == Some(event) => {
                        if metadata.size > 8 * 1024 * 1024 { return Err("oversized history body".into()); }
                        let mut bytes = vec![0; metadata.size as usize];
                        timeout(Duration::from_secs(20), stream.read_exact(&mut bytes))
                            .await.map_err(|_| "history body timeout")?.map_err(|e| e.to_string())?;
                        if can_query(context, source) && content_allowed(context, source, metadata.format) && history.lock().map_err(|_| "history unavailable")?.wants_body() {
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
                        poll_needed = live_protocol;
                        if !can_query(context, source) { body_queue.clear(); next_cursor = None; }
                        if body_queue.is_empty() && next_cursor.is_none() {
                            history.lock().map_err(|_| "history unavailable")?.source_activity(generation, source, HistoryActivity::Updated, None);
                        }
                    }
                    FrameV2::Poll if live_protocol && !leader && !pending_list && pending_body.is_none() => {
                        granted = true;
                    }
                    FrameV2::Idle if pending_poll => { pending_poll = false; }
                    FrameV2::Offer { event, meta } if !pending_list && pending_body.is_none() => {
                        if leader && !pending_poll { return Err("offer outside granted turn".into()); }
                        timeout(Duration::from_secs(20), receive_offer(stream, source, source_epoch, generation, event, meta, context))
                            .await.map_err(|_| "receive timeout")??;
                        pending_poll = false;
                    }
                    FrameV2::Status { event } if !pending_list && pending_body.is_none() => {
                        if event.origin != source || event.epoch != source_epoch {
                            return Err("receipt identity mismatch".into());
                        }
                        let state = if history.lock().map_err(|_| "history unavailable")?.body_available(event) {
                            shuttli_model::sync::DeliveryState::Applied
                        } else { shuttli_model::sync::DeliveryState::Unknown };
                        write_frame(stream, &FrameV2::Receipt { event, state: Some(state) }).await?;
                        pending_poll = false;
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
                        pending_poll = false;
                        body_queue.clear();
                        next_cursor = None;
                    }
                    _ => return Err("unexpected mobile session frame".into()),
                }
            }
        }
    }
}

/// Original live loop. Initiator determines Poll ownership exactly as v1
/// desktops do; the receiving platform only supplies the commit adapter.
async fn live_v1_session<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    source: DeviceId,
    source_epoch: [u8; 16],
    generation: u64,
    context: &SessionContext,
    mut commands: mpsc::Receiver<SendCommand>,
    initiator: bool,
) -> Result<()> {
    let version = WireVersion::V1;
    let mut stop = context.stop.clone();
    loop {
        if initiator {
            if let Ok(command) = commands.try_recv() {
                execute_v1_send(stream, &command, context).await?;
            }
            write_live_frame(stream, &FrameV2::Poll, version).await?;
        }
        let frame = tokio::select! {
            _ = stop.changed() => { if *stop.borrow() { return Ok(()); } else { continue; } }
            frame = timeout(Duration::from_secs(20), read_live_frame(stream, version)) => frame.map_err(|_| "live session timeout")??,
        };
        match frame {
            FrameV2::Poll if !initiator => {
                if let Ok(command) = commands.try_recv() {
                    execute_v1_send(stream, &command, context).await?;
                } else {
                    write_live_frame(stream, &FrameV2::Idle, version).await?;
                }
            }
            FrameV2::Idle if initiator => {}
            FrameV2::Offer { event, meta } => {
                timeout(
                    Duration::from_secs(20),
                    receive_offer_version(
                        stream,
                        ReceiveSource {
                            id: source,
                            epoch: source_epoch,
                            generation,
                        },
                        event,
                        meta,
                        context,
                        version,
                    ),
                )
                .await
                .map_err(|_| "receive timeout")??;
            }
            FrameV2::Status { event } => {
                if event.origin != source || event.epoch != source_epoch {
                    return Err("receipt owner mismatch".into());
                }
                let state = if context
                    .history
                    .lock()
                    .map_err(|_| "history unavailable")?
                    .body_available(event)
                {
                    shuttli_model::sync::DeliveryState::Applied
                } else {
                    shuttli_model::sync::DeliveryState::Unknown
                };
                write_live_frame(
                    stream,
                    &FrameV2::Receipt {
                        event,
                        state: Some(state),
                    },
                    version,
                )
                .await?;
            }
            _ => return Err("unexpected v1 live operation".into()),
        }
        if initiator {
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
}
async fn execute_v1_send<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    command: &SendCommand,
    context: &SessionContext,
) -> Result<()> {
    let outcome = timeout(
        Duration::from_secs(20),
        send_offer_version(stream, command, context, WireVersion::V1),
    )
    .await
    .ok()
    .and_then(std::result::Result::ok)
    .unwrap_or(SendState::Unknown);
    if let Ok(mut results) = context.results.lock() {
        results.insert((command.event, command.target), outcome);
    }
    if outcome == SendState::Unknown {
        Err("outgoing result unknown".into())
    } else {
        Ok(())
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

fn content_allowed(context: &SessionContext, source: DeviceId, format: Format) -> bool {
    context.peers.lock().ok().is_some_and(|directory| {
        directory
            .direct()
            .iter()
            .any(|peer| peer.id == source && peer.directions.allows(format))
    })
}

fn permitted_to_send(context: &SessionContext, target: DeviceId, format: Format) -> bool {
    !*context.stop.borrow()
        && context
            .history
            .lock()
            .ok()
            .is_some_and(|history| history.is_active())
        && context.peers.lock().ok().is_some_and(|directory| {
            directory.direct().iter().any(|peer| {
                peer.id == target
                    && peer.online
                    && peer.directions.send
                    && peer.directions.allows(format)
            })
        })
}

async fn send_offer<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    command: &SendCommand,
    context: &SessionContext,
) -> Result<SendState> {
    send_offer_version(stream, command, context, WireVersion::V2).await
}
async fn send_offer_version<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    command: &SendCommand,
    context: &SessionContext,
    version: WireVersion,
) -> Result<SendState> {
    if command.event.origin != context.identity.id
        || command.event.epoch != context.epoch
        || command.event.seq == 0
    {
        return Ok(SendState::Failed);
    }
    let result = shuttli_transport::send_live_version(
        stream,
        shuttli_transport::LiveOffer {
            event: command.event,
            metadata: &command.metadata,
            body: &command.body,
        },
        || permitted_to_send(context, command.target, command.metadata.format),
        |frame| {
            match frame {
                FrameV2::PeerList { revision, peers } => {
                    context
                        .peers
                        .lock()
                        .map_err(|_| "peer directory unavailable")?
                        .accept_hints(
                            command.target,
                            shuttli_model::mobile::PeerList { revision, peers },
                            now_ms(),
                        )
                        .map_err(|e| format!("{e:?}"))?;
                }
                FrameV2::HistoryChanged { .. } => { /* Reconcile after the live transaction. */ }
                _ => unreachable!(),
            };
            Ok(())
        },
        || {
            if let Ok(mut results) = context.results.lock() {
                results.insert((command.event, command.target), SendState::Sending);
            }
        },
        version,
    )
    .await?;
    Ok(match result {
        shuttli_transport::SendOutcome::Applied => SendState::Applied,
        shuttli_transport::SendOutcome::Rejected => SendState::Failed,
    })
}

async fn receive_offer<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    source: DeviceId,
    source_epoch: [u8; 16],
    generation: u64,
    event: EventId,
    meta: Metadata,
    context: &SessionContext,
) -> Result<()> {
    receive_offer_version(
        stream,
        ReceiveSource {
            id: source,
            epoch: source_epoch,
            generation,
        },
        event,
        meta,
        context,
        WireVersion::V2,
    )
    .await
}
struct ReceiveSource {
    id: DeviceId,
    epoch: [u8; 16],
    generation: u64,
}
async fn receive_offer_version<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    received_from: ReceiveSource,
    event: EventId,
    meta: Metadata,
    context: &SessionContext,
    version: WireVersion,
) -> Result<()> {
    let ReceiveSource {
        id: source,
        epoch: source_epoch,
        generation,
    } = received_from;
    let allowed = || {
        can_query(context, source)
            && content_allowed(context, source, meta.format)
            && context
                .history
                .lock()
                .ok()
                .is_some_and(|h| h.generation() == generation && h.wants_body())
    };
    if event.origin != source
        || event.epoch != source_epoch
        || event.seq == 0
        || !shuttli_transport::valid_metadata(&meta)
        || !allowed()
    {
        write_live_frame(
            stream,
            &FrameV2::Error {
                code: "offer_rejected".into(),
            },
            version,
        )
        .await?;
        return Ok(());
    }
    context
        .history
        .lock()
        .map_err(|_| "history unavailable")?
        .source_activity(generation, source, HistoryActivity::Receiving, Some(event));
    write_live_frame(stream, &FrameV2::Ready, version).await?;
    let body = shuttli_transport::receive_body(stream, &meta, allowed).await?;
    // Recheck and commit before acknowledging; background/clear revokes generation.
    if !allowed() {
        return Err("receive permission changed".into());
    }
    let committed = context
        .history
        .lock()
        .map_err(|_| "history unavailable")?
        .receive_live(generation, source, event, meta, body, now_ms())
        .is_ok();
    context
        .history
        .lock()
        .map_err(|_| "history unavailable")?
        .source_activity(
            generation,
            source,
            if committed {
                HistoryActivity::Updated
            } else {
                HistoryActivity::Failed
            },
            None,
        );
    write_live_frame(
        stream,
        &if committed {
            FrameV2::Applied
        } else {
            FrameV2::Error {
                code: "receive_not_committed".into(),
            }
        },
        version,
    )
    .await
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

#[cfg(test)]
#[path = "tests/transport.rs"]
mod tests;
