//! The HTTP surface a remote window reviews through. Every route is a thin wrapper over
//! [`crate::service`], which a window on this machine calls directly.

mod auth;
mod board_routes;
mod review_routes;
pub(crate) mod users;
mod web_page;

use board_routes::{
    amend_review_request, change_settings, list_review_requests, settings,
    add_column, attach_task_resource, create_task, delete_column, delete_task,
    delete_task_resource, explain_task_changes, link_task_file, list_agent_sessions, list_columns,
    list_tasks, open_task_notes, open_work_log, place_column, place_tasks, project_commands,
    rename_column, rename_task, resume_task_resource, run_project_command, set_column_arrivals,
    set_column_sort, set_project_config, set_task_tags, start_task_resource, stop_task_resource,
};
use review_routes::{
    agent_dispatch_log_request, blame_session_file, cancel_comment_dispatch_request,
    commit_history, commit_run_outcome, commit_state, create_session_file, discard_hunk,
    discard_hunks, find_session_files, hunk_patch, open_session, resolve_comment,
    resolve_comment_by_key, search_session_contents, send_comment_batch, session_file,
    session_file_at, session_state, session_submodules, stage_all, stage_file, stage_hunk,
    stage_selection, start_commit_run, suggest_commit_message, unstage_file, unstage_hunk,
    update_agent, update_comment, update_commit_view, write_session_file,
};

use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use axum::{
    Json, Router,
    extract::{FromRef, State},
    http::StatusCode,
    middleware,
    response::{Html, IntoResponse},
    routing::{delete, get, post},
};

use crate::{
    agent::detect_agent_availability,
    api::{AppState, ServerState, bind_host, port, server_url},
    pass_keys::{LONGEST_TICKET_LIFETIME, PassKeys, RedeemedTickets, SERVE_TICKET_LIFETIME},
};
use users::Users;

const SERVER_LIFETIME: Duration = Duration::from_secs(30 * 60);
/// The state the window and the server share. The app builds this once and hands a clone to
/// the server it carries, so a remote window reviews the same sessions this one does.
pub(crate) fn build_state(last_activity: Arc<Mutex<Instant>>) -> AppState {
    AppState {
        inner: Arc::new(Mutex::new(ServerState {
            home_repo: repo_started_in(),
            ..ServerState::default()
        })),
        agent_availability: detect_agent_availability(),
        last_activity: Arc::clone(&last_activity),
        terminals: Arc::new(crate::terminal::TerminalRegistry::new(last_activity)),
        // The servers are told they are talking to this application rather than to the
        // client crate they are reached through: `clientInfo` is what a server writes into
        // its log, and a report about rust-analyzer under a review is only findable if the
        // log says which program was asking.
        lsp: Arc::new(
            moon_lsp::LspRegistry::new(crate::shell_path::installed_tools_path().to_string())
                .identifying_as(moon_lsp::ClientIdentity::new(
                    "moonreview",
                    env!("CARGO_PKG_VERSION"),
                )),
        ),
        settings_path: crate::settings::path(),
    }
}

/// The repo around the folder this process was started in. A folder that cannot be read -
/// deleted since, say - is in no repo, the same as one outside every repo.
fn repo_started_in() -> Option<std::path::PathBuf> {
    let folder = std::env::current_dir().ok()?;
    crate::git::find_repo_root(&folder).ok().flatten()
}

/// What the routes are served with: the reviews, the users a request is let in as - which hold
/// the keys it is checked against - and the login tickets this server has let a browser in with
/// already. A handler asks for whichever part it needs - see the [`FromRef`]s below.
#[derive(Clone)]
pub(crate) struct Served {
    app: AppState,
    users: Users,
    redeemed_tickets: Arc<RedeemedTickets>,
}

impl FromRef<Served> for AppState {
    fn from_ref(served: &Served) -> Self {
        served.app.clone()
    }
}

/// The keys in force right now: `Tools › Users` replaces them - see [`users::Users::kick_everyone`].
impl FromRef<Served> for PassKeys {
    fn from_ref(served: &Served) -> Self {
        served.users.keys()
    }
}

impl FromRef<Served> for Users {
    fn from_ref(served: &Served) -> Self {
        served.users.clone()
    }
}

