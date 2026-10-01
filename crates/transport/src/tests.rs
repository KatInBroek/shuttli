use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio::time::{Duration, timeout};

fn event() -> EventId {
    EventId {
        origin: [1; 32],
        epoch: [2; 16],
        seq: 1,
    }
}
fn metadata(bytes: &[u8]) -> Metadata {
    Metadata {
        format: Format::Text,
        size: bytes.len() as u64,
        digest: shuttli_content::canonical_digest(Format::Text, bytes).unwrap(),
    }
}

#[tokio::test]
async fn both_wire_versions_share_offer_body_receipt_and_rejection() {
    for version in [WireVersion::V1, WireVersion::V2] {
        for accept in [false, true] {
            let (mut sender, mut receiver) = tokio::io::duplex(4096);
            let bytes = b"same live content";
            let meta = metadata(bytes);
            let stage = Arc::new(AtomicUsize::new(0));
            let stage_copy = stage.clone();
            let transaction = tokio::spawn(async move {
                send_live_version(
                    &mut sender,
                    LiveOffer {
                        event: event(),
                        metadata: &meta,
                        body: bytes,
                    },
                    || true,
                    |_| Ok(()),
                    || {
                        stage_copy.store(1, Ordering::SeqCst);
                    },
                    version,
                )
                .await
            });
            let meta = match read_live_frame(&mut receiver, version).await.unwrap() {
                FrameV2::Offer { event: id, meta } => {
                    assert_eq!(id, event());
                    meta
                }
                _ => panic!(),
            };
            assert_eq!(stage.load(Ordering::SeqCst), 0);
            if !accept {
                write_live_frame(
                    &mut receiver,
                    &FrameV2::Error {
                        code: "denied".into(),
                    },
                    version,
                )
                .await
                .unwrap();
                assert_eq!(transaction.await.unwrap().unwrap(), SendOutcome::Rejected);
                assert_eq!(stage.load(Ordering::SeqCst), 0);
                continue;
            }
            write_live_frame(&mut receiver, &FrameV2::Ready, version)
                .await
                .unwrap();
            assert_eq!(
                receive_body(&mut receiver, &meta, || true).await.unwrap(),
                bytes
            );
            assert_eq!(stage.load(Ordering::SeqCst), 1);
            // A completed payload is not success until the receiver commits and replies.
            assert!(!transaction.is_finished());
            write_live_frame(&mut receiver, &FrameV2::Applied, version)
                .await
                .unwrap();
            assert_eq!(transaction.await.unwrap().unwrap(), SendOutcome::Applied);
        }
    }
}

#[tokio::test]
async fn partial_frames_timeouts_and_unknown_receipts_cannot_be_success() {
    let (mut remote, mut local) = tokio::io::duplex(1024);
    let bytes = FrameV2::HistoryChanged { revision: 7 }.encode().unwrap();
    let mut packet = (bytes.len() as u32).to_be_bytes().to_vec();
    packet.extend(bytes);
    let mut reader = FrameReader::default();
    remote.write_all(&packet[..2]).await.unwrap();
    assert!(
        timeout(Duration::from_millis(5), reader.read(&mut local))
            .await
            .is_err()
    );
    assert!(!reader.is_idle());
    remote.write_all(&packet[2..6]).await.unwrap();
    assert!(
        timeout(Duration::from_millis(5), reader.read(&mut local))
            .await
            .is_err()
    );
    remote.write_all(&packet[6..]).await.unwrap();
    assert!(matches!(
        reader.read(&mut local).await.unwrap(),
        FrameV2::HistoryChanged { revision: 7 }
    ));
    assert!(reader.is_idle());
    for version in [WireVersion::V1, WireVersion::V2] {
        let (mut sender, mut receiver) = tokio::io::duplex(1024);
        let tx = tokio::spawn(async move {
            send_live_version(
                &mut sender,
                LiveOffer {
                    event: event(),
                    metadata: &metadata(b"body"),
                    body: b"body",
                },
                || true,
                |_| Ok(()),
                || {},
                version,
            )
            .await
        });
        read_live_frame(&mut receiver, version).await.unwrap();
        write_live_frame(&mut receiver, &FrameV2::Ready, version)
            .await
            .unwrap();
        let mut bytes = [0; 4];
        receiver.read_exact(&mut bytes).await.unwrap();
        drop(receiver); // Receiver may have committed; absence of ACK remains uncertain.
        assert!(tx.await.unwrap().is_err());
    }
}

