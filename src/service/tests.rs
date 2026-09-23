use std::{
    fs,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

use super::*;
use crate::{api::FileContentPayload, git::run_git_no_output};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// A throwaway repo with a session open on it, plus the directory next to that repo where
/// the fixtures that are deliberately *not* in it live - a stand-in for `~/.cargo` and for
/// everything else on disk that a read must not reach.
struct ServedRepo {
    enclosing: PathBuf,
    repo_path: PathBuf,
    state: AppState,
    session_id: String,
}

impl Drop for ServedRepo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.enclosing);
    }
}

fn served_repo(name: &str) -> ServedRepo {
    let enclosing = std::env::temp_dir().join(format!(
        "moonreview-outside-{}-{}-{name}",
        std::process::id(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    ));
    let repo_path = enclosing.join("repo");
    fs::create_dir_all(&repo_path).expect("failed to create the fixture repo");
    run_git_no_output(&repo_path, &["init"]).expect("failed to init the fixture repo");
    fs::write(repo_path.join("lib.rs"), "fn one() {}\n").expect("failed to write in the repo");

    let state = crate::server::build_state(Arc::new(Mutex::new(Instant::now())));
    let session_id = open_session(
        &state,
        OpenSessionRequest {
            repo_path: repo_path.display().to_string(),
            diff_target: None,
            active_commit: None,
        },
    )
    .expect("failed to open the fixture session")
    .session_id;

    ServedRepo {
        enclosing,
        repo_path,
        state,
        session_id,
    }
}

impl ServedRepo {
    /// Write a file that is not in the repo, and hand back the absolute path of it - the
    /// shape of path a language server hands back when a definition lands in a dependency.
    fn write_outside(&self, relative: &str, content: &str) -> String {
        let path = self.enclosing.join(relative);
        fs::create_dir_all(path.parent().expect("a fixture path has a parent"))
            .expect("failed to create the fixture directory");
        fs::write(&path, content).expect("failed to write outside the repo");
        path.display().to_string()
    }

    /// Say that a language server named this file as where a definition is, which is the
    /// only thing that ever puts a path outside the repo within reach.
    fn a_server_named(&self, file_path: &str) {
        crate::lsp::remember_files_named(
            &self.state,
            &self.session_id,
            &[crate::api::LspLocation {
                file_path: file_path.to_string(),
                line_number: 1,
                line_text: None,
            }],
        )
        .expect("failed to record what the server named");
    }

    fn read(&self, file_path: &str) -> Result<FileContentPayload> {
        session_file(&self.state, &self.session_id, file_path)
    }
}

/// The refusal that guards the machine hosting the repo. `--remote` serves this read over
/// HTTP, so a path nobody named is refused however it is written: an absolute path to a
/// credential file, a path next to the repo, and a walk out of the repo with `..`.
#[test]
fn a_path_no_language_server_named_is_refused_however_it_is_written() {
    let served = served_repo("refused");
    let secret = served.write_outside("secret.txt", "a private key\n");

    // Each of these is refused, and which refusal it gets says which route it took: a path
    // that names a real file is refused for being outside the repo, and one that resolves
    // to nothing on disk is asked of HEAD instead and refused for not being in the repo's
    // history either. Neither route reads anything.
    for (path, refusal) in [
        ("/etc/passwd", "file path is outside the repository"),
        ("../secret.txt", "file path is outside the repository"),
        (secret.as_str(), "file path is outside the repository"),
        (
            "../../../etc/passwd",
            "file is not available in the working tree or HEAD",
        ),
    ] {
        let refused = served.read(path);
        assert_eq!(
            refused
                .err()
                .map(|error| error.to_string())
                .unwrap_or_default(),
            refusal,
            "{path} was not named by any language server and must be refused"
        );
    }

    // The repo's own files are read exactly as they always were.
    let inside = served
        .read("lib.rs")
        .expect("expected the repo's file to read");
    assert_eq!(inside.content, "fn one() {}\n");
    assert!(!inside.outside_the_repo);
}

/// A file a language server named as where a definition is reads, and says it is outside
/// the repo so the pane showing it offers no save.
#[test]
fn a_file_a_language_server_named_reads_and_says_it_is_outside_the_repo() {
    let served = served_repo("allowed");
    let dependency = served.write_outside("registry/dep/src/lib.rs", "pub fn dep() {}\n");

    assert!(
        served.read(&dependency).is_err(),
        "nothing may be read out there before a server has named it"
    );
    served.a_server_named(&dependency);

    let payload = served
        .read(&dependency)
        .expect("expected the file the server named to read");
    assert_eq!(payload.content, "pub fn dep() {}\n");
    assert!(payload.outside_the_repo);
}

/// The list is of resolved files, not of the strings that were handed in: a `..` walk off
/// the named file and a symlink pointing away from it are both refused, while a symlink
/// onto the named file is the named file and reads.
#[test]
fn a_dotdot_or_a_symlink_that_leaves_the_named_file_is_refused() {
    let served = served_repo("resolved");
    let dependency = served.write_outside("registry/dep/src/lib.rs", "pub fn dep() {}\n");
    served.write_outside("registry/secret.txt", "a private key\n");
    served.a_server_named(&dependency);

    let walked = served
        .enclosing
        .join("registry/dep/src/../../secret.txt")
        .display()
        .to_string();
    assert!(
        served.read(&walked).is_err(),
        "a walk off the named file lands somewhere nobody named"
    );

    let pointed_away = served.enclosing.join("registry/dep/src/away.rs");
    std::os::unix::fs::symlink(served.enclosing.join("registry/secret.txt"), &pointed_away)
        .expect("failed to link the fixture");
    assert!(
        served.read(&pointed_away.display().to_string()).is_err(),
        "a link out of the named file is the file it points at, which nobody named"
    );

    let pointed_at_it = served.enclosing.join("registry/dep/src/same.rs");
    std::os::unix::fs::symlink(&dependency, &pointed_at_it).expect("failed to link the fixture");
    assert_eq!(
        served
            .read(&pointed_at_it.display().to_string())
            .expect("a link onto the named file is the named file")
            .content,
        "pub fn dep() {}\n"
    );
}

/// Reading a dependency to understand it is the whole point; editing one in place is not.
/// The write keeps its containment check whatever any server has named.
#[test]
fn writing_to_a_file_a_language_server_named_is_still_refused() {
    let served = served_repo("read-only");
    let dependency = served.write_outside("registry/dep/src/lib.rs", "pub fn dep() {}\n");
    served.a_server_named(&dependency);

    let refused = write_session_file(
        &served.state,
        &served.session_id,
        &dependency,
        "pub fn theirs() {}\n",
    );

    assert!(refused.is_err(), "a file outside the repo is read-only");
    assert_eq!(
        fs::read_to_string(&dependency).expect("failed to read the dependency back"),
        "pub fn dep() {}\n",
        "and nothing may have been written to it"
    );
    // The repo's own files are still written the way the file pane writes them.
    write_session_file(&served.state, &served.session_id, "lib.rs", "fn two() {}\n")
        .expect("expected the repo's own file to be written");
    assert_eq!(
        fs::read_to_string(served.repo_path.join("lib.rs")).expect("failed to read back"),
        "fn two() {}\n"
    );
}

#[test]
fn unchanged_file_path_only_selects_clean_local_files() {
    let repo_path = std::env::temp_dir().join(format!(
        "moonreview-service-test-{}-{}",
        std::process::id(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(repo_path.join("src")).expect("failed to create test directory");
    fs::write(repo_path.join("src/example.rs"), "fn main() {}\n")
        .expect("failed to write test file");

    let file_target = DiffTarget {
        base: None,
        pathspec: Some("src/example.rs".to_string()),
        comparison: None,
    };
    assert_eq!(
        unchanged_file_path(&repo_path, &file_target, None, false).as_deref(),
        Some("src/example.rs")
    );
    assert_eq!(
        unchanged_file_path(&repo_path, &file_target, None, true),
        None
    );
    assert_eq!(
        unchanged_file_path(&repo_path, &file_target, Some("abc123"), false),
        None
    );

    let directory_target = DiffTarget {
        pathspec: Some("src".to_string()),
        ..Default::default()
    };
    assert_eq!(
        unchanged_file_path(&repo_path, &directory_target, None, false),
        None
    );

    let diff_target = DiffTarget {
        base: Some("main".to_string()),
        pathspec: Some("src/example.rs".to_string()),
        comparison: None,
    };
    assert_eq!(
        unchanged_file_path(&repo_path, &diff_target, None, false),
        None
    );

    fs::remove_dir_all(repo_path).expect("failed to remove test directory");
}

/// A shell and a task board are as much use in a plain folder as in a repo, so a window
/// opens on one - and everything the review is made of comes back empty rather than as
/// the failure of a `git status` run where there is no git.
#[test]
fn a_folder_that_is_no_repo_opens_with_an_empty_review() {
    let enclosing = std::env::temp_dir().join(format!(
        "moonreview-no-repo-{}-{}",
        std::process::id(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    ));
    let folder = enclosing.join("notes");
    fs::create_dir_all(&folder).expect("failed to create the fixture folder");
    fs::write(folder.join("todo.md"), "- write it down\n").expect("failed to write a file");

    let state = crate::server::build_state(Arc::new(Mutex::new(Instant::now())));
    let session_id = open_session(
        &state,
        OpenSessionRequest {
            repo_path: folder.display().to_string(),
            diff_target: None,
            active_commit: None,
        },
    )
    .expect("a folder that is no repo should still open")
    .session_id;

    let payload = session_state(&state, &session_id).expect("the state should still read");
    assert_eq!(payload.repo_name, "notes");
    assert_eq!(payload.branch_name, None);
    assert!(payload.hunks.is_empty(), "no repo means nothing to review");
    assert!(payload.commits.is_empty());
    assert!(
        !payload.read_only,
        "the files of the folder are still there to edit"
    );

    let hub = session_submodules(&state, &session_id).expect("the hub should still read");
    assert_eq!(hub.root.changed_files, 0);
    assert!(hub.submodules.is_empty());

    // And a `git init` in that same folder is what turns the review on.
    run_git_no_output(&folder, &["init"]).expect("failed to init the folder");
    let payload = session_state(&state, &session_id).expect("the state should read again");
    assert!(
        payload.branch_name.is_some(),
        "a folder that has become a repo is one with a branch"
    );

    fs::remove_dir_all(&enclosing).expect("failed to remove test directory");
}
