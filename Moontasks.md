# Moontasks

`moontasks` opens a sprint board over the repo. `moonreview` and `moonshell` reach the same
board from the command palette, in a tab of their own.
Each card is a task with an agent behind it: name a task, pick an agent, and it starts working
in the repo straight away.

| | |
| --- | --- |
| the box over the columns | filter the board: every column shows the cards whose title or notes hold what you typed, and hides the rest |
| `⌘F` on the board | put the keyboard in that box; Escape empties it |
| drag a card by its title | move it between columns, and put it where you drop it: the cards make room as you go and the column keeps that order |
| drag a column by its heading | move the column, cards and all |
| double click a heading | rename the column |
| `+` on a column's heading | a new task at the top of it |
| `+` under a column's last card | a new task at the bottom of it |
| `+` at the right-hand end | add a column |
| `[start]` at the foot of a card | everything a card starts, on the one menu: a review of the repo in a tab, a shell inside the task, or an agent |
| a running resource | click its name to bring its terminal back on screen |
| the notes under the title | the first lines of the task's `notes.md` — click them to open the file in a pane down the right, ready to edit |
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
screen. It stays marked for a moment after it lands, which is how you find it again among the
ones it landed between. A column being dragged does the same thing sideways.

## The columns

A board starts with TODO, IN PROGRESS and DONE, and they are yours from there: rename them,
drag them into another order, add your own, remove the ones you do not use.

A card names the column it is in rather than a place on the board, so moving a column moves
its cards with it and changes nothing about any of them. Renaming changes only what the column
is called — every card in it stays in it. A column still holding cards will not be removed:
it is the only record of where those cards are, so the board says to move them out first
rather than choosing somewhere for them.

The new-task box stands where its card will be — under the heading for the `+` on it, under
the last card for the `+` down there — so you can see where the task is about to land while
you name it.

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
    metadata.json     # title, column, place in the column, and the shells and agent runs
    brief.md          # what the agents working here have been told
    notes.md          # the task's description and shared notes, shown on the card
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