impl FromRef<Served> for Arc<RedeemedTickets> {
    fn from_ref(served: &Served) -> Self {
        Arc::clone(&served.redeemed_tickets)
    }
}

/// Every route: the few anyone may reach, and the API behind a pass key.
///
/// What is public holds no data - the health check, the root page, and the files of the
/// browser build, which are the same for every server. The page at `/moon/` is public too, as
/// the place a browser logs in; it only starts the window once the browser has - see
/// [`web_page::moon_page`].
pub(crate) fn router(state: AppState, users: Users) -> Router {
    let served = Served {
        app: state,
        users,
        redeemed_tickets: Arc::new(RedeemedTickets::default()),
    };
    Router::new()
        .route("/", get(root))
        .route("/healthz", get(healthz))
        .route("/moon", get(web_page::moon_without_slash))
        .route("/moon/", get(web_page::moon_page))
        .route("/moon/login", post(web_page::log_in))
        .route("/moon/{*path}", get(web_page::moon_file))
        .merge(
            protected_routes().route_layer(middleware::from_fn_with_state(
                served.clone(),
                auth::require_pass_key,
            )),
        )
        .layer(middleware::from_fn(auth::browser_boundary))
        .with_state(served)
}

/// The API: everything that reads the repo, writes to it or runs something in it.
fn protected_routes() -> Router<Served> {
    Router::new()
        .route("/api/pass-key", get(pass_key_admitted).post(mint_pass_key))
        .route("/api/users", get(users::list))
        .route("/api/users/kick-all", post(users::kick_everyone))
        .route("/api/users/{id}/kick", post(users::kick))
        .route("/api/login-ticket", post(mint_login_ticket))
        .route(
            "/api/session/{session_id}/resolve/{hunk_id}/{comment_index}",
            get(resolve_comment),
        )
        .route(
            "/api/session/{session_id}/resolve-key/{hunk_id}/{comment_key}",
            get(resolve_comment_by_key),
        )
        .route("/api/session/open", post(open_session))
        .route("/api/session/{session_id}/state", get(session_state))
        .route(
            "/api/session/{session_id}/submodules",
            get(session_submodules),
        )
        .route("/api/session/{session_id}/history", get(commit_history))
        .route("/api/session/{session_id}/agent", post(update_agent))
        .route("/api/session/{session_id}/commit", post(update_commit_view))
        .route("/api/session/{session_id}/hunk/{hunk_id}", get(hunk_patch))
        .route(
            "/api/session/{session_id}/file",
            get(session_file).post(write_session_file),
        )
        .route(
            "/api/session/{session_id}/file/new",
            post(create_session_file),
        )
        .route("/api/session/{session_id}/blame", post(blame_session_file))
        .route("/api/session/{session_id}/file-at", get(session_file_at))
        .route("/api/session/{session_id}/files", get(find_session_files))
        .route(
            "/api/session/{session_id}/content",
            get(search_session_contents),
        )
        .route("/api/session/{session_id}/comment", post(update_comment))
        .route(
            "/api/session/{session_id}/comment-batch",
            post(send_comment_batch),
        )
        .route(
            "/api/session/{session_id}/comment-dispatch/cancel",
            post(cancel_comment_dispatch_request),
        )
        .route(
            "/api/session/{session_id}/agent-dispatch/log",
            get(agent_dispatch_log_request),
        )
        .route("/api/session/{session_id}/stage", post(stage_hunk))
        .route("/api/session/{session_id}/stage-file", post(stage_file))
        .route(
            "/api/session/{session_id}/stage-selection",
            post(stage_selection),
        )
        .route("/api/session/{session_id}/discard", post(discard_hunk))
        .route(
            "/api/session/{session_id}/discard-batch",
            post(discard_hunks),
        )
        .route("/api/session/{session_id}/unstage", post(unstage_hunk))
        .route("/api/session/{session_id}/unstage-file", post(unstage_file))
        .route(
            "/api/session/{session_id}/tasks",
            get(list_tasks).post(create_task),
        )
        .route(
            "/api/session/{session_id}/tasks/{task_id}",
            delete(delete_task),
        )
        .route(
            "/api/session/{session_id}/tasks/placement",
            post(place_tasks),
        )
        .route(
            "/api/session/{session_id}/columns",
            get(list_columns).post(add_column),
        )
        .route("/api/settings", get(settings).post(change_settings))
        .route(
            "/api/session/{session_id}/review-requests",
            get(list_review_requests),
        )
        .route(
            "/api/session/{session_id}/tasks/{task_id}/review-requests/{index}",
            post(amend_review_request),
        )
        .route(
            "/api/session/{session_id}/project",
            get(project_commands).post(set_project_config),
        )
        .route(
            "/api/session/{session_id}/project/run/{which}",
            post(run_project_command),
        )
        .route(
            "/api/session/{session_id}/run-in-shell",
            post(crate::terminal::run_in_shell),
        )
        .route(
            "/api/session/{session_id}/columns/{column_id}",
            delete(delete_column),
        )
        .route(
            "/api/session/{session_id}/columns/{column_id}/title",
            post(rename_column),
        )
        .route(
            "/api/session/{session_id}/columns/{column_id}/arrivals",
            post(set_column_arrivals),
        )
        .route(
            "/api/session/{session_id}/columns/{column_id}/sort",
            post(set_column_sort),
        )
        .route(
            "/api/session/{session_id}/columns/{column_id}/placement",
            post(place_column),
        )
        .route(
            "/api/session/{session_id}/tasks/{task_id}/resources",
            post(start_task_resource),
        )
        .route(
            "/api/session/{session_id}/tasks/{task_id}/explanation",
            post(explain_task_changes),
        )
        .route(
            "/api/session/{session_id}/agent-sessions",
            get(list_agent_sessions),
        )
        .route(
            "/api/session/{session_id}/tasks/{task_id}/resources/attach",
            post(attach_task_resource),
        )
        .route(
            "/api/session/{session_id}/tasks/{task_id}/resources/{resource_id}/resume",
            post(resume_task_resource),
        )
        .route(
            "/api/session/{session_id}/tasks/{task_id}/resources/{resource_id}/stop",
            post(stop_task_resource),
        )
        .route(
            "/api/session/{session_id}/tasks/{task_id}/resources/{resource_id}",
            delete(delete_task_resource),
        )
        .route(
            "/api/session/{session_id}/tasks/{task_id}/title",
            post(rename_task),
        )
        .route(
            "/api/session/{session_id}/tasks/{task_id}/tags",
            post(set_task_tags),
        )
        .route(
            "/api/session/{session_id}/tasks/{task_id}/notes/open",
            post(open_task_notes),
        )
        .route(
            "/api/session/{session_id}/tasks/{task_id}/files",
            post(link_task_file),
        )
        .route(
            "/api/session/{session_id}/work-log/open",
            post(open_work_log),
        )
        .route("/api/session/{session_id}/commit-state", get(commit_state))
        .route("/api/session/{session_id}/stage-all", post(stage_all))
        .route(
            "/api/session/{session_id}/commit-message",
            post(suggest_commit_message),
        )
        .route(
            "/api/session/{session_id}/commit-run",
            post(start_commit_run),
        )
        .route(
            "/api/session/{session_id}/commit-run/{terminal_id}/outcome",
            get(commit_run_outcome),
        )
        .route(
            "/api/session/{session_id}/lsp/status",
            get(crate::lsp::routes::status),
        )
        .route(
            "/api/session/{session_id}/lsp/working",
            get(crate::lsp::routes::working),
        )
        .route(
            "/api/session/{session_id}/lsp/triggers",
            get(crate::lsp::routes::triggers),
        )
        .route(
            "/api/session/{session_id}/lsp/open",
            post(crate::lsp::routes::did_open),
        )
        .route(
            "/api/session/{session_id}/lsp/change",
            post(crate::lsp::routes::did_change),
        )
        .route(
            "/api/session/{session_id}/lsp/close",
            post(crate::lsp::routes::did_close),
        )
        .route(
            "/api/session/{session_id}/lsp/places",
            post(crate::lsp::routes::places),
        )
        .route(
            "/api/session/{session_id}/lsp/completion",
            post(crate::lsp::routes::completion),
        )
        .route(
            "/api/session/{session_id}/lsp/prepare-rename",
            post(crate::lsp::routes::prepare_rename),
        )
        .route(
            "/api/session/{session_id}/lsp/rename",
            post(crate::lsp::routes::rename),
        )
        .route(
            "/api/session/{session_id}/lsp/format",
            post(crate::lsp::routes::format),
        )
        .route(
            "/api/session/{session_id}/lsp/hover",
            post(crate::lsp::routes::hover),
        )
        .route(
            "/api/session/{session_id}/lsp/diagnostics",
            get(crate::lsp::routes::diagnostics),
        )
        .route(
            "/api/session/{session_id}/lsp/save",
            post(crate::lsp::routes::did_save),
        )
        .route(
            "/api/session/{session_id}/lsp/code-actions",
            post(crate::lsp::routes::code_actions),
        )
        .route(
            "/api/session/{session_id}/lsp/signature",
            post(crate::lsp::routes::signature_help),
        )
        .route(
            "/api/session/{session_id}/terminals",
            get(crate::terminal::list_terminals).post(crate::terminal::create_terminal),
        )
        .route(
            "/api/session/{session_id}/terminals/running",
            get(crate::terminal::terminals_running_a_command),
        )
        .route(
            "/api/session/{session_id}/terminals/attention",
            get(crate::terminal::terminals_wanting_attention),
        )
        .route(
            "/api/session/{session_id}/terminals/visualizations",
            get(crate::visualizations::routes::announced),
        )
        .route(
            "/api/session/{session_id}/visualizations/page",
            get(crate::visualizations::routes::page),
        )
        .route(
            "/api/session/{session_id}/terminals/{terminal_id}",
            get(crate::terminal::terminal_view).delete(crate::terminal::close_terminal),
        )
        .route(
            "/api/session/{session_id}/terminals/{terminal_id}/name",
            post(crate::terminal::rename_terminal),
        )
        .route(
            "/api/session/{session_id}/terminals/{terminal_id}/socket",
            get(crate::terminal::terminal_socket),
        )
}

