//! Peer hints never confer identity or permission. The transport must bind
//! direct observations and roster sources to mutually authenticated TLS peers.
use shuttli_core::sync::{Publication, Reception, Rejection, SyncCore};
use shuttli_model::sync::{ClipboardStamp, EventId, Metadata, PeerPolicy, Settings};
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
    epoch: [u8; 16],
    core: SyncCore,
}

impl PeerDirectory {
    pub fn new(own: DeviceId) -> Self {
        let mut epoch = [0; 16];
        getrandom::getrandom(&mut epoch).expect("session entropy");
        Self {
            own,
            direct: BTreeMap::new(),
            hints: BTreeMap::new(),
            saved_directions: BTreeMap::new(),
            epoch,
            core: SyncCore::new(
                own,
                epoch,
                Settings {
                    automatic: false,
                    ..Settings::default()
                },
            ),
        }
    }
    pub fn epoch(&self) -> [u8; 16] {
        self.epoch
    }
    pub fn policy_revision(&self) -> u64 {
        self.core.revision()
    }

    fn configure_core(&mut self) {
        let settings = Settings {
            automatic: false,
            peers: self
                .direct
                .values()
                .map(|p| {
                    (
                        hex::encode(p.id),
                        PeerPolicy {
                            send: p.directions.send,
                            receive: p.directions.receive,
                            text: p.directions.text,
                            png: p.directions.image,
                            ..PeerPolicy::default()
                        },
                    )
                })
                .collect(),
            ..Settings::default()
        };
        if self.core.settings() != &settings {
            self.core
                .configure(settings)
                .expect("bounded peer settings");
        }
    }

    pub fn manual(&mut self, metadata: &Metadata) -> Result<Vec<Publication>, Rejection> {
        let mut permits = self.core.manual(
            ClipboardStamp {
                generation: 0,
                digest: metadata.digest,
                sensitive: false,
            },
            metadata,
        )?;
        permits.retain(|permit| {
            self.direct
                .get(&permit.target())
                .is_some_and(|peer| peer.online)
        });
        Ok(permits)
    }
    pub fn may_send(&self, permit: &Publication) -> bool {
        self.core.may_send(permit)
    }
    pub fn authorize_hints(&self, target: DeviceId) -> bool {
        self.core.authorize_peer_hints(target).is_ok()
    }
    pub fn authorize_history(
        &self,
        target: DeviceId,
        event: EventId,
        meta: &Metadata,
        body: bool,
    ) -> bool {
        event.epoch == self.epoch
            && self
                .core
                .authorize_history_export(target, event, meta, body)
                .is_ok()
    }
    pub fn receive_ticket(
        &mut self,
        source: DeviceId,
        event: EventId,
        metadata: Metadata,
        generation: u64,
    ) -> Result<Reception, Rejection> {
        self.core.baseline(Some(cache_stamp(generation)));
        self.core.receive(source, event, metadata)
    }
    pub fn may_receive(&self, permit: &Reception, generation: u64) -> bool {
        self.core.may_apply(permit, cache_stamp(generation)).is_ok()
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
        self.core
            .register(id, hex::encode(id))
            .map_err(|_| DirectoryError::Capacity)?;
        self.configure_core();
        Ok(())
    }

    pub(crate) fn reset_hints(&mut self, source: DeviceId) {
        self.hints.remove(&source);
    }

    pub fn disconnected(&mut self, id: DeviceId) {
        if let Some(peer) = self.direct.get_mut(&id) {
            peer.online = false;
        }
        self.hints.remove(&id);
        self.configure_core();
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
        self.configure_core();
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
        self.configure_core();
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
        let mut candidates: BTreeMap<DeviceId, Candidate> = self
            .hints
            .iter()
            .flat_map(|(source, roster)| {
                roster
                    .hints
                    .iter()
                    .filter(|hint| !self.direct.get(&hint.id).is_some_and(|p| p.online))
                    .map(|hint| {
                        (
                            hint.id,
                            Candidate {
                                hint: hint.clone(),
                                source: *source,
                            },
                        )
                    })
            })
            .collect();
        // Direct authentication survives hint expiry. Retry known endpoints
        // after backgrounding or a broken connection, without granting consent.
        for peer in self.direct.values().filter(|p| !p.online) {
            candidates.insert(
                peer.id,
                Candidate {
                    hint: PeerHint {
                        id: peer.id,
                        endpoint: peer.endpoint.clone(),
                        capabilities: peer.capabilities,
                    },
                    source: peer.id,
                },
            );
        }
        candidates.into_values().collect()
    }
}

fn cache_stamp(generation: u64) -> ClipboardStamp {
    ClipboardStamp {
        generation,
        digest: [0; 32],
        sensitive: false,
    }
}

#[cfg(test)]
#[path = "tests/peers.rs"]
mod tests;
