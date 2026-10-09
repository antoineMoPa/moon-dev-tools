//! Profiles - public personal account and automatically saved layout, never credentials.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Layout {
    pub repo: Option<String>,
    pub frame: String,
    pub layout: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct ProfileView {
    pub profile_namespace: String,
    pub id: Option<String>,
    pub login: Option<String>,
    /// The Unix user this person's shells and agents run as, on a server that gives each
    /// person one.
    #[serde(default)]
    pub unix_user: Option<String>,
    /// Whether this server lets nobody past the Account pane before they sign in.
    #[serde(default)]
    pub sign_in_required: bool,
    pub device_flow: bool,
    pub layout: Option<Layout>,
}
impl ProfileView {
    /// Whether the Account pane's sign-in is all a window may show: the server lets nobody
    /// further before they sign in, and whoever this is has not. Everything else it could ask
    /// for would be refused.
    pub(crate) fn only_sign_in_is_offered(&self) -> bool {
        self.sign_in_required && self.login.is_none()
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct DeviceView {
    pub flow_id: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
}

/// The server supplies a home repository only when the entry URL has none.
#[cfg(any(target_arch = "wasm32", test))]
pub(crate) fn entry_repo(
    saved: Option<&Layout>,
    repo: Option<String>,
    default_repo: bool,
) -> Option<String> {
    if default_repo && saved.is_some_and(|s| s.repo.is_some()) {
        None
    } else {
        repo
    }
}

/// Explicit entry links take precedence; panes are restored only for the same repository
/// and frame. A bare entry resumes the person's saved arrangement.
#[cfg(any(target_arch = "wasm32", test))]
pub(crate) fn entry_layout(
    saved: Option<&Layout>,
    repo: Option<String>,
    frame: Option<String>,
) -> Layout {
    let compatible = saved.filter(|s| {
        repo.as_ref().is_none_or(|r| s.repo.as_ref() == Some(r))
            && frame.as_ref().is_none_or(|f| &s.frame == f)
    });
    Layout {
        repo: repo.or_else(|| saved.and_then(|s| s.repo.clone())),
        frame: frame
            .or_else(|| saved.map(|s| s.frame.clone()))
            .unwrap_or_else(|| "tasks".into()),
        layout: compatible.and_then(|s| s.layout.clone()),
    }
}
#[cfg(any(target_arch = "wasm32", test))]
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
#[derive(Clone)]
pub(crate) enum Action {
    Refresh,
    Device,
    Poll(String),
    Disconnect,
    Save,
}
#[cfg(any(target_arch = "wasm32", test))]
pub(crate) fn enqueue(queue: &mut std::collections::VecDeque<Action>, action: Action) {
    if matches!(action, Action::Save) && queue.iter().any(|a| matches!(a, Action::Save)) {
        return;
    }
    queue.push_back(action);
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn entry_links_preserve_destination_and_never_restore_other_repository_panes() {
        let saved = Layout {
            repo: Some("/saved".into()),
            frame: "tasks".into(),
            layout: Some("four spaces".into()),
        };
        assert_eq!(entry_layout(Some(&saved), None, None), saved);
        let other = entry_layout(Some(&saved), Some("/explicit".into()), None);
        assert_eq!(other.repo.as_deref(), Some("/explicit"));
        assert!(other.layout.is_none());
        assert!(
            entry_layout(Some(&saved), None, Some("review".into()))
                .layout
                .is_none()
        );
        assert_eq!(
            entry_layout(Some(&saved), Some("/saved".into()), None).layout,
            saved.layout
        );
    }
    #[test]
    fn injected_home_is_fallback_for_new_users_and_saved_layout_resumes_across_browsers() {
        let home = Some("/home".into());
        let placeholder = Layout {
            repo: None,
            frame: "tasks".into(),
            layout: None,
        };
        assert_eq!(entry_repo(Some(&placeholder), home.clone(), true), home);
        assert_eq!(
            entry_layout(None, entry_repo(None, home.clone(), true), None).repo,
            home
        );
        let saved = Layout {
            repo: Some("/personal".into()),
            frame: "tasks".into(),
            layout: Some("all four spaces".into()),
        };
        assert_eq!(
            entry_layout(
                Some(&saved),
                entry_repo(Some(&saved), home.clone(), true),
                None
            ),
            saved
        );
        let explicit = entry_layout(
            Some(&saved),
            entry_repo(Some(&saved), home.clone(), false),
            None,
        );
        assert_eq!(explicit.repo, home);
        assert!(explicit.layout.is_none());
    }
    #[test]
    fn only_the_sign_in_is_offered_until_someone_signs_in_where_that_is_required() {
        let seen_by = |sign_in_required: bool, login: Option<&str>| ProfileView {
            profile_namespace: "browser:1".into(),
            id: login.map(|_| "42".into()),
            login: login.map(Into::into),
            unix_user: login.filter(|_| sign_in_required).map(|login| format!("moon-{login}")),
            sign_in_required,
            device_flow: true,
            layout: None,
        };
        assert!(seen_by(true, None).only_sign_in_is_offered());
        assert!(!seen_by(true, Some("alice")).only_sign_in_is_offered());
        // A server that runs everyone as itself is used without an account, as it always was.
        assert!(!seen_by(false, None).only_sign_in_is_offered());
        assert!(!seen_by(false, Some("alice")).only_sign_in_is_offered());
    }
    #[test]
    fn autosaves_coalesce_without_overtaking_identity_changes() {
        let mut queue = std::collections::VecDeque::new();
        enqueue(&mut queue, Action::Save);
        enqueue(&mut queue, Action::Disconnect);
        enqueue(&mut queue, Action::Save);
        assert!(matches!(queue.pop_front(), Some(Action::Save)));
        assert!(matches!(queue.pop_front(), Some(Action::Disconnect)));
        assert!(queue.is_empty());
    }
}
