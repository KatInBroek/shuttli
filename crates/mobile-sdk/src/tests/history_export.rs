use crate::{HistoryMode, MobileHistory};
use shuttli_model::{
    mobile::HistoryCursor,
    sync::{EventId, Format, Metadata},
};
use std::sync::Arc;

fn record(cache: &mut MobileHistory, origin: [u8; 32], epoch: [u8; 16], seq: u64, data: &[u8]) {
    let meta = Metadata {
        format: Format::Text,
        size: data.len() as u64,
        digest: shuttli_content::canonical_digest(Format::Text, data).unwrap(),
    };
    cache
        .record_local_sent(
            cache.generation(),
            EventId { origin, epoch, seq },
            meta,
            Arc::from(data),
            seq,
        )
        .unwrap();
}

#[test]
fn export_is_owned_bounded_authorized_and_snapshot_bound() {
    let mut cache = MobileHistory::new(30);
    let generation = cache.enter_foreground();
    let own = [1; 32];
    let epoch = [2; 16];
    for seq in 1..=23 {
        record(&mut cache, own, epoch, seq, b"local copy");
    }
    record(&mut cache, [3; 32], epoch, 1, b"received copy");
    record(&mut cache, own, [4; 16], 24, b"previous epoch");
    let page = cache
        .export_page(generation, own, epoch, None, 20, |_, _, _| true)
        .unwrap();
    assert_eq!(page.items.len(), 20);
    assert!(
        page.items
            .iter()
            .all(|i| i.event.origin == own && i.event.epoch == epoch && i.body_available)
    );
    let next = cache
        .export_page(generation, own, epoch, page.next, 20, |_, _, _| true)
        .unwrap();
    assert_eq!(next.items.len(), 3);
    assert!(next.next.is_none());
    assert!(
        cache
            .export_page(generation, own, epoch, None, 0, |_, _, _| true)
            .is_err()
    );
    assert!(
        cache
            .export_page(generation, own, epoch, None, 21, |_, _, _| true)
            .is_err()
    );
    let invalid = HistoryCursor {
        offset: 999,
        ..page.next.unwrap()
    };
    assert_eq!(
        cache.export_page(generation, own, epoch, Some(invalid), 20, |_, _, _| true),
        Err("history_cursor_stale")
    );
    record(&mut cache, own, epoch, 25, b"new copy");
    assert_eq!(
        cache.export_page(generation, own, epoch, page.next, 20, |_, _, _| true),
        Err("history_cursor_stale")
    );
    assert!(
        cache
            .export_page(generation, own, epoch, None, 20, |_, _, _| false)
            .unwrap()
            .items
            .is_empty()
    );
    cache.enter_background();
    assert_eq!(
        cache.export_page(generation, own, epoch, None, 20, |_, _, _| true),
        Err("history_denied")
    );
}

#[test]
fn status_only_does_not_expose_bodies_or_content_derived_revision() {
    let own = [1; 32];
    let epoch = [2; 16];
    let mut pages = Vec::new();
    for bytes in [b"secret one", b"secret two"] {
        let mut cache = MobileHistory::new(20);
        cache.enter_foreground();
        record(&mut cache, own, epoch, 1, bytes);
        cache.set_mode(HistoryMode::Status);
        let generation = cache.generation();
        let page = cache
            .export_page(generation, own, epoch, None, 20, |_, _, _| true)
            .unwrap();
        assert!(!page.items[0].body_available);
        assert_eq!(page.items[0].metadata.digest, [0; 32]);
        pages.push(page);
        cache.clear();
        assert_eq!(
            cache.export_page(generation, own, epoch, None, 20, |_, _, _| true),
            Err("history_denied")
        );
    }
    assert_eq!(pages[0], pages[1]);
}
