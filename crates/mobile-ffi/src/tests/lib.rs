use super::*;
use shuttli_model::mobile::{HistoryListResponse, HistorySummary};
use shuttli_model::sync::Metadata;

#[test]
fn saved_consent_is_installed_before_any_transport_starts() {
    let session = MobileSession::new();
    let id = [2; 32];
    assert!(session.restore_device_directions(hex::encode(id), false, false));
    assert!(!session.restore_device_directions("invalid".into(), true, true));
    session.enter_foreground();
    let bytes = generate_identity_bytes();
    // A non-tailnet bind fails, but the directory has already been prepared.
    assert!(
        !session
            .start_listener(bytes, "127.0.0.1".into(), "Phone".into())
            .is_empty()
    );
    let slot = session.peers.lock().unwrap();
    let mut directory = slot.as_ref().unwrap().lock().unwrap();
    directory
        .observed_direct(
            id,
            "Computer".into(),
            "100.100.100.2:45987".into(),
            shuttli_model::mobile::Capabilities::desktop(),
        )
        .unwrap();
    assert_eq!(
        directory.direct()[0].directions,
        Directions {
            send: false,
            receive: false,
            ..Directions::default()
        }
    );
    drop(directory);
    drop(slot);
    assert!(session.restore_device_directions(hex::encode(id), true, false));
    assert!(session.device_rows()[0].send);
    assert!(!session.device_rows()[0].receive);
    for n in 3..=33 {
        assert!(session.restore_device_directions(hex::encode([n; 32]), false, false));
    }
    assert!(!session.restore_device_directions(hex::encode([34; 32]), true, true));
    assert!(session.restore_device_directions(hex::encode(id), false, false));
}

#[test]
fn native_ui_identity_and_content_policy_use_authenticated_identity() {
    let session = MobileSession::new();
    let (identity, bytes) = Identity::generate().unwrap();
    assert_eq!(
        session.identity_fingerprint(bytes.clone()),
        hex::encode(identity.id)
    );
    assert!(session.identity_fingerprint(vec![]).is_empty());
    assert!(!session.request_history_refresh());
    let id = [2; 32];
    assert!(session.restore_device_policy(hex::encode(id), true, false, false, true));
    session.enter_foreground();
    let _ = session.start_listener(bytes, "127.0.0.1".into(), "Phone".into());
    let peers = session.peers.lock().unwrap().as_ref().unwrap().clone();
    peers
        .lock()
        .unwrap()
        .observed_direct(
            id,
            "Device".into(),
            "100.64.0.2:45987".into(),
            shuttli_model::mobile::Capabilities::desktop(),
        )
        .unwrap();
    let row = session.device_rows().pop().unwrap();
    assert!(row.send && row.image);
    assert!(!row.receive && !row.text);
    assert_eq!(row.endpoint, "100.64.0.2:45987");
    assert!(session.set_receive(hex::encode(id), true));
    assert!(session.set_send(hex::encode(id), false));
    let row = session.device_rows().pop().unwrap();
    assert!(!row.send && row.receive && !row.text && row.image);
}

#[test]
fn native_bridge_preserves_one_session_across_lifecycle() {
    let session = MobileSession::new();
    assert_eq!(sdk_api_version(), 7);
    let first = session.enter_foreground();
    session.enter_background();
    assert!(session.enter_foreground() > first);
    assert_eq!(session.history_count(), 0);
    assert_eq!(session.connected_peers_count(), 0);
    assert!(!generate_identity_bytes().is_empty());
    assert!(
        !session
            .start_listener(vec![], "100.64.0.1".into(), "Phone".into())
            .is_empty()
    );
}

