//! The question an agent asks of a folder it has not been run in - whether it trusts it -
//! answered for an agent nobody is at the window for: one started by `moon agent start`.
//!
//! The question comes up before the agent has a box, with the choice that refuses
//! highlighted, and a line the agent is told ends in an Enter - see [`super::told`]. Told to
//! an agent still asking, the Enter would be the answer, and the agent would leave. Starting
//! an agent in a folder from a command line is what says the folder is trusted, so the answer
//! is given here: the screen is read as the agent comes up, and when the question is on it
//! the keys that say yes are pressed. The lines the shell is told wait until it has been
//! looked at.
//!
//! A question is known by how it reads on the screen, down to where the highlight is, since
//! that is what the keys start from. An agent that words it any other way is asked nothing
//! and has nothing pressed.

use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

use anyhow::{Result, anyhow};

use super::{Shown, TerminalRegistry, TerminalSession};
use crate::api::AgentKind;

/// How one agent asks whether it trusts the folder, and how it is told that it does.
struct TrustQuestion {
    agent: AgentKind,
    /// The rows of the screen that are the question as it comes up: the choice that refuses
    /// highlighted.
    asked_as: &'static str,
    /// The key that moves the highlight from there to the choice that trusts.
    moves_to_yes: &'static [u8],
    /// The row of the screen that is that choice, highlighted.
    on_yes_as: &'static str,
    /// The key that takes the highlighted choice.
    takes_it: &'static [u8],
}

const TRUST_QUESTIONS: &[TrustQuestion] = &[TrustQuestion {
    agent: AgentKind::Claude,
    asked_as: "❯ No, exit\n   Yes, I trust this folder",
    moves_to_yes: b"\x1b[B",
    on_yes_as: "❯ Yes, I trust this folder",
    takes_it: b"\r",
}];

/// How long an agent is given to come up with the question or with anything else, and to
/// have it answered.
const COMES_UP_WITHIN: Duration = Duration::from_secs(10);

/// How long an agent has to have printed nothing, with something on its screen, for what is
/// on its screen to be what it came up with - and for it to be reading keys: a key written as
/// the question is first painted is dropped. Longer than the wait the card's title makes -
/// `TYPE_AHEAD_QUIET` - because nothing is lost by it: nobody is at the window to type first.
const HAS_COME_UP_ONCE_QUIET_FOR: Duration = Duration::from_millis(500);

/// How often the screen is read while the agent comes up.
const LOOKS_AGAIN_AFTER: Duration = Duration::from_millis(100);

impl TerminalRegistry {
    /// Have the agent in this shell told that it trusts its folder, should it ask as it comes
    /// up - see [this module](self). Answers at once; the lines the shell is told wait until
    /// its screen has been looked at.
    pub(crate) fn trust_its_folder(&self, terminal_id: &str) -> Result<()> {
        let session = self
            .get(terminal_id)
            .ok_or_else(|| anyhow!("unknown terminal {terminal_id}"))?;
        let Some(question) = TRUST_QUESTIONS
            .iter()
            .find(|question| Some(question.agent) == session.program.agent())
        else {
            return Ok(());
        };
        session.trust_unanswered.store(true, Ordering::Relaxed);
        let looked_at = Arc::clone(&session);
        std::thread::spawn(move || {
            answer_as_it_comes_up(&looked_at, question);
            looked_at.trust_unanswered.store(false, Ordering::Relaxed);
        });
        Ok(())
    }
}

