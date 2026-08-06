//! `moontasks` — the window opens on the task board of the repo.

fn main() -> anyhow::Result<()> {
    moonreview::run(moonreview::Frame::Tasks)
}
