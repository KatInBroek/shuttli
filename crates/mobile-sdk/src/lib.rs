//! Session-only mobile history. Network adapters supply authenticated pages;
//! this crate never accesses the OS clipboard. Text stays in memory; images
//! may use encrypted temporary objects with a process-local key.
mod history_export;
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
    /// A live receipt is independent of whether the source still retains an
    /// exportable history body. A later list must not recall accepted content.
    pub live_received: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryActivity {
    Waiting,
    Updating,
    Receiving,
    Updated,
    Denied,
    Failed,
    Paused,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceFreshness {
    pub checked_at_ms: u64,
    pub revision: u64,
    pub partial: bool,
    pub activity: HistoryActivity,
    pub receiving: Option<EventId>,
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
        for source in self.freshness.values_mut() {
            source.activity = HistoryActivity::Paused;
            source.receiving = None;
        }
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
        if self.mode != mode {
            self.generation = self.generation.wrapping_add(1);
        }
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
        if self.limit != limit {
            self.generation = self.generation.wrapping_add(1);
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
    pub fn source_activity(
        &mut self,
        generation: u64,
        source: DeviceId,
        activity: HistoryActivity,
        receiving: Option<EventId>,
    ) {
        if !self.active
            || generation != self.generation
            || (self.freshness.len() >= 32 && !self.freshness.contains_key(&source))
        {
            return;
        }
        let entry = self.freshness.entry(source).or_insert(SourceFreshness {
            checked_at_ms: 0,
            revision: 0,
            partial: false,
            activity: HistoryActivity::Waiting,
            receiving: None,
        });
        entry.activity = activity;
        entry.receiving = receiving;
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
                    if !summary.body_available && !row.live_received {
                        row.body = None;
                    }
                    let mut updated = summary.clone();
                    if row.live_received && row.body.is_some() && updated.metadata.digest == [0; 32]
                    {
                        updated.metadata.digest = row.summary.metadata.digest;
                    }
                    row.summary = updated;
                })
                .or_insert_with(|| TimelineItem {
                    source,
                    summary,
                    body: None,
                    live_received: false,
                });
        }
        self.freshness.insert(
            source,
            SourceFreshness {
                checked_at_ms,
                revision: page.revision,
                partial,
                activity: HistoryActivity::Updating,
                receiving: None,
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
            match metadata.format {
                shuttli_model::sync::Format::Text => Some(CachedBody::Memory(body)),
                shuttli_model::sync::Format::Png => match &self.image_cache {
                    Some(cache) => cache.put(&body).ok(),
                    None => Some(CachedBody::Memory(body)),
                },
            }
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
                    body_available: cached.is_some(),
                },
                body: cached,
                live_received: false,
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
    /// The native reception target is this bounded app cache. This does not
    /// write an OS clipboard, publish a new event, or manufacture a list revision.
    pub fn receive_live(
        &mut self,
        generation: u64,
        source: DeviceId,
        event: EventId,
        metadata: shuttli_model::sync::Metadata,
        bytes: Vec<u8>,
        received_at_ms: u64,
    ) -> Result<(), HistoryError> {
        if !self.active || generation != self.generation {
            return Err(HistoryError::Background);
        }
        if !self.wants_body() {
            return Err(HistoryError::Disabled);
        }
        if event.origin != source
            || event.seq == 0
            || !shuttli_transport::valid_metadata(&metadata)
            || metadata.size != bytes.len() as u64
            || shuttli_content::canonical_digest(metadata.format, &bytes).ok()
                != Some(metadata.digest)
            || self.rows.get(&event).is_some_and(|row| {
                row.source != source
                    || row.summary.metadata.format != metadata.format
                    || row.summary.metadata.size != metadata.size
                    || (row.summary.metadata.digest != [0; 32]
                        && row.summary.metadata.digest != metadata.digest)
            })
        {
            return Err(HistoryError::InvalidBody);
        }
        let body = match metadata.format {
            shuttli_model::sync::Format::Text => CachedBody::Memory(bytes.into()),
            shuttli_model::sync::Format::Png => match &self.image_cache {
                Some(cache) => cache.put(&bytes).map_err(|_| HistoryError::InvalidBody)?,
                None => CachedBody::Memory(bytes.into()),
            },
        };
        let copied_at_ms = self
            .rows
            .get(&event)
            .map_or(received_at_ms, |row| row.summary.copied_at_ms);
        self.rows.insert(
            event,
            TimelineItem {
                source,
                summary: HistorySummary {
                    event,
                    metadata,
                    copied_at_ms,
                    body_available: true,
                },
                body: Some(body),
                live_received: true,
            },
        );
        self.trim();
        if !self.body_available(event) {
            return Err(HistoryError::Missing);
        }
        Ok(())
    }
    pub fn body_available(&self, event: EventId) -> bool {
        self.rows.get(&event).is_some_and(|row| match &row.body {
            Some(CachedBody::Memory(_)) => true,
            Some(CachedBody::EncryptedImage(object)) => self
                .image_cache
                .as_ref()
                .is_some_and(|cache| cache.contains(object)),
            None => false,
        })
    }
    pub(crate) fn live_receipt_available(&self, event: EventId) -> bool {
        self.rows.get(&event).is_some_and(|row| row.live_received) && self.body_available(event)
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
#[path = "tests/lib.rs"]
mod tests;
