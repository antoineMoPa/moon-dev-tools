//! Browse the project's folders and open their files in tabs

// What each key does, by name. `act` is where each name is carried out.
//
// Not called `keys`: a function of a script takes the place of Rhai's method of the same name,
// and `map.keys()` is used below.
fn key_bindings() {
    #{
        "n": "down", "j": "down", "Down": "down",
        "p": "up", "k": "up", "Up": "up",
        "PageDown": "page_down", "PageUp": "page_up",
        "Home": "first", "<": "first",
        "End": "last", ">": "last",
        "Enter": "open", "f": "open", "o": "open", "Right": "open",
        "^": "parent", "-": "parent", "Backspace": "parent", "Left": "parent",
        "g": "refresh",
        ".": "hidden",
        "!": "shell",
    }
}

// How far PageUp and PageDown move.
fn page() { 20 }

// Ages, and what each is counted in once a file is that old: the last one reached is said.
fn ages() {
    [
        #{ seconds: 60, says: "min" },
        #{ seconds: 3600, says: "h" },
        #{ seconds: 86400, says: "d" },
        #{ seconds: 2592000, says: "mo" },
        #{ seconds: 31536000, says: "y" },
    ]
}

// What git calls a change, and the ink it is marked in. Anything not here is marked `warn`.
fn change_inks() {
    #{ "??": "added", "A": "added", "D": "removed" }
}

fn init(project_root) {
    let state = #{
        root: project_root,
        dir: project_root,
        entries: [],
        selected: 0,
        show_hidden: false,
        changes: #{},
        changed_folders: #{},
    };
    state.load("");
    // Read again every couple of seconds, so a file an agent writes shows up without a `g`.
    every(2000);
    state
}

fn tick() {
    this.load(this.selected_name());
}

fn on_key(key) {
    let bound = key_bindings();
    if key in bound {
        this.act(bound[key]);
    }
}

fn update(event) {
    if "select" in event {
        this.selected = event.select;
    }
    if "open" in event {
        this.selected = event.open;
        this.act("open");
    }
    if "act" in event {
        this.act(event.act);
    }
    if "copy" in event {
        copy(event.copy);
        notify(`copied ${event.copy}`);
    }
    if "shell_in" in event {
        open_shell(`cd ${shell_quote(event.shell_in)}`);
    }
}

// What a right click on an entry offers.
fn menu_of(entry, index) {
    let open = #{ label: "open", event: #{ open: index } };
    if entry.name == ".." {
        return [open];
    }
    let folder = if entry.is_dir { entry.path } else { this.dir };
    [
        open,
        #{ label: "copy full path", event: #{ copy: entry.path } },
        #{ label: "copy path from the project", event: #{ copy: from_project(entry.path, this.root) } },
        #{ label: "shell here", event: #{ shell_in: folder } },
    ]
}

// A path as it reads from the project's own folder; whole, for one outside it.
fn from_project(path, root) {
    if path.starts_with(root + "/") {
        return path.sub_string(root.len() + 1);
    }
    path
}

fn act(does) {
    let last = this.entries.len() - 1;
    switch does {
        "down" => { this.selected = clamp(this.selected + 1, 0, last); }
        "up" => { this.selected = clamp(this.selected - 1, 0, last); }
        "page_down" => { this.selected = clamp(this.selected + page(), 0, last); }
        "page_up" => { this.selected = clamp(this.selected - page(), 0, last); }
        "first" => { this.selected = 0; }
        "last" => { this.selected = clamp(last, 0, last); }
        "open" => { this.open_selected(); }
        // Going up leaves the cursor on the folder just left.
        "parent" => { this.enter(parent_of(this.dir), name_of(this.dir)); }
        "refresh" => { this.load(this.selected_name()); }
        "hidden" => {
            this.show_hidden = !this.show_hidden;
            this.load(this.selected_name());
        }
        "shell" => { open_shell(`cd ${shell_quote(this.dir)}`); }
    }
}

fn open_selected() {
    if this.entries.is_empty() {
        return;
    }
    let entry = this.entries[this.selected];
    if !entry.is_dir {
        open_file(entry.path);
        return;
    }
    let keep = if entry.name == ".." { name_of(this.dir) } else { "" };
    this.enter(entry.path, keep);
}

// Move to another folder. One that cannot be read is said so, and not moved to.
fn enter(dir, keep) {
    let from = this.dir;
    this.dir = dir;
    try {
        this.load(keep);
    } catch (error) {
        this.dir = from;
        notify(`could not open ${dir}: ${error}`);
    }
}

