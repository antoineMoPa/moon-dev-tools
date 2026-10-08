//! Profile routes - only the admitted caller's account and saved layout.
use super::{Layout, ProfileView, Profiles, github};
use crate::server::users::UserId;
use axum::{Extension, Json, extract::State, http::StatusCode};
use serde::Deserialize;
type Error = (StatusCode, String);
fn failed(error: anyhow::Error) -> Error {
    (StatusCode::BAD_REQUEST, error.to_string())
}
pub(in crate::server) async fn me(
    State(profiles): State<Profiles>,
    Extension(user): Extension<UserId>,
) -> Json<ProfileView> {
    Json(profiles.view(&user))
}
pub(in crate::server) async fn device(
    State(profiles): State<Profiles>,
    Extension(user): Extension<UserId>,
) -> Result<Json<github::DeviceView>, Error> {
    let generation = profiles.begin_auth(&user);
    let (view, pending) = github::begin(user.to_string(), generation)
        .await
        .map_err(failed)?;
    let generations = profiles.generations.lock().unwrap();
    if generations.get(&user.to_string()) != Some(&generation) {
        return Err((StatusCode::CONFLICT, "Sign-in was cancelled".into()));
    }
    let mut flows = profiles.pending.lock().unwrap();
    flows.retain(|_, p| p.expires > std::time::Instant::now());
    if flows.len() >= 100 {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            "Too many sign-ins in progress".into(),
        ));
    }
    flows.insert(view.flow_id.clone(), pending);
    Ok(Json(view))
}
#[derive(Deserialize)]
pub(in crate::server) struct Flow {
    flow_id: String,
}
pub(in crate::server) async fn poll(
    State(profiles): State<Profiles>,
    Extension(user): Extension<UserId>,
    Json(request): Json<Flow>,
) -> Result<Json<serde_json::Value>, Error> {
    let pending = {
        let mut flows = profiles.pending.lock().unwrap();
        if !flows
            .get(&request.flow_id)
            .is_some_and(|p| p.user == user.to_string())
        {
            return Err((
                StatusCode::NOT_FOUND,
                "Sign-in not found; start again".into(),
            ));
        }
        flows.remove(&request.flow_id).unwrap()
    };
    let generation = pending.generation;
    match github::poll(pending).await.map_err(failed)? {
        github::Poll::Waiting(pending) => {
            let generations = profiles.generations.lock().unwrap();
            if generations.get(&user.to_string()) != Some(&generation) {
                return Err((StatusCode::CONFLICT, "Sign-in was cancelled".into()));
            }
            profiles
                .pending
                .lock()
                .unwrap()
                .insert(request.flow_id, pending);
            Ok(Json(serde_json::json!({"connected":false})))
        }
        github::Poll::Connected(account) => {
            profiles
                .connect_if_current(&user, account, generation)
                .map_err(failed)?;
            Ok(Json(serde_json::json!({"connected":true})))
        }
    }
}
pub(in crate::server) async fn disconnect(
    State(profiles): State<Profiles>,
    Extension(user): Extension<UserId>,
) -> Result<StatusCode, Error> {
    profiles.forget_admission(&user).map_err(failed)?;
    Ok(StatusCode::NO_CONTENT)
}
#[derive(Deserialize)]
pub(in crate::server) struct SaveLayout {
    profile_id: String,
    layout: Layout,
}
pub(in crate::server) async fn save_layout(
    State(profiles): State<Profiles>,
    Extension(user): Extension<UserId>,
    Json(request): Json<SaveLayout>,
) -> Result<StatusCode, Error> {
    if !profiles
        .account(&user)
        .is_some_and(|a| a.id == request.profile_id)
    {
        return Err((
            StatusCode::CONFLICT,
            "Account changed; reload this window".into(),
        ));
    }
    if request
        .layout
        .layout
        .as_ref()
        .is_some_and(|l| l.len() > 1_000_000)
        || request
            .layout
            .repo
            .as_ref()
            .is_some_and(|r| r.len() > 16_384)
        || !matches!(request.layout.frame.as_str(), "review" | "tasks" | "shell")
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "Saved layout is invalid or too large".into(),
        ));
    }
    profiles
        .change(|inner| {
            anyhow::ensure!(
                inner.admissions.get(&user.to_string()) == Some(&request.profile_id),
                "Account changed; reload this window"
            );
            inner.accounts.get_mut(&request.profile_id).unwrap().layout = Some(request.layout);
            Ok(())
        })
        .map_err(failed)?;
    Ok(StatusCode::NO_CONTENT)
}
