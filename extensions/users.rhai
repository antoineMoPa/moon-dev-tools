//! Who is in this machine's moon server - every pass key and browser login, with where it last came from - to kick one out, or everyone at once by making a new secret

// What each key does, by name. `act` is where each name is carried out.
//
// Not called `keys`: a function of a script takes the place of Rhai's method of the same name,
// and `map.keys()` would then answer with this.
fn key_bindings() {
    #{
        "n": "down", "j": "down", "Down": "down",
        "p": "up", "k": "up", "Up": "up",
        "x": "kick",
        "X": "kick_everyone",
        "g": "refresh",
        "Escape": "cancel",
    }
}

// What the server calls a kind of user, and what the table says.
fn kind_names() {
    #{ pass_key: "pass key", browser: "browser" }
}

// How often the list is read again.
fn refresh_every() { 3000 }

fn init(project_root) {
    let state = #{
        // This pane's own way in: a key minted from the secret on this machine, which is how
        // the pane is a user of the server like any other - the one marked `this pane`.
        key: pass_key(),
        users: [],
        selected: 0,
        // What the server said when it could not be asked, which is what the pane shows then.
        error: "",
        // Kicking everyone is asked twice: the first click only puts the question.
        confirming: false,
    };
    state.refresh();
    every(refresh_every());
    state
}

fn tick() {
    this.refresh();
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
    if "act" in event {
        this.act(event.act);
    }
    if "copy" in event {
        copy(event.copy);
        notify(`copied ${event.copy}`);
    }
}

fn act(does) {
    let last = this.users.len() - 1;
    switch does {
        "down" => { this.selected = clamp(this.selected + 1, 0, last); }
        "up" => { this.selected = clamp(this.selected - 1, 0, last); }
        "refresh" => { this.refresh(); }
        "cancel" => { this.confirming = false; }
        "kick" => { this.kick_selected(); }
        "kick_everyone" => { this.confirming = true; }
        "kick_everyone_indeed" => {
            this.confirming = false;
            this.kick_everyone();
        }
    }
}

// A request to the server as this pane's user. A 401 is this pane's key having been taken
// back - everyone was kicked, and it with them - so a new one is minted and the request made
// once more. Anything but an answer is an error for the caller to catch.
fn ask(method, path) {
    let answered = fetch(server_url() + path, this.as_this_pane(method));
    if answered.status == 401 {
        this.key = pass_key();
        answered = fetch(server_url() + path, this.as_this_pane(method));
    }
    answered
}

fn as_this_pane(method) {
    #{ method: method, headers: #{ Authorization: `Bearer ${this.key}` } }
}

fn refresh() {
    try {
        this.take_listing(this.ask("GET", "/api/users"));
    } catch (error) {
        this.error = `${server_url()} did not answer: ${error}`;
    }
}

fn take_listing(answered) {
    if !answered.ok {
        this.error = `${answered.status}: ${answered.body}`;
        return;
    }
    this.error = "";
    let was_on = this.selected_id();
    let users = parse_json(answered.body).users;
    this.users = users;
    // The cursor stays on the user it was on, wherever that now is.
    for (user, index) in users {
        if user.id == was_on {
            this.selected = index;
        }
    }
    this.selected = clamp(this.selected, 0, users.len() - 1);
}

fn kick_selected() {
    if this.users.is_empty() {
        return;
    }
    let user = this.users[this.selected];
    if user.you || user.kicked {
        return;
    }
    try {
        let answered = this.ask("POST", `/api/users/${user.id}/kick`);
        if answered.ok {
            notify(`kicked ${user.id}`);
        } else {
            this.error = `${answered.status}: ${answered.body}`;
        }
    } catch (error) {
        this.error = `${server_url()} did not answer: ${error}`;
    }
    this.refresh();
}

// A new secret: every key and login there was is out, this pane's own key with them - so the
// pane mints itself another before it asks again.
fn kick_everyone() {
    try {
        let answered = this.ask("POST", "/api/users/kick-all");
        if answered.ok {
            notify("a new secret: every key and browser login is out");
        } else {
            this.error = `${answered.status}: ${answered.body}`;
        }
    } catch (error) {
        this.error = `${server_url()} did not answer: ${error}`;
    }
    this.key = pass_key();
    this.refresh();
}

fn view() {
    let top = [heading(`users of ${server_url()}`)];
    if this.error != "" {
        return column([
            row(top),
            text(this.error, #{ ink: "warn" }),
            row([button("retry", #{ act: "refresh" })]),
        ]);
    }

    let actions = if this.confirming {
        [
            text(
                "make a new secret? every pass key and browser login on this server stops working, this pane's included",
                #{ ink: "warn" }
            ),
            button("yes, kick everyone", #{ act: "kick_everyone_indeed" }),
            button("cancel", #{ act: "cancel" }),
        ]
    } else {
        let selected = if this.users.is_empty() { #{ you: true, kicked: false } } else { this.users[this.selected] };
        [
            button("kick", #{ act: "kick" }, selected.you || selected.kicked),
            button("kick everyone", #{ act: "kick_everyone" }),
            button("refresh", #{ act: "refresh" }),
        ]
    };

    let kinds = kind_names();
    let now = now();
    let rows = [];
    for (user, index) in this.users {
        let standing = if user.kicked {
            text("kicked", #{ ink: "removed" })
        } else if user.you {
            text("this pane", #{ ink: "accent" })
        } else {
            text("in", #{ ink: "added" })
        };
        rows.push(#{
            cells: [
                standing,
                text(kinds[user.kind]),
                text(user.ip, #{ mono: true }),
                text(user.id, #{ mono: true, small: true }),
                muted(ago(now - user.last_seen)),
                muted(ago(now - user.first_seen)),
                muted(`${user.requests}`),
            ],
            selected: index == this.selected,
            on_click: #{ select: index },
            menu: menu_of(user, index),
        });
    }

    column([
        row(top),
        row(actions),
        muted("x kick · X kick everyone · g refresh · a kick outlives the server; a new secret takes every key back"),
        separator(),
        table(["", "kind", "ip", "id", "last seen", "first seen", "requests"], rows),
    ])
}

// What a right click on a user offers. Kicking selects the user first, so it acts on the one
// clicked.
fn menu_of(user, index) {
    let menu = [];
    if !user.you && !user.kicked {
        menu.push(#{ label: "kick", event: #{ select: index, act: "kick" } });
    }
    menu += [
        #{ label: "copy ip", event: #{ copy: user.ip } },
        #{ label: "copy id", event: #{ copy: user.id } },
    ];
    menu
}

// How long ago `seconds` is, in the largest unit that reads.
fn ago(seconds) {
    if seconds < 5 {
        "just now"
    } else if seconds < 60 {
        `${seconds}s ago`
    } else if seconds < 3600 {
        `${seconds / 60}m ago`
    } else if seconds < 86400 {
        `${seconds / 3600}h ago`
    } else {
        `${seconds / 86400}d ago`
    }
}

fn selected_id() {
    if this.users.is_empty() {
        return "";
    }
    this.users[this.selected].id
}

// `value`, kept between `low` and `high` - and at `low` when there is nothing between them.
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
