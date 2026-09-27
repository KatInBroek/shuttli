extern crate alloc;
use alloc::{string::String, vec::Vec};
use serde::{Deserialize, Serialize};
use shuttli_model::sync::{Format, HistoryEntry, PeerInfo, PeerPolicy, Settings};
pub const VERSION: u16 = 1;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Status,
    Devices,
    Settings,
    Configure {
        settings: Settings,
    },
    /// Atomic direction changes do not overwrite another client's preferences.
    SetDirections {
        send: Option<bool>,
        receive: Option<bool>,
    },
    Peer {
        id: String,
        policy: PeerPolicy,
    },
    Send,
    History {
        offset: usize,
        limit: usize,
    },
    Preview {
        id: i64,
    },
    Copy {
        id: i64,
        local_only: bool,
    },
    Resend {
        id: i64,
    },
    ClearHistory,
    Refresh,
    Autostart {
        enabled: Option<bool>,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlRequest {
    pub version: u16,
    pub action: Action,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Status {
    pub device: String,
    pub clipboard: String,
    pub clipboard_available: bool,
    pub last_error: Option<String>,
    pub settings: Settings,
    pub policy_revision: u64,
    pub sequence: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Answer {
    Status {
        status: Status,
    },
    Devices {
        devices: Vec<PeerInfo>,
        settings: Settings,
    },
    Settings {
        settings: Settings,
    },
    History {
        entries: Vec<HistoryEntry>,
    },
    Preview {
        format: Format,
        base64: String,
    },
    Done {
        message: String,
    },
    Error {
        message: String,
    },
}
pub trait ControlApi {
    fn request(&mut self, request: ControlRequest) -> Answer;
}
