//! Session-only mobile history. Network adapters supply authenticated pages;
//! this crate never reads the OS clipboard or persists fetched content.
pub mod peers;
use sha2::{Digest, Sha256};
pub use shuttli_identity::Identity;
use shuttli_model::{
    mobile::{HistoryListResponse, HistorySummary, MAX_HISTORY_PAGE},
    sync::{DeviceId, EventId},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

pub const DEFAULT_HISTORY_LIMIT: usize = 20;
pub const MAX_SESSION_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct TimelineItem {
    pub source: DeviceId,
    pub summary: HistorySummary,
    pub body: Option<Arc<[u8]>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceFreshness {
    pub checked_at_ms: u64,
    pub revision: u64,
    pub partial: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryError {
    Background,
    InvalidPage,
    Missing,
    InvalidBody,
}

pub struct MobileHistory {
    active: bool,
    generation: u64,
    limit: usize,
    rows: BTreeMap<EventId, TimelineItem>,
    freshness: BTreeMap<DeviceId, SourceFreshness>,
}

impl Default for MobileHistory {
    fn default() -> Self {
        Self::new(DEFAULT_HISTORY_LIMIT)
    }
}

impl MobileHistory {
    pub fn new(limit: usize) -> Self {
        Self {
            active: false,
            generation: 0,
            limit: limit.min(10_000),
            rows: BTreeMap::new(),
            freshness: BTreeMap::new(),
        }
    }
    pub fn enter_foreground(&mut self) -> u64 {
        self.active = true;
        self.generation = self.generation.wrapping_add(1);
        self.generation
    }
    pub fn enter_background(&mut self) {
        self.active = false;
        self.generation = self.generation.wrapping_add(1);
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn source(&self, id: DeviceId) -> Option<SourceFreshness> {
        self.freshness.get(&id).copied()
    }
    /// Authenticated transport must bind `source` to the TLS peer identity.
    /// Validate the full page before changing the visible timeline.
    pub fn merge_page(
        &mut self,
        generation: u64,
        source: DeviceId,
        page: HistoryListResponse,
        checked_at_ms: u64,
    ) -> Result<(), HistoryError> {
        if !self.active || generation != self.generation {
            return Err(HistoryError::Background);
        }
        if page.items.len() > MAX_HISTORY_PAGE as usize
            || page.revision == 0
            || page.next.is_some_and(|next| {
                next.source_epoch != page.source_epoch || next.revision != page.revision
            })
        {
            return Err(HistoryError::InvalidPage);
        }
        let mut seen = BTreeSet::new();
        for item in &page.items {
            if item.event.origin != source
                || item.event.epoch != page.source_epoch
                || item.event.seq == 0
                || item.metadata.size > 8 * 1024 * 1024
                || !seen.insert(item.event)
                || self.rows.get(&item.event).is_some_and(|existing| {
                    existing.source != source
                        || existing.summary.metadata.format != item.metadata.format
                        || existing.summary.metadata.size != item.metadata.size
                        || (existing.summary.metadata.digest != item.metadata.digest
                            && existing.summary.metadata.digest != [0; 32]
                            && item.metadata.digest != [0; 32])
                })
            {
                return Err(HistoryError::InvalidPage);
            }
        }
        let partial = page.next.is_some();
        for summary in page.items {
            let id = summary.event;
            self.rows
                .entry(id)
                .and_modify(|row| {
                    if !summary.body_available {
                        row.body = None;
                    }
                    row.summary = summary.clone();
                })
                .or_insert_with(|| TimelineItem {
                    source,
                    summary,
                    body: None,
                });
        }
        self.freshness.insert(
            source,
            SourceFreshness {
                checked_at_ms,
                revision: page.revision,
                partial,
            },
        );
        self.trim();
        Ok(())
    }
    /// HISTORY_GET caches an exact body; it never writes the OS clipboard.
    pub fn cache_body(
        &mut self,
        generation: u64,
        event: EventId,
        bytes: Vec<u8>,
    ) -> Result<(), HistoryError> {
        if !self.active || generation != self.generation {
            return Err(HistoryError::Background);
        }
        let row = self.rows.get_mut(&event).ok_or(HistoryError::Missing)?;
        if !row.summary.body_available
            || bytes.len() as u64 != row.summary.metadata.size
            || Sha256::digest(&bytes).as_slice() != row.summary.metadata.digest
        {
            return Err(HistoryError::InvalidBody);
        }
        row.body = Some(bytes.into());
        self.trim();
        Ok(())
    }
    pub fn body_for_explicit_copy(&self, event: EventId) -> Option<Arc<[u8]>> {
        self.rows.get(&event)?.body.clone()
    }
    pub fn timeline(&self) -> Vec<TimelineItem> {
        let mut rows: Vec<_> = self.rows.values().cloned().collect();
        rows.sort_by(|a, b| {
            b.summary
                .copied_at_ms
                .cmp(&a.summary.copied_at_ms)
                .then_with(|| b.summary.event.cmp(&a.summary.event))
        });
        rows
    }
    pub fn clear(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.rows.clear();
        self.freshness.clear();
    }
    fn trim(&mut self) {
        let rows = self.timeline();
        for row in rows.iter().skip(self.limit) {
            self.rows.remove(&row.summary.event);
        }
        let mut bytes = 0usize;
        for row in rows.iter().take(self.limit) {
            if let Some(item) = self.rows.get_mut(&row.summary.event) {
                if let Some(body) = &item.body {
                    if bytes.saturating_add(body.len()) > MAX_SESSION_BYTES {
                        item.body = None;
                    } else {
                        bytes += body.len();
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shuttli_model::sync::{Format, Metadata};

    fn page(source: DeviceId, epoch: [u8; 16], seq: u64, text: &[u8]) -> HistoryListResponse {
        HistoryListResponse {
            source_epoch: epoch,
            revision: 1,
            items: vec![HistorySummary {
                event: EventId {
                    origin: source,
                    epoch,
                    seq,
                },
                metadata: Metadata {
                    format: Format::Text,
                    size: text.len() as u64,
                    digest: Sha256::digest(text).into(),
                },
                copied_at_ms: 100,
                body_available: true,
            }],
            next: None,
        }
    }

    #[test]
    fn merge_is_event_based_and_never_copies_to_os() {
        let mut history = MobileHistory::default();
        let generation = history.enter_foreground();
        for (source, seq) in [([1; 32], 1), ([2; 32], 1), ([1; 32], 2)] {
            history
                .merge_page(generation, source, page(source, [3; 16], seq, b"same"), 200)
                .unwrap();
        }
        assert_eq!(history.timeline().len(), 3);
        history
            .merge_page(generation, [1; 32], page([1; 32], [3; 16], 1, b"same"), 300)
            .unwrap();
        assert_eq!(history.timeline().len(), 3);
        let first = EventId {
            origin: [1; 32],
            epoch: [3; 16],
            seq: 1,
        };
        assert!(history.body_for_explicit_copy(first).is_none());
        history
            .cache_body(generation, first, b"same".to_vec())
            .unwrap();
        assert_eq!(&*history.body_for_explicit_copy(first).unwrap(), b"same");
        assert_eq!(history.source([1; 32]).unwrap().checked_at_ms, 300);
    }

    #[test]
    fn forged_or_changed_pages_and_bodies_do_not_mutate_history() {
        let mut history = MobileHistory::default();
        let generation = history.enter_foreground();
        let source = [1; 32];
        history
            .merge_page(generation, source, page(source, [3; 16], 1, b"one"), 1)
            .unwrap();
        let event = history.timeline()[0].summary.event;
        assert_eq!(
            history.cache_body(generation, event, b"two".to_vec()),
            Err(HistoryError::InvalidBody)
        );
        assert!(history.body_for_explicit_copy(event).is_none());
        assert_eq!(
            history.merge_page(generation, source, page([2; 32], [3; 16], 2, b"bad"), 2),
            Err(HistoryError::InvalidPage)
        );
        assert_eq!(
            history.merge_page(generation, source, page(source, [3; 16], 1, b"changed"), 2),
            Err(HistoryError::InvalidPage)
        );
        assert_eq!(history.timeline().len(), 1);
        history.enter_background();
        assert_eq!(
            history.cache_body(generation, event, b"one".to_vec()),
            Err(HistoryError::Background)
        );
        assert_eq!(
            history.merge_page(generation, source, page(source, [3; 16], 2, b"two"), 3),
            Err(HistoryError::Background)
        );
        history.clear();
        assert!(history.timeline().is_empty());
    }

    #[test]
    fn status_only_digest_can_upgrade_and_revocation_clears_cached_body() {
        let mut history = MobileHistory::default();
        let generation = history.enter_foreground();
        let source = [4; 32];
        let full = page(source, [5; 16], 1, b"secret");
        let event = full.items[0].event;
        let mut status = full.clone();
        status.items[0].body_available = false;
        status.items[0].metadata.digest = [0; 32];
        history
            .merge_page(generation, source, status.clone(), 1)
            .unwrap();
        assert!(history.body_for_explicit_copy(event).is_none());
        history.merge_page(generation, source, full, 2).unwrap();
        history
            .cache_body(generation, event, b"secret".to_vec())
            .unwrap();
        assert!(history.body_for_explicit_copy(event).is_some());
        history.merge_page(generation, source, status, 3).unwrap();
        assert!(history.body_for_explicit_copy(event).is_none());
    }

    #[test]
    fn bounded_timeline_marks_partial_sources() {
        let mut history = MobileHistory::new(1);
        let generation = history.enter_foreground();
        let source = [1; 32];
        history
            .merge_page(generation, source, page(source, [3; 16], 1, b"old"), 1)
            .unwrap();
        let mut next = page(source, [3; 16], 2, b"new");
        next.items[0].copied_at_ms = 200;
        next.next = Some(shuttli_model::mobile::HistoryCursor {
            source_epoch: [3; 16],
            revision: 1,
            offset: 2,
        });
        history.merge_page(generation, source, next, 2).unwrap();
        assert_eq!(history.timeline().len(), 1);
        assert_eq!(history.timeline()[0].summary.event.seq, 2);
        assert!(history.source(source).unwrap().partial);
    }
}
