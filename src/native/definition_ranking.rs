//! Reading a line the repo search turned up as the definition of a name, rather than as one
//! more mention of it.
//!
//! `ag` finds every line holding the name: the one that declares it, and the two hundred that
//! call it. Telling them apart is a guess made from how the line starts - `fn greet(` declares
//! and `greet(name)` calls - so the guess is a table per language rather than a chain of
//! conditions, and adding a language is a row in it.
//!
//! Every hit comes back scored rather than filtered. A mention is a worse answer than a
//! definition and a far better one than nothing, and a file in a language with no row here has
//! only mentions to offer. What is done with the order is
//! [`crate::native::definition`]'s business - this only reads the lines.

use crate::api::ContentMatch;

/// What a definition of a name starts with, in each language a repo here is written in.
///
/// Keyed by the extension of the file a *hit* is in rather than of the file that was clicked
/// in: a ⌘-click in a `.rs` file can perfectly well land in a `.ts` one, and the line has to
/// be read in the language it was written in.
///
/// A trailing space is part of each of these on purpose - `fn` in prose is not a definition,
/// and `fnord` is not one either.
const DEFINITION_KEYWORDS: &[(&str, &[&str])] = &[
    ("rs", RUST),
    ("ts", TYPESCRIPT),
    ("tsx", TYPESCRIPT),
    ("js", TYPESCRIPT),
    ("jsx", TYPESCRIPT),
    ("mjs", TYPESCRIPT),
    ("py", PYTHON),
    ("go", GO),
    ("rb", RUBY),
    ("java", JAVA),
    ("kt", KOTLIN),
    ("swift", SWIFT),
    ("c", C),
    ("h", C),
    ("cc", CPP),
    ("cpp", CPP),
    ("hpp", CPP),
    ("cs", CSHARP),
    ("php", PHP),
    ("sh", SHELL),
    ("bash", SHELL),
    ("zsh", SHELL),
    ("sql", SQL),
    ("css", CSS),
    ("scss", CSS),
];

const RUST: &[&str] = &[
    "fn ",
    "struct ",
    "enum ",
    "trait ",
    "impl ",
    "type ",
    "const ",
    "static ",
    "mod ",
    "union ",
    "macro_rules! ",
];
const TYPESCRIPT: &[&str] = &[
    "function ",
    "class ",
    "interface ",
    "type ",
    "enum ",
    "const ",
    "let ",
    "var ",
    "namespace ",
];
const PYTHON: &[&str] = &["def ", "class "];
const GO: &[&str] = &["func ", "type ", "var ", "const ", "package "];
const RUBY: &[&str] = &["def ", "class ", "module "];
const JAVA: &[&str] = &["class ", "interface ", "enum ", "record ", "void "];
const KOTLIN: &[&str] = &["fun ", "class ", "interface ", "object ", "val ", "var "];
const SWIFT: &[&str] = &[
    "func ",
    "class ",
    "struct ",
    "enum ",
    "protocol ",
    "extension ",
    "let ",
    "var ",
];
const C: &[&str] = &["struct ", "enum ", "union ", "typedef ", "#define "];
const CPP: &[&str] = &[
    "struct ",
    "enum ",
    "union ",
    "class ",
    "typedef ",
    "namespace ",
    "#define ",
];
const CSHARP: &[&str] = &["class ", "interface ", "enum ", "struct ", "record ", "void "];
const PHP: &[&str] = &["function ", "class ", "interface ", "trait ", "const "];
const SHELL: &[&str] = &["function "];
const SQL: &[&str] = &["table ", "view ", "function ", "procedure ", "index "];
const CSS: &[&str] = &["@mixin ", "@keyframes ", "--"];

/// What a line reading like a definition of the name is worth. Far more than anything else
/// scored here, because it is the only signal that answers the question actually being asked -
/// every other hit is a mention, and a mention is where you already were.
const DEFINES: i32 = 100;
/// What spelling the name the same way is worth. The search runs `--ignore-case`, so looking
/// up `Config` also turns up `config`; the one that is spelled the way it was clicked is the
/// likelier answer, but only enough to break a tie between two mentions or two definitions.
const SPELLED_THE_SAME: i32 = 10;

/// A hit and how much it reads like the definition of the name, rather than a mention of it.
pub(super) struct Ranked {
    pub(super) score: i32,
    pub(super) found: ContentMatch,
}

impl Ranked {
    /// Whether this hit reads as the definition of the name rather than as a mention of it,
    /// which is the one thing a caller needs of the score - what the numbers are worth is
    /// this module's business.
    pub(super) fn reads_as_a_definition(&self) -> bool {
        self.score >= DEFINES
    }
}

