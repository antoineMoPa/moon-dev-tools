//! Real selection integration. Requires Xvfb, xclip and Chromium on Linux.
#![cfg(target_os = "linux")]
use moon_display::{
    CLIPBOARD_LIMIT, ClipboardAction, ClipboardRequest, Display, Event, Held, Input, Size,
};
use std::{
    io::Write,
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::Duration,
};
struct Program(Child);
impl Drop for Program {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn transfer(display: &Display, action: ClipboardAction) -> Result<Option<String>, String> {
    let (reply, result) = mpsc::channel();
    display.send(Input::Clipboard(ClipboardRequest { action, reply }));
    result
        .recv_timeout(Duration::from_secs(4))
        .expect("clipboard always answers")
}
fn read(display: &Display) -> String {
    let output = Command::new("xclip")
        .args(["-selection", "clipboard", "-o"])
        .env("DISPLAY", display.name())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}
fn write(display: &Display, text: &str) -> Program {
    let mut child = Command::new("xclip")
        .args(["-selection", "clipboard", "-quiet"])
        .env("DISPLAY", display.name())
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(text.as_bytes())
        .unwrap();
    Program(child)
}
#[test]
fn paste_owns_utf8_selection_and_rejects_oversized_text() {
    let (display, _events) = Display::start(Size {
        width: 320,
        height: 200,
    })
    .unwrap();
    let text = "hello\n世界\t🦀 café";
    assert_eq!(
        transfer(&display, ClipboardAction::Paste(text.into())).unwrap(),
        None
    );
    assert_eq!(read(&display), text);
    // A large ordinary selection exceeds X11's traditional 64 KiB boundary but fits ours.
    let large = "é".repeat(CLIPBOARD_LIMIT / 2);
    transfer(&display, ClipboardAction::Paste(large.clone())).unwrap();
    assert_eq!(read(&display), large);
    assert!(
        transfer(
            &display,
            ClipboardAction::Paste("x".repeat(CLIPBOARD_LIMIT + 1))
        )
        .unwrap_err()
        .contains("64 KiB")
    );
    assert_eq!(read(&display), large);
}
#[test]
fn empty_copy_times_out_without_destroying_remote_clipboard_or_stalling_screen() {
    let (display, events) = Display::start(Size {
        width: 320,
        height: 200,
    })
    .unwrap();
    let _owner = write(&display, "previous remote text");
    for _ in 0..100 {
        if Command::new("xclip")
            .args(["-selection", "clipboard", "-o"])
            .env("DISPLAY", display.name())
            .output()
            .is_ok_and(|o| o.stdout == b"previous remote text")
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let (reply, result) = mpsc::channel();
    display.send(Input::Clipboard(ClipboardRequest {
        action: ClipboardAction::Copy { cut: false },
        reply,
    }));
    display.send(Input::Resized(Size {
        width: 400,
        height: 300,
    }));
    let end = std::time::Instant::now() + Duration::from_secs(1);
    loop {
        match events
            .recv_timeout(end.saturating_duration_since(std::time::Instant::now()))
            .unwrap()
        {
            Event::Drawn(patch) if patch.view.width == 400 => break,
            _ => {}
        }
    }
    assert!(
        result
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
            .is_err()
    );
    assert_eq!(read(&display), "previous remote text");
}
#[test]
fn forwarded_chromium_pastes_copies_and_cuts_multiline_utf8() {
    let (display, events) = Display::start(Size {
        width: 800,
        height: 600,
    })
    .unwrap();
    let fixture = std::env::temp_dir().join(format!("moon-clipboard-{}.html", display.number()));
    std::fs::write(&fixture, "<!doctype html><meta charset=utf-8><textarea autofocus style='width:90vw;height:90vh'></textarea><script>window.onload=()=>{document.querySelector('textarea').focus();document.title='Moon clipboard ready';}</script>").unwrap();
    let profile = std::env::temp_dir().join(format!("moon-chromium-{}", display.number()));
    let _browser = Program(
        Command::new("chromium")
            .env("DISPLAY", display.name())
            .args([
                "--no-sandbox",
                "--disable-dev-shm-usage",
                "--no-first-run",
                "--disable-gpu",
            ])
            .arg(format!("--user-data-dir={}", profile.display()))
            .arg(format!("--app=file://{}", fixture.display()))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("Chromium installed"),
    );
    loop {
        if matches!(events.recv_timeout(Duration::from_secs(20)).unwrap(), Event::WindowsOpen(n) if n > 0)
        {
            break;
        }
    }
    wait_for_chromium_ready(&display);
    let text = "Moon clipboard\n世界 🦀 café\nlast line";
    transfer(&display, ClipboardAction::Paste(text.into())).unwrap();
    std::thread::sleep(Duration::from_millis(150));
    select_all(&display);
    assert_eq!(
        transfer(&display, ClipboardAction::Copy { cut: false }).unwrap(),
        Some(text.into())
    );
    select_all(&display);
    assert_eq!(
        transfer(&display, ClipboardAction::Copy { cut: true }).unwrap(),
        Some(text.into())
    );
    // A selectionless copy never returns the previous cut.
    assert!(transfer(&display, ClipboardAction::Copy { cut: false }).is_err());
    transfer(&display, ClipboardAction::Paste("after cut".into())).unwrap();
    std::thread::sleep(Duration::from_millis(150));
    select_all(&display);
    assert_eq!(
        transfer(&display, ClipboardAction::Copy { cut: false }).unwrap(),
        Some("after cut".into())
    );
    let _ = std::fs::remove_file(fixture);
}
fn select_all(display: &Display) {
    display.send(Input::Holding(Held {
        control: true,
        ..Held::default()
    }));
    display.send(Input::Stroke {
        keysym: u32::from(b'a'),
    });
    display.send(Input::Holding(Held::default()));
}

#[test]
fn fresh_remote_incremental_selection_is_bounded_and_decoded() {
    let (display, _events) = Display::start(Size {
        width: 320,
        height: 200,
    })
    .unwrap();
    for text in ["世界🦀\n".repeat(4000), "x".repeat(CLIPBOARD_LIMIT + 1)] {
        let (reply, result) = mpsc::channel();
        display.send(Input::Clipboard(ClipboardRequest {
            action: ClipboardAction::Copy { cut: false },
            reply,
        }));
        // Simulate the focused application's delayed copy selection using xclip. Its large
        // selection uses INCR, exercising both the streaming UTF-8 path and its byte limit.
        std::thread::sleep(Duration::from_millis(50));
        let _owner = write(&display, &text);
        let result = result.recv_timeout(Duration::from_secs(4)).unwrap();
        if text.len() <= CLIPBOARD_LIMIT {
            assert_eq!(result.unwrap(), Some(text));
        } else {
            assert!(result.unwrap_err().contains("64 KiB"));
        }
    }
}

#[test]
fn timed_out_selection_events_cannot_complete_a_later_copy() {
    use x11rb::{
        connection::Connection,
        protocol::{
            Event as XEvent,
            xproto::{
                ConnectionExt, CreateWindowAux, EventMask, PropMode, SELECTION_NOTIFY_EVENT,
                SelectionNotifyEvent, WindowClass,
            },
        },
        wrapper::ConnectionExt as _,
    };
    let (display, _events) = Display::start(Size {
        width: 320,
        height: 200,
    })
    .unwrap();
    let (conn, screen) = x11rb::connect(Some(&display.name())).unwrap();
    let owner = conn.generate_id().unwrap();
    conn.create_window(
        x11rb::COPY_DEPTH_FROM_PARENT,
        owner,
        conn.setup().roots[screen].root,
        -100,
        -100,
        1,
        1,
        0,
        WindowClass::INPUT_OUTPUT,
        x11rb::COPY_FROM_PARENT,
        &CreateWindowAux::new(),
    )
    .unwrap()
    .check()
    .unwrap();
    let clipboard = conn
        .intern_atom(false, b"CLIPBOARD")
        .unwrap()
        .reply()
        .unwrap()
        .atom;
    let begin = || {
        let (reply, result) = mpsc::channel();
        display.send(Input::Clipboard(ClipboardRequest {
            action: ClipboardAction::Copy { cut: false },
            reply,
        }));
        std::thread::sleep(Duration::from_millis(50));
        conn.set_selection_owner(owner, clipboard, x11rb::CURRENT_TIME)
            .unwrap()
            .check()
            .unwrap();
        result
    };
    let request = || {
        let until = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(XEvent::SelectionRequest(request)) = conn.poll_for_event().unwrap() {
                return request;
            }
            assert!(
                std::time::Instant::now() < until,
                "copy requested fresh selection"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    let first = begin();
    let stale = request();
    assert!(first.recv_timeout(Duration::from_secs(3)).unwrap().is_err());
    let second = begin();
    let current = request();
    assert_ne!(
        stale.requestor, current.requestor,
        "each copy owns its X requestor"
    );
    // An owner retaining the timed-out request cannot write to its destroyed requestor.
    assert!(
        conn.change_property8(
            PropMode::REPLACE,
            stale.requestor,
            stale.property,
            stale.target,
            b"stale"
        )
        .unwrap()
        .check()
        .is_err()
    );
    // Even a delayed queued notification delivered to this connection carries the old
    // requestor. It must not complete the newer transfer or trigger a property read.
    let notify = |request: &x11rb::protocol::xproto::SelectionRequestEvent| SelectionNotifyEvent {
        response_type: SELECTION_NOTIFY_EVENT,
        sequence: 0,
        time: request.time,
        requestor: request.requestor,
        selection: request.selection,
        target: request.target,
        property: request.property,
    };
    conn.send_event(
        false,
        current.requestor,
        EventMask::NO_EVENT,
        notify(&stale),
    )
    .unwrap()
    .check()
    .unwrap();
    conn.flush().unwrap();
    assert!(second.recv_timeout(Duration::from_millis(50)).is_err());
    conn.change_property8(
        PropMode::REPLACE,
        current.requestor,
        current.property,
        current.target,
        b"fresh text",
    )
    .unwrap()
    .check()
    .unwrap();
    conn.send_event(
        false,
        current.requestor,
        EventMask::NO_EVENT,
        notify(&current),
    )
    .unwrap()
    .check()
    .unwrap();
    conn.flush().unwrap();
    assert_eq!(
        second
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap(),
        Some("fresh text".into())
    );
}

fn wait_for_chromium_ready(display: &Display) {
    use x11rb::{
        connection::Connection,
        protocol::xproto::{AtomEnum, ConnectionExt},
    };
    let (conn, screen) = x11rb::connect(Some(&display.name())).unwrap();
    let root = conn.setup().roots[screen].root;
    let name = conn
        .intern_atom(false, b"_NET_WM_NAME")
        .unwrap()
        .reply()
        .unwrap()
        .atom;
    let until = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        for window in conn.query_tree(root).unwrap().reply().unwrap().children {
            if let Ok(reply) = conn
                .get_property(false, window, name, AtomEnum::ANY, 0, 256)
                .unwrap()
                .reply()
            {
                if String::from_utf8_lossy(&reply.value).contains("Moon clipboard ready") {
                    return;
                }
            }
        }
        assert!(
            std::time::Instant::now() < until,
            "Chromium focused its fixture textarea and updated its title"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}
