//! Where a remote backend's server is: the address it was given turned into the URLs its
//! requests and sockets go to. The same whichever transport carries them.

use anyhow::{Result, bail};

pub(super) fn base_url_for(target: &str) -> Result<String> {
    let target = target.trim().trim_end_matches('/');
    if target.is_empty() {
        bail!("remote server address must not be empty");
    }
    if target.starts_with("http://") || target.starts_with("https://") {
        return Ok(target.to_string());
    }
    if target.contains("://") {
        bail!("remote server address must be http:// or https://, got {target}");
    }
    // A bare `host` or `host:port` is the common case over an SSH tunnel.
    if target.contains(':') {
        Ok(format!("http://{target}"))
    } else {
        Ok(format!("http://{target}:{}", crate::api::DEFAULT_PORT))
    }
}

pub(super) fn label_for(base_url: &str) -> String {
    base_url
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .to_string()
}

pub(super) fn websocket_url(base_url: &str, path: &str) -> String {
    let socket_base = if let Some(rest) = base_url.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = base_url.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        base_url.to_string()
    };
    format!("{socket_base}{path}")
}

pub(crate) fn urlencode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(*byte as char);
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_host_becomes_an_http_url_on_the_default_port() {
        assert_eq!(
            base_url_for("dev-box").expect("expected a URL"),
            format!("http://dev-box:{}", crate::api::DEFAULT_PORT)
        );
    }

    #[test]
    fn host_and_port_shorthand_keeps_the_port() {
        assert_eq!(
            base_url_for("127.0.0.1:9000").expect("expected a URL"),
            "http://127.0.0.1:9000"
        );
    }

    #[test]
    fn explicit_urls_are_kept_and_trailing_slashes_dropped() {
        assert_eq!(
            base_url_for("https://review.example.com/").expect("expected a URL"),
            "https://review.example.com"
        );
    }

    #[test]
    fn non_http_schemes_are_rejected() {
        let error = base_url_for("ssh://dev-box").expect_err("expected ssh:// to be rejected");
        assert!(error.to_string().contains("must be http:// or https://"));
    }

    #[test]
    fn websocket_urls_follow_the_http_scheme() {
        assert_eq!(
            websocket_url("http://dev-box:42000", "/socket"),
            "ws://dev-box:42000/socket"
        );
        assert_eq!(
            websocket_url("https://dev-box", "/socket"),
            "wss://dev-box/socket"
        );
    }

    #[test]
    fn file_paths_with_spaces_and_unicode_are_encoded() {
        assert_eq!(urlencode("src/a b/é.rs"), "src%2Fa%20b%2F%C3%A9.rs");
    }
}
