//! The two extensions built into the executable, `files` and `docker`, run the way their panes
//! run them - docker against a stand-in that says what a daemon would.

use std::time::{Duration, Instant};

use serde_json::json;

use super::{Heard, PATIENCE, Scratch, start, system_path};
use crate::extensions::{Effect, Input, named};

#[test]
fn the_extensions_the_executable_ships_are_offered_with_what_they_are_for() {
    // Act
    let files = named("files").expect("files is shipped");
    let docker = named("docker").expect("docker is shipped");

    // Assert
    assert!(files.about.contains("folders"), "got {:?}", files.about);
    assert!(
        docker.about.contains("containers"),
        "got {:?}",
        docker.about
    );
}

/// A folder to browse: a subfolder, a file, and a dotfile.
fn browsed(name: &str) -> Scratch {
    let scratch = Scratch::new(name);
    scratch.write("a-folder/inner.txt", "inside\n");
    scratch.write("b.txt", "hello\n");
    scratch.write(".hidden", "secret\n");
    scratch
}

#[test]
fn files_lists_folders_before_files_and_leaves_dotfiles_out() {
    // Arrange / Act
    let scratch = browsed("files-list");
    let running = start(
        named("files").expect("files is shipped"),
        &scratch.dir,
        system_path(),
    );
    let mut heard = Heard::default();

    // Assert
    heard.until(&running, |heard| heard.shows("b.txt"));
    let texts = heard.texts();
    let folder = texts.iter().position(|text| text == "a-folder/");
    let file = texts.iter().position(|text| text == "b.txt");
    assert!(folder < file, "folders come first: {texts:?}");
    assert!(!heard.shows(".hidden"), "dotfiles are left out: {texts:?}");
}

#[test]
fn files_goes_into_a_folder_and_back_up_to_where_it_was() {
    // Arrange
    let scratch = browsed("files-walk");
    let running = start(
        named("files").expect("files is shipped"),
        &scratch.dir,
        system_path(),
    );
    let mut heard = Heard::default();
    heard.until(&running, |heard| heard.shows("b.txt"));

    // Act: past `..` onto the folder, and into it.
    running.send(Input::Key("n".to_string()));
    running.send(Input::Key("Enter".to_string()));
    heard.until(&running, |heard| heard.shows("inner.txt"));
    running.send(Input::Key("^".to_string()));

    // Assert: back, with the cursor on the folder that was left.
    heard.until(&running, |heard| heard.shows("b.txt"));
    let selected = heard.selected_row();
    assert!(
        selected.iter().any(|text| text == "a-folder/"),
        "the cursor is on the folder: {selected:?}"
    );
}

#[test]
fn files_opens_a_file_in_the_window() {
    // Arrange
    let scratch = browsed("files-open");
    let running = start(
        named("files").expect("files is shipped"),
        &scratch.dir,
        system_path(),
    );
    let mut heard = Heard::default();
    heard.until(&running, |heard| heard.shows("b.txt"));

    // Act: `..`, the folder, then the file.
    running.send(Input::Key("n".to_string()));
    running.send(Input::Key("n".to_string()));
    running.send(Input::Key("Enter".to_string()));

    // Assert
    let opened = Effect::OpenFile {
        path: scratch.dir.join("b.txt"),
        line: None,
    };
    heard.until(&running, |heard| heard.effects.contains(&opened));
}

#[test]
fn files_copies_an_entrys_path_from_its_menu() {
    // Arrange
    let scratch = browsed("files-menu");
    let running = start(
        named("files").expect("files is shipped"),
        &scratch.dir,
        system_path(),
    );
    let mut heard = Heard::default();
    heard.until(&running, |heard| heard.shows("b.txt"));
    let row = heard.row_showing("b.txt");
    let labels: Vec<&str> = row.menu.iter().map(|item| item.label.as_str()).collect();
    let full = row
        .menu
        .iter()
        .find(|item| item.label == "copy full path")
        .unwrap_or_else(|| panic!("the menu offers the full path: {labels:?}"));

    // Act: what picking it sends.
    running.send(Input::Event(full.event.clone()));

    // Assert
    let copied = Effect::Copy(scratch.dir.join("b.txt").display().to_string());
    heard.until(&running, |heard| heard.effects.contains(&copied));
    let from_project = row
        .menu
        .iter()
        .find(|item| item.label == "copy path from the project")
        .expect("the menu offers the path from the project");
    assert_eq!(from_project.event, json!({ "copy": "b.txt" }));
}

