//! Peer hints never confer identity or permission. The transport must bind
//! direct observations and roster sources to mutually authenticated TLS peers.
use shuttli_model::{
    mobile::{Capabilities, PeerHint, PeerList},
    sync::{DeviceId, Format},
};
use shuttli_protocol::{valid_endpoint, valid_peer_list};
use std::collections::BTreeMap;

pub const HINT_TTL_MS: u64 = 90_000;
pub const MAX_DIRECT_PEERS: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Directions {
    pub send: bool,
    pub receive: bool,
    pub text: bool,
    pub image: bool,
}

impl Default for Directions {
    fn default() -> Self {
        Self {
            send: false,
            receive: true,
            text: true,
            image: true,
        }
    }
}

impl Directions {
    pub fn allows(&self, format: Format) -> bool {
        match format {
            Format::Text => self.text,
            Format::Png => self.image,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectPeer {
    pub id: DeviceId,
    pub name: String,
    pub endpoint: String,
    pub capabilities: Capabilities,
    pub online: bool,
    pub directions: Directions,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub hint: PeerHint,
    pub source: DeviceId,
}

#[derive(Clone, Debug)]
struct SourceHints {
    revision: u64,
    expires_at_ms: u64,
    hints: Vec<PeerHint>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectoryError {
    UnknownSource,
    InvalidList,
    StaleRevision,
    Capacity,
}

pub struct PeerDirectory {
    own: DeviceId,
    direct: BTreeMap<DeviceId, DirectPeer>,
    hints: BTreeMap<DeviceId, SourceHints>,
    saved_directions: BTreeMap<DeviceId, Directions>,
}

impl PeerDirectory {
    pub fn new(own: DeviceId) -> Self {
        Self {
            own,
            direct: BTreeMap::new(),
            hints: BTreeMap::new(),
            saved_directions: BTreeMap::new(),
        }
    }
    pub fn own_id(&self) -> DeviceId {
        self.own
    }

    /// `id` is derived from the directly presented certificate, not a hint.
    pub fn observed_direct(
        &mut self,
        id: DeviceId,
        name: String,
        endpoint: String,
        capabilities: Capabilities,
    ) -> Result<(), DirectoryError> {
        if id == self.own || !valid_endpoint(&endpoint) {
            return Err(DirectoryError::InvalidList);
        }
        if self.direct.len() >= MAX_DIRECT_PEERS && !self.direct.contains_key(&id) {
            return Err(DirectoryError::Capacity);
        }
        if name.is_empty() || name.len() > 256 || name.chars().any(char::is_control) {
            return Err(DirectoryError::InvalidList);
        }
        let directions = self
            .direct
            .get(&id)
            .map(|p| p.directions)
            .or_else(|| self.saved_directions.get(&id).copied())
            .unwrap_or_default();
        self.direct.insert(
            id,
            DirectPeer {
                id,
                name,
                endpoint,
                capabilities,
                online: true,
                directions,
            },
        );
        Ok(())
    }

    pub fn disconnected(&mut self, id: DeviceId) {
        if let Some(peer) = self.direct.get_mut(&id) {
            peer.online = false;
        }
        self.hints.remove(&id);
    }

    pub fn directions(
        &mut self,
        id: DeviceId,
        directions: Directions,
    ) -> Result<(), DirectoryError> {
        let peer = self
            .direct
            .get_mut(&id)
            .ok_or(DirectoryError::UnknownSource)?;
        peer.directions = directions;
        self.saved_directions.insert(id, directions);
        Ok(())
    }

    /// Restore consent only for an exact public-key identity. Hints cannot
    /// consume this preference until the peer authenticates directly.
    pub fn restore_directions(&mut self, id: DeviceId, directions: Directions) -> bool {
        if id == self.own
            || self.saved_directions.len() >= MAX_DIRECT_PEERS
                && !self.saved_directions.contains_key(&id)
        {
            return false;
        }
        self.saved_directions.insert(id, directions);
        if let Some(peer) = self.direct.get_mut(&id) {
            peer.directions = directions;
        }
        true
    }

    pub fn direct(&self) -> Vec<DirectPeer> {
        self.direct.values().cloned().collect()
    }

    /// Replaces one authenticated source's snapshot. Other sources are kept.
    pub fn accept_hints(
        &mut self,
        source: DeviceId,
        list: PeerList,
        now_ms: u64,
    ) -> Result<(), DirectoryError> {
        if !self.direct.get(&source).is_some_and(|p| p.online) {
            return Err(DirectoryError::UnknownSource);
        }
        if !valid_peer_list(&list, source, self.own) {
            return Err(DirectoryError::InvalidList);
        }
        if self
            .hints
            .get(&source)
            .is_some_and(|old| list.revision <= old.revision)
        {
            return Err(DirectoryError::StaleRevision);
        }
        self.hints.insert(
            source,
            SourceHints {
                revision: list.revision,
                expires_at_ms: now_ms.saturating_add(HINT_TTL_MS),
                hints: list.peers,
            },
        );
        Ok(())
    }

    pub fn expire(&mut self, now_ms: u64) {
        self.hints.retain(|_, value| value.expires_at_ms > now_ms);
    }

    /// Suggestions for a direct TLS probe; consumers must verify the expected
    /// public-key identity before adding the device to `direct`.
    pub fn candidates(&self) -> Vec<Candidate> {
        self.hints
            .iter()
            .flat_map(|(source, roster)| {
                roster
                    .hints
                    .iter()
                    .filter(|hint| !self.direct.get(&hint.id).is_some_and(|p| p.online))
                    .map(|hint| Candidate {
                        hint: hint.clone(),
                        source: *source,
                    })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hint(id: u8) -> PeerHint {
        PeerHint {
            id: [id; 32],
            endpoint: format!("100.100.100.{id}:45987"),
            capabilities: Capabilities::desktop(),
        }
    }

    #[test]
    fn overlapping_rosters_expire_per_source_without_granting_send() {
        let mut directory = PeerDirectory::new([1; 32]);
        for id in [2, 3] {
            let h = hint(id);
            directory
                .observed_direct(h.id, format!("source{id}"), h.endpoint, h.capabilities)
                .unwrap();
        }
        directory
            .accept_hints(
                [2; 32],
                PeerList {
                    revision: 1,
                    peers: vec![hint(4)],
                },
                1,
            )
            .unwrap();
        directory
            .accept_hints(
                [3; 32],
                PeerList {
                    revision: 1,
                    peers: vec![hint(4), hint(5)],
                },
                20,
            )
            .unwrap();
        assert_eq!(directory.candidates().len(), 3);
        directory.expire(1 + HINT_TTL_MS);
        assert_eq!(directory.candidates().len(), 2);
        let target = hint(4);
        directory
            .observed_direct(
                target.id,
                "target".into(),
                target.endpoint,
                target.capabilities,
            )
            .unwrap();
        assert_eq!(directory.candidates().len(), 1);
        assert_eq!(
            directory
                .direct()
                .iter()
                .find(|p| p.id == [4; 32])
                .unwrap()
                .directions,
            Directions::default()
        );
    }

    #[test]
    fn forged_and_stale_rosters_cannot_change_trust_or_permissions() {
        let mut directory = PeerDirectory::new([1; 32]);
        let h = hint(2);
        assert_eq!(
            directory.accept_hints(
                h.id,
                PeerList {
                    revision: 1,
                    peers: vec![]
                },
                0
            ),
            Err(DirectoryError::UnknownSource)
        );
        directory
            .observed_direct(h.id, "source".into(), h.endpoint, h.capabilities)
            .unwrap();
        directory
            .directions(
                h.id,
                Directions {
                    send: true,
                    receive: false,
                    ..Directions::default()
                },
            )
            .unwrap();
        assert_eq!(
            directory.accept_hints(
                h.id,
                PeerList {
                    revision: 1,
                    peers: vec![hint(1)]
                },
                0
            ),
            Err(DirectoryError::InvalidList)
        );
        directory
            .accept_hints(
                h.id,
                PeerList {
                    revision: 1,
                    peers: vec![hint(3)],
                },
                0,
            )
            .unwrap();
        assert_eq!(
            directory.accept_hints(
                h.id,
                PeerList {
                    revision: 1,
                    peers: vec![]
                },
                10
            ),
            Err(DirectoryError::StaleRevision)
        );
        directory.disconnected(h.id);
        assert!(directory.candidates().is_empty());
        assert_eq!(
            directory.direct()[0].directions,
            Directions {
                send: true,
                receive: false,
                ..Directions::default()
            }
        );
    }

    #[test]
    fn restored_consent_requires_exact_direct_identity() {
        let mut directory = PeerDirectory::new([1; 32]);
        let id = [2; 32];
        assert!(directory.restore_directions(
            id,
            Directions {
                send: true,
                receive: false,
                ..Directions::default()
            }
        ));
        assert!(directory.direct().is_empty());
        assert!(!directory.restore_directions(
            [1; 32],
            Directions {
                send: true,
                receive: true,
                ..Directions::default()
            }
        ));
        directory
            .observed_direct(
                id,
                "known".into(),
                "100.64.0.2:45987".into(),
                Capabilities::desktop(),
            )
            .unwrap();
        directory
            .observed_direct(
                [3; 32],
                "new".into(),
                "100.64.0.3:45987".into(),
                Capabilities::desktop(),
            )
            .unwrap();
        assert_eq!(
            directory
                .direct()
                .iter()
                .find(|peer| peer.id == id)
                .unwrap()
                .directions,
            Directions {
                send: true,
                receive: false,
                ..Directions::default()
            }
        );
        assert_eq!(
            directory
                .direct()
                .iter()
                .find(|peer| peer.id == [3; 32])
                .unwrap()
                .directions,
            Directions::default()
        );
    }
}
