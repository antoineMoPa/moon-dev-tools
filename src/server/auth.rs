//! Who is let through to the API: a request that shows one of this machine's pass keys - see
//! [`crate::pass_keys`] - that has not been kicked out - see [`super::users`] - and comes from
//! no other site.
//!
//! One layer over every protected route rather than a check in each handler, so a route added
//! later is protected by being added, not by someone remembering to.

use std::net::SocketAddr;

use axum::{
    extract::{ConnectInfo, Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};

use super::users::{Admission, UserId, Users};

/// The cookie a browser carries its pass key in, set by `POST /moon/login` - see
/// [`super::web_page`].
pub(super) const PASS_KEY_COOKIE: &str = "moon_pass_key";

/// How long a browser keeps its cookie: as long as the session in it is good for, so the
/// browser drops it when the server would refuse it anyway - see
/// [`crate::pass_keys::BROWSER_SESSION_LIFETIME`].
const COOKIE_MAX_AGE_SECONDS: u64 = crate::pass_keys::BROWSER_SESSION_LIFETIME.as_secs();

/// What `Authorization` says before the key.
const BEARER_PREFIX: &str = "Bearer ";

/// What an `Origin` header starts with before the host a browser sent the request from.
const WEB_SCHEMES: &[&str] = &["http://", "https://"];

/// `Sec-Fetch-Site` values of a request the page itself made, or one the person typed into the
/// address bar. Every other value is another site's page asking - including `same-site`, which
/// is any other port on localhost, where `SameSite=Strict` still hands the cookie over.
const OWN_FETCH_SITES: &[&str] = &["same-origin", "none"];

const NO_KEY: &str = "moon needs a pass key: run `moon generate-pass-key` on the machine the \
server is on, and log in at /moon or pass it with --pass-key\n";
const REFUSED_KEY: &str = "that pass key is not valid for this server\n";
const KICKED: &str = "that pass key or login was kicked out of this server; get a new pass key\n";
const MALFORMED_AUTHORIZATION: &str = "Authorization must be `Bearer <pass key>`\n";
const OTHER_ORIGIN: &str = "moon only answers pages it served itself\n";

/// Apply to the entire router, including login, errors and redirects. The browser must not
/// cache credentials or repository data, or embed the window in a page controlling its UI.
///
/// The referrer policy is `same-origin` rather than `no-referrer`: under `no-referrer` a
/// browser posts a form with `Origin: null`, which the check on `/moon/login` rightly refuses,
/// so no browser could log in. `same-origin` still sends nothing - not the repo in the address
/// - to any other site.
pub(super) async fn browser_boundary(request: Request, next: Next) -> Response {
    let mut response = if request.uri().path() == "/moon/login"
        && (!origin_is_this_server(request.headers(), request.uri())
            || !fetched_by_this_server_s_page(request.headers()))
    {
        (StatusCode::FORBIDDEN, OTHER_ORIGIN).into_response()
    } else {
        next.run(request).await
    };
    let headers = response.headers_mut();
    for (name, value) in [
        (header::CACHE_CONTROL, "no-store"),
        (
            header::CONTENT_SECURITY_POLICY,
            "frame-ancestors 'none'; base-uri 'none'; form-action 'self'",
        ),
        (header::X_FRAME_OPTIONS, "DENY"),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        (header::REFERRER_POLICY, "same-origin"),
    ] {
        headers.insert(name, HeaderValue::from_static(value));
    }
    response
}

/// The layer: a request with a key this machine made goes on to its route, and one without is
/// answered 401 before the route is reached. One sent from another site's page is answered 403
/// whatever key it carries - a cookie is sent along by the browser, not by the page, so
/// holding one says nothing about who asked. One whose key was kicked is answered 401 too.
///
/// The route is told who asked: the [`UserId`] goes in the request's extensions, for the
/// users list to mark the asker and a shell's socket to hear of its user being kicked.
pub(super) async fn require_pass_key(
    State(users): State<Users>,
    mut request: Request,
    next: Next,
) -> Response {
    let headers = request.headers();
    if !origin_is_this_server(headers, request.uri()) {
        return (StatusCode::FORBIDDEN, OTHER_ORIGIN).into_response();
    }
    let keys = users.keys();
    let user = match headers.get(header::AUTHORIZATION) {
        Some(authorization) => {
            let Some(key) = authorization
                .to_str()
                .ok()
                .and_then(|value| value.strip_prefix(BEARER_PREFIX))
            else {
                return (StatusCode::UNAUTHORIZED, MALFORMED_AUTHORIZATION).into_response();
            };
            // A pass key: what a native window or `curl` is given to hold.
            let Some(id) = keys.admitted_key_id(key) else {
                return (StatusCode::UNAUTHORIZED, REFUSED_KEY).into_response();
            };
            UserId::PassKey(id)
        }
        None => {
            let Some(session) = cookie_key(headers) else {
                return (StatusCode::UNAUTHORIZED, NO_KEY).into_response();
            };
            if !fetched_by_this_server_s_page(headers) {
                return (StatusCode::FORBIDDEN, OTHER_ORIGIN).into_response();
            }
            // A browser session, never a pass key: a cookie is good for a week at most.
            let Some(id) = keys.admitted_browser_session_id(session) else {
                return (StatusCode::UNAUTHORIZED, REFUSED_KEY).into_response();
            };
            UserId::BrowserSession(id)
        }
    };
    let ip = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .expect("the server is started with `into_make_service_with_connect_info`")
        .ip();
    if users.let_in(&user, ip) == Admission::Kicked {
        return (StatusCode::UNAUTHORIZED, KICKED).into_response();
    }
    request.extensions_mut().insert(user);
    next.run(request).await
}

/// The pass key in the request's cookie, whether or not it is a valid one.
pub(super) fn cookie_key(headers: &HeaderMap) -> Option<&str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|cookies| cookies.split(';'))
        .filter_map(|cookie| cookie.trim().split_once('='))
        .find(|(name, _)| *name == PASS_KEY_COOKIE)
        .map(|(_, key)| key)
}

