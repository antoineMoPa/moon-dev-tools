//! Handles: what a task is tagged by on the wire - `@fix-the-races`.
//!
//! A card's folder is named `<slug>-<uuid>`: the front of its title, and the id that makes it
//! the only folder of that name - see `create_task` in the store. A tag is the front of that
//! name, and it has to reach at least the end of the slug: the slug alone, or the slug, a dash
//! and however much of the uuid follows. A task's handle is the shortest such tag that names
//! it and no other task of the board - the slug, while no other card has it.
//!
//! A folder whose name ends in no uuid - the board's own `board-task`, or one made by hand -
//! is tagged by its whole name and nothing shorter.
//!
//! The other tasks are every task folder of the board, whether or not an agent is running in
//! it, so a handle does not change as agents start and stop. It can still change: a card made
//! later with the same title takes the bare slug away from the first.
//!
//! Everything here reads a list of folder names and nothing else, which is
//! `store::list_task_ids`.

/// How long the uuid a card's folder name ends in is: `6f9c1e2a-0b1c-4d2e-8f3a-9b8c7d6e5f4a`.
const UUID_LENGTH: usize = 36;

/// Where the dashes of a uuid are.
const UUID_DASHES: [usize; 4] = [8, 13, 18, 23];

/// How much of the uuid a handle starts with, once the slug alone is not enough. One
/// character would do for two cards of a name, and reads as a typo: `bing-bong-3`.
const SHORTEST_UUID_START: usize = 3;

/// A tag that names no one task of the board.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Unresolved {
    /// No task is tagged by it. Which tasks there are to tag instead is not said here: that
    /// is the ones with an agent running, which a list of folder names does not hold.
    Unknown { tag: String },
    /// Several tasks are. `handles` is theirs.
    Ambiguous { tag: String, handles: Vec<String> },
}

impl std::fmt::Display for Unresolved {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown { tag } => write!(f, "@{tag} is no task of this board"),
            Self::Ambiguous { tag, handles } => {
                let handles: Vec<String> =
                    handles.iter().map(|handle| format!("@{handle}")).collect();
                write!(f, "@{tag} is several tasks: {}", handles.join(", "))
            }
        }
    }
}

impl std::error::Error for Unresolved {}

/// The slug and the uuid of a card's folder name, or `None` for a name that ends in no uuid.
fn slug_and_uuid(folder: &str) -> Option<(&str, &str)> {
    let uuid_at = folder.len().checked_sub(UUID_LENGTH)?;
    // `get` rather than a slice: a name made by hand may have a wide character where the cut
    // falls, and that is a name with no uuid at its end.
    let (front, uuid) = (folder.get(..uuid_at)?, folder.get(uuid_at..)?);
    let slug = front.strip_suffix('-')?;
    (!slug.is_empty() && is_uuid(uuid)).then_some((slug, uuid))
}

fn is_uuid(text: &str) -> bool {
    text.len() == UUID_LENGTH
        && text.bytes().enumerate().all(|(at, byte)| {
            if UUID_DASHES.contains(&at) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
}

/// Whether a tag reads as this folder: the front of its name, reaching at least the end of
/// its slug - and the whole name, for a folder with no uuid to cut short.
fn tags(tag: &str, folder: &str) -> bool {
    match slug_and_uuid(folder) {
        Some((slug, _)) => folder.starts_with(tag) && tag.len() >= slug.len(),
        None => tag == folder,
    }
}

/// The task a tag names, out of every task folder of the board.
///
/// A tag that is a folder's whole name is that folder, whatever else it reads as: a card
/// titled `board task` has the slug `board-task`, and the board's own task, which has no
/// shorter name to go by, must not become unreachable for it.
pub(crate) fn resolve<'folder>(
    tag: &str,
    folders: &'folder [String],
) -> Result<&'folder str, Unresolved> {
    if let Some(named) = folders.iter().find(|folder| *folder == tag) {
        return Ok(named.as_str());
    }
    let tagged: Vec<&String> = folders.iter().filter(|folder| tags(tag, folder)).collect();
    match tagged.as_slice() {
        [one] => Ok(one.as_str()),
        [] => Err(Unresolved::Unknown {
            tag: tag.to_string(),
        }),
        several => Err(Unresolved::Ambiguous {
            tag: tag.to_string(),
            handles: several
                .iter()
                .map(|folder| handle_of(folder, folders))
                .collect(),
        }),
    }
}

