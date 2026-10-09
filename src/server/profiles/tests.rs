//! Profile tests - admissions share verified identities, never another person's credentials.
use super::*;
fn fixture() -> Profiles {
    let dir = std::env::temp_dir().join(format!(
        "moon-profiles-{}",
        crate::moontasks::store::new_uuid()
    ));
    fs::create_dir_all(&dir).unwrap();
    Profiles::kept_at(&dir.join("secret")).unwrap()
}
fn account(id: &str) -> Account {
    Account {
        id: format!("github:{id}"),
        login: format!("person{id}"),
        name: format!("Person {id}"),
        email: format!("{id}@users.noreply.github.com"),
        token: format!("secret-{id}"),
        unix_user: None,
        layout: None,
        windows: HashMap::new(),
    }
}
fn user(id: &str) -> UserId {
    UserId::BrowserSession(id.into())
}
#[test]
fn persistent_identity_keeps_layout_across_browsers_and_login_renames() {
    let profiles = fixture();
    profiles.connect(&user("a"), account("1")).unwrap();
    profiles
        .change(|s| {
            s.accounts.get_mut("github:1").unwrap().layout = Some(Layout {
                repo: Some("shared".into()),
                frame: "tasks".into(),
                layout: Some("layout".into()),
            });
            Ok(())
        })
        .unwrap();
    let mut renamed = account("1");
    renamed.login = "renamed".into();
    profiles.connect(&user("b"), renamed).unwrap();
    let restarted = Profiles::kept_at(&profiles.path.parent().unwrap().join("secret")).unwrap();
    // The profile path is derived from the secret extension.
    assert_eq!(
        profiles.view(&user("b")).layout.unwrap().layout.as_deref(),
        Some("layout")
    );
    assert_eq!(restarted.view(&user("a")).login.as_deref(), Some("renamed"));
}
#[test]
fn separate_people_and_unconnected_callers_do_not_get_credentials_or_layout() {
    let profiles = fixture();
    profiles.connect(&user("a"), account("1")).unwrap();
    profiles.connect(&user("b"), account("2")).unwrap();
    assert!(profiles.git_environment(&user("unknown")).is_err());
    let json = serde_json::to_string(&profiles.view(&user("a"))).unwrap();
    assert!(!json.contains("secret-"));
    assert!(!json.contains("person2"));
    let env = profiles.git_environment(&user("b")).unwrap();
    assert!(env.iter().any(|(k, v)| k == "GH_TOKEN" && v == "secret-2"));
    assert!(!env.iter().any(|(_, v)| v == "secret-1"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&profiles.path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
#[test]
fn helper_only_answers_for_github_https() {
    let profiles = fixture();
    profiles.connect(&user("a"), account("1")).unwrap();
    let env = profiles.git_environment(&user("a")).unwrap();
    let helper = env
        .iter()
        .find(|(k, v)| k.starts_with("GIT_CONFIG_VALUE") && v.starts_with("!f()"))
        .unwrap()
        .1
        .trim_start_matches('!');
    for (protocol, host, allowed) in [
        ("https", "github.com", true),
        ("https", "evil.example", false),
        ("http", "github.com", false),
        ("https", "github.com:443", false),
    ] {
        use std::process::{Command, Stdio};
        let mut child = Command::new("sh")
            .args(["-c", &format!("{helper} get")])
            .env("GH_TOKEN", "test-token")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        write!(
            child.stdin.take().unwrap(),
            "protocol={protocol}\nhost={host}\n\n"
        )
        .unwrap();
        let output = child.wait_with_output().unwrap();
        assert_eq!(
            String::from_utf8(output.stdout)
                .unwrap()
                .contains("test-token"),
            allowed
        );
    }
}
#[tokio::test]
async fn layout_writes_replace_one_snapshot_and_refuse_stale_identity() {
    use axum::{Extension, Json, extract::State};
    let profiles = fixture();
    profiles.connect(&user("a"), account("1")).unwrap();
    for contents in ["first", "latest"] {
        let request =
            serde_json::from_value(serde_json::json!({"profile_id":"github:1", "layout":{
                "repo":"shared", "frame":"tasks", "layout":contents
            }}))
            .unwrap();
        routes::save_layout(State(profiles.clone()), Extension(user("a")), Json(request))
            .await
            .unwrap();
    }
    profiles.connect(&user("b"), account("1")).unwrap();
    assert_eq!(
        profiles.view(&user("b")).layout.unwrap().layout.as_deref(),
        Some("latest")
    );
    profiles.connect(&user("a"), account("2")).unwrap();
    let request = serde_json::from_value(serde_json::json!({"profile_id":"github:1", "layout":{
        "repo":null,"frame":"tasks","layout":"stale"
    }}))
    .unwrap();
    assert_eq!(
        routes::save_layout(State(profiles.clone()), Extension(user("a")), Json(request))
            .await
            .unwrap_err()
            .0,
        axum::http::StatusCode::CONFLICT
    );
    assert!(profiles.view(&user("a")).layout.is_none());
    assert_eq!(
        profiles.view(&user("b")).layout.unwrap().layout.as_deref(),
        Some("latest")
    );
}
#[test]
fn legacy_windows_migrate_deterministically_and_are_never_written_back() {
    let profiles = fixture();
    let secret = profiles.path.parent().unwrap().join("secret");
    let mut old = serde_json::to_value(account("1")).unwrap();
    old.as_object_mut().unwrap().remove("layout");
    old["windows"] = serde_json::json!({
        "000-empty":{"title":"My window","repo":"empty","frame":"tasks","layout":null},
        "z-work":{"title":"My window","repo":"work","frame":"tasks","layout":"z"},
        "a-work":{"title":"My window","repo":"work","frame":"tasks","layout":"a"},
        "default":{"title":"My window","repo":"default","frame":"tasks","layout":"default"}
    });
    fs::write(
        &profiles.path,
        serde_json::to_vec(&serde_json::json!({
            "accounts":{"github:1":old}, "admissions":{"browser:a":"github:1"}
        }))
        .unwrap(),
    )
    .unwrap();
    let migrated = Profiles::kept_at(&secret).unwrap();
    assert_eq!(
        migrated.view(&user("a")).layout.unwrap().layout.as_deref(),
        Some("default")
    );
    migrated.connect(&user("b"), account("1")).unwrap();
    let stored: serde_json::Value =
        serde_json::from_slice(&fs::read(&profiles.path).unwrap()).unwrap();
    assert!(stored["accounts"]["github:1"].get("windows").is_none());
    assert_eq!(
        Profiles::kept_at(&secret)
            .unwrap()
            .view(&user("b"))
            .layout
            .unwrap()
            .repo
            .as_deref(),
        Some("default")
    );
    old["windows"].as_object_mut().unwrap().remove("default");
    fs::write(
        &profiles.path,
        serde_json::to_vec(&serde_json::json!({
            "accounts":{"github:1":old}, "admissions":{"browser:a":"github:1"}
        }))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        Profiles::kept_at(&secret)
            .unwrap()
            .view(&user("a"))
            .layout
            .unwrap()
            .layout
            .as_deref(),
        Some("a")
    );
}
#[test]
fn forgetting_admissions_keeps_profiles_but_requires_verified_reconnection() {
    let profiles = fixture();
    profiles.connect(&user("a"), account("1")).unwrap();
    profiles.connect(&user("b"), account("1")).unwrap();
    profiles.forget_admission(&user("a")).unwrap();
    assert!(profiles.git_environment(&user("a")).is_err());
    assert!(profiles.git_environment(&user("b")).is_ok());
    profiles.forget_all_admissions().unwrap();
    assert!(profiles.git_environment(&user("b")).is_err());
    assert_eq!(profiles.inner.lock().unwrap().accounts.len(), 1);
}
#[test]
fn git_commit_uses_personal_identity_without_changing_shared_repo_config() {
    let profiles = fixture();
    profiles.connect(&user("a"), account("1")).unwrap();
    let repo = profiles.path.parent().unwrap().join("repo");
    fs::create_dir(&repo).unwrap();
    fn git(repo: &Path, args: &[&str]) -> String {
        let output = std::process::Command::new("git")
            .current_dir(repo)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }
    git(&repo, &["init", "-q"]);
    git(&repo, &["config", "user.name", "Host owner"]);
    git(&repo, &["config", "user.email", "host@example.com"]);
    fs::write(repo.join("file"), "shared contents").unwrap();
    git(&repo, &["add", "file"]);
    let output = std::process::Command::new("git")
        .current_dir(&repo)
        .args(["commit", "-m", "personal commit"])
        .envs(profiles.git_environment(&user("a")).unwrap())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        git(&repo, &["log", "-1", "--format=%an <%ae>|%cn <%ce>"]).trim(),
        "Person 1 <1@users.noreply.github.com>|Person 1 <1@users.noreply.github.com>"
    );
    assert_eq!(git(&repo, &["config", "user.name"]).trim(), "Host owner");
    assert_eq!(
        git(&repo, &["config", "user.email"]).trim(),
        "host@example.com"
    );
}
#[test]
fn a_completed_sign_in_cannot_reconnect_after_disconnect_or_kick() {
    let profiles = fixture();
    let admission = user("a");
    let old = profiles.begin_auth(&admission);
    profiles.forget_admission(&admission).unwrap();
    assert!(
        profiles
            .connect_if_current(&admission, account("1"), old)
            .is_err()
    );
    let old = profiles.begin_auth(&admission);
    profiles.forget_all_admissions().unwrap();
    assert!(
        profiles
            .connect_if_current(&admission, account("1"), old)
            .is_err()
    );
    let old = profiles.begin_auth(&admission);
    let current = profiles.begin_auth(&admission);
    assert!(
        profiles
            .connect_if_current(&admission, account("1"), old)
            .is_err()
    );
    profiles
        .connect_if_current(&admission, account("2"), current)
        .unwrap();
    assert_eq!(profiles.view(&admission).id.as_deref(), Some("github:2"));
}
#[test]
fn git_credential_fill_resets_host_helpers_and_only_returns_personal_github_token() {
    use std::process::{Command, Stdio};
    let profiles = fixture();
    profiles.connect(&user("a"), account("1")).unwrap();
    let repo = profiles.path.parent().unwrap();
    assert!(
        Command::new("git")
            .current_dir(repo)
            .args(["init", "-q"])
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new("git")
            .current_dir(repo)
            .args([
                "config",
                "credential.helper",
                "!f() { echo username=host; echo password=host-secret; }; f"
            ])
            .status()
            .unwrap()
            .success()
    );
    for (host, allowed) in [("github.com", true), ("evil.example", false)] {
        let mut child = Command::new("git")
            .current_dir(repo)
            .args(["credential", "fill"])
            .envs(profiles.git_environment(&user("a")).unwrap())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        write!(
            child.stdin.take().unwrap(),
            "protocol=https\nhost={host}\n\n"
        )
        .unwrap();
        let output = child.wait_with_output().unwrap();
        let text = String::from_utf8(output.stdout).unwrap();
        assert_eq!(output.status.success(), allowed, "{text}");
        assert_eq!(text.contains("secret-1"), allowed);
        assert!(!text.contains("host-secret"));
    }
    assert!(
        Command::new("git")
            .current_dir(repo)
            .args(["remote", "add", "origin", "git@github.com:owner/repo.git"])
            .status()
            .unwrap()
            .success()
    );
    let output = Command::new("git")
        .current_dir(repo)
        .args(["remote", "get-url", "origin"])
        .envs(profiles.git_environment(&user("a")).unwrap())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        "https://github.com/owner/repo.git"
    );
}

#[test]
fn personal_shell_commits_keep_identity_isolated_without_credentials() {
    use std::{
        process::Command,
        thread,
        time::{Duration, Instant},
    };
    let profiles = fixture();
    profiles.connect(&user("a"), account("1")).unwrap();
    profiles.connect(&user("b"), account("2")).unwrap();
    let root = profiles.path.parent().unwrap().join("repo");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .args(args)
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    };
    git(&["init"]);
    git(&["config", "user.name", "Host"]);
    git(&["config", "user.email", "host@example.com"]);
    git(&[
        "-c",
        "commit.gpgSign=false",
        "commit",
        "--allow-empty",
        "-m",
        "host",
    ]);
    git(&["config", "commit.gpgSign", "true"]);
    let config_before = fs::read(root.join(".git/config")).unwrap();
    let state =
        crate::server::build_state(std::sync::Arc::new(std::sync::Mutex::new(Instant::now())));
    let registry = state.terminals.clone();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let spawn = |id: &str| {
        let env = profiles.shell_git_environment(&user(id));
        assert!(
            !env.iter()
                .any(|(key, value)| key.contains("TOKEN") || value.contains("secret-"))
        );
        use axum::{
            Extension, Json,
            extract::{Path as AxumPath, State},
            response::IntoResponse,
        };
        let session_id = crate::service::open_session_for_profile(
            &state,
            crate::api::OpenSessionRequest {
                repo_path: root.display().to_string(),
                diff_target: None,
                active_commit: None,
            },
            Some(profiles.namespace(&user(id)).as_str().into()),
        )
        .unwrap()
        .session_id;
        let terminal = runtime.block_on(async {
            let response = if id == "b" {
                crate::terminal::run_in_shell(
                    AxumPath(session_id),
                    State(state.clone()),
                    State(profiles.clone()),
                    Extension(user(id)),
                    Json(serde_json::from_value(serde_json::json!({"command":"true"})).unwrap()),
                )
                .await
                .unwrap()
                .into_response()
            } else {
                crate::terminal::create_terminal(
                    AxumPath(session_id),
                    State(state.clone()),
                    State(profiles.clone()),
                    Extension(user(id)),
                    Json(
                        serde_json::from_value(
                            serde_json::json!({"folder":root.display().to_string()}),
                        )
                        .unwrap(),
                    ),
                )
                .await
                .unwrap()
                .into_response()
            };
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            serde_json::from_slice::<serde_json::Value>(&body).unwrap()["terminal_id"]
                .as_str()
                .unwrap()
                .to_string()
        });
        (terminal.clone(), registry.attach(&terminal).unwrap().1)
    };
    // The admitted account cannot launch its identity in another profile's workspace.
    runtime.block_on(async {
        use axum::{
            Extension, Json,
            extract::{Path as AxumPath, State},
        };
        let other_session = crate::service::open_session_for_profile(
            &state,
            crate::api::OpenSessionRequest {
                repo_path: root.display().to_string(),
                diff_target: None,
                active_commit: None,
            },
            Some(profiles.namespace(&user("b")).as_str().into()),
        )
        .unwrap()
        .session_id;
        assert!(
            crate::terminal::create_terminal(
                AxumPath(other_session),
                State(state.clone()),
                State(profiles.clone()),
                Extension(user("a")),
                Json(serde_json::from_value(serde_json::json!({})).unwrap()),
            )
            .await
            .is_err()
        );
    });
    let (alice_id, alice) = spawn("a");
    let (bob_id, bob) = spawn("b");
    let commit = |shell: &std::sync::Arc<crate::terminal::TerminalSession>, marker: &str| {
        shell
            .write_input(
                format!(
                    "git commit --allow-empty -m {marker}; printf '%s\\n' $? > {marker}.status\r"
                )
                .as_bytes(),
            )
            .unwrap();
        let status = root.join(format!("{marker}.status"));
        let deadline = Instant::now() + Duration::from_secs(10);
        while !status.exists() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        fs::read_to_string(status).unwrap()
    };
    assert_eq!(commit(&alice, "alice"), "0\n");
    assert_eq!(
        git(&["log", "-1", "--format=%an|%ae|%cn|%ce"]).trim(),
        "Person 1|1@users.noreply.github.com|Person 1|1@users.noreply.github.com"
    );
    assert_eq!(commit(&bob, "bob"), "0\n");
    assert_eq!(
        git(&["log", "-1", "--format=%an|%ae|%cn|%ce"]).trim(),
        "Person 2|2@users.noreply.github.com|Person 2|2@users.noreply.github.com"
    );
    profiles.forget_admission(&user("a")).unwrap();
    // An existing process retains its launch environment; newly opened shells use the
    // current admission, and unsigned-in users cannot fall back to the host's identity.
    assert_eq!(commit(&alice, "existing"), "0\n");
    let (anonymous_id, anonymous) = spawn("a");
    let head = git(&["rev-parse", "HEAD"]);
    assert_ne!(commit(&anonymous, "anonymous"), "0\n");
    assert_eq!(git(&["rev-parse", "HEAD"]), head);
    assert_eq!(fs::read(root.join(".git/config")).unwrap(), config_before);
    for id in [alice_id, bob_id, anonymous_id] {
        registry.remove(&id);
    }
    fs::remove_dir_all(profiles.path.parent().unwrap()).unwrap();
}

