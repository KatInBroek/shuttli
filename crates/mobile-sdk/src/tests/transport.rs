use super::*;
use crate::HistoryMode;
use shuttli_model::{
    mobile::HistorySummary,
    sync::{Format, Metadata},
};

#[tokio::test(flavor = "current_thread")]
async fn partial_control_frame_survives_cancelled_read_future() {
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
}

#[tokio::test(flavor = "current_thread")]
async fn session_cleanup_preserves_replacement_and_settles_only_its_source() {
    let source = [2; 32];
    let event = EventId {
        origin: [1; 32],
        epoch: [1; 16],
        seq: 1,
    };
    let history = Arc::new(Mutex::new(MobileHistory::default()));
    let generation = history.lock().unwrap().enter_foreground();
    history.lock().unwrap().source_activity(
        generation,
        source,
        HistoryActivity::Receiving,
        Some(event),
    );
    let (_, stop) = watch::channel(false);
    let context = SessionContext {
        identity: Arc::new(Identity::generate().unwrap().0),
        name: "Phone".into(),
        epoch: [1; 16],
        history: history.clone(),
        peers: Arc::new(Mutex::new(PeerDirectory::new([1; 32]))),
        results: Arc::new(Mutex::new(BTreeMap::new())),
        routes: Arc::new(tokio::sync::Mutex::new(BTreeMap::new())),
        stop,
        refresh: watch::channel(0).1,
    };
    let (old, _) = mpsc::channel(8);
    let (current, _) = mpsc::channel(8);
    context.routes.lock().await.insert(source, current.clone());
    finish_session(&context, source, generation, &old).await;
    assert_eq!(
        history.lock().unwrap().source(source).unwrap().receiving,
        Some(event)
    );
    let queued = EventId { seq: 2, ..event };
    let applied = EventId { seq: 3, ..event };
    context.results.lock().unwrap().extend([
        ((event, source), SendState::Sending),
        ((queued, source), SendState::Queued),
        ((applied, source), SendState::Applied),
        ((event, [3; 32]), SendState::Queued),
    ]);
    finish_session(&context, source, generation, &current).await;
    assert!(!context.routes.lock().await.contains_key(&source));
    let freshness = history.lock().unwrap().source(source).unwrap();
    assert_eq!(freshness.activity, HistoryActivity::Failed);
    assert_eq!(freshness.receiving, None);
    let results = context.results.lock().unwrap();
    assert_eq!(results[&(event, source)], SendState::Unknown);
    assert_eq!(results[&(queued, source)], SendState::Failed);
    assert_eq!(results[&(applied, source)], SendState::Applied);
    assert_eq!(results[&(event, [3; 32])], SendState::Queued);
}

#[test]
fn listener_only_accepts_tailnet_ipv4() {
    assert!(tailscale_ipv4("100.64.0.1".parse().unwrap()));
    assert!(tailscale_ipv4("100.127.255.254".parse().unwrap()));
    assert!(!tailscale_ipv4("100.128.0.1".parse().unwrap()));
    assert!(!tailscale_ipv4("192.168.1.2".parse().unwrap()));
}

