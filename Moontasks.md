# Moontasks

`moontasks` opens a sprint board over the repo. `moonreview` and `moonshell` reach the same
board from the command palette, in a tab of their own.
Each card is a task with an agent behind it: name a task, and `[start]` puts an agent to work
on it in the repo.

| | |
| --- | --- |
| the box over the columns | filter the board: every column shows the cards whose title or notes hold what you typed, and hides the rest |
| `⌘F` on the board | put the keyboard in that box; Escape empties it |
| drag a card | move it between columns, and put it where you drop it: the cards make room as you go and the column keeps that order |
| `cmd+click` a card | mark it, or take the mark off - anywhere on the card, buttons and all |
| `shift+click` a card | mark the run of cards between it and the last one clicked |
| Escape, or a click on the board beside the cards | let the marks go |
| drag a column by its heading | move the column, cards and all |
| click a card's title | open the task's own pane: its title and notes, and what it can start |
| double click a card's title | rename the task |
| double click a heading | rename the column |
| `+` on a column's heading | write a new task, for the top of that column; `[create]` on that pane makes the card |
| `+` under a column's last card | the same, for the bottom of it |
| `+` at the right-hand end | add a column |
| `[start]` at the foot of a card | everything a card starts, on the one menu: a review of the repo in a tab, a shell inside the task, an agent, or `file…` to put a file of the repo on the card |
| a running resource | click its name to bring its terminal back on screen |
| a card drawn in the accent color | a marked card: one you clicked, or the task whose tab was last in front, which is the same thing said twice |
| a file on a card | click its path to open it in a pane; the mark at the end takes it off the card, and leaves the file where it is |
| the notes under the title | the first lines of the task's `notes.md` — click them to open the task's own pane with the keyboard in its notes box, ready to write |
| `[add notes]` | the same, on a task that has none yet |

`[add notes]` and `[start]` are a card's offers: they fade up when the pointer comes onto the
card and fade away again when it leaves, over a sixth of a second, and hold their rows while
they are out of sight — so a card at rest is its title and its description, and no card changes
height as the pointer crosses the column. A card whose `[start]` menu is up keeps them out,
because the menu hangs below the card and reaching into it takes the pointer off.

A filtered board is still the board: the columns keep their place and their names, a column
whose cards are all hidden says so rather than reading as an empty one, and a card can be
dragged from where the filter left it — dropped above a card showing, it lands above that card,
whatever is hidden between them. Creating a task empties the query, so the new card is on the
board rather than behind a filter that was typed before it existed.

A card being dragged leaves the place it came from and takes up the one it is being held over,
which the cards around it move aside for, so the drop changes nothing that was not already on
screen. What was carried is marked where it lands, which is how you find it again among the
cards it landed between - and it stays marked, rather than flashing and fading, because that
mark is the same one everything else on the board is picked out by. A column being dragged does
the same thing sideways.

## Marks, and cards moved together

There is one mark on the board and it means one thing: this card is picked out. A click on a
card marks it and opens its page - and the two keep each other, so a card let go of takes its
page with it. Only the page: a shell started in the task, or a file opened off its card, is a
tab of yours and stays until you close it. `cmd+click` marks another beside it. `shift+click` marks the
run between that card and the last one clicked, read down the column the two are in - among the
cards the filter is showing, so what is taken is what the eye sees between them. And a task's
own tab coming to the front marks its card, because that is the same thing said another way:
this is the task being worked in. One card marked is a task to read; several are a group to
drag. Escape, or a click on the board beside the cards, lets them go - and puts away the pages
they had open.

The two keys do two jobs: `shift` is the run key, `cmd` the one-card key - which is how a card
in the middle of a run is taken back out of it. While either is held the whole card is one
thing to click: a card is a stack of buttons, and a click meant to mark it must not press
whichever of them it landed on.

Picking up any marked card carries all of them, and they land in the column they are dropped on
as one run, in the order the board already had them. The card on the cursor says how many it is
bringing; the others are drawn where they are going, faint, as it is carried. A card picked up
with cmd or shift held joins the marks rather than replacing them - the keys that gather cards
are held down over the gesture that takes them somewhere. A card picked up with nothing held
and no mark on it goes alone, and becomes the mark itself.

A card is picked up anywhere on it, buttons included: a press that carries the card is the card
being moved, and one that stays where it went down is a click - on the card, or on the button
it landed on. That is measured in distance rather than time, so a slow, careful click is still
a click. A drag carries cards only when it began on one that was showing: a press on the board
beside them lets the marks go, wherever it wanders afterwards.

## Files on a card

`file…` on the `[start]` menu opens the file finder on the repo - the same one `cmd+P` opens -
and the file picked there goes onto the card and opens in a pane. A task usually has a handful
of files it is about, and this is where they are kept: the card carries the path, and clicking
it opens the file.

