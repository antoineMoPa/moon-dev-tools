# Extensions

A pane of the window can be a script. Two are built into `moon`, out of
[extensions/](extensions/): `files` browses the project's folders, and `docker` lists docker's
containers, to start, stop and restart them, open a shell in one, or follow its logs. Any `.rhai` file in `~/.moonreview/extensions/` is another, offered in
the command palette (`⌘⇧P`) under its file name.

A file there with the same name as a shipped one takes its place, which is how a shipped one is
changed without building `moon` again:

```bash
mkdir -p ~/.moonreview/extensions
cp extensions/files.rhai ~/.moonreview/extensions/
```

The copy is yours from then on: it no longer follows the shipped one as `moon` is updated, and
the palette does not say which of the two you have. Delete it to go back.

Scripts are [Rhai](https://rhai.rs). The `//!` lines a script starts with are what the palette
says about it.

## Writing one

A script in `~/.moonreview/extensions/` is read again every time it is saved, with its pane open
beside the editor. The pane keeps its state across the change, and `init` is run again on the
side: a field it answers with that the state has not got is added, and an `every` it sets takes
effect. The fields the state already has keep their values - to start from scratch, run
`restart <name>` from the palette.

A shipped script is only read when its pane opens; copy it to change it.

When something goes wrong, the pane says so at the top, over the last view it drew, and the
error is also written to the Messages pane (a click on the status bar). It stays up until you
next click or press a key in the pane, or save the script. A script saved with a mistake in it
does not replace the one running: the error says the version that last loaded is still the one
running.

## The functions a script has

```rhai
//! Count clicks

fn init(project_root) {        // the state, whatever the script wants it to be
    #{ count: 0 }
}

fn view() {                    // what the pane shows, read from `this` - the state
    column([
        heading(`clicked ${this.count} times`),
        button("click", #{ add: 1 }),
    ])
}

fn update(event) {             // a button or a row was clicked: `event` is what it carried
    this.count += event.add;
}

fn on_key(key) {               // optional: a key pressed while the pane has the keyboard
    if key == "+" { this.count += 1; }
}

fn tick() {                    // optional: as often as `every(ms)` asked
}
```

`init` may leave `project_root` out. `this` is the state in every function but `init`, which
answers with it instead.

`tick` is called only while the pane is in front in its frame, and once as soon as it comes to
the front again.

`on_key` hears a key as the character it typed - `n`, `^`, `S` - or, for a key that types
nothing, by its name: `Enter`, `Escape`, `Backspace`, `Delete`, `Tab`, `Up`, `Down`, `Left`,
`Right`, `Home`, `End`, `PageUp`, `PageDown`, with `Shift+` in front when shift is held
(`Shift+Tab`). Anything typed with ⌘, ctrl or alt held stays the window's. A script with no
`on_key` gets no keys at all: the window keeps them. Nor does it get any while one of its
inputs has the keyboard - what is typed goes into the box, and Escape leaves it.

The code at the top of a script, outside every function, runs before each call of one of its
functions, in a scope of its own - so keep it to constants and `import`s. Its constants are
there for every function as `global::NAME`; a function cannot see them by name alone:

```rhai
const PAGE = 20;
fn down() { this.selected += global::PAGE; }
```

An `import` names a file from the script's own folder.

## Rhai, and what catches people out

- **A function of yours takes the place of Rhai's method of the same name.** Write `fn keys()`
  and every `map.keys()` in the script calls yours. `len`, `contains`, `keys`, `values`,
  `split`, `trim` and the rest of the built-in methods are names to leave alone.
- **A helper that reads `this` is called as a method**: `this.load()`, or `state.load()` in
  `init`. Called plainly - `load()` - it has no `this`.
- **Some string methods change the string rather than answer with a new one.** `s.trim()` trims
  `s`, and `let t = s.trim()` is `()`.
- **A field the state has not got reads as `()`**, and the error that follows is about `()`:
  `Function not found: + ((), i64)` is a field that was never set.

## What a view is made of

A view is a tree of maps, each naming its `kind`. These build them:

| | |
| --- | --- |
| `text(words)`, `text(words, style)` | `style` is a map of any of `ink`, `strong`, `mono`, `small` |
| `muted(words)` | `text` in the muted ink |
| `heading(words)` | |
| `row([...])` | side by side, wrapping when the pane is narrow |
| `column([...])` | one under the other |
| `button(label, event)`, `button(label, event, disabled)` | `update(event)` on a click |
| `table(columns, rows)` | each row a map of `cells` (one per column), and optionally `selected`, `on_click`, `on_double_click`, and `menu` - a right click's entries, each `#{ label, event }` |
| `code(words)` | monospace, scrolled to its end |
| `input(id, value, hint, event)` | a line to type in; `update` gets `event` - a map - with what is typed in its `value`, on every change. `id` tells it from any other box in the view |
| `separator()` | |

`ink` is one of `normal`, `muted`, `accent`, `warn`, `added`, `removed` - the window's own
colors, so an extension reads right in both themes.

A table takes the height left in the pane, so it goes last, and only the first table of a view
follows its selected row. It draws only the rows on screen. A button in a cell would fight the
row for the click, so a row's actions go in its `menu`, or in a row of buttons over the table
acting on the selected row.

A kind the window does not draw, or a field it would not read, is refused rather than left off
the screen: the pane says what was wrong.

## What a script can do

| | |
| --- | --- |
| `run(program, [args])`, `run(program, [args], #{ timeout_ms, cwd, env })` | runs it and waits: `#{ ok, code, stdout, stderr }`. In the project unless `cwd` says otherwise, and stopped after 30 s unless `timeout_ms` does |
| `run_then(program, [args], event)`, `run_then(program, [args], options, event)` | the same without waiting; `update` gets `event` with the `result` added |
| `fetch(url)`, `fetch(url, #{ method, headers, body })` | an HTTP request, and waits: `#{ ok, status, headers, body }`, the body as text and the header names in lowercase. A status that is not a success is still a response; no response at all is an error to `catch`. Given up on after 30 s |
| `fetch_then(url, event)`, `fetch_then(url, options, event)` | the same without waiting; `update` gets `event` with the `result` added - or `error`, when there was no response |
| `read_dir(path)` | the entries of a folder in name order: `#{ name, path, is_dir, is_link, link_to, size, modified }` |
| `open_file(path)`, `open_file(path, line)` | opens a file of the project in a tab, at a line counted from one |
| `open_shell(command)` | opens a shell in the project with `command` typed into it and sent |
| `copy(words)` | puts them on the clipboard |
| `notify(words)` | a toast |
| `print(words)`, `debug(value)` | a line in the Messages pane |
| `every(ms)` | how often `tick()` is called; `every(0)` stops it |
| `now()` | seconds since the epoch |
| `parent_of(path)`, `name_of(path)`, `join_path(dir, name)` | |
| `shell_quote(words)` | quoted for a shell, for `open_shell` |
| `parse_json(text)` | an object, a list, or a plain value |

The event given to `run_then` or `fetch_then` must not already have the field the answer goes
in - `result`, and `error` for `fetch_then`.

Programs are looked for on your login shell's `PATH`, the same one the window's shells get.

A script runs on a thread of its own, so a slow `run` or `fetch` holds up that pane and nothing
else. A loop that never ends is stopped and said so, and so is a program that outlives its
timeout.

Extensions run on the machine the window is on, so a window on a `--remote` project has none.

## What a script can do to your machine

Everything above, with your permissions: run any program, reach any address, read any folder -
and write anywhere, through a program it runs. Nothing asks first, and a pane put back when a
window opens starts its script straight away. Put in `~/.moonreview/extensions/` only what you
would run in a shell yourself.
