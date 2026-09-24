//! The window as a page, at `/moon`: this crate built to wasm, and embedded in the executable,
//! by build.rs. A browser that opens it is the same window a
//! `moon review --remote` would be, on this server.

use std::sync::Arc;

use axum::{
    Form,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{Html, IntoResponse, Redirect, Response},
};
use serde::Deserialize;

use crate::{
    api::AppState,
    backend::remote::urlencode,
    pass_keys::{PassKeys, RedeemedTickets},
};

use super::{
    auth::{cookie_key, logged_in_cookie},
    mark_activity,
};

/// Every file of the browser build, by its path under `/moon/`, gzipped - build.rs packs them,
/// and they are sent as they are, for the browser to unpack.
const WEB_BUNDLE: &[(&str, &[u8])] = include!(concat!(env!("OUT_DIR"), "/web_bundle.rs"));

/// What each kind of file in the bundle is served as. A module script has to be served as
/// JavaScript and the module as wasm, or the browser refuses them.
const CONTENT_TYPES: &[(&str, &str)] = &[
    ("html", "text/html; charset=utf-8"),
    ("js", "text/javascript"),
    ("wasm", "application/wasm"),
    ("css", "text/css"),
];

/// `/moon` without its slash: the page's files are named relative to it, so it is only ever
/// served as `/moon/`. The query - which repo, which frame - comes along.
pub(super) async fn moon_without_slash(Query(asked): Query<Vec<(String, String)>>) -> Redirect {
    // Adding the server's repo here would disclose its filesystem path before login.
    Redirect::temporary(&address_with_query(&asked))
}

/// The page, once it says which repo it is on. One that does not is sent to the repo this
/// process is for, when there is one: `moon serve` run in a repo, or a window open on one, is a
/// way of saying which repo the browser is for. The address says so rather than the page quietly
/// assuming it, so it can be copied to open the same repo.
///
/// A browser that has not logged in is shown the login page in its place, at the same address,
/// so that logging in comes back to the same repo and frame.
pub(super) async fn moon_page(
    State(state): State<AppState>,
    State(keys): State<PassKeys>,
    Query(asked): Query<Vec<(String, String)>>,
    headers: HeaderMap,
) -> Response {
    mark_activity(&state);
    if !cookie_key(&headers).is_some_and(|session| keys.admits_browser_session(session)) {
        return login_page(&asked, None);
    }
    let names_repo = asked.iter().any(|(name, _)| name == REPO_PARAMETER);
    if !names_repo && home_repo(&state).is_some() {
        return Redirect::temporary(&page_address(&state, asked)).into_response();
    }
    bundled("index.html", &headers)
}

/// What the login page's forms send: a pass key someone pasted, or the login ticket of a link
/// - see [`crate::pass_keys`]. One or the other, never both.
#[derive(Deserialize)]
pub(super) struct LoginForm {
    pass_key: Option<String>,
    ticket: Option<String>,
}

const NEITHER_OR_BOTH: &str = "a login posts either `pass_key` or `ticket`\n";

/// `POST /moon/login`: a good pass key or an unspent ticket logs the browser in - it is given a
/// session of its own in its cookie, good for a week, and sent back to the page it was on; any
/// other is the login page again, saying so. The cookie never holds the key or the ticket: a
/// pass key lasts as long as the secret, and a browser's login is to run out on its own - see
/// [`crate::pass_keys::BROWSER_SESSION_LIFETIME`].
///
/// 303 so the browser follows with a GET, and reloading the page it lands on does not post the
/// key a second time.
pub(super) async fn log_in(
    State(state): State<AppState>,
    State(keys): State<PassKeys>,
    State(redeemed): State<Arc<RedeemedTickets>>,
    Query(asked): Query<Vec<(String, String)>>,
    Form(form): Form<LoginForm>,
) -> Response {
    mark_activity(&state);
    match (form.pass_key, form.ticket) {
        (Some(key), None) => {
            if !keys.admits(&key) {
                return login_page(&asked, Some(KEY_REFUSED));
            }
        }
        (None, Some(ticket)) => {
            if let Err(refusal) = redeemed.redeem(&keys, &ticket) {
                return login_page(&asked, Some(refusal.explanation()));
            }
        }
        (None, None) | (Some(_), Some(_)) => {
            return (StatusCode::BAD_REQUEST, NEITHER_OR_BOTH).into_response();
        }
    }
    (
        StatusCode::SEE_OTHER,
        [
            // Never the key or the ticket: a session of the browser's own, good for a week.
            (header::SET_COOKIE, logged_in_cookie(&keys.browser_session())),
            (
                header::LOCATION,
                page_address(&state, asked)
                    .parse()
                    .expect("a page address is URL-encoded, which a header can carry"),
            ),
        ],
    )
        .into_response()
}

