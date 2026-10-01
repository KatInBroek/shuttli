//! Small, versioned native boundary. Only UI-safe operations cross UniFFI;
//! authority tickets, TLS state and database handles remain in Rust.
use shuttli_mobile_sdk::{
    HistoryMode, Identity, MobileHistory,
    peers::{Directions, MAX_DIRECT_PEERS, PeerDirectory},
    transport::{MobileTransport, SendCommand, SendResults, SendState},
};
use shuttli_model::sync::{EventId, Format, Metadata};
use std::time::{SystemTime, UNIX_EPOCH};
use std::{
    collections::BTreeMap,
    net::Ipv4Addr,
    path::Path,
    sync::{Arc, Mutex},
};

uniffi::setup_scaffolding!();

#[uniffi::export]
pub fn sdk_api_version() -> u32 {
    7
}

#[derive(Clone, uniffi::Enum)]
pub enum MobileHistoryActivity {
    Waiting,
    Updating,
    Receiving,
    Updated,
    Denied,
    Failed,
    Paused,
    Unavailable,
    Unsupported,
}

impl From<shuttli_mobile_sdk::HistoryActivity> for MobileHistoryActivity {
    fn from(value: shuttli_mobile_sdk::HistoryActivity) -> Self {
        use shuttli_mobile_sdk::HistoryActivity as H;
        match value {
            H::Waiting => Self::Waiting,
            H::Updating => Self::Updating,
            H::Receiving => Self::Receiving,
            H::Updated => Self::Updated,
            H::Denied => Self::Denied,
            H::Failed => Self::Failed,
            H::Paused => Self::Paused,
        }
    }
}

#[uniffi::export]
pub fn generate_identity_bytes() -> Vec<u8> {
    Identity::generate().map_or_else(|_| Vec::new(), |(_, bytes)| bytes)
}

#[derive(Clone, uniffi::Enum)]
pub enum MobileContentKind {
    Text,
    Image,
}

#[derive(Clone, uniffi::Enum)]
pub enum MobileHistoryMode {
    Off,
    Status,
    Content,
}

impl From<MobileHistoryMode> for HistoryMode {
    fn from(value: MobileHistoryMode) -> Self {
        match value {
            MobileHistoryMode::Off => Self::Off,
            MobileHistoryMode::Status => Self::Status,
            MobileHistoryMode::Content => Self::Content,
        }
    }
}

impl From<HistoryMode> for MobileHistoryMode {
    fn from(value: HistoryMode) -> Self {
        match value {
            HistoryMode::Off => Self::Off,
            HistoryMode::Status => Self::Status,
            HistoryMode::Content => Self::Content,
        }
    }
}

#[derive(Clone, uniffi::Record)]
pub struct MobileHistoryRow {
    pub event_key: String,
    pub source_name: String,
    pub copied_at_ms: u64,
    pub kind: MobileContentKind,
    pub bytes: u64,
    pub available: bool,
    pub is_local: bool,
    pub receiving: bool,
}

#[derive(Clone, uniffi::Record)]
pub struct MobileDeviceRow {
    pub id: String,
    pub name: String,
    pub online: bool,
    pub send: bool,
    pub receive: bool,
    pub history_activity: MobileHistoryActivity,
    pub checked_at_ms: u64,
    pub history_partial: bool,
    pub text: bool,
    pub image: bool,
    pub endpoint: String,
}

#[derive(Clone, uniffi::Enum)]
pub enum MobileTransferState {
    Queued,
    Sending,
    Applied,
    Failed,
    Unknown,
}

#[derive(Clone, uniffi::Record)]
pub struct MobileTransferRow {
    pub event_key: String,
    pub target_id: String,
    pub target_name: String,
    pub state: MobileTransferState,
}

fn event_key(event: EventId) -> String {
    let mut bytes = Vec::with_capacity(56);
    bytes.extend_from_slice(&event.origin);
    bytes.extend_from_slice(&event.epoch);
    bytes.extend_from_slice(&event.seq.to_be_bytes());
    hex::encode(bytes)
}

