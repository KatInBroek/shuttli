//! Strict version dispatch and bounded v2 control frames. TLS identity and
//! application policy checks belong to the caller, never to JSON fields.
use serde::{Deserialize, Serialize};
use shuttli_model::{
    mobile::{Capabilities, HistoryListRequest, HistoryListResponse, MAX_PEER_HINTS, PeerList},
    sync::{DeviceId, EventId, Metadata},
};
use std::{
    collections::BTreeSet,
    net::{IpAddr, SocketAddr},
};

pub const MAX_CONTROL_FRAME_BYTES: usize = 16_384;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Hello {
    V1 {
        name: String,
        epoch: [u8; 16],
    },
    V2 {
        name: String,
        epoch: [u8; 16],
        capabilities: Capabilities,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HelloV1 {
    #[serde(rename = "type")]
    kind: String,
    version: u16,
    name: String,
    epoch: [u8; 16],
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HelloV2 {
    #[serde(rename = "type")]
    kind: String,
    version: u16,
    name: String,
    epoch: [u8; 16],
    capabilities: Capabilities,
}

fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 256 && !name.chars().any(char::is_control)
}

pub fn decode_hello(bytes: &[u8]) -> Result<Hello, &'static str> {
    if bytes.is_empty() || bytes.len() > MAX_CONTROL_FRAME_BYTES {
        return Err("invalid hello size");
    }
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| "invalid hello")?;
    match value.get("version").and_then(serde_json::Value::as_u64) {
        Some(1) => {
            let hello: HelloV1 = serde_json::from_value(value).map_err(|_| "invalid v1 hello")?;
            if hello.kind != "hello" || !valid_name(&hello.name) {
                return Err("invalid v1 hello");
            }
            Ok(Hello::V1 {
                name: hello.name,
                epoch: hello.epoch,
            })
        }
        Some(2) => {
            let hello: HelloV2 = serde_json::from_value(value).map_err(|_| "invalid v2 hello")?;
            if hello.kind != "hello" || !valid_name(&hello.name) {
                return Err("invalid v2 hello");
            }
            Ok(Hello::V2 {
                name: hello.name,
                epoch: hello.epoch,
                capabilities: hello.capabilities,
            })
        }
        _ => Err("unsupported hello version"),
    }
}

pub fn encode_hello(hello: &Hello) -> Result<Vec<u8>, &'static str> {
    let bytes = match hello {
        Hello::V1 { name, epoch } if valid_name(name) => serde_json::to_vec(&HelloV1 {
            kind: "hello".into(),
            version: 1,
            name: name.clone(),
            epoch: *epoch,
        }),
        Hello::V2 {
            name,
            epoch,
            capabilities,
        } if valid_name(name) => serde_json::to_vec(&HelloV2 {
            kind: "hello".into(),
            version: 2,
            name: name.clone(),
            epoch: *epoch,
            capabilities: *capabilities,
        }),
        _ => return Err("invalid hello name"),
    }
    .map_err(|_| "invalid hello")?;
    if bytes.len() > MAX_CONTROL_FRAME_BYTES {
        return Err("invalid hello size");
    }
    Ok(bytes)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum FrameV2 {
    PeerList {
        revision: u64,
        peers: Vec<shuttli_model::mobile::PeerHint>,
    },
    HistoryListRequest {
        cursor: Option<shuttli_model::mobile::HistoryCursor>,
        limit: u16,
    },
    HistoryListResponse {
        source_epoch: [u8; 16],
        revision: u64,
        items: Vec<shuttli_model::mobile::HistorySummary>,
        next: Option<shuttli_model::mobile::HistoryCursor>,
    },
    HistoryGet {
        event: EventId,
    },
    HistoryBody {
        event: EventId,
        metadata: Metadata,
    },
    HistoryChanged {
        revision: u64,
    },
    Error {
        code: String,
    },
}