#[test]
fn files_marks_what_git_has_not_seen() {
    // Arrange
    let scratch = browsed("files-git");
    let initialized = std::process::Command::new("git")
        .arg("init")
        .current_dir(&scratch.dir)
        .output()
        .expect("git runs");
    assert!(initialized.status.success());

    // Act
    let running = start(
        named("files").expect("files is shipped"),
        &scratch.dir,
        system_path(),
    );
    let mut heard = Heard::default();

    // Assert
    heard.until(&running, |heard| heard.shows("b.txt"));
    assert!(
        heard.shows("??"),
        "an untracked file is marked: {:?}",
        heard.texts()
    );
    assert!(
        heard.shows("*"),
        "and so is the folder holding one: {:?}",
        heard.texts()
    );
}

/// A docker that lists two containers of one compose project, and writes down anything else
/// it is asked.
const FAKE_DOCKER: &str = r#"#!/bin/sh
if [ "$1" = ps ]; then
  echo '{"id":"c1","name":"web-1","image":"nginx","state":"running","status":"Up 2 hours","ports":"0.0.0.0:80->80/tcp, [::]:80->80/tcp","project":"shop"}'
  echo '{"id":"c2","name":"db-1","image":"postgres","state":"exited","status":"Exited (0) 3 days ago","ports":"","project":"shop"}'
  exit 0
fi
echo "$@" >> "$(dirname "$0")/asked"
"#;

#[test]
fn docker_lists_the_containers_of_a_project_together() {
    // Arrange
    let scratch = Scratch::new("docker-list");
    scratch.program("docker", FAKE_DOCKER);

    // Act
    let running = start(
        named("docker").expect("docker is shipped"),
        &scratch.dir,
        scratch.path(),
    );
    let mut heard = Heard::default();

    // Assert
    heard.until(&running, |heard| heard.shows("web-1"));
    let texts = heard.texts();
    let db = texts.iter().position(|text| text == "db-1");
    let web = texts.iter().position(|text| text == "web-1");
    assert!(db < web, "in name order within the project: {texts:?}");
    assert!(heard.shows("80->80/tcp"), "each port once: {texts:?}");
}

#[test]
fn docker_restarts_the_selected_container_and_follows_its_logs_in_a_shell() {
    // Arrange
    let scratch = Scratch::new("docker-act");
    scratch.program("docker", FAKE_DOCKER);
    let running = start(
        named("docker").expect("docker is shipped"),
        &scratch.dir,
        scratch.path(),
    );
    let mut heard = Heard::default();
    heard.until(&running, |heard| heard.shows("web-1"));

    // Act: down onto web-1, restart it, and open its logs.
    running.send(Input::Key("n".to_string()));
    running.send(Input::Key("r".to_string()));
    running.send(Input::Key("l".to_string()));

    // Assert
    let logs = Effect::OpenShell("docker logs --follow --tail 500 'web-1'".to_string());
    heard.until(&running, |heard| heard.effects.contains(&logs));
    let asked = scratch.dir.join("bin/asked");
    let until = Instant::now() + PATIENCE;
    while std::fs::read_to_string(&asked).unwrap_or_default() != "restart c1\n" {
        assert!(
            Instant::now() < until,
            "docker was never asked to restart web-1"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn docker_filters_its_list_by_what_is_typed_and_keeps_the_cursor_on_its_container() {
    // Arrange
    let scratch = Scratch::new("docker-filter");
    scratch.program("docker", FAKE_DOCKER);
    let running = start(
        named("docker").expect("docker is shipped"),
        &scratch.dir,
        scratch.path(),
    );
    let mut heard = Heard::default();
    heard.until(&running, |heard| {
        heard.shows("web-1") && heard.shows("db-1")
    });
    // Onto web-1 - db-1 is listed first.
    running.send(Input::Key("n".to_string()));

    // Act: what the filter box sends as `nginx` is typed into it.
    running.send(Input::Event(json!({ "filter": true, "value": "NGINX" })));

    // Assert
    heard.until(&running, |heard| !heard.shows("db-1"));
    assert!(heard.shows("web-1"), "{:?}", heard.texts());
    assert!(heard.selected_row().iter().any(|text| text == "web-1"));

    // Act: every word has to match, each anywhere.
    running.send(Input::Event(
        json!({ "filter": true, "value": "shop exited" }),
    ));

    // Assert
    heard.until(&running, |heard| {
        heard.shows("db-1") && !heard.shows("web-1")
    });
}

#[test]
fn docker_says_what_the_daemon_said_when_it_cannot_be_reached() {
    // Arrange
    let scratch = Scratch::new("docker-down");
    scratch.program(
        "docker",
        "#!/bin/sh\necho 'Cannot connect to the Docker daemon' >&2\nexit 1\n",
    );

    // Act
    let running = start(
        named("docker").expect("docker is shipped"),
        &scratch.dir,
        scratch.path(),
    );
    let mut heard = Heard::default();

    // Assert
    heard.until(&running, |heard| {
        heard
            .texts()
            .iter()
            .any(|text| text.contains("Cannot connect to the Docker daemon"))
    });
}
