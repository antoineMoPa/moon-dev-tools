//! Every keyboard shortcut the window answers to, in one table.
//!
//! The table is data — an action, the keys that fire it, and how far it reaches — so adding a
//! shortcut is one row rather than another branch inside the key handler, and so the whole
//! set can be shown to the user or read out of a config file later without the handler
//! changing at all.
//!
//! Chords can be more than one press. `C-x o` is `Ctrl+X` followed by a bare `o`, which is
//! how emacs-style prefixes work: the first press arms the map, the second one resolves it.
//! `M-x` prefixes fit the same shape, with Alt on the first press.

use egui::{Key, Modifiers};

/// Something the window can be asked to do. The app decides what each one means; this file
/// only decides what fires it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Action {
    OpenPalette,
    NewShellTab,
    CloseTab,
    SaveFile,
    ToggleTheme,
    /// Stage the hunk under the caret, or mark it reviewed in a review that cannot be staged.
    AdvanceHunk,
    /// The reverse: unstage it, or mark it unreviewed.
    ReverseHunk,
    /// Move the keyboard to the next frame of the workspace.
    FocusNextFrame,
    /// Open the find bar over whichever pane has the keyboard.
    Find,
}

/// One press: a key and the modifiers held with it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Press {
    pub(crate) mods: Modifiers,
    pub(crate) key: Key,
}

const fn press(mods: Modifiers, key: Key) -> Press {
    Press { mods, key }
}

/// How far a binding reaches.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Reach {
    /// Answered whoever has the keyboard, shells included. Only for chords a terminal
    /// program would not otherwise be sent.
    Anywhere,
    /// Answered only when the keyboard is not in a shell or a text box, where a bare letter
    /// is text rather than a command.
    WhenNotTyping,
}

pub(crate) struct Binding {
    pub(crate) action: Action,
    /// The presses in order. One for a plain shortcut, more for a prefixed chord.
    pub(crate) chord: &'static [Press],
    pub(crate) reach: Reach,
}

const COMMAND_SHIFT: Modifiers = Modifiers::COMMAND.plus(Modifiers::SHIFT);

/// The whole keyboard, laid out as values. Order matters only in that the first exact match
/// wins, which cannot happen while every chord here is distinct.
pub(crate) const BINDINGS: &[Binding] = &[
    Binding {
        action: Action::OpenPalette,
        chord: &[press(COMMAND_SHIFT, Key::P)],
        reach: Reach::Anywhere,
    },
    Binding {
        action: Action::NewShellTab,
        chord: &[press(Modifiers::COMMAND, Key::T)],
        reach: Reach::Anywhere,
    },
    Binding {
        action: Action::CloseTab,
        chord: &[press(Modifiers::COMMAND, Key::W)],
        reach: Reach::Anywhere,
    },
    Binding {
        action: Action::SaveFile,
        chord: &[press(Modifiers::COMMAND, Key::S)],
        reach: Reach::Anywhere,
    },
    Binding {
        action: Action::ToggleTheme,
        chord: &[press(Modifiers::COMMAND, Key::J)],
        reach: Reach::Anywhere,
    },
    Binding {
        action: Action::Find,
        chord: &[press(Modifiers::COMMAND, Key::F)],
        reach: Reach::Anywhere,
    },
    // Ctrl+X is a prefix here, so a program running in a shell no longer gets it. That is the
    // trade this shortcut makes: leaving a shell has to be possible from inside one.
    Binding {
        action: Action::FocusNextFrame,
        chord: &[press(Modifiers::CTRL, Key::X), press(Modifiers::NONE, Key::O)],
        reach: Reach::Anywhere,
    },
    Binding {
        action: Action::AdvanceHunk,
        chord: &[press(Modifiers::NONE, Key::S)],
        reach: Reach::WhenNotTyping,
    },
    Binding {
        action: Action::ReverseHunk,
        chord: &[press(Modifiers::NONE, Key::U)],
        reach: Reach::WhenNotTyping,
    },
];

/// What a press did to the map.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Step {
    /// It finished a chord.
    Fired(Action),
    /// It began one. The press belongs to the map and must not reach the pane below.
    Armed,
    /// Nothing here wants it.
    Passed,
}

/// The keyboard's running state: which prefix, if any, is waiting for its next press.
#[derive(Default)]
pub(crate) struct Keymap {
    /// The presses of a chord that has begun but not finished — the `C-x` of `C-x o`.
    armed: Vec<Press>,
}

