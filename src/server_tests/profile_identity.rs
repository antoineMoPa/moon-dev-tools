//! A page pins its starting identity so another tab's account change cannot retarget it.

use reqwest::{StatusCode, blocking::Client, header::COOKIE};

use super::serve;
use crate::pass_keys::of_this_test_run;

#[test]
fn a_browser_can_pin_its_admission_before_connecting_github() {
    let served = serve("profile-pin-anonymous");
    let keys = of_this_test_run();
    let session = keys.browser_session();
    let id = keys.admitted_browser_session_id(&session).unwrap();
    let client = Client::new();
    let response = client
        .get(format!("{}/api/me", served.base_url))
        .header(COOKIE, format!("moon_pass_key={session}"))
        .header("x-moon-profile", format!("browser:{id}"))
        .send()
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let profile: serde_json::Value = response.json().unwrap();
    assert_eq!(profile["profile_namespace"], format!("browser:{id}"));
    assert!(profile["id"].is_null());
    let response = client
        .head(format!("{}/api/me", served.base_url))
        .header(COOKIE, format!("moon_pass_key={session}"))
        .header("x-moon-profile", format!("browser:{id}"))
        .send()
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.bytes().unwrap().is_empty());
}

#[test]
fn a_stale_profile_cannot_mutate_or_open_a_new_socket() {
    let served = serve("profile-pin-stale");
    // Even an otherwise admitted bearer cannot execute the route after naming an old
    // profile. Use mint-pass-key so success would be a real mutation, not just a GET.
    let response = served
        .client
        .post(format!("{}/api/pass-key", served.base_url))
        .header("x-moon-profile", "github:other-person")
        .send()
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert!(response.text().unwrap().contains("Account changed"));
    let response = served
        .client
        .head(format!("{}/api/me", served.base_url))
        .header("x-moon-profile", "github:other-person")
        .send()
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert!(response.bytes().unwrap().is_empty());

    let response = served
        .client
        .get(format!(
            "{}/api/pass-key?moon_profile=github%3Aother-person",
            served.base_url
        ))
        .send()
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
}

#[test]
fn conflicting_profile_expectations_are_refused() {
    let served = serve("profile-pin-conflict");
    let response = served
        .client
        .get(format!("{}/api/me?moon_profile=one", served.base_url))
        .header("x-moon-profile", "two")
        .send()
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}
