//! The command line: which command was asked for, and which frame a window opens on.
//!
//! Only the frames are compiled for the browser, whose window is started by its page - see
//! `crate::web` - rather than by a command.

#[cfg(not(target_arch = "wasm32"))]
mod agent;
#[cfg(not(target_arch = "wasm32"))]
mod args;
#[cfg(not(target_arch = "wasm32"))]
mod command;
mod frame;
#[cfg(not(target_arch = "wasm32"))]
mod launch;
#[cfg(not(target_arch = "wasm32"))]
mod open;
#[cfg(not(target_arch = "wasm32"))]
mod wire;
#[cfg(test)]
mod tests;

#[cfg(not(target_arch = "wasm32"))]
pub(crate) use command::run;
/// What bare `moon` prints, which the docs screenshot of a shell shows.
#[cfg(test)]
pub(crate) use command::help_text as moon_help_text;
#[cfg(not(target_arch = "wasm32"))]
use command::{MoonCommand, help_text_for};
pub(crate) use frame::FRAMES;
pub use frame::Frame;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use frame::{FRAME_ENV, NEW_WINDOW_FRAMES, PROGRAM};