impl Keymap {
    /// Take this frame's key presses and return the actions they fired.
    ///
    /// Presses the map claims are removed from the input, so a shell below does not also
    /// receive them. `typing` says whether the keyboard is in a shell or a text box, which
    /// is what holds back the bare-letter bindings.
    pub(crate) fn resolve(&mut self, ctx: &egui::Context, typing: bool) -> Vec<Action> {
        ctx.input_mut(|input| {
            let mut fired = Vec::new();
            let mut kept = Vec::with_capacity(input.events.len());
            // The platform sends the character a key produced right after the key itself, so
            // a claimed press has to take its text with it or the letter would still be typed.
            let mut drop_next_text = false;

            for event in std::mem::take(&mut input.events) {
                if let egui::Event::Text(_) = &event
                    && std::mem::take(&mut drop_next_text)
                {
                    continue;
                }
                let Some(press) = press_of(&event) else {
                    kept.push(event);
                    continue;
                };

                match self.step(press, typing) {
                    Step::Fired(action) => {
                        fired.push(action);
                        drop_next_text = true;
                    }
                    Step::Armed => drop_next_text = true,
                    Step::Passed => {
                        drop_next_text = false;
                        kept.push(event);
                    }
                }
            }

            input.events = kept;
            fired
        })
    }

    /// Whether a prefix is waiting for the press that completes it, which the UI says so.
    pub(crate) fn armed_prefix(&self) -> Option<&[Press]> {
        (!self.armed.is_empty()).then_some(&self.armed)
    }

    fn step(&mut self, press: Press, typing: bool) -> Step {
        let mut chord = std::mem::take(&mut self.armed);
        chord.push(press);

        for binding in BINDINGS {
            if binding.reach == Reach::WhenNotTyping && typing {
                continue;
            }
            if matches(binding.chord, &chord) {
                return Step::Fired(binding.action);
            }
            // A longer chord starts with this one, so the map waits for the rest of it.
            if binding.chord.len() > chord.len() && matches(&binding.chord[..chord.len()], &chord) {
                self.armed = chord;
                return Step::Armed;
            }
        }

        Step::Passed
    }
}

fn matches(chord: &[Press], pressed: &[Press]) -> bool {
    chord.len() == pressed.len()
        && chord
            .iter()
            .zip(pressed)
            .all(|(want, got)| want.key == got.key && got.mods.matches_exact(want.mods))
}

fn press_of(event: &egui::Event) -> Option<Press> {
    match event {
        egui::Event::Key {
            key,
            pressed: true,
            modifiers,
            ..
        } => Some(Press {
            mods: *modifiers,
            key: *key,
        }),
        _ => None,
    }
}

/// A chord written the way it is spoken: `⌘⇧P`, or `C-x o` for a prefixed one.
pub(crate) fn describe(chord: &[Press]) -> String {
    chord
        .iter()
        .map(describe_press)
        .collect::<Vec<_>>()
        .join(" ")
}

fn describe_press(press: &Press) -> String {
    // A chord with a command modifier is a platform shortcut and reads as ⌘; one built on
    // Ctrl or Alt is an emacs-style chord and reads the way emacs writes it.
    //
    // ⌘ is the only modifier glyph the bundled fonts carry, so the others are spelled out
    // rather than drawn as empty boxes.
    if press.mods.command || press.mods.mac_cmd {
        let mut out = String::new();
        for (held, word) in [
            (press.mods.ctrl, "ctrl "),
            (press.mods.alt, "alt "),
            (press.mods.shift, "shift "),
        ] {
            if held {
                out.push_str(word);
            }
        }
        out.push('⌘');
        out.push_str(press.key.name());
        return out;
    }

    let mut out = String::new();
    if press.mods.ctrl {
        out.push_str("C-");
    }
    if press.mods.alt {
        out.push_str("M-");
    }
    if press.mods.shift {
        out.push_str("S-");
    }
    out.push_str(&press.key.name().to_lowercase());
    out
}