#[test]
fn changed_hint_cannot_query_after_receive_or_history_is_disabled() {
    let (identity, _) = Identity::generate().unwrap();
    let source = [2; 32];
    let history = Arc::new(Mutex::new(MobileHistory::default()));
    history.lock().unwrap().enter_foreground();
    let peers = Arc::new(Mutex::new(PeerDirectory::new(identity.id)));
    peers
        .lock()
        .unwrap()
        .observed_direct(
            source,
            "Desktop".into(),
            "100.100.100.2:45987".into(),
            Capabilities::desktop(),
        )
        .unwrap();
    let (_, stop) = watch::channel(false);
    let context = SessionContext {
        identity: Arc::new(identity),
        name: "Phone".into(),
        epoch: [4; 16],
        history: history.clone(),
        peers: peers.clone(),
        results: Arc::new(Mutex::new(BTreeMap::new())),
        routes: Arc::new(tokio::sync::Mutex::new(BTreeMap::new())),
        stop,
        refresh: watch::channel(0).1,
    };
    assert!(can_query(&context, source));
    peers
        .lock()
        .unwrap()
        .directions(
            source,
            crate::peers::Directions {
                send: false,
                receive: false,
                ..crate::peers::Directions::default()
            },
        )
        .unwrap();
    assert!(!can_query(&context, source));
    peers
        .lock()
        .unwrap()
        .directions(source, crate::peers::Directions::default())
        .unwrap();
    history.lock().unwrap().set_mode(HistoryMode::Off);
    assert!(!can_query(&context, source));
    history.lock().unwrap().set_mode(HistoryMode::Status);
    assert!(can_query(&context, source));
    history.lock().unwrap().set_limit(0);
    assert!(!can_query(&context, source));
}