#[test]
fn native_settings_and_manual_actions_fail_closed_without_a_connection() {
    let session = MobileSession::default();
    assert!(matches!(session.history_mode(), MobileHistoryMode::Content));
    assert_eq!(session.history_limit(), 20);
    assert!(session.set_history_limit(0));
    assert!(!session.set_history_limit(10_001));
    session.set_history_mode(MobileHistoryMode::Status);
    assert!(matches!(session.history_mode(), MobileHistoryMode::Status));
    session.set_history_mode(MobileHistoryMode::Off);
    assert!(matches!(session.history_mode(), MobileHistoryMode::Off));
    assert!(!session.configure_image_cache(String::new()));
    assert!(!session.configure_image_cache("x".repeat(4097)));
    for id in ["invalid".to_string(), "aa".repeat(31), "aa".repeat(32)] {
        assert!(!session.set_receive(id.clone(), true));
        assert!(!session.set_send(id, true));
    }
    assert!(session.device_rows().is_empty());
    assert!(session.transfer_rows().is_empty());
    assert_eq!(session.send_text("manual copy".into()), 0);
    assert_eq!(session.send_text(String::new()), 0);
    assert_eq!(session.send_text("a\0b".into()), 0);
    assert_eq!(session.send_text("x".repeat(1_048_577)), 0);
    assert_eq!(session.send_image(vec![0; 32]), 0);
    assert_eq!(session.resend_local("invalid".into()), 0);
    let foreign = EventId {
        origin: [9; 32],
        epoch: [3; 16],
        seq: 1,
    };
    assert_eq!(session.resend_local(event_key(foreign)), 0);
    assert!(session.history_body(event_key(foreign)).is_empty());
    let (_, identity) = Identity::generate().unwrap();
    assert!(
        !session
            .start_listener(identity.clone(), "100.64.0.1".into(), "Phone".into())
            .is_empty()
    );
    session.enter_foreground();
    assert!(
        !session
            .start_listener(identity, "invalid".into(), "Phone".into())
            .is_empty()
    );
    assert_eq!(session.send_text("manual copy".into()), 0);
}

#[test]
fn event_keys_are_exact_and_cannot_select_another_item() {
    let event = EventId {
        origin: [4; 32],
        epoch: [5; 16],
        seq: 7,
    };
    assert_eq!(parse_event_key(&event_key(event)), Some(event));
    assert_eq!(parse_event_key("invalid"), None);
    assert_eq!(parse_event_key(&"aa".repeat(57)), None);
}

#[test]
fn native_history_returns_only_verified_cached_content() {
    let session = MobileSession::new();
    let generation = session.enter_foreground();
    let event = EventId {
        origin: [2; 32],
        epoch: [3; 16],
        seq: 1,
    };
    let data = b"copy me";
    let mut cache = session.history.lock().unwrap();
    cache
        .merge_page(
            generation,
            event.origin,
            HistoryListResponse {
                source_epoch: event.epoch,
                revision: 1,
                items: vec![HistorySummary {
                    event,
                    metadata: Metadata {
                        format: Format::Text,
                        size: data.len() as u64,
                        digest: shuttli_content::canonical_digest(Format::Text, data).unwrap(),
                    },
                    copied_at_ms: 1,
                    body_available: true,
                }],
                next: None,
            },
            1,
        )
        .unwrap();
    drop(cache);
    let key = session.history_rows()[0].event_key.clone();
    assert!(session.history_body(key.clone()).is_empty());
    session
        .history
        .lock()
        .unwrap()
        .cache_body(generation, event, data.to_vec())
        .unwrap();
    assert_eq!(session.history_body(key), data);
    session.clear_history();
    assert!(session.history_rows().is_empty());
}

#[test]
fn device_status_distinguishes_unsupported_offline_and_disabled_history() {
    let session = MobileSession::new();
    session.enter_foreground();
    let bytes = generate_identity_bytes();
    let _ = session.start_listener(bytes, "127.0.0.1".into(), "Peer".into());
    let peers = session.peers.lock().unwrap().as_ref().unwrap().clone();
    let id = [2; 32];
    peers
        .lock()
        .unwrap()
        .observed_direct(
            id,
            "Legacy".into(),
            "100.64.0.2:45987".into(),
            shuttli_model::mobile::Capabilities::legacy_desktop(),
        )
        .unwrap();
    assert!(matches!(
        session.device_rows()[0].history_activity,
        MobileHistoryActivity::Unsupported
    ));
    peers.lock().unwrap().disconnected(id);
    assert!(matches!(
        session.device_rows()[0].history_activity,
        MobileHistoryActivity::Unavailable
    ));
    session.set_receive(hex::encode(id), false);
    assert!(matches!(
        session.device_rows()[0].history_activity,
        MobileHistoryActivity::Paused
    ));
}
