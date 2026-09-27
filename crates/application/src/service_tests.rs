use super::service::Service;
use shuttli_api::control::*;
use shuttli_core::sync::{Publication, WriteAuthorization};
use shuttli_model::sync::*;
use shuttli_ports::sync::*;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, mpsc};
fn payload(n: u8) -> Payload {
    Payload {
        meta: Metadata {
            format: Format::Text,
            size: 1,
            digest: [n; 32],
        },
        data: Arc::from([n]),
    }
}
fn value(n: u8) -> ClipboardValue {
    ClipboardValue {
        stamp: ClipboardStamp {
            generation: n as u64,
            digest: [n; 32],
            sensitive: false,
        },
        payload: Some(payload(n)),
    }
}
struct Clip(Arc<Mutex<ClipboardValue>>);
impl Clipboard for Clip {
    fn read(&mut self) -> Result<ClipboardValue> {
        Ok(self.0.lock().unwrap().clone())
    }
    fn write(&mut self, p: &Payload, authorization: WriteAuthorization) -> Result<ClipboardValue> {
        let mut v = self.0.lock().unwrap();
        if v.stamp != authorization.baseline() {
            return Err("stale".into());
        }
        assert_eq!(&p.meta, authorization.metadata());
        *v = ClipboardValue {
            stamp: ClipboardStamp {
                generation: v.stamp.generation + 1,
                digest: p.meta.digest,
                sensitive: false,
            },
            payload: Some(p.clone()),
        };
        Ok(v.clone())
    }
    fn description(&self) -> &str {
        "test clipboard"
    }
}
#[derive(Default)]
struct NetState {
    input: VecDeque<NetworkEvent>,
    sent: Vec<Publication>,
    revision: u64,
}
struct Net(Arc<Mutex<NetState>>);
impl Network for Net {
    fn poll(&mut self) -> Option<NetworkEvent> {
        self.0.lock().unwrap().input.pop_front()
    }
    fn send(&mut self, p: Publication, _: Payload) -> Result<()> {
        self.0.lock().unwrap().sent.push(p);
        Ok(())
    }
    fn policy_revision(&mut self, r: u64) {
        self.0.lock().unwrap().revision = r;
    }
    fn refresh(&mut self) {}
    fn reconcile(&mut self, _: DeviceId, _: EventId) -> Result<()> {
        Ok(())
    }
}
struct MemoryStore {
    settings: Settings,
    fail: Arc<Mutex<bool>>,
    intent: bool,
    rows: Arc<Mutex<Vec<HistoryEntry>>>,
}
impl Store for MemoryStore {
    fn settings(&self) -> Result<Settings> {
        Ok(self.settings.clone())
    }
    fn save_settings(&mut self, s: &Settings) -> Result<()> {
        if *self.fail.lock().unwrap() {
            return Err("disk full".into());
        }
        self.settings = s.clone();
        Ok(())
    }
    fn peers(&self) -> Result<Vec<PeerInfo>> {
        Ok(vec![
            PeerInfo {
                id: "02".repeat(32),
                name: "B".into(),
                address: "test".into(),
                online: true,
            },
            PeerInfo {
                id: "03".repeat(32),
                name: "C".into(),
                address: "test".into(),
                online: true,
            },
        ])
    }
    fn peer(&mut self, _: &PeerInfo) -> Result<()> {
        Ok(())
    }
    fn reserve(&mut self, _: EventId) -> Result<bool> {
        Ok(true)
    }
    fn receipt(&self, _: EventId) -> Result<Option<DeliveryState>> {
        Ok(None)
    }
    fn intent(&mut self, _: EventId) -> Result<()> {
        self.intent = true;
        Ok(())
    }
    fn record(
        &mut self,
        event: EventId,
        peer: &str,
        direction: &str,
        state: DeliveryState,
        p: &Payload,
        detail: &str,
    ) -> Result<i64> {
        if direction == "receive" && state == DeliveryState::Applied {
            assert!(self.intent)
        }
        let mut rows = self.rows.lock().unwrap();
        let id = rows.len() as i64 + 1;
        rows.push(HistoryEntry {
            id,
            event,
            peer: peer.into(),
            direction: direction.into(),
            state,
            format: p.meta.format,
            bytes: p.meta.size,
            time: 0,
            available: true,
            detail: detail.into(),
        });
        Ok(id)
    }
    fn update(&mut self, _: EventId, _: &str, _: DeliveryState, _: &str) -> Result<()> {
        Ok(())
    }
    fn pending_receipts(&self) -> Result<Vec<(EventId, String)>> {
        Ok(vec![])
    }
    fn history(&self, _: usize, _: usize) -> Result<Vec<HistoryEntry>> {
        Ok(vec![])
    }
    fn content(&self, _: i64) -> Result<Payload> {
        Ok(payload(7))
    }
    fn clear(&mut self) -> Result<()> {
        Ok(())
    }
    fn prune(&mut self) -> Result<()> {
        Ok(())
    }
}
struct PlatformStub;
impl Platform for PlatformStub {
    fn notify(&mut self, _: &str, _: &str) {}
    fn autostart(&mut self, _: Option<bool>) -> Result<String> {
        Ok("unavailable".into())
    }
}
struct Harness {
    app: Service,
    clipboard: Arc<Mutex<ClipboardValue>>,
    network: Arc<Mutex<NetState>>,
    fail: Arc<Mutex<bool>>,
    rows: Arc<Mutex<Vec<HistoryEntry>>>,
}
impl Harness {
    fn new() -> Self {
        let clipboard = Arc::new(Mutex::new(value(0)));
        let network = Arc::new(Mutex::new(NetState::default()));
        let fail = Arc::new(Mutex::new(false));
        let mut settings = Settings::default();
        for id in ["02".repeat(32), "03".repeat(32)] {
            settings.peers.insert(
                id,
                PeerPolicy {
                    send: true,
                    ..PeerPolicy::default()
                },
            );
        }
        let rows = Arc::new(Mutex::new(Vec::new()));
        let app = Service::new(
            [1; 32],
            [1; 16],
            Box::new(Clip(clipboard.clone())),
            Box::new(MemoryStore {
                settings,
                fail: fail.clone(),
                intent: false,
                rows: rows.clone(),
            }),
            Box::new(Net(network.clone())),
            Box::new(PlatformStub),
        )
        .unwrap();
        Self {
            app,
            clipboard,
            network,
            fail,
            rows,
        }
    }
    fn command(&mut self, action: Action) -> Answer {
        self.app.request(ControlRequest {
            version: VERSION,
            action,
        })
    }
    fn settings(&mut self) -> Settings {
        match self.command(Action::Settings) {
            Answer::Settings { settings } => settings,
            _ => panic!(),
        }
    }
}
#[test]
fn remote_apply_in_three_device_topology_never_relays() {
    let mut h = Harness::new();
    let (tx, rx) = mpsc::sync_channel(1);
    h.network
        .lock()
        .unwrap()
        .input
        .push_back(NetworkEvent::Offer {
            peer: [2; 32],
            event: EventId {
                origin: [2; 32],
                epoch: [2; 16],
                seq: 1,
            },
            meta: payload(5).meta,
            reply: tx,
        });
    h.app.tick();
    let ticket = rx.recv().unwrap().unwrap();
    let (tx, rx) = mpsc::sync_channel(1);
    h.network
        .lock()
        .unwrap()
        .input
        .push_back(NetworkEvent::Received {
            ticket,
            payload: payload(5),
            reply: tx,
        });
    h.app.tick();
    rx.recv().unwrap().unwrap();
    h.app.tick();
    assert_eq!(h.clipboard.lock().unwrap().stamp.digest, [5; 32]);
    assert!(h.network.lock().unwrap().sent.is_empty());
    assert!(
        h.rows
            .lock()
            .unwrap()
            .iter()
            .all(|r| r.direction == "receive" && r.peer == "02".repeat(32))
    );
    *h.clipboard.lock().unwrap() = value(6);
    h.app.tick();
    assert_eq!(h.network.lock().unwrap().sent.len(), 2);
}
#[test]
fn permission_changed_between_offer_and_completion_prevents_write() {
    let mut h = Harness::new();
    let (tx, rx) = mpsc::sync_channel(1);
    h.network
        .lock()
        .unwrap()
        .input
        .push_back(NetworkEvent::Offer {
            peer: [2; 32],
            event: EventId {
                origin: [2; 32],
                epoch: [2; 16],
                seq: 1,
            },
            meta: payload(5).meta,
            reply: tx,
        });
    h.app.tick();
    let ticket = rx.recv().unwrap().unwrap();
    let mut settings = h.settings();
    settings.receive = false;
    h.command(Action::Configure { settings });
    let (tx, rx) = mpsc::sync_channel(1);
    h.network
        .lock()
        .unwrap()
        .input
        .push_back(NetworkEvent::Received {
            ticket,
            payload: payload(5),
            reply: tx,
        });
    h.app.tick();
    assert!(rx.recv().unwrap().is_err());
    assert_eq!(h.clipboard.lock().unwrap().stamp.digest, [0; 32]);
}
#[test]
fn failed_persistence_closes_directions_and_preserves_peer_settings() {
    let mut h = Harness::new();
    let mut settings = h.settings();
    settings.notifications = false;
    *h.fail.lock().unwrap() = true;
    assert!(matches!(
        h.command(Action::Configure {
            settings: settings.clone()
        }),
        Answer::Error { .. }
    ));
    let actual = h.settings();
    assert!(!actual.send && !actual.receive);
    assert_eq!(actual.peers, settings.peers);
    assert!(matches!(h.command(Action::Send), Answer::Error { .. }));
    assert!(h.network.lock().unwrap().sent.is_empty());
}
#[test]
fn local_only_history_copy_and_notifications_do_not_publish() {
    let mut h = Harness::new();
    assert!(matches!(
        h.command(Action::Copy {
            id: 1,
            local_only: true
        }),
        Answer::Done { .. }
    ));
    h.app.tick();
    assert!(h.network.lock().unwrap().sent.is_empty());
    let mut settings = h.settings();
    settings.notifications = false;
    h.command(Action::Configure { settings });
    h.app.tick();
    assert!(h.network.lock().unwrap().sent.is_empty());
    h.command(Action::Resend { id: 1 });
    assert_eq!(h.network.lock().unwrap().sent.len(), 2);
}