/// Every hit, best first. A hit that reads like nothing in particular still comes back: a
/// mention is a worse answer than a definition and a much better one than nothing, and a file
/// in a language with no table here has only mentions to offer.
pub(super) fn rank(word: &str, matches: Vec<ContentMatch>) -> Vec<Ranked> {
    let mut ranked: Vec<Ranked> = matches
        .into_iter()
        .map(|found| Ranked {
            score: score(word, &found),
            found,
        })
        .collect();
    // Stable, so hits that score the same stay in the order the search walked the repo in.
    ranked.sort_by_key(|ranked| std::cmp::Reverse(ranked.score));
    ranked
}

fn score(word: &str, found: &ContentMatch) -> i32 {
    let defines = match defines(word, found) {
        true => DEFINES,
        false => 0,
    };
    let spelling = match found.line.contains(word) {
        true => SPELLED_THE_SAME,
        false => 0,
    };
    defines + spelling
}

/// Whether the line reads as a definition of the name: one of the words its language starts a
/// definition with, and the name itself straight after it.
///
/// Case is ignored here and paid for separately, so the definition of `greet` still wins when
/// `Greet` was what was clicked.
fn defines(word: &str, found: &ContentMatch) -> bool {
    let line = &found.line;
    keywords_of(&found.file_path).iter().any(|keyword| {
        line.match_indices(keyword).any(|(at, _)| {
            let rest = &line[at + keyword.len()..];
            rest.get(..word.len())
                .is_some_and(|named| named.eq_ignore_ascii_case(word))
                // `fn greeting` is not the definition of `greet`.
                && !rest[word.len()..].starts_with(|next: char| next.is_alphanumeric() || next == '_')
        })
    })
}

/// What a definition looks like in the file this hit is in. A file whose extension is in no
/// table has nothing to score by, and every hit in it is read as a plain mention.
fn keywords_of(file_path: &str) -> &'static [&'static str] {
    let Some(extension) = std::path::Path::new(file_path).extension() else {
        return &[];
    };
    let extension = extension.to_string_lossy().to_lowercase();
    DEFINITION_KEYWORDS
        .iter()
        .find(|(known, _)| *known == extension)
        .map_or(&[], |(_, keywords)| *keywords)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(file_path: &str, line: &str) -> ContentMatch {
        ContentMatch {
            file_path: file_path.to_string(),
            line_number: 1,
            line: line.to_string(),
        }
    }

    /// The whole point of the ranking: the line that declares the name comes before the two
    /// hundred lines that call it.
    #[test]
    fn a_line_that_declares_the_name_beats_a_line_that_calls_it() {
        let ranked = rank(
            "greet",
            vec![
                found("src/main.rs", "greet(name);"),
                found("src/lib.rs", "pub fn greet(name: &str) -> String {"),
            ],
        );

        assert_eq!(ranked[0].found.file_path, "src/lib.rs");
        assert!(ranked[0].score > ranked[1].score);
    }

    /// A hit is read in the language of the file it is in, not of the file that was clicked.
    #[test]
    fn a_hit_is_read_in_the_language_of_the_file_it_is_in() {
        assert!(defines("Session", &found("src/api.ts", "interface Session {")));
        // `interface` declares nothing in rust, and the rust table does not hold it.
        assert!(!defines("Session", &found("src/api.rs", "interface Session {")));
        assert!(defines("Session", &found("src/api.rs", "pub struct Session {")));
        // Nor does the typescript table hold rust's words.
        assert!(!defines("Session", &found("src/api.ts", "pub struct Session {")));
    }

    /// The search ignores case, so both spellings come back; the one spelled the way it was
    /// clicked is the likelier answer.
    #[test]
    fn a_hit_spelled_the_way_it_was_clicked_beats_one_that_only_matches_ignoring_case() {
        let ranked = rank(
            "Config",
            vec![
                found("src/one.rs", "let config = load();"),
                found("src/two.rs", "let Config = load();"),
            ],
        );

        assert_eq!(ranked[0].found.file_path, "src/two.rs");
        assert!(ranked[0].score > ranked[1].score);
    }

    /// A file in a language with no table still has its hits offered - they are mentions, and
    /// a mention is a much better answer than nothing.
    #[test]
    fn a_hit_in_a_file_of_an_unknown_language_still_comes_back() {
        let ranked = rank(
            "widget",
            vec![found("notes/design.txt", "the widget is drawn last")],
        );

        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].score, SPELLED_THE_SAME);
    }

    /// A name that only starts another name is not a definition of it.
    #[test]
    fn a_longer_name_that_starts_with_the_word_is_not_a_definition_of_it() {
        assert!(!defines("greet", &found("src/lib.rs", "fn greeting() {}")));
        assert!(defines("greet", &found("src/lib.rs", "fn greet() {}")));
    }
}
