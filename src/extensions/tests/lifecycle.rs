//! A script's life in its pane: what it draws, what it is sent, what is refused, and how it is
//! read again, restarted and paused.

use std::time::Duration;

use serde_json::json;

use super::{Heard, Scratch, script, start, system_path};
use crate::extensions::{Extension, Input, Source};

#[test]
fn a_view_is_drawn_from_the_state_and_an_event_changes_the_state() {
    // Arrange
    let running = start(
        script(
            r#"
            fn init(root) { #{ count: 0 } }
            fn view() { column([text(`count ${this.count}`), button("more", #{ add: 2 })]) }
            fn update(event) { this.count += event.add; }
            "#,
        ),
        &std::env::temp_dir(),
        system_path(),
    );
    let mut heard = Heard::default();
    heard.until(&running, |heard| heard.shows("count 0"));

    // Act: what the button carries, the way a click sends it.
    running.send(Input::Event(json!({ "add": 2 })));

    // Assert
    heard.until(&running, |heard| heard.shows("count 2"));
    assert_eq!(heard.error, None);
}

#[test]
fn a_key_reaches_on_key_as_the_character_it_typed_or_the_name_of_the_key() {
    // Arrange
    let running = start(
        script(
            r#"
            fn init(root) { #{ keys: [] } }
            fn view() { text(`${this.keys}`) }
            fn on_key(key) { this.keys.push(key); }
            "#,
        ),
        &std::env::temp_dir(),
        system_path(),
    );
    let mut heard = Heard::default();

    // Act
    running.send(Input::Key("^".to_string()));
    running.send(Input::Key("Enter".to_string()));

    // Assert
    heard.until(&running, |heard| heard.shows(r#"["^", "Enter"]"#));
}

#[test]
fn a_view_of_a_kind_the_window_does_not_draw_is_refused_and_the_last_view_stays() {
    // Arrange
    let running = start(
        script(
            r#"
            fn init(root) { #{ broken: false } }
            fn view() { if this.broken { #{ kind: "blink", text: "!" } } else { text("fine") } }
            fn update(event) { this.broken = true; }
            "#,
        ),
        &std::env::temp_dir(),
        system_path(),
    );
    let mut heard = Heard::default();
    heard.until(&running, |heard| heard.shows("fine"));

    // Act
    running.send(Input::Event(json!({})));

    // Assert
    heard.until(&running, |heard| heard.error.is_some());
    let error = heard.error.clone().expect("an error");
    assert!(
        error.contains("blink"),
        "the error names what was wrong: {error}"
    );
    assert!(
        heard.shows("fine"),
        "the last view that could be drawn is still drawn"
    );
}

#[test]
fn a_misspelled_field_is_refused_rather_than_left_off_the_screen() {
    // Arrange / Act
    let running = start(
        script(
            r#"
            fn init(root) { #{} }
            fn view() { #{ kind: "text", txt: "hello" } }
            "#,
        ),
        &std::env::temp_dir(),
        system_path(),
    );
    let mut heard = Heard::default();

    // Assert
    heard.until(&running, |heard| heard.error.is_some());
    let error = heard.error.clone().expect("an error");
    assert!(error.contains("txt"), "the error names the field: {error}");
}

#[test]
fn a_table_row_without_a_cell_for_each_column_is_refused() {
    // Arrange / Act
    let running = start(
        script(
            r#"
            fn init(root) { #{} }
            fn view() { table(["name", "size"], [#{ cells: [text("a")] }]) }
            "#,
        ),
        &std::env::temp_dir(),
        system_path(),
    );
    let mut heard = Heard::default();

    // Assert
    heard.until(&running, |heard| heard.error.is_some());
    let error = heard.error.clone().expect("an error");
    assert!(error.contains("1 cells for 2 columns"), "got: {error}");
}

#[test]
fn a_script_that_never_finishes_is_stopped_and_says_so() {
    // Arrange / Act
    let running = start(
        script(
            r#"
            fn init(root) { #{} }
            fn view() { loop {} }
            "#,
        ),
        &std::env::temp_dir(),
        system_path(),
    );
    let mut heard = Heard::default();

    // Assert
    heard.until(&running, |heard| heard.error.is_some());
    let error = heard.error.clone().expect("an error");
    assert!(
        error.contains("view"),
        "the error says where it stopped: {error}"
    );
}

#[test]
fn a_script_file_is_read_again_when_it_changes_and_the_pane_keeps_its_state() {
    // Arrange
    let scratch = Scratch::new("reload");
    let file = scratch.write(
        "counter.rhai",
        r#"
        fn init(root) { #{ count: 0 } }
        fn view() { text(`first ${this.count}`) }
        fn update(event) { this.count += 1; }
        "#,
    );
    let running = start(
        Extension {
            name: "counter".to_string(),
            about: String::new(),
            source: Source::File(file.clone()),
        },
        &scratch.dir,
        system_path(),
    );
    let mut heard = Heard::default();
    running.send(Input::Event(json!({})));
    heard.until(&running, |heard| heard.shows("first 1"));

    // Act: the same script, saying it differently. The pause is so the file's time moves on.
    std::thread::sleep(Duration::from_millis(20));
    std::fs::write(
        &file,
        r#"
        fn init(root) { #{ count: 0 } }
        fn view() { text(`second ${this.count}`) }
        fn update(event) { this.count += 1; }
        "#,
    )
    .expect("failed to rewrite the script");

    // Assert
    heard.until(&running, |heard| heard.shows("second 1"));
}

#[test]
fn an_error_stays_up_while_ticks_go_on_and_is_logged_once() {
    // Arrange
    let running = start(
        script(
            r#"
            fn init(root) { every(20); #{ ticks: 0 } }
            fn tick() { this.ticks += 1; }
            fn view() { text(`ticks ${this.ticks}`) }
            fn update(event) { throw "no good"; }
            "#,
        ),
        &std::env::temp_dir(),
        system_path(),
    );
    let mut heard = Heard::default();
    heard.until(&running, |heard| heard.view.is_some());

    // Act
    running.send(Input::Event(json!({})));
    heard.until(&running, |heard| heard.error.is_some());
    let when_it_failed = heard.only_text();

    // Assert: ticks go on drawing, and the error stays with them.
    heard.until(&running, |heard| heard.only_text() != when_it_failed);
    std::thread::sleep(Duration::from_millis(100));
    heard.until(&running, |_| true);
    assert!(
        heard
            .error
            .as_deref()
            .is_some_and(|error| error.contains("no good"))
    );
    assert_eq!(heard.logged_failures(), 1, "{:?}", heard.effects);
}

#[test]
fn a_script_saved_with_a_new_field_in_init_gets_it_and_keeps_the_rest() {
    // Arrange
    let scratch = Scratch::new("new-field");
    let file = scratch.write(
        "counter.rhai",
        r#"
        fn init(root) { #{ count: 0 } }
        fn view() { text(`count ${this.count}`) }
        fn update(event) { this.count += 1; }
        "#,
    );
    let running = start(
        Extension {
            name: "counter".to_string(),
            about: String::new(),
            source: Source::File(file.clone()),
        },
        &scratch.dir,
        system_path(),
    );
    let mut heard = Heard::default();
    running.send(Input::Event(json!({})));
    heard.until(&running, |heard| heard.shows("count 1"));

    // Act: a field that was not there when the pane started.
    std::thread::sleep(Duration::from_millis(20));
    std::fs::write(
        &file,
        r#"
        fn init(root) { #{ count: 0, label: "hits" } }
        fn view() { text(`${this.label} ${this.count}`) }
        fn update(event) { this.count += 1; }
        "#,
    )
    .expect("failed to rewrite the script");

    // Assert
    heard.until(&running, |heard| heard.shows("hits 1"));
}

#[test]
fn a_restart_starts_the_script_over_from_init() {
    // Arrange
    let running = start(
        script(
            r#"
            fn init(root) { #{ count: 0 } }
            fn view() { text(`count ${this.count}`) }
            fn update(event) { this.count += 1; }
            "#,
        ),
        &std::env::temp_dir(),
        system_path(),
    );
    let mut heard = Heard::default();
    running.send(Input::Event(json!({})));
    heard.until(&running, |heard| heard.shows("count 1"));

    // Act
    running.send(Input::Restart);

    // Assert
    heard.until(&running, |heard| heard.shows("count 0"));
}

#[test]
fn a_hidden_pane_is_not_ticked_and_catches_up_as_it_shows() {
    // Arrange
    let running = start(
        script(
            r#"
            fn init(root) { every(20); #{ ticks: 0 } }
            fn tick() { this.ticks += 1; }
            fn view() { text(`ticks ${this.ticks}`) }
            "#,
        ),
        &std::env::temp_dir(),
        system_path(),
    );
    let mut heard = Heard::default();
    heard.until(&running, |heard| heard.view.is_some());

    // Act
    running.send(Input::Visible(false));
    std::thread::sleep(Duration::from_millis(60));
    heard.until(&running, |_| true);
    let hidden_at = heard.only_text();
    std::thread::sleep(Duration::from_millis(200));
    heard.until(&running, |_| true);

    // Assert
    assert_eq!(heard.only_text(), hidden_at, "no tick while hidden");
    running.send(Input::Visible(true));
    heard.until(&running, |heard| heard.only_text() != hidden_at);
}

#[test]
fn a_constant_at_the_top_of_a_script_reaches_its_functions_through_global() {
    // Arrange / Act
    let running = start(
        script(
            r#"
            const GREETING = "hello";
            fn init() { #{} }
            fn view() { text(global::GREETING) }
            "#,
        ),
        &std::env::temp_dir(),
        system_path(),
    );
    let mut heard = Heard::default();

    // Assert: `init` also left the project's folder out, which it may.
    heard.until(&running, |heard| heard.shows("hello"));
}

#[test]
fn only_a_script_with_on_key_takes_keys_from_the_window() {
    // Arrange / Act
    let keyless = start(
        script("fn init(root) { #{} }\nfn view() { text(\"fine\") }"),
        &std::env::temp_dir(),
        system_path(),
    );
    let keyed = start(
        script("fn init(root) { #{} }\nfn view() { text(\"fine\") }\nfn on_key(key) {}"),
        &std::env::temp_dir(),
        system_path(),
    );
    let (mut keyless_heard, mut keyed_heard) = (Heard::default(), Heard::default());

    // Assert
    keyless_heard.until(&keyless, |heard| heard.takes_keys.is_some());
    keyed_heard.until(&keyed, |heard| heard.takes_keys.is_some());
    assert_eq!(keyless_heard.takes_keys, Some(false));
    assert_eq!(keyed_heard.takes_keys, Some(true));
}

#[test]
fn an_input_whose_on_change_already_has_a_value_is_refused() {
    // Arrange / Act
    let running = start(
        script(
            r#"
            fn init(root) { #{} }
            fn view() { input("filter", "", "", #{ value: "mine" }) }
            "#,
        ),
        &std::env::temp_dir(),
        system_path(),
    );
    let mut heard = Heard::default();

    // Assert
    heard.until(&running, |heard| heard.error.is_some());
    let error = heard.error.clone().expect("an error");
    assert!(error.contains("without a `value`"), "{error}");
}