#[test]
fn receipt_queries_never_expose_another_peers_events() {
    let mut h = Harness::new();
    let (tx, rx) = mpsc::sync_channel(1);
    h.network
        .lock()
        .unwrap()
        .input
        .push_back(NetworkEvent::ReceiptQuery {
            peer: [3; 32],
            event: EventId {
                origin: [2; 32],
                epoch: [2; 16],
                seq: 1,
            },
            reply: tx,
        });
    h.app.tick();
    assert!(rx.recv().unwrap().is_err());
    assert!(h.network.lock().unwrap().sent.is_empty());
}

#[test]
fn direction_patches_preserve_other_preferences_and_invalidate_old_policy() {
    let mut h = Harness::new();
    let mut expected = h.settings();
    expected.history_limit = 7;
    expected.notifications = false;
    assert!(!matches!(
        h.command(Action::Configure {
            settings: expected.clone()
        }),
        Answer::Error { .. }
    ));
    let previous = h.network.lock().unwrap().revision;
    h.command(Action::SetDirections {
        send: Some(false),
        receive: None,
    });
    h.command(Action::SetDirections {
        send: None,
        receive: Some(false),
    });
    expected.send = false;
    expected.receive = false;
    assert_eq!(h.settings(), expected);
    assert!(h.network.lock().unwrap().revision > previous);
    *h.clipboard.lock().unwrap() = value(8);
    h.app.tick();
    assert!(h.network.lock().unwrap().sent.is_empty());
    assert!(matches!(h.command(Action::Send), Answer::Error { .. }));
    h.command(Action::SetDirections {
        send: Some(true),
        receive: Some(true),
    });
    h.app.tick();
    assert!(
        h.network.lock().unwrap().sent.is_empty(),
        "enabling must not publish existing clipboard"
    );
    *h.clipboard.lock().unwrap() = value(9);
    h.app.tick();
    assert_eq!(h.network.lock().unwrap().sent.len(), 2);
}