/// The chord that fires an action, for anything that shows the keyboard to the user.
pub(crate) fn chord_of(action: Action) -> Option<&'static [Press]> {
    BINDINGS
        .iter()
        .find(|binding| binding.action == action)
        .map(|binding| binding.chord)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key_event(mods: Modifiers, key: Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: mods,
        }
    }

    /// Drive the map over a run of events, the way a frame's input would.
    fn run(keymap: &mut Keymap, typing: bool, events: Vec<egui::Event>) -> (Vec<Action>, Vec<egui::Event>) {
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            events,
            focused: true,
            ..Default::default()
        };
        let fired;
        let left;
        ctx.begin_pass(input);
        fired = keymap.resolve(&ctx, typing);
        left = ctx.input(|input| input.events.clone());
        ctx.end_pass().drop_without_applying_deltas();
        (fired, left)
    }

    #[test]
    fn a_plain_chord_fires_and_is_taken_out_of_the_input() {
        let mut keymap = Keymap::default();
        let (fired, left) = run(
            &mut keymap,
            false,
            vec![key_event(Modifiers::COMMAND, Key::T)],
        );

        assert_eq!(fired, vec![Action::NewShellTab]);
        assert!(left.is_empty(), "the pane below must not see it too");
    }

    #[test]
    fn a_prefix_waits_for_the_press_that_completes_it() {
        let mut keymap = Keymap::default();

        let (fired, left) = run(&mut keymap, false, vec![key_event(Modifiers::CTRL, Key::X)]);
        assert!(fired.is_empty(), "C-x on its own does nothing yet");
        assert!(left.is_empty(), "but it is the map's, not the shell's");
        assert!(keymap.armed_prefix().is_some(), "the map is armed");

        let (fired, _) = run(&mut keymap, false, vec![key_event(Modifiers::NONE, Key::O)]);
        assert_eq!(fired, vec![Action::FocusNextFrame]);
        assert!(keymap.armed_prefix().is_none(), "the prefix is spent");
    }

    #[test]
    fn a_prefix_that_leads_nowhere_is_dropped_rather_than_kept() {
        let mut keymap = Keymap::default();
        run(&mut keymap, false, vec![key_event(Modifiers::CTRL, Key::X)]);

        let (fired, left) = run(&mut keymap, false, vec![key_event(Modifiers::NONE, Key::Z)]);
        assert!(fired.is_empty());
        assert_eq!(left.len(), 1, "the stray press is the pane's to deal with");
        assert!(keymap.armed_prefix().is_none(), "and the map is disarmed");
    }

    #[test]
    fn the_letter_a_chord_used_is_not_typed_as_well() {
        let mut keymap = Keymap::default();
        run(&mut keymap, false, vec![key_event(Modifiers::CTRL, Key::X)]);

        let (fired, left) = run(
            &mut keymap,
            false,
            vec![
                key_event(Modifiers::NONE, Key::O),
                egui::Event::Text("o".to_string()),
            ],
        );
        assert_eq!(fired, vec![Action::FocusNextFrame]);
        assert!(left.is_empty(), "the 'o' must not also reach the shell");
    }

    #[test]
    fn bare_letters_are_text_while_a_shell_has_the_keyboard() {
        let mut keymap = Keymap::default();
        let (fired, left) = run(&mut keymap, true, vec![key_event(Modifiers::NONE, Key::S)]);

        assert!(fired.is_empty(), "`s` in a shell is the letter s");
        assert_eq!(left.len(), 1);

        let (fired, _) = run(&mut keymap, false, vec![key_event(Modifiers::NONE, Key::S)]);
        assert_eq!(fired, vec![Action::AdvanceHunk]);
    }

    #[test]
    fn command_chords_still_reach_the_app_from_inside_a_shell() {
        let mut keymap = Keymap::default();
        let (fired, _) = run(&mut keymap, true, vec![key_event(Modifiers::COMMAND, Key::W)]);

        assert_eq!(fired, vec![Action::CloseTab]);
    }

    /// ⌘⇧P must not also be read as the ⌘P that no binding claims, and ⌘S must not fire
    /// while Shift is held.
    #[test]
    fn modifiers_have_to_match_exactly() {
        let mut keymap = Keymap::default();
        let (fired, _) = run(&mut keymap, false, vec![key_event(COMMAND_SHIFT, Key::S)]);
        assert!(fired.is_empty(), "⌘⇧S is not ⌘S");

        let (fired, _) = run(&mut keymap, false, vec![key_event(COMMAND_SHIFT, Key::P)]);
        assert_eq!(fired, vec![Action::OpenPalette]);
    }

    #[test]
    fn chords_read_the_way_they_are_spoken() {
        assert_eq!(
            describe(chord_of(Action::FocusNextFrame).expect("bound")),
            "C-x o"
        );
        assert_eq!(
            describe(chord_of(Action::OpenPalette).expect("bound")),
            "shift ⌘P"
        );
    }
}
