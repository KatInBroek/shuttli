//! Session-only mobile history. Network adapters supply authenticated pages;
//! this crate never reads the OS clipboard or persists fetched content.
pub mod image_cache;
pub mod peers;
pub mod transport;
use image_cache::{CachedBody, ImageCache};
pub use shuttli_identity::Identity;
use shuttli_model::{
    mobile::{HistoryListResponse, HistorySummary, MAX_HISTORY_PAGE},
    sync::{DeviceId, EventId},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::Arc,
};

pub const DEFAULT_HISTORY_LIMIT: usize = 20;
pub const MAX_SESSION_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct TimelineItem {
    pub source: DeviceId,
    pub summary: HistorySummary,
    pub body: Option<CachedBody>,
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
    Disabled,
    InvalidPage,
    Missing,
    InvalidBody,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryMode {
    Off,
    Status,
    Content,
}

pub struct MobileHistory {
    active: bool,
    generation: u64,
    limit: usize,
    mode: HistoryMode,
    rows: BTreeMap<EventId, TimelineItem>,
    freshness: BTreeMap<DeviceId, SourceFreshness>,
    image_cache: Option<ImageCache>,
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
            mode: HistoryMode::Content,
            rows: BTreeMap::new(),
            freshness: BTreeMap::new(),
            image_cache: None,
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
    pub fn is_active(&self) -> bool {
        self.active
    }
    pub fn mode(&self) -> HistoryMode {
        self.mode
    }
    pub fn limit(&self) -> usize {
        self.limit
    }
    pub fn query_enabled(&self) -> bool {
        self.active && self.limit > 0 && self.mode != HistoryMode::Off
    }
    pub fn wants_body(&self) -> bool {
        self.query_enabled() && self.mode == HistoryMode::Content
    }
    pub fn set_mode(&mut self, mode: HistoryMode) {
        self.mode = mode;
        if mode == HistoryMode::Off {
            self.rows.clear();
            self.freshness.clear();
        } else if mode == HistoryMode::Status {
            for row in self.rows.values_mut() {
                row.body = None;
            }
        }
    }
    pub fn set_limit(&mut self, limit: usize) -> bool {
        if limit > 10_000 {
            return false;
        }
        self.limit = limit;
        self.trim();
        true
    }
    pub fn configure_image_cache(&mut self, base: &Path) -> Result<(), String> {
        if self.image_cache.is_none() {
            self.image_cache = Some(ImageCache::new(base)?);
        }
        Ok(())
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
        if !self.query_enabled() {
            return Err(HistoryError::Disabled);
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
        if !self.wants_body() {
            return Err(HistoryError::Disabled);
        }
        let row = self.rows.get_mut(&event).ok_or(HistoryError::Missing)?;
        if !row.summary.body_available
            || bytes.len() as u64 != row.summary.metadata.size
            || shuttli_content::canonical_digest(row.summary.metadata.format, &bytes).ok()
                != Some(row.summary.metadata.digest)
        {
            return Err(HistoryError::InvalidBody);
        }
        row.body = Some(match row.summary.metadata.format {
            shuttli_model::sync::Format::Text => CachedBody::Memory(bytes.into()),
            shuttli_model::sync::Format::Png => match &self.image_cache {
                Some(cache) => cache.put(&bytes).map_err(|_| HistoryError::InvalidBody)?,
                None => CachedBody::Memory(bytes.into()),
            },
        });
        self.trim();
        Ok(())
    }
    /// Called only after an explicit phone send. A new event is kept separate
    /// from identical earlier copies and remains available for manual resend.
    pub fn record_local_sent(
        &mut self,
        generation: u64,
        event: EventId,
        metadata: shuttli_model::sync::Metadata,
        body: Arc<[u8]>,
        copied_at_ms: u64,
    ) -> Result<(), HistoryError> {
        if !self.active || generation != self.generation {
            return Err(HistoryError::Background);
        }
        if self.mode == HistoryMode::Off || self.limit == 0 {
            return Ok(());
        }
        if event.seq == 0
            || body.is_empty()
            || body.len() > 8 * 1024 * 1024
            || metadata.size != body.len() as u64
            || shuttli_content::canonical_digest(metadata.format, &body).ok()
                != Some(metadata.digest)
            || self.rows.contains_key(&event)
        {
            return Err(HistoryError::InvalidBody);
        }
        let cached = if self.mode == HistoryMode::Content {
            Some(match metadata.format {
                shuttli_model::sync::Format::Text => CachedBody::Memory(body),
                shuttli_model::sync::Format::Png => match &self.image_cache {
                    Some(cache) => cache.put(&body).map_err(|_| HistoryError::InvalidBody)?,
                    None => CachedBody::Memory(body),
                },
            })
        } else {
            None
        };
        self.rows.insert(
            event,
            TimelineItem {
                source: event.origin,
                summary: HistorySummary {
                    event,
                    metadata,
                    copied_at_ms,
                    body_available: true,
                },
                body: cached,
            },
        );
        self.trim();
        Ok(())
    }
    pub fn body_for_explicit_copy(&self, event: EventId) -> Option<Arc<[u8]>> {
        let row = self.rows.get(&event)?;
        match row.body.as_ref()? {
            CachedBody::Memory(bytes) => Some(bytes.clone()),
            CachedBody::EncryptedImage(object) => {
                let bytes = self.image_cache.as_ref()?.get(object)?;
                (shuttli_content::canonical_digest(row.summary.metadata.format, &bytes).ok()?
                    == row.summary.metadata.digest)
                    .then_some(bytes)
            }
        }
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
                    if bytes.saturating_add(body.size()) > MAX_SESSION_BYTES {
                        item.body = None;
                    } else {
                        bytes += body.size();
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
                    digest: shuttli_content::canonical_digest(Format::Text, text).unwrap(),
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
    fn local_manual_sends_keep_separate_events_and_can_be_resubmitted() {
        let mut history = MobileHistory::default();
        let generation = history.enter_foreground();
        let body: Arc<[u8]> = Arc::from(&b"again"[..]);
        let metadata = Metadata {
            format: Format::Text,
            size: body.len() as u64,
            digest: shuttli_content::canonical_digest(Format::Text, &body).unwrap(),
        };
        for seq in [1, 2] {
            history
                .record_local_sent(
                    generation,
                    EventId {
                        origin: [1; 32],
                        epoch: [2; 16],
                        seq,
                    },
                    metadata.clone(),
                    body.clone(),
                    seq,
                )
                .unwrap();
        }
        assert_eq!(history.timeline().len(), 2);
        history.enter_background();
        assert!(
            history
                .record_local_sent(
                    generation,
                    EventId {
                        origin: [1; 32],
                        epoch: [2; 16],
                        seq: 3
                    },
                    metadata,
                    body,
                    3
                )
                .is_err()
        );
    }

    #[test]
    fn history_modes_and_limit_bound_queries_and_cached_bodies() {
        let mut history = MobileHistory::default();
        let generation = history.enter_foreground();
        let source = [3; 32];
        let page = page(source, [4; 16], 1, b"fixture");
        let event = page.items[0].event;
        history
            .merge_page(generation, source, page.clone(), 1)
            .unwrap();
        history
            .cache_body(generation, event, b"fixture".to_vec())
            .unwrap();
        history.set_mode(HistoryMode::Status);
        assert!(history.query_enabled());
        assert!(!history.wants_body());
        assert!(history.body_for_explicit_copy(event).is_none());
        assert_eq!(
            history.cache_body(generation, event, b"fixture".to_vec()),
            Err(HistoryError::Disabled)
        );
        history.set_mode(HistoryMode::Off);
        assert!(!history.query_enabled());
        assert!(history.timeline().is_empty());
        assert_eq!(
            history.merge_page(generation, source, page.clone(), 2),
            Err(HistoryError::Disabled)
        );
        history.set_mode(HistoryMode::Content);
        assert!(!history.set_limit(10_001));
        assert_eq!(history.limit(), 20);
        assert!(history.set_limit(0));
        assert!(!history.query_enabled());
        assert!(history.set_limit(1));
        history.merge_page(generation, source, page, 3).unwrap();
        assert_eq!(history.timeline().len(), 1);
    }

    #[test]
    fn png_history_uses_encrypted_temporary_objects_and_clear_removes_them() {
        let mut png = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png, 1, 1);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&[1, 2, 3, 255]).unwrap();
        }
        let mut suffix = [0; 8];
        getrandom::getrandom(&mut suffix).unwrap();
        let base = std::env::temp_dir().join(format!("shuttli-image-test-{}", hex::encode(suffix)));
        std::fs::create_dir(&base).unwrap();
        let mut history = MobileHistory::default();
        history.configure_image_cache(&base).unwrap();
        let generation = history.enter_foreground();
        let event = EventId {
            origin: [3; 32],
            epoch: [4; 16],
            seq: 1,
        };
        let metadata = Metadata {
            format: Format::Png,
            size: png.len() as u64,
            digest: shuttli_content::canonical_digest(Format::Png, &png).unwrap(),
        };
        history
            .record_local_sent(generation, event, metadata, Arc::from(png.clone()), 1)
            .unwrap();
        let cache_dir = base.join("shuttli-images-v1");
        let file = std::fs::read_dir(&cache_dir)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        assert_ne!(std::fs::read(&file).unwrap(), png);
        assert_eq!(
            &*history.body_for_explicit_copy(event).unwrap(),
            png.as_slice()
        );
        std::fs::write(&file, b"tampered ciphertext").unwrap();
        assert!(history.body_for_explicit_copy(event).is_none());
        history.clear();
        assert!(!file.exists());
        let orphan = cache_dir.join("img-orphan.bin");
        std::fs::write(&orphan, b"old encrypted cache").unwrap();
        let _next_session = ImageCache::new(&base).unwrap();
        assert!(!orphan.exists());
        std::fs::remove_dir_all(base).unwrap();
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