/// Read the agent's screen until it has come up, and say yes to the question if that is what
/// it came up with.
///
/// Every key is pressed for what the screen shows at that moment, never for what the key
/// before it should have done: the choice is only taken once it is the one that trusts that
/// is seen highlighted, so a key the agent dropped cannot turn the answer into a refusal. A
/// key counts as a person's - see [`TerminalSession::write_input`] - so a line the shell is
/// told next waits a moment behind the last one, by which time the agent has put its box up.
fn answer_as_it_comes_up(session: &TerminalSession, question: &TrustQuestion) {
    while !session.has_ended() && session.started.elapsed() < COMES_UP_WITHIN {
        // A screen that cannot be read is one with no question to be seen on it.
        let Ok(screen) = session.shown(Shown::Screen) else {
            return;
        };
        let settled = !screen.is_empty()
            && session
                .last_output
                .lock()
                .unwrap()
                .is_some_and(|last| last.elapsed() >= HAS_COME_UP_ONCE_QUIET_FOR);
        if settled {
            let pressed = if screen.contains(question.on_yes_as) {
                question.takes_it
            } else if screen.contains(question.asked_as) {
                question.moves_to_yes
            } else {
                // It came up with something else: its box, most likely.
                return;
            };
            // A write that fails is a shell that ended a moment ago.
            if session.write_input(pressed).is_err() || pressed == question.takes_it {
                return;
            }
        }
        std::thread::sleep(LOOKS_AGAIN_AFTER);
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::{
        sync::Mutex,
        time::{Duration, Instant},
    };

    use super::*;
    use crate::terminal::tests::spawn_fake_claude;

    /// Asks what Claude asks of a folder it has not been run in, the way Claude does: a key at
    /// a time, the first of them dropped as a key written while it is still painting is, Down
    /// moving the highlight to the choice that trusts, and Enter - which reaches it as an
    /// empty read - taking whichever choice is highlighted: leaving, on the one that refuses. Trusted, it says back each line it is
    /// sent as `got[line]`.
    const ASKS_WHETHER_IT_TRUSTS_THE_FOLDER: &str = "#!/bin/sh\nPATH=/usr/bin:/bin\n\
         stty -icanon -echo min 1\n\
         printf ' ❯ No, exit\\n   Yes, I trust this folder\\n'\n\
         on=no\n\
         dropped=$(dd bs=8 count=1 2>/dev/null)\n\
         while :; do\n\
           key=$(dd bs=8 count=1 2>/dev/null)\n\
           case \"$key\" in\n\
             *'[B') on=yes; printf '   No, exit\\n ❯ Yes, I trust this folder\\n';;\n\
             '') break;;\n\
           esac\n\
         done\n\
         if [ $on = no ]; then printf 'left\\n'; exit 1; fi\n\
         stty sane\n\
         printf 'trusted\\n'\n\
         while IFS= read -r line; do printf 'got[%s]\\n' \"$line\"; done\n";

    /// Comes up with a box and no question, and says back each line it is sent.
    const COMES_UP_WITH_ITS_BOX: &str = "#!/bin/sh\nprintf '> '\n\
         while IFS= read -r line; do printf 'got[%s]\\n' \"$line\"; done\n";

    fn registry() -> Arc<TerminalRegistry> {
        Arc::new(TerminalRegistry::new(Arc::new(Mutex::new(Instant::now()))))
    }

    fn printed(session: &TerminalSession) -> String {
        String::from_utf8_lossy(&session.scrollback.lock().unwrap().replay()).to_string()
    }

    fn prints(session: &TerminalSession, wanted: &str) -> bool {
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline && !printed(session).contains(wanted) {
            std::thread::sleep(Duration::from_millis(20));
        }
        printed(session).contains(wanted)
    }

    /// What `moon agent start` and then `moon agent tell` come to, in a folder the agent has
    /// not been run in: the question is answered yes, and the line told meanwhile is typed
    /// into the agent's box after it rather than being the answer.
    #[test]
    fn an_agent_asking_whether_it_trusts_the_folder_is_told_yes_before_any_line() {
        let registry = registry();
        let terminal_id = spawn_fake_claude(&registry, ASKS_WHETHER_IT_TRUSTS_THE_FOLDER);
        let session = registry.get(&terminal_id).expect("expected the shell");

        registry
            .trust_its_folder(&terminal_id)
            .expect("expected the shell to be looked at");
        registry
            .tell(&terminal_id, "hello")
            .expect("expected the shell to be told");

        assert!(
            prints(&session, "got[hello]"),
            "the line should have reached the trusted agent, printed {:?}",
            printed(&session)
        );
        let printed = printed(&session);
        assert!(printed.contains("trusted"), "printed {printed:?}");
        assert!(!printed.contains("left"), "printed {printed:?}");
        registry.remove(&terminal_id);
    }

    /// An agent that comes up with its box has nothing pressed in it, and is told its line
    /// once it has been seen to have come up.
    #[test]
    fn an_agent_that_asks_nothing_has_nothing_pressed() {
        let registry = registry();
        let terminal_id = spawn_fake_claude(&registry, COMES_UP_WITH_ITS_BOX);
        let session = registry.get(&terminal_id).expect("expected the shell");

        registry
            .trust_its_folder(&terminal_id)
            .expect("expected the shell to be looked at");
        registry
            .tell(&terminal_id, "hello")
            .expect("expected the shell to be told");

        assert!(
            prints(&session, "got[hello]"),
            "printed {:?}",
            printed(&session)
        );
        assert!(!session.has_been_typed_into(), "no key should be pressed");
        assert!(!session.trust_unanswered.load(Ordering::Relaxed));
        registry.remove(&terminal_id);
    }
}
