//! `moonshell` - the window opens on a shell in the folder.

fn main() -> anyhow::Result<()> {
    moonreview::run(moonreview::Frame::Shell)
}
