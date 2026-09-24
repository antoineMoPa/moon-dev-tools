//! Who the server lets in: a request showing one of this machine's pass keys - as a bearer
//! token or as the browser's cookie - from no other site's page. See `crate::server::auth`.

use std::{io::Read, time::Duration};

use reqwest::{
    StatusCode,
    blocking::Client,
    header::{
        ACCEPT_ENCODING, AUTHORIZATION, CONTENT_ENCODING, COOKIE, LOCATION, ORIGIN, SET_COOKIE,
    },
    redirect::Policy,
};

use super::serve;
use crate::pass_keys::of_this_test_run;

/// A client with no key, which follows no redirects so a login's answer can be read as it is.
fn bare_client() -> Client {
    Client::builder()
        .timeout(Duration::from_secs(20))
        .redirect(Policy::none())
        .build()
        .expect("failed to build the test client")
}

/// The one route that answers the layer alone, so what comes back is the layer's word.
const CHECK: &str = "/api/pass-key";

#[test]
fn the_api_refuses_a_request_without_a_key_and_admits_one_with_a_bearer_or_a_cookie() {
    let served = serve("auth-api");
    let client = bare_client();
    let key = of_this_test_run().generate();
    let url = format!("{}{CHECK}", served.base_url);

    let without = client.get(&url).send().expect("failed to ask");
    assert_eq!(without.status(), StatusCode::UNAUTHORIZED);
    assert!(
        without
            .text()
            .expect("expected a body")
            .contains("generate-pass-key"),
        "a refusal should say how to get a key"
    );

    let bearer = client
        .get(&url)
        .header(AUTHORIZATION, format!("Bearer {key}"))
        .send()
        .expect("failed to ask");
    assert_eq!(bearer.status(), StatusCode::NO_CONTENT);

    let session = of_this_test_run().browser_session();
    let cookie = client
        .get(&url)
        .header(COOKIE, format!("theme=dark; moon_pass_key={session}"))
        .send()
        .expect("failed to ask");
    assert_eq!(cookie.status(), StatusCode::NO_CONTENT);

    // A cookie holds a browser session, which runs out: a pass key put in one is refused, so
    // no browser holds a login that lasts past a week.
    let key_in_cookie = client
        .get(&url)
        .header(COOKIE, format!("moon_pass_key={key}"))
        .send()
        .expect("failed to ask");
    assert_eq!(key_in_cookie.status(), StatusCode::UNAUTHORIZED);

    // And a session is no bearer token: `Authorization` takes pass keys alone.
    let session_as_bearer = client
        .get(&url)
        .header(AUTHORIZATION, format!("Bearer {session}"))
        .send()
        .expect("failed to ask");
    assert_eq!(session_as_bearer.status(), StatusCode::UNAUTHORIZED);

    // The public routes need nothing.
    let health = client
        .get(format!("{}/healthz", served.base_url))
        .send()
        .expect("failed to ask");
    assert_eq!(health.status(), StatusCode::OK);
}

/// A route with data behind it, not only the check: every route is behind the one layer.
#[test]
fn a_session_route_is_refused_without_a_key() {
    let served = serve("auth-session");
    let session_id = served.open_session();

    let refused = bare_client()
        .get(format!(
            "{}/api/session/{session_id}/state",
            served.base_url
        ))
        .send()
        .expect("failed to ask");

    assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);
}

