//! The containers docker runs: start, stop and restart them, open a shell in one, follow its logs

// What `docker ps` says about each container, as a line of JSON of its own.
fn listing_format() {
    `{"id":{{json .ID}},"name":{{json .Names}},"image":{{json .Image}},"state":{{json .State}},"status":{{json .Status}},"ports":{{json .Ports}},"project":{{json (.Label "com.docker.compose.project")}}}`
}

// What each key does, by name. `act` is where each name is carried out.
//
// Not called `keys`: a function of a script takes the place of Rhai's method of the same name,
// and `map.keys()` would then answer with this.
fn key_bindings() {
    #{
        "n": "down", "j": "down", "Down": "down",
        "p": "up", "k": "up", "Up": "up",
        "s": "start", "S": "stop", "r": "restart",
        "l": "logs", "Enter": "logs",
        "e": "shell",
        "g": "refresh",
    }
}

// Whether a container passes what is typed in the filter box: every word of it, in any case,
// somewhere in its name, image, project, status or state.
fn passes(container, query) {
    let about = `${container.name} ${container.image} ${container.project} ${container.status} ${container.state}`.to_lower();
    for word in query.to_lower().split(" ") {
        if word != "" && !about.contains(word) {
            return false;
        }
    }
    true
}

// The ink a container's state is written in. A state not here is written plain.
fn state_inks() {
    #{
        running: "added",
        restarting: "warn",
        paused: "warn",
        removing: "warn",
        dead: "removed",
        exited: "muted",
        created: "muted",
    }
}

// What each of the project buttons asks of `docker compose`.
fn compose_verbs() {
    #{ project_start: "start", project_stop: "stop", project_restart: "restart" }
}

// How often the list is read again.
fn refresh_every() { 3000 }

fn init(project_root) {
    let state = #{
        // Every container docker listed, and the ones of them the filter box lets through -
        // which are what the table shows, and what `selected` counts in.
        listed: [],
        containers: [],
        query: "",
        selected: 0,
        // What docker said when it could not be reached, which is what the pane shows then.
        error: "",
        // What is being done right now, for the heading.
        busy: "",
        // Whether a listing is on its way, so a slow daemon is not asked twice at once.
        listing: false,
        has_colima: run("colima", ["version"]).ok,
    };
    state.take_listing(run("docker", ["ps", "--all", "--format", listing_format()]));
    every(refresh_every());
    state
}

