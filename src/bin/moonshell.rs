//! `moonshell` — the window opens on a shell in the repo.

fn main() -> anyhow::Result<()> {
    moonreview::run(moonreview::Frame::Shell)
}