#[tokio::test]
async fn notices_do_not_consume_ready_or_applied() {
    let (mut sender, mut receiver) = tokio::io::duplex(2048);
    let notices = Arc::new(AtomicUsize::new(0));
    let seen = notices.clone();
    let tx = tokio::spawn(async move {
        send_live(
            &mut sender,
            event(),
            &metadata(b"body"),
            b"body",
            || true,
            |_| {
                seen.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
            || {},
        )
        .await
    });
    read_frame(&mut receiver).await.unwrap();
    write_frame(
        &mut receiver,
        &FrameV2::PeerList {
            revision: 1,
            peers: vec![],
        },
    )
    .await
    .unwrap();
    write_frame(&mut receiver, &FrameV2::Ready).await.unwrap();
    let mut bytes = [0; 4];
    receiver.read_exact(&mut bytes).await.unwrap();
    write_frame(&mut receiver, &FrameV2::HistoryChanged { revision: 2 })
        .await
        .unwrap();
    write_frame(&mut receiver, &FrameV2::Applied).await.unwrap();
    assert_eq!(tx.await.unwrap().unwrap(), SendOutcome::Applied);
    assert_eq!(notices.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn invalid_bodies_and_revocation_are_rejected_before_commit() {
    let (mut writer, mut reader) = tokio::io::duplex(131072);
    let mut bad = metadata(b"body");
    bad.digest = [0; 32];
    writer.write_all(b"body").await.unwrap();
    assert!(receive_body(&mut reader, &bad, || true).await.is_err());
    let bytes = vec![b'x'; 65537];
    let meta = metadata(&bytes);
    writer.write_all(&bytes).await.unwrap();
    let checks = AtomicUsize::new(0);
    assert!(
        receive_body(&mut reader, &meta, || checks.fetch_add(1, Ordering::SeqCst)
            < 2)
        .await
        .is_err()
    );
    assert!(!valid_metadata(&Metadata {
        size: 0,
        ..meta.clone()
    }));
    assert!(!valid_metadata(&Metadata {
        size: 1024 * 1024 + 1,
        ..meta.clone()
    }));
    assert!(!valid_metadata(&Metadata {
        format: Format::Png,
        size: 8 * 1024 * 1024 + 1,
        ..meta
    }));
    assert!(leads_session([1; 32], [2; 32]));
    assert!(!leads_session([2; 32], [1; 32]));
}

#[tokio::test]
async fn initial_denial_invalid_metadata_and_wrong_ready_do_not_send_bodies() {
    for version in [WireVersion::V1, WireVersion::V2] {
        let (mut sender, mut receiver) = tokio::io::duplex(1024);
        assert_eq!(
            send_live_version(
                &mut sender,
                LiveOffer {
                    event: event(),
                    metadata: &metadata(b"body"),
                    body: b"body"
                },
                || false,
                |_| Ok(()),
                || panic!(),
                version
            )
            .await
            .unwrap(),
            SendOutcome::Rejected
        );
        assert!(
            timeout(
                Duration::from_millis(5),
                read_live_frame(&mut receiver, version)
            )
            .await
            .is_err()
        );
        let meta = Metadata {
            digest: [0; 32],
            ..metadata(b"body")
        };
        assert_eq!(
            send_live_version(
                &mut sender,
                LiveOffer {
                    event: event(),
                    metadata: &meta,
                    body: b"body"
                },
                || true,
                |_| Ok(()),
                || panic!(),
                version
            )
            .await
            .unwrap(),
            SendOutcome::Rejected
        );
        let tx = tokio::spawn(async move {
            send_live_version(
                &mut sender,
                LiveOffer {
                    event: event(),
                    metadata: &metadata(b"body"),
                    body: b"body",
                },
                || true,
                |_| Ok(()),
                || panic!(),
                version,
            )
            .await
        });
        read_live_frame(&mut receiver, version).await.unwrap();
        write_live_frame(&mut receiver, &FrameV2::Applied, version)
            .await
            .unwrap();
        assert!(tx.await.unwrap().is_err());
    }
}

#[tokio::test]
async fn shared_history_response_validates_content_and_rechecks_revocation_per_chunk() {
    use crate::history::{Export, respond};
    let bytes: Arc<[u8]> = Arc::from(vec![b'x'; 70_000]);
    let meta = metadata(&bytes);
    let (mut sender, mut receiver) = tokio::io::duplex(4096);
    let calls = AtomicUsize::new(0);
    let task = tokio::spawn(async move {
        respond(
            &mut sender,
            Ok((
                7,
                Export::Body {
                    event: event(),
                    metadata: meta,
                    bytes,
                },
            )),
            |token| token == 7 && calls.fetch_add(1, Ordering::SeqCst) < 2,
        )
        .await
    });
    assert!(matches!(
        read_frame(&mut receiver).await.unwrap(),
        FrameV2::HistoryBody { .. }
    ));
    let mut received = Vec::new();
    receiver.read_to_end(&mut received).await.unwrap();
    assert_eq!(received.len(), 65_536);
    assert_eq!(
        task.await.unwrap(),
        Err("history export permission changed".into())
    );
    for authorized in [false, true] {
        let (mut sender, mut receiver) = tokio::io::duplex(4096);
        let task = tokio::spawn(async move {
            respond(
                &mut sender,
                Ok((
                    1,
                    Export::Body {
                        event: event(),
                        metadata: metadata(b"expected"),
                        bytes: Arc::from(&b"tampered"[..]),
                    },
                )),
                |_| authorized,
            )
            .await
        });
        assert!(
            matches!(read_frame(&mut receiver).await.unwrap(),FrameV2::Error { code } if code == if authorized { "history_unavailable" } else { "history_denied" })
        );
        task.await.unwrap().unwrap();
    }
}