/// The guard on the switch: a server that is not root gives nobody a Unix user, whoever signs
/// in, and what a signed-in person starts runs as the server's own user - see
/// `crate::unix_users`.
#[test]
fn without_the_switch_no_unix_user_is_made_and_a_shell_runs_as_the_server() {
    use std::{
        thread,
        time::{Duration, Instant},
    };
    assert!(!crate::unix_users::each_person_has_one());
    let profiles = fixture();
    profiles.connect(&user("a"), account("1")).unwrap();
    assert_eq!(profiles.view(&user("a")).unix_user, None);
    assert!(!profiles.view(&user("a")).sign_in_required);
    let owner = profiles.session_owner(&user("a")).unwrap();
    assert!(owner.person.is_none());

    let folder = profiles.path.parent().unwrap().to_path_buf();
    let state =
        crate::server::build_state(std::sync::Arc::new(std::sync::Mutex::new(Instant::now())));
    let session_id = crate::service::open_session_for_profile(
        &state,
        crate::api::OpenSessionRequest {
            repo_path: folder.display().to_string(),
            diff_target: None,
            active_commit: None,
        },
        Some(owner),
    )
    .unwrap()
    .session_id;
    assert_eq!(crate::api::person_of(&state, &session_id).unwrap(), None);
    let uid_file = folder.join("uid");
    let terminal_id = crate::terminal::start_workspace_shell_running(
        &state,
        &session_id,
        &format!("id -u > {}.part && mv {0}.part {0}", uid_file.display()),
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while !uid_file.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    // SAFETY: reads this process's own credentials.
    let server_uid = unsafe { libc::getuid() };
    assert_eq!(
        fs::read_to_string(&uid_file).unwrap().trim(),
        server_uid.to_string()
    );
    state.terminals.remove(&terminal_id);
    fs::remove_dir_all(folder).unwrap();
}