fn parse_event_key(key: &str) -> Option<EventId> {
    let bytes = hex::decode(key).ok()?;
    if bytes.len() != 56 {
        return None;
    }
    Some(EventId {
        origin: bytes[..32].try_into().ok()?,
        epoch: bytes[32..48].try_into().ok()?,
        seq: u64::from_be_bytes(bytes[48..56].try_into().ok()?),
    })
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[derive(uniffi::Object)]
pub struct MobileSession {
    history: Arc<Mutex<MobileHistory>>,
    peers: Mutex<Option<Arc<Mutex<PeerDirectory>>>>,
    saved_directions: Mutex<BTreeMap<shuttli_model::sync::DeviceId, Directions>>,
    listener: Mutex<Option<MobileTransport>>,
    results: SendResults,
}

#[uniffi::export]
impl MobileSession {
    #[uniffi::constructor]
    pub fn new() -> Self {
        Self {
            history: Arc::new(Mutex::new(MobileHistory::default())),
            peers: Mutex::new(None),
            saved_directions: Mutex::new(BTreeMap::new()),
            listener: Mutex::new(None),
            results: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    pub fn enter_foreground(&self) -> u64 {
        self.history
            .lock()
            .expect("mobile session lock")
            .enter_foreground()
    }

    pub fn enter_background(&self) {
        if let Some(mut listener) = self.listener.lock().expect("mobile listener lock").take() {
            listener.stop();
        }
        if let Some(peers) = self.peers.lock().expect("mobile peers lock").as_ref() {
            let mut directory = peers.lock().expect("mobile directory lock");
            for peer in directory.direct() {
                directory.disconnected(peer.id);
            }
        }
        self.history
            .lock()
            .expect("mobile session lock")
            .enter_background();
    }

    /// Returns an empty string on success, a user-safe failure reason otherwise.
    /// The native app owns identity persistence and supplies its VPN IPv4.
    pub fn start_listener(
        &self,
        identity_bytes: Vec<u8>,
        tailscale_ip: String,
        name: String,
    ) -> String {
        if !self
            .history
            .lock()
            .expect("mobile session lock")
            .is_active()
        {
            return "Open the app before starting the listener".into();
        }
        let identity = match Identity::from_bytes(&identity_bytes) {
            Ok(identity) => identity,
            Err(_) => return "Device identity is unavailable".into(),
        };
        let ip: Ipv4Addr = match tailscale_ip.parse() {
            Ok(ip) => ip,
            Err(_) => return "Tailscale IPv4 address is unavailable".into(),
        };
        let peers = {
            let mut slot = self.peers.lock().expect("mobile peers lock");
            if let Some(existing) = slot.as_ref() {
                if existing.lock().expect("mobile directory lock").own_id() != identity.id {
                    return "Device identity changed during this session".into();
                }
            } else {
                let mut directory = PeerDirectory::new(identity.id);
                for (&id, &directions) in self
                    .saved_directions
                    .lock()
                    .expect("mobile consent lock")
                    .iter()
                {
                    directory.restore_directions(id, directions);
                }
                *slot = Some(Arc::new(Mutex::new(directory)));
            }
            slot.as_ref().expect("directory created").clone()
        };
        let mut listener = self.listener.lock().expect("mobile listener lock");
        if listener.is_some() {
            return String::new();
        }
        match MobileTransport::start(
            ip,
            identity,
            name,
            self.history.clone(),
            peers,
            self.results.clone(),
        ) {
            Ok(started) => {
                *listener = Some(started);
                String::new()
            }
            Err(_) => "Could not listen on the Tailscale address".into(),
        }
    }

    pub fn connected_peers_count(&self) -> u32 {
        self.peers
            .lock()
            .expect("mobile peers lock")
            .as_ref()
            .map_or(0, |peers| {
                peers
                    .lock()
                    .expect("mobile directory lock")
                    .direct()
                    .iter()
                    .filter(|p| p.online)
                    .count() as u32
            })
    }

    pub fn history_count(&self) -> u32 {
        let history = self.history.lock().expect("mobile session lock");
        history.timeline().len() as u32
    }

    pub fn history_mode(&self) -> MobileHistoryMode {
        self.history
            .lock()
            .expect("mobile session lock")
            .mode()
            .into()
    }

    pub fn configure_image_cache(&self, cache_directory: String) -> bool {
        if cache_directory.is_empty() || cache_directory.len() > 4096 {
            return false;
        }
        self.history
            .lock()
            .expect("mobile session lock")
            .configure_image_cache(Path::new(&cache_directory))
            .is_ok()
    }

    pub fn history_limit(&self) -> u32 {
        self.history.lock().expect("mobile session lock").limit() as u32
    }

    pub fn set_history_mode(&self, mode: MobileHistoryMode) {
        self.history
            .lock()
            .expect("mobile session lock")
            .set_mode(mode.into());
    }

    pub fn set_history_limit(&self, limit: u32) -> bool {
        self.history
            .lock()
            .expect("mobile session lock")
            .set_limit(limit as usize)
    }

    pub fn history_rows(&self) -> Vec<MobileHistoryRow> {
        let own = self
            .peers
            .lock()
            .expect("mobile peers lock")
            .as_ref()
            .map(|peers| peers.lock().expect("mobile directory lock").own_id());
        let names: std::collections::BTreeMap<_, _> = self
            .peers
            .lock()
            .expect("mobile peers lock")
            .as_ref()
            .map_or_else(std::collections::BTreeMap::new, |peers| {
                peers
                    .lock()
                    .expect("mobile directory lock")
                    .direct()
                    .into_iter()
                    .map(|p| (p.id, p.name))
                    .collect()
            });
        let history = self.history.lock().expect("mobile session lock");
        history
            .timeline()
            .into_iter()
            .map(|row| MobileHistoryRow {
                event_key: event_key(row.summary.event),
                source_name: names
                    .get(&row.source)
                    .cloned()
                    .unwrap_or_else(|| hex::encode(&row.source[..4])),
                copied_at_ms: row.summary.copied_at_ms,
                kind: match row.summary.metadata.format {
                    Format::Text => MobileContentKind::Text,
                    Format::Png => MobileContentKind::Image,
                },
                bytes: row.summary.metadata.size,
                available: history.body_available(row.summary.event),
                is_local: own == Some(row.source),
                receiving: history
                    .source(row.source)
                    .is_some_and(|source| source.receiving == Some(row.summary.event)),
            })
            .collect()
    }

    pub fn history_body(&self, event_key: String) -> Vec<u8> {
        parse_event_key(&event_key)
            .and_then(|event| self.history.lock().ok()?.body_for_explicit_copy(event))
            .map_or_else(Vec::new, |body| body.to_vec())
    }

    pub fn clear_history(&self) {
        self.history.lock().expect("mobile session lock").clear();
        self.results.lock().expect("mobile results lock").clear();
        if let Some(listener) = self.listener.lock().expect("mobile listener lock").as_ref() {
            listener.request_refresh();
        }
    }

    pub fn device_rows(&self) -> Vec<MobileDeviceRow> {
        self.peers
            .lock()
            .expect("mobile peers lock")
            .as_ref()
            .map_or_else(Vec::new, |peers| {
                peers
                    .lock()
                    .expect("mobile directory lock")
                    .direct()
                    .into_iter()
                    .map(|peer| {
                        let history = self.history.lock().expect("mobile history lock");
                        let freshness = history.source(peer.id);
                        let activity = if !peer.directions.receive || !history.query_enabled() {
                            MobileHistoryActivity::Paused
                        } else if !peer.online {
                            MobileHistoryActivity::Unavailable
                        } else if !peer.capabilities.history_pull {
                            MobileHistoryActivity::Unsupported
                        } else {
                            freshness.map_or(MobileHistoryActivity::Waiting, |s| s.activity.into())
                        };
                        MobileDeviceRow {
                            id: hex::encode(peer.id),
                            name: peer.name,
                            online: peer.online,
                            send: peer.directions.send,
                            receive: peer.directions.receive,
                            history_activity: activity,
                            checked_at_ms: freshness.map_or(0, |s| s.checked_at_ms),
                            history_partial: freshness.is_some_and(|s| s.partial),
                            text: peer.directions.text,
                            image: peer.directions.image,
                            endpoint: peer.endpoint,
                        }
                    })
                    .collect()
            })
    }

    pub fn set_receive(&self, peer_id: String, enabled: bool) -> bool {
        let Ok(bytes) = hex::decode(peer_id) else {
            return false;
        };
        let Ok(id) = <[u8; 32]>::try_from(bytes) else {
            return false;
        };
        self.peers
            .lock()
            .expect("mobile peers lock")
            .as_ref()
            .is_some_and(|peers| {
                let Ok(mut directory) = peers.lock() else {
                    return false;
                };
                let Some(peer) = directory.direct().into_iter().find(|p| p.id == id) else {
                    return false;
                };
                directory
                    .directions(
                        id,
                        shuttli_mobile_sdk::peers::Directions {
                            receive: enabled,
                            ..peer.directions
                        },
                    )
                    .is_ok()
            })
    }

    pub fn set_send(&self, peer_id: String, enabled: bool) -> bool {
        let Ok(bytes) = hex::decode(peer_id) else {
            return false;
        };
        let Ok(id) = <[u8; 32]>::try_from(bytes) else {
            return false;
        };
        self.peers
            .lock()
            .expect("mobile peers lock")
            .as_ref()
            .is_some_and(|peers| {
                let Ok(mut directory) = peers.lock() else {
                    return false;
                };
                let Some(peer) = directory.direct().into_iter().find(|p| p.id == id) else {
                    return false;
                };
                directory
                    .directions(
                        id,
                        shuttli_mobile_sdk::peers::Directions {
                            send: enabled,
                            ..peer.directions
                        },
                    )
                    .is_ok()
            })
    }

    pub fn identity_fingerprint(&self, identity_bytes: Vec<u8>) -> String {
        Identity::from_bytes(&identity_bytes)
            .map_or_else(|_| String::new(), |identity| hex::encode(identity.id))
    }

    pub fn request_history_refresh(&self) -> bool {
        if let Some(listener) = self.listener.lock().expect("mobile listener lock").as_ref() {
            listener.request_refresh();
            true
        } else {
            false
        }
    }

    pub fn restore_device_directions(&self, peer_id: String, send: bool, receive: bool) -> bool {
        self.restore_device_policy(peer_id, send, receive, true, true)
    }

    pub fn restore_device_policy(
        &self,
        peer_id: String,
        send: bool,
        receive: bool,
        text: bool,
        image: bool,
    ) -> bool {
        let Ok(bytes) = hex::decode(peer_id) else {
            return false;
        };
        let Ok(id) = <[u8; 32]>::try_from(bytes) else {
            return false;
        };
        let peers = self.peers.lock().expect("mobile peers lock");
        let mut saved = self.saved_directions.lock().expect("mobile consent lock");
        if saved.len() >= MAX_DIRECT_PEERS && !saved.contains_key(&id) {
            return false;
        }
        let directions = Directions {
            send,
            receive,
            text,
            image,
        };
        if let Some(peers) = peers.as_ref() {
            if !peers
                .lock()
                .expect("mobile directory lock")
                .restore_directions(id, directions)
            {
                return false;
            }
        }
        saved.insert(id, directions);
        true
    }

    pub fn send_text(&self, text: String) -> u32 {
        if text.is_empty() || text.len() > 1024 * 1024 || text.chars().any(|c| c == '\0') {
            return 0;
        }
        self.send_payload(Format::Text, text.into_bytes().into())
    }

    pub fn send_image(&self, png_bytes: Vec<u8>) -> u32 {
        if shuttli_content::canonical_digest(Format::Png, &png_bytes).is_err() {
            return 0;
        }
        self.send_payload(Format::Png, png_bytes.into())
    }

    pub fn resend_local(&self, key: String) -> u32 {
        let Some(event) = parse_event_key(&key) else {
            return 0;
        };
        let own = self
            .peers
            .lock()
            .expect("mobile peers lock")
            .as_ref()
            .map(|peers| peers.lock().expect("mobile directory lock").own_id());
        if own != Some(event.origin) {
            return 0;
        }
        let history = self.history.lock().expect("mobile session lock");
        let row = history
            .timeline()
            .into_iter()
            .find(|row| row.summary.event == event);
        let payload = row.and_then(|row| {
            history
                .body_for_explicit_copy(event)
                .map(|body| (row.summary.metadata.format, body))
        });
        drop(history);
        payload.map_or(0, |(format, body)| self.send_payload(format, body))
    }

    pub fn transfer_rows(&self) -> Vec<MobileTransferRow> {
        let names: BTreeMap<_, _> = self
            .peers
            .lock()
            .expect("mobile peers lock")
            .as_ref()
            .map_or_else(BTreeMap::new, |peers| {
                peers
                    .lock()
                    .expect("mobile directory lock")
                    .direct()
                    .into_iter()
                    .map(|p| (p.id, p.name))
                    .collect()
            });
        self.results
            .lock()
            .expect("mobile results lock")
            .iter()
            .rev()
            .map(|((event, target), state)| MobileTransferRow {
                event_key: event_key(*event),
                target_id: hex::encode(target),
                target_name: names
                    .get(target)
                    .cloned()
                    .unwrap_or_else(|| hex::encode(&target[..4])),
                state: match state {
                    SendState::Queued => MobileTransferState::Queued,
                    SendState::Sending => MobileTransferState::Sending,
                    SendState::Applied => MobileTransferState::Applied,
                    SendState::Failed => MobileTransferState::Failed,
                    SendState::Unknown => MobileTransferState::Unknown,
                },
            })
            .collect()
    }
}

impl MobileSession {
    fn send_payload(&self, format: Format, bytes: Arc<[u8]>) -> u32 {
        if !self
            .history
            .lock()
            .expect("mobile session lock")
            .is_active()
        {
            return 0;
        }
        let listener = self.listener.lock().expect("mobile listener lock");
        let Some(listener) = listener.as_ref() else {
            return 0;
        };
        let directory = self.peers.lock().expect("mobile peers lock");
        let Some(directory) = directory.as_ref() else {
            return 0;
        };
        let mut directory = directory.lock().expect("mobile directory lock");
        let metadata = Metadata {
            format,
            size: bytes.len() as u64,
            digest: match shuttli_content::canonical_digest(format, &bytes) {
                Ok(digest) => digest,
                Err(_) => return 0,
            },
        };
        let permits = match directory.manual(&metadata) {
            Ok(permits) if !permits.is_empty() => permits,
            _ => return 0,
        };
        let event = permits[0].event();
        let mut queued = 0;
        let mut results = self.results.lock().expect("mobile results lock");
        for permit in permits {
            let target = permit.target();
            let command = SendCommand {
                permit: Arc::new(permit),
                body: bytes.clone(),
            };
            let state = if listener.enqueue(command) {
                queued += 1;
                SendState::Queued
            } else {
                SendState::Failed
            };
            results.insert((event, target), state);
        }
        while results.len() > 100 {
            results.pop_first();
        }
        drop(results);
        if queued > 0 {
            let mut history = self.history.lock().expect("mobile session lock");
            let generation = history.generation();
            let _ = history.record_local_sent(generation, event, metadata, bytes, now_ms());
        }
        queued
    }
}

impl Default for MobileSession {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "tests/lib.rs"]
mod tests;
