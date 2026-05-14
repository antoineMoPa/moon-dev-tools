mod agent;
mod api;
mod cli;
mod comments;
mod git;
mod moved_hunks;
mod reviewed_cache;
mod server;

use anyhow::Result;

fn main() -> Result<()> {
    cli::run()
}
