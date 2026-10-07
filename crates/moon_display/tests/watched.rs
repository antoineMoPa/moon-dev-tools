//! Integration test of a real display - needs Xvfb, and `xlogo` from x11-apps to draw on it.
#![cfg(target_os = "linux")]

use std::{
    process::{Child, Command},
    sync::mpsc::Receiver,
    time::{Duration, Instant},
};

use moon_display::{Display, Event, Input, Patch, Size};

const VIEW: Size = Size {
    width: 320,
    height: 200,
};

struct Program(Child);

impl Drop for Program {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// The first event within a few seconds that `wanted` has an answer for.
fn wait_for<T>(events: &Receiver<Event>, wanted: impl Fn(Event) -> Option<T>) -> T {
    let give_up = Instant::now() + Duration::from_secs(10);
    loop {
        let left = give_up.saturating_duration_since(Instant::now());
        let event = events
            .recv_timeout(left)
            .expect("the display never did what was waited for");
        if let Some(found) = wanted(event) {
            return found;
        }
    }
}

/// The first patch that `wanted` says yes to.
fn wait_for_patch(events: &Receiver<Event>, wanted: impl Fn(&Patch) -> bool) -> Patch {
    wait_for(events, |event| match event {
        Event::Drawn(patch) if wanted(&patch) => Some(patch),
        _ => None,
    })
}

fn wait_for_windows(events: &Receiver<Event>, open: usize) {
    wait_for(events, |event| {
        (event == Event::WindowsOpen(open)).then_some(())
    });
}

fn whole(view: Size) -> impl Fn(&Patch) -> bool {
    move |patch| patch.view == view && patch.size == view
}

#[test]
fn a_program_s_window_fills_the_view_and_follows_it() {
    let (display, patches) = Display::start(VIEW).expect("Xvfb is installed");

    // Before anything is drawn, the display shows its empty screen, all of the view.
    let empty = wait_for_patch(&patches, whole(VIEW));
    assert_eq!(empty.rgb.len(), 320 * 200 * 3);

    let logo = Program(
        Command::new("xlogo")
            .env("DISPLAY", display.name())
            .spawn()
            .expect("xlogo is installed"),
    );
    // The logo is drawn across its whole window, and its window is the whole view: what
    // changed reaches all four edges, give or take the logo's own margin.
    wait_for_windows(&patches, 1);
    let drawn = wait_for_patch(&patches, |patch| {
        patch.size.width > 300 && patch.size.height > 180
    });
    assert!(
        drawn
            .rgb
            .chunks_exact(3)
            .any(|pixel| pixel != &empty.rgb[..3])
    );

    let larger = Size {
        width: 400,
        height: 300,
    };
    display.send(Input::Resized(larger));
    wait_for_patch(&patches, whole(larger));
    let shown = display.shown().expect("the display has been looked at");
    assert_eq!(shown.size, larger);
    assert_eq!(shown.rgb.len(), 400 * 300 * 3);

    // The program ending is its window closing, which is said too.
    drop(logo);
    wait_for_windows(&patches, 0);
}
