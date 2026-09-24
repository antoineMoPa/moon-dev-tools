//! Who the server has let in, as `Tools › Users` reads it, and kicking them out - see
//! `crate::server::users`.

use std::time::Duration;

use reqwest::{
    StatusCode,
    blocking::Client,
    header::{AUTHORIZATION, COOKIE},
};

use super::{Served, serve, serve_with};
use crate::{
    pass_keys::{PassKeys, of_this_test_run},
    server::users::{User, UserId, UserKind, UserList, Users},
};

/// The one route that answers the layer alone: a request that only makes its sender a user.
const CHECK: &str = "/api/pass-key";

fn bare_client() -> Client {
    Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .expect("failed to build the test client")
}

/// Ask once as the holder of `key`, so the server has seen it.
fn knock_with_key(served: &Served, key: &str) -> StatusCode {
    bare_client()
        .get(format!("{}{CHECK}", served.base_url))
        .header(AUTHORIZATION, format!("Bearer {key}"))
        .send()
        .expect("failed to ask")
        .status()
}

fn users_of(served: &Served) -> Vec<User> {
    served
        .client
        .get(format!("{}/api/users", served.base_url))
        .send()
        .expect("failed to ask for the users")
        .error_for_status()
        .expect("the server refused the users")
        .json::<UserList>()
        .expect("failed to decode the users")
        .users
}

fn kick(served: &Served, id: &str) -> reqwest::blocking::Response {
    served
        .client
        .post(format!("{}/api/users/{id}/kick", served.base_url))
        .send()
        .expect("failed to ask for the kick")
}

#[test]
fn the_list_shows_each_key_and_browser_login_with_its_address_and_marks_the_asker() {
    let served = serve("users-list");
    let other_key = of_this_test_run().generate();
    assert_eq!(knock_with_key(&served, &other_key), StatusCode::NO_CONTENT);
    let session = of_this_test_run().browser_session();
    let browsed = bare_client()
        .get(format!("{}{CHECK}", served.base_url))
        .header(COOKIE, format!("moon_pass_key={session}"))
        .send()
        .expect("failed to ask")
        .status();
    assert_eq!(browsed, StatusCode::NO_CONTENT);

    let users = users_of(&served);

    let you: Vec<&User> = users.iter().filter(|user| user.you).collect();
    assert_eq!(you.len(), 1, "one asker: {users:?}");
    assert_eq!(you[0].kind, UserKind::PassKey);
    let kinds: Vec<UserKind> = users.iter().map(|user| user.kind).collect();
    assert_eq!(
        kinds
            .iter()
            .filter(|kind| **kind == UserKind::PassKey)
            .count(),
        2,
        "the asker and the other key: {users:?}"
    );
    assert_eq!(
        kinds
            .iter()
            .filter(|kind| **kind == UserKind::Browser)
            .count(),
        1,
        "the browser: {users:?}"
    );
    assert!(
        users
            .iter()
            .all(|user| user.ip.is_loopback() && !user.kicked),
        "everyone came from this machine and nobody is kicked: {users:?}"
    );
    // An id names a key without being one: nothing listed lets anyone in.
    for user in &users {
        let (UserId::PassKey(id) | UserId::BrowserSession(id)) = &user.id;
        assert_eq!(knock_with_key(&served, id), StatusCode::UNAUTHORIZED);
    }
}

#[test]
fn a_kicked_key_is_refused_from_then_on_and_shows_as_kicked() {
    let served = serve("users-kick");
    let other_key = of_this_test_run().generate();
    assert_eq!(knock_with_key(&served, &other_key), StatusCode::NO_CONTENT);
    let other = users_of(&served)
        .into_iter()
        .find(|user| !user.you)
        .expect("the other key was seen")
        .id;

    let kicked = kick(&served, &other.to_string());

    assert_eq!(kicked.status(), StatusCode::NO_CONTENT);
    let refused = bare_client()
        .get(format!("{}{CHECK}", served.base_url))
        .header(AUTHORIZATION, format!("Bearer {other_key}"))
        .send()
        .expect("failed to ask");
    assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);
    assert!(
        refused.text().expect("expected a body").contains("kicked"),
        "a refusal says the key was kicked"
    );
    let shown = users_of(&served)
        .into_iter()
        .find(|user| user.id == other)
        .expect("a kicked user is still listed");
    assert!(shown.kicked);
    assert_eq!(shown.requests, 2, "the refused knock counts as seen");
    // The asker's own key still works: a kick is one user's.
    assert_eq!(
        served
            .client
            .get(format!("{}{CHECK}", served.base_url))
            .send()
            .expect("failed to ask")
            .status(),
        StatusCode::NO_CONTENT
    );
}

#[test]
fn a_kick_names_a_user_the_server_has_seen() {
    let served = serve("users-kick-unknown");

    assert_eq!(kick(&served, "key:nobody").status(), StatusCode::NOT_FOUND);
    assert_eq!(kick(&served, "nobody").status(), StatusCode::BAD_REQUEST);
}

/// Kicking everyone is a new secret, on a secret of this test's own: the one the other tests
/// share would take their keys with it.
#[test]
fn kicking_everyone_is_a_new_secret_that_refuses_every_old_key_and_admits_new_ones() {
    let secret =
        std::env::temp_dir().join(format!("moonreview-users-kick-all-{}", std::process::id()));
    let _ = std::fs::remove_file(&secret);
    let _ = std::fs::remove_file(secret.with_file_name(format!(
        "{}.kicked",
        secret.file_name().expect("a file").to_string_lossy()
    )));
    let users = Users::kept_at(&secret).expect("expected a secret of this test's own");
    let served = serve_with("users-kick-all", users.clone());
    let other_key = users.keys().generate();
    assert_eq!(knock_with_key(&served, &other_key), StatusCode::NO_CONTENT);

    let kicked = served
        .client
        .post(format!("{}/api/users/kick-all", served.base_url))
        .send()
        .expect("failed to ask");

    assert_eq!(kicked.status(), StatusCode::NO_CONTENT);
    // The asker's key went with the rest.
    assert_eq!(
        served
            .client
            .get(format!("{}{CHECK}", served.base_url))
            .send()
            .expect("failed to ask")
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        knock_with_key(&served, &other_key),
        StatusCode::UNAUTHORIZED
    );
    // A key of the new secret - which is what is on disk now, for `moon generate-pass-key`.
    let renewed = PassKeys::kept_at(&secret)
        .expect("expected the renewed secret")
        .generate();
    assert_eq!(knock_with_key(&served, &renewed), StatusCode::NO_CONTENT);
    let _ = std::fs::remove_file(&secret);
}
