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
| `⌘J` | switch light and dark |
| `?` | the shortcut list |

Clicking a diff line selects it and opens a comment on it; shift-click extends the run. The
comment is anchored to exactly those lines, and `stage lines` stages exactly those lines.

On macOS the **View** menu carries "Open in Browser" and the theme switch. Everywhere else
those live in the command palette, which also has them on macOS.

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

`moontasks` opens a sprint board over the repo. `moonreview` and `moonshell` reach the same
board from the command palette, in a tab of their own.
Each card is a task with an agent behind it: name a task, pick an agent, and it starts working
in the repo straight away.

| | |
| --- | --- |
| drag a card by its title | move it between TODO, IN PROGRESS, IN LOCAL REVIEW, IN REMOTE REVIEW and DONE |
| `[start review]` | open the review of the repo in a tab |
| `[launch shell]` / `[new agent]` | start another shell or another agent inside the task |
| a running resource | click its name to bring its terminal back on screen |

An agent that finishes moves its own card to IN LOCAL REVIEW — either by calling the MCP tool
below, which is what it is told to do, or simply by exiting, which the board notices the next
time it reads the folder.

Closing an agent's tab does not end it. A task's shells belong to the task and keep running
with nothing attached until the card reaches DONE, so you can close a noisy agent and come
back to it. `stop` ends one on purpose, and `resume` starts it again where it left off.

The board is a folder in the repo, which is the whole of its state:

```
.moontasks/
  .gitignore          # ignores the whole board, written when the board is created
  fix-the-login-page-6f9c1e2a-…/
    metadata.json     # title, column, and the shells and agent runs of the task
    brief.md          # what the agents working here have been told
    mcp.json          # the MCP server they are given
    opencode.json     # the same, in the shape OpenCode reads
    …                 # anything you or an agent puts here
```

Nothing about it is moonreview's alone: create a task with `mkdir` and a `metadata.json`,
move a card by editing the file, and the board picks it up on its next read.

A board is working state — running agents, scratch files, an MCP config carrying this run's
port — so it ignores itself from the moment it is created, and opening moonreview in a repo
leaves `git status` exactly as it was. To share the board with the rest of the team instead,
delete `.moontasks/.gitignore` and commit the folder; it will not come back.

#### What the agents are told

An agent started on a task does not have to be asked twice. It opens on the task's title as
its prompt, and it is given a brief — the same text `brief.md` holds — naming the task, its
folder, and the tool to call when the work is ready to be looked at.

The brief is worth its own file because it is the difference between an agent that has the
tools and one that uses them.

#### The MCP server

`moonreview mcp` is an MCP server over the board, with two tools:

- `moontasks_set_status` — move this task to another column, which is how an agent reports
  that it is done
- `moontasks_get_task` — read the task it is working in

moonreview starts it for the agent, so there is no reason to run it by hand. All three agents
are wired up, each the way it takes one: Claude through `--mcp-config`, Codex through `-c`
config overrides, and OpenCode through the config file its `OPENCODE_CONFIG` names.

#### What survives a restart

The shells are the server's, so they outlive the window: closing it and opening it again
finds the same agents still working. They do not outlive the server — a `moonreview serve`
that is restarted, or a native window that is quit, takes its shells with it. What survives is
the record of every run, so the board comes back with each one marked as ended, its task moved
on to IN LOCAL REVIEW, and `resume` there to start it again. For Claude that resumes the exact
session, because moonreview gives it the session id when it starts it; Codex and OpenCode are
resumed by their own reckoning of the most recent session in the repo.

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
pkill moon;  cargo install --path . ; moonreview
```

`cargo install --path .` builds the library once and links the three executables from it.

## Origin of name

This is a project started during lunch time, so an AI tool named it noon-review which
was a terrible name, so I updated to moon review which sounds close and is more fun,
later adding the friendly moon emoji. That could also be a reference to reviewing at night
after a long hacking day.
