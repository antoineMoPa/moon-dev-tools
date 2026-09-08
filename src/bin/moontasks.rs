//! `moontasks` - the window opens on the task board of the folder.

fn main() -> anyhow::Result<()> {
    moonreview::run(moonreview::Frame::Tasks)
}
