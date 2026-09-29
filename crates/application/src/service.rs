//! Shared use cases. Core decisions stay serial; port effects yield to controls.
use base64::{Engine as _, engine::general_purpose::STANDARD};
use shuttli_api::control::*;
use shuttli_core::sync::SyncCore;
use shuttli_model::mobile::{HistoryCursor, HistoryListRequest, HistoryListResponse};
use shuttli_model::sync::*;
use shuttli_ports::{sync::*, worker::Port};
use std::{
    cell::{Cell, RefCell},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

struct State {
    core: SyncCore,
    settings: Settings,
    peers: Vec<PeerInfo>,
    network: Box<dyn Network>,
    last_error: Option<String>,
    available: bool,
    sequence: u64,
    configuration: u64,
    configuring: bool,
}
pub struct Service {
    state: RefCell<State>,
    clipboard: Port<dyn Clipboard>,
    store: Port<dyn Store>,
    platform: Port<dyn Platform>,
    device: String,
    id: DeviceId,
    epoch: [u8; 16],
    description: String,
    busy: Cell<bool>,
    stopping: Cell<bool>,
    generation: Arc<AtomicU64>,
}
struct Busy<'a>(&'a Cell<bool>);
impl Drop for Busy<'_> {
    fn drop(&mut self) {
        self.0.set(false);
    }
}
impl Service {
    pub async fn new(
        id: DeviceId,
        epoch: [u8; 16],
        clipboard: Port<dyn Clipboard>,
        store: Port<dyn Store>,
        mut network: Box<dyn Network>,
        platform: Port<dyn Platform>,
    ) -> Result<Self> {
        let (settings, peers, pending) = store
            .call(|s| Ok((s.settings()?, s.peers()?, s.pending_receipts()?)))
            .await?;
        let mut core = SyncCore::new(id, epoch, settings.clone());
        for p in &peers {
            core.register(parse_id(&p.id)?, p.id.clone())
                .map_err(|e| format!("{e:?}"))?;
        }
        network.policy_revision(core.revision());
        let description = clipboard.call(|c| Ok(c.description().to_owned())).await?;
        let observed = clipboard.call(|c| c.read()).await;
        let (available, last_error) = match observed {
            Ok(v) => {
                core.baseline(Some(v.stamp));
                (true, None)
            }
            Err(e) => (false, Some(e)),
        };
        for (event, peer) in pending.into_iter().take(16) {
            let _ = network.reconcile(parse_id(&peer)?, event);
        }
        Ok(Self {
            state: RefCell::new(State {
                core,
                settings,
                peers,
                network,
                available,
                last_error,
                sequence: 0,
                configuration: 0,
                configuring: false,
            }),
            clipboard,
            store,
            platform,
            device: format_id(id),
            id,
            epoch,
            description,
            busy: Cell::new(false),
            stopping: Cell::new(false),
            generation: Arc::new(AtomicU64::new(0)),
        })
    }
    fn enter(&self) -> Result<Busy<'_>> {
        if self.busy.replace(true) {
            return Err("operation in progress; retry shortly".into());
        }
        Ok(Busy(&self.busy))
    }
    fn current(&self, generation: u64) -> Result<()> {
        if self.generation.load(Ordering::SeqCst) != generation {
            Err("operation cancelled by settings change".into())
        } else {
            Ok(())
        }
    }
    pub fn stopping(&self) -> bool {
        self.stopping.get()
    }
    pub async fn cleanup(&self) -> Result<()> {
        self.store.call(|s| s.clear()).await
    }
    pub async fn tick(&self) {
        if self.stopping.get() {
            return;
        }
        let Ok(_busy) = self.enter() else {
            return;
        };
        if self.state.borrow().configuring {
            return;
        }
        // Drain outcomes before clipboard reads so a slow capture cannot starve receipts.
        for _ in 0..32 {
            let event = self.state.borrow_mut().network.poll();
            let Some(event) = event else {
                break;
            };
            if let Err(error) = self.network_event(event).await {
                self.state.borrow_mut().last_error = Some(error);
            }
        }
        self.observe().await;
    }
    async fn observe(&self) {
        let generation = self.generation.load(Ordering::SeqCst);
        let result = self.clipboard.call(|c| c.read()).await;
        if self.current(generation).is_err() || self.state.borrow().configuring {
            return;
        }
        match result {
            Ok(v) => {
                self.state.borrow_mut().available = true;
                if let Some(p) = v.payload {
                    let result = self.state.borrow_mut().core.observe_local(v.stamp, &p.meta);
                    match result {
                        Ok(observation) => {
                            if let Some(event) = observation.local_event {
                                let peer = self.device.clone();
                                let payload = p.clone();
                                match self
                                    .store
                                    .call(move |s| {
                                        s.record(
                                            event,
                                            &peer,
                                            "local",
                                            DeliveryState::Applied,
                                            &payload,
                                            "Local clipboard copy",
                                        )
                                    })
                                    .await
                                {
                                    Ok(id) if id != 0 => self.state.borrow_mut().sequence += 1,
                                    Ok(_) => {}
                                    Err(e) => self.state.borrow_mut().last_error = Some(e),
                                }
                            }
                            if let Err(e) = self
                                .publish(observation.publications, p, "automatic observation")
                                .await
                            {
                                self.state.borrow_mut().last_error = Some(e);
                            }
                        }
                        Err(
                            shuttli_core::sync::Rejection::Disabled
                            | shuttli_core::sync::Rejection::Sensitive,
                        ) => {}
                        Err(e) => self.state.borrow_mut().last_error = Some(format!("{e:?}")),
                    }
                } else {
                    self.state.borrow_mut().core.baseline(Some(v.stamp));
                }
            }
            Err(e) => {
                let mut state = self.state.borrow_mut();
                state.available = false;
                state.core.baseline(None);
                state.last_error = Some(e);
            }
        }
    }
    async fn publish(
        &self,
        permits: Vec<shuttli_core::sync::Publication>,
        payload: Payload,
        reason: &str,
    ) -> Result<usize> {
        let mut count = 0;
        for permit in permits {
            let event = permit.event();
            let target = format_id(permit.target());
            let peer = target.clone();
            let p = payload.clone();
            let detail = format!("{reason}; awaiting remote OS readback");
            self.store
                .call(move |s| s.record(event, &peer, "send", DeliveryState::Sending, &p, &detail))
                .await?;
            // Persistence yields: recheck authority immediately before enqueueing content.
            let result = {
                let mut state = self.state.borrow_mut();
                if !state.core.may_send(&permit) {
                    Err((DeliveryState::Cancelled, "send permission changed".into()))
                } else {
                    state
                        .network
                        .send(permit, payload.clone())
                        .map_err(|e| (DeliveryState::Failed, e))
                }
            };
            if let Err((state, e)) = result {
                self.store
                    .call(move |s| s.update(event, &target, state, &e))
                    .await?;
            } else {
                count += 1;
            }
        }
        if count > 0 {
            self.state.borrow_mut().sequence += 1;
            self.notify("Sending clipboard", &format!("{count} device(s)"), None);
        }
        Ok(count)
    }
    async fn network_event(&self, event: NetworkEvent) -> Result<()> {
        match event {
            NetworkEvent::PeerHintsPermission { peer, reply } => {
                let result = self
                    .state
                    .borrow()
                    .core
                    .authorize_peer_hints(peer)
                    .map(|_| self.state.borrow().core.revision())
                    .map_err(|e| format!("{e:?}"));
                let _ = reply.send(result);
            }
            NetworkEvent::HistoryListQuery {
                peer,
                cursor,
                limit,
                reply,
            } => {
                let _ = reply.send(self.history_list_for(peer, cursor, limit).await);
            }
            NetworkEvent::HistoryGetQuery { peer, event, reply } => {
                let _ = reply.send(self.history_body_for(peer, event).await);
            }
            NetworkEvent::ReceiptQuery { peer, event, reply } => {
                let result = if peer == event.origin {
                    self.store.call(move |s| s.receipt(event)).await
                } else {
                    Err("receipt query is not owned by this peer".into())
                };
                let _ = reply.send(result);
            }
            NetworkEvent::Peer(peer) => {
                let p = peer.clone();
                self.store.call(move |s| s.peer(&p)).await?;
                let known = {
                    let mut state = self.state.borrow_mut();
                    state
                        .core
                        .register(parse_id(&peer.id)?, peer.id.clone())
                        .map_err(|e| format!("{e:?}"))?;
                    if let Some(old) = state.peers.iter_mut().find(|p| p.id == peer.id) {
                        *old = peer.clone();
                        true
                    } else {
                        state.peers.push(peer.clone());
                        state.sequence += 1;
                        false
                    }
                };
                if !known {
                    self.notify(
                        "Device discovered",
                        "Outgoing sync is disabled until you allow this device",
                        Some(&peer.id),
                    );
                }
            }
            NetworkEvent::Offer {
                peer,
                event,
                meta,
                reply,
            } => {
                let result = async {
                    let ticket = self
                        .state
                        .borrow()
                        .core
                        .receive(peer, event, meta)
                        .map_err(|e| format!("receive rejected: {e:?}"))?;
                    let payload = Payload {
                        meta: ticket.metadata().clone(),
                        data: Arc::from([]),
                    };
                    self.store
                        .call(move |s| {
                            if !s.reserve(event)? {
                                return Err(
                                    "duplicate or stale event; no clipboard reapplication".into()
                                );
                            }
                            s.record(
                                event,
                                &format_id(peer),
                                "receive",
                                DeliveryState::Receiving,
                                &payload,
                                "receiving content",
                            )?;
                            Ok(())
                        })
                        .await?;
                    if ticket.policy_revision() != self.state.borrow().core.revision() {
                        return Err("receive permission changed".into());
                    }
                    self.state.borrow_mut().sequence += 1;
                    self.notify(
                        "Receiving clipboard",
                        "Transfer in progress",
                        Some(&format_id(peer)),
                    );
                    Ok(ticket)
                }
                .await;
                let _ = reply.send(result);
            }
            NetworkEvent::Received {
                ticket,
                payload,
                reply,
            } => {
                let event = ticket.event();
                let peer = format_id(event.origin);
                let result: Result<()> = async {
                    let generation = self.generation.load(Ordering::SeqCst);
                    let current = self.clipboard.call(|c| c.read()).await?;
                    self.state
                        .borrow()
                        .core
                        .may_apply(&ticket, current.stamp)
                        .map_err(|e| format!("receive cancelled: {e:?}"))?;
                    self.store.call(move |s| s.intent(event)).await?;
                    let authorization = self
                        .state
                        .borrow()
                        .core
                        .authorize_apply(&ticket, current.stamp)
                        .map_err(|e| format!("{e:?}"))?;
                    let p = payload.clone();
                    let lease = self.generation.clone();
                    let value = self
                        .clipboard
                        .call(move |c| {
                            if lease.load(Ordering::SeqCst) != generation {
                                return Err("receive permission changed".into());
                            }
                            c.write(&p, authorization)
                        })
                        .await?;
                    // Even if a control arrived during OS readback, this write must
                    // establish a baseline and retain its truthful durable receipt.
                    self.state.borrow_mut().core.written(value.stamp);
                    let p = payload.clone();
                    let target = peer.clone();
                    self.store
                        .call(move |s| {
                            s.record(
                                event,
                                &target,
                                "receive",
                                DeliveryState::Applied,
                                &p,
                                "OS clipboard readback verified",
                            )
                        })
                        .await?;
                    {
                        let mut state = self.state.borrow_mut();
                        state.available = true;
                        state.sequence += 1;
                    }
                    self.notify(
                        "Clipboard received",
                        "System clipboard readback verified",
                        Some(&peer),
                    );
                    Ok(())
                }
                .await;
                if let Err(e) = &result {
                    {
                        let mut state = self.state.borrow_mut();
                        state.last_error = Some(e.clone());
                        state.core.baseline(None);
                    }
                    let detail = e.clone();
                    let _ = self
                        .store
                        .call(move |s| {
                            s.record(
                                event,
                                &peer,
                                "receive",
                                DeliveryState::Failed,
                                &payload,
                                &detail,
                            )
                        })
                        .await;
                }
                let _ = reply.send(result);
            }
            NetworkEvent::Delivery {
                event,
                peer,
                state,
                detail,
            } => {
                let target = peer.clone();
                let message = detail.clone();
                self.store
                    .call(move |s| s.update(event, &target, state, &message))
                    .await?;
                self.state.borrow_mut().sequence += 1;
                self.notify(
                    if state == DeliveryState::Applied {
                        "Clipboard sent"
                    } else {
                        "Clipboard transfer failed"
                    },
                    &detail,
                    Some(&peer),
                );
            }
        }
        Ok(())
    }
    fn notify(&self, title: &str, body: &str, peer: Option<&str>) {
        let state = self.state.borrow();
        if state.settings.notifications
            && !peer
                .and_then(|p| state.settings.peers.get(p))
                .is_some_and(|p| p.quiet)
        {
            let title = title.to_owned();
            let body = body.to_owned();
            // Notifications are optional and bounded independently of core work.
            drop(self.platform.call(move |p| {
                p.notify(&title, &body);
                Ok(())
            }));
        }
    }
    async fn history_list_for(
        &self,
        requester: DeviceId,
        cursor: Option<HistoryCursor>,
        limit: u16,
    ) -> Result<(u64, HistoryListResponse)> {
        if !(HistoryListRequest { cursor, limit }).valid() {
            return Err("invalid history page request".into());
        }
        self.state
            .borrow()
            .core
            .authorize_peer_hints(requester)
            .map_err(|e| format!("{e:?}"))?;
        let offset = cursor.map_or(0, |c| c.offset as usize);
        if offset > 10_000 || cursor.is_some_and(|c| c.source_epoch != self.epoch) {
            return Err("stale history cursor".into());
        }
        let (revision, mut rows) = self
            .store
            .call(move |s| s.local_history(offset, limit as usize + 1))
            .await?;
        if cursor.is_some_and(|c| c.revision != revision) {
            return Err("stale history cursor".into());
        }
        let has_next = rows.len() > limit as usize;
        rows.truncate(limit as usize);
        let state = self.state.borrow();
        state
            .core
            .authorize_peer_hints(requester)
            .map_err(|e| format!("{e:?}"))?;
        rows.retain_mut(|row| {
            if row.event.origin != self.id || row.event.epoch != self.epoch {
                return false;
            }
            if state
                .core
                .authorize_history_export(requester, row.event, &row.metadata, false)
                .is_err()
            {
                return false;
            }
            row.body_available &= state
                .core
                .authorize_history_export(requester, row.event, &row.metadata, true)
                .is_ok();
            if !row.body_available {
                // A status-only item must not expose a guessable content hash.
                row.metadata.digest = [0; 32];
            }
            true
        });
        let next = if has_next {
            let next_offset = offset
                .checked_add(limit as usize)
                .ok_or("history cursor overflow")?;
            Some(HistoryCursor {
                source_epoch: self.epoch,
                revision,
                offset: next_offset
                    .try_into()
                    .map_err(|_| "history cursor overflow")?,
            })
        } else {
            None
        };
        Ok((
            state.core.revision(),
            HistoryListResponse {
                source_epoch: self.epoch,
                revision,
                items: rows,
                next,
            },
        ))
    }
    async fn history_body_for(
        &self,
        requester: DeviceId,
        event: EventId,
    ) -> Result<(u64, Payload)> {
        if event.origin != self.id || event.epoch != self.epoch || event.seq == 0 {
            return Err("history event is not local to this session".into());
        }
        self.state
            .borrow()
            .core
            .authorize_peer_hints(requester)
            .map_err(|e| format!("{e:?}"))?;
        let payload = self.store.call(move |s| s.local_content(event)).await?;
        let state = self.state.borrow();
        state
            .core
            .authorize_history_export(requester, event, &payload.meta, true)
            .map_err(|e| format!("{e:?}"))?;
        Ok((state.core.revision(), payload))
    }
    async fn configure(&self, settings: Settings) -> Result<Answer> {
        if !settings.validate() {
            return Err("invalid settings".into());
        }
        let generation = {
            let mut state = self.state.borrow_mut();
            state.configuration = state
                .configuration
                .checked_add(1)
                .ok_or("configuration exhausted")?;
            self.generation.store(state.configuration, Ordering::SeqCst);
            state.settings = settings.clone();
            state.configuring = true;
            // Revoke immediately, before waiting on disk or the clipboard worker.
            // New permissions become effective only after durable save + baseline.
            state
                .core
                .configure(Settings {
                    send: false,
                    receive: false,
                    ..settings.clone()
                })
                .map_err(|e| format!("{e:?}"))?;
            let revision = state.core.revision();
            state.network.policy_revision(revision);
            state.sequence += 1;
            state.configuration
        };
        let s = settings.clone();
        let result = async {
            self.store
                .call(move |store| store.save_settings(&s))
                .await?;
            self.current(generation)?;
            let observed = self.clipboard.call(|c| c.read()).await;
            self.current(generation)?;
            let mut state = self.state.borrow_mut();
            state.core.baseline(observed.ok().map(|v| v.stamp));
            state
                .core
                .configure(settings.clone())
                .map_err(|e| format!("{e:?}"))?;
            let revision = state.core.revision();
            state.network.policy_revision(revision);
            state.configuring = false;
            state.sequence += 1;
            Ok(Answer::Settings { settings })
        }
        .await;
        if let Err(error) = &result {
            if self.current(generation).is_ok() {
                let mut state = self.state.borrow_mut();
                state.settings.send = false;
                state.settings.receive = false;
                state.configuring = false;
                state.last_error = Some(format!(
                    "settings not saved; synchronization stopped: {error}"
                ));
            }
        }
        result
    }
    async fn action(&self, action: Action) -> Result<Answer> {
        if self.stopping.get() {
            return Err("agent is stopping".into());
        }
        if matches!(action, Action::Quit) {
            self.stopping.set(true);
            let mut state = self.state.borrow_mut();
            state.configuration = state
                .configuration
                .checked_add(1)
                .ok_or("configuration exhausted")?;
            self.generation.store(state.configuration, Ordering::SeqCst);
            let settings = Settings {
                send: false,
                receive: false,
                ..state.settings.clone()
            };
            state
                .core
                .configure(settings)
                .map_err(|e| format!("{e:?}"))?;
            let revision = state.core.revision();
            state.network.policy_revision(revision);
            return Ok(done("Agent is stopping"));
        }
        // These commands can run while data effects are pending. RefCell borrows
        // never span an await and the host polls all futures on one thread.
        match action {
            Action::Status => {
                let s = self.state.borrow();
                let mut devices = DeviceCounts {
                    discovered: s.peers.len(),
                    ..DeviceCounts::default()
                };
                for peer in &s.peers {
                    let policy = s.settings.peers.get(&peer.id).cloned().unwrap_or_default();
                    devices.send += usize::from(s.settings.send && policy.send);
                    devices.receive += usize::from(s.settings.receive && policy.receive);
                }
                return Ok(Answer::Status {
                    status: Status {
                        device: self.device.clone(),
                        clipboard: self.description.clone(),
                        clipboard_available: s.available,
                        last_error: s.last_error.clone(),
                        settings: s.settings.clone(),
                        policy_revision: s.core.revision(),
                        sequence: s.sequence,
                        devices,
                    },
                });
            }
            Action::Settings => {
                return Ok(Answer::Settings {
                    settings: self.state.borrow().settings.clone(),
                });
            }
            Action::Devices => {
                let s = self.state.borrow();
                return Ok(Answer::Devices {
                    devices: s.peers.clone(),
                    settings: s.settings.clone(),
                });
            }
            Action::Configure { expected, settings } => {
                if expected != self.state.borrow().settings {
                    return Err("settings changed; refresh before retrying".into());
                }
                return self.configure(settings).await;
            }
            Action::SetDirections { send, receive } => {
                if send.is_none() && receive.is_none() {
                    return Err("at least one direction is required".into());
                }
                let mut settings = self.state.borrow().settings.clone();
                if let Some(v) = send {
                    settings.send = v;
                }
                if let Some(v) = receive {
                    settings.receive = v;
                }
                return self.configure(settings).await;
            }
            Action::Peer {
                id,
                expected,
                policy,
            } => {
                parse_id(&id)?;
                let mut settings = {
                    let s = self.state.borrow();
                    if !s.peers.iter().any(|p| p.id == id) {
                        return Err("unknown device; discover it first".into());
                    }
                    s.settings.clone()
                };
                if settings.peers.get(&id).cloned().unwrap_or_default() != expected {
                    return Err("device settings changed; refresh before retrying".into());
                }
                settings.peers.insert(id, policy);
                return self.configure(settings).await;
            }
            Action::Refresh => {
                self.state.borrow_mut().network.refresh();
                return Ok(done("Discovery refresh requested"));
            }
            _ => {}
        }
        let _busy = self.enter()?;
        if self.state.borrow().configuring {
            return Err("settings update in progress".into());
        }
        let generation = self.generation.load(Ordering::SeqCst);
        Ok(match action {
            Action::Send => {
                let v = self.clipboard.call(|c| c.read()).await?;
                self.current(generation)?;
                let p = v.payload.ok_or("clipboard has no supported content")?;
                let permits = self
                    .state
                    .borrow_mut()
                    .core
                    .manual(v.stamp, &p.meta)
                    .map_err(|e| format!("{e:?}"))?;
                let n = self.publish(permits, p, "explicit user command").await?;
                done(&format!("Queued for {n} allowed device(s)"))
            }
            Action::History { offset, limit } => Answer::History {
                entries: self
                    .store
                    .call(move |s| {
                        s.prune()?;
                        s.history(offset, limit)
                    })
                    .await?,
            },
            Action::Preview { id } => {
                let p = self.store.call(move |s| s.content(id)).await?;
                Answer::Preview {
                    format: p.meta.format,
                    base64: STANDARD.encode(&p.data),
                }
            }
            Action::Copy { id, local_only } => {
                let p = self.store.call(move |s| s.content(id)).await?;
                let current = self.clipboard.call(|c| c.read()).await?;
                self.current(generation)?;
                let authorization = self
                    .state
                    .borrow()
                    .core
                    .authorize_local_copy(current.stamp, p.meta.clone());
                let payload = p.clone();
                let lease = self.generation.clone();
                let written = self
                    .clipboard
                    .call(move |c| {
                        if lease.load(Ordering::SeqCst) != generation {
                            return Err("copy cancelled by settings change".into());
                        }
                        c.write(&payload, authorization)
                    })
                    .await?;
                self.state.borrow_mut().core.written(written.stamp);
                self.current(generation)?;
                if !local_only && self.state.borrow().settings.automatic {
                    let permits = self
                        .state
                        .borrow_mut()
                        .core
                        .manual(written.stamp, &p.meta)
                        .map_err(|e| format!("{e:?}"))?;
                    self.publish(permits, p, "explicit user command").await?;
                }
                done("Copied history content to the system clipboard")
            }
            Action::Resend { id } => {
                let p = self.store.call(move |s| s.content(id)).await?;
                self.current(generation)?;
                let stamp = ClipboardStamp {
                    generation: 0,
                    digest: p.meta.digest,
                    sensitive: false,
                };
                let permits = self
                    .state
                    .borrow_mut()
                    .core
                    .manual(stamp, &p.meta)
                    .map_err(|e| format!("{e:?}"))?;
                let n = self.publish(permits, p, "explicit user command").await?;
                done(&format!("History queued as a new event for {n} device(s)"))
            }
            Action::ClearHistory => {
                self.store.call(|s| s.clear()).await?;
                self.state.borrow_mut().sequence += 1;
                done("History cleared; replay protection retained")
            }
            Action::Autostart { enabled } => Answer::Autostart {
                status: self.platform.call(move |p| p.autostart(enabled)).await?,
            },
            _ => unreachable!("control action handled above"),
        })
    }
    pub async fn request(&self, request: ControlRequest) -> Answer {
        if request.version != VERSION {
            return Answer::Error {
                message: "unsupported API version".into(),
            };
        }
        self.action(request.action)
            .await
            .unwrap_or_else(|message| Answer::Error { message })
    }
}
fn done(message: &str) -> Answer {
    Answer::Done {
        message: message.into(),
    }
}
fn format_id(id: DeviceId) -> String {
    id.iter().map(|b| format!("{b:02x}")).collect()
}
fn parse_id(id: &str) -> Result<DeviceId> {
    if id.len() != 64 || !id.is_ascii() {
        return Err("expected full 64-character device fingerprint".into());
    }
    let mut bytes = [0; 32];
    for (i, b) in bytes.iter_mut().enumerate() {
        *b = u8::from_str_radix(&id[2 * i..2 * i + 2], 16)
            .map_err(|_| "invalid device fingerprint")?;
    }
    Ok(bytes)
}
