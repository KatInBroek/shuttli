use crate::{files::atomic_write, recent::RecentHistory};
use rusqlite::{Connection, OptionalExtension, params};
use shuttli_model::sync::*;
use shuttli_ports::sync::{Payload, Result, Store};
use std::path::{Path, PathBuf};
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
fn key(event: EventId) -> String {
    format!(
        "{}:{}:{}",
        hex::encode(event.origin),
        hex::encode(event.epoch),
        event.seq
    )
}
fn encoded<T: serde::Serialize>(v: &T) -> Result<String> {
    serde_json::to_string(v).map_err(err)
}
fn parsed<T: serde::de::DeserializeOwned>(v: &str) -> Result<T> {
    serde_json::from_str(v).map_err(err)
}

// Compatibility is confined to persisted data, never exposed as an API feature.
fn decode_settings(bytes: &[u8]) -> Result<(Settings, bool)> {
    let mut value: serde_json::Value = serde_json::from_slice(bytes).map_err(err)?;
    let mut migrated = false;
    if let Some(peers) = value
        .get_mut("peers")
        .and_then(serde_json::Value::as_object_mut)
    {
        for policy in peers.values_mut() {
            if let Some(fields) = policy.as_object_mut() {
                if let Some(blocked) = fields.remove("blocked") {
                    migrated = true;
                    match blocked.as_bool() {
                        Some(true) => {
                            fields.insert("send".into(), false.into());
                            fields.insert("receive".into(), false.into());
                        }
                        Some(false) => {}
                        None => return Err("invalid legacy device policy".into()),
                    }
                }
            }
        }
    }
    let settings: Settings = serde_json::from_value(value).map_err(err)?;
    if !settings.validate() {
        return Err("invalid settings".into());
    }
    Ok((settings, migrated))
}

