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
fn live_commit_is_verified_idempotent_bounded_and_merges_with_later_history() {
    let mut history = MobileHistory::new(2);
    let generation = history.enter_foreground();
    let source = [1; 32];
    let summary = page(source, [3; 16], 1, b"live").items.remove(0);
    let event = summary.event;
    for _ in 0..2 {
        history
            .receive_live(
                generation,
                source,
                event,
                summary.metadata.clone(),
                b"live".to_vec(),
                200,
            )
            .unwrap();
    }
    assert_eq!(history.timeline().len(), 1);
    assert!(
        history.source(source).is_none(),
        "a live commit does not claim a list reconciliation"
    );
    history
        .merge_page(generation, source, page(source, [3; 16], 1, b"live"), 300)
        .unwrap();
    assert_eq!(history.timeline().len(), 1);
    assert_eq!(history.timeline()[0].summary.copied_at_ms, 100);
    assert_eq!(&*history.body_for_explicit_copy(event).unwrap(), b"live");
    assert_eq!(
        history.receive_live(
            generation,
            [2; 32],
            event,
            summary.metadata.clone(),
            b"live".to_vec(),
            200
        ),
        Err(HistoryError::InvalidBody)
    );
    assert_eq!(
        history.receive_live(
            generation,
            source,
            event,
            summary.metadata.clone(),
            b"fake".to_vec(),
            200
        ),
        Err(HistoryError::InvalidBody)
    );
    history.enter_background();
    assert_eq!(
        history.receive_live(
            generation,
            source,
            event,
            summary.metadata.clone(),
            b"live".to_vec(),
            200
        ),
        Err(HistoryError::Background)
    );
    let generation = history.enter_foreground();
    history.set_mode(HistoryMode::Status);
    assert_eq!(
        history.receive_live(
            generation,
            source,
            event,
            summary.metadata.clone(),
            b"live".to_vec(),
            200
        ),
        Err(HistoryError::Disabled)
    );
    history.set_mode(HistoryMode::Content);
    history.set_limit(0);
    assert_eq!(
        history.receive_live(
            generation,
            source,
            event,
            summary.metadata.clone(),
            b"live".to_vec(),
            200
        ),
        Err(HistoryError::Disabled)
    );
    history.set_limit(2);
    history.clear();
    assert_eq!(
        history.receive_live(
            generation,
            source,
            event,
            summary.metadata,
            b"live".to_vec(),
            200
        ),
        Err(HistoryError::Background)
    );
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
        .record_local_sent(
            generation,
            event,
            metadata.clone(),
            Arc::from(png.clone()),
            1,
        )
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
    std::fs::write(&file, vec![0; png.len() + 16]).unwrap();
    assert!(history.body_available(event));
    assert!(history.body_for_explicit_copy(event).is_none());
    assert!(!history.body_available(event));
    history.clear();
    assert!(!file.exists());
    let orphan = cache_dir.join("img-orphan.bin");
    std::fs::write(&orphan, b"old encrypted cache").unwrap();
    let _next_session = ImageCache::new(&base).unwrap();
    assert!(!orphan.exists());
    std::fs::remove_dir_all(base).unwrap();
    let later = EventId { seq: 2, ..event };
    history
        .record_local_sent(history.generation(), later, metadata, Arc::from(png), 2)
        .unwrap();
    assert_eq!(history.timeline().len(), 1);
    assert!(!history.timeline()[0].summary.body_available);
    assert!(!history.body_available(later));
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
