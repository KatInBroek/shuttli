//! Platform ports for the running application. Payloads are bounded in adapters.
use shuttli_core::sync::{Publication, Reception, WriteAuthorization};
use shuttli_model::mobile::{HistoryCursor, HistoryListResponse, HistorySummary};
use shuttli_model::sync::*;
use std::sync::{Arc, mpsc::SyncSender};

pub type Result<T> = std::result::Result<T, String>;
#[derive(Clone, Debug)]
pub struct Payload {
    pub meta: Metadata,
    pub data: Arc<[u8]>,
}
#[derive(Clone, Debug)]
pub struct ClipboardValue {
    pub stamp: ClipboardStamp,
    pub payload: Option<Payload>,
}

pub trait Clipboard: Send {
    fn read(&mut self) -> Result<ClipboardValue>;
    /// Compare generation immediately before writing, then read through the OS.
    fn write(
        &mut self,
        payload: &Payload,
        authorization: WriteAuthorization,
    ) -> Result<ClipboardValue>;
    fn description(&self) -> &str;
}
pub trait Store: Send {
    fn settings(&self) -> Result<Settings>;
    fn save_settings(&mut self, settings: &Settings) -> Result<()>;
    fn peers(&self) -> Result<Vec<PeerInfo>>;
    fn peer(&mut self, peer: &PeerInfo) -> Result<()>;
    /// Reservation persists replay high-water before accepting body bytes.
    fn reserve(&mut self, event: EventId) -> Result<bool>;
    /// Intent is durable before performing an OS write.
    fn intent(&mut self, event: EventId) -> Result<()>;
    fn receipt(&self, event: EventId) -> Result<Option<DeliveryState>>;
    fn record(
        &mut self,
        event: EventId,
        peer: &str,
        direction: &str,
        state: DeliveryState,
        payload: &Payload,
        detail: &str,
    ) -> Result<i64>;
    fn update(
        &mut self,
        event: EventId,
        peer: &str,
        state: DeliveryState,
        detail: &str,
    ) -> Result<()>;
    /// Minimal durable outgoing recovery records; independent of session history.
    fn pending_receipts(&self) -> Result<Vec<(EventId, String)>>;
    fn history(&self, offset: usize, limit: usize) -> Result<Vec<HistoryEntry>>;
    fn content(&self, id: i64) -> Result<Payload>;
    /// Only this device's retained local copies, independent of outgoing sends.
    fn local_history(&self, offset: usize, limit: usize) -> Result<(u64, Vec<HistorySummary>)>;
    fn local_content(&self, event: EventId) -> Result<Payload>;
    fn clear(&mut self) -> Result<()>;
    fn prune(&mut self) -> Result<()>;
}
pub enum NetworkEvent {
    PeerHintsPermission {
        peer: DeviceId,
        reply: SyncSender<Result<u64>>,
    },
    HistoryRevisionQuery {
        peer: DeviceId,
        reply: SyncSender<Result<(u64, u64)>>,
    },
    HistoryListQuery {
        peer: DeviceId,
        cursor: Option<HistoryCursor>,
        limit: u16,
        reply: SyncSender<Result<(u64, HistoryListResponse)>>,
    },
    HistoryGetQuery {
        peer: DeviceId,
        event: EventId,
        reply: SyncSender<Result<(u64, Payload)>>,
    },
    ReceiptQuery {
        peer: DeviceId,
        event: EventId,
        reply: SyncSender<Result<Option<DeliveryState>>>,
    },
    Peer(PeerInfo),
    Offer {
        peer: DeviceId,
        event: EventId,
        meta: Metadata,
        reply: SyncSender<Result<Reception>>,
    },
    Received {
        ticket: Reception,
        payload: Payload,
        reply: SyncSender<Result<()>>,
    },
    Delivery {
        event: EventId,
        peer: String,
        state: DeliveryState,
        detail: String,
    },
}
pub trait Network: Send {
    fn poll(&mut self) -> Option<NetworkEvent>;
    fn send(&mut self, publication: Publication, payload: Payload) -> Result<()>;
    fn policy_revision(&mut self, revision: u64);
    fn refresh(&mut self);
    fn reconcile(&mut self, peer: DeviceId, event: EventId) -> Result<()>;
}
pub trait Platform: Send {
    fn notify(&mut self, title: &str, body: &str);
    fn autostart(&mut self, enabled: Option<bool>) -> Result<AutostartStatus>;
}
