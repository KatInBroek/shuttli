use super::*;

#[test]
fn history_and_roster_require_current_outgoing_consent() {
    let mut e = engine();
    let local = EventId {
        origin: [1; 32],
        epoch: [2; 16],
        seq: 7,
    };
    assert_eq!(e.authorize_peer_hints([3; 32]), Ok(()));
    assert_eq!(
        e.authorize_history_export([3; 32], local, &meta(1), true),
        Ok(())
    );
    assert_eq!(e.authorize_peer_hints([9; 32]), Err(Rejection::Invalid));
    assert_eq!(e.authorize_peer_hints([1; 32]), Err(Rejection::Invalid));
    assert_eq!(
        e.authorize_history_export([3; 32], incoming([3; 32]), &meta(1), false),
        Err(Rejection::Invalid)
    );
    assert_eq!(
        e.authorize_history_export([3; 32], EventId { seq: 0, ..local }, &meta(1), false),
        Err(Rejection::Invalid)
    );

    let mut settings = e.settings().clone();
    settings.automatic = false;
    e.configure(settings.clone()).unwrap();
    assert_eq!(
        e.authorize_history_export([3; 32], local, &meta(1), true),
        Ok(())
    );

    settings.peers.get_mut("B").unwrap().history = Some(HistoryMode::Status);
    e.configure(settings.clone()).unwrap();
    assert_eq!(
        e.authorize_history_export([3; 32], local, &meta(1), false),
        Ok(())
    );
    assert_eq!(
        e.authorize_history_export([3; 32], local, &meta(1), true),
        Err(Rejection::Disabled)
    );

    settings.history = HistoryMode::Off;
    e.configure(settings.clone()).unwrap();
    assert_eq!(
        e.authorize_history_export([3; 32], local, &meta(1), false),
        Err(Rejection::Disabled)
    );
    settings.history = HistoryMode::Content;
    settings.peers.get_mut("B").unwrap().history = None;
    settings.peers.get_mut("B").unwrap().send = false;
    e.configure(settings.clone()).unwrap();
    assert_eq!(e.authorize_peer_hints([3; 32]), Err(Rejection::Disabled));
    settings.peers.get_mut("B").unwrap().send = true;
    settings.text = false;
    e.configure(settings.clone()).unwrap();
    assert_eq!(
        e.authorize_history_export([3; 32], local, &meta(1), false),
        Err(Rejection::Disabled)
    );
    settings.text = true;
    settings.send = false;
    e.configure(settings).unwrap();
    assert_eq!(e.authorize_peer_hints([3; 32]), Err(Rejection::Disabled));
}

fn incoming(peer: DeviceId) -> EventId {
    EventId {
        origin: peer,
        epoch: [5; 16],
        seq: 1,
    }
}

#[test]
fn authorizations_bind_exact_event_target_content_baseline_and_revision() {
    let mut e = engine();
    let revision = e.revision();
    let outgoing = e.manual(stamp(7), &meta(7)).unwrap();
    assert_eq!(outgoing.len(), 2);
    for (permit, target) in outgoing.iter().zip([[3; 32], [4; 32]]) {
        assert_eq!(permit.target(), target);
        assert_eq!(permit.metadata(), &meta(7));
        assert_eq!(permit.policy_revision(), revision);
        assert_eq!(permit.event().origin, [1; 32]);
        assert_eq!(permit.event().epoch, [2; 16]);
        assert!(e.may_send(permit));
    }
    let event = incoming([3; 32]);
    let ticket = e.receive([3; 32], event, meta(9)).unwrap();
    assert_eq!(ticket.event(), event);
    assert_eq!(ticket.metadata(), &meta(9));
    assert_eq!(ticket.policy_revision(), revision);
    assert_eq!(ticket.baseline(), stamp(0));
    let authority = e.authorize_apply(&ticket, stamp(0)).unwrap();
    assert_eq!(authority.baseline(), stamp(0));
    assert_eq!(authority.metadata(), &meta(9));
    let local = e.authorize_local_copy(stamp(8), meta(2));
    assert_eq!(local.baseline(), stamp(8));
    assert_eq!(local.metadata(), &meta(2));
    e.written(stamp(2));
    assert!(e.observe(stamp(2), &meta(2)).unwrap().is_empty());
}

