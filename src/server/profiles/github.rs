//! GitHub - carries OAuth device sign-in and verifies its resulting identity.
use super::Account;
pub(super) use crate::api::profiles::DeviceView;
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::time::{Duration, Instant};

pub(in crate::server) fn client_id() -> Option<String> {
    std::env::var("MOON_GITHUB_CLIENT_ID")
        .ok()
        .and_then(|id| normalized_client_id(&id))
}
fn normalized_client_id(id: &str) -> Option<String> {
    let id = id.trim();
    (!id.is_empty()).then(|| id.to_string())
}
fn oauth_error(code: &str) -> &'static str {
    match code {
        "incorrect_client_credentials" => {
            "GitHub rejected the client ID. Set MOON_GITHUB_CLIENT_ID to the Client ID of your GitHub OAuth app and restart the server."
        }
        "device_flow_disabled" => {
            "GitHub device flow is disabled. Enable Device Flow in your GitHub OAuth app settings, then try again."
        }
        "access_denied" => "GitHub sign-in was cancelled or denied; start again to approve access.",
        "expired_token" => "GitHub sign-in expired; start again.",
        "incorrect_device_code" => "GitHub did not recognize this sign-in code; start again.",
        "unsupported_grant_type" => {
            "GitHub rejected the device sign-in request; check the server's GitHub OAuth configuration."
        }
        _ => {
            "GitHub rejected device sign-in; check the server's GitHub OAuth app configuration and try again."
        }
    }
}
// GitHub can return OAuth errors with HTTP 200. Decode known error codes before
// the success schema, and never expose response bodies, descriptions or credentials.
fn decode_reply<T: serde::de::DeserializeOwned>(
    status: reqwest::StatusCode,
    body: &[u8],
) -> Result<T> {
    let value: serde_json::Value = serde_json::from_slice(body)
        .context("GitHub returned an invalid sign-in response; try again")?;
    if let Some(code) = value.get("error").and_then(|error| error.as_str()) {
        bail!("{}", oauth_error(code));
    }
    if !status.is_success() {
        bail!(
            "GitHub sign-in service returned HTTP {}; try again",
            status.as_u16()
        );
    }
    serde_json::from_value(value).context("GitHub returned an invalid sign-in response; try again")
}
fn client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .user_agent("Moon shared web")
        .timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none())
        .build()?)
}
#[derive(Deserialize)]
struct Identity {
    id: u64,
    login: String,
    name: Option<String>,
}
async fn verify(token: String) -> Result<Account> {
    if token.trim().is_empty() || token.len() > 4096 {
        bail!("GitHub returned an invalid access token; start sign-in again");
    }
    let response = client()?
        .get("https://api.github.com/user")
        .bearer_auth(&token)
        .send()
        .await
        .context("Could not reach GitHub; check the server network connection and try again")?;
    if !response.status().is_success() {
        bail!("GitHub could not verify your sign-in; start again");
    }
    let identity: Identity = response
        .json()
        .await
        .context("GitHub returned an invalid identity")?;
    if identity.id == 0 || identity.login.is_empty() {
        bail!("GitHub returned an invalid identity");
    }
    // Numeric-id noreply addresses remain tied to the verified account, even after a rename.
    let email = format!(
        "{}+{}@users.noreply.github.com",
        identity.id, identity.login
    );
    Ok(Account {
        id: format!("github:{}", identity.id),
        name: identity
            .name
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| identity.login.clone()),
        login: identity.login,
        email,
        token,
        unix_user: None,
        layout: None,
        windows: Default::default(),
    })
}
#[derive(Deserialize)]
struct DeviceReply {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in: u64,
    interval: u64,
}
pub(in crate::server) struct Pending {
    pub user: String,
    pub generation: u64,
    code: String,
    client_id: String,
    pub expires: Instant,
    next: Instant,
    interval: Duration,
}
pub(in crate::server) async fn begin(
    user: String,
    generation: u64,
) -> Result<(DeviceView, Pending)> {
    let client_id =
        client_id().context("GitHub sign-in is not configured. Set MOON_GITHUB_CLIENT_ID to your GitHub OAuth app Client ID and restart the server.")?;
    if client_id == "your_client_id" {
        bail!(
            "Replace your_client_id in MOON_GITHUB_CLIENT_ID with your GitHub OAuth app Client ID and restart the server."
        );
    }
    let response = client()?
        .post("https://github.com/login/device/code")
        .header("Accept", "application/json")
        .form(&[("client_id", client_id.as_str()), ("scope", "repo")])
        .send()
        .await
        .context("Could not reach GitHub; check the server network connection and try again")?;
    let status = response.status();
    let body = response
        .bytes()
        .await
        .context("Could not read GitHub sign-in response; try again")?;
    let reply: DeviceReply = decode_reply(status, &body)?;
    if reply.device_code.is_empty()
        || reply.user_code.is_empty()
        || reply.verification_uri != "https://github.com/login/device"
        || reply.expires_in == 0
    {
        bail!("GitHub returned an invalid sign-in response; try again");
    }
    let interval = Duration::from_secs(reply.interval.max(5));
    let now = Instant::now();
    let pending = Pending {
        user,
        generation,
        code: reply.device_code,
        client_id,
        expires: now + Duration::from_secs(reply.expires_in.min(3600)),
        next: now + interval,
        interval,
    };
    Ok((
        DeviceView {
            flow_id: crate::moontasks::store::new_uuid(),
            user_code: reply.user_code,
            verification_uri: reply.verification_uri,
            expires_in: reply.expires_in,
            interval: interval.as_secs(),
        },
        pending,
    ))
}
#[derive(Deserialize)]
struct TokenReply {
    access_token: Option<String>,
    error: Option<String>,
}
fn decode_token_reply(status: reqwest::StatusCode, body: &[u8]) -> Result<TokenReply> {
    let value: serde_json::Value = serde_json::from_slice(body)
        .context("GitHub returned an invalid sign-in response; try again")?;
    let waiting = status.is_success()
        && matches!(
            value.get("error").and_then(|e| e.as_str()),
            Some("authorization_pending" | "slow_down")
        );
    if waiting {
        serde_json::from_value(value)
            .context("GitHub returned an invalid sign-in response; try again")
    } else {
        decode_reply(status, body)
    }
}
pub(in crate::server) enum Poll {
    Waiting(Pending),
    Connected(Account),
}
pub(in crate::server) async fn poll(mut pending: Pending) -> Result<Poll> {
    let now = Instant::now();
    if now >= pending.expires {
        bail!("GitHub sign-in expired; start again");
    }
    if now < pending.next {
        return Ok(Poll::Waiting(pending));
    }
    let response = client()?
        .post("https://github.com/login/oauth/access_token")
        .header("Accept", "application/json")
        .form(&[
            ("client_id", pending.client_id.as_str()),
            ("device_code", pending.code.as_str()),
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
        ])
        .send()
        .await
        .context("Could not reach GitHub; check the server network connection and try again")?;
    let status = response.status();
    let body = response
        .bytes()
        .await
        .context("Could not read GitHub sign-in response; try again")?;
    let reply = decode_token_reply(status, &body)?;
    if let Some(token) = reply.access_token {
        return Ok(Poll::Connected(verify(token).await?));
    }
    match reply.error.as_deref() {
        Some("authorization_pending") => (),
        Some("slow_down") => pending.interval += Duration::from_secs(5),
        _ => bail!("GitHub returned an invalid sign-in response; start again"),
    }
    pending.next = Instant::now() + pending.interval;
    Ok(Poll::Waiting(pending))
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::StatusCode;
    #[test]
    fn client_id_ignores_surrounding_whitespace() {
        assert_eq!(
            normalized_client_id("  Iv1.real\n").as_deref(),
            Some("Iv1.real")
        );
        assert_eq!(normalized_client_id(" \n"), None);
    }
    #[test]
    fn configuration_errors_are_actionable_even_on_http_success() {
        for status in [StatusCode::OK, StatusCode::BAD_REQUEST] {
            for (code, expected) in [
                ("incorrect_client_credentials", "MOON_GITHUB_CLIENT_ID"),
                ("device_flow_disabled", "Enable Device Flow"),
            ] {
                let body = serde_json::to_vec(&serde_json::json!({"error":code,"error_description":"private response","access_token":"private token"})).unwrap();
                let error = decode_reply::<DeviceReply>(status, &body)
                    .err()
                    .unwrap()
                    .to_string();
                assert!(error.contains(expected));
                assert!(!error.contains("private"));
                let error = decode_token_reply(status, &body).err().unwrap().to_string();
                assert!(error.contains(expected));
                assert!(!error.contains("private"));
            }
        }
    }
    #[test]
    fn unknown_and_malformed_errors_do_not_disclose_response_contents() {
        for body in [
            b"private token".as_slice(),
            br#"{"error":"private token","error_description":"private response"}"#,
            br#"{"device_code":"private token"}"#,
        ] {
            let error = decode_reply::<DeviceReply>(StatusCode::OK, body)
                .err()
                .unwrap()
                .to_string();
            assert!(!error.contains("private"));
        }
        let error = decode_token_reply(
            StatusCode::SERVICE_UNAVAILABLE,
            br#"{"access_token":"private token"}"#,
        )
        .err()
        .unwrap()
        .to_string();
        assert!(error.contains("503"));
        assert!(!error.contains("private"));
    }
    #[test]
    fn polling_preserves_waiting_and_returns_specific_terminal_errors() {
        for code in ["authorization_pending", "slow_down"] {
            let body = serde_json::to_vec(&serde_json::json!({"error":code})).unwrap();
            assert_eq!(
                decode_token_reply(StatusCode::OK, &body)
                    .unwrap()
                    .error
                    .as_deref(),
                Some(code)
            );
            assert!(decode_token_reply(StatusCode::BAD_REQUEST, &body).is_err());
        }
        for (code, expected) in [
            ("access_denied", "denied"),
            ("expired_token", "expired"),
            ("incorrect_device_code", "start again"),
        ] {
            let body = serde_json::to_vec(&serde_json::json!({"error":code})).unwrap();
            assert!(
                decode_token_reply(StatusCode::OK, &body)
                    .err()
                    .unwrap()
                    .to_string()
                    .contains(expected)
            );
        }
        assert_eq!(
            decode_token_reply(StatusCode::OK, br#"{"access_token":"verified internally"}"#)
                .unwrap()
                .access_token
                .as_deref(),
            Some("verified internally")
        );
    }
}
