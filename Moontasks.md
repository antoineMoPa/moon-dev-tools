# Moontasks

`moontasks` opens a sprint board over the repo. `moonreview` and `moonshell` reach the same
board from the command palette, in a tab of their own.
Each card is a task with an agent behind it: name a task, pick an agent, and it starts working
in the repo straight away.

| | |
| --- | --- |
| drag a card by its title | move it between columns, and put it where you drop it: the cards make room as you go and the column keeps that order |
| drag a column by its heading | move the column, cards and all |
| double click a heading | rename the column |
| `+` on a column's heading | a new task at the top of it |
| `+` under a column's last card | a new task at the bottom of it |
| `+` at the right-hand end | add a column |
| `[start review]` | open the review of the repo in a tab |
| `[launch shell]` / `[new agent]` | start another shell or another agent inside the task |
| a running resource | click its name to bring its terminal back on screen |
| the notes under the title | the first lines of the task's `notes.md` — click them to open the file in a pane down the right, ready to edit |
| `[add notes]` | the same, on a task that has none yet |

A card being dragged leaves the place it came from and takes up the one it is being held over,
which the cards around it move aside for, so the drop changes nothing that was not already on
screen. It stays marked for a moment after it lands, which is how you find it again among the
ones it landed between. A column being dragged does the same thing sideways.

## The columns

A board starts with TODO, IN PROGRESS and DONE, and they are yours from there: rename them,
drag them into another order, add your own, remove the ones you do not use.

A card names the column it is in rather than a place on the board, so moving a column moves
its cards with it and changes nothing about any of them. That name is the whole of it — what
the heading reads is what the card says and what a script calls it by, in any case you like:
`"todo"` is the TODO column. Renaming a column renames what its cards are in, so the cards
come along. A column still holding cards will not be removed: it is the only record of where
those cards are, so the board says to move them out first rather than choosing somewhere for
them.

The new-task box stands where its card will be — under the heading for the `+` on it, under
the last card for the `+` down there — so you can see where the task is about to land while
you name it.

A card changes column when you move it, or when a hook you are running moves it. Nothing an
agent does moves a card: starting one, resuming one, attaching a session, one exiting — the
card stays where you put it. A paused board is the whole rule again, and a board is paused
until you press play.

The one thing the board does with no hook involved is written in terms of a column, and it is
pinned to one a board starts with:

| | |
| --- | --- |
| `DONE` | where a card lets go of its shells |

Dragging that column elsewhere keeps its part in the rule; renaming or deleting it turns the
rule off rather than taking shells away in a column nobody named.

The columns live in `.moontasks/board.json`, written the first time you change one. A board
without that file has the three defaults.

## What the board decided

A hook fires with nobody in front of it, so the board writes down what it did and why in
`.moontasks/hooks.log` — *Autopilot → View Autopilot Logs*, or `autopilot log` in the command
palette. A hook says its own piece there with `print`; the board adds when a run ended, when a
card changed column, when it was started or paused, and anything a hook threw. A line the same
as the one before it is not written twice, so a tick saying why it is waiting every two seconds
is one line, and the log reads as the places its reasoning changed.

That log is the answer to "why did nothing happen": the shipped autopilot says when it is
waiting on a run, and when no card was ready for it.

## Signing

A run given its work up front has nobody sitting at it, and gpg's pinentry needs someone in
front of a terminal to ask for a passphrase. So the board starts those runs with
`commit.gpgsign=false` in their own environment: what they leave on a scratch branch is
unsigned, and the commit you make once you have reviewed it is signed by you as it always was.

It is the run's environment and nothing else — no config of yours is touched, and a shell you
open on a card has you in front of it and signs like anything else you do. The brief tells an
unattended run to leave git's configuration, gpg and this machine's services alone, because an
agent that meets a failing `git commit` will otherwise go and fix your gpg agent.

## Whose run is whose

A run started by a hook is the board's; a shell you opened, or an agent you are talking to, is
yours. The board writes down which when the run starts, and the shipped autopilot only waits on
its own — so sitting in an agent on one card does not stop work being picked up on another.
Resuming a run makes it yours, because from then on there is someone at the keyboard.

An agent that exits stops appearing as running the next time the board reads the folder. Its
card stays where you put it until you move it to DONE.

Closing an agent's tab does not end it. A task's shells belong to the task and keep running
with nothing attached until the card reaches DONE, so you can close a noisy agent and come
back to it. `stop` ends one on purpose, and `resume` starts it again where it left off.

The board is a folder in the repo, which is the whole of its state:

```
.moontasks/
  .gitignore          # ignores the board, written when the board is created
  board.json          # the columns, and whether the board is running
  hooks/
    tick.rhai         # what picks work up: this is autopilot, and it is yours
    run_finished.rhai # what happens when a run ends
    task_entered_column.rhai
    REFERENCE.md      # every function a hook can call, generated on every start
  fix-the-login-page-6f9c1e2a-…/
    metadata.json     # title, column, place, tags, worktree, and the agent runs
    brief.md          # what the agents working here have been told
    notes.md          # the task's description and shared notes, shown on the card
    runs/             # what each one-shot run printed, one file per run
    …                 # anything you or an agent puts here
```

