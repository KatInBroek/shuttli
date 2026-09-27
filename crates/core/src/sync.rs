//! Provenance and permission authority. OS and networking cannot mint permits.
extern crate alloc;
use alloc::{collections::BTreeMap, string::String, vec::Vec};
use shuttli_model::sync::*;

#[derive(Debug)]
pub struct Publication {
    event: EventId,
    target: DeviceId,
    meta: Metadata,
    revision: u64,
}
impl Publication {
    pub fn event(&self) -> EventId {
        self.event
    }
    pub fn target(&self) -> DeviceId {
        self.target
    }
    pub fn metadata(&self) -> &Metadata {
        &self.meta
    }
    pub fn policy_revision(&self) -> u64 {
        self.revision
    }
}
#[derive(Debug)]
pub struct Reception {
    event: EventId,
    meta: Metadata,
    baseline: ClipboardStamp,
    revision: u64,
}
impl Reception {
    pub fn event(&self) -> EventId {
        self.event
    }
    pub fn metadata(&self) -> &Metadata {
        &self.meta
    }
    pub fn policy_revision(&self) -> u64 {
        self.revision
    }
    pub fn baseline(&self) -> ClipboardStamp {
        self.baseline
    }
}
/// Single-use authority for a clipboard write, never serializable into UI or wire.
#[derive(Debug)]
pub struct WriteAuthorization {
    baseline: ClipboardStamp,
    meta: Metadata,
}
impl WriteAuthorization {
    pub fn baseline(&self) -> ClipboardStamp {
        self.baseline
    }
    pub fn metadata(&self) -> &Metadata {
        &self.meta
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rejection {
    Disabled,
    Sensitive,
    Unavailable,
    Stale,
    Invalid,
    Exhausted,
}

/// A local observation is a history fact, not permission to send.
#[derive(Debug)]
pub struct Observation {
    pub local_event: Option<EventId>,
    pub publications: Vec<Publication>,
}

pub struct SyncCore {
    device: DeviceId,
    epoch: [u8; 16],
    seq: u64,
    revision: u64,
    settings: Settings,
    peers: BTreeMap<DeviceId, String>,
    baseline: Option<ClipboardStamp>,
}
impl SyncCore {
    pub fn new(device: DeviceId, epoch: [u8; 16], settings: Settings) -> Self {
        Self {
            device,
            epoch,
            seq: 0,
            revision: 1,
            settings,
            peers: BTreeMap::new(),
            baseline: None,
        }
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn settings(&self) -> &Settings {
        &self.settings
    }
    pub fn configure(&mut self, settings: Settings) -> Result<(), Rejection> {
        if !settings.validate() {
            return Err(Rejection::Invalid);
        }
        self.revision = self.revision.checked_add(1).ok_or(Rejection::Exhausted)?;
        self.settings = settings;
        Ok(())
    }
    pub fn register(&mut self, id: DeviceId, key: String) -> Result<(), Rejection> {
        if id == self.device || (self.peers.len() >= 256 && !self.peers.contains_key(&id)) {
            return Err(Rejection::Invalid);
        }
        self.peers.insert(id, key);
        Ok(())
    }
    fn policy(&self, id: DeviceId) -> PeerPolicy {
        self.peers
            .get(&id)
            .and_then(|k| self.settings.peers.get(k))
            .cloned()
            .unwrap_or_default()
    }
    fn allowed(&self, id: DeviceId, meta: &Metadata, send: bool) -> bool {
        let p = self.policy(id);
        (if send {
            self.settings.send && p.send
        } else {
            self.settings.receive && p.receive
        }) && meta.size <= p.max_bytes
            && meta.size <= 8 * 1024 * 1024
            && match meta.format {
                Format::Text => self.settings.text && p.text && meta.size <= 1024 * 1024,
                Format::Png => self.settings.png && p.png,
            }
    }
    pub fn baseline(&mut self, stamp: Option<ClipboardStamp>) {
        self.baseline = stamp;
    }
    /// Called for a stable external observation. Equal content is conservative:
    /// clipboard-manager reassertions must never relay received bytes.
    pub fn observe(
        &mut self,
        stamp: ClipboardStamp,
        meta: &Metadata,
    ) -> Result<Vec<Publication>, Rejection> {
        Ok(self.observe_local(stamp, meta)?.publications)
    }
    /// Capture a changed local value even when sending is disabled. Startup,
    /// read recovery and equal-content reassertions only establish a baseline.
    pub fn observe_local(
        &mut self,
        stamp: ClipboardStamp,
        meta: &Metadata,
    ) -> Result<Observation, Rejection> {
        let old = self.baseline.replace(stamp);
        if old.is_none() || old.is_some_and(|b| b.digest == stamp.digest) {
            return Ok(Observation {
                local_event: None,
                publications: Vec::new(),
            });
        }
        Self::validate_content(stamp, meta)?;
        let event = self.next_event()?;
        let publications = if self.settings.automatic && self.settings.send {
            self.publications(event, meta)
        } else {
            Vec::new()
        };
        Ok(Observation {
            local_event: Some(event),
            publications,
        })
    }
    /// Explicit local intent can send unchanged/previously received content.
    pub fn manual(
        &mut self,
        stamp: ClipboardStamp,
        meta: &Metadata,
    ) -> Result<Vec<Publication>, Rejection> {
        self.issue(stamp, meta)
    }
    fn issue(
        &mut self,
        stamp: ClipboardStamp,
        meta: &Metadata,
    ) -> Result<Vec<Publication>, Rejection> {
        Self::validate_content(stamp, meta)?;
        if !self.settings.send {
            return Err(Rejection::Disabled);
        }
        let event = self.next_event()?;
        Ok(self.publications(event, meta))
    }
    fn validate_content(stamp: ClipboardStamp, meta: &Metadata) -> Result<(), Rejection> {
        if stamp.sensitive {
            return Err(Rejection::Sensitive);
        }
        if stamp.digest != meta.digest {
            return Err(Rejection::Invalid);
        }
        Ok(())
    }
    fn next_event(&mut self) -> Result<EventId, Rejection> {
        self.seq = self.seq.checked_add(1).ok_or(Rejection::Exhausted)?;
        Ok(EventId {
            origin: self.device,
            epoch: self.epoch,
            seq: self.seq,
        })
    }
    fn publications(&self, event: EventId, meta: &Metadata) -> Vec<Publication> {
        self.peers
            .keys()
            .filter(|id| self.allowed(**id, meta, true))
            .map(|id| Publication {
                event,
                target: *id,
                meta: meta.clone(),
                revision: self.revision,
            })
            .collect()
    }
    pub fn receive(
        &self,
        peer: DeviceId,
        event: EventId,
        meta: Metadata,
    ) -> Result<Reception, Rejection> {
        if peer != event.origin
            || peer == self.device
            || event.seq == 0
            || !self.peers.contains_key(&peer)
        {
            return Err(Rejection::Invalid);
        }
        if !self.allowed(peer, &meta, false) {
            return Err(Rejection::Disabled);
        }
        let baseline = self.baseline.ok_or(Rejection::Unavailable)?;
        Ok(Reception {
            event,
            meta,
            baseline,
            revision: self.revision,
        })
    }
    pub fn may_apply(&self, ticket: &Reception, current: ClipboardStamp) -> Result<(), Rejection> {
        if ticket.revision != self.revision
            || !self.allowed(ticket.event.origin, &ticket.meta, false)
        {
            return Err(Rejection::Disabled);
        }
        if self.baseline != Some(ticket.baseline) || current != ticket.baseline {
            return Err(Rejection::Stale);
        }
        Ok(())
    }
    pub fn authorize_apply(
        &self,
        ticket: &Reception,
        current: ClipboardStamp,
    ) -> Result<WriteAuthorization, Rejection> {
        self.may_apply(ticket, current)?;
        Ok(WriteAuthorization {
            baseline: current,
            meta: ticket.meta.clone(),
        })
    }
    pub fn authorize_local_copy(
        &self,
        current: ClipboardStamp,
        meta: Metadata,
    ) -> WriteAuthorization {
        WriteAuthorization {
            baseline: current,
            meta,
        }
    }
    /// Remote and local-only writes establish the baseline through one path.
    /// They have no return value capable of authorizing publication.
    pub fn written(&mut self, stamp: ClipboardStamp) {
        self.baseline = Some(stamp);
    }
    pub fn may_send(&self, p: &Publication) -> bool {
        p.revision == self.revision && self.allowed(p.target, &p.meta, true)
    }
}

#[cfg(test)]
#[path = "tests/sync.rs"]
mod tests;
