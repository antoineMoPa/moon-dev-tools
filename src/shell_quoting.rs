//! Shell quoting - writes a string so that a shell reads it back as itself: a command printed
//! by a script, a path in a line of shell, each argument of a program `moon launch` starts.

/// A string a shell reads back as itself: single quotes hold every character literally, and
/// the one they cannot hold is closed, escaped, and reopened.
pub(crate) fn single_quoted(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quote_of_its_own_is_closed_escaped_and_reopened() {
        assert_eq!(single_quoted("it's"), "'it'\\''s'");
    }
}
