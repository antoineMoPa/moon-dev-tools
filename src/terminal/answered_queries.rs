//! Questions a program asked its terminal that have been answered already, taken out of what a
//! newly attached window is replayed.
//!
//! A program asks its terminal things by printing them: what it is (`ESC [ c`), whether it
//! speaks the kitty keyboard protocol (`ESC [ ? u`), which version it is (`ESC [ > q`), what
//! colour its background is (`ESC ] 11 ; ? BEL`). The terminal answers by typing the answer
//! back. Claude Code asks all but the last on startup.
//!
//! A window attaching to a shell is handed everything the shell printed so far, and its
//! emulator reads that the way it reads anything - questions included - and answers them. When
//! a window was attached as they were asked, they were answered then, and the second answer
//! reaches a program that is long past asking: in the middle of its work it is typed an
//! `ESC`, which to Claude Code is the key that interrupts it. So the questions a window saw
//! go out of the replay, and only those: what was printed while no window was attached was
//! answered by nobody, and a program still waiting on it is owed its answer.

/// What a CSI question looks like after `ESC [`: the private marker it opens with, if any, the
/// intermediate byte before its final byte, if any, and the final byte.
struct CsiQuestion {
    marker: Option<u8>,
    intermediate: Option<u8>,
    final_byte: u8,
}

const CSI_QUESTIONS: &[CsiQuestion] = &[
    // Primary, secondary and tertiary device attributes: what the terminal is.
    CsiQuestion { marker: None, intermediate: None, final_byte: b'c' },
    CsiQuestion { marker: Some(b'>'), intermediate: None, final_byte: b'c' },
    CsiQuestion { marker: Some(b'='), intermediate: None, final_byte: b'c' },
    // Device status reports: whether it is well, where the cursor is.
    CsiQuestion { marker: None, intermediate: None, final_byte: b'n' },
    CsiQuestion { marker: Some(b'?'), intermediate: None, final_byte: b'n' },
    // Which kitty keyboard flags are on. `>` pushes flags and `<` pops them; only `?` asks.
    CsiQuestion { marker: Some(b'?'), intermediate: None, final_byte: b'u' },
    // XTVERSION: the terminal's name and version.
    CsiQuestion { marker: Some(b'>'), intermediate: None, final_byte: b'q' },
    // DECRQM: whether a mode is set, ANSI and private.
    CsiQuestion { marker: None, intermediate: Some(b'$'), final_byte: b'p' },
    CsiQuestion { marker: Some(b'?'), intermediate: Some(b'$'), final_byte: b'p' },
];

/// The window operations, `ESC [ Ps t`, that ask rather than do: the window's state, place
/// and size in pixels and in cells, the screen's size, the icon and window titles. The rest of
/// `t` moves and resizes the window, which a replay is welcome to repeat.
const WINDOW_REPORTS: &[&str] = &["11", "13", "14", "15", "16", "18", "19", "20", "21"];

/// The DCS questions, by how their payload opens: DECRQSS, the setting of a control function,
/// and XTGETTCAP, a terminfo capability.
const DCS_QUESTIONS: &[&[u8]] = &[b"$q", b"+q"];

/// How an OSC question ends: a colour, a palette entry or the clipboard, asked for with `?`
/// where the value would go.
const OSC_QUESTION_END: &[u8] = b";?";

const ESC: u8 = 0x1b;
const BEL: u8 = 0x07;

/// `output` with the questions in it taken out, and everything else as it was - including a
/// sequence cut off at its end, which a later chunk finishes.
pub(super) fn without_answered_queries(output: &[u8]) -> Vec<u8> {
    let mut kept = Vec::with_capacity(output.len());
    let mut at = 0;
    while at < output.len() {
        match question_at(output, at) {
            Some(end) => at = end,
            None => {
                kept.push(output[at]);
                at += 1;
            }
        }
    }
    kept
}

/// Where the question starting at `at` ends, if one does.
fn question_at(output: &[u8], at: usize) -> Option<usize> {
    if output[at] != ESC {
        return None;
    }
    match output.get(at + 1)? {
        b'[' => csi_question(output, at + 2),
        b']' => osc_question(output, at + 2),
        b'P' => dcs_question(output, at + 2),
        _ => None,
    }
}

