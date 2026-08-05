mod agent;
mod api;
#[cfg(feature = "native")]
mod backend;
mod cli;
mod comments;
mod git;
mod moved_hunks;
#[cfg(feature = "native")]
mod native;
mod reviewed_cache;
mod server;
#[cfg(test)]
mod server_tests;
mod service;
mod terminal;

use anyhow::Result;

fn main() -> Result<()> {
    cli::run()
}
