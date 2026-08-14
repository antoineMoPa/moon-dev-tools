// Runs when a card arrives in a column — whether you dragged it there or a script moved it.
//
// Nothing here by default: the board should not do anything you did not ask it to. This is the
// hook to reach for when you want a column to mean something. A few things people write here:
//
//   // A column that runs the tests before you look at anything.
//   if event.column.to_lower() == "review" {
//       run_task_agent(event.task_id, "claude", "Run the test suite in this checkout and fix
//                                                anything that fails. Commit what you fix.");
//   }
//
//   // A note of when work started, on the card itself.
//   if event.column.to_lower() == "in progress" {
//       append_task_notes(event.task_id, `Picked up — moved out of ${event.from}.`);
//   }
//
// A column is named by the name its heading reads. The board does not care what case you
// name it in, but Rhai's `==` does, which is what the `to_lower()` above is for.
//
// `event` holds `task_id`, `title`, `column`, and `from`. `REFERENCE.md` beside this file
// lists everything you can call.
