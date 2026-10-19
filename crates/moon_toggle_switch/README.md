# moon_toggle_switch

A pill-shaped segmented switch for [egui](https://github.com/emilk/egui): one track, a word per
choice, a thumb under the chosen one. No borders; the two grounds make the switch.

```rust
let switched = ToggleSwitch::new(&[(Mode::Files, "files"), (Mode::Content, "content")], mode)
    .style(ToggleSwitchStyle::of(ui))
    .show(ui);
if let Some(to) = switched {
    mode = to;
}
```

`show_in` draws it inside a rect you already have, to lay it over the end of a text line.