Nothing about it is moonreview's alone: create a task with `mkdir` and a `metadata.json`,
move a card by editing the file, and the board picks it up on its next read.

A board is working state — running agents, scratch files, whatever an agent leaves in a task
folder — so it ignores itself from the moment it is created, and opening moonreview in a repo
leaves `git status` exactly as it was. To share the board with the rest of the team, delete
`.moontasks/.gitignore` and commit the folder; it will not come back. To keep a history of
your hooks without putting the board in the repo, `git init` inside `.moontasks/` and commit
them there.

## What the agents are told

An agent started on a task does not have to be asked twice. It is given a brief — the same
text `brief.md` holds — naming the task, its folder, and asking it to say plainly when the
work is ready to be looked at. It is also pointed at `notes.md` there: the task's description
and shared notes, which the card shows and either of you may write.

The card's title is then typed into its box, as if you had typed it: once the agent has
stopped printing, which is it having drawn an input to type into and being sat waiting — a
quarter of a second of silence, rather than a flat wait long enough for the slowest of the
three. Three seconds is the point where it types anyway. Nothing sends it. A title that is the
whole of what you wanted is one Enter away, and one that is not is there to be written over.
The Enter is never moonreview's, so an agent that was still asking whether it trusts the
folder loses the text rather than acting on it.

If you get there first it types nothing at all. A title arriving in the middle of a sentence
someone is writing is worse than no title, so the first keystroke of yours in that shell is
the end of it.

The brief is worth its own file because it is the difference between an agent that knows which
task it is on and one that has to be told twice.

## Runs that finish on their own

`[new agent]` can start one unattended run with the whole job supplied up front. It still
appears on the card, supports `stop` and `resume`, and saves its output under `runs/`.

Such a run is told something different from an agent you opened a conversation with: nobody is
reading it, there is nobody to ask, so where the work leaves something open it makes the call,
writes down which call it made in `notes.md`, and commits before it stops. What is on the
branch when it ends is how the board judges what it did.

It also says what it is doing while it does it. An agent asked for one answer prints nothing
until it has one, so a run is started in the mode where it reports itself and those reports are
turned back into readable lines on their way past — the same lines in the tab, in the
scrollback and in `runs/`.

## Tags, and a checkout of its own

`[tags]` adds arbitrary tags. `autopilot` is the one the shipped hooks read, and it is the only
tag a script may not write — adding and removing it is yours. `[worktree]` creates an isolated
branch and checkout under `~/.moonreview/worktrees`. Moving the card to DONE or deleting it
removes the checkout but keeps the branch.

## Reviewing a card

`[start review]` removes the worktree, checks its branch out in the repo, and opens the review.
It refuses if either the worktree or repo has uncommitted changes.

Reviewing is always yours. It puts a branch in the repo over whatever you have open, so nothing
unattended can reach it — there is no hook function that starts one.

## Hooks, and the board that runs itself

`[play]` at the top left starts the board acting on itself; `[pause]` stops it. Paused is where
a board starts, and pausing leaves whatever is already running alone — what stops is the board
acting again.

While it is running, three moments fire a script in `.moontasks/hooks/`:

| | |
| --- | --- |
| `tick.rhai` | every couple of seconds. This is what picks work up |
| `run_finished.rhai` | a one-shot run ended, with `event.commits` saying what it left on the branch |
| `task_entered_column.rhai` | a card arrived somewhere, whether you dragged it or a script moved it |

The shipped `tick.rhai` **is** autopilot: take the top card tagged `autopilot`, give it a
checkout, run a one-shot agent on what its notes say, one card at a time. It is written in
[Rhai](https://rhai.rs), it is installed once into a board that has none, and after that it is
yours — editing it is how you change what the board does. Nothing in the app has a path around
it. `[edit autopilot]`, beside the play button, opens that file in a pane; so do the Autopilot
menu and `edit autopilot` in the command palette.

What a hook may call is one table, listed in the generated `REFERENCE.md`. Three things are not
on it, and no prompt is involved in keeping them off: no script can create a card, write the
`autopilot` tag, or edit a file. A hook's hands are the runs it starts. A script that fails or
runs too long says so on the card it was working, and moves nothing.

## What survives a restart

The shells are the server's, so they outlive the window: closing it and opening it again
finds the same agents still working. They do not outlive the server — a `moonreview serve`
that is restarted, or a native window that is quit, takes its shells with it. What survives is
the record of every run, so the board comes back with each one marked as ended and `resume`
available to start it again. For Claude that resumes the exact session, because moonreview
gives it the session id when it starts it; Codex and OpenCode are resumed by their own
reckoning of the most recent session in the repo.
