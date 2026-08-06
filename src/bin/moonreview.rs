//! `moonreview` — the window opens on a review of the repo.

fn main() -> anyhow::Result<()> {
    moonreview::run(moonreview::Frame::Review)
}