/// `moon serve`: the server in this terminal, which prints a link that logs a browser in -
/// once, and for [`SERVE_TICKET_LIFETIME`]: what a terminal prints ends up in scrollback and
/// logs, which is no place for a key that lasts. The ticket rides in the fragment, which a
/// browser never sends to a server; the login page takes it off the address. A key that lasts,
/// for a window's `--pass-key`, is `moon generate-pass-key`'s to print, on purpose.
pub(crate) async fn run_server() -> Result<()> {
    let last_activity = Arc::new(Mutex::new(Instant::now()));
    let state = build_state(Arc::clone(&last_activity));
    let users = Users::for_this_machine()?;
    let listener = bind().await?;

    let ticket = users.keys().login_ticket(SERVE_TICKET_LIFETIME);
    println!("Moon Review listening on {}", server_url());
    println!("The window in a browser: {}/moon", server_url());
    println!(
        "Log in within {} minutes: {}/moon/#ticket={ticket}",
        SERVE_TICKET_LIFETIME.as_secs() / 60,
        server_url()
    );
    println!(
        "`moon generate-pass-key` prints a pass key that lasts, for --pass-key or the login page"
    );
    serve_on(state, users, listener, Some(last_activity)).await
}

/// Serve the review API from inside a window, which decides itself when the process ends.
pub(crate) async fn serve(state: AppState) -> Result<()> {
    let users = Users::for_this_machine()?;
    let listener = bind().await?;

    println!("Moon Review listening on {}", server_url());
    println!("The window in a browser: {}/moon", server_url());
    serve_on(state, users, listener, None).await
}

