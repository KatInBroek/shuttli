//! Shared values; no I/O and no authority to publish.
extern crate alloc;
use alloc::{collections::BTreeMap, string::String};
use serde::{Deserialize, Serialize};

pub type DeviceId = [u8; 32];
pub type Digest = [u8; 32];
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct EventId {
    pub origin: DeviceId,
    pub epoch: [u8; 16],
    pub seq: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    Text,
    Png,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Metadata {
    pub format: Format,
    pub size: u64,
    pub digest: Digest,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PeerPolicy {
    pub send: bool,
    pub receive: bool,
    pub text: bool,
    pub png: bool,
    pub max_bytes: u64,
    pub quiet: bool,
    #[serde(default)]
    pub history: Option<HistoryMode>,
}
impl Default for PeerPolicy {
    fn default() -> Self {
        Self {
            send: false,
            receive: true,
            text: true,
            png: true,
            max_bytes: 8 * 1024 * 1024,
            quiet: false,
            history: None,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryMode {
    Off,
    Status,
    Content,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub version: u16,
    pub send: bool,
    pub receive: bool,
    pub automatic: bool,
    pub text: bool,
    pub png: bool,
    pub notifications: bool,
    pub history: HistoryMode,
    pub history_limit: usize,
    pub history_days: u32,
    pub history_bytes: u64,
    #[serde(default = "default_history_memory_bytes")]
    pub history_memory_bytes: u64,
    pub peers: BTreeMap<String, PeerPolicy>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            send: true,
            receive: true,
            automatic: true,
            text: true,
            png: true,
            notifications: true,
            history: HistoryMode::Content,
            history_limit: 20,
            history_days: 7,
            history_bytes: 128 * 1024 * 1024,
            history_memory_bytes: default_history_memory_bytes(),
            peers: BTreeMap::new(),
        }
    }
}
impl Settings {
    pub fn validate(&self) -> bool {
        self.version == 1
            && self.history_limit <= 10_000
            && self.history_days <= 365
            && self.history_bytes <= 1024 * 1024 * 1024
            && self.history_memory_bytes <= 64 * 1024 * 1024
            && self.peers.len() <= 256
            && self.peers.values().all(|p| p.max_bytes <= 8 * 1024 * 1024)
    }
    pub fn closed() -> Self {
        Self {
            send: false,
            receive: false,
            ..Self::default()
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryState {
    Sending,
    Receiving,
    Applied,
    Failed,
    Cancelled,
    Superseded,
    Unknown,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub id: i64,
    pub event: EventId,
    pub peer: String,
    pub direction: String,
    pub state: DeliveryState,
    pub format: Format,
    pub bytes: u64,
    pub time: u64,
    pub available: bool,
    pub detail: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PeerInfo {
    pub id: String,
    pub name: String,
    pub address: String,
    pub online: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClipboardStamp {
    pub generation: u64,
    pub digest: Digest,
    pub sensitive: bool,
}

fn default_history_memory_bytes() -> u64 {
    16 * 1024 * 1024
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutostartState {
    Enabled,
    Disabled,
    RequiresUserAction,
    Unavailable,
    Unknown,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutostartStatus {
    pub state: AutostartState,
    pub message: String,
}