fn tick() {
    if this.listing {
        return;
    }
    this.listing = true;
    run_then("docker", ["ps", "--all", "--format", listing_format()], #{ listed: true });
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
    if "listed" in event {
        this.listing = false;
        this.take_listing(event.result);
    }
    if "filter" in event {
        this.query = event.value;
        this.refilter();
    }
    if "copy" in event {
        copy(event.copy);
        notify(`copied ${event.copy}`);
    }
    if "did" in event {
        this.busy = "";
        if !event.result.ok {
            notify(`${event.did} failed: ${event.result.stderr}`);
        }
        this.tick();
    }
}

fn act(does) {
    let last = this.containers.len() - 1;
    switch does {
        "down" => { this.selected = clamp(this.selected + 1, 0, last); }
        "up" => { this.selected = clamp(this.selected - 1, 0, last); }
        "refresh" => { this.tick(); }
        "start_colima" => { open_shell("colima start"); }
    }
    if this.containers.is_empty() {
        return;
    }

    let container = this.containers[this.selected];
    let name = shell_quote(container.name);
    switch does {
        "logs" => { open_shell(`docker logs --follow --tail 500 ${name}`); }
        "shell" => {
            open_shell(`docker exec -it ${name} sh -c 'command -v bash >/dev/null && exec bash || exec sh'`);
        }
        "start" | "stop" | "restart" => {
            this.busy = `${does} ${container.name}…`;
            run_then("docker", [does, container.id], #{ did: `${does} ${container.name}` });
        }
    }

    let verbs = compose_verbs();
    if does in verbs && container.project != "" {
        let verb = verbs[does];
        this.busy = `${verb} ${container.project}…`;
        run_then(
            "docker",
            ["compose", "--project-name", container.project, verb],
            #{ did: `${verb} ${container.project}` }
        );
    }
}

fn take_listing(ran) {
    if !ran.ok {
        this.error = ran.stderr;
        this.containers = [];
        return;
    }
    this.error = "";
    let listed = [];
    for line in ran.stdout.split("\n") {
        line.trim();
        if line != "" {
            listed.push(parse_json(line));
        }
    }
    // A compose project's containers together, and in name order within it.
    listed.sort(|one, other| {
        let first = `${one.project}/${one.name}`;
        let second = `${other.project}/${other.name}`;
        if first < second { -1 } else if first > second { 1 } else { 0 }
    });
    this.listed = listed;
    this.refilter();
}

// Work out what the table shows from the listing and the filter box. The cursor stays on the
// container it was on, wherever that now is.
fn refilter() {
    let was_on = this.selected_id();
    let kept = [];
    for container in this.listed {
        if passes(container, this.query) {
            kept.push(container);
        }
    }
    this.containers = kept;
    for (container, index) in kept {
        if container.id == was_on {
            this.selected = index;
        }
    }
    this.selected = clamp(this.selected, 0, kept.len() - 1);
}

fn filter_box() {
    input("filter", this.query, "filter: name, image, project, status", #{ filter: true })
}

fn selected_id() {
    if this.containers.is_empty() {
        return "";
    }
    this.containers[this.selected].id
}

fn view() {
    let top = [heading("docker")];
    if this.busy != "" {
        top.push(text(this.busy, #{ ink: "accent" }));
    }

    if this.error != "" {
        let children = [row(top), separator(), text(this.error, #{ ink: "warn", mono: true })];
        if this.has_colima {
            children.push(row([button("start colima", #{ act: "start_colima" }), muted("runs colima start in a shell")]));
        }
        return column(children);
    }
    if this.listed.is_empty() {
        return column([row(top), separator(), muted("no containers")]);
    }
    if this.containers.is_empty() {
        return column([row(top), this.filter_box(), separator(), muted("no container matches")]);
    }

    let selected = this.containers[this.selected];
    let running = selected.state == "running";
    let actions = [
        button("start", #{ act: "start" }, running),
        button("stop", #{ act: "stop" }, !running),
        button("restart", #{ act: "restart" }, !running),
        button("logs", #{ act: "logs" }),
        button("shell", #{ act: "shell" }, !running),
    ];
    if selected.project != "" {
        actions += [
            muted(`${selected.project}:`),
            button("start all", #{ act: "project_start" }),
            button("stop all", #{ act: "project_stop" }),
            button("restart all", #{ act: "project_restart" }),
        ];
    }

    let inks = state_inks();
    let rows = [];
    for (container, index) in this.containers {
        let ink = if container.state in inks { inks[container.state] } else { "normal" };
        rows.push(#{
            cells: [
                text(container.state, #{ ink: ink }),
                text(container.name, #{ strong: true }),
                muted(container.project),
                text(container.image, #{ mono: true, small: true }),
                muted(container.status),
                text(ports_of(container.ports), #{ mono: true, small: true }),
            ],
            selected: index == this.selected,
            on_click: #{ select: index },
            on_double_click: #{ select: index, act: "logs" },
            menu: menu_of(container, index),
        });
    }

    column([
        row(top),
        this.filter_box(),
        row(actions),
        muted("s start · S stop · r restart · l logs · e shell · g refresh · double-click for logs"),
        separator(),
        table(["state", "name", "project", "image", "status", "ports"], rows),
    ])
}

// What a right click on a container offers. Each acting entry selects the container first, so
// it acts on the one clicked.
fn menu_of(container, index) {
    let menu = [#{ label: "logs", event: #{ select: index, act: "logs" } }];
    if container.state == "running" {
        menu += [
            #{ label: "shell", event: #{ select: index, act: "shell" } },
            #{ label: "restart", event: #{ select: index, act: "restart" } },
            #{ label: "stop", event: #{ select: index, act: "stop" } },
        ];
    } else {
        menu.push(#{ label: "start", event: #{ select: index, act: "start" } });
    }
    menu += [
        #{ label: "copy name", event: #{ copy: container.name } },
        #{ label: "copy id", event: #{ copy: container.id } },
    ];
    menu
}

// Docker lists each published port twice, once per address family. Once, without the address
// every port is published on, is what there is to read.
fn ports_of(ports) {
    let kept = [];
    for port in ports.split(", ") {
        if port.starts_with("[::]:") {
            continue;
        }
        if port.starts_with("0.0.0.0:") {
            port = port.sub_string(8);
        }
        kept.push(port);
    }
    kept.reduce(|joined, port| if joined == "" { port } else { `${joined}, ${port}` }, "")
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
