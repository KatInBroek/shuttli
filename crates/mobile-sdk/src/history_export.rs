//! Export only this identity's current-session events. Imported history and
//! live receptions remain local cache entries, never a forwarding source.
use crate::MobileHistory;
use sha2::{Digest, Sha256};
use shuttli_model::{
    mobile::{HistoryCursor, HistoryListResponse, MAX_HISTORY_PAGE},
    sync::{DeviceId, Format},
};

impl MobileHistory {
    pub(crate) fn export_page(
        &self,
        generation: u64,
        own: DeviceId,
        epoch: [u8; 16],
        cursor: Option<HistoryCursor>,
        limit: u16,
        allowed: impl Fn(shuttli_model::sync::EventId, &shuttli_model::sync::Metadata, bool) -> bool,
    ) -> Result<HistoryListResponse, &'static str> {
        if generation != self.generation() || !self.query_enabled() {
            return Err("history_denied");
        }
        if !(1..=MAX_HISTORY_PAGE).contains(&limit) {
            return Err("history_invalid_request");
        }
        let mut items: Vec<_> = self
            .timeline()
            .into_iter()
            .filter(|row| row.summary.event.origin == own && row.summary.event.epoch == epoch)
            .filter(|row| allowed(row.summary.event, &row.summary.metadata, false))
            .map(|row| {
                let mut item = row.summary;
                item.body_available = self.wants_body()
                    && self.body_available(item.event)
                    && allowed(item.event, &item.metadata, true);
                if !item.body_available {
                    item.metadata.digest = [0; 32];
                }
                item
            })
            .collect();
        // Hash only the authorized visible snapshot. A status-only view must
        // not expose a cursor token derived from a hidden content digest.
        let mut hash = Sha256::new();
        hash.update(epoch);
        for item in &items {
            hash.update(item.event.seq.to_be_bytes());
            hash.update(item.copied_at_ms.to_be_bytes());
            hash.update([match item.metadata.format {
                Format::Text => 0,
                Format::Png => 1,
            }]);
            hash.update(item.metadata.size.to_be_bytes());
            hash.update(item.metadata.digest);
            hash.update([u8::from(item.body_available)]);
        }
        let revision =
            u64::from_be_bytes(hash.finalize()[..8].try_into().expect("digest prefix")).max(1);
        let offset = cursor.map_or(0, |c| c.offset as usize);
        if cursor.is_some_and(|c| c.source_epoch != epoch || c.revision != revision)
            || offset > items.len()
        {
            return Err("history_cursor_stale");
        }
        let end = offset.saturating_add(limit as usize).min(items.len());
        let next = (end < items.len()).then_some(HistoryCursor {
            source_epoch: epoch,
            revision,
            offset: end as u32,
        });
        items.truncate(end);
        items.drain(..offset);
        Ok(HistoryListResponse {
            source_epoch: epoch,
            revision,
            items,
            next,
        })
    }
}

#[cfg(test)]
#[path = "tests/history_export.rs"]
mod tests;