#[test]
fn invalid_configuration_and_revision_exhaustion_leave_policy_unchanged() {
    let mut e = engine();
    let original = e.settings().clone();
    let revision = e.revision();
    let mut invalid = original.clone();
    invalid.history_limit = 10_001;
    assert_eq!(e.configure(invalid), Err(Rejection::Invalid));
    assert_eq!(e.revision(), revision);
    assert_eq!(e.settings(), &original);
    e.revision = u64::MAX;
    let mut updated = original.clone();
    updated.send = false;
    assert_eq!(e.configure(updated), Err(Rejection::Exhausted));
    assert_eq!(e.settings(), &original);
    assert_eq!(e.revision(), u64::MAX);
}

#[test]
fn device_limit_rejects_self_and_new_entries_but_allows_existing_refresh() {
    let mut e = SyncCore::new([1; 32], [2; 16], Settings::default());
    assert_eq!(
        e.register([1; 32], String::from("self")),
        Err(Rejection::Invalid)
    );
    for n in 0u16..256 {
        let mut id = [0; 32];
        id[..2].copy_from_slice(&n.to_be_bytes());
        e.register(id, alloc::format!("peer-{n}")).unwrap();
    }
    assert_eq!(
        e.register([9; 32], String::from("overflow")),
        Err(Rejection::Invalid)
    );
    e.register([0; 32], String::from("renamed")).unwrap();
    assert_eq!(e.peers.len(), 256);
    assert_eq!(e.peers.get(&[0; 32]).unwrap(), "renamed");
}

#[test]
fn sequence_exhaustion_and_inconsistent_metadata_never_issue_authority() {
    let mut e = engine();
    assert!(matches!(
        e.manual(stamp(1), &meta(2)),
        Err(Rejection::Invalid)
    ));
    assert_eq!(e.seq, 0);
    e.seq = u64::MAX - 1;
    assert_eq!(
        e.manual(stamp(1), &meta(1)).unwrap()[0].event().seq,
        u64::MAX
    );
    assert!(matches!(
        e.manual(stamp(2), &meta(2)),
        Err(Rejection::Exhausted)
    ));
    assert_eq!(e.seq, u64::MAX);
}

#[test]
fn untrusted_and_unavailable_receives_are_rejected_before_a_write() {
    let mut e = engine();
    let mut zero = incoming([3; 32]);
    zero.seq = 0;
    for (peer, event) in [
        ([3; 32], incoming([4; 32])),
        ([1; 32], incoming([1; 32])),
        ([3; 32], zero),
        ([9; 32], incoming([9; 32])),
    ] {
        assert!(matches!(
            e.receive(peer, event, meta(1)),
            Err(Rejection::Invalid)
        ));
    }
    e.baseline(None);
    assert!(matches!(
        e.receive([3; 32], incoming([3; 32]), meta(1)),
        Err(Rejection::Unavailable)
    ));
    assert!(e.observe(stamp(3), &meta(3)).unwrap().is_empty());
    assert!(e.receive([3; 32], incoming([3; 32]), meta(1)).is_ok());
}

#[test]
fn automatic_off_observes_baseline_without_later_replaying_it() {
    let mut e = engine();
    let mut s = e.settings().clone();
    s.automatic = false;
    e.configure(s.clone()).unwrap();
    assert!(e.observe(stamp(4), &meta(4)).unwrap().is_empty());
    assert_eq!(e.manual(stamp(4), &meta(4)).unwrap().len(), 2);
    s.automatic = true;
    e.configure(s).unwrap();
    assert!(e.observe(stamp(4), &meta(4)).unwrap().is_empty());
    assert_eq!(e.observe(stamp(5), &meta(5)).unwrap().len(), 2);
    e.baseline(None);
    assert!(e.observe(stamp(6), &meta(6)).unwrap().is_empty());
}

#[test]
fn independent_observation_invalidates_inflight_write_even_if_readback_is_old() {
    let mut e = engine();
    let ticket = e.receive([3; 32], incoming([3; 32]), meta(9)).unwrap();
    e.baseline(Some(stamp(2)));
    assert!(matches!(
        e.authorize_apply(&ticket, stamp(0)),
        Err(Rejection::Stale)
    ));
    e.baseline(Some(stamp(0)));
    assert!(matches!(
        e.authorize_apply(&ticket, stamp(2)),
        Err(Rejection::Stale)
    ));
    assert!(e.authorize_apply(&ticket, stamp(0)).is_ok());
}

