# 🌚 moonreview

The missing local code review step when working with AI agents.

![Moon Review Screenshot](screenshot.gif)

moonreview is a tiny local code review UI for git.

It shows git hunks, lets you comment, stage or unstage them individually. Comments can either be sent to your local claude, codex, or opencode (using your currently signed-in account) or collected in one big review text for copy pasting in your favourite AI tool.

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
cargo install --path .
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

Everything still links statically — the result is one executable with no runtime
dependency on Zig or on a separate server process.

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
moonreview
```

Run `moonreview` inside any git repository you want to review. That opens the native window,
with the review server running inside it.

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
| `⌘⇧P` | command palette — open a review, a shell, or the agent monitor |
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

## Development

I usually use this as part of my debug loop:

```bash
pkill moon;  cargo install --path . ; moonreview
```

## Origin of name

This is a project started during lunch time, so an AI tool named it noon-review which
was a terrible name, so I updated to moon review which sounds close and is more fun,
later adding the friendly moon emoji. That could also be a reference to reviewing at night
after a long hacking day.
