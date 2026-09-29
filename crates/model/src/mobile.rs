//! Wire v2 mobile values. All data here is untrusted until a peer is
//! authenticated and current core policy authorizes the operation.
extern crate alloc;
use alloc::{string::String, vec::Vec};
use serde::{Deserialize, Serialize};

use crate::sync::{DeviceId, EventId, Metadata};

pub const MAX_PEER_HINTS: usize = 64;
pub const MAX_HISTORY_PAGE: u16 = 64;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capabilities {
    pub peer_hints: bool,
    pub history_pull: bool,
    pub history_change: bool,
    pub accept_live_offer: bool,
}

impl Capabilities {
    pub const fn pull_only() -> Self {
        Self {
            peer_hints: true,
            history_pull: true,
            history_change: true,
            accept_live_offer: false,
        }
    }
    pub const fn desktop() -> Self {
        Self {
            accept_live_offer: true,
            ..Self::pull_only()
        }
    }
}

/// A route suggestion, never a trust or permission grant. The receiver adds
/// its own source and local expiry and verifies the hinted peer directly.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PeerHint {
    pub id: DeviceId,
    pub endpoint: String,
    pub capabilities: Capabilities,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PeerList {
    pub revision: u64,
    pub peers: Vec<PeerHint>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryCursor {
    pub source_epoch: [u8; 16],
    pub revision: u64,
    pub offset: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryListRequest {
    pub cursor: Option<HistoryCursor>,
    pub limit: u16,
}

impl HistoryListRequest {
    pub fn valid(&self) -> bool {
        (1..=MAX_HISTORY_PAGE).contains(&self.limit)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistorySummary {
    pub event: EventId,
    pub metadata: Metadata,
    pub copied_at_ms: u64,
    pub body_available: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryListResponse {
    pub source_epoch: [u8; 16],
    pub revision: u64,
    pub items: Vec<HistorySummary>,
    pub next: Option<HistoryCursor>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pull_only_cannot_accept_unsolicited_clipboard_offers() {
        assert!(!Capabilities::pull_only().accept_live_offer);
        assert!(Capabilities::desktop().accept_live_offer);
    }

    #[test]
    fn history_page_size_is_bounded() {
        for (limit, valid) in [(0, false), (1, true), (64, true), (65, false)] {
            assert_eq!(
                HistoryListRequest {
                    cursor: None,
                    limit
                }
                .valid(),
                valid
            );
        }
    }
}