/// The page a browser logs in on, kept apart as HTML - see `login.html` beside this file.
const LOGIN_PAGE: &str = include_str!("login.html");
const KEY_REFUSED: &str = "that pass key is not valid";

/// The login page, posting back to the query it was asked with. 401 either way: until the key
/// is in, the page the address names is not being shown.
///
/// The query is written back through [`urlencode`], which leaves nothing an attribute could be
/// broken out of but `&` - escaped here, as HTML has it. `refusal` is always a constant - this
/// file's own, or a ticket refusal's explanation - never anything the request said.
fn login_page(asked: &[(String, String)], refusal: Option<&'static str>) -> Response {
    let action = match asked.is_empty() {
        true => "/moon/login".to_owned(),
        false => format!("/moon/login?{}", query_pairs(asked).join("&amp;")),
    };
    let refusal = refusal.map_or(String::new(), |refusal| {
        format!("<p class=\"refusal\">{refusal}</p>")
    });
    let page = LOGIN_PAGE
        .replace("{{action}}", &action)
        .replace("{{refusal}}", &refusal);
    (StatusCode::UNAUTHORIZED, Html(page)).into_response()
}

/// The query parameter the page reads its repo from - see `crate::web`.
const REPO_PARAMETER: &str = "repo";

/// Where the page is for this query, with the repo this process is for added when the query
/// names none. Temporary wherever it is sent: which repo that is depends on the process, which a
/// browser must not remember past it.
fn page_address(state: &AppState, mut asked: Vec<(String, String)>) -> String {
    let names_repo = asked.iter().any(|(name, _)| name == REPO_PARAMETER);
    if !names_repo && let Some(repo) = home_repo(state) {
        asked.insert(0, (REPO_PARAMETER.to_owned(), repo));
    }
    address_with_query(&asked)
}

fn address_with_query(asked: &[(String, String)]) -> String {
    if asked.is_empty() {
        return "/moon/".to_owned();
    }
    format!("/moon/?{}", query_pairs(asked).join("&"))
}

/// The pairs of a query, each written out as `name=value` through [`urlencode`].
fn query_pairs(asked: &[(String, String)]) -> Vec<String> {
    asked
        .iter()
        .map(|(name, value)| format!("{}={}", urlencode(name), urlencode(value)))
        .collect()
}

/// The repo this process is for - see [`crate::api::ServerState::home_repo`].
fn home_repo(state: &AppState) -> Option<String> {
    let inner = state
        .inner
        .lock()
        .expect("the server state lock is poisoned");
    inner
        .home_repo
        .as_ref()
        .map(|repo| repo.to_string_lossy().into_owned())
}

pub(super) async fn moon_file(
    State(state): State<AppState>,
    Path(path): Path<String>,
    headers: HeaderMap,
) -> Response {
    mark_activity(&state);
    bundled(&path, &headers)
}

/// Whether the client said it reads gzip, which every browser does - a client that did not is
/// told so rather than handed bytes it cannot read.
fn takes_gzip(headers: &HeaderMap) -> bool {
    headers
        .get_all(header::ACCEPT_ENCODING)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .any(|coding| {
            coding
                .split(';')
                .next()
                .is_some_and(|name| name.trim() == "gzip")
        })
}

fn bundled(path: &str, headers: &HeaderMap) -> Response {
    if !takes_gzip(headers) {
        return (
            StatusCode::NOT_ACCEPTABLE,
            "the page is only sent gzipped; ask with `Accept-Encoding: gzip`\n",
        )
            .into_response();
    }
    let Some((_, contents)) = WEB_BUNDLE.iter().find(|(published, _)| *published == path) else {
        return (
            StatusCode::NOT_FOUND,
            format!("no {path} in the browser build"),
        )
            .into_response();
    };
    let extension = path.rsplit_once('.').map_or("", |(_, extension)| extension);
    let Some((_, content_type)) = CONTENT_TYPES.iter().find(|(known, _)| *known == extension)
    else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("the browser build has {path}, whose kind CONTENT_TYPES does not name"),
        )
            .into_response();
    };
    (
        [
            (header::CONTENT_TYPE, *content_type),
            (header::CONTENT_ENCODING, "gzip"),
        ],
        *contents,
    )
        .into_response()
}
