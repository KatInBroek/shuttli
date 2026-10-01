use super::*;
use crate::HistoryMode;
use shuttli_model::{
    mobile::HistorySummary,
    sync::{Format, Metadata},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

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
    let (stopped, stop) = watch::channel(false);
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
    let (old, mut old_receiver) = mpsc::channel(8);
    let (current, _) = mpsc::channel(8);
    register_route(&context, source, source, old.clone())
        .await
        .unwrap();
    register_route(&context, source, [1; 32], current.clone())
        .await
        .unwrap();
    assert!(
        register_route(&context, source, source, old.clone())
            .await
            .is_err()
    );
    assert!(
        register_route(&context, source, [1; 32], old.clone())
            .await
            .is_err()
    );
    assert!(
        context.routes.lock().await[&source]
            .commands
            .same_channel(&current)
    );
    finish_session(&context, source, generation, &old.downgrade()).await;
    drop(old);
    assert!(old_receiver.recv().await.is_none());
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
    finish_session(&context, source, generation, &current.downgrade()).await;
    assert!(!context.routes.lock().await.contains_key(&source));
    let freshness = history.lock().unwrap().source(source).unwrap();
    assert_eq!(freshness.activity, HistoryActivity::Failed);
    assert_eq!(freshness.receiving, None);
    {
        let results = context.results.lock().unwrap();
        assert_eq!(results[&(event, source)], SendState::Unknown);
        assert_eq!(results[&(queued, source)], SendState::Failed);
        assert_eq!(results[&(applied, source)], SendState::Applied);
        assert_eq!(results[&(event, [3; 32])], SendState::Queued);
    }
    // Background cleanup may finish after a new foreground listener starts.
    // Its stop token must prevent it from changing the shared directory/results.
    observe_peer(
        &context,
        source,
        "Peer".into(),
        "100.100.100.2:45987".into(),
        Capabilities::live(),
    )
    .unwrap();
    context.routes.lock().await.insert(
        source,
        Route {
            initiator: source,
            commands: current.clone(),
        },
    );
    context
        .results
        .lock()
        .unwrap()
        .insert((queued, source), SendState::Queued);
    stopped.send(true).unwrap();
    assert!(
        observe_peer(
            &context,
            source,
            "Old peer".into(),
            "100.100.100.2:45987".into(),
            Capabilities::live()
        )
        .is_err()
    );
    finish_session(&context, source, generation, &current.downgrade()).await;
    assert!(context.peers.lock().unwrap().direct()[0].online);
    assert_eq!(
        context.results.lock().unwrap()[&(queued, source)],
        SendState::Queued
    );
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
        epoch: peers.lock().unwrap().epoch(),
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
            Capabilities::pull_only(),
        )
        .unwrap();
    let (stop, receiver) = watch::channel(false);
    let (requested, refresh_rx) = watch::channel(0u64);
    let (mut phone, mut desktop) = tokio::io::duplex(65_536);
    let (command_sender, commands) = mpsc::channel(8);
    let context = SessionContext {
        identity: Arc::new(Identity::generate().unwrap().0),
        name: "Phone".into(),
        epoch: peers.lock().unwrap().epoch(),
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
            epoch: peers.lock().unwrap().epoch(),
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
            accept_peer(stream, "100.64.0.2:45987".parse().unwrap(), server_context).await
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
        let command = test_command(&peers, source, body.clone());
        let event = command.event();
        let route = context
            .routes
            .lock()
            .await
            .get(&source)
            .unwrap()
            .commands
            .clone();
        route.send(command).await.unwrap();
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
        suspend_peer_state(&peers, &context.results);
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
    let bytes: Arc<[u8]> = Arc::from(&b"manual copy"[..]);
    let peers = Arc::new(Mutex::new(PeerDirectory::new(identity.id)));
    let epoch = peers.lock().unwrap().epoch();
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
    peers
        .lock()
        .unwrap()
        .directions(
            source,
            crate::peers::Directions {
                send: true,
                ..Default::default()
            },
        )
        .unwrap();
    let command = test_command(&peers, source, bytes.clone());
    peers
        .lock()
        .unwrap()
        .directions(source, Default::default())
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
    context.history.lock().unwrap().enter_foreground();
    let (mut phone, mut desktop) = tokio::io::duplex(1024);
    assert_eq!(
        send_offer(&mut phone, &command, &context).await.unwrap().0,
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
        send_offer(&mut phone, &command, &context).await.unwrap().0,
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
    assert_eq!(
        send_offer(&mut phone, &command, &context).await.unwrap().0,
        SendState::Failed
    );
    let command = test_command(&peers, source, bytes.clone());
    let event = command.event();
    let task = tokio::spawn(async move { send_offer(&mut phone, &command, &context).await });
    assert!(
        matches!(read_frame(&mut desktop).await.unwrap(), FrameV2::Offer { event: offered, .. } if offered == event)
    );
    write_frame(&mut desktop, &FrameV2::Ready).await.unwrap();
    let mut body = vec![0; bytes.len()];
    desktop.read_exact(&mut body).await.unwrap();
    assert_eq!(body, &*bytes);
    write_frame(&mut desktop, &FrameV2::Applied).await.unwrap();
    assert_eq!(task.await.unwrap().unwrap(), (SendState::Applied, true));
}

#[tokio::test(flavor = "current_thread")]
async fn live_and_pull_interleave_in_both_identity_orders_without_forwarding_or_duplicate_fetch() {
    for source in [[0; 32], [255; 32]] {
        let identity = Identity::generate().unwrap().0;
        let own = identity.id;
        let desktop_leads = source < own;
        let history = Arc::new(Mutex::new(MobileHistory::default()));
        let generation = history.lock().unwrap().enter_foreground();
        let peers = Arc::new(Mutex::new(PeerDirectory::new(own)));
        peers
            .lock()
            .unwrap()
            .observed_direct(
                source,
                "Peer".into(),
                "100.64.0.2:45987".into(),
                Capabilities::live(),
            )
            .unwrap();
        peers
            .lock()
            .unwrap()
            .directions(
                source,
                crate::peers::Directions {
                    send: true,
                    ..Default::default()
                },
            )
            .unwrap();
        let (stop, stop_rx) = watch::channel(false);
        let (refresh, refresh_rx) = watch::channel(0);
        let context = SessionContext {
            identity: Arc::new(identity),
            name: "Peer".into(),
            epoch: peers.lock().unwrap().epoch(),
            history: history.clone(),
            peers: peers.clone(),
            results: Arc::new(Mutex::new(BTreeMap::new())),
            routes: Arc::new(tokio::sync::Mutex::new(BTreeMap::new())),
            stop: stop_rx,
            refresh: refresh_rx,
        };
        let epoch = [3; 16];
        let (sender, commands) = mpsc::channel(8);
        let (mut local, mut remote) = tokio::io::duplex(4096);
        let running = context.clone();
        let session = tokio::spawn(async move {
            mobile_session(&mut local, source, epoch, generation, &running, commands).await
        });
        if desktop_leads {
            write_frame(&mut remote, &FrameV2::Poll).await.unwrap();
        }
        assert!(matches!(
            read_operation(&mut remote).await.unwrap(),
            FrameV2::HistoryListRequest { .. }
        ));
        write_frame(
            &mut remote,
            &FrameV2::HistoryListResponse {
                source_epoch: epoch,
                revision: 1,
                items: vec![],
                next: None,
            },
        )
        .await
        .unwrap();
        if !desktop_leads {
            assert!(matches!(
                read_operation(&mut remote).await.unwrap(),
                FrameV2::Poll
            ));
        }
        // Recovery may query an event from before the sender restarted.
        let previous = EventId {
            origin: source,
            epoch: [9; 16],
            seq: 1,
        };
        write_frame(&mut remote, &FrameV2::Status { event: previous })
            .await
            .unwrap();
        assert!(matches!(
            read_operation(&mut remote).await.unwrap(),
            FrameV2::Receipt { event: queried, state: Some(shuttli_model::sync::DeliveryState::Unknown) }
                if queried == previous
        ));
        if !desktop_leads {
            assert!(matches!(
                read_operation(&mut remote).await.unwrap(),
                FrameV2::Poll
            ));
        }
        let event = EventId {
            origin: source,
            epoch,
            seq: 1,
        };
        let body = b"live remote copy";
        let meta = Metadata {
            format: Format::Text,
            size: body.len() as u64,
            digest: shuttli_content::canonical_digest(Format::Text, body).unwrap(),
        };
        write_frame(
            &mut remote,
            &FrameV2::Offer {
                event,
                meta: meta.clone(),
            },
        )
        .await
        .unwrap();
        assert!(matches!(
            read_operation(&mut remote).await.unwrap(),
            FrameV2::Ready
        ));
        remote.write_all(body).await.unwrap();
        assert!(matches!(
            read_operation(&mut remote).await.unwrap(),
            FrameV2::Applied
        ));
        assert_eq!(
            &*history
                .lock()
                .unwrap()
                .body_for_explicit_copy(event)
                .unwrap(),
            body
        );
        assert!(
            context.results.lock().unwrap().is_empty(),
            "reception cannot publish or forward"
        );
        let bytes: Arc<[u8]> = Arc::from(&b"explicit local send"[..]);
        let command = test_command(&peers, source, bytes.clone());
        let outgoing = command.event();
        sender.send(command).await.unwrap();
        refresh.send_modify(|r| *r += 1);
        let mut sent = false;
        let mut listed = false;
        for _ in 0..12 {
            if desktop_leads {
                write_frame(&mut remote, &FrameV2::Poll).await.unwrap();
            }
            match timeout(Duration::from_secs(2), read_operation(&mut remote))
                .await
                .unwrap()
                .unwrap()
            {
                FrameV2::Poll => {
                    write_frame(&mut remote, &FrameV2::Idle).await.unwrap();
                }
                FrameV2::Idle => {}
                FrameV2::HistoryListRequest { .. } => {
                    write_frame(
                        &mut remote,
                        &FrameV2::HistoryListResponse {
                            source_epoch: epoch,
                            revision: 2,
                            items: vec![HistorySummary {
                                event,
                                metadata: meta.clone(),
                                copied_at_ms: 100,
                                body_available: true,
                            }],
                            next: None,
                        },
                    )
                    .await
                    .unwrap();
                    listed = true;
                }
                FrameV2::Offer { event: offered, .. } => {
                    assert_eq!(offered, outgoing);
                    write_frame(&mut remote, &FrameV2::HistoryChanged { revision: 2 })
                        .await
                        .unwrap();
                    write_frame(&mut remote, &FrameV2::Ready).await.unwrap();
                    let mut received = vec![0; bytes.len()];
                    remote.read_exact(&mut received).await.unwrap();
                    assert_eq!(received, &*bytes);
                    write_frame(&mut remote, &FrameV2::HistoryChanged { revision: 3 })
                        .await
                        .unwrap();
                    write_frame(&mut remote, &FrameV2::Applied).await.unwrap();
                    sent = true;
                }
                FrameV2::HistoryGet { .. } => {
                    panic!("verified live body must not be fetched twice")
                }
                other => panic!("unexpected frame {other:?}"),
            }
            if sent && listed {
                break;
            }
        }
        assert!(sent && listed);
        timeout(Duration::from_secs(1), async {
            while context.results.lock().unwrap().get(&(outgoing, source))
                != Some(&SendState::Applied)
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(history.lock().unwrap().timeline().len(), 1);
        stop.send(true).unwrap();
        assert!(session.await.unwrap().is_ok());
    }
}

#[tokio::test(flavor = "current_thread")]
async fn original_v1_tls_peer_can_send_and_receive_with_app_cache_adapter() {
    let phone = Identity::generate().unwrap().0;
    let remote_identity = Identity::generate().unwrap().0;
    let source = remote_identity.id;
    let history = Arc::new(Mutex::new(MobileHistory::default()));
    history.lock().unwrap().enter_foreground();
    let (stop, stop_rx) = watch::channel(false);
    let peers = Arc::new(Mutex::new(PeerDirectory::new(phone.id)));
    let context = SessionContext {
        identity: Arc::new(phone),
        name: "Peer".into(),
        epoch: peers.lock().unwrap().epoch(),
        history: history.clone(),
        peers: peers.clone(),
        results: Arc::new(Mutex::new(BTreeMap::new())),
        routes: Arc::new(tokio::sync::Mutex::new(BTreeMap::new())),
        stop: stop_rx,
        refresh: watch::channel(0).1,
    };
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let running = context.clone();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        accept_peer(stream, "100.64.0.2:45987".parse().unwrap(), running).await
    });
    let mut remote = TlsConnector::from(remote_identity.client.clone())
        .connect(
            rustls::pki_types::ServerName::try_from("shuttli.local").unwrap(),
            TcpStream::connect(address).await.unwrap(),
        )
        .await
        .unwrap();
    write_hello(
        &mut remote,
        &Hello::V1 {
            name: "Legacy peer".into(),
            epoch: [3; 16],
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        read_hello(&mut remote).await.unwrap(),
        Hello::V1 { .. }
    ));
    write_live_frame(
        &mut remote,
        &FrameV2::Select { initiator: source },
        WireVersion::V1,
    )
    .await
    .unwrap();
    assert!(matches!(
        read_live_frame(&mut remote, WireVersion::V1).await.unwrap(),
        FrameV2::Select { .. }
    ));
    let previous = EventId {
        origin: source,
        epoch: [9; 16],
        seq: 1,
    };
    let pulled = b"queried history content";
    let pulled_meta = Metadata {
        format: Format::Text,
        size: pulled.len() as u64,
        digest: shuttli_content::canonical_digest(Format::Text, pulled).unwrap(),
    };
    {
        let mut cache = history.lock().unwrap();
        let generation = cache.generation();
        cache
            .merge_page(
                generation,
                source,
                shuttli_model::mobile::HistoryListResponse {
                    source_epoch: previous.epoch,
                    revision: 1,
                    items: vec![HistorySummary {
                        event: previous,
                        metadata: pulled_meta,
                        copied_at_ms: 1,
                        body_available: true,
                    }],
                    next: None,
                },
                2,
            )
            .unwrap();
        cache
            .cache_body(generation, previous, pulled.to_vec())
            .unwrap();
        assert!(cache.body_available(previous));
        assert!(!cache.live_receipt_available(previous));
    }
    write_live_frame(
        &mut remote,
        &FrameV2::Status { event: previous },
        WireVersion::V1,
    )
    .await
    .unwrap();
    assert!(matches!(
        read_live_frame(&mut remote, WireVersion::V1).await.unwrap(),
        FrameV2::Receipt { event: queried, state: Some(shuttli_model::sync::DeliveryState::Unknown) }
            if queried == previous
    ));
    let event = EventId {
        origin: source,
        epoch: [3; 16],
        seq: 1,
    };
    let bytes = b"v1 live content";
    let meta = Metadata {
        format: Format::Text,
        size: bytes.len() as u64,
        digest: shuttli_content::canonical_digest(Format::Text, bytes).unwrap(),
    };
    assert_eq!(
        shuttli_transport::send_live_version(
            &mut remote,
            shuttli_transport::LiveOffer {
                event,
                metadata: &meta,
                body: bytes
            },
            || true,
            |_| Ok(()),
            || {},
            WireVersion::V1
        )
        .await
        .unwrap(),
        shuttli_transport::SendOutcome::Applied
    );
    assert_eq!(
        &*history
            .lock()
            .unwrap()
            .body_for_explicit_copy(event)
            .unwrap(),
        bytes
    );
    context
        .peers
        .lock()
        .unwrap()
        .directions(
            source,
            crate::peers::Directions {
                send: true,
                ..Default::default()
            },
        )
        .unwrap();
    let command = test_command(&context.peers, source, Arc::from(&bytes[..]));
    let own = command.event();
    let route = context
        .routes
        .lock()
        .await
        .get(&source)
        .unwrap()
        .commands
        .clone();
    route.send(command).await.unwrap();
    write_live_frame(&mut remote, &FrameV2::Poll, WireVersion::V1)
        .await
        .unwrap();
    assert!(
        matches!(read_live_frame(&mut remote, WireVersion::V1).await.unwrap(), FrameV2::Offer { event: offered, .. } if offered == own)
    );
    write_live_frame(&mut remote, &FrameV2::Ready, WireVersion::V1)
        .await
        .unwrap();
    assert_eq!(
        shuttli_transport::receive_body(&mut remote, &meta, || true)
            .await
            .unwrap(),
        bytes
    );
    write_live_frame(&mut remote, &FrameV2::Applied, WireVersion::V1)
        .await
        .unwrap();
    timeout(Duration::from_secs(1), async {
        while context.results.lock().unwrap().get(&(own, source)) != Some(&SendState::Applied) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    write_live_frame(&mut remote, &FrameV2::Status { event }, WireVersion::V1)
        .await
        .unwrap();
    assert!(matches!(
        read_live_frame(&mut remote, WireVersion::V1).await.unwrap(),
        FrameV2::Receipt {
            state: Some(shuttli_model::sync::DeliveryState::Applied),
            ..
        }
    ));
    history.lock().unwrap().clear();
    write_live_frame(&mut remote, &FrameV2::Status { event }, WireVersion::V1)
        .await
        .unwrap();
    assert!(matches!(
        read_live_frame(&mut remote, WireVersion::V1).await.unwrap(),
        FrameV2::Receipt {
            state: Some(shuttli_model::sync::DeliveryState::Unknown),
            ..
        }
    ));
    let forged = EventId {
        origin: context.identity.id,
        ..previous
    };
    write_live_frame(
        &mut remote,
        &FrameV2::Status { event: forged },
        WireVersion::V1,
    )
    .await
    .unwrap();
    assert_eq!(server.await.unwrap().unwrap_err(), "receipt owner mismatch");
    stop.send(true).unwrap();
}

fn test_command(
    peers: &Arc<Mutex<PeerDirectory>>,
    target: DeviceId,
    body: Arc<[u8]>,
) -> SendCommand {
    let metadata = Metadata {
        format: Format::Text,
        size: body.len() as u64,
        digest: shuttli_content::canonical_digest(Format::Text, &body).unwrap(),
    };
    let permit = peers
        .lock()
        .unwrap()
        .manual(&metadata)
        .unwrap()
        .into_iter()
        .find(|p| p.target() == target)
        .unwrap();
    SendCommand {
        permit: Arc::new(permit),
        body,
    }
}

#[tokio::test(flavor = "current_thread")]
async fn two_endpoints_pull_each_others_history_and_keep_live_delivery_and_clear_working() {
    run_bidirectional_history(100).await;
    run_bidirectional_history(5).await;
}

async fn run_bidirectional_history(limit: usize) {
    let mut identities = [
        Identity::generate().unwrap().0,
        Identity::generate().unwrap().0,
    ];
    identities.sort_by_key(|identity| identity.id);
    if limit == 5 {
        identities.reverse();
    }
    let ids = [identities[0].id, identities[1].id];
    let mut contexts = Vec::new();
    let mut stops = Vec::new();
    let mut receivers = Vec::new();
    let mut senders = Vec::new();
    for (i, identity) in identities.into_iter().enumerate() {
        let mut directory = PeerDirectory::new(identity.id);
        let other = ids[1 - i];
        directory
            .observed_direct(
                other,
                "Peer".into(),
                format!("100.64.0.{}:45987", 2 - i),
                Capabilities::live(),
            )
            .unwrap();
        directory
            .directions(
                other,
                crate::peers::Directions {
                    send: true,
                    ..Default::default()
                },
            )
            .unwrap();
        let history = Arc::new(Mutex::new(MobileHistory::new(limit)));
        let generation = history.lock().unwrap().enter_foreground();
        let (stop, stop_rx) = watch::channel(false);
        let (sender, receiver) = mpsc::channel(8);
        let context = SessionContext {
            epoch: directory.epoch(),
            identity: Arc::new(identity),
            name: "Peer".into(),
            history,
            peers: Arc::new(Mutex::new(directory)),
            results: Arc::new(Mutex::new(BTreeMap::new())),
            routes: Arc::new(tokio::sync::Mutex::new(BTreeMap::new())),
            stop: stop_rx,
            refresh: watch::channel(0).1,
        };
        // More than one page forces a follower to request another grant after
        // a reply, exercising the same turn protocol in either identity order.
        for seq in 0..23 {
            let command = test_command(
                &context.peers,
                other,
                Arc::from(format!("copy {i}-{seq}").into_bytes()),
            );
            context
                .history
                .lock()
                .unwrap()
                .record_local_sent(
                    generation,
                    command.event(),
                    command.metadata().clone(),
                    command.body,
                    100 + seq,
                )
                .unwrap();
        }
        contexts.push(context);
        stops.push(stop);
        senders.push(sender);
        receivers.push(receiver);
    }
    let (mut first, mut second) = tokio::io::duplex(4096);
    let a = contexts[0].clone();
    let b = contexts[1].clone();
    let a_epoch = a.epoch;
    let b_epoch = b.epoch;
    let mut receivers = receivers.into_iter();
    let a_rx = receivers.next().unwrap();
    let b_rx = receivers.next().unwrap();
    let sessions = [
        tokio::spawn(async move { mobile_session(&mut first, ids[1], b_epoch, 1, &a, a_rx).await }),
        tokio::spawn(
            async move { mobile_session(&mut second, ids[0], a_epoch, 1, &b, b_rx).await },
        ),
    ];
    timeout(Duration::from_secs(8), async {
        loop {
            if contexts.iter().enumerate().all(|(i, c)| {
                c.history
                    .lock()
                    .unwrap()
                    .timeline()
                    .iter()
                    .filter(|r| r.source == ids[1 - i] && r.body.is_some())
                    .count()
                    >= if limit == 100 { 23 } else { 1 }
            }) {
                break;
            }
            assert!(
                !sessions.iter().any(|s| s.is_finished()),
                "history requests must not disconnect peers"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("bidirectional paginated history bodies");
    for context in &contexts {
        assert!(context.history.lock().unwrap().timeline().len() <= limit);
    }
    let refused = test_command(&contexts[0].peers, ids[1], Arc::from(&b"revoked copy"[..]));
    let refused_event = refused.event();
    contexts[0]
        .peers
        .lock()
        .unwrap()
        .directions(ids[1], crate::peers::Directions::default())
        .unwrap();
    senders[0].send(refused).await.unwrap();
    timeout(Duration::from_secs(3), async {
        while contexts[0]
            .results
            .lock()
            .unwrap()
            .get(&(refused_event, ids[1]))
            != Some(&SendState::Failed)
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    contexts[0]
        .peers
        .lock()
        .unwrap()
        .directions(
            ids[1],
            crate::peers::Directions {
                send: true,
                ..Default::default()
            },
        )
        .unwrap();
    let command = test_command(&contexts[0].peers, ids[1], Arc::from(&b"new live copy"[..]));
    let event = command.event();
    senders[0].send(command).await.unwrap();
    timeout(Duration::from_secs(3), async {
        while contexts[0].results.lock().unwrap().get(&(event, ids[1])) != Some(&SendState::Applied)
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(
        contexts[1]
            .history
            .lock()
            .unwrap()
            .live_receipt_available(event)
    );
    contexts[1].history.lock().unwrap().clear();
    contexts[1].refresh.clone().mark_changed();
    timeout(Duration::from_secs(8), async {
        while !contexts[1]
            .history
            .lock()
            .unwrap()
            .timeline()
            .iter()
            .any(|r| r.source == ids[0] && r.body.is_some())
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("clearing a cache must not disable the existing session");
    for stop in stops {
        stop.send(true).unwrap();
    }
    for session in sessions {
        let result = session.await.unwrap();
        assert!(
            result.is_ok() || result == Err("session closed".into()),
            "{result:?}"
        );
    }
}

async fn read_operation<S: AsyncRead + Unpin>(stream: &mut S) -> Result<FrameV2> {
    loop {
        let frame = read_frame(stream).await?;
        if !matches!(
            frame,
            FrameV2::PeerList { .. } | FrameV2::HistoryChanged { .. }
        ) {
            return Ok(frame);
        }
    }
}