pub struct SqlStore {
    db: Connection,
    dir: PathBuf,
    settings: Settings,
    recent: RecentHistory,
}
impl SqlStore {
    pub fn open(dir: &Path) -> Result<Self> {
        let path = dir.join("state.sqlite3");
        let db = Connection::open(&path).map_err(err)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).map_err(err)?;
        }
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA busy_timeout=3000; PRAGMA secure_delete=ON;
CREATE TABLE IF NOT EXISTS settings_guard(id INTEGER PRIMARY KEY CHECK(id=1),pending INTEGER NOT NULL,secret_initialized INTEGER NOT NULL);
INSERT OR IGNORE INTO settings_guard VALUES(1,0,0);
CREATE TABLE IF NOT EXISTS peers(id TEXT PRIMARY KEY,info TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS replay(origin TEXT NOT NULL,epoch TEXT NOT NULL,seq INTEGER NOT NULL,PRIMARY KEY(origin,epoch));
CREATE TABLE IF NOT EXISTS receipts(event TEXT PRIMARY KEY,state TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS outbox(event_key TEXT NOT NULL,event TEXT NOT NULL,peer TEXT NOT NULL,state TEXT NOT NULL,PRIMARY KEY(event_key,peer));
UPDATE receipts SET state='unknown' WHERE state IN ('receiving','intent');").map_err(err)?;
        let legacy: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='history')",
                [],
                |r| r.get(0),
            )
            .map_err(err)?;
        if legacy {
            // Retain only recovery IDs, never old history metadata or bodies.
            db.execute_batch(r#"BEGIN IMMEDIATE;
INSERT OR IGNORE INTO outbox SELECT event_key,event,peer,state FROM history WHERE direction='send' AND state IN ('"sending"','"unknown"') ORDER BY id DESC LIMIT 16;
DROP TABLE history;
COMMIT;
PRAGMA wal_checkpoint(TRUNCATE);
VACUUM;
PRAGMA wal_checkpoint(TRUNCATE);"#).map_err(err)?;
        }
        db.execute(
            "UPDATE outbox SET state='\"unknown\"' WHERE state='\"sending\"'",
            [],
        )
        .map_err(err)?;
        let settings_path = dir.join("settings.json");
        let (mut settings, migrated) = match std::fs::read(settings_path) {
            Ok(bytes) => decode_settings(&bytes).unwrap_or_else(|_| (Settings::closed(), false)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Settings::default(), false),
            Err(_) => (Settings::closed(), false),
        };
        let pending: bool = db
            .query_row("SELECT pending FROM settings_guard WHERE id=1", [], |r| {
                r.get(0)
            })
            .map_err(err)?;
        if pending {
            settings.send = false;
            settings.receive = false;
        }
        let recent = RecentHistory::open(dir.join("history-images"))?;
        let mut store = Self {
            db,
            dir: dir.into(),
            settings,
            recent,
        };
        for mut peer in store.peers()? {
            peer.online = false;
            store.peer(&peer)?;
        }
        store.prune()?;
        if migrated {
            store.save_settings(&store.settings.clone())?;
        }
        Ok(store)
    }
}
impl Store for SqlStore {
    fn settings(&self) -> Result<Settings> {
        Ok(self.settings.clone())
    }
    fn save_settings(&mut self, settings: &Settings) -> Result<()> {
        if !settings.validate() {
            return Err("invalid settings".into());
        }
        self.db
            .execute("UPDATE settings_guard SET pending=1 WHERE id=1", [])
            .map_err(err)?;
        atomic_write(
            &self.dir.join("settings.json"),
            &serde_json::to_vec_pretty(settings).map_err(err)?,
        )?;
        self.settings = settings.clone();
        self.prune()?;
        self.db
            .execute("UPDATE settings_guard SET pending=0 WHERE id=1", [])
            .map_err(err)?;
        Ok(())
    }
    fn peers(&self) -> Result<Vec<PeerInfo>> {
        let mut q = self
            .db
            .prepare("SELECT info FROM peers ORDER BY id LIMIT 256")
            .map_err(err)?;
        let rows = q.query_map([], |r| r.get::<_, String>(0)).map_err(err)?;
        rows.map(|r| parsed(&r.map_err(err)?)).collect()
    }
    fn peer(&mut self, p: &PeerInfo) -> Result<()> {
        let n: i64 = self
            .db
            .query_row("SELECT COUNT(*) FROM peers", [], |r| r.get(0))
            .map_err(err)?;
        if n >= 256 && !self.peers()?.iter().any(|v| v.id == p.id) {
            return Err("device capacity reached".into());
        }
        self.db.execute("INSERT INTO peers(id,info) VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET info=excluded.info",params![p.id,encoded(p)?]).map_err(err)?;
        Ok(())
    }
    fn reserve(&mut self, e: EventId) -> Result<bool> {
        if e.seq > i64::MAX as u64 {
            return Err("sequence exhausted".into());
        }
        let tx = self.db.transaction().map_err(err)?;
        let origin = hex::encode(e.origin);
        let epoch = hex::encode(e.epoch);
        let old: Option<u64> = tx
            .query_row(
                "SELECT seq FROM replay WHERE origin=?1 AND epoch=?2",
                params![origin, epoch],
                |r| r.get(0),
            )
            .optional()
            .map_err(err)?;
        if old.is_some_and(|n| n >= e.seq) {
            return Ok(false);
        }
        if old.is_none() {
            let n: i64 = tx
                .query_row("SELECT COUNT(*) FROM replay", [], |r| r.get(0))
                .map_err(err)?;
            if n >= 4096 {
                return Err("replay identity/epoch capacity reached".into());
            }
        }
        tx.execute("INSERT INTO replay(origin,epoch,seq) VALUES(?1,?2,?3) ON CONFLICT(origin,epoch) DO UPDATE SET seq=excluded.seq",params![origin,epoch,e.seq]).map_err(err)?;
        tx.execute(
            "INSERT INTO receipts(event,state) VALUES(?1,'receiving')",
            [key(e)],
        )
        .map_err(err)?;
        tx.execute("DELETE FROM receipts WHERE rowid NOT IN (SELECT rowid FROM receipts ORDER BY rowid DESC LIMIT 4096)",[]).map_err(err)?;
        tx.commit().map_err(err)?;
        Ok(true)
    }
    fn receipt(&self, event: EventId) -> Result<Option<DeliveryState>> {
        let state: Option<String> = self
            .db
            .query_row(
                "SELECT state FROM receipts WHERE event=?1",
                [key(event)],
                |r| r.get(0),
            )
            .optional()
            .map_err(err)?;
        Ok(state.map(|s| {
            if s == "applied" {
                DeliveryState::Applied
            } else {
                DeliveryState::Unknown
            }
        }))
    }
    fn intent(&mut self, e: EventId) -> Result<()> {
        let n = self
            .db
            .execute(
                "UPDATE receipts SET state='intent' WHERE event=?1 AND state='receiving'",
                [key(e)],
            )
            .map_err(err)?;
        if n != 1 {
            return Err("missing reservation".into());
        }
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
            let count = self
                .db
                .execute(
                    "UPDATE receipts SET state='applied' WHERE event=?1 AND state='intent'",
                    [key(event)],
                )
                .map_err(err)?;
            if count != 1 {
                return Err("missing durable apply intent".into());
            }
        }
        if direction == "send" {
            self.db.execute("INSERT INTO outbox(event_key,event,peer,state) VALUES(?1,?2,?3,?4) ON CONFLICT(event_key,peer) DO UPDATE SET state=excluded.state", params![key(event),encoded(&event)?,peer,encoded(&state)?]).map_err(err)?;
            self.db.execute("DELETE FROM outbox WHERE rowid NOT IN (SELECT rowid FROM outbox ORDER BY rowid DESC LIMIT 4096)", []).map_err(err)?;
        }
        self.recent.record(
            &self.settings,
            HistoryEntry {
                id: 0,
                event,
                peer: peer.into(),
                direction: direction.into(),
                state,
                format: p.meta.format,
                bytes: p.meta.size,
                time: 0,
                available: false,
                detail: detail.into(),
            },
            p,
        )
    }
    fn update(
        &mut self,
        event: EventId,
        peer: &str,
        state: DeliveryState,
        detail: &str,
    ) -> Result<()> {
        self.db
            .execute(
                "UPDATE outbox SET state=?1 WHERE event_key=?2 AND peer=?3",
                params![encoded(&state)?, key(event), peer],
            )
            .map_err(err)?;
        self.recent.update(event, peer, state, detail);
        Ok(())
    }
    fn pending_receipts(&self) -> Result<Vec<(EventId, String)>> {
        let mut statement = self.db.prepare("SELECT event,peer FROM outbox WHERE state='\"unknown\"' ORDER BY rowid DESC LIMIT 16").map_err(err)?;
        let rows = statement
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(err)?;
        rows.map(|r| {
            let (event, peer) = r.map_err(err)?;
            Ok((parsed(&event)?, peer))
        })
        .collect()
    }
    fn history(&self, offset: usize, limit: usize) -> Result<Vec<HistoryEntry>> {
        Ok(self.recent.history(offset, limit))
    }
    fn content(&self, id: i64) -> Result<Payload> {
        self.recent.content(id)
    }
    fn local_history(
        &self,
        offset: usize,
        limit: usize,
    ) -> Result<(u64, Vec<shuttli_model::mobile::HistorySummary>)> {
        Ok(self.recent.local_history(offset, limit))
    }
    fn local_content(&self, event: EventId) -> Result<Payload> {
        self.recent.local_content(event)
    }
    fn clear(&mut self) -> Result<()> {
        self.recent.clear()
    }
    fn prune(&mut self) -> Result<()> {
        self.recent.prune(&self.settings)
    }
}
#[cfg(test)]
use crate::content::payload;
#[cfg(test)]
mod tests {
    use super::*;
    fn temp() -> PathBuf {
        let mut b = [0; 8];
        getrandom::getrandom(&mut b).unwrap();
        let p = std::env::temp_dir().join(format!("shuttli-store-{}", hex::encode(b)));
        std::fs::create_dir(&p).unwrap();
        p
    }
    fn event(seq: u64) -> EventId {
        EventId {
            origin: [1; 32],
            epoch: [2; 16],
            seq,
        }
    }
    #[test]
    fn fresh_profile_defaults_to_automatic_and_explicit_manual_survives_restart() {
        let dir = temp();
        {
            let mut store = SqlStore::open(&dir).unwrap();
            let mut settings = store.settings().unwrap();
            assert!(settings.automatic && settings.send && settings.receive);
            assert!(settings.peers.is_empty());
            assert!(!PeerPolicy::default().send);
            settings.automatic = false;
            store.save_settings(&settings).unwrap();
        }
        {
            let store = SqlStore::open(&dir).unwrap();
            assert!(!store.settings().unwrap().automatic);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn replay_survives_restart_and_history_clear() {
        let dir = temp();
        {
            let mut s = SqlStore::open(&dir).unwrap();
            assert!(s.reserve(event(2)).unwrap());
            assert!(!s.reserve(event(1)).unwrap());
            s.clear().unwrap();
        }
        let mut s = SqlStore::open(&dir).unwrap();
        assert!(!s.reserve(event(2)).unwrap());
        assert!(s.reserve(event(3)).unwrap());
        drop(s);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn interrupted_settings_save_closes_directions_on_restart_and_preserves_peers() {
        let dir = temp();
        let mut settings = Settings::default();
        settings.peers.insert(
            hex::encode([9; 32]),
            PeerPolicy {
                send: true,
                ..PeerPolicy::default()
            },
        );
        {
            let mut store = SqlStore::open(&dir).unwrap();
            store.save_settings(&settings).unwrap();
            std::fs::rename(dir.join("settings.json"), dir.join("previous.json")).unwrap();
            std::fs::create_dir(dir.join("settings.json")).unwrap();
            assert!(store.save_settings(&settings).is_err());
            std::fs::remove_dir(dir.join("settings.json")).unwrap();
            std::fs::rename(dir.join("previous.json"), dir.join("settings.json")).unwrap();
        }
        let mut store = SqlStore::open(&dir).unwrap();
        let recovered = store.settings().unwrap();
        assert!(!recovered.send && !recovered.receive);
        assert_eq!(recovered.peers, settings.peers);
        store.save_settings(&settings).unwrap();
        drop(store);
        let store = SqlStore::open(&dir).unwrap();
        assert!(store.settings().unwrap().send && store.settings().unwrap().receive);
        drop(store);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn malformed_settings_fail_closed() {
        let dir = temp();
        std::fs::write(dir.join("settings.json"), b"broken").unwrap();
        let s = SqlStore::open(&dir).unwrap();
        assert!(!s.settings().unwrap().send);
        assert!(!s.settings().unwrap().receive);
        drop(s);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(test)]
mod recovery_tests {
    use super::*;
    #[test]
    fn durable_receipt_outlives_history_and_does_not_require_content() {
        let mut random = [0; 8];
        getrandom::getrandom(&mut random).unwrap();
        let dir = std::env::temp_dir().join(format!("shuttli-receipt-{}", hex::encode(random)));
        std::fs::create_dir(&dir).unwrap();
        let event = EventId {
            origin: [1; 32],
            epoch: [2; 16],
            seq: 1,
        };
        {
            let mut store = SqlStore::open(&dir).unwrap();
            assert!(store.reserve(event).unwrap());
            store.intent(event).unwrap();
            let p = payload(Format::Text, b"fixture".to_vec()).unwrap();
            store
                .record(event, "peer", "receive", DeliveryState::Applied, &p, "")
                .unwrap();
            store.clear().unwrap();
            assert_eq!(store.receipt(event).unwrap(), Some(DeliveryState::Applied));
        }
        let mut store = SqlStore::open(&dir).unwrap();
        assert_eq!(store.receipt(event).unwrap(), Some(DeliveryState::Applied));
        assert!(!store.reserve(event).unwrap());
        drop(store);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn failed_reservation_rolls_back_replay_high_water() {
        let mut random = [0; 8];
        getrandom::getrandom(&mut random).unwrap();
        let dir =
            std::env::temp_dir().join(format!("shuttli-failed-reserve-{}", hex::encode(random)));
        std::fs::create_dir(&dir).unwrap();
        let mut store = SqlStore::open(&dir).unwrap();
        let event = EventId {
            origin: [1; 32],
            epoch: [2; 16],
            seq: 1,
        };
        store.db.execute_batch("CREATE TRIGGER fail_receipt BEFORE INSERT ON receipts BEGIN SELECT RAISE(ABORT,'injected failure'); END;").unwrap();
        assert!(store.reserve(event).is_err());
        store.db.execute_batch("DROP TRIGGER fail_receipt").unwrap();
        assert!(store.reserve(event).unwrap());
        drop(store);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(test)]
mod session_history_tests {
    use super::*;
    fn dir() -> PathBuf {
        let mut bytes = [0; 8];
        getrandom::getrandom(&mut bytes).unwrap();
        let p = std::env::temp_dir().join(format!("shuttli-recent-{}", hex::encode(bytes)));
        std::fs::create_dir(&p).unwrap();
        p
    }
    fn event(seq: u64) -> EventId {
        EventId {
            origin: [3; 32],
            epoch: [4; 16],
            seq,
        }
    }
    fn image() -> Payload {
        let mut bytes = Vec::new();
        {
            let mut e = png::Encoder::new(&mut bytes, 2, 2);
            e.set_color(png::ColorType::Rgba);
            e.set_depth(png::BitDepth::Eight);
            e.write_header()
                .unwrap()
                .write_image_data(&[7; 16])
                .unwrap();
        }
        payload(Format::Png, bytes).unwrap()
    }
    fn record(s: &mut SqlStore, seq: u64, p: &Payload) -> i64 {
        s.record(
            event(seq),
            "peer",
            "send",
            DeliveryState::Sending,
            p,
            "fixture",
        )
        .unwrap()
    }
    fn files(p: &Path) -> Vec<PathBuf> {
        std::fs::read_dir(p.join("history-images"))
            .unwrap()
            .map(|x| x.unwrap().path())
            .collect()
    }
    #[test]
    fn text_and_list_are_memory_only_bounded_and_recovery_survives_clear_restart() {
        let dir = dir();
        let mut s = SqlStore::open(&dir).unwrap();
        let mut settings = s.settings().unwrap();
        settings.history_limit = 2;
        s.save_settings(&settings).unwrap();
        let p = payload(
            Format::Text,
            b"PRIVATE-SYNTHETIC-TEXT-MUST-STAY-IN-RAM".to_vec(),
        )
        .unwrap();
        let first = record(&mut s, 1, &p);
        record(&mut s, 2, &p);
        let last = record(&mut s, 3, &p);
        assert!(s.content(first).is_err());
        assert_eq!(s.content(last).unwrap().data, p.data);
        assert_eq!(s.history(0, 100).unwrap().len(), 2);
        assert!(files(&dir).is_empty());
        let table_count: u64 =
            s.db.query_row(
                "SELECT count(*) FROM sqlite_master WHERE name='history'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(table_count, 0);
        s.db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
            .unwrap();
        let db = std::fs::read(dir.join("state.sqlite3")).unwrap();
        assert!(!db.windows(p.data.len()).any(|w| w == p.data.as_ref()));
        s.clear().unwrap();
        assert!(s.history(0, 100).unwrap().is_empty());
        drop(s);
        let s = SqlStore::open(&dir).unwrap();
        assert!(s.history(0, 100).unwrap().is_empty());
        assert!(s.content(last).is_err());
        assert_eq!(s.pending_receipts().unwrap().len(), 3);
        drop(s);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn png_cache_is_encrypted_and_count_shrink_clear_remove_files() {
        let dir = dir();
        let mut s = SqlStore::open(&dir).unwrap();
        let p = image();
        let mut settings = s.settings().unwrap();
        settings.history_limit = 2;
        s.save_settings(&settings).unwrap();
        let first = record(&mut s, 1, &p);
        record(&mut s, 2, &p);
        let third = record(&mut s, 3, &p);
        assert!(s.content(first).is_err());
        assert_eq!(files(&dir).len(), 2);
        assert_eq!(s.content(third).unwrap().data, p.data);
        for file in files(&dir) {
            let raw = std::fs::read(file).unwrap();
            assert!(!raw.windows(p.data.len()).any(|w| w == p.data.as_ref()));
        }
        settings.history_limit = 1;
        s.save_settings(&settings).unwrap();
        assert_eq!(files(&dir).len(), 1);
        assert_eq!(s.history(0, 100).unwrap()[0].id, third);
        let file = files(&dir).pop().unwrap();
        let mut raw = std::fs::read(&file).unwrap();
        raw[24] ^= 1;
        std::fs::write(&file, raw).unwrap();
        assert!(s.content(third).is_err());
        s.clear().unwrap();
        assert!(files(&dir).is_empty());
        assert!(s.content(third).is_err());
        drop(s);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn budgets_and_peer_mode_prune_images_without_affecting_permissions() {
        let dir = dir();
        let mut s = SqlStore::open(&dir).unwrap();
        let png = image();
        let mut settings = s.settings().unwrap();
        settings.history_bytes = png.meta.size + 40;
        settings.history_memory_bytes = 5;
        s.save_settings(&settings).unwrap();
        record(&mut s, 1, &png);
        record(&mut s, 2, &png);
        assert_eq!(files(&dir).len(), 1);
        let text = payload(Format::Text, b"text".to_vec()).unwrap();
        record(&mut s, 3, &text);
        record(&mut s, 4, &text);
        assert_eq!(s.history(0, 100).unwrap().len(), 1);
        assert!(files(&dir).is_empty());
        let id = record(&mut s, 5, &png);
        settings.peers.insert(
            "peer".into(),
            PeerPolicy {
                history: Some(HistoryMode::Status),
                ..PeerPolicy::default()
            },
        );
        s.save_settings(&settings).unwrap();
        assert!(files(&dir).is_empty());
        assert!(s.content(id).is_err());
        assert!(s.settings().unwrap().send);
        settings.history = HistoryMode::Off;
        s.save_settings(&settings).unwrap();
        assert!(s.history(0, 100).unwrap().is_empty());
        drop(s);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn local_history_retains_one_png_for_fanout_and_evicts_whole_events() {
        let dir = dir();
        let mut s = SqlStore::open(&dir).unwrap();
        let p = image();
        let mut settings = s.settings().unwrap();
        settings.history_limit = 1;
        settings.history_bytes = p.meta.size + 40;
        s.save_settings(&settings).unwrap();
        let local = s
            .record(
                event(1),
                "self",
                "local",
                DeliveryState::Applied,
                &p,
                "local",
            )
            .unwrap();
        let sent = s
            .record(
                event(1),
                "peer",
                "send",
                DeliveryState::Sending,
                &p,
                "sending",
            )
            .unwrap();
        s.record(
            event(1),
            "other",
            "send",
            DeliveryState::Sending,
            &p,
            "sending",
        )
        .unwrap();
        assert_eq!(s.history(0, 100).unwrap().len(), 3);
        assert_eq!(files(&dir).len(), 1);
        assert_eq!(s.content(local).unwrap().data, p.data);
        assert_eq!(s.content(sent).unwrap().data, p.data);
        s.update(event(1), "peer", DeliveryState::Failed, "offline")
            .unwrap();
        let history = s.history(0, 100).unwrap();
        assert!(
            history
                .iter()
                .any(|r| r.direction == "local" && r.state == DeliveryState::Applied)
        );
        assert!(
            history
                .iter()
                .any(|r| r.peer == "peer" && r.state == DeliveryState::Failed)
        );
        settings.peers.insert(
            "peer".into(),
            PeerPolicy {
                history: Some(HistoryMode::Off),
                ..PeerPolicy::default()
            },
        );
        s.save_settings(&settings).unwrap();
        assert!(s.content(sent).is_err());
        assert_eq!(s.content(local).unwrap().data, p.data);
        assert_eq!(files(&dir).len(), 1);
        let newer = s
            .record(
                event(2),
                "self",
                "local",
                DeliveryState::Applied,
                &p,
                "local",
            )
            .unwrap();
        assert!(s.content(local).is_err());
        assert_eq!(s.history(0, 100).unwrap().len(), 1);
        assert_eq!(s.content(newer).unwrap().data, p.data);
        assert_eq!(files(&dir).len(), 1);
        s.clear().unwrap();
        assert!(files(&dir).is_empty());
        assert!(s.history(0, 100).unwrap().is_empty());
        drop(s);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn local_history_honors_global_modes_zero_limit_and_memory_budget() {
        let dir = dir();
        let mut s = SqlStore::open(&dir).unwrap();
        let p = payload(Format::Text, b"local fixture".to_vec()).unwrap();
        let mut settings = s.settings().unwrap();
        for mode in [HistoryMode::Content, HistoryMode::Status, HistoryMode::Off] {
            s.clear().unwrap();
            settings.history = mode;
            s.save_settings(&settings).unwrap();
            let id = s
                .record(
                    event(1),
                    "self",
                    "local",
                    DeliveryState::Applied,
                    &p,
                    "local",
                )
                .unwrap();
            assert_eq!(s.content(id).is_ok(), mode == HistoryMode::Content);
            let exported = s.local_history(0, 20).unwrap().1;
            assert_eq!(exported.len(), usize::from(mode != HistoryMode::Off));
            if let Some(summary) = exported.first() {
                assert_eq!(summary.event, event(1));
                assert_eq!(summary.metadata, p.meta);
                assert_eq!(summary.body_available, mode == HistoryMode::Content);
            }
            assert_eq!(
                s.local_content(event(1)).is_ok(),
                mode == HistoryMode::Content
            );
            assert_eq!(
                s.history(0, 100).unwrap().is_empty(),
                mode == HistoryMode::Off
            );
        }
        settings.history = HistoryMode::Content;
        settings.history_limit = 0;
        s.save_settings(&settings).unwrap();
        assert_eq!(
            s.record(
                event(2),
                "self",
                "local",
                DeliveryState::Applied,
                &p,
                "local"
            )
            .unwrap(),
            0
        );
        settings.history_limit = 20;
        settings.history_memory_bytes = p.meta.size;
        s.save_settings(&settings).unwrap();
        let first = s
            .record(
                event(3),
                "self",
                "local",
                DeliveryState::Applied,
                &p,
                "local",
            )
            .unwrap();
        s.record(
            event(3),
            "peer",
            "send",
            DeliveryState::Sending,
            &p,
            "sending",
        )
        .unwrap();
        assert_eq!(s.history(0, 100).unwrap().len(), 2);
        assert_eq!(s.local_history(0, 20).unwrap().1.len(), 1);
        s.record(
            event(4),
            "self",
            "local",
            DeliveryState::Applied,
            &p,
            "local",
        )
        .unwrap();
        assert!(s.content(first).is_err());
        assert_eq!(s.history(0, 100).unwrap().len(), 1);
        assert!(files(&dir).is_empty());
        drop(s);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn restart_removes_orphans_and_old_disk_history_without_reading_keychain() {
        let dir = dir();
        {
            let s = SqlStore::open(&dir).unwrap();
            s.db.execute_batch("CREATE TABLE history(id INTEGER,event_key TEXT,event TEXT,peer TEXT,direction TEXT,state TEXT,body BLOB)").unwrap();
            s.db.execute(
                "INSERT INTO history VALUES(1,?1,?2,'peer','send','\"unknown\"',?3)",
                params![
                    key(event(1)),
                    encoded(&event(1)).unwrap(),
                    b"legacy encrypted fixture".as_slice()
                ],
            )
            .unwrap();
        }
        std::fs::create_dir(dir.join("history-images")).unwrap();
        std::fs::write(
            dir.join("history-images/orphan.enc"),
            b"abandoned encrypted fixture",
        )
        .unwrap();
        let s = SqlStore::open(&dir).unwrap();
        assert!(files(&dir).is_empty());
        assert!(s.history(0, 100).unwrap().is_empty());
        assert_eq!(s.pending_receipts().unwrap().len(), 1);
        let table_count: u64 =
            s.db.query_row(
                "SELECT count(*) FROM sqlite_master WHERE name='history'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(table_count, 0);
        drop(s);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(test)]
mod policy_migration_tests {
    use super::*;
    #[test]
    fn legacy_block_migrates_to_both_directions_off_and_is_removed_from_disk() {
        for blocked in [false, true] {
            let dir = std::env::temp_dir().join(format!(
                "shuttli-policy-{:?}",
                crate::random_epoch().unwrap()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            let mut original = Settings::default();
            let id = "02".repeat(32);
            original.peers.insert(
                id.clone(),
                PeerPolicy {
                    send: true,
                    quiet: true,
                    ..PeerPolicy::default()
                },
            );
            let mut legacy = serde_json::to_value(&original).unwrap();
            legacy["peers"][&id]["blocked"] = blocked.into();
            std::fs::write(
                dir.join("settings.json"),
                serde_json::to_vec(&legacy).unwrap(),
            )
            .unwrap();
            let migrated = SqlStore::open(&dir).unwrap().settings().unwrap();
            assert_eq!(migrated.send, original.send);
            assert_eq!(migrated.receive, original.receive);
            assert_eq!(migrated.peers[&id].send, !blocked);
            assert_eq!(migrated.peers[&id].receive, !blocked);
            assert!(migrated.peers[&id].quiet);
            let saved = std::fs::read(dir.join("settings.json")).unwrap();
            assert!(!String::from_utf8_lossy(&saved).contains("blocked"));
            assert_eq!(
                serde_json::from_slice::<Settings>(&saved).unwrap(),
                migrated
            );
            assert_eq!(SqlStore::open(&dir).unwrap().settings().unwrap(), migrated);
            std::fs::remove_dir_all(dir).unwrap();
        }
    }
    #[test]
    fn malformed_legacy_policy_cannot_silently_enable_a_device() {
        let mut settings = Settings::default();
        settings
            .peers
            .insert("02".repeat(32), PeerPolicy::default());
        let mut value = serde_json::to_value(settings).unwrap();
        value["peers"]["02".repeat(32)]["blocked"] = "true".into();
        assert!(decode_settings(&serde_json::to_vec(&value).unwrap()).is_err());
        // The live API accepts only the new policy, with no hidden blocking flag.
        value["peers"]["02".repeat(32)]["blocked"] = false.into();
        assert!(serde_json::from_value::<Settings>(value).is_err());
    }
}
