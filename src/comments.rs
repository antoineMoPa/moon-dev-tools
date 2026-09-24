//! Review comments, as the anchored blocks a hunk's comment is written in, and what the server
//! does with them: the agents a comment is sent to, and the export of all of them.

// The agents are the server's: the window in a browser only reads and writes the blocks.
#[cfg(not(target_arch = "wasm32"))]
mod server_side;

#[cfg(not(target_arch = "wasm32"))]
pub(crate) use server_side::*;

const ANCHOR_OPEN: &str = "[[mr-anchor]]";
const SELECTION_MARK: &str = "[[selection]]";
const RESOLVED_MARK: &str = "[[resolved]]";
const COMMENT_MARK: &str = "[[comment]]";
const ANCHOR_CLOSE: &str = "[[/mr-anchor]]";

pub(crate) struct AnchoredComment {
    pub(crate) selection: String,
    pub(crate) comment: String,
    pub(crate) resolved: bool,
}

pub(crate) fn build_anchored_comment_value(comments: &[AnchoredComment]) -> String {
    comments
        .iter()
        .map(|entry| {
            let mut lines = vec![
                ANCHOR_OPEN.to_string(),
                SELECTION_MARK.to_string(),
                entry.selection.trim().to_string(),
            ];
            if entry.resolved {
                lines.push(RESOLVED_MARK.to_string());
            }
            lines.push(COMMENT_MARK.to_string());
            lines.push(entry.comment.trim().to_string());
            lines.push(ANCHOR_CLOSE.to_string());
            lines.join("\n")
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub(crate) fn parse_anchored_comments(value: &str) -> Vec<AnchoredComment> {
    let mut comments = Vec::new();
    let mut remaining = value;

    while let Some(start) = remaining.find(ANCHOR_OPEN) {
        let after_open = &remaining[start + ANCHOR_OPEN.len()..];
        let Some(selection_start) = after_open.find(SELECTION_MARK) else {
            break;
        };
        let after_selection_mark = &after_open[selection_start + SELECTION_MARK.len()..];
        let Some(comment_start) = after_selection_mark.find(COMMENT_MARK) else {
            break;
        };
        let selection = after_selection_mark[..comment_start].trim();
        let resolved = after_selection_mark
            .find(RESOLVED_MARK)
            .is_some_and(|index| index < comment_start);
        let after_comment_mark = &after_selection_mark[comment_start + COMMENT_MARK.len()..];
        let Some(close_start) = after_comment_mark.find(ANCHOR_CLOSE) else {
            break;
        };
        let comment = after_comment_mark[..close_start].trim();

        if !selection.is_empty() || !comment.is_empty() {
            comments.push(AnchoredComment {
                selection: selection.to_string(),
                comment: comment.to_string(),
                resolved,
            });
        }

        remaining = &after_comment_mark[close_start + ANCHOR_CLOSE.len()..];
    }

    comments
}
