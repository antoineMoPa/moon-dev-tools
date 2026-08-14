// Runs every couple of seconds while the board is running — the play button up on the left.
//
// This is autopilot, and it is all here: take the top card tagged `autopilot`, give it a
// checkout of its own, and set a one-shot agent to work on what the card's notes say. One card
// at a time.
//
// It is yours. Change the agent, take work from a different column, refuse cards that have
// already been tried — it is an ordinary script, and nothing in the app depends on it saying
// what it says today. `REFERENCE.md` beside this file lists everything you can call.

// Who does the work.
const AGENT = "claude";

// Where work is taken from, and where a card goes while it is being worked. A column is named
// by the name its heading reads, and the case is nobody's business — `"todo"` is the TODO
// column. Rename either of these on the board and put the new name here, or the board will say
// it cannot find it rather than guess which column you meant.
const READY = "TODO";
const WORKING = "IN PROGRESS";

// What the run_finished hook marks cards with. Cleared when a card is picked up again.
const MARKS = ["needs-review", "stuck"];

// Whether two names are the same column. The board reads them that way; Rhai's `==` is
// stricter than the board and would call TODO and Todo two columns.
fn same_column(one, other) {
    one.to_lower() == other.to_lower()
}

// The two columns as the board has them, so what goes on a card is spelled the way the heading
// is. A board missing either is not one this script can work, and it says so rather than
// starting a run that would then have nowhere to go.
let ready = ();
let working = ();
for column in list_columns() {
    if same_column(column.name, READY) { ready = column.name; }
    if same_column(column.name, WORKING) { working = column.name; }
}
if type_of(ready) == "()" {
    throw `this board has no ${READY} column — that is where tick.rhai takes work from.`;
}
if type_of(working) == "()" {
    throw `this board has no ${WORKING} column — that is where tick.rhai puts work it started.`;
}

let tasks = list_tasks();

// One at a time: nothing new starts while another run the board started is still going. Take
// this out and the board will work every tagged card at once, which is a real thing to want
// and a good way to fight over a repo.
//
// Only the board's own runs count. A shell you opened, or an agent you are sitting and
// talking to, is `started_by == "person"` and none of the board's business — it works around
// you rather than waiting for you to finish.
for task in tasks {
    for run in task.resources {
        // Read as "not yours" rather than "the board's": a run from a moonreview too old to
        // record who started it says nothing here, and the board waits on it as it always did.
        if run.running && run.started_by != "person" {
            print(`waiting on ${run.label} on "${task.title}" — nothing new is picked up while a run the board started is going`);
            return;
        }
    }
}

for task in tasks {
    if !same_column(task.status, ready) { continue; }
    if !task.tags.contains("autopilot") { continue; }

    // The notes are the brief, so a card nobody has written yet is not work. `trim` changes
    // the string it is called on, which is why this is a copy.
    let notes = task.notes;
    notes.trim();
    if notes == "" { continue; }

    // A card sent back for another go still has its checkout, and its old marks.
    if type_of(task.worktree) == "()" {
        create_worktree(task.id);
    }
    for mark in MARKS {
        remove_task_tag(task.id, mark);
    }

    print(`picked up "${task.title}" — ${AGENT} is on it in its own checkout`);
    run_task_agent(task.id, AGENT, notes);
    move_task(task.id, working, 0);
    append_task_notes(task.id, `[autopilot] ${AGENT} is working on this in its own checkout.`);
    return;
}

// Nothing matched, which is the usual state of a board and worth saying once: the log is where
// you look when you expected a card to be picked up and it was not.
print(`nothing to pick up — no card in ${ready} is tagged autopilot with notes written`);