/// How many ports up from the asked one are tried before giving up: enough for every window
/// and `moon serve` one machine has open at once.
const PORTS_TRIED: u16 = 20;

/// Bind the asked port - see [`crate::api::port`] - or, when another server already has it,
/// the first free one after it: a second window or a `moon serve` beside a window gets a
/// server of its own rather than an error. The port bound is recorded, so the address printed
/// and handed to agents is the one that answers - see [`crate::api::record_bound_port`].
async fn bind() -> Result<tokio::net::TcpListener> {
    let asked = port()?;
    let host = bind_host();
    for port in asked..asked.saturating_add(PORTS_TRIED) {
        match tokio::net::TcpListener::bind((host.as_str(), port)).await {
            Ok(listener) => {
                if port != asked {
                    println!("port {asked} is taken; listening on {port} instead");
                }
                crate::api::record_bound_port(port);
                return Ok(listener);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => continue,
            Err(error) => {
                return Err(error).with_context(|| format!("failed to bind {host}:{port}"));
            }
        }
    }
    anyhow::bail!(
        "every port from {asked} to {} on {host} is taken",
        asked.saturating_add(PORTS_TRIED) - 1
    )
}

/// Serve on a listener the caller already bound, which is how a test gets a free port.
/// `idle_shutdown` is the clock the standalone server stops on; a window passes `None`.
///
/// Every request is told the address it came from, which is what the users list shows - see
/// [`auth::require_pass_key`].
pub(crate) async fn serve_on(
    state: AppState,
    users: Users,
    listener: tokio::net::TcpListener,
    idle_shutdown: Option<Arc<Mutex<Instant>>>,
) -> Result<()> {
    let service = router(state, users).into_make_service_with_connect_info::<SocketAddr>();
    match idle_shutdown {
        Some(last_activity) => axum::serve(listener, service)
            .with_graceful_shutdown(shutdown_signal(last_activity))
            .await
            .context("server failed"),
        None => axum::serve(listener, service)
            .await
            .context("server failed"),
    }
}

async fn shutdown_signal(last_activity: Arc<Mutex<Instant>>) {
    loop {
        let idle_for = last_activity
            .lock()
            .map(|value| value.elapsed())
            .unwrap_or(SERVER_LIFETIME);
        let remaining = SERVER_LIFETIME.saturating_sub(idle_for);
        let timeout = tokio::time::sleep(remaining);
        tokio::pin!(timeout);

        tokio::select! {
            _ = &mut timeout => {
                let idle_for = last_activity
                    .lock()
                    .map(|value| value.elapsed())
                    .unwrap_or(SERVER_LIFETIME);
                if idle_for >= SERVER_LIFETIME {
                    eprintln!(
                        "[moonreview] shutting down after {} minutes of inactivity",
                        SERVER_LIFETIME.as_secs() / 60
                    );
                    return;
                }
            }
            result = tokio::signal::ctrl_c() => {
                if let Err(error) = result {
                    eprintln!("[moonreview] failed to listen for shutdown signal: {error}");
                }
                return;
            }
        }
    }
}

fn mark_activity(state: &AppState) {
    crate::api::mark_activity(&state.last_activity);
}

async fn root(State(state): State<AppState>) -> impl IntoResponse {
    mark_activity(&state);
    Html(
        "<!doctype html><title>Moon Review</title><p>A review server. Open <a href=\"/moon\">the window</a> here, or point one at it with <code>moon review --remote</code>.</p>",
    )
}

async fn healthz(State(state): State<AppState>) -> &'static str {
    mark_activity(&state);
    "ok"
}

