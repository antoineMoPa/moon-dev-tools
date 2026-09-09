# 🌚 moon-dev-tools

A collection of local tools for the agentic era.

`moon-dev-tools` brings task planning, agent workspaces, shells and code review together. It
installs one executable, `moon`, which opens on three things:

| | |
| --- | --- |
| `moon tasks` | a sprint board for organizing tasks, agents and shells |
| `moon review` | a local code review UI for git |
| `moon shell` | a shell in the repo |
| `moon open <file>` | opens a file in the window already open on its project, else the one last in front |

![Moontasks sprint board with a shell beside it](docs/assets/moontasks-workspace.png)

**Moontasks** is a sprint board that keeps you organized. Each card is a task folder with notes,
running shells and agents attached to it, so agent work has a visible place instead of
disappearing into terminal tabs. Open any resource beside the board and move cards through the
columns as work progresses.

![Moonreview showing local changes](docs/assets/review-dark.png)

**Moonreview** is the review frame. It shows git hunks and lets you comment, stage or unstage
them individually, then commit and push what you staged from a pane beside the review. Send
comments to your local Claude, Codex or OpenCode.

## Quick install

Install the latest prebuilt release:

```bash
curl -fsSL https://raw.githubusercontent.com/antoineMoPa/moon-dev-tools/main/install.sh | sh
```

This installs `moon` and the desktop launchers. If `~/.local/bin` is not already on
your `PATH`, add it in your shell configuration.

## Build from source

Requirements:

