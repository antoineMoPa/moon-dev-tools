# libghostty-vt-sys, vendored

`libghostty-vt-sys` 0.2.1 from crates.io, kept here so moon's browser build can compile Ghostty's
VT engine to wasm. Root `Cargo.toml` puts it in place of the crates.io one with
`[patch.crates-io]`. What differs from 0.2.1, all for `wasm32-unknown-unknown`:

- `build.rs`, `zig_target`: `wasm32-unknown-unknown` builds as Zig's `wasm32-freestanding`.
  Ghostty's own build supports it and emits `libghostty-vt.a` for it.
- `build.rs`: Ghostty's wasm install step leaves out its headers, so they are copied from the
  Ghostty source it built.
- `src/bindings.rs`: the generated layout assertions hold only on 64-bit targets, so each is
  `#[cfg(target_pointer_width = "64")]`. The struct definitions themselves use `usize` and
  pointers, and hold on wasm32 as they are.

Ghostty's wasm build also imports one function from its host, `env.log`, for its log messages.
moon's build.rs points that import at `web/ghostty_env.js`.

Worth offering upstream; drop this copy once a release carries them. Upstream is
https://github.com/uzaaft/libghostty-rs, and its PR #36 (open since May 2026, stalled on conflicts)
makes the same `build.rs` changes but turns bindgen's layout tests off instead of wrapping them.
Until then, this copy could also live as a git `[patch.crates-io]` on a fork of upstream, branched
from 0.2.1's commit `46a9d2ac`, instead of here.
