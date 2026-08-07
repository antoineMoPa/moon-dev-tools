# 🌚 moonreview

The missing local code review step when working with AI agents.

![Moon Review Screenshot](screenshot.gif)

moonreview is a tiny local code review UI for git.

It shows git hunks, lets you comment, stage or unstage them individually. Comments can either be sent to your local claude, codex, or opencode (using your currently signed-in account) or collected in one big review text for copy pasting in your favourite AI tool.

It installs three executables. They are one window opened on three different things:

| | |
| --- | --- |
| `moonreview` | a review of the repo |
| `moontasks` | the task board, and the agents working through it |
| `moonshell` | a shell in the repo |

Whichever you start, the other two are a command palette away — they are frames of the same
window, not separate apps.

There are two frontends over one review server:

- a **native window**, which carries the server inside the same executable — this is the default
- the **web frontend**, in a browser tab, which is the same review and stays fully supported

## Installation

Build from source:

Requirements:

- [Rust](https://www.rust-lang.org/tools/install)
- Node.js with npm
- [Zig](https://ziglang.org/) 0.15.x, for the native window's terminal

```bash
git submodule update --init --recursive   # egui_frames and egui_tty, see Crates below
cargo install --path .                   # installs moonreview, moontasks and moonshell
moonreview install-launchers             # optional: launchers the OS itself offers
moonreview
```

Source builds require Rust plus the existing Node/npm frontend toolchain used by `build.rs`.

The native window embeds Ghostty's terminal emulator
([libghostty-vt](https://libghostty.tip.ghostty.org/)), which is built from Ghostty's Zig
source, so a Zig 0.15.x toolchain has to be on `PATH` at build time. On macOS:

```bash
brew install zig@0.15
export PATH="$(brew --prefix zig@0.15)/bin:$PATH"
```

Everything still links statically — the result is three executables with no runtime
dependency on Zig or on a separate server process. They share one library, so the build
compiles once and links three times.

To build without the native window (web frontend only, no Zig needed):

```bash
cargo install --path . --no-default-features
```

## Easy installation

Install the latest prebuilt binary:

```bash
curl -fsSL https://raw.githubusercontent.com/antoineMoPa/moonreview/main/install.sh | sh
```

If `~/.local/bin` is not already on your `PATH`, you may need to update your PATH in your
shells.

## Desktop launchers

`cargo install` leaves three executables on `PATH`, which is all a shell needs. To open them
from the OS as well — Spotlight and Launchpad on macOS, the application menu on Linux:

```bash
moonreview install-launchers
```

It writes one launcher per installed executable: a `.app` bundle in `/Applications` on macOS —
in `~/Applications` instead, for an account that cannot write the shared folder — and a
`.desktop` entry in `~/.local/share/applications` on Linux. The window has the same thing in
its macOS menu bar and in the command palette, as `install desktop launchers`. Each launcher
runs the executable where it is installed, so `cargo install` over it is also an upgrade of
what the launcher opens; rerun the command only after moving the executables somewhere else.

A window opened that way starts outside every repo — there is no terminal it could have
inherited one from — so it asks which repo to review, with the folder picker of the OS.

`install.sh` writes the launchers itself, so a prebuilt install needs nothing further.

## Usage

```bash
moonreview   # a review of the repo
moontasks    # the task board
moonshell    # a shell in the repo
```

Run any of them inside a git repository. Each opens the same native window, with the review
server running inside it, on a different first tab.

Pass two paths to compare arbitrary files in a read-only review:

```bash
moonreview a.txt b.txt
```

Every review target works the same in either frontend:

```bash
moonreview .              # only the current directory
moonreview src/main.rs    # only that file or directory
moonreview 4542abe        # one commit, read only
moonreview diff dev       # against a git target, read only
```

Inside the window:

| | |
| --- | --- |
| click a diff line | select it and open a comment on it |
| shift-click | extend the selection over more lines |
| `⌘⏎` | save the comment being written |
| `s` / `u` | stage / unstage the hunk under the caret |
| `⌘⇧P` | command palette — open a review, a shell, the task board, or the agent monitor |
| `⌘N` | another window of this same program, on the same repo |
| `⌘J` | switch light and dark |
| `?` | the shortcut list |

Clicking a diff line selects it and opens a comment on it; shift-click extends the run. The
comment is anchored to exactly those lines, and `stage lines` stages exactly those lines.

On macOS the **View** menu carries "Open in Browser" and the theme switch, and the **Window**
menu opens another window of any of the three programs — `New Moontasks Window` from the
review, `New Moonreview Window` from the board — each on the repo this window is on. Everywhere
else those live in the command palette, which also has them on macOS.

### The web frontend

`--web` opens a browser tab against a background server instead, which is how moonreview
worked before the window existed:

```bash
moonreview --web
```

The window also serves the web frontend, so **View → Open in Browser** (`⌘B`), or
`open in browser` in the command palette, opens the same review in a browser without starting
anything else.

### Moontasks

`moontasks` opens a sprint board over the repo, each card a task with an agent behind it.
See [Moontasks.md](Moontasks.md).

## Settings

What belongs to you rather than to a repo or a window is kept in one file:

```
~/.moonreview/settings.json
```

Right now that is which agent the review hands comments to — the selector at the top right.
Pick one and the next window comes up on it. The file is meant to be readable and editable:

```json
{
  "selected_agent": "claude"
}
```

### Reviewing another machine

Run the server where the repo is:

```bash
# on the remote machine
MOONREVIEW_HOST=0.0.0.0 moonreview serve
```

Then point a local window at it:

```bash
moonreview --remote dev-box --repo /home/you/project
```

`--remote` takes `host`, `host:port`, or a full URL, and defaults to port 42000. Leave
`--repo` off and the window asks which path to review. Shells opened in the window run on
the remote machine, as does everything the review does to the repo.

The task board works the same way round: `.moontasks` is the remote repo's folder and the
agents run there, so closing the window leaves them working and reopening it finds them.

The server binds `127.0.0.1` unless `MOONREVIEW_HOST` says otherwise, and it has no
authentication, so prefer an SSH tunnel over exposing the port:

```bash
ssh -N -L 42000:127.0.0.1:42000 dev-box   # then: moonreview --remote 127.0.0.1
```

## Stopping the server

Closing the native window ends the process. For the `--web` flow:

```bash
pkill moonreview
```

A standalone `serve` also times out after 30 minutes of inactivity.

## Crates

Two pieces of the native window are libraries in their own right, kept as submodules under
`crates/` and published separately:

- [**egui_frames**](crates/egui_frames) — tabs, splits and draggable panes for egui. The
  arrangement moonreview's window is made of, with nothing about reviews in it.
- [**egui_tty**](crates/egui_tty) — a terminal emulator widget for egui, on Ghostty's VT engine.
  What a shell tab holds.

After cloning, pull them in:

```bash
git submodule update --init --recursive
```

## Development

I usually use this as part of my debug loop:

```bash
pkill moon;  cargo install --path .
```

To install launchers:

```bash
cargo install --path .; moonreview install-launchers
```

On mac you will need to drag applications from the Applications folder to your menu bar.

## Origin of name

This is a project started during lunch time, so an AI tool named it noon-review which
was a terrible name, so I updated to moon review which sounds close and is more fun,
later adding the friendly moon emoji. That could also be a reference to reviewing at night
after a long hacking day.