// Read the folder again: folders first, then files, each in name order. `keep` is the entry to
// leave the cursor on; failing that, it stays where it was.
fn load(keep) {
    let folders = [];
    let files = [];
    for entry in read_dir(this.dir) {
        if !this.show_hidden && entry.name.starts_with(".") {
            continue;
        }
        if entry.is_dir {
            folders.push(entry);
        } else {
            files.push(entry);
        }
    }

    let entries = [];
    let parent = parent_of(this.dir);
    if parent != this.dir {
        entries.push(#{
            name: "..", path: parent, is_dir: true, is_link: false, link_to: "", size: 0, modified: 0,
        });
    }
    entries += folders;
    entries += files;
    this.entries = entries;
    this.changes = changes_in(this.root);
    this.changed_folders = folders_holding(this.changes, this.root);

    let found = -1;
    for (entry, index) in entries {
        if entry.name == keep {
            found = index;
        }
    }
    this.selected = if found >= 0 { found } else { clamp(this.selected, 0, entries.len() - 1) };
}

fn selected_name() {
    if this.entries.is_empty() {
        return "";
    }
    this.entries[this.selected].name
}

// What git says has changed, by path: `M`, `A`, `D`, `??` and the rest. Empty outside a repo.
fn changes_in(root) {
    let changes = #{};
    let ran = run("git", ["status", "--porcelain", "--untracked-files=all"]);
    if !ran.ok {
        return changes;
    }
    for line in ran.stdout.split("\n") {
        if line.len() < 4 {
            continue;
        }
        let code = line.sub_string(0, 2);
        code.trim();
        let path = line.sub_string(3);
        // A rename is written `old -> new`, and it is the new one that is on disk.
        let arrow = path.index_of(" -> ");
        if arrow >= 0 {
            path = path.sub_string(arrow + 4);
        }
        changes[join_path(root, path)] = code;
    }
    changes
}

// Every folder with a change somewhere inside it, worked out once a load, so a folder's mark is
// one look rather than a walk through every change for every row.
fn folders_holding(changes, root) {
    let folders = #{};
    for path in changes.keys() {
        let folder = parent_of(path);
        // A folder already in means every folder above it is too.
        while folder.len() > root.len() && !(folder in folders) {
            folders[folder] = true;
            folder = parent_of(folder);
        }
    }
    folders
}

// The change an entry carries: its own, or `*` for a folder with changes somewhere inside.
fn mark_of(entry) {
    if entry.name == ".." {
        return "";
    }
    if entry.path in this.changes {
        return this.changes[entry.path];
    }
    if entry.is_dir && entry.path in this.changed_folders {
        return "*";
    }
    ""
}

fn mark_ink(mark) {
    let inks = change_inks();
    for code in inks.keys() {
        if mark.contains(code) {
            return inks[code];
        }
    }
    "warn"
}

fn view() {
    let rows = [];
    for (entry, index) in this.entries {
        let mark = this.mark_of(entry);
        let name = if entry.is_dir {
            text(entry.name + "/", #{ ink: "accent" })
        } else if entry.is_link {
            text(`${entry.name} -> ${entry.link_to}`)
        } else {
            text(entry.name)
        };
        let size = if entry.is_dir { "" } else { size_of(entry.size) };
        rows.push(#{
            // What is short and the same width every row first, and the name last, where it has
            // the rest of the pane.
            cells: [
                text(mark, #{ ink: mark_ink(mark), mono: true }),
                text(size, #{ ink: "muted", mono: true }),
                muted(ago(entry.modified)),
                name,
            ],
            selected: index == this.selected,
            on_click: #{ select: index },
            on_double_click: #{ open: index },
            menu: this.menu_of(entry, index),
        });
    }

    column([
        heading(shown(this.dir, this.root)),
        muted("enter open · backspace up · n/p move · g refresh · . dotfiles · ! shell"),
        separator(),
        table(["", "size", "modified", "name"], rows),
    ])
}

// The folder as the heading says it: from the project's own name down, while it is inside the
// project, and whole once it is not.
fn shown(dir, root) {
    if dir == root {
        return name_of(root) + "/";
    }
    if dir.starts_with(root + "/") {
        return name_of(root) + dir.sub_string(root.len()) + "/";
    }
    dir
}

fn size_of(bytes) {
    if bytes < 1024 {
        return `${bytes} B`;
    }
    let units = ["K", "M", "G", "T"];
    let value = bytes.to_float() / 1024.0;
    let unit = 0;
    while value >= 1024.0 && unit < units.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    let shown = if value < 10.0 {
        ((value * 10.0).round() / 10.0).to_string()
    } else {
        value.round().to_int().to_string()
    };
    `${shown} ${units[unit]}`
}

fn ago(unix) {
    if unix == 0 {
        return "";
    }
    let seconds = now() - unix;
    let said = "just now";
    for age in ages() {
        if seconds >= age.seconds {
            said = `${seconds / age.seconds} ${age.says} ago`;
        }
    }
    said
}

// `value`, kept between `low` and `high` - and at `low` when there is nothing between them,
// which is an empty folder's cursor.
fn clamp(value, low, high) {
    let kept = value;
    if kept > high {
        kept = high;
    }
    if kept < low {
        kept = low;
    }
    kept
}