/// `GET /api/pass-key`: nothing but the layer's answer, which is how a window checks the key
/// it was given while it is still connecting rather than on the first thing it asks for.
async fn pass_key_admitted() -> StatusCode {
    StatusCode::NO_CONTENT
}

/// `POST /api/pass-key`: a new key, for a window already let in to hand on - to a person who
/// asked for one, or a window it starts. It lets its holder do nothing the asker cannot.
async fn mint_pass_key(State(keys): State<PassKeys>) -> Json<crate::api::PassKeyMinted> {
    Json(crate::api::PassKeyMinted {
        pass_key: keys.generate(),
    })
}

/// `POST /api/login-ticket`: a login ticket, for a window already let in to open a browser
/// with - see [`crate::pass_keys`]. A lifetime past [`LONGEST_TICKET_LIFETIME`] is refused: a
/// ticket is for the moment of logging in, and what is meant to last is a pass key.
async fn mint_login_ticket(
    State(keys): State<PassKeys>,
    Json(asked): Json<crate::api::LoginTicketRequest>,
) -> Result<Json<crate::api::LoginTicketMinted>, (StatusCode, String)> {
    let lifetime = Duration::from_secs(asked.lifetime_seconds);
    if lifetime > LONGEST_TICKET_LIFETIME {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "a login ticket lasts at most {} seconds, not {}\n",
                LONGEST_TICKET_LIFETIME.as_secs(),
                asked.lifetime_seconds
            ),
        ));
    }
    Ok(Json(crate::api::LoginTicketMinted {
        ticket: keys.login_ticket(lifetime),
    }))
}
