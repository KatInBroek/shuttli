use super::*;

fn hint(id: u8) -> PeerHint {
    PeerHint {
        id: [id; 32],
        endpoint: format!("100.100.100.{id}:45987"),
        capabilities: Capabilities::desktop(),
    }
}

#[test]
fn overlapping_rosters_expire_per_source_without_granting_send() {
    let mut directory = PeerDirectory::new([1; 32]);
    for id in [2, 3] {
        let h = hint(id);
        directory
            .observed_direct(h.id, format!("source{id}"), h.endpoint, h.capabilities)
            .unwrap();
    }
    directory
        .accept_hints(
            [2; 32],
            PeerList {
                revision: 1,
                peers: vec![hint(4)],
            },
            1,
        )
        .unwrap();
    directory
        .accept_hints(
            [3; 32],
            PeerList {
                revision: 1,
                peers: vec![hint(4), hint(5)],
            },
            20,
        )
        .unwrap();
    assert_eq!(directory.candidates().len(), 3);
    directory.expire(1 + HINT_TTL_MS);
    assert_eq!(directory.candidates().len(), 2);
    let target = hint(4);
    directory
        .observed_direct(
            target.id,
            "target".into(),
            target.endpoint,
            target.capabilities,
        )
        .unwrap();
    assert_eq!(directory.candidates().len(), 1);
    assert_eq!(
        directory
            .direct()
            .iter()
            .find(|p| p.id == [4; 32])
            .unwrap()
            .directions,
        Directions::default()
    );
}

#[test]
fn forged_and_stale_rosters_cannot_change_trust_or_permissions() {
    let mut directory = PeerDirectory::new([1; 32]);
    let h = hint(2);
    assert_eq!(
        directory.accept_hints(
            h.id,
            PeerList {
                revision: 1,
                peers: vec![]
            },
            0
        ),
        Err(DirectoryError::UnknownSource)
    );
    directory
        .observed_direct(h.id, "source".into(), h.endpoint, h.capabilities)
        .unwrap();
    directory
        .directions(
            h.id,
            Directions {
                send: true,
                receive: false,
                ..Directions::default()
            },
        )
        .unwrap();
    assert_eq!(
        directory.accept_hints(
            h.id,
            PeerList {
                revision: 1,
                peers: vec![hint(1)]
            },
            0
        ),
        Err(DirectoryError::InvalidList)
    );
    directory
        .accept_hints(
            h.id,
            PeerList {
                revision: 1,
                peers: vec![hint(3)],
            },
            0,
        )
        .unwrap();
    assert_eq!(
        directory.accept_hints(
            h.id,
            PeerList {
                revision: 1,
                peers: vec![]
            },
            10
        ),
        Err(DirectoryError::StaleRevision)
    );
    directory.disconnected(h.id);
    assert!(directory.candidates().is_empty());
    assert_eq!(
        directory.direct()[0].directions,
        Directions {
            send: true,
            receive: false,
            ..Directions::default()
        }
    );
}

#[test]
fn restored_consent_requires_exact_direct_identity() {
    let mut directory = PeerDirectory::new([1; 32]);
    let id = [2; 32];
    assert!(directory.restore_directions(
        id,
        Directions {
            send: true,
            receive: false,
            ..Directions::default()
        }
    ));
    assert!(directory.direct().is_empty());
    assert!(!directory.restore_directions(
        [1; 32],
        Directions {
            send: true,
            receive: true,
            ..Directions::default()
        }
    ));
    directory
        .observed_direct(
            id,
            "known".into(),
            "100.64.0.2:45987".into(),
            Capabilities::desktop(),
        )
        .unwrap();
    directory
        .observed_direct(
            [3; 32],
            "new".into(),
            "100.64.0.3:45987".into(),
            Capabilities::desktop(),
        )
        .unwrap();
    assert_eq!(
        directory
            .direct()
            .iter()
            .find(|peer| peer.id == id)
            .unwrap()
            .directions,
        Directions {
            send: true,
            receive: false,
            ..Directions::default()
        }
    );
    assert_eq!(
        directory
            .direct()
            .iter()
            .find(|peer| peer.id == [3; 32])
            .unwrap()
            .directions,
        Directions::default()
    );
}

#[test]
fn content_permissions_and_directory_capacity_are_independent() {
    let policy = Directions {
        text: false,
        image: true,
        ..Directions::default()
    };
    assert!(!policy.allows(Format::Text));
    assert!(policy.allows(Format::Png));
    assert!(
        !Directions {
            image: false,
            ..policy
        }
        .allows(Format::Png)
    );
    let mut directory = PeerDirectory::new([1; 32]);
    assert_eq!(
        directory.directions([2; 32], policy),
        Err(DirectoryError::UnknownSource)
    );
    for (name, endpoint) in [
        ("", "100.64.0.2:45987"),
        ("invalid\nname", "100.64.0.2:45987"),
        ("valid", "invalid"),
    ] {
        assert_eq!(
            directory.observed_direct(
                [2; 32],
                name.into(),
                endpoint.into(),
                Capabilities::desktop()
            ),
            Err(DirectoryError::InvalidList)
        );
    }
    for id in 2..=33 {
        directory
            .observed_direct(
                [id; 32],
                "Device".into(),
                format!("100.64.0.{id}:45987"),
                Capabilities::desktop(),
            )
            .unwrap();
        assert!(directory.restore_directions([id; 32], policy));
    }
    assert_eq!(
        directory.observed_direct(
            [34; 32],
            "Overflow".into(),
            "100.64.0.34:45987".into(),
            Capabilities::desktop()
        ),
        Err(DirectoryError::Capacity)
    );
    assert!(!directory.restore_directions([34; 32], policy));
    assert!(directory.restore_directions([2; 32], Directions::default()));
    assert!(
        directory
            .observed_direct(
                [2; 32],
                "Renamed".into(),
                "100.64.0.2:45987".into(),
                Capabilities::desktop()
            )
            .is_ok()
    );
}