#[test]
fn permission_and_format_matrix_is_checked_for_both_send_and_receive() {
    for format in [Format::Text, Format::Png] {
        for send in [false, true] {
            // Each independent deny condition must beat otherwise valid settings.
            for denied in 1..7 {
                let mut e = engine();
                let mut s = e.settings().clone();
                let mut m = meta(1);
                m.format = format;
                let p = s.peers.get_mut("B").unwrap();
                match denied {
                    1 if send => s.send = false,
                    1 => s.receive = false,
                    2 if send => p.send = false,
                    2 => p.receive = false,
                    3 if format == Format::Text => s.text = false,
                    3 => s.png = false,
                    4 if format == Format::Text => p.text = false,
                    4 => p.png = false,
                    5 => {
                        p.max_bytes = 1;
                        m.size = 2;
                    }
                    _ => {
                        m.size = if format == Format::Text {
                            1024 * 1024 + 1
                        } else {
                            8 * 1024 * 1024 + 1
                        }
                    }
                }
                e.configure(s).unwrap();
                if send {
                    let permits = e.manual(stamp(1), &m);
                    if denied == 1 {
                        assert!(matches!(permits, Err(Rejection::Disabled)));
                    } else {
                        assert!(permits.unwrap().iter().all(|p| p.target() != [3; 32]));
                    }
                } else {
                    assert!(matches!(
                        e.receive([3; 32], incoming([3; 32]), m),
                        Err(Rejection::Disabled)
                    ));
                }
            }
        }
    }
}

#[test]
fn exact_size_boundaries_and_default_receive_policy_are_accepted() {
    let mut e = engine();
    for (format, size) in [(Format::Text, 1024 * 1024), (Format::Png, 8 * 1024 * 1024)] {
        let mut m = meta(1);
        m.format = format;
        m.size = size;
        assert_eq!(e.manual(stamp(1), &m).unwrap().len(), 2);
        assert!(e.receive([3; 32], incoming([3; 32]), m).is_ok());
    }
    e.register([9; 32], String::from("new")).unwrap();
    assert!(e.receive([9; 32], incoming([9; 32]), meta(1)).is_ok());
    assert!(
        e.manual(stamp(1), &meta(1))
            .unwrap()
            .iter()
            .all(|p| p.target() != [9; 32])
    );
}
fn stamp(n: u8) -> ClipboardStamp {
    ClipboardStamp {
        generation: n as u64,
        digest: [n; 32],
        sensitive: false,
    }
}
fn meta(n: u8) -> Metadata {
    Metadata {
        format: Format::Text,
        size: 1,
        digest: [n; 32],
    }
}
fn engine() -> SyncCore {
    let mut e = SyncCore::new([1; 32], [2; 16], Settings::default());
    e.register([3; 32], String::from("B")).unwrap();
    e.register([4; 32], String::from("C")).unwrap();
    let mut s = e.settings().clone();
    s.peers.insert(
        String::from("B"),
        PeerPolicy {
            send: true,
            ..PeerPolicy::default()
        },
    );
    s.peers.insert(
        String::from("C"),
        PeerPolicy {
            send: true,
            ..PeerPolicy::default()
        },
    );
    e.configure(s).unwrap();
    e.baseline(Some(stamp(0)));
    e
}
#[test]
fn remote_write_and_reassertion_never_relay() {
    let mut e = engine();
    let r = e
        .receive(
            [3; 32],
            EventId {
                origin: [3; 32],
                epoch: [5; 16],
                seq: 1,
            },
            meta(9),
        )
        .unwrap();
    e.may_apply(&r, stamp(0)).unwrap();
    e.written(stamp(9));
    let mut echo = stamp(9);
    echo.generation += 1;
    assert!(e.observe(echo, &meta(9)).unwrap().is_empty());
    assert_eq!(e.observe(stamp(10), &meta(10)).unwrap().len(), 2);
}
#[test]
fn disabled_interval_is_not_replayed() {
    let mut e = engine();
    let mut s = e.settings().clone();
    s.send = false;
    e.configure(s.clone()).unwrap();
    assert!(e.observe(stamp(7), &meta(7)).unwrap().is_empty());
    s.send = true;
    e.configure(s).unwrap();
    assert!(e.observe(stamp(7), &meta(7)).unwrap().is_empty());
    assert_eq!(e.observe(stamp(8), &meta(8)).unwrap().len(), 2);
}
#[test]
fn stale_and_revoked_receives_cannot_commit() {
    let mut e = engine();
    let r = e
        .receive(
            [3; 32],
            EventId {
                origin: [3; 32],
                epoch: [5; 16],
                seq: 1,
            },
            meta(9),
        )
        .unwrap();
    assert_eq!(e.may_apply(&r, stamp(1)), Err(Rejection::Stale));
    let mut s = e.settings().clone();
    s.receive = false;
    e.configure(s).unwrap();
    assert_eq!(e.may_apply(&r, stamp(0)), Err(Rejection::Disabled));
}
#[test]
fn restart_sensitive_and_default_send_closed() {
    let mut e = SyncCore::new([1; 32], [1; 16], Settings::default());
    e.register([2; 32], String::from("new")).unwrap();
    assert!(e.observe(stamp(7), &meta(7)).unwrap().is_empty());
    assert!(e.manual(stamp(7), &meta(7)).unwrap().is_empty());
    let mut e = engine();
    let mut x = stamp(1);
    x.sensitive = true;
    assert!(matches!(e.manual(x, &meta(1)), Err(Rejection::Sensitive)));
}
#[test]
fn manual_new_event_and_policy_revocation() {
    let mut e = engine();
    let a = e.manual(stamp(7), &meta(7)).unwrap();
    let b = e.manual(stamp(7), &meta(7)).unwrap();
    assert_ne!(a[0].event(), b[0].event());
    assert_eq!(a[0].event(), a[1].event());
    let mut s = e.settings().clone();
    s.send = false;
    e.configure(s).unwrap();
    assert!(!e.may_send(&a[0]));
}

