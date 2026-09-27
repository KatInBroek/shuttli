//! Sync use cases. All presentation clients share this service.
use base64::{Engine as _, engine::general_purpose::STANDARD};
use shuttli_api::control::*;
use shuttli_core::sync::SyncCore;
use shuttli_model::sync::*;
use shuttli_ports::sync::*;

pub struct Service {
    core: SyncCore,
    clipboard: Box<dyn Clipboard>,
    store: Box<dyn Store>,
    network: Box<dyn Network>,
    platform: Box<dyn Platform>,
    device: String,
    last_error: Option<String>,
    available: bool,
    sequence: u64,
}
impl Service {
    pub fn new(
        id: DeviceId,
        epoch: [u8; 16],
        clipboard: Box<dyn Clipboard>,
        store: Box<dyn Store>,
        network: Box<dyn Network>,
        platform: Box<dyn Platform>,
    ) -> Result<Self> {
        let settings = store.settings()?;
        let mut core = SyncCore::new(id, epoch, settings);
        for p in store.peers()? {
            core.register(parse_id(&p.id)?, p.id)
                .map_err(|e| format!("{e:?}"))?;
        }
        let mut service = Self {
            core,
            clipboard,
            store,
            network,
            platform,
            device: format_id(id),
            last_error: None,
            available: false,
            sequence: 0,
        };
        service.network.policy_revision(service.core.revision());
        match service.clipboard.read() {
            Ok(v) => {
                service.core.baseline(Some(v.stamp));
                service.available = true;
            }
            Err(e) => service.last_error = Some(e),
        }
        // Query only our own interrupted deliveries. Never resend their bodies.
        for (event, peer) in service.store.pending_receipts()?.into_iter().take(16) {
            let _ = service.network.reconcile(parse_id(&peer)?, event);
        }
        Ok(service)
    }
    pub fn tick(&mut self) {
        self.observe();
        for _ in 0..32 {
            let Some(event) = self.network.poll() else {
                break;
            };
            if let Err(e) = self.network_event(event) {
                self.last_error = Some(e)
            }
        }
    }
    fn observe(&mut self) {
        match self.clipboard.read() {
            Ok(v) => {
                self.available = true;
                if let Some(p) = v.payload {
                    match self.core.observe_local(v.stamp, &p.meta) {
                        Ok(observation) => {
                            if let Some(event) = observation.local_event {
                                match self.store.record(
                                    event,
                                    &self.device,
                                    "local",
                                    DeliveryState::Applied,
                                    &p,
                                    "Local clipboard copy",
                                ) {
                                    Ok(id) if id != 0 => self.sequence += 1,
                                    Ok(_) => {}
                                    Err(e) => self.last_error = Some(e),
                                }
                            }
                            if let Err(e) =
                                self.publish(observation.publications, p, "automatic observation")
                            {
                                self.last_error = Some(e)
                            }
                        }
                        Err(
                            shuttli_core::sync::Rejection::Disabled
                            | shuttli_core::sync::Rejection::Sensitive,
                        ) => {}
                        Err(e) => self.last_error = Some(format!("{e:?}")),
                    }
                } else {
                    self.core.baseline(Some(v.stamp));
                }
            }
            Err(e) => {
                self.available = false;
                self.core.baseline(None);
                self.last_error = Some(e);
            }
        }
    }
    fn publish(
        &mut self,
        permits: Vec<shuttli_core::sync::Publication>,
        p: Payload,
        reason: &str,
    ) -> Result<usize> {
        let count = permits.len();
        for permit in permits {
            let event = permit.event();
            let target = format_id(permit.target());
            self.store.record(
                event,
                &target,
                "send",
                DeliveryState::Sending,
                &p,
                &format!("{reason}; awaiting remote OS readback"),
            )?;
            if let Err(e) = self.network.send(permit, p.clone()) {
                self.store
                    .update(event, &target, DeliveryState::Failed, &e)?;
            }
        }
        if count > 0 {
            self.sequence += 1;
            self.notify("Sending clipboard", &format!("{count} device(s)"), None);
        }
        Ok(count)
    }
    fn network_event(&mut self, event: NetworkEvent) -> Result<()> {
        match event {
            NetworkEvent::ReceiptQuery { peer, event, reply } => {
                let result = if peer == event.origin {
                    self.store.receipt(event)
                } else {
                    Err("receipt query is not owned by this peer".into())
                };
                let _ = reply.send(result);
            }

            NetworkEvent::Peer(p) => {
                let known = self.store.peers()?.iter().any(|x| x.id == p.id);
                self.core
                    .register(parse_id(&p.id)?, p.id.clone())
                    .map_err(|e| format!("{e:?}"))?;
                self.store.peer(&p)?;
                if !known {
                    self.sequence += 1;
                    self.notify(
                        "Device discovered",
                        "Outgoing sync is disabled until you allow this device",
                        Some(&p.id),
                    );
                }
            }
            NetworkEvent::Offer {
                peer,
                event,
                meta,
                reply,
            } => {
                let result = (|| {
                    let ticket = self
                        .core
                        .receive(peer, event, meta)
                        .map_err(|e| format!("receive rejected: {e:?}"))?;
                    if !self.store.reserve(event)? {
                        return Err("duplicate or stale event; no clipboard reapplication".into());
                    }
                    self.store.record(
                        event,
                        &format_id(peer),
                        "receive",
                        DeliveryState::Receiving,
                        &Payload {
                            meta: ticket.metadata().clone(),
                            data: std::sync::Arc::from([]),
                        },
                        "receiving content",
                    )?;
                    self.sequence += 1;
                    self.notify(
                        "Receiving clipboard",
                        "Transfer in progress",
                        Some(&format_id(peer)),
                    );
                    Ok(ticket)
                })();
                let _ = reply.send(result);
            }
            NetworkEvent::Received {
                ticket,
                payload,
                reply,
            } => {
                let result = (|| {
                    let current = self.clipboard.read()?;
                    self.core
                        .may_apply(&ticket, current.stamp)
                        .map_err(|e| format!("receive cancelled: {e:?}"))?;
                    self.store.intent(ticket.event())?;
                    let value = match self.clipboard.write(
                        &payload,
                        self.core
                            .authorize_apply(&ticket, current.stamp)
                            .map_err(|e| format!("{e:?}"))?,
                    ) {
                        Ok(v) => v,
                        Err(e) => {
                            self.core.baseline(None);
                            return Err(e);
                        }
                    };
                    self.core.written(value.stamp);
                    self.store.record(
                        ticket.event(),
                        &format_id(ticket.event().origin),
                        "receive",
                        DeliveryState::Applied,
                        &payload,
                        "OS clipboard readback verified",
                    )?;
                    self.available = true;
                    self.sequence += 1;
                    self.notify(
                        "Clipboard received",
                        "System clipboard readback verified",
                        Some(&format_id(ticket.event().origin)),
                    );
                    Ok(())
                })();
                if let Err(e) = &result {
                    self.last_error = Some(e.clone());
                    let _ = self.store.record(
                        ticket.event(),
                        &format_id(ticket.event().origin),
                        "receive",
                        DeliveryState::Failed,
                        &payload,
                        e,
                    );
                }
                let _ = reply.send(result);
            }
            NetworkEvent::Delivery {
                event,
                peer,
                state,
                detail,
            } => {
                self.store.update(event, &peer, state, &detail)?;
                self.sequence += 1;
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
    fn notify(&mut self, title: &str, body: &str, peer: Option<&str>) {
        if self.core.settings().notifications
            && !peer
                .and_then(|p| self.core.settings().peers.get(p))
                .is_some_and(|p| p.quiet)
        {
            self.platform.notify(title, body);
        }
    }
    fn configure(&mut self, s: Settings) -> Result<Answer> {
        if !s.validate() {
            return Err("invalid settings".into());
        }
        if let Err(e) = self.store.save_settings(&s) {
            self.core
                .configure(Settings {
                    send: false,
                    receive: false,
                    ..self.core.settings().clone()
                })
                .map_err(|e| format!("{e:?}"))?;
            self.network.policy_revision(self.core.revision());
            return Err(format!("settings not saved; synchronization stopped: {e}"));
        }
        self.core
            .configure(s.clone())
            .map_err(|e| format!("{e:?}"))?;
        self.network.policy_revision(self.core.revision());
        // Reconfiguration never republishes the clipboard or queues a disabled interval.
        match self.clipboard.read() {
            Ok(v) => self.core.baseline(Some(v.stamp)),
            Err(_) => self.core.baseline(None),
        }
        self.sequence += 1;
        Ok(Answer::Settings { settings: s })
    }
    fn action(&mut self, a: Action) -> Result<Answer> {
        Ok(match a {
            Action::Status => Answer::Status {
                status: Status {
                    device: self.device.clone(),
                    clipboard: self.clipboard.description().into(),
                    clipboard_available: self.available,
                    last_error: self.last_error.clone(),
                    settings: self.core.settings().clone(),
                    policy_revision: self.core.revision(),
                    sequence: self.sequence,
                },
            },
            Action::Devices => Answer::Devices {
                devices: self.store.peers()?,
                settings: self.core.settings().clone(),
            },
            Action::Settings => Answer::Settings {
                settings: self.core.settings().clone(),
            },
            Action::Configure { settings } => return self.configure(settings),
            Action::SetDirections { send, receive } => {
                if send.is_none() && receive.is_none() {
                    return Err("at least one direction is required".into());
                }
                let mut settings = self.core.settings().clone();
                if let Some(value) = send {
                    settings.send = value;
                }
                if let Some(value) = receive {
                    settings.receive = value;
                }
                return self.configure(settings);
            }
            Action::Peer { id, policy } => {
                parse_id(&id)?;
                if !self.store.peers()?.iter().any(|p| p.id == id) {
                    return Err("unknown device; discover it first".into());
                }
                let mut s = self.core.settings().clone();
                s.peers.insert(id, policy);
                return self.configure(s);
            }
            Action::Refresh => {
                self.network.refresh();
                done("Discovery refresh requested")
            }
            Action::Send => {
                let v = self.clipboard.read()?;
                let p = v.payload.ok_or("clipboard has no supported content")?;
                let permits = self
                    .core
                    .manual(v.stamp, &p.meta)
                    .map_err(|e| format!("{e:?}"))?;
                let n = self.publish(permits, p, "explicit user command")?;
                done(&format!("Queued for {n} allowed device(s)"))
            }
            Action::History { offset, limit } => {
                self.store.prune()?;
                Answer::History {
                    entries: self.store.history(offset, limit)?,
                }
            }
            Action::Preview { id } => {
                let p = self.store.content(id)?;
                Answer::Preview {
                    format: p.meta.format,
                    base64: STANDARD.encode(&p.data),
                }
            }
            Action::Copy { id, local_only } => {
                let p = self.store.content(id)?;
                let current = self.clipboard.read()?;
                let written = self.clipboard.write(
                    &p,
                    self.core
                        .authorize_local_copy(current.stamp, p.meta.clone()),
                )?;
                self.core.written(written.stamp);
                if !local_only && self.core.settings().automatic {
                    let permits = self
                        .core
                        .manual(written.stamp, &p.meta)
                        .map_err(|e| format!("{e:?}"))?;
                    self.publish(permits, p, "explicit user command")?;
                }
                done("Copied history content to the system clipboard")
            }
            Action::Resend { id } => {
                let p = self.store.content(id)?;
                let stamp = ClipboardStamp {
                    generation: 0,
                    digest: p.meta.digest,
                    sensitive: false,
                };
                let permits = self
                    .core
                    .manual(stamp, &p.meta)
                    .map_err(|e| format!("{e:?}"))?;
                let n = self.publish(permits, p, "explicit user command")?;
                done(&format!("History queued as a new event for {n} device(s)"))
            }
            Action::ClearHistory => {
                self.store.clear()?;
                self.sequence += 1;
                done("History cleared; replay protection retained")
            }
            Action::Autostart { enabled } => done(&self.platform.autostart(enabled)?),
        })
    }
}
impl ControlApi for Service {
    fn request(&mut self, r: ControlRequest) -> Answer {
        if r.version != VERSION {
            return Answer::Error {
                message: "unsupported API version".into(),
            };
        }
        match self.action(r.action) {
            Ok(a) => a,
            Err(e) => Answer::Error { message: e },
        }
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
