use super::service::Service;
use shuttli_api::control::*;
use shuttli_core::sync::{Publication, WriteAuthorization};
use shuttli_model::sync::*;
use shuttli_ports::sync::*;
use shuttli_runtime::{block_on, worker};
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
type Gate = Arc<Mutex<Option<(mpsc::SyncSender<()>, mpsc::Receiver<()>)>>>;
fn wait_gate(gate: &Gate) {
    let wait = gate.lock().unwrap().take();
    if let Some((entered, release)) = wait {
        entered.send(()).unwrap();
        release
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
    }
}
fn install_gate(gate: &Gate) -> (mpsc::Receiver<()>, mpsc::SyncSender<()>) {
    let (entered, ready) = mpsc::sync_channel(1);
    let (release, wait) = mpsc::sync_channel(1);
    *gate.lock().unwrap() = Some((entered, wait));
    (ready, release)
}
struct Clip(Arc<Mutex<ClipboardValue>>, Gate);
impl Clipboard for Clip {
    fn read(&mut self) -> Result<ClipboardValue> {
        wait_gate(&self.1);
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
    gate: Gate,
    fail: Arc<Mutex<bool>>,
    intent: bool,
    rows: Arc<Mutex<Vec<HistoryEntry>>>,
}
impl Store for MemoryStore {
    fn settings(&self) -> Result<Settings> {
        Ok(self.settings.clone())
    }
    fn save_settings(&mut self, s: &Settings) -> Result<()> {
        wait_gate(&self.gate);
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
    fn autostart(&mut self, _: Option<bool>) -> Result<AutostartStatus> {
        Ok(AutostartStatus {
            state: AutostartState::Unavailable,
            message: "unavailable".into(),
        })
    }
}
struct Harness {
    app: Service,
    clip_gate: Gate,
    store_gate: Gate,
    clipboard: Arc<Mutex<ClipboardValue>>,
    network: Arc<Mutex<NetState>>,
    fail: Arc<Mutex<bool>>,
    rows: Arc<Mutex<Vec<HistoryEntry>>>,
}
impl Harness {
    fn new() -> Self {
        let clipboard = Arc::new(Mutex::new(value(0)));
        let clip_gate = Gate::default();
        let store_gate = Gate::default();
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
        let app = block_on(Service::new(
            [1; 32],
            [1; 16],
            worker(
                "test-clipboard",
                Box::new(Clip(clipboard.clone(), clip_gate.clone())) as Box<dyn Clipboard>,
            )
            .unwrap(),
            worker(
                "test-store",
                Box::new(MemoryStore {
                    settings,
                    gate: store_gate.clone(),
                    fail: fail.clone(),
                    intent: false,
                    rows: rows.clone(),
                }) as Box<dyn Store>,
            )
            .unwrap(),
            Box::new(Net(network.clone())),
            worker("test-platform", Box::new(PlatformStub) as Box<dyn Platform>).unwrap(),
        ))
        .unwrap();
        Self {
            app,
            clip_gate,
            store_gate,
            clipboard,
            network,
            fail,
            rows,
        }
    }
    fn command(&self, action: Action) -> Answer {
        block_on(self.app.request(ControlRequest {
            version: VERSION,
            action,
        }))
    }
    fn configure(&self, settings: Settings) -> Answer {
        let expected = self.settings();
        self.command(Action::Configure { expected, settings })
    }
    fn settings(&self) -> Settings {
        match self.command(Action::Settings) {
            Answer::Settings { settings } => settings,
            _ => panic!(),
        }
    }
}
#[test]
fn remote_apply_in_three_device_topology_never_relays() {
    let h = Harness::new();
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
    block_on(h.app.tick());
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
    block_on(h.app.tick());
    rx.recv().unwrap().unwrap();
    block_on(h.app.tick());
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
    block_on(h.app.tick());
    assert_eq!(h.network.lock().unwrap().sent.len(), 2);
}
#[test]
fn permission_changed_between_offer_and_completion_prevents_write() {
    let h = Harness::new();
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
    block_on(h.app.tick());
    let ticket = rx.recv().unwrap().unwrap();
    let mut settings = h.settings();
    settings.receive = false;
    h.configure(settings);
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
    block_on(h.app.tick());
    assert!(rx.recv().unwrap().is_err());
    assert_eq!(h.clipboard.lock().unwrap().stamp.digest, [0; 32]);
}
#[test]
fn failed_persistence_closes_directions_and_preserves_peer_settings() {
    let h = Harness::new();
    let mut settings = h.settings();
    settings.notifications = false;
    *h.fail.lock().unwrap() = true;
    assert!(matches!(
        h.configure(settings.clone()),
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
    let h = Harness::new();
    assert!(matches!(
        h.command(Action::Copy {
            id: 1,
            local_only: true
        }),
        Answer::Done { .. }
    ));
    block_on(h.app.tick());
    assert!(h.network.lock().unwrap().sent.is_empty());
    let mut settings = h.settings();
    settings.notifications = false;
    h.configure(settings);
    block_on(h.app.tick());
    assert!(h.network.lock().unwrap().sent.is_empty());
    h.command(Action::Resend { id: 1 });
    assert_eq!(h.network.lock().unwrap().sent.len(), 2);
}

#[test]
fn receipt_queries_never_expose_another_peers_events() {
    let h = Harness::new();
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
    block_on(h.app.tick());
    assert!(rx.recv().unwrap().is_err());
    assert!(h.network.lock().unwrap().sent.is_empty());
}

#[test]
fn direction_patches_preserve_other_preferences_and_invalidate_old_policy() {
    let h = Harness::new();
    let mut expected = h.settings();
    expected.history_limit = 7;
    expected.notifications = false;
    assert!(!matches!(
        h.configure(expected.clone()),
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
    block_on(h.app.tick());
    assert!(h.network.lock().unwrap().sent.is_empty());
    assert!(matches!(h.command(Action::Send), Answer::Error { .. }));
    h.command(Action::SetDirections {
        send: Some(true),
        receive: Some(true),
    });
    block_on(h.app.tick());
    assert!(
        h.network.lock().unwrap().sent.is_empty(),
        "enabling must not publish existing clipboard"
    );
    *h.clipboard.lock().unwrap() = value(9);
    block_on(h.app.tick());
    assert_eq!(h.network.lock().unwrap().sent.len(), 2);
}

#[test]
fn empty_direction_patch_is_rejected_and_storage_failure_stops_sync() {
    let h = Harness::new();
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
        let h = Harness::new();
        let mut settings = h.settings();
        match mode {
            0 => settings.automatic = false,
            1 => settings.send = false,
            _ => settings.peers.clear(),
        }
        h.configure(settings);
        assert!(h.rows.lock().unwrap().is_empty());
        *h.clipboard.lock().unwrap() = value(8);
        block_on(h.app.tick());
        block_on(h.app.tick());
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
    let h = Harness::new();
    *h.clipboard.lock().unwrap() = value(8);
    block_on(h.app.tick());
    let rows = h.rows.lock().unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].direction, "local");
    assert!(rows.iter().all(|r| r.event == rows[0].event));
    assert!(rows[1..].iter().all(|r| r.direction == "send"));
}

#[test]
fn stale_settings_cannot_restore_disabled_sending() {
    let h = Harness::new();
    let old = h.settings();
    h.command(Action::SetDirections {
        send: Some(false),
        receive: None,
    });
    let mut edited = old.clone();
    edited.history_limit = 7;
    assert!(matches!(
        h.command(Action::Configure {
            expected: old,
            settings: edited
        }),
        Answer::Error { .. }
    ));
    assert!(!h.settings().send);
    assert_eq!(h.settings().history_limit, 20);
}
#[test]
fn stale_peer_policy_cannot_restore_revoked_directions() {
    let h = Harness::new();
    let id = "02".repeat(32);
    let old = h.settings().peers[&id].clone();
    assert!(matches!(
        h.command(Action::Peer {
            id: id.clone(),
            expected: old.clone(),
            policy: PeerPolicy {
                send: false,
                receive: false,
                ..old.clone()
            }
        }),
        Answer::Settings { .. }
    ));
    assert!(matches!(
        h.command(Action::Peer {
            id: id.clone(),
            expected: old.clone(),
            policy: PeerPolicy { quiet: true, ..old }
        }),
        Answer::Error { .. }
    ));
    assert!(!h.settings().peers[&id].send);
    assert!(!h.settings().peers[&id].receive);
}
fn request(action: Action) -> ControlRequest {
    ControlRequest {
        version: VERSION,
        action,
    }
}
fn finish(tasks: &mut Vec<shuttli_runtime::Task<'_>>) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while !tasks.is_empty() {
        assert!(
            std::time::Instant::now() < deadline,
            "effects did not complete"
        );
        shuttli_runtime::poll_tasks(tasks);
        if !tasks.is_empty() {
            std::thread::park_timeout(std::time::Duration::from_millis(1));
        }
    }
}
#[test]
fn slow_clipboard_does_not_block_status_or_revocation_and_old_send_is_cancelled() {
    let h = Harness::new();
    *h.clipboard.lock().unwrap() = value(8);
    let (entered, release) = install_gate(&h.clip_gate);
    let revision = h.network.lock().unwrap().revision;
    let mut tasks: Vec<shuttli_runtime::Task<'_>> = vec![Box::pin(async {
        assert!(matches!(
            h.app.request(request(Action::Send)).await,
            Answer::Error { .. }
        ));
    })];
    shuttli_runtime::poll_tasks(&mut tasks);
    entered
        .recv_timeout(std::time::Duration::from_secs(1))
        .unwrap();
    tasks.push(Box::pin(async {
        assert!(matches!(
            h.app
                .request(request(Action::SetDirections {
                    send: Some(false),
                    receive: None
                }))
                .await,
            Answer::Settings { .. }
        ));
    }));
    shuttli_runtime::poll_tasks(&mut tasks);
    let Answer::Status { status } = h.command(Action::Status) else {
        panic!()
    };
    assert!(!status.settings.send);
    assert!(h.network.lock().unwrap().revision > revision);
    assert!(h.network.lock().unwrap().sent.is_empty());
    release.send(()).unwrap();
    finish(&mut tasks);
    assert!(h.network.lock().unwrap().sent.is_empty());
}
#[test]
fn newer_pause_supersedes_enable_waiting_for_slow_storage() {
    let h = Harness::new();
    h.command(Action::SetDirections {
        send: Some(false),
        receive: None,
    });
    let (entered, release) = install_gate(&h.store_gate);
    let mut tasks: Vec<shuttli_runtime::Task<'_>> = vec![Box::pin(async {
        assert!(matches!(
            h.app
                .request(request(Action::SetDirections {
                    send: Some(true),
                    receive: None
                }))
                .await,
            Answer::Error { .. }
        ));
    })];
    shuttli_runtime::poll_tasks(&mut tasks);
    entered
        .recv_timeout(std::time::Duration::from_secs(1))
        .unwrap();
    tasks.push(Box::pin(async {
        assert!(matches!(
            h.app
                .request(request(Action::SetDirections {
                    send: Some(false),
                    receive: None
                }))
                .await,
            Answer::Settings { .. }
        ));
    }));
    shuttli_runtime::poll_tasks(&mut tasks);
    assert!(!h.settings().send);
    release.send(()).unwrap();
    finish(&mut tasks);
    assert!(!h.settings().send);
    assert!(h.network.lock().unwrap().sent.is_empty());
}
#[test]
fn old_api_version_is_rejected_and_autostart_is_typed() {
    let h = Harness::new();
    assert!(matches!(
        block_on(h.app.request(ControlRequest {
            version: VERSION - 1,
            action: Action::Send
        })),
        Answer::Error { .. }
    ));
    assert!(h.network.lock().unwrap().sent.is_empty());
    assert!(matches!(
        h.command(Action::Autostart { enabled: None }),
        Answer::Autostart {
            status: AutostartStatus {
                state: AutostartState::Unavailable,
                ..
            }
        }
    ));
}

#[test]
fn quit_revokes_pending_work_and_rejects_new_operations() {
    let h = Harness::new();
    let revision = h.network.lock().unwrap().revision;
    let (entered, release) = install_gate(&h.clip_gate);
    let mut tasks: Vec<shuttli_runtime::Task<'_>> = vec![Box::pin(async {
        assert!(matches!(
            h.app.request(request(Action::Send)).await,
            Answer::Error { .. }
        ));
    })];
    shuttli_runtime::poll_tasks(&mut tasks);
    entered
        .recv_timeout(std::time::Duration::from_secs(1))
        .unwrap();
    assert!(matches!(h.command(Action::Quit), Answer::Done { .. }));
    assert!(h.app.stopping());
    assert!(h.network.lock().unwrap().revision > revision);
    assert!(matches!(h.command(Action::Send), Answer::Error { .. }));
    release.send(()).unwrap();
    finish(&mut tasks);
    assert!(h.network.lock().unwrap().sent.is_empty());
}