#[test]
fn local_observation_exists_independently_of_send_mode_and_targets() {
    for (send, automatic, peers) in [
        (false, true, true),
        (true, false, true),
        (true, true, false),
        (true, true, true),
    ] {
        let mut e = engine();
        let mut settings = e.settings().clone();
        settings.send = send;
        settings.automatic = automatic;
        e.configure(settings).unwrap();
        if !peers {
            e.peers.clear();
        }
        let observed = e.observe_local(stamp(1), &meta(1)).unwrap();
        let event = observed.local_event.unwrap();
        assert_eq!(event.origin, [1; 32]);
        assert_eq!(
            observed.publications.len(),
            if send && automatic && peers { 2 } else { 0 }
        );
        assert!(observed.publications.iter().all(|p| p.event() == event));
        assert!(
            e.observe_local(stamp(1), &meta(1))
                .unwrap()
                .local_event
                .is_none()
        );
        assert!(
            e.observe_local(stamp(2), &meta(2))
                .unwrap()
                .local_event
                .unwrap()
                .seq
                > event.seq
        );
    }
}

#[test]
fn history_capture_rejects_sensitive_invalid_and_exhausted_observations() {
    let mut e = engine();
    let mut secret = stamp(3);
    secret.sensitive = true;
    assert!(matches!(
        e.observe_local(secret, &meta(3)),
        Err(Rejection::Sensitive)
    ));
    assert!(matches!(
        e.observe_local(stamp(4), &meta(5)),
        Err(Rejection::Invalid)
    ));
    e.seq = u64::MAX;
    assert!(matches!(
        e.observe_local(stamp(6), &meta(6)),
        Err(Rejection::Exhausted)
    ));
}

#[test]
fn startup_recovery_remote_and_local_only_writes_are_not_local_history() {
    let mut e = engine();
    e.baseline(None);
    assert!(
        e.observe_local(stamp(3), &meta(3))
            .unwrap()
            .local_event
            .is_none()
    );
    e.written(stamp(4));
    assert!(
        e.observe_local(stamp(4), &meta(4))
            .unwrap()
            .local_event
            .is_none()
    );
    let mut reasserted = stamp(4);
    reasserted.generation += 20;
    assert!(
        e.observe_local(reasserted, &meta(4))
            .unwrap()
            .local_event
            .is_none()
    );
    assert!(
        e.observe_local(stamp(5), &meta(5))
            .unwrap()
            .local_event
            .is_some()
    );
}