impl FrameV2 {
    pub fn decode(bytes: &[u8]) -> Result<Self, &'static str> {
        if bytes.is_empty() || bytes.len() > MAX_CONTROL_FRAME_BYTES {
            return Err("invalid frame size");
        }
        let frame: Self = serde_json::from_slice(bytes).map_err(|_| "invalid v2 frame")?;
        match &frame {
            Self::PeerList { revision, peers }
                if *revision == 0
                    || peers.len() > MAX_PEER_HINTS
                    || peers.iter().map(|p| p.id).collect::<BTreeSet<_>>().len() != peers.len()
                    || !peers.iter().all(|p| valid_endpoint(&p.endpoint)) =>
            {
                Err("invalid peer list")
            }
            Self::HistoryListRequest { cursor, limit }
                if !HistoryListRequest {
                    cursor: *cursor,
                    limit: *limit,
                }
                .valid() =>
            {
                Err("invalid history limit")
            }
            Self::HistoryListResponse {
                source_epoch,
                revision,
                items,
                next,
            } if *revision == 0
                || items.len() > shuttli_model::mobile::MAX_HISTORY_PAGE as usize
                || next.is_some_and(|c| {
                    c.source_epoch != *source_epoch || c.revision != *revision
                })
                || !valid_summaries(items, *source_epoch) =>
            {
                Err("invalid history list")
            }
            Self::HistoryChanged { revision } if *revision == 0 => Err("invalid revision"),
            Self::HistoryGet { event } if event.seq == 0 => Err("invalid event"),
            Self::HistoryBody { event, metadata }
                if event.seq == 0 || metadata.size > 8 * 1024 * 1024 =>
            {
                Err("invalid history body")
            }
            Self::Error { code } if code.len() > 64 || code.chars().any(char::is_control) => {
                Err("invalid error")
            }
            _ => Ok(frame),
        }
    }
    pub fn encode(&self) -> Result<Vec<u8>, &'static str> {
        let bytes = serde_json::to_vec(self).map_err(|_| "invalid v2 frame")?;
        Self::decode(&bytes)?;
        Ok(bytes)
    }
}

impl From<PeerList> for FrameV2 {
    fn from(value: PeerList) -> Self {
        Self::PeerList {
            revision: value.revision,
            peers: value.peers,
        }
    }
}

impl From<HistoryListResponse> for FrameV2 {
    fn from(value: HistoryListResponse) -> Self {
        Self::HistoryListResponse {
            source_epoch: value.source_epoch,
            revision: value.revision,
            items: value.items,
            next: value.next,
        }
    }
}

/// An endpoint supplied in a roster must be a suggestion for the known TLS
/// peer identity, never a source of identity or permission by itself.
pub fn valid_peer_list(list: &PeerList, sender: DeviceId, receiver: DeviceId) -> bool {
    if list.revision == 0 || list.peers.len() > MAX_PEER_HINTS {
        return false;
    }
    let mut seen = BTreeSet::new();
    list.peers.iter().all(|p| {
        p.id != sender && p.id != receiver && seen.insert(p.id) && valid_endpoint(&p.endpoint)
    })
}

fn valid_endpoint(endpoint: &str) -> bool {
    if endpoint.len() > 256 {
        return false;
    }
    let Ok(address) = endpoint.parse::<SocketAddr>() else {
        return false;
    };
    address.port() == 45987
        && matches!(address.ip(), IpAddr::V4(ip) if ip.octets()[0] == 100 && (64..=127).contains(&ip.octets()[1]))
}