#[test]
fn a_key_of_another_secret_or_a_malformed_authorization_is_refused() {
    let served = serve("auth-wrong");
    let client = bare_client();
    let url = format!("{}{CHECK}", served.base_url);
    let other_secret = std::env::temp_dir().join(format!(
        "moonreview-server-other-secret-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&other_secret);
    let foreign = crate::pass_keys::PassKeys::kept_at(&other_secret)
        .expect("expected a second secret")
        .generate();

    for authorization in [
        format!("Bearer {foreign}"),
        format!("Basic {}", of_this_test_run().generate()),
    ] {
        let refused = client
            .get(&url)
            .header(AUTHORIZATION, authorization.clone())
            .send()
            .expect("failed to ask");
        assert_eq!(
            refused.status(),
            StatusCode::UNAUTHORIZED,
            "{authorization}"
        );
    }
    let _ = std::fs::remove_file(&other_secret);
}

/// Another site's page sends the browser's cookie along - a page on another port of
/// localhost is the same site - so a cookie from one is refused, key or not.
#[test]
fn a_request_from_another_site_s_page_is_refused_whatever_it_carries() {
    let served = serve("auth-origin");
    let client = bare_client();
    let key = of_this_test_run().generate();
    let url = format!("{}{CHECK}", served.base_url);

    let other_origin = client
        .get(&url)
        .header(AUTHORIZATION, format!("Bearer {key}"))
        .header(ORIGIN, "http://localhost:3000")
        .send()
        .expect("failed to ask");
    assert_eq!(other_origin.status(), StatusCode::FORBIDDEN);

    let other_site = client
        .get(&url)
        .header(COOKIE, format!("moon_pass_key={}", of_this_test_run().browser_session()))
        .header("sec-fetch-site", "same-site")
        .send()
        .expect("failed to ask");
    assert_eq!(other_site.status(), StatusCode::FORBIDDEN);

    // The page's own requests name this server as their origin.
    let own_origin = client
        .get(&url)
        .header(COOKIE, format!("moon_pass_key={}", of_this_test_run().browser_session()))
        .header(ORIGIN, served.base_url.clone())
        .header("sec-fetch-site", "same-origin")
        .send()
        .expect("failed to ask");
    assert_eq!(own_origin.status(), StatusCode::NO_CONTENT);
}

/// A shell's websocket is behind the same layer: no key is no shell, and a page elsewhere
/// cannot open one with a key it did not make the request with.
#[test]
fn a_shell_socket_is_refused_without_a_key_or_from_another_origin() {
    use tungstenite::{client::IntoClientRequest, http::HeaderValue};

    let served = serve("auth-socket");
    let session_id = served.open_session();
    let socket_url = format!(
        "{}/api/session/{session_id}/terminals/no-such-shell/socket",
        served.base_url.replacen("http://", "ws://", 1)
    );

    let refused_status = |request| match tungstenite::connect(request) {
        Err(tungstenite::Error::Http(response)) => response.status(),
        Err(error) => panic!("expected an HTTP refusal, got {error}"),
        Ok(_) => panic!("expected the upgrade to be refused"),
    };

    let without = socket_url
        .as_str()
        .into_client_request()
        .expect("expected a request");
    assert_eq!(refused_status(without), StatusCode::UNAUTHORIZED);

    let mut other_origin = socket_url
        .as_str()
        .into_client_request()
        .expect("expected a request");
    let headers = other_origin.headers_mut();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {}", of_this_test_run().generate()))
            .expect("a pass key is a header value"),
    );
    headers.insert(ORIGIN, HeaderValue::from_static("http://evil.example"));
    assert_eq!(refused_status(other_origin), StatusCode::FORBIDDEN);
}

/// A browser that has not logged in is shown the login page at `/moon/`, not the window, and
/// one that has is shown the window.
#[test]
fn the_page_is_the_login_page_until_the_browser_has_a_key() {
    let served = serve("auth-page");
    let client = bare_client();
    let page = format!("{}/moon/?repo=%2Fsomewhere&frame=shell", served.base_url);

    let login = client.get(&page).send().expect("failed to ask");
    assert_eq!(login.status(), StatusCode::UNAUTHORIZED);
    let login = login.text().expect("expected a page");
    assert!(login.contains("name=\"pass_key\""), "got {login}");
    assert!(
        login.contains("action=\"/moon/login?repo=%2Fsomewhere&amp;frame=shell\""),
        "the login should post back to the same repo and frame: {login}"
    );

    let logged_in = format!("moon_pass_key={}", of_this_test_run().browser_session());
    let window = client
        .get(&page)
        .header(COOKIE, &logged_in)
        .header(ACCEPT_ENCODING, "gzip")
        .send()
        .expect("failed to ask");
    assert_eq!(window.status(), StatusCode::OK);
    assert_eq!(
        window
            .headers()
            .get(CONTENT_ENCODING)
            .map(|coding| coding.as_bytes()),
        Some(&b"gzip"[..]),
        "the page is embedded gzipped and sent that way"
    );
    let mut unpacked = String::new();
    flate2::read::GzDecoder::new(window.bytes().expect("expected a page").as_ref())
        .read_to_string(&mut unpacked)
        .expect("the page should be gzip");
    assert!(
        unpacked.contains("moonreview.js"),
        "a logged-in browser should be given the window"
    );

    // A client that does not say it reads gzip is told so, not handed bytes it cannot read.
    let unasked = client
        .get(&page)
        .header(COOKIE, &logged_in)
        .send()
        .expect("failed to ask");
    assert_eq!(unasked.status(), StatusCode::NOT_ACCEPTABLE);
}

/// A good key is kept in an `HttpOnly`, `SameSite=Strict` cookie and the browser sent back
/// to where it was; a bad one is the login page again, saying so.
#[test]
fn logging_in_with_a_good_key_sets_the_cookie_and_a_bad_one_is_refused() {
    let served = serve("auth-login");
    let client = bare_client();
    let login = format!(
        "{}/moon/login?repo=%2Fsomewhere&frame=shell",
        served.base_url
    );
    let key = of_this_test_run().generate();

    let admitted = client
        .post(&login)
        .form(&[("pass_key", key.as_str())])
        .send()
        .expect("failed to log in");
    assert_eq!(admitted.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        admitted.headers()[LOCATION],
        "/moon/?repo=%2Fsomewhere&frame=shell"
    );
    let cookie = admitted.headers()[SET_COOKIE]
        .to_str()
        .expect("expected a readable cookie");
    let session = cookie
        .strip_prefix("moon_pass_key=")
        .and_then(|rest| rest.split(';').next())
        .expect("expected the session cookie");
    assert_ne!(session, key, "the cookie must not hold the pass key");
    assert!(
        of_this_test_run().admits_browser_session(session),
        "the cookie should hold a browser session: {cookie}"
    );
    assert!(
        cookie.contains(&format!(
            "Max-Age={}",
            crate::pass_keys::BROWSER_SESSION_LIFETIME.as_secs()
        )),
        "a browser's login lasts a week: {cookie}"
    );
    for attribute in ["HttpOnly", "SameSite=Strict", "Path=/"] {
        assert!(
            cookie.contains(attribute),
            "{attribute} missing from {cookie}"
        );
    }

    let refused = client
        .post(&login)
        .form(&[("pass_key", "not.a-key")])
        .send()
        .expect("failed to log in");
    assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);
    assert!(refused.headers().get(SET_COOKIE).is_none());
    assert!(
        refused
            .text()
            .expect("expected a page")
            .contains("that pass key is not valid")
    );
}

