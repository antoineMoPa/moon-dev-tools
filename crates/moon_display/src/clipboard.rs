//! Explicit text selection transfers on the screen's event loop. Selection replies and
//! incremental transfers are advanced by X events, so an unresponsive owner never stalls
//! drawing or input. XFixes timestamps distinguish a new copy from old clipboard contents.
use crate::{CLIPBOARD_LIMIT, ClipboardAction, ClipboardRequest, Held, keys::Keyboard};
use anyhow::ensure;
use std::{
    sync::mpsc::Sender,
    time::{Duration, Instant},
};
use x11rb::{
    connection::Connection,
    protocol::{
        Event,
        xfixes::{ConnectionExt as _, SelectionEventMask},
        xproto::{
            AtomEnum, ConnectionExt, CreateWindowAux, EventMask, PropMode, Property,
            SELECTION_NOTIFY_EVENT, SelectionNotifyEvent, WindowClass,
        },
    },
    rust_connection::RustConnection,
    wrapper::ConnectionExt as _,
};

x11rb::atom_manager! {
    Atoms: AtomsCookie { CLIPBOARD, TARGETS, UTF8_STRING, INCR, MOON_CLIPBOARD, MOON_COPY_TIME, }
}
const TIMEOUT: Duration = Duration::from_secs(2);
type Reply = Sender<Result<Option<String>, String>>;
struct Pending {
    window: u32,
    reply: Reply,
    until: Instant,
    waiting_for_owner: bool,
    timestamp: Option<u32>,
    cut: bool,
    incremental: bool,
    bytes: Vec<u8>,
}
pub(crate) struct Clipboard {
    window: u32,
    root: u32,
    atoms: Atoms,
    text: Vec<u8>,
    pending: Option<Pending>,
}
impl Clipboard {
    pub(crate) fn new(conn: &RustConnection, root: u32) -> anyhow::Result<Self> {
        let atoms = Atoms::new(conn)?.reply()?;
        let window = conn.generate_id()?;
        conn.create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            window,
            root,
            -100,
            -100,
            1,
            1,
            0,
            WindowClass::INPUT_OUTPUT,
            x11rb::COPY_FROM_PARENT,
            &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
        )?
        .check()?;
        conn.xfixes_query_version(5, 0)?.reply()?;
        conn.xfixes_select_selection_input(
            window,
            atoms.CLIPBOARD,
            SelectionEventMask::SET_SELECTION_OWNER
                | SelectionEventMask::SELECTION_WINDOW_DESTROY
                | SelectionEventMask::SELECTION_CLIENT_CLOSE,
        )?
        .check()?;
        Ok(Self {
            window,
            root,
            atoms,
            text: Vec::new(),
            pending: None,
        })
    }
    pub(crate) fn request(
        &mut self,
        conn: &RustConnection,
        keys: &mut Keyboard,
        request: ClipboardRequest,
    ) {
        if self.pending.is_some() {
            let _ = request.reply.send(Err(
                "A desktop clipboard transfer is already in progress".into()
            ));
            return;
        }
        let result = (|| -> anyhow::Result<()> {
            match request.action {
                ClipboardAction::Paste(text) => {
                    ensure!(
                        text.len() <= CLIPBOARD_LIMIT,
                        "Clipboard text exceeds 64 KiB"
                    );
                    self.text = text.into_bytes();
                    conn.set_selection_owner(
                        self.window,
                        self.atoms.CLIPBOARD,
                        x11rb::CURRENT_TIME,
                    )?
                    .check()?;
                    chord(conn, keys, b'v')?;
                    let _ = request.reply.send(Ok(None));
                }
                ClipboardAction::Copy { cut } => {
                    // A unique requestor window isolates delayed SelectionNotify and INCR
                    // chunks from a transfer that has timed out. Destroy it on completion.
                    let window = conn.generate_id()?;
                    conn.create_window(
                        x11rb::COPY_DEPTH_FROM_PARENT,
                        window,
                        self.root,
                        -100,
                        -100,
                        1,
                        1,
                        0,
                        WindowClass::INPUT_OUTPUT,
                        x11rb::COPY_FROM_PARENT,
                        &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
                    )?
                    .check()?;
                    self.pending = Some(Pending {
                        window,
                        reply: request.reply.clone(),
                        until: Instant::now() + TIMEOUT,
                        waiting_for_owner: true,
                        timestamp: None,
                        cut,
                        incremental: false,
                        bytes: Vec::new(),
                    });
                    // PropertyNotify supplies a server timestamp and an event-order fence;
                    // copy never changes the selection unless the application copies text.
                    conn.change_property8(
                        PropMode::REPLACE,
                        window,
                        self.atoms.MOON_COPY_TIME,
                        AtomEnum::STRING,
                        b"copy",
                    )?;
                }
            }
            conn.flush()?;
            Ok(())
        })();
        if let Err(error) = result {
            self.finish(conn, Err(format!("Desktop clipboard: {error}")));
            let _ = request
                .reply
                .send(Err(format!("Desktop clipboard: {error}")));
        }
    }
    pub(crate) fn tick(&mut self, conn: &RustConnection) {
        if self
            .pending
            .as_ref()
            .is_some_and(|p| Instant::now() >= p.until)
        {
            self.finish(
                conn,
                Err(
                    "The application did not provide copied text (select text and try again)"
                        .into(),
                ),
            );
        }
    }
    fn finish(&mut self, conn: &RustConnection, result: Result<Option<String>, String>) {
        if let Some(pending) = self.pending.take() {
            let _ = conn.destroy_window(pending.window);
            let _ = conn.flush();
            let _ = pending.reply.send(result);
        }
    }
    pub(crate) fn heard(&mut self, conn: &RustConnection, keys: &mut Keyboard, event: &Event) {
        if let Err(error) = self.event(conn, keys, event) {
            self.finish(conn, Err(format!("Desktop clipboard: {error}")));
        }
    }
    fn event(
        &mut self,
        conn: &RustConnection,
        keys: &mut Keyboard,
        event: &Event,
    ) -> anyhow::Result<()> {
        match event {
            Event::SelectionRequest(r)
                if r.owner == self.window && r.selection == self.atoms.CLIPBOARD =>
            {
                let property = if r.property == x11rb::NONE {
                    r.target
                } else {
                    r.property
                };
                let supported = if r.target == self.atoms.TARGETS {
                    conn.change_property32(
                        PropMode::REPLACE,
                        r.requestor,
                        property,
                        AtomEnum::ATOM,
                        &[self.atoms.TARGETS, self.atoms.UTF8_STRING],
                    )?
                    .check()
                    .is_ok()
                } else if r.target == self.atoms.UTF8_STRING {
                    conn.change_property8(
                        PropMode::REPLACE,
                        r.requestor,
                        property,
                        self.atoms.UTF8_STRING,
                        &self.text,
                    )?
                    .check()
                    .is_ok()
                } else {
                    false
                };
                let notify = SelectionNotifyEvent {
                    response_type: SELECTION_NOTIFY_EVENT,
                    sequence: 0,
                    time: r.time,
                    requestor: r.requestor,
                    selection: r.selection,
                    target: r.target,
                    property: if supported { property } else { x11rb::NONE },
                };
                conn.send_event(false, r.requestor, EventMask::NO_EVENT, notify)?;
            }
            Event::PropertyNotify(r)
                if self.pending.as_ref().is_some_and(|p| r.window == p.window)
                    && r.atom == self.atoms.MOON_COPY_TIME
                    && r.state == Property::NEW_VALUE =>
            {
                if let Some(pending) = &mut self.pending {
                    if pending.timestamp.is_none() {
                        pending.timestamp = Some(r.time);
                        chord(conn, keys, if pending.cut { b'x' } else { b'c' })?;
                    }
                }
            }
            Event::XfixesSelectionNotify(r) if r.selection == self.atoms.CLIPBOARD => {
                if let Some(pending) = &mut self.pending {
                    let fresh = pending
                        .timestamp
                        .is_some_and(|time| (r.timestamp.wrapping_sub(time) as i32) >= 0);
                    if r.owner != self.window
                        && r.owner != x11rb::NONE
                        && fresh
                        && pending.waiting_for_owner
                    {
                        pending.waiting_for_owner = false;
                        conn.delete_property(pending.window, self.atoms.MOON_CLIPBOARD)?;
                        conn.convert_selection(
                            pending.window,
                            self.atoms.CLIPBOARD,
                            self.atoms.UTF8_STRING,
                            self.atoms.MOON_CLIPBOARD,
                            r.timestamp,
                        )?;
                    }
                }
            }
            Event::SelectionNotify(r)
                if self
                    .pending
                    .as_ref()
                    .is_some_and(|p| r.requestor == p.window)
                    && r.selection == self.atoms.CLIPBOARD =>
            {
                if self.pending.as_ref().is_some_and(|p| !p.waiting_for_owner) {
                    ensure!(
                        r.property == self.atoms.MOON_CLIPBOARD,
                        "The application clipboard contains no text"
                    );
                    let reply = conn
                        .get_property(
                            false,
                            r.requestor,
                            r.property,
                            AtomEnum::ANY,
                            0,
                            (CLIPBOARD_LIMIT / 4 + 1) as u32,
                        )?
                        .reply()?;
                    if reply.type_ == self.atoms.INCR {
                        let size = reply.value32().and_then(|mut n| n.next()).unwrap_or(0) as usize;
                        ensure!(size <= CLIPBOARD_LIMIT, "Clipboard text exceeds 64 KiB");
                        self.pending.as_mut().unwrap().incremental = true;
                        conn.delete_property(r.requestor, r.property)?;
                    } else {
                        ensure!(
                            reply.type_ == self.atoms.UTF8_STRING && reply.format == 8,
                            "The application clipboard contains no UTF-8 text"
                        );
                        ensure!(
                            reply.bytes_after == 0 && reply.value.len() <= CLIPBOARD_LIMIT,
                            "Clipboard text exceeds 64 KiB"
                        );
                        let text = String::from_utf8(reply.value)?;
                        conn.delete_property(r.requestor, r.property)?;
                        self.finish(conn, Ok(Some(text)));
                    }
                }
            }
            Event::PropertyNotify(r)
                if self.pending.as_ref().is_some_and(|p| r.window == p.window)
                    && r.atom == self.atoms.MOON_CLIPBOARD
                    && r.state == Property::NEW_VALUE =>
            {
                if self.pending.as_ref().is_some_and(|p| p.incremental) {
                    let reply = conn
                        .get_property(
                            true,
                            r.window,
                            r.atom,
                            self.atoms.UTF8_STRING,
                            0,
                            (CLIPBOARD_LIMIT / 4 + 1) as u32,
                        )?
                        .reply()?;
                    ensure!(
                        reply.type_ == self.atoms.UTF8_STRING && reply.format == 8,
                        "The application clipboard contains no UTF-8 text"
                    );
                    let pending = self.pending.as_mut().unwrap();
                    ensure!(
                        reply.bytes_after == 0
                            && pending.bytes.len() + reply.value.len() <= CLIPBOARD_LIMIT,
                        "Clipboard text exceeds 64 KiB"
                    );
                    if reply.value.is_empty() {
                        let text = String::from_utf8(std::mem::take(&mut pending.bytes))?;
                        self.finish(conn, Ok(Some(text)));
                    } else {
                        pending.bytes.extend(reply.value);
                    }
                }
            }
            _ => {}
        }
        conn.flush()?;
        Ok(())
    }
}
fn chord(conn: &RustConnection, keys: &mut Keyboard, letter: u8) -> anyhow::Result<()> {
    let held = keys.held();
    keys.hold(
        conn,
        Held {
            control: true,
            ..Held::default()
        },
    )?;
    keys.stroke(conn, u32::from(letter))?;
    keys.hold(conn, held)?;
    Ok(())
}
