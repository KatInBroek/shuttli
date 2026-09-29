//! Small, versioned native boundary. Only UI-safe operations cross UniFFI;
//! authority tickets, TLS state and database handles remain in Rust.
use sha2::{Digest, Sha256};
use shuttli_mobile_sdk::{
    Identity, MobileHistory,
    peers::PeerDirectory,
    transport::{MobileTransport, SendCommand, SendResults, SendState},
};
use shuttli_model::sync::{EventId, Format, Metadata};
use std::{
    collections::BTreeMap,
    net::Ipv4Addr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

uniffi::setup_scaffolding!();

#[uniffi::export]
pub fn sdk_api_version() -> u32 {
    2
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

#[derive(Clone, uniffi::Record)]
pub struct MobileHistoryRow {
    pub event_key: String,
    pub source_name: String,
    pub copied_at_ms: u64,
    pub kind: MobileContentKind,
    pub bytes: u64,
    pub available: bool,
}

#[derive(Clone, uniffi::Record)]
pub struct MobileDeviceRow {
    pub id: String,
    pub name: String,
    pub online: bool,
    pub send: bool,
    pub receive: bool,
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

#[derive(uniffi::Object)]
pub struct MobileSession {
    history: Arc<Mutex<MobileHistory>>,
    peers: Mutex<Option<Arc<Mutex<PeerDirectory>>>>,
    listener: Mutex<Option<MobileTransport>>,
    results: SendResults,
    sequence: AtomicU64,
}

#[uniffi::export]
impl MobileSession {
    #[uniffi::constructor]
    pub fn new() -> Self {
        Self {
            history: Arc::new(Mutex::new(MobileHistory::default())),
            peers: Mutex::new(None),
            listener: Mutex::new(None),
            results: Arc::new(Mutex::new(BTreeMap::new())),
            sequence: AtomicU64::new(1),
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
                *slot = Some(Arc::new(Mutex::new(PeerDirectory::new(identity.id))));
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
        self.history
            .lock()
            .expect("mobile session lock")
            .timeline()
            .len() as u32
    }

    pub fn history_rows(&self) -> Vec<MobileHistoryRow> {
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
        self.history
            .lock()
            .expect("mobile session lock")
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
                available: row.body.is_some(),
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
                    .map(|peer| MobileDeviceRow {
                        id: hex::encode(peer.id),
                        name: peer.name,
                        online: peer.online,
                        send: peer.directions.send,
                        receive: peer.directions.receive,
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

    pub fn send_text(&self, text: String) -> u32 {
        if text.is_empty() || text.len() > 1024 * 1024 || text.chars().any(|c| c == '\0') {
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
        let directory = directory.lock().expect("mobile directory lock");
        let targets: Vec<_> = directory
            .direct()
            .into_iter()
            .filter(|p| p.online && p.directions.send)
            .collect();
        if targets.is_empty() {
            return 0;
        }
        let seq = self.sequence.fetch_add(1, Ordering::SeqCst);
        if seq == 0 || seq == u64::MAX {
            return 0;
        }
        let event = EventId {
            origin: directory.own_id(),
            epoch: listener.epoch(),
            seq,
        };
        let bytes: Arc<[u8]> = text.into_bytes().into();
        let metadata = Metadata {
            format: Format::Text,
            size: bytes.len() as u64,
            digest: Sha256::digest(&bytes).into(),
        };
        let mut queued = 0;
        let mut results = self.results.lock().expect("mobile results lock");
        for target in targets {
            let command = SendCommand {
                event,
                target: target.id,
                metadata: metadata.clone(),
                body: bytes.clone(),
            };
            let state = if listener.enqueue(command) {
                queued += 1;
                SendState::Queued
            } else {
                SendState::Failed
            };
            results.insert((event, target.id), state);
        }
        while results.len() > 100 {
            results.pop_first();
        }
        queued
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

impl Default for MobileSession {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use shuttli_model::mobile::{HistoryListResponse, HistorySummary};
    use shuttli_model::sync::Metadata;

    #[test]
    fn native_bridge_preserves_one_session_across_lifecycle() {
        let session = MobileSession::new();
        assert_eq!(sdk_api_version(), 2);
        let first = session.enter_foreground();
        session.enter_background();
        assert!(session.enter_foreground() > first);
        assert_eq!(session.history_count(), 0);
        assert_eq!(session.connected_peers_count(), 0);
        assert!(!generate_identity_bytes().is_empty());
        assert!(
            !session
                .start_listener(vec![], "100.64.0.1".into(), "Phone".into())
                .is_empty()
        );
    }

    #[test]
    fn event_keys_are_exact_and_cannot_select_another_item() {
        let event = EventId {
            origin: [4; 32],
            epoch: [5; 16],
            seq: 7,
        };
        assert_eq!(parse_event_key(&event_key(event)), Some(event));
        assert_eq!(parse_event_key("invalid"), None);
        assert_eq!(parse_event_key(&"aa".repeat(57)), None);
    }

    #[test]
    fn native_history_returns_only_verified_cached_content() {
        let session = MobileSession::new();
        let generation = session.enter_foreground();
        let event = EventId {
            origin: [2; 32],
            epoch: [3; 16],
            seq: 1,
        };
        let data = b"copy me";
        let mut cache = session.history.lock().unwrap();
        cache
            .merge_page(
                generation,
                event.origin,
                HistoryListResponse {
                    source_epoch: event.epoch,
                    revision: 1,
                    items: vec![HistorySummary {
                        event,
                        metadata: Metadata {
                            format: Format::Text,
                            size: data.len() as u64,
                            digest: Sha256::digest(data).into(),
                        },
                        copied_at_ms: 1,
                        body_available: true,
                    }],
                    next: None,
                },
                1,
            )
            .unwrap();
        drop(cache);
        let key = session.history_rows()[0].event_key.clone();
        assert!(session.history_body(key.clone()).is_empty());
        session
            .history
            .lock()
            .unwrap()
            .cache_body(generation, event, data.to_vec())
            .unwrap();
        assert_eq!(session.history_body(key), data);
        session.clear_history();
        assert!(session.history_rows().is_empty());
    }
}
