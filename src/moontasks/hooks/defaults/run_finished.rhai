// Runs when a one-shot agent ends, however it ended.
//
// Whether the run did anything is not a judgement call: `event.commits` is how many commits
// the card's branch has that it did not when its checkout was made. A run that committed
// nothing left nothing to look at, whatever it said while it was running.
//
// Nothing here starts a review. Reviewing checks the card's branch out in the repo itself, over
// whatever you have open, so it stays a button you press — [start review] on the card.

if event.commits == 0 {
    add_task_tag(event.task_id, "stuck");
    append_task_notes(
        event.task_id,
        `[autopilot] ${event.agent} ended without committing anything. Its output is on the card — open the run to read it. Moving this card back to TODO will start it again.`,
    );
    return;
}

add_task_tag(event.task_id, "needs-review");
append_task_notes(
    event.task_id,
    `[autopilot] ${event.agent} finished, with ${event.commits} commit(s) on this card's branch. Press [start review] to check that branch out here and look at it.`,
);
