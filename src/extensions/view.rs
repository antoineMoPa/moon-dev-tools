//! What a script's `view()` answers with, as the window reads it.
//!
//! A view is a tree of maps, each naming its `kind`. Anything the window does not know - a
//! kind it has no drawing for, a field it would not read - is refused, and the pane says so
//! over the last view it could draw: a field misspelled in a script is a mistake to be told
//! about, not something to be quietly left off the screen.

use anyhow::{Result, bail};
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Element {
    Text {
        text: String,
        #[serde(default)]
        ink: Ink,
        #[serde(default)]
        strong: bool,
        /// Set in the code face, for what lines up in columns: sizes, ids, paths.
        #[serde(default)]
        mono: bool,
        #[serde(default)]
        small: bool,
    },
    Heading {
        text: String,
    },
    /// Side by side, wrapping onto another line when the pane is too narrow.
    Row {
        children: Vec<Element>,
    },
    /// One under the other.
    Column {
        children: Vec<Element>,
    },
    Button {
        label: String,
        /// What `update` is called with when it is clicked.
        on_click: Value,
        #[serde(default)]
        disabled: bool,
    },
    /// Rows under a heading of column names. Only the rows on screen are drawn, so a table of
    /// thousands costs what a table of twenty does.
    ///
    /// Takes the height left in the pane, so it is the last thing in a view.
    Table {
        columns: Vec<String>,
        rows: Vec<TableRow>,
    },
    /// A block of monospace text, scrolled to its end - the tail of a log.
    Code {
        text: String,
    },
    Separator,
    /// A line to type in. What is typed is kept by the window, so typing never waits on the
    /// script; each change is sent to `update` as `on_change` with the text in its `value`.
    Input {
        /// Tells this box from any other in the view, from one view to the next: it is what
        /// keeps the cursor in the box while the view is drawn again under it.
        id: String,
        value: String,
        #[serde(default)]
        hint: String,
        /// A map; what is typed goes in its `value`.
        on_change: Value,
    },
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct TableRow {
    /// One per column. A button in a cell would fight the row for the click, so a row's
    /// actions belong in its `menu`, or in a toolbar over the table acting on the selected row.
    pub(crate) cells: Vec<Element>,
    #[serde(default)]
    pub(crate) selected: bool,
    #[serde(default)]
    pub(crate) on_click: Option<Value>,
    #[serde(default)]
    pub(crate) on_double_click: Option<Value>,
    /// What a right click on the row offers, in order.
    #[serde(default)]
    pub(crate) menu: Vec<MenuItem>,
}

/// One entry of a row's menu: what it says, and what `update` is called with when it is
/// picked.
#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct MenuItem {
    pub(crate) label: String,
    pub(crate) event: Value,
}

/// The palette's colors a script can pick from, by what they mean rather than by value, so
/// an extension reads right in both themes and every workspace color.
#[derive(Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Ink {
    #[default]
    Normal,
    Muted,
    Accent,
    Warn,
    Added,
    Removed,
}

impl Element {
    /// What serde cannot say about a view: that every row of a table has a cell for each of
    /// its columns, and that an input's `on_change` has room for what is typed.
    pub(crate) fn check(&self) -> Result<()> {
        match self {
            Self::Row { children } | Self::Column { children } => {
                children.iter().try_for_each(Self::check)
            }
            Self::Table { columns, rows } => {
                for (index, row) in rows.iter().enumerate() {
                    if row.cells.len() != columns.len() {
                        bail!(
                            "row {index} of a table has {} cells for {} columns",
                            row.cells.len(),
                            columns.len()
                        );
                    }
                    row.cells.iter().try_for_each(Self::check)?;
                }
                Ok(())
            }
            Self::Input { on_change, .. } => match on_change {
                Value::Object(fields) if !fields.contains_key("value") => Ok(()),
                _ => bail!(
                    "an input's on_change is a map without a `value`, which is where what is \
                     typed goes"
                ),
            },
            Self::Text { .. }
            | Self::Heading { .. }
            | Self::Button { .. }
            | Self::Code { .. }
            | Self::Separator => Ok(()),
        }
    }

    /// The first selected row of the first table in the view: the one the keyboard is on, which
    /// the table scrolls to when it moves.
    pub(crate) fn selected_row(&self) -> Option<usize> {
        match self {
            Self::Row { children } | Self::Column { children } => {
                children.iter().find_map(Self::selected_row)
            }
            Self::Table { rows, .. } => rows.iter().position(|row| row.selected),
            Self::Text { .. }
            | Self::Heading { .. }
            | Self::Button { .. }
            | Self::Code { .. }
            | Self::Separator
            | Self::Input { .. } => None,
        }
    }
}
