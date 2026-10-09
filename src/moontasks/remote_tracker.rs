//! Remote tracker - the link from a task to its issue in a tracker kept elsewhere - Linear,
//! Jira, GitHub, GitLab - and the short id a card shows that link as.
//!
//! The link is `remote_task_tracker_url` in the task's `metadata.json`: the issue's address,
//! as the browser has it. The card writes the issue's id - `BM-3343`, `moon#12` - and a click
//! opens the address.

/// The schemes a tracker link may be written with. A click on the card hands the link to the
/// machine's browser, so nothing that is not a web address is kept as one.
const WEB_SCHEMES: [&str; 2] = ["https://", "http://"];

/// Whether this is an address a browser opens.
pub(crate) fn is_a_web_link(url: &str) -> bool {
    WEB_SCHEMES.iter().any(|scheme| url.starts_with(scheme))
}

/// The words an address puts in front of an issue's number where issues are numbered inside
/// a repo: GitHub's `…/moon/issues/12`, GitLab's `…/moon/-/issues/12` and
/// `…/moon/-/work_items/12`.
const NUMBERED_UNDER: [&str; 2] = ["issues", "work_items"];

/// What GitLab puts between a repo's name and what is in the repo, which is no part of
/// either.
const GITLAB_SEPARATOR: &str = "-";

/// What a card writes for the link: the issue's id where the link has one, and otherwise the
/// link without its scheme - a link to a tracker that names its issues some other way is
/// still a link to open.
pub(crate) fn label_of(url: &str) -> String {
    issue_id_in(url).unwrap_or_else(|| after_the_scheme(url).to_string())
}

/// The issue's id, as its tracker writes one: `moon#12` for an issue numbered inside a repo,
/// and `BM-3343` for one with a key of its own.
///
/// Looked for after the host, which is no part of the issue. The numbered kind is looked
/// for first: a repo may be called anything, `MOON-2` included.
pub(crate) fn issue_id_in(url: &str) -> Option<String> {
    let (_host, after_the_host) = after_the_scheme(url).split_once('/')?;
    numbered_issue_in(after_the_host).or_else(|| keyed_issue_in(after_the_host).map(str::to_string))
}

/// An issue GitHub or GitLab numbers inside a repo, as both write one elsewhere: the repo's
/// name, a `#`, and the number - `moon#12` in `https://github.com/acme/moon/issues/12` and
/// in `https://gitlab.com/acme/tools/moon/-/issues/12`.
fn numbered_issue_in(after_the_host: &str) -> Option<String> {
    let path = after_the_host
        .split(['?', '#'])
        .next()
        .expect("a split has a first piece");
    let parts: Vec<&str> = path.split('/').collect();
    let at = parts
        .windows(2)
        .position(|pair| NUMBERED_UNDER.contains(&pair[0]) && is_a_number(pair[1]))?;
    let repo = parts[..at]
        .iter()
        .rev()
        .find(|part| **part != GITLAB_SEPARATOR)?;
    Some(format!("{repo}#{}", parts[at + 1]))
}

/// An issue with a key of its own, as Linear and Jira both write one: a team's or project's
/// key in capitals, a dash, and the issue's number - `BM-3343` in
/// `https://linear.app/acme/issue/BM-3343/a-title`, `LU-323` in
/// `https://acme.atlassian.net/browse/LU-323` and in `…/boards/7?selectedIssue=LU-323`.
///
/// Looked for among the words the address's own punctuation sets apart: the first of them
/// shaped like a key is it.
fn keyed_issue_in(after_the_host: &str) -> Option<&str> {
    after_the_host
        .split(|character: char| !(character.is_ascii_alphanumeric() || "-_".contains(character)))
        .find(|word| is_a_keyed_issue(word))
}

fn is_a_keyed_issue(word: &str) -> bool {
    let Some((key, number)) = word.split_once('-') else {
        return false;
    };
    key.starts_with(|character: char| character.is_ascii_uppercase())
        && key.chars().all(|character| {
            character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_'
        })
        && is_a_number(number)
}

fn is_a_number(word: &str) -> bool {
    !word.is_empty() && word.chars().all(|character| character.is_ascii_digit())
}

fn after_the_scheme(url: &str) -> &str {
    WEB_SCHEMES
        .iter()
        .find_map(|scheme| url.strip_prefix(scheme))
        .unwrap_or(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_card_writes_the_issue_id_its_tracker_writes() {
        for (url, label) in [
            (
                "https://linear.app/acme/issue/BM-3343/remote-tracker-link-field",
                "BM-3343",
            ),
            ("https://linear.app/acme/issue/BM-3343", "BM-3343"),
            ("https://acme.atlassian.net/browse/LU-323", "LU-323"),
            (
                "https://acme.atlassian.net/jira/software/projects/LU/boards/7?selectedIssue=LU-323",
                "LU-323",
            ),
            (
                "https://jira.acme.com/browse/R2D2-7?focusedCommentId=10",
                "R2D2-7",
            ),
            ("https://github.com/acme/moon/issues/12", "moon#12"),
            (
                "https://github.com/acme/moon/issues/12#issuecomment-99",
                "moon#12",
            ),
            ("https://gitlab.com/acme/tools/moon/-/issues/42", "moon#42"),
            ("https://gitlab.acme.com/acme/moon/-/work_items/7", "moon#7"),
            // A repo called like a key is still the repo.
            ("https://github.com/acme/MOON-2/issues/5", "MOON-2#5"),
            // No issue of either kind: the link itself, without its scheme.
            (
                "https://github.com/acme/moon/issues",
                "github.com/acme/moon/issues",
            ),
            // A host is not where an issue is named.
            ("https://JIRA-1.acme.com/", "JIRA-1.acme.com/"),
        ] {
            assert_eq!(label_of(url), label, "{url}");
        }
    }

    #[test]
    fn only_a_web_address_is_a_tracker_link() {
        assert!(is_a_web_link("https://linear.app/acme/issue/BM-3343"));
        assert!(is_a_web_link("http://jira.local/browse/LU-323"));
        assert!(!is_a_web_link("file:///etc/passwd"));
        assert!(!is_a_web_link("BM-3343"));
    }
}