/// The `Set-Cookie` that logs a browser in with `session` - a browser session, never a pass
/// key. `HttpOnly` so no script - the page's own included - can read it back out, and
/// `SameSite=Strict` so another site's links and forms arrive without it.
pub(super) fn logged_in_cookie(session: &str) -> HeaderValue {
    HeaderValue::from_str(&format!(
        "{PASS_KEY_COOKIE}={session}; HttpOnly; SameSite=Strict; Path=/; Max-Age={COOKIE_MAX_AGE_SECONDS}"
    ))
    .expect("a session is base64url, dots and a prefix letter, which a header can carry")
}

/// Whether the page a request came from, when it came from one, is this server's. A browser
/// names the page's origin on every websocket and every cross-origin request, so an `Origin`
/// that is not this server's host is another site reaching in - the one way a page elsewhere
/// can open a shell here, since a websocket is not held back by CORS. A request with no
/// `Origin` is not from a page at all: a native window, `curl`, or the page's own GET.
fn origin_is_this_server(headers: &HeaderMap, uri: &axum::http::Uri) -> bool {
    let Some(origin) = headers.get(header::ORIGIN) else {
        return true;
    };
    let host = headers
        .get(header::HOST)
        .and_then(|host| host.to_str().ok())
        .or_else(|| uri.authority().map(|authority| authority.as_str()));
    let (Ok(origin), Some(host)) = (origin.to_str(), host) else {
        return false;
    };
    WEB_SCHEMES
        .iter()
        .filter_map(|scheme| origin.strip_prefix(scheme))
        .any(|origin_host| origin_host == host)
}

/// Whether a request carrying the cookie was made by this server's own page. Browsers that say
/// where a request came from say it in `Sec-Fetch-Site`, and that covers the requests that
/// send no `Origin` - an image or a link from a page on another port of localhost, which is
/// the same site and so is sent the cookie.
fn fetched_by_this_server_s_page(headers: &HeaderMap) -> bool {
    let Some(site) = headers.get("sec-fetch-site") else {
        return true;
    };
    site.to_str()
        .is_ok_and(|site| OWN_FETCH_SITES.contains(&site))
}