fn csi_question(output: &[u8], start: usize) -> Option<usize> {
    let mut at = start;
    let marker = match output.get(at)? {
        byte @ (b'<' | b'=' | b'>' | b'?') => {
            at += 1;
            Some(*byte)
        }
        _ => None,
    };
    let params_start = at;
    while matches!(output.get(at)?, b'0'..=b'9' | b';' | b':') {
        at += 1;
    }
    let params = &output[params_start..at];
    let intermediate = match output.get(at)? {
        byte @ 0x20..=0x2f => {
            at += 1;
            Some(*byte)
        }
        _ => None,
    };
    let final_byte = *output.get(at)?;
    if !(0x40..=0x7e).contains(&final_byte) {
        return None;
    }
    let end = at + 1;

    let listed = CSI_QUESTIONS.iter().any(|question| {
        question.marker == marker
            && question.intermediate == intermediate
            && question.final_byte == final_byte
    });
    let window_report = marker.is_none()
        && intermediate.is_none()
        && final_byte == b't'
        && WINDOW_REPORTS
            .iter()
            .any(|report| params == report.as_bytes());
    (listed || window_report).then_some(end)
}

fn osc_question(output: &[u8], start: usize) -> Option<usize> {
    let (payload_end, end) = string_end(output, start)?;
    output[start..payload_end]
        .ends_with(OSC_QUESTION_END)
        .then_some(end)
}

fn dcs_question(output: &[u8], start: usize) -> Option<usize> {
    let (_, end) = string_end(output, start)?;
    DCS_QUESTIONS
        .iter()
        .any(|opening| output[start..].starts_with(opening))
        .then_some(end)
}

/// Where a string sequence's payload ends, and where the sequence does: at BEL, or at the
/// string terminator `ESC \`. `None` for one the output stops in the middle of.
fn string_end(output: &[u8], start: usize) -> Option<(usize, usize)> {
    let mut at = start;
    loop {
        match *output.get(at)? {
            BEL => return Some((at, at + 1)),
            ESC if output.get(at + 1) == Some(&b'\\') => return Some((at, at + 2)),
            _ => at += 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::without_answered_queries;

    #[test]
    fn claude_codes_startup_questions_are_taken_out() {
        let printed = b"\x1b[?1004h\x1b[c\x1b[?u\x1b[>0qready\x1b[31mred\x1b[0m";
        assert_eq!(
            without_answered_queries(printed),
            b"\x1b[?1004hready\x1b[31mred\x1b[0m".to_vec()
        );
    }

    #[test]
    fn colour_and_capability_questions_are_taken_out() {
        let printed = b"a\x1b]11;?\x07b\x1b]4;1;?\x1b\\c\x1bP+q544e\x1b\\d\x1bP$qm\x1b\\e";
        assert_eq!(without_answered_queries(printed), b"abcde".to_vec());
    }

    #[test]
    fn status_mode_and_window_questions_are_taken_out_but_window_moves_are_kept() {
        let printed = b"\x1b[6n\x1b[?2026$p\x1b[18t\x1b[8;40;120t\x1b[5n";
        assert_eq!(without_answered_queries(printed), b"\x1b[8;40;120t".to_vec());
    }

    #[test]
    fn what_is_not_a_question_is_kept_as_it_was() {
        let printed: &[u8] =
            b"\x1b]0;title\x07\x1b[>1u\x1b[<u\x1b[2J\x1b[H\x1b[1;32mok\x1b[0m\r\n\x1b(B";
        assert_eq!(without_answered_queries(printed), printed.to_vec());
    }

    #[test]
    fn a_sequence_cut_off_at_the_end_is_kept() {
        let printed: &[u8] = b"text\x1b]11;";
        assert_eq!(without_answered_queries(printed), printed.to_vec());
        let printed: &[u8] = b"text\x1b[";
        assert_eq!(without_answered_queries(printed), printed.to_vec());
    }
}