- [Rust](https://www.rust-lang.org/tools/install)
- [Zig](https://ziglang.org/) 0.15.x, for the window's terminal
- [the silver searcher](https://github.com/ggreer/the_silver_searcher) (`ag`), which is what
  finds files by name for `⌘P` and text in them for `⌘⇧F`

```bash
./scripts/setup-dev.sh            # Rust update, Zig and submodules
export PATH="$(brew --prefix zig@0.15)/bin:$PATH"
cargo install --locked --path .   # installs moon
moon install-launchers            # optional: launchers the OS itself offers
moon review
```

`--locked` builds the dependency versions in `Cargo.lock`; without it `cargo install` re-resolves
to the newest compatible versions, which is how you end up compiling a release nobody tested.

The window embeds Ghostty's terminal emulator
([libghostty-vt](https://ghostty.org/)), which is built from Ghostty's Zig
source, so a Zig 0.15.x toolchain has to be on `PATH` at build time. On macOS:

```bash
brew install zig@0.15
export PATH="$(brew --prefix zig@0.15)/bin:$PATH"
```

Everything still links statically - the result is one executable with no runtime dependency
on Zig or on a separate server process.

## Desktop launchers

`cargo install` leaves one executable on `PATH`, which is all a shell needs. To open the
windows from the OS as well - Spotlight and Launchpad on macOS, the application menu on Linux:

```bash
moon install-launchers
```

It writes one launcher per window: a `.app` bundle in `/Applications` on macOS -
in `~/Applications` instead, for an account that cannot write the shared folder - and a
`.desktop` entry in `~/.local/share/applications` on Linux. The window has the same thing in
its macOS menu bar and in the command palette, as `install desktop launchers`. Each launcher
runs the executable where it is installed, so `cargo install` over it is also an upgrade of
what the launcher opens; rerun the command only after moving the executable somewhere else.

A window opened that way starts outside every repo. `moon tasks` and `moon shell` open on the
project the last window opened - neither needs a repo - and `moon review`, which has nothing
to show without one, asks which repo to open with the folder picker of the OS.

`install.sh` writes the launchers itself, so a prebuilt install needs nothing further.

## Usage

```bash
moon tasks    # the sprint board
moon review   # review local changes
moon shell    # a shell in the folder
```

Run any of them inside a git repository. `moon tasks` and `moon shell` run just as well in a
folder that is no repository: the review is the part that needs one. The other tools remain
one command-palette action away (`⌘⇧P`).

### Moontasks

Write a card for a piece of work, pick an agent from `[start]`, and Moontasks starts it in the
repo. Cards
group the task brief, shared notes, agent runs, shells and the files the task is about in one
place. Drag cards and columns
to make the board match your workflow.

Task state lives in the repo's `.moontasks/` directory. See [Moontasks.md](Moontasks.md) for the
complete board behavior and controls.

### Moonreview

Pass two paths to compare arbitrary files in a read-only review:

```bash
moon review a.txt b.txt
```

Every review target opens the same window:

```bash
moon review .              # only the current directory
moon review src/main.rs    # only that file or directory
moon review 4542abe        # one commit, read only
moon review diff dev       # against a git target, read only
```

Inside the window:

| | |
| --- | --- |
| click a diff line | select it and open a comment on it |
| shift-click | extend the selection over more lines |
| `⌘⏎` | save the comment being written |
| `s` / `u` | stage / unstage the hunk under the caret |
| `⌘⇧P` | command palette - open a review, a shell, the task board, the submodules of the repo, or the agent monitor |
| `⌘P` | find a file of the repo by name, from any directory under it, and open it |
| `⌘⇧F` | search the files of the repo for text, and open a file at the line that holds it |
| `⌘⇧R` | bring the review of this repo forward, opening it if it was closed |
| `⌘N` | another window of this same program, on its launch screen |
| `⌘J` | switch light and dark |
| `?` | the shortcut list |

Clicking a diff line selects it and opens a comment on it; shift-click extends the run. The
comment is anchored to exactly those lines, and `stage lines` stages exactly those lines.

On macOS the **View** menu carries the theme switch and the command palette, and the **Window**
menu opens another window on any of the three - `New Moontasks Window` from the
review, `New Moonreview Window` from the board. A new window opens on its launch screen, so it
is a new place to work rather than a second view of this one; `moon tasks --pick` is the same
thing from a shell. Everywhere else those live in the command palette, which also has them on
macOS.

### Working on another machine

Run the server where the repo is:

```bash
# on the remote machine
MOONREVIEW_HOST=0.0.0.0 moon serve
```

Then point a local window at it:

```bash
moon review --remote dev-box --repo /home/you/project
```

`--remote` takes `host`, `host:port`, or a full URL, and defaults to port 42000. Leave
`--repo` off and the window asks which path to review. Shells opened in the window run on
the remote machine, as does everything the review does to the repo.

The task board works the same way round: `.moontasks` is the remote repo's folder and the
agents run there, so closing the window leaves them working and reopening it finds them.

The server binds `127.0.0.1` unless `MOONREVIEW_HOST` says otherwise, and it has no
authentication, so prefer an SSH tunnel over exposing the port:

```bash
ssh -N -L 42000:127.0.0.1:42000 dev-box   # then: moon review --remote 127.0.0.1
```

## Stopping the server

Closing the window ends the process, server included. A standalone `serve` stops with
`pkill moon`, and times out on its own after 30 minutes of inactivity.

## Crates

Three pieces of the window are libraries in their own right, kept as submodules under
`crates/` and published separately:

- [**egui_frames**](crates/egui_frames) - tabs, splits and draggable panes for egui. The
  arrangement the Moon tools workspace is made of, with nothing product-specific in it.
- [**egui_tty**](crates/egui_tty) - a terminal emulator widget for egui, on Ghostty's VT engine.
  What a shell tab holds.
- [**egui_moon_editor**](crates/egui_moon_editor) - a code editor widget for egui: a text
  buffer, a fringe of line numbers that scrolls with the code, and marks drawn into the text.
  What a file tab holds.

After cloning, pull them in:

```bash
git submodule update --init --recursive
```

## Development

I usually use this as part of my debug loop:

```bash
pkill moon;  cargo install --locked --path .
```

To install launchers:

```bash
cargo install --locked --path .; moon install-launchers
```

On mac you will need to drag applications from the Applications folder to your menu bar.

## Origin of the names

Moonreview started as a lunch-time project named `noon-review` by an AI tool. That was a
terrible name, so it became Moonreview: close enough to the original, more fun, and fitting for
reviewing after a long hacking day. Moontasks and Moonshell joined it later, and
`moon-dev-tools` became the home for the whole collection.