/// A task's handle: the shortest tag that names it and no other folder of the board.
///
/// The slug, then the slug with more and more of the uuid, from [`SHORTEST_UUID_START`]
/// characters of it. The whole name is where that ends, and it always names the task - see
/// [`resolve`].
pub(crate) fn handle_of(folder: &str, folders: &[String]) -> String {
    let Some((slug, _)) = slug_and_uuid(folder) else {
        return folder.to_string();
    };
    let uuid_at = slug.len() + 1;
    std::iter::once(slug.len())
        .chain((SHORTEST_UUID_START..UUID_LENGTH).map(|of_the_uuid| uuid_at + of_the_uuid))
        .map(|length| &folder[..length])
        .find(|tag| {
            folders
                .iter()
                .all(|other| other == folder || !tags(tag, other))
        })
        .unwrap_or(folder)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const BING_BONG: &str = "bing-bong-313a1e2a-0b1c-4d2e-8f3a-9b8c7d6e5f4a";
    const OTHER_BING_BONG: &str = "bing-bong-9f0c1e2a-0b1c-4d2e-8f3a-9b8c7d6e5f4a";
    const FIX_THE_RACES: &str = "fix-the-races-6f9c1e2a-0b1c-4d2e-8f3a-9b8c7d6e5f4a";
    const BOARD_TASK: &str = "board-task";

    fn board(folders: &[&str]) -> Vec<String> {
        folders.iter().map(|folder| folder.to_string()).collect()
    }

    #[test]
    fn a_slug_no_other_card_has_is_the_handle() {
        let folders = board(&[BING_BONG, FIX_THE_RACES, BOARD_TASK]);

        assert_eq!(handle_of(BING_BONG, &folders), "bing-bong");
        assert_eq!(handle_of(FIX_THE_RACES, &folders), "fix-the-races");
        assert_eq!(resolve("bing-bong", &folders), Ok(BING_BONG));
        assert_eq!(resolve("fix-the-races", &folders), Ok(FIX_THE_RACES));
    }

    /// A tag may say more of the folder's name than the handle does, down to the whole of it.
    #[test]
    fn a_longer_start_of_the_folder_name_names_the_same_task() {
        let folders = board(&[BING_BONG, FIX_THE_RACES]);

        assert_eq!(resolve("bing-bong-3", &folders), Ok(BING_BONG));
        assert_eq!(resolve("bing-bong-313a1e2a-0b", &folders), Ok(BING_BONG));
        assert_eq!(resolve(BING_BONG, &folders), Ok(BING_BONG));
    }

    /// A tag has to reach the end of the slug: part of a title is not a handle.
    #[test]
    fn a_tag_shorter_than_the_slug_names_nothing() {
        let folders = board(&[BING_BONG, FIX_THE_RACES]);

        assert_eq!(
            resolve("bing", &folders),
            Err(Unresolved::Unknown {
                tag: "bing".to_string(),
            })
        );
        assert!(resolve("", &folders).is_err());
    }

    #[test]
    fn two_cards_of_one_title_are_told_apart_by_the_start_of_their_uuids() {
        let folders = board(&[BING_BONG, OTHER_BING_BONG, FIX_THE_RACES]);

        assert_eq!(handle_of(BING_BONG, &folders), "bing-bong-313");
        assert_eq!(handle_of(OTHER_BING_BONG, &folders), "bing-bong-9f0");
        assert_eq!(resolve("bing-bong-313", &folders), Ok(BING_BONG));
        assert_eq!(resolve("bing-bong-9f0", &folders), Ok(OTHER_BING_BONG));
        assert_eq!(
            resolve("bing-bong", &folders),
            Err(Unresolved::Ambiguous {
                tag: "bing-bong".to_string(),
                handles: vec!["bing-bong-313".to_string(), "bing-bong-9f0".to_string()],
            })
        );
    }

    /// Three characters of the uuid is where a handle starts, not where it has to stop.
    #[test]
    fn uuids_that_start_alike_take_as_much_as_tells_them_apart() {
        let twin = "bing-bong-313a7777-0b1c-4d2e-8f3a-9b8c7d6e5f4a";
        let folders = board(&[BING_BONG, twin]);

        assert_eq!(handle_of(BING_BONG, &folders), "bing-bong-313a1");
        assert_eq!(handle_of(twin, &folders), "bing-bong-313a7");
        assert_eq!(resolve("bing-bong-313a1", &folders), Ok(BING_BONG));
        assert!(matches!(
            resolve("bing-bong-313", &folders),
            Err(Unresolved::Ambiguous { .. })
        ));
    }

    /// `bing-bong-3` is one card's whole slug and, read the other way, another card's slug
    /// with the first character of its uuid.
    #[test]
    fn a_slug_that_reads_as_another_slug_and_the_start_of_its_uuid() {
        let bing_bong_3 = "bing-bong-3-77771e2a-0b1c-4d2e-8f3a-9b8c7d6e5f4a";
        let folders = board(&[BING_BONG, bing_bong_3]);

        // The shorter slug is the front of no other folder's slug, so it is enough.
        assert_eq!(handle_of(BING_BONG, &folders), "bing-bong");
        assert_eq!(resolve("bing-bong", &folders), Ok(BING_BONG));
        // The longer one tags both cards, so its card takes some of its uuid.
        assert!(matches!(
            resolve("bing-bong-3", &folders),
            Err(Unresolved::Ambiguous { .. })
        ));
        assert_eq!(handle_of(bing_bong_3, &folders), "bing-bong-3-777");
        assert_eq!(resolve("bing-bong-3-777", &folders), Ok(bing_bong_3));
        assert_eq!(resolve("bing-bong-313", &folders), Ok(BING_BONG));
    }

    #[test]
    fn a_folder_with_no_uuid_is_tagged_by_its_whole_name_only() {
        let by_hand = "write-the-parser-1111";
        let folders = board(&[BOARD_TASK, by_hand, BING_BONG]);

        assert_eq!(handle_of(BOARD_TASK, &folders), "board-task");
        assert_eq!(handle_of(by_hand, &folders), "write-the-parser-1111");
        assert_eq!(resolve("board-task", &folders), Ok(BOARD_TASK));
        assert_eq!(resolve("write-the-parser-1111", &folders), Ok(by_hand));
        assert!(matches!(
            resolve("write-the-parser", &folders),
            Err(Unresolved::Unknown { .. })
        ));
        assert!(matches!(
            resolve("board", &folders),
            Err(Unresolved::Unknown { .. })
        ));
    }

    /// A card titled `board task` has the board task's whole name as its slug. The name
    /// stays the board task's, and the card takes some of its uuid.
    #[test]
    fn a_card_whose_slug_is_another_folders_whole_name_leaves_it_the_name() {
        let card = "board-task-6f9c1e2a-0b1c-4d2e-8f3a-9b8c7d6e5f4a";
        let folders = board(&[BOARD_TASK, card]);

        assert_eq!(resolve("board-task", &folders), Ok(BOARD_TASK));
        assert_eq!(handle_of(card, &folders), "board-task-6f9");
        assert_eq!(resolve("board-task-6f9", &folders), Ok(card));
    }

    #[test]
    fn every_handle_names_the_task_it_is_the_handle_of() {
        let folders = board(&[
            BING_BONG,
            OTHER_BING_BONG,
            "bing-bong-3-77771e2a-0b1c-4d2e-8f3a-9b8c7d6e5f4a",
            "board-task-6f9c1e2a-0b1c-4d2e-8f3a-9b8c7d6e5f4a",
            BOARD_TASK,
            FIX_THE_RACES,
            "ünïcode-symbols-6f9c1e2a-0b1c-4d2e-8f3a-9b8c7d6e5f4a",
            "write-the-parser-1111",
        ]);

        for folder in &folders {
            let handle = handle_of(folder, &folders);
            assert_eq!(
                resolve(&handle, &folders),
                Ok(folder.as_str()),
                "@{handle} should name {folder}"
            );
        }
    }

    #[test]
    fn an_unresolved_tag_says_what_it_failed_to_name() {
        let folders = board(&[BING_BONG, OTHER_BING_BONG, FIX_THE_RACES]);

        let unknown = resolve("nobody", &folders).expect_err("expected no task");
        assert_eq!(unknown.to_string(), "@nobody is no task of this board");
        let ambiguous = resolve("bing-bong", &folders).expect_err("expected several tasks");
        assert_eq!(
            ambiguous.to_string(),
            "@bing-bong is several tasks: @bing-bong-313, @bing-bong-9f0"
        );
    }
}