#[test]
fn empty_direction_patch_is_rejected_and_storage_failure_stops_sync() {
    let mut h = Harness::new();
    let original = h.settings();
    let revision = h.network.lock().unwrap().revision;
    assert!(matches!(
        h.command(Action::SetDirections {
            send: None,
            receive: None
        }),
        Answer::Error { .. }
    ));
    assert_eq!(h.settings(), original);
    assert_eq!(h.network.lock().unwrap().revision, revision);
    *h.fail.lock().unwrap() = true;
    assert!(matches!(
        h.command(Action::SetDirections {
            send: Some(false),
            receive: None
        }),
        Answer::Error { .. }
    ));
    assert_eq!(
        h.settings(),
        Settings {
            send: false,
            receive: false,
            ..original
        }
    );
    assert!(h.network.lock().unwrap().revision > revision);
}

#[test]
fn ordinary_copies_are_recorded_in_manual_disabled_and_no_target_modes() {
    for mode in 0..3 {
        let mut h = Harness::new();
        let mut settings = h.settings();
        match mode {
            0 => settings.automatic = false,
            1 => settings.send = false,
            _ => settings.peers.clear(),
        }
        h.command(Action::Configure { settings });
        assert!(h.rows.lock().unwrap().is_empty());
        *h.clipboard.lock().unwrap() = value(8);
        h.app.tick();
        h.app.tick();
        let rows = h.rows.lock().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].direction, "local");
        assert_eq!(rows[0].peer, "01".repeat(32));
        assert!(h.network.lock().unwrap().sent.is_empty());
        drop(rows);
        let Answer::Status { status } = h.command(Action::Status) else {
            panic!()
        };
        assert!(status.sequence >= 2); // settings + local history refresh
    }
}

#[test]
fn automatic_copy_and_all_target_results_share_the_local_event() {
    let mut h = Harness::new();
    *h.clipboard.lock().unwrap() = value(8);
    h.app.tick();
    let rows = h.rows.lock().unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].direction, "local");
    assert!(rows.iter().all(|r| r.event == rows[0].event));
    assert!(rows[1..].iter().all(|r| r.direction == "send"));
}
