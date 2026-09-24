//! The three frames a window opens on, and the words each goes by.
//!
//! Apart from [`super::command`] because the window in a browser is one of these frames too,
//! and it has no command line to be started from.

/// What the window opens on, which is the whole difference between the three windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Frame {
    /// `moon review`: the review of the repo.
    Review,
    /// `moon tasks`: the task board, and the agents working through it.
    Tasks,
    /// `moon shell`: a shell in the folder, which need not be a repo.
    Shell,
}

/// Every frame, in the order they are named in help and given launchers.
pub(crate) const FRAMES: &[Frame] = &[Frame::Review, Frame::Tasks, Frame::Shell];

/// The same three in the order a window offers to open another one, which is not the order
/// they are written about in: the board comes first, because a new window is usually a new
/// piece of work rather than a second look at this one.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) const NEW_WINDOW_FRAMES: &[Frame] = &[Frame::Tasks, Frame::Review, Frame::Shell];

/// The one executable all of this is: the three windows, the server behind them, and the
/// commands that reach a window which is already open.
pub(crate) const PROGRAM: &str = "moon";

/// How a launcher says which window it is.
///
/// A desktop entry runs `moon <subcommand>` and needs none of this, but a macOS bundle runs
/// its executable with no arguments at all - and with the executable a link to the installed
/// `moon`, which the OS resolves before starting it, so not even the name it was started
/// under says which window was asked for. What a bundle *can* carry is `LSEnvironment`, so
/// that is where its window is written; see [`crate::native::launchers`].
#[cfg(not(target_arch = "wasm32"))]
pub(crate) const FRAME_ENV: &str = "MOON_FRAME";

/// Everything that differs between the three frames in name and wording, kept in one place so
/// a new frame is a row here rather than a branch wherever text is written.
struct FrameProgram {
    frame: Frame,
    /// The word after `moon` that opens a window on this frame.
    subcommand: &'static str,
    /// The name this frame goes by wherever a name has to be one token: its icon and
    /// launcher files, and its bundle identifier.
    slug: &'static str,
    /// The name a desktop launcher shows: the one the OS puts under the icon. This and the
    /// next are only read by the launchers and the CLI's help, which a browser has neither of.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    display_name: &'static str,
    /// What the window opens on, as one line of prose, for the CLI's help.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    opens: &'static str,
    /// How the launch screen asks which repo to open.
    asks_for_repo: &'static str,
    /// The same, when the repo is on the far side of a remote connection and can only be
    /// typed out.
    asks_for_remote_repo: &'static str,
    /// What the launch screen's folder picker button says. A review needs a repo and asks
    /// for one; the other two are as much use in a folder git knows nothing about.
    picker_button: &'static str,
    /// What the launch screen's button says.
    opens_button: &'static str,
    /// What the screen between that button and the open window says it is doing.
    opening: &'static str,
    /// Whether this frame is any use in a folder git knows nothing about, which decides what
    /// a window does when the folder it was started in is in no repo: a shell and a task
    /// board just open on it, and a review - which is made entirely out of what git knows -
    /// asks for a repo instead. Only a command line has a folder it was started in.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    opens_without_a_repo: bool,
}

const FRAME_PROGRAMS: &[FrameProgram] = &[
    FrameProgram {
        frame: Frame::Review,
        subcommand: "review",
        slug: "moonreview",
        display_name: "Moonreview",
        opens: "a review of the repo",
        asks_for_repo: "Which repo to review:",
        asks_for_remote_repo: "Path of the repo to review, on that machine:",
        picker_button: "Choose a repo…",
        opens_button: "Open review",
        opening: "opening the review…",
        opens_without_a_repo: false,
    },
    FrameProgram {
        frame: Frame::Tasks,
        subcommand: "tasks",
        slug: "moontasks",
        display_name: "Moontasks",
        opens: "the task board",
        asks_for_repo: "Which folder to open the board of:",
        asks_for_remote_repo: "Path of the folder to open the board of, on that machine:",
        picker_button: "Choose a folder…",
        opens_button: "Open board",
        opening: "opening the board…",
        opens_without_a_repo: true,
    },
    FrameProgram {
        frame: Frame::Shell,
        subcommand: "shell",
        slug: "moonshell",
        display_name: "Moonshell",
        opens: "a shell in the folder",
        asks_for_repo: "Which folder to open a shell in:",
        asks_for_remote_repo: "Path of the folder to open a shell in, on that machine:",
        picker_button: "Choose a folder…",
        opens_button: "Open shell",
        opening: "opening the shell…",
        opens_without_a_repo: true,
    },
];

impl Frame {
    /// The word after `moon` that opens a window on this frame.
    pub(crate) fn subcommand(self) -> &'static str {
        self.entry().subcommand
    }

    /// What somebody types to open this window, which is what the window calls itself
    /// wherever it names itself to a person.
    pub(crate) fn command(self) -> String {
        format!("{PROGRAM} {}", self.subcommand())
    }

    /// The single token this frame's files and identifiers are named with - see the field.
    pub(crate) fn slug(self) -> &'static str {
        self.entry().slug
    }

    /// The name a desktop launcher shows: the one the OS puts under the icon.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn display_name(self) -> &'static str {
        self.entry().display_name
    }

    /// What the window opens on, as one line of prose.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn opens(self) -> &'static str {
        self.entry().opens
    }

    /// How the launch screen asks which repo to open, which depends on whether this machine
    /// can browse for it.
    pub(crate) fn asks_for_repo(self, picks_folders: bool) -> &'static str {
        let entry = self.entry();
        if picks_folders {
            entry.asks_for_repo
        } else {
            entry.asks_for_remote_repo
        }
    }

    /// What the launch screen's folder picker button says.
    pub(crate) fn picker_button(self) -> &'static str {
        self.entry().picker_button
    }

    /// What the launch screen's button says.
    pub(crate) fn opens_button(self) -> &'static str {
        self.entry().opens_button
    }

    /// What the screen between that button and the open window says it is doing.
    pub(crate) fn opening(self) -> &'static str {
        self.entry().opening
    }

    /// Whether this frame is any use in a folder git knows nothing about.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn opens_without_a_repo(self) -> bool {
        self.entry().opens_without_a_repo
    }

    fn entry(self) -> &'static FrameProgram {
        FRAME_PROGRAMS
            .iter()
            .find(|entry| entry.frame == self)
            .expect("every frame is in the table")
    }
}

/// The frame a word names, for the word after `moon` and for what a launcher wrote in the
/// environment - they are the same word.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn frame_named(name: &str) -> Option<Frame> {
    FRAME_PROGRAMS
        .iter()
        .find(|entry| entry.subcommand == name)
        .map(|entry| entry.frame)
}