#[tokio::test(flavor = "current_thread")]
async fn foreground_session_fetches_into_app_cache_without_os_copy() {
    let source = [2; 32];
    let epoch = [3; 16];
    let event = EventId {
        origin: source,
        epoch,
        seq: 1,
    };
    let bytes = b"remote copy";
    let metadata = Metadata {
        format: Format::Text,
        size: bytes.len() as u64,
        digest: shuttli_content::canonical_digest(Format::Text, bytes).unwrap(),
    };
    let history = Arc::new(Mutex::new(MobileHistory::default()));
    let generation = history.lock().unwrap().enter_foreground();
    let peers = Arc::new(Mutex::new(PeerDirectory::new([1; 32])));
    peers
        .lock()
        .unwrap()
        .observed_direct(
            source,
            "Desktop".into(),
            "100.100.100.2:45987".into(),
            Capabilities::desktop(),
        )
        .unwrap();
    let (stop, receiver) = watch::channel(false);
    let (requested, refresh_rx) = watch::channel(0u64);
    let (mut phone, mut desktop) = tokio::io::duplex(65_536);
    let (command_sender, commands) = mpsc::channel(8);
    let context = SessionContext {
        identity: Arc::new(Identity::generate().unwrap().0),
        name: "Phone".into(),
        epoch: [4; 16],
        history: history.clone(),
        peers: peers.clone(),
        results: Arc::new(Mutex::new(BTreeMap::new())),
        routes: Arc::new(tokio::sync::Mutex::new(BTreeMap::new())),
        stop: receiver,
        refresh: refresh_rx,
    };
    let session = tokio::spawn(async move {
        mobile_session(&mut phone, source, epoch, generation, &context, commands).await
    });
    assert!(matches!(
        timeout(Duration::from_secs(1), read_frame(&mut desktop))
            .await
            .unwrap()
            .unwrap(),
        FrameV2::HistoryListRequest {
            cursor: None,
            limit: 20
        }
    ));
    write_frame(&mut desktop, &FrameV2::HistoryChanged { revision: 1 })
        .await
        .unwrap();
    write_frame(&mut desktop, &FrameV2::HistoryChanged { revision: 2 })
        .await
        .unwrap();
    write_frame(
        &mut desktop,
        &FrameV2::HistoryListResponse {
            source_epoch: epoch,
            revision: 1,
            items: vec![HistorySummary {
                event,
                metadata: metadata.clone(),
                copied_at_ms: 1,
                body_available: true,
            }],
            next: None,
        },
    )
    .await
    .unwrap();
    assert!(
        matches!(read_frame(&mut desktop).await.unwrap(), FrameV2::HistoryGet { event: e } if e == event)
    );
    write_frame(&mut desktop, &FrameV2::HistoryBody { event, metadata })
        .await
        .unwrap();
    desktop.write_all(bytes).await.unwrap();
    timeout(Duration::from_secs(1), async {
        loop {
            if history
                .lock()
                .unwrap()
                .body_for_explicit_copy(event)
                .is_some()
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        &*history
            .lock()
            .unwrap()
            .body_for_explicit_copy(event)
            .unwrap(),
        bytes
    );
    assert!(matches!(
        timeout(Duration::from_secs(1), read_frame(&mut desktop))
            .await
            .unwrap()
            .unwrap(),
        FrameV2::HistoryListRequest { cursor: None, .. }
    ));
    write_frame(
        &mut desktop,
        &FrameV2::HistoryListResponse {
            source_epoch: epoch,
            revision: 2,
            items: vec![],
            next: None,
        },
    )
    .await
    .unwrap();
    timeout(Duration::from_secs(1), async {
        while history.lock().unwrap().source(source).unwrap().activity != HistoryActivity::Updated {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(history.lock().unwrap().timeline().len(), 1);
    write_frame(&mut desktop, &FrameV2::HistoryChanged { revision: 3 })
        .await
        .unwrap();
    assert!(matches!(
        read_frame(&mut desktop).await.unwrap(),
        FrameV2::HistoryListRequest { .. }
    ));
    write_frame(
        &mut desktop,
        &FrameV2::Error {
            code: "history_denied".into(),
        },
    )
    .await
    .unwrap();
    timeout(Duration::from_secs(1), async {
        while history.lock().unwrap().source(source).unwrap().activity != HistoryActivity::Denied {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(history.lock().unwrap().body_available(event));
    // UI refresh bypasses the 15-second timer and preserves cached data.
    peers
        .lock()
        .unwrap()
        .directions(
            source,
            crate::peers::Directions {
                text: false,
                ..crate::peers::Directions::default()
            },
        )
        .unwrap();
    requested.send_modify(|r| *r += 1);
    assert!(matches!(
        timeout(Duration::from_secs(1), read_frame(&mut desktop))
            .await
            .unwrap()
            .unwrap(),
        FrameV2::HistoryListRequest { .. }
    ));
    let blocked = EventId { seq: 2, ..event };
    let summary = HistorySummary {
        event: blocked,
        metadata: Metadata {
            format: Format::Text,
            size: bytes.len() as u64,
            digest: shuttli_content::canonical_digest(Format::Text, bytes).unwrap(),
        },
        copied_at_ms: 2,
        body_available: true,
    };
    write_frame(
        &mut desktop,
        &FrameV2::HistoryListResponse {
            source_epoch: epoch,
            revision: 4,
            items: vec![summary.clone()],
            next: None,
        },
    )
    .await
    .unwrap();
    timeout(Duration::from_secs(1), async {
        while history.lock().unwrap().timeline().len() < 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(!history.lock().unwrap().body_available(blocked));
    assert!(history.lock().unwrap().body_available(event));
    peers
        .lock()
        .unwrap()
        .directions(source, crate::peers::Directions::default())
        .unwrap();
    requested.send_modify(|r| *r += 1);
    assert!(matches!(
        timeout(Duration::from_secs(1), read_frame(&mut desktop))
            .await
            .unwrap()
            .unwrap(),
        FrameV2::HistoryListRequest { .. }
    ));
    write_frame(
        &mut desktop,
        &FrameV2::HistoryListResponse {
            source_epoch: epoch,
            revision: 4,
            items: vec![summary.clone()],
            next: None,
        },
    )
    .await
    .unwrap();
    assert!(
        matches!(read_frame(&mut desktop).await.unwrap(), FrameV2::HistoryGet { event: e } if e == blocked)
    );
    write_frame(
        &mut desktop,
        &FrameV2::HistoryBody {
            event: blocked,
            metadata: summary.metadata,
        },
    )
    .await
    .unwrap();
    desktop.write_all(bytes).await.unwrap();
    timeout(Duration::from_secs(1), async {
        while !history.lock().unwrap().body_available(blocked) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    stop.send(true).unwrap();
    assert!(session.await.unwrap().is_ok());
    drop(command_sender);
}

#[tokio::test(flavor = "current_thread")]
async fn incoming_tls_binds_identity_before_history_and_explicit_sending() {
    for valid_selection in [false, true] {
        let (phone_identity, _) = Identity::generate().unwrap();
        let (desktop_identity, _) = Identity::generate().unwrap();
        let source = desktop_identity.id;
        let own = phone_identity.id;
        let history = Arc::new(Mutex::new(MobileHistory::default()));
        history.lock().unwrap().enter_foreground();
        let peers = Arc::new(Mutex::new(PeerDirectory::new(own)));
        let (stop, stop_rx) = watch::channel(false);
        let context = SessionContext {
            identity: Arc::new(phone_identity),
            name: "Phone".into(),
            epoch: [4; 16],
            history: history.clone(),
            peers: peers.clone(),
            results: Arc::new(Mutex::new(BTreeMap::new())),
            routes: Arc::new(tokio::sync::Mutex::new(BTreeMap::new())),
            stop: stop_rx,
            refresh: watch::channel(0).1,
        };
        // Loopback carries the real TLS handshake. The fictional tailnet
        // address models the address already admitted by run_listener.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server_context = context.clone();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            accept_desktop(stream, "100.64.0.2:45987".parse().unwrap(), server_context).await
        });
        let stream = TcpStream::connect(address).await.unwrap();
        let name = rustls::pki_types::ServerName::try_from("shuttli.local").unwrap();
        let mut desktop = TlsConnector::from(desktop_identity.client.clone())
            .connect(name, stream)
            .await
            .unwrap();
        write_hello(
            &mut desktop,
            &Hello::V2 {
                name: "Desktop".into(),
                epoch: [3; 16],
                capabilities: Capabilities::desktop(),
            },
        )
        .await
        .unwrap();
        assert!(
            matches!(read_hello(&mut desktop).await.unwrap(), Hello::V2 { capabilities, .. } if !capabilities.accept_live_offer && capabilities.history_pull)
        );
        write_frame(
            &mut desktop,
            &FrameV2::Select {
                initiator: if valid_selection { source } else { [9; 32] },
            },
        )
        .await
        .unwrap();
        if !valid_selection {
            assert!(
                server
                    .await
                    .unwrap()
                    .unwrap_err()
                    .contains("selection mismatch")
            );
            assert!(peers.lock().unwrap().direct().is_empty());
            continue;
        }
        assert!(
            matches!(read_frame(&mut desktop).await.unwrap(), FrameV2::Select { initiator } if initiator == source)
        );
        assert!(matches!(
            read_frame(&mut desktop).await.unwrap(),
            FrameV2::HistoryListRequest { .. }
        ));
        write_frame(
            &mut desktop,
            &FrameV2::HistoryListResponse {
                source_epoch: [3; 16],
                revision: 1,
                items: vec![],
                next: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(peers.lock().unwrap().direct()[0].id, source);
        assert!(!peers.lock().unwrap().direct()[0].directions.send);
        peers
            .lock()
            .unwrap()
            .directions(
                source,
                crate::peers::Directions {
                    send: true,
                    ..crate::peers::Directions::default()
                },
            )
            .unwrap();
        let body: Arc<[u8]> = Arc::from(&b"explicit phone send"[..]);
        let event = EventId {
            origin: own,
            epoch: context.epoch,
            seq: 1,
        };
        let route = context.routes.lock().await.get(&source).cloned().unwrap();
        route
            .send(SendCommand {
                event,
                target: source,
                metadata: Metadata {
                    format: Format::Text,
                    size: body.len() as u64,
                    digest: shuttli_content::canonical_digest(Format::Text, &body).unwrap(),
                },
                body: body.clone(),
            })
            .await
            .unwrap();
        assert!(
            matches!(timeout(Duration::from_secs(1), read_frame(&mut desktop)).await.unwrap().unwrap(), FrameV2::Offer { event: e, .. } if e == event)
        );
        write_frame(&mut desktop, &FrameV2::Ready).await.unwrap();
        let mut received = vec![0; body.len()];
        desktop.read_exact(&mut received).await.unwrap();
        assert_eq!(received, &*body);
        write_frame(&mut desktop, &FrameV2::Applied).await.unwrap();
        timeout(Duration::from_secs(1), async {
            while context.results.lock().unwrap().get(&(event, source)) != Some(&SendState::Applied)
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        stop.send(true).unwrap();
        assert!(server.await.unwrap().is_ok());
        assert!(!peers.lock().unwrap().direct()[0].online);
        assert!(context.routes.lock().await.is_empty());
        assert_eq!(
            context.results.lock().unwrap()[&(event, source)],
            SendState::Applied
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn explicit_offer_requires_send_consent_and_applied_reply() {
    let (identity, _) = Identity::generate().unwrap();
    let source = [2; 32];
    let epoch = [4; 16];
    let event = EventId {
        origin: identity.id,
        epoch,
        seq: 1,
    };
    let bytes: Arc<[u8]> = Arc::from(&b"manual copy"[..]);
    let command = SendCommand {
        event,
        target: source,
        metadata: Metadata {
            format: Format::Text,
            size: bytes.len() as u64,
            digest: shuttli_content::canonical_digest(Format::Text, &bytes).unwrap(),
        },
        body: bytes.clone(),
    };
    let peers = Arc::new(Mutex::new(PeerDirectory::new(identity.id)));
    peers
        .lock()
        .unwrap()
        .observed_direct(
            source,
            "Desktop".into(),
            "100.100.100.2:45987".into(),
            Capabilities::desktop(),
        )
        .unwrap();
    let (_, stop) = watch::channel(false);
    let context = SessionContext {
        identity: Arc::new(identity),
        name: "Phone".into(),
        epoch,
        history: Arc::new(Mutex::new(MobileHistory::default())),
        peers: peers.clone(),
        results: Arc::new(Mutex::new(BTreeMap::new())),
        routes: Arc::new(tokio::sync::Mutex::new(BTreeMap::new())),
        stop,
        refresh: watch::channel(0).1,
    };
    let (mut phone, mut desktop) = tokio::io::duplex(1024);
    assert_eq!(
        send_offer(&mut phone, &command, &context).await.unwrap(),
        SendState::Failed
    );
    peers
        .lock()
        .unwrap()
        .directions(
            source,
            crate::peers::Directions {
                send: true,
                receive: true,
                ..crate::peers::Directions::default()
            },
        )
        .unwrap();
    peers
        .lock()
        .unwrap()
        .directions(
            source,
            crate::peers::Directions {
                send: true,
                text: false,
                ..crate::peers::Directions::default()
            },
        )
        .unwrap();
    assert_eq!(
        send_offer(&mut phone, &command, &context).await.unwrap(),
        SendState::Failed
    );
    peers
        .lock()
        .unwrap()
        .directions(
            source,
            crate::peers::Directions {
                send: true,
                ..crate::peers::Directions::default()
            },
        )
        .unwrap();
    let task = tokio::spawn(async move { send_offer(&mut phone, &command, &context).await });
    assert!(
        matches!(read_frame(&mut desktop).await.unwrap(), FrameV2::Offer { event: offered, .. } if offered == event)
    );
    write_frame(&mut desktop, &FrameV2::Ready).await.unwrap();
    let mut body = vec![0; bytes.len()];
    desktop.read_exact(&mut body).await.unwrap();
    assert_eq!(body, &*bytes);
    write_frame(&mut desktop, &FrameV2::Applied).await.unwrap();
    assert_eq!(task.await.unwrap().unwrap(), SendState::Applied);
}
