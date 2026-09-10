// The vocabulary a view is written in, merged in ahead of every extension.
//
// Each of these only builds a map, and the map is what the window reads: a view can be written
// out as map literals just as well, and a script that defines one of these itself replaces it.

fn text(words) { #{ kind: "text", text: words } }

// `style` takes any of: ink ("muted", "accent", "warn", "added", "removed"), strong, mono, small.
fn text(words, style) {
    let element = #{ kind: "text", text: words };
    element += style;
    element
}

fn muted(words) { text(words, #{ ink: "muted" }) }

fn heading(words) { #{ kind: "heading", text: words } }

fn row(children) { #{ kind: "row", children: children } }

fn column(children) { #{ kind: "column", children: children } }

fn button(label, event) { #{ kind: "button", label: label, on_click: event } }

fn button(label, event, disabled) { #{ kind: "button", label: label, on_click: event, disabled: disabled } }

// `rows` are maps of: cells (one per column), and optionally selected, on_click, on_double_click.
fn table(columns, rows) { #{ kind: "table", columns: columns, rows: rows } }

fn code(words) { #{ kind: "code", text: words } }

// A line to type in. `event` - a map - comes back to `update` on every change, with what was
// typed in its `value`. `id` tells it from any other box in the view.
fn input(id, value, hint, event) { #{ kind: "input", id: id, value: value, hint: hint, on_change: event } }

fn separator() { #{ kind: "separator" } }
