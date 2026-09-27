use super::*;
use crate::content::{MAX_BYTES, payload};
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use shuttli_model::sync::{ClipboardStamp, Format};
use shuttli_ports::sync::{ClipboardValue, Payload};
use std::io::{Read, Write as IoWrite};
use std::os::unix::net::UnixStream;
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
use x11rb::{
    COPY_DEPTH_FROM_PARENT, CURRENT_TIME, NONE,
    connection::Connection,
    protocol::{
        Event,
        xfixes::{ConnectionExt as _, SelectionEventMask},
        xproto::*,
    },
    rust_connection::RustConnection,
    wrapper::ConnectionExt as _,
};

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
struct Atoms {
    clipboard: Atom,
    targets: Atom,
    utf8: Atom,
    png: Atom,
    property: Atom,
    incr: Atom,
}
impl Atoms {
    fn new(c: &RustConnection) -> Result<Self> {
        let a = |n: &[u8]| -> Result<Atom> {
            Ok(c.intern_atom(false, n)
                .map_err(err)?
                .reply()
                .map_err(err)?
                .atom)
        };
        Ok(Self {
            clipboard: a(b"CLIPBOARD")?,
            targets: a(b"TARGETS")?,
            utf8: a(b"UTF8_STRING")?,
            png: a(b"image/png")?,
            property: a(b"SHUTTLI_DATA")?,
            incr: a(b"INCR")?,
        })
    }
}
fn connection() -> Result<(RustConnection, Window, Atoms)> {
    let (c, s) = x11rb::connect(None).map_err(err)?;
    let w = c.generate_id().map_err(err)?;
    c.create_window(
        COPY_DEPTH_FROM_PARENT,
        w,
        c.setup().roots[s].root,
        0,
        0,
        1,
        1,
        0,
        WindowClass::INPUT_OUTPUT,
        0,
        &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
    )
    .map_err(err)?
    .check()
    .map_err(err)?;
    let a = Atoms::new(&c)?;
    Ok((c, w, a))
}
struct Write {
    payload: Payload,
    expected: u64,
    reply: mpsc::SyncSender<Result<()>>,
}
pub struct XClipboard {
    generation: Arc<AtomicU64>,
    writes: mpsc::SyncSender<Write>,
    wake: UnixStream,
    cache: Option<ClipboardValue>,
}
impl XClipboard {
    pub fn open() -> Result<Self> {
        let (c, w, a) = connection()?;
        c.xfixes_query_version(5, 0)
            .map_err(err)?
            .reply()
            .map_err(err)?;
        c.xfixes_select_selection_input(
            w,
            a.clipboard,
            SelectionEventMask::SET_SELECTION_OWNER
                | SelectionEventMask::SELECTION_WINDOW_DESTROY
                | SelectionEventMask::SELECTION_CLIENT_CLOSE,
        )
        .map_err(err)?
        .check()
        .map_err(err)?;
        c.flush().map_err(err)?;
        let g = Arc::new(AtomicU64::new(1));
        let generation = g.clone();
        let (tx, rx) = mpsc::sync_channel::<Write>(1);
        let (mut receiver, wake) = UnixStream::pair().map_err(err)?;
        receiver.set_nonblocking(true).map_err(err)?;
        wake.set_nonblocking(true).map_err(err)?;
        thread::Builder::new()
            .name("clipboard-owner".into())
            .spawn(move || {
                let mut owned: Option<Payload> = None;
                // Large selections use ICCCM INCR; bound simultaneous readers and lifetime.
                let mut streams: HashMap<(Window, Atom), (Payload, usize, Atom, Instant)> =
                    HashMap::new();
                loop {
                    let mut wake_bytes = [0; 64];
                    while receiver.read(&mut wake_bytes).is_ok_and(|n| n > 0) {}
                    loop {
                        match c.poll_for_event() {
                            Ok(Some(Event::XfixesSelectionNotify(_))) => {
                                g.fetch_add(1, Ordering::SeqCst);
                            }
                            Ok(Some(Event::SelectionClear(_))) => {
                                owned = None;
                            }
                            Ok(Some(Event::SelectionRequest(e))) => {
                                let prop = if e.property == NONE {
                                    e.target
                                } else {
                                    e.property
                                };
                                let mut ok = false;
                                if let Some(ref p) = owned {
                                    let t = if p.meta.format == Format::Text {
                                        a.utf8
                                    } else {
                                        a.png
                                    };
                                    if e.target == a.targets {
                                        let ts = [a.targets, t];
                                        ok = c
                                            .change_property32(
                                                PropMode::REPLACE,
                                                e.requestor,
                                                prop,
                                                AtomEnum::ATOM,
                                                &ts,
                                            )
                                            .is_ok();
                                    } else if e.target == t {
                                        if p.data.len() <= 60 * 1024 {
                                            ok = c
                                                .change_property8(
                                                    PropMode::REPLACE,
                                                    e.requestor,
                                                    prop,
                                                    t,
                                                    &p.data,
                                                )
                                                .is_ok();
                                        } else if streams.len() < 8 {
                                            let _ = c.change_window_attributes(
                                                e.requestor,
                                                &ChangeWindowAttributesAux::new()
                                                    .event_mask(EventMask::PROPERTY_CHANGE),
                                            );
                                            ok = c
                                                .change_property32(
                                                    PropMode::REPLACE,
                                                    e.requestor,
                                                    prop,
                                                    a.incr,
                                                    &[p.data.len() as u32],
                                                )
                                                .is_ok();
                                            if ok {
                                                streams.insert(
                                                    (e.requestor, prop),
                                                    (p.clone(), 0, t, Instant::now()),
                                                );
                                            }
                                        }
                                    }
                                }
                                let reply = SelectionNotifyEvent {
                                    response_type: SELECTION_NOTIFY_EVENT,
                                    sequence: 0,
                                    time: e.time,
                                    requestor: e.requestor,
                                    selection: e.selection,
                                    target: e.target,
                                    property: if ok { prop } else { NONE },
                                };
                                let _ =
                                    c.send_event(false, e.requestor, EventMask::NO_EVENT, reply);
                            }
                            Ok(Some(Event::PropertyNotify(e))) if e.state == Property::DELETE => {
                                let key = (e.window, e.atom);
                                let mut done = false;
                                if let Some((p, offset, t, start)) = streams.get_mut(&key) {
                                    let end = (*offset + 60 * 1024).min(p.data.len());
                                    let _ = c.change_property8(
                                        PropMode::REPLACE,
                                        e.window,
                                        e.atom,
                                        *t,
                                        &p.data[*offset..end],
                                    );
                                    done = *offset == p.data.len();
                                    *offset = end;
                                    *start = Instant::now();
                                }
                                if done {
                                    streams.remove(&key);
                                }
                            }
                            Ok(Some(_)) => {}
                            Ok(None) => break,
                            Err(_) => return,
                        }
                    }
                    streams.retain(|_, v| v.3.elapsed() < Duration::from_secs(5));
                    match rx.try_recv() {
                        Ok(write) => {
                            let result = if write.expected != g.load(Ordering::SeqCst) {
                                Err("clipboard changed before write".into())
                            } else {
                                owned = Some(write.payload);
                                c.set_selection_owner(w, a.clipboard, CURRENT_TIME)
                                    .map_err(err)
                                    .and_then(|v| v.check().map_err(err))
                            };
                            let _ = write.reply.send(result);
                            let _ = c.flush();
                            continue;
                        }
                        Err(mpsc::TryRecvError::Disconnected) => return,
                        Err(mpsc::TryRecvError::Empty) => {}
                    }
                    if c.flush().is_err() {
                        return;
                    }
                    let mut fds = [
                        PollFd::new(c.stream(), PollFlags::IN),
                        PollFd::new(&receiver, PollFlags::IN),
                    ];
                    if poll(
                        &mut fds,
                        Some(&Timespec {
                            tv_sec: 5,
                            tv_nsec: 0,
                        }),
                    )
                    .is_err()
                    {
                        return;
                    }
                }
            })
            .map_err(err)?;
        Ok(Self {
            generation,
            writes: tx,
            wake,
            cache: None,
        })
    }
    fn read_os(&self) -> Result<ClipboardValue> {
        let before = self.generation.load(Ordering::SeqCst);
        let (c, w, a) = connection()?;
        if c.get_selection_owner(a.clipboard)
            .map_err(err)?
            .reply()
            .map_err(err)?
            .owner
            == NONE
        {
            return Ok(ClipboardValue {
                stamp: ClipboardStamp {
                    generation: before,
                    digest: [0; 32],
                    sensitive: false,
                },
                payload: None,
            });
        }
        let targets = selection(&c, w, &a, a.targets)?;
        if targets.len() > 4096 {
            return Err("too many clipboard targets".into());
        }
        let atoms: Vec<u32> = targets
            .chunks_exact(4)
            .map(|v| u32::from_ne_bytes(v.try_into().expect("four bytes")))
            .collect();
        let format = if atoms.contains(&a.png) {
            Some((Format::Png, a.png))
        } else if atoms.contains(&a.utf8) {
            Some((Format::Text, a.utf8))
        } else {
            None
        };
        let mut sensitive = false;
        for t in &atoms {
            let name = c.get_atom_name(*t).map_err(err)?.reply().map_err(err)?.name;
            if [
                b"x-kde-passwordManagerHint".as_slice(),
                b"org.nspasteboard.ConcealedType",
                b"org.nspasteboard.TransientType",
            ]
            .contains(&name.as_slice())
            {
                sensitive = true;
            }
        }
        let data = if let Some((f, t)) = format {
            Some(payload(f, selection(&c, w, &a, t)?)?)
        } else {
            None
        };
        let after = self.generation.load(Ordering::SeqCst);
        if before != after {
            return Err("clipboard changed during capture".into());
        }
        Ok(ClipboardValue {
            stamp: ClipboardStamp {
                generation: after,
                digest: data.as_ref().map_or([0; 32], |p| p.meta.digest),
                sensitive,
            },
            payload: data,
        })
    }
}
fn selection(c: &RustConnection, w: Window, a: &Atoms, target: Atom) -> Result<Vec<u8>> {
    c.convert_selection(w, a.clipboard, target, a.property, CURRENT_TIME)
        .map_err(err)?;
    c.flush().map_err(err)?;
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut incr = false;
    let mut result = Vec::new();
    loop {
        if Instant::now() > deadline {
            return Err("clipboard request timed out".into());
        }
        let event = c.poll_for_event().map_err(err)?;
        let read = match event {
            Some(Event::SelectionNotify(e)) => {
                if e.property == NONE {
                    return Err("clipboard format unavailable".into());
                }
                true
            }
            Some(Event::PropertyNotify(e)) => incr && e.state == Property::NEW_VALUE,
            _ => false,
        };
        if read {
            let p = c
                .get_property(
                    true,
                    w,
                    a.property,
                    AtomEnum::ANY,
                    0,
                    (MAX_BYTES / 4 + 1) as u32,
                )
                .map_err(err)?
                .reply()
                .map_err(err)?;
            if p.bytes_after > 0 || result.len() + p.value.len() > MAX_BYTES {
                return Err("clipboard exceeds size limit".into());
            }
            if p.type_ == a.incr {
                if p.value32()
                    .and_then(|mut v| v.next())
                    .is_some_and(|n| n as usize > MAX_BYTES)
                {
                    return Err("clipboard INCR exceeds size limit".into());
                }
                incr = true;
                c.flush().map_err(err)?;
                continue;
            }
            if !incr {
                return Ok(p.value);
            }
            if p.value.is_empty() {
                return Ok(result);
            }
            result.extend(p.value);
            c.flush().map_err(err)?;
        } else {
            thread::sleep(Duration::from_millis(2));
        }
    }
}
impl Clipboard for XClipboard {
    fn description(&self) -> &str {
        "X11/XWayland (XFixes; conservative equal-content suppression)"
    }
    fn read(&mut self) -> Result<ClipboardValue> {
        if let Some(v) = &self.cache {
            if v.stamp.generation == self.generation.load(Ordering::SeqCst) {
                return Ok(v.clone());
            }
        }
        let v = self.read_os()?;
        self.cache = Some(v.clone());
        Ok(v)
    }
    fn write(
        &mut self,
        payload: &Payload,
        authorization: shuttli_core::sync::WriteAuthorization,
    ) -> Result<ClipboardValue> {
        if authorization.metadata() != &payload.meta {
            return Err("write content differs from core authorization".into());
        }
        let expected = authorization.baseline();

        let current = self.read()?;
        if current.stamp != expected {
            return Err("clipboard superseded".into());
        }
        let (tx, rx) = mpsc::sync_channel(1);
        self.writes
            .send(Write {
                payload: payload.clone(),
                expected: expected.generation,
                reply: tx,
            })
            .map_err(err)?;
        self.wake.write_all(&[1]).map_err(err)?;
        rx.recv_timeout(Duration::from_secs(3)).map_err(err)??;
        // Read via a separate X client. The owner thread must actually serve it.
        self.cache = None;
        for _ in 0..5 {
            match self.read_os() {
                Ok(v) if v.stamp.digest == payload.meta.digest => {
                    self.cache = Some(v.clone());
                    return Ok(v);
                }
                Ok(_) => return Err("clipboard readback mismatch".into()),
                Err(_) => thread::sleep(Duration::from_millis(10)),
            }
        }
        Err("clipboard readback failed".into())
    }
}