A file is written into the task's `metadata.json` the way an agent run is, so it is still on
the card after a restart, and a link is only made to a file that is in the working tree at the
time - a card is a way back to a file, and one pointing at nothing is worse than none. The
mark at the end of the row takes the file off the card without asking, because nothing is lost
by it: the file stays exactly where it is, and linking it again is one menu away.

## The columns

A board starts with TODO, IN PROGRESS and DONE, and they are yours from there: rename them,
drag them into another order, add your own, remove the ones you do not use.

A card names the column it is in rather than a place on the board, so moving a column moves
its cards with it and changes nothing about any of them. Renaming changes only what the column
is called — every card in it stays in it. A column still holding cards will not be removed:
it is the only record of where those cards are, so the board says to move them out first
rather than choosing somewhere for them.

A `+` opens a pane to write the new task on rather than a box on the board: the same pane a
card opens, with its title and notes empty and `[create]` standing where that task's `[start]`
will. `[create]` — or Enter in the title box — makes the card, and that very pane becomes the
new task's own, with its notes already written. Nothing is created before it is pressed, and
closing the tab instead makes nothing: a task's folder under `.moontasks` is named after its
title and keeps that name for good, so there is no task until there is a name for it. The `+`
that was pressed is the end of the column the card joins — the heading's puts it on top, the
one under the last card puts it at the bottom.

The column holds the place while you write: an empty card stands at that end of it from the
moment the `+` is pressed until `[create]` fills it in, outlined the way the hole a dragged card
leaves is, because it means the same thing — a card is going here. So the task is written with
its place on the board already in front of you, rather than turning up somewhere once it is
made.

One new-task pane is open at a time: pressing `+` again brings the one you are writing forward
rather than starting a second, and a card clicked while it is open opens beside it rather than
sweeping it away.

A card changes column only when you move it. Nothing the board or an agent does moves a card:
starting an agent, resuming one, attaching a session, an agent exiting - the card stays where
you put it.

The one thing the board does on its own is written in terms of a column, and it is pinned to
one a board starts with:

| | |
| --- | --- |
| `done` | where a card lets go of its shells |

It is pinned by id, so renaming that column or dragging it elsewhere keeps its part in the
rule and nothing changes. Deleting it turns the rule off rather than taking shells away in a
column nobody pinned it to.

The columns live in `.moontasks/board.json`, written the first time you change one. A board
without that file has the three defaults.

An agent that exits stops appearing as running the next time the board reads the folder. Its
card stays where you put it until you move it to DONE.

Closing an agent's tab does not end it. A task's shells belong to the task and keep running
with nothing attached until the card reaches DONE, so you can close a noisy agent and come
back to it. `stop` ends one on purpose, and `resume` starts it again where it left off.

The board is a folder in the repo, which is the whole of its state:

```
.moontasks/
  .gitignore          # ignores the whole board, written when the board is created
  board.json          # the columns, once you have changed them
  fix-the-login-page-6f9c1e2a-…/
    metadata.json     # title, column, place in the column, the agent runs and the linked files
    brief.md          # what the agents working here have been told
    notes.md          # the task's description and shared notes, shown on the card
    request_review.md        # how to write the file below, for an agent about to
    request_for_review.txt   # the repos this work touched, in deploy order, once there are any
    …                 # anything you or an agent puts here
```

Nothing about it is moonreview's alone: create a task with `mkdir` and a `metadata.json`,
move a card by editing the file, and the board picks it up on its next read.

A board is working state — running agents, scratch files, whatever an agent leaves in a task
folder — so it ignores itself from the moment it is created, and opening moonreview in a repo
leaves `git status` exactly as it was. To share the board with the rest of the team instead,
delete `.moontasks/.gitignore` and commit the folder; it will not come back.

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

The brief also says where to write the deploy list - `request_for_review.txt`, above - in one
line, pointing at `request_review.md`, which is written into the folder beside it and holds the
format. It is a file rather than more of the brief because the brief is a system prompt on every
run of every task, and the format is read once, by the agent that has work to hand over.

The brief is worth its own file because it is the difference between an agent that knows which
task it is on and one that has to be told twice.

## What survives a restart

The shells are the server's, so they outlive the window: closing it and opening it again
finds the same agents still working. They do not outlive the server — a `moonreview serve`
that is restarted, or a native window that is quit, takes its shells with it. What survives is
the record of every run, so the board comes back with each one marked as ended and `resume`
available to start it again. For Claude that resumes the exact session, because moonreview
gives it the session id when it starts it; Codex and OpenCode are resumed by their own
reckoning of the most recent session in the repo.