fn valid_summaries(items: &[shuttli_model::mobile::HistorySummary], epoch: [u8; 16]) -> bool {
    let mut seen = BTreeSet::new();
    items.iter().all(|item| {
        item.event.epoch == epoch
            && item.event.seq != 0
            && item.metadata.size <= 8 * 1024 * 1024
            && seen.insert(item.event)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use shuttli_model::{mobile::PeerHint, sync::Format};

    #[test]
    fn hello_version_is_explicit_and_strict() {
        for hello in [
            Hello::V1 {
                name: "desktop".into(),
                epoch: [1; 16],
            },
            Hello::V2 {
                name: "phone".into(),
                epoch: [2; 16],
                capabilities: Capabilities::pull_only(),
            },
        ] {
            assert_eq!(decode_hello(&encode_hello(&hello).unwrap()).unwrap(), hello);
        }
        assert!(decode_hello(br#"{"type":"hello","version":3,"name":"x","epoch":[]}"#).is_err());
        assert!(decode_hello(br#"{"type":"hello","version":1,"name":"x","epoch":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],"capabilities":{}}"#).is_err());
        assert!(decode_hello(br#"{"type":"other","version":1,"name":"x","epoch":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]}"#).is_err());
        assert!(
            encode_hello(&Hello::V1 {
                name: "\n".into(),
                epoch: [0; 16]
            })
            .is_err()
        );
    }

    #[test]
    fn frames_reject_unbounded_or_malformed_history() {
        let frame = FrameV2::HistoryListRequest {
            cursor: None,
            limit: 20,
        };
        assert!(matches!(
            FrameV2::decode(&frame.encode().unwrap()),
            Ok(FrameV2::HistoryListRequest { limit: 20, .. })
        ));
        assert!(
            FrameV2::HistoryListRequest {
                cursor: None,
                limit: 21
            }
            .encode()
            .is_err()
        );
        assert!(FrameV2::decode(&vec![b' '; MAX_CONTROL_FRAME_BYTES + 1]).is_err());
        let item = shuttli_model::mobile::HistorySummary {
            event: EventId {
                origin: [1; 32],
                epoch: [2; 16],
                seq: 1,
            },
            metadata: Metadata {
                format: Format::Text,
                size: 1,
                digest: [3; 32],
            },
            copied_at_ms: 0,
            body_available: false,
        };
        assert!(
            FrameV2::HistoryListResponse {
                source_epoch: [2; 16],
                revision: 1,
                items: vec![item.clone()],
                next: None
            }
            .encode()
            .is_ok()
        );
        assert!(
            FrameV2::HistoryListResponse {
                source_epoch: [2; 16],
                revision: 1,
                items: vec![item.clone(), item],
                next: None
            }
            .encode()
            .is_err()
        );
        let full_page = FrameV2::HistoryListResponse {
            source_epoch: [255; 16],
            revision: u64::MAX,
            items: (1..=shuttli_model::mobile::MAX_HISTORY_PAGE)
                .map(|seq| shuttli_model::mobile::HistorySummary {
                    event: EventId {
                        origin: [255; 32],
                        epoch: [255; 16],
                        seq: seq.into(),
                    },
                    metadata: Metadata {
                        format: Format::Png,
                        size: 8 * 1024 * 1024,
                        digest: [255; 32],
                    },
                    copied_at_ms: u64::MAX,
                    body_available: true,
                })
                .collect(),
            next: None,
        };
        assert!(full_page.encode().is_ok());
    }

    #[test]
    fn roster_is_only_bounded_tailnet_route_hints() {
        let hint = PeerHint {
            id: [3; 32],
            endpoint: "100.100.100.3:45987".into(),
            capabilities: Capabilities::desktop(),
        };
        let list = PeerList {
            revision: 1,
            peers: vec![hint.clone()],
        };
        assert!(valid_peer_list(&list, [1; 32], [2; 32]));
        assert!(FrameV2::from(list.clone()).encode().is_ok());
        assert!(!valid_peer_list(
            &PeerList {
                revision: 1,
                peers: vec![hint.clone(), hint.clone()]
            },
            [1; 32],
            [2; 32]
        ));
        assert!(!valid_peer_list(&list, [3; 32], [2; 32]));
        for endpoint in [
            "192.168.1.2:45987",
            "100.100.100.3:22",
            "example.com:45987",
            "fd7a:115c::1",
        ] {
            assert!(!valid_peer_list(
                &PeerList {
                    revision: 1,
                    peers: vec![PeerHint {
                        endpoint: endpoint.into(),
                        ..hint.clone()
                    }]
                },
                [1; 32],
                [2; 32]
            ));
        }
    }
}