#[test]
fn public_pages_do_not_disclose_the_home_repository() {
    let served = serve("auth-private-home");
    served.open_session();
    let client = bare_client();

    let redirect = client
        .get(format!("{}/moon?frame=shell", served.base_url))
        .send()
        .expect("expected redirect");
    assert_eq!(redirect.status(), StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(redirect.headers()[LOCATION], "/moon/?frame=shell");

    let page = client
        .get(format!("{}/moon/", served.base_url))
        .send()
        .expect("expected login page");
    assert_eq!(page.status(), StatusCode::UNAUTHORIZED);
    assert!(page.headers().get(LOCATION).is_none());
    assert!(!page.text().expect("expected body").contains("?repo="));

    let admitted = client
        .get(format!("{}/moon/", served.base_url))
        .header(
            COOKIE,
            format!("moon_pass_key={}", of_this_test_run().browser_session()),
        )
        .send()
        .expect("expected repo redirect");
    assert_eq!(admitted.status(), StatusCode::TEMPORARY_REDIRECT);
    assert!(
        admitted.headers()[LOCATION]
            .to_str()
            .unwrap()
            .starts_with("/moon/?repo=")
    );
}

#[test]
fn login_rejects_foreign_and_opaque_origins_without_setting_a_cookie() {
    let served = serve("auth-login-origin");
    let client = bare_client();
    let key = of_this_test_run().generate();
    let url = format!("{}/moon/login", served.base_url);
    for origin in ["http://evil.example", "null", "http://localhost:3000"] {
        let response = client
            .post(&url)
            .header(ORIGIN, origin)
            .form(&[("pass_key", &key)])
            .send()
            .expect("expected response");
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{origin}");
        assert!(response.headers().get(SET_COOKIE).is_none());
    }
    let response = client
        .post(&url)
        .header("sec-fetch-site", "same-site")
        .form(&[("pass_key", &key)])
        .send()
        .expect("expected response");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(response.headers().get(SET_COOKIE).is_none());

    let response = client
        .post(&url)
        .header(ORIGIN, &served.base_url)
        .header("sec-fetch-site", "same-origin")
        .form(&[("pass_key", &key)])
        .send()
        .expect("expected response");
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert!(response.headers().get(SET_COOKIE).is_some());
}

#[test]
fn responses_prevent_caching_framing_and_referrer_leaks() {
    let served = serve("auth-response-headers");
    let client = bare_client();
    for path in ["/moon", "/moon/", "/healthz", CHECK, "/missing"] {
        let response = client
            .get(format!("{}{path}", served.base_url))
            .send()
            .expect("expected response");
        let headers = response.headers();
        for (name, value) in [
            ("cache-control", "no-store"),
            ("x-frame-options", "DENY"),
            ("x-content-type-options", "nosniff"),
            ("referrer-policy", "same-origin"),
        ] {
            assert_eq!(headers[name], value, "{path}: {name}");
        }
        assert!(
            headers["content-security-policy"]
                .to_str()
                .unwrap()
                .contains("frame-ancestors 'none'")
        );
    }
    let response = served
        .client
        .post(format!("{}{CHECK}", served.base_url))
        .send()
        .expect("expected minted key response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "no-store");
}

/// All route registrations are exercised without credentials, including routes added later.
/// A new endpoint outside the shared authentication layer must fail this check.
#[test]
fn every_registered_api_path_requires_a_key() {
    let served = serve("auth-all-routes");
    let client = bare_client();
    let source = include_str!("../server.rs");
    let mut checked = 0;
    for path in source.split('"').filter(|part| part.starts_with("/api/")) {
        let mut path = path.to_owned();
        while let Some(start) = path.find('{') {
            let end = start + path[start..].find('}').expect("closed route parameter");
            path.replace_range(start..=end, "test");
        }
        let mut recognized = false;
        for method in [
            reqwest::Method::GET,
            reqwest::Method::POST,
            reqwest::Method::DELETE,
        ] {
            let response = client
                .request(method.clone(), format!("{}{path}", served.base_url))
                .send()
                .expect("expected response");
            if response.status() == StatusCode::METHOD_NOT_ALLOWED {
                continue;
            }
            recognized = true;
            assert_eq!(
                response.status(),
                StatusCode::UNAUTHORIZED,
                "{method} {path}"
            );
        }
        assert!(recognized, "no tested method for {path}");
        checked += 1;
    }
    assert!(
        checked > 50,
        "expected to exercise the server's API surface"
    );
}

/// A login link's ticket logs a browser in once, with a pass key of the browser's own in the
/// cookie - never the ticket, which went through places a key must not. The same link a
/// second time is the login page, saying why.
#[test]
fn a_login_ticket_logs_in_once_with_a_new_pass_key() {
    let served = serve("auth-ticket");
    let client = bare_client();
    let login = format!(
        "{}/moon/login?repo=%2Fsomewhere&frame=shell",
        served.base_url
    );
    let ticket = of_this_test_run().login_ticket(crate::pass_keys::OPEN_IN_WEB_TICKET_LIFETIME);

    let admitted = client
        .post(&login)
        .form(&[("ticket", ticket.as_str())])
        .send()
        .expect("failed to log in");
    assert_eq!(admitted.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        admitted.headers()[LOCATION],
        "/moon/?repo=%2Fsomewhere&frame=shell"
    );
    let cookie = admitted.headers()[SET_COOKIE]
        .to_str()
        .expect("expected a readable cookie");
    let session = cookie
        .strip_prefix("moon_pass_key=")
        .and_then(|rest| rest.split(';').next())
        .expect("expected the session cookie");
    assert_ne!(session, ticket);
    assert!(
        of_this_test_run().admits_browser_session(session),
        "the cookie should hold a browser session"
    );

    let again = client
        .post(&login)
        .form(&[("ticket", ticket.as_str())])
        .send()
        .expect("failed to log in");
    assert_eq!(again.status(), StatusCode::UNAUTHORIZED);
    assert!(again.headers().get(SET_COOKIE).is_none());
    assert!(
        again
            .text()
            .expect("expected a page")
            .contains("that login link has expired or was used already")
    );
}

/// A ticket is for the login form alone: shown to the API it is no key, and a key posted as a
/// ticket is no ticket. A form with both, or neither, is not a login at all.
#[test]
fn a_ticket_is_no_pass_key_and_a_pass_key_no_ticket() {
    let served = serve("auth-ticket-apart");
    let client = bare_client();
    let ticket = of_this_test_run().login_ticket(crate::pass_keys::OPEN_IN_WEB_TICKET_LIFETIME);
    let key = of_this_test_run().generate();

    let as_key = client
        .get(format!("{}{CHECK}", served.base_url))
        .header(AUTHORIZATION, format!("Bearer {ticket}"))
        .send()
        .expect("failed to ask");
    assert_eq!(as_key.status(), StatusCode::UNAUTHORIZED);

    let login = format!("{}/moon/login", served.base_url);
    let as_ticket = client
        .post(&login)
        .form(&[("ticket", key.as_str())])
        .send()
        .expect("failed to log in");
    assert_eq!(as_ticket.status(), StatusCode::UNAUTHORIZED);
    assert!(as_ticket.headers().get(SET_COOKIE).is_none());
    assert!(
        as_ticket
            .text()
            .expect("expected a page")
            .contains("that login link is not valid")
    );

    for form in [
        vec![("ticket", ticket.as_str()), ("pass_key", key.as_str())],
        vec![],
    ] {
        let refused = client
            .post(&login)
            .form(&form)
            .send()
            .expect("failed to log in");
        assert_eq!(refused.status(), StatusCode::BAD_REQUEST, "{form:?}");
        assert!(refused.headers().get(SET_COOKIE).is_none());
    }
}

/// A window already let in asks for a ticket to open a browser with; nothing else may, and
/// nobody gets one meant to last.
#[test]
fn a_login_ticket_is_minted_only_for_a_client_let_in_and_only_short_lived() {
    let served = serve("auth-ticket-mint");
    let url = format!("{}/api/login-ticket", served.base_url);
    let asked = serde_json::json!({ "lifetime_seconds": 60 });

    let refused = bare_client()
        .post(&url)
        .json(&asked)
        .send()
        .expect("failed to ask");
    assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);

    let minted: crate::api::LoginTicketMinted = served
        .client
        .post(&url)
        .json(&asked)
        .send()
        .expect("failed to ask")
        .error_for_status()
        .expect("expected a ticket")
        .json()
        .expect("expected a ticket in the answer");
    assert_eq!(
        crate::pass_keys::RedeemedTickets::default().redeem(&of_this_test_run(), &minted.ticket),
        Ok(())
    );

    let too_long = served
        .client
        .post(&url)
        .json(&serde_json::json!({ "lifetime_seconds": 24 * 60 * 60 }))
        .send()
        .expect("failed to ask");
    assert_eq!(too_long.status(), StatusCode::BAD_REQUEST);
}
