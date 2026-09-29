//! Session history: RAM text and disposable, encrypted image files.
//! No OS keyring and no serialized history list or text body.
use crate::{content::payload, files::atomic_write};
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit, Payload as Aad},
};
use shuttli_model::mobile::HistorySummary;
use shuttli_model::sync::*;
use shuttli_ports::sync::{Payload, Result};
use std::{
    collections::{BTreeSet, VecDeque},
    io::Read,
    path::PathBuf,
};
use zeroize::Zeroizing;

fn error(e: impl std::fmt::Display) -> String {
    e.to_string()
}
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
pub(crate) fn mode(settings: &Settings, peer: &str) -> HistoryMode {
    let requested = settings
        .peers
        .get(peer)
        .and_then(|p| p.history)
        .unwrap_or(settings.history);
    match (settings.history, requested) {
        (HistoryMode::Off, _) | (_, HistoryMode::Off) => HistoryMode::Off,
        (HistoryMode::Status, _) | (_, HistoryMode::Status) => HistoryMode::Status,
        _ => HistoryMode::Content,
    }
}
enum Body {
    Shared(i64),
    Text(Payload),
    Image { path: PathBuf, bytes: u64 },
}
struct Record {
    entry: HistoryEntry,
    meta: Metadata,
    body: Option<Body>,
}
pub(crate) struct RecentHistory {
    root: PathBuf,
    secret: Zeroizing<[u8; 32]>,
    rows: VecDeque<Record>,
    next: i64,
    revision: u64,
    cache_writable: bool,
}
impl RecentHistory {
    pub fn open(root: PathBuf) -> Result<Self> {
        // The directory is exclusively owned by this feature. Never follow a
        // substituted root symlink; remove abandoned files before a new session.
        if let Ok(metadata) = std::fs::symlink_metadata(&root) {
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err("image history cache is not a private directory".into());
            }
            std::fs::remove_dir_all(&root).map_err(error)?;
        }
        std::fs::create_dir(&root).map_err(error)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))
                .map_err(error)?;
        }
        let mut secret = Zeroizing::new([0; 32]);
        getrandom::getrandom(&mut *secret).map_err(error)?;
        let mut id = [0; 8];
        getrandom::getrandom(&mut id).map_err(error)?;
        // Session-random, JSON-safe IDs keep an old UI selection from referring
        // to a different item after the daemon restarts.
        let next = (u64::from_le_bytes(id) & ((1 << 52) - 1)) as i64;
        Ok(Self {
            root,
            secret,
            rows: VecDeque::new(),
            next,
            revision: 1,
            cache_writable: true,
        })
    }
    fn cipher(&self) -> XChaCha20Poly1305 {
        XChaCha20Poly1305::new((&*self.secret).into())
    }
    fn aad(id: i64, meta: &Metadata) -> Result<String> {
        Ok(format!(
            "image-history-v1:{id}:{}",
            serde_json::to_string(meta).map_err(error)?
        ))
    }
    fn discard(row: &mut Record) -> Result<()> {
        if let Some(Body::Image { path, .. }) = &row.body {
            match std::fs::remove_file(path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(format!("image history cleanup failed: {e}")),
            }
        }
        row.body = None;
        row.entry.available = false;
        Ok(())
    }
    fn remove(&mut self, index: usize) -> Result<()> {
        Self::discard(&mut self.rows[index])?;
        self.rows.remove(index);
        self.bump();
        Ok(())
    }
    fn bump(&mut self) {
        self.revision = self.revision.wrapping_add(1).max(1);
    }
    pub fn record(
        &mut self,
        settings: &Settings,
        mut entry: HistoryEntry,
        p: &Payload,
    ) -> Result<i64> {
        self.prune(settings)?; // A failed deletion prevents further cache growth.
        if mode(settings, &entry.peer) == HistoryMode::Off || settings.history_limit == 0 {
            return Ok(0);
        }
        let index = self.rows.iter().position(|r| {
            r.entry.event == entry.event
                && r.entry.peer == entry.peer
                && r.entry.direction == entry.direction
        });
        let id = if let Some(index) = index {
            Self::discard(&mut self.rows[index])?;
            self.rows[index].entry.id
        } else {
            self.next = self.next.checked_add(1).ok_or("history ID exhausted")?;
            self.next
        };
        entry.id = id;
        entry.time = now();
        entry.available = false;
        let retain = mode(settings, &entry.peer) == HistoryMode::Content
            && entry.state != DeliveryState::Receiving
            && p.meta.size == p.data.len() as u64;
        // Automatic fan-out shares the local event's one retained body. Peer
        // status/off policies still govern whether that peer row can preview it.
        let shared = self
            .rows
            .iter()
            .find(|r| {
                entry.direction == "send"
                    && r.entry.direction == "local"
                    && r.entry.event == entry.event
                    && r.meta == p.meta
                    && r.body.is_some()
            })
            .map(|r| r.entry.id);
        let body = if !retain {
            None
        } else if let Some(id) = shared {
            Some(Body::Shared(id))
        } else {
            match p.meta.format {
                Format::Text if p.meta.size <= settings.history_memory_bytes => {
                    Some(Body::Text(p.clone()))
                }
                Format::Png
                    if self.cache_writable && p.meta.size + 40 <= settings.history_bytes =>
                {
                    let mut nonce = [0; 24];
                    getrandom::getrandom(&mut nonce).map_err(error)?;
                    let aad = Self::aad(id, &p.meta)?;
                    let mut bytes = nonce.to_vec();
                    bytes.extend(
                        self.cipher()
                            .encrypt(
                                XNonce::from_slice(&nonce),
                                Aad {
                                    msg: &p.data,
                                    aad: aad.as_bytes(),
                                },
                            )
                            .map_err(|_| "image history encryption failed")?,
                    );
                    let path = self.root.join(format!("{id}.enc"));
                    match atomic_write(&path, &bytes) {
                        Ok(()) => Some(Body::Image {
                            path,
                            bytes: bytes.len() as u64,
                        }),
                        // History caching is optional; a cache failure must not
                        // turn an already applied clipboard into a false failure.
                        Err(_) => {
                            self.cache_writable = false;
                            let _ = std::fs::remove_file(&path);
                            entry
                                .detail
                                .push_str("; image preview unavailable: cache write failed");
                            None
                        }
                    }
                }
                _ => {
                    entry
                        .detail
                        .push_str("; content exceeds history cache budget");
                    None
                }
            }
        };
        entry.available = body.is_some();
        let row = Record {
            entry,
            meta: p.meta.clone(),
            body,
        };
        if let Some(index) = index {
            self.rows[index] = row;
        } else {
            self.rows.push_front(row);
        }
        self.bump();
        self.prune(settings)?;
        Ok(id)
    }
    pub fn update(&mut self, event: EventId, peer: &str, state: DeliveryState, detail: &str) {
        let mut changed = false;
        for row in &mut self.rows {
            if row.entry.event == event
                && row.entry.peer == peer
                && (row.entry.direction == "send" || row.entry.state != DeliveryState::Applied)
            {
                row.entry.state = state;
                row.entry.detail = detail.into();
                changed = true;
            }
        }
        if changed {
            self.bump();
        }
    }
    pub fn history(&self, offset: usize, limit: usize) -> Vec<HistoryEntry> {
        self.rows
            .iter()
            .skip(offset.min(10_000))
            .take(limit.min(100))
            .map(|r| r.entry.clone())
            .collect()
    }
    pub fn local_history(&self, offset: usize, limit: usize) -> (u64, Vec<HistorySummary>) {
        let items = self
            .rows
            .iter()
            .filter(|r| r.entry.direction == "local")
            .skip(offset.min(10_000))
            .take(limit.min(21))
            .map(|r| HistorySummary {
                event: r.entry.event,
                metadata: r.meta.clone(),
                copied_at_ms: r.entry.time.saturating_mul(1000),
                body_available: r.entry.available && r.body.is_some(),
            })
            .collect();
        (self.revision, items)
    }
    pub fn local_content(&self, event: EventId) -> Result<Payload> {
        let row = self
            .rows
            .iter()
            .find(|r| r.entry.direction == "local" && r.entry.event == event)
            .ok_or("local history item expired or cleared")?;
        self.content(row.entry.id)
    }
    pub fn content(&self, id: i64) -> Result<Payload> {
        let row = self
            .rows
            .iter()
            .find(|r| r.entry.id == id)
            .ok_or("history item expired or cleared")?;
        match row.body.as_ref().ok_or("history content not retained")? {
            Body::Shared(owner) => {
                // References only point to a local row; never chains or cycles.
                let source = self
                    .rows
                    .iter()
                    .find(|r| r.entry.id == *owner && r.entry.direction == "local")
                    .ok_or("history content expired or cleared")?;
                if source.meta != row.meta {
                    return Err("history content mismatch".into());
                }
                self.content(*owner)
            }
            Body::Text(p) => Ok(p.clone()),
            Body::Image {
                path,
                bytes: expected,
            } => {
                let mut bytes = Vec::new();
                std::fs::File::open(path)
                    .map_err(error)?
                    .take(*expected + 1)
                    .read_to_end(&mut bytes)
                    .map_err(error)?;
                if bytes.len() as u64 != *expected || bytes.len() < 40 {
                    return Err("corrupt image history cache".into());
                }
                let aad = Self::aad(id, &row.meta)?;
                let data = self
                    .cipher()
                    .decrypt(
                        XNonce::from_slice(&bytes[..24]),
                        Aad {
                            msg: &bytes[24..],
                            aad: aad.as_bytes(),
                        },
                    )
                    .map_err(|_| "image history authentication failed")?;
                let result = payload(Format::Png, data)?;
                if result.meta != row.meta {
                    return Err("image history content mismatch".into());
                }
                Ok(result)
            }
        }
    }
    pub fn clear(&mut self) -> Result<()> {
        while !self.rows.is_empty() {
            self.remove(self.rows.len() - 1)?;
        }
        // Also clean abandoned atomic-write staging files. No caller-supplied path.
        for entry in std::fs::read_dir(&self.root).map_err(error)? {
            let entry = entry.map_err(error)?;
            std::fs::remove_file(entry.path()).map_err(error)?;
        }
        getrandom::getrandom(&mut *self.secret).map_err(error)?;
        self.cache_writable = true;
        Ok(())
    }
    pub fn prune(&mut self, settings: &Settings) -> Result<()> {
        if settings.history == HistoryMode::Off || settings.history_limit == 0 {
            return self.clear();
        }
        let cutoff = now().saturating_sub(settings.history_days as u64 * 86400);
        let mut index = 0;
        while index < self.rows.len() {
            let mode = mode(settings, &self.rows[index].entry.peer);
            if mode == HistoryMode::Off || self.rows[index].entry.time < cutoff {
                self.remove(index)?;
            } else {
                if mode == HistoryMode::Status {
                    if self.rows[index].body.is_some() {
                        self.bump();
                    }
                    Self::discard(&mut self.rows[index])?;
                }
                index += 1;
            }
        }
        loop {
            let (memory, disk) = self
                .rows
                .iter()
                .fold((0u64, 0u64), |(m, d), r| match &r.body {
                    Some(Body::Text(p)) => (m + p.meta.size, d),
                    Some(Body::Image { bytes, .. }) => (m, d + bytes),
                    None | Some(Body::Shared(_)) => (m, d),
                });
            let events: BTreeSet<_> = self.rows.iter().map(|r| r.entry.event).collect();
            if events.len() <= settings.history_limit
                && self.rows.len() <= 10_000
                && memory <= settings.history_memory_bytes
                && disk <= settings.history_bytes
            {
                break;
            }
            let event = self
                .rows
                .back()
                .expect("over budget requires a row")
                .entry
                .event;
            for index in (0..self.rows.len()).rev() {
                if self.rows[index].entry.event == event {
                    self.remove(index)?;
                }
            }
        }
        let retained: BTreeSet<_> = self
            .rows
            .iter()
            .filter(|r| r.entry.direction == "local" && r.body.is_some())
            .map(|r| r.entry.id)
            .collect();
        for row in &mut self.rows {
            if let Some(Body::Shared(id)) = &row.body {
                if !retained.contains(id) {
                    Self::discard(row)?;
                }
            }
        }
        Ok(())
    }
}
impl Drop for RecentHistory {
    fn drop(&mut self) {
        let _ = self.clear();
        let _ = std::fs::remove_dir(&self.root);
    }
}
