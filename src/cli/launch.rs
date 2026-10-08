//! `moon launch <command>` - starts a program with windows in the moon whose shell it is typed
//! in: on the desktop of a `moon serve`, or on the screen a `moon desktop` is the session of.
//!
//! It is what `moon › Applications` does for the programs it lists, for a program it does not
//! list. How the command reaches the moon is [`crate::instances`]' business.
//!
//! The words after `launch` are the program and its arguments, as the shell handed them over.
//! Every place a moon starts an application takes one line of shell, so they are made that
//! here, once: each word quoted, so that the shell which runs the line hands the program the
//! same arguments.

use anyhow::{Context, Result, bail};

use super::{MoonCommand, PROGRAM};
use crate::{instances, shell_quoting::single_quoted};

/// `moon launch <command> [<argument>...]`.
pub(super) fn parse_launch(args: Vec<String>) -> Result<MoonCommand> {
    let Some(program) = args.first() else {
        bail!(
            "`{PROGRAM} launch` needs a command to start\n\n{}",
            help_text()
        );
    };
    // Only ahead of the program is `--help` this command's own: after it, it is one of the
    // program's arguments like any other, and reaches it.
    if program == "--help" || program == "-h" {
        return Ok(MoonCommand::LaunchHelp);
    }
    // A word starting with a dash is an option being tried, not a program to start.
    if program.starts_with('-') {
        bail!(
            "`{PROGRAM} launch` takes a command to start and no options of its own, not \
             {program}\n\n{}",
            help_text()
        );
    }
    Ok(MoonCommand::Launch {
        command: line_of_shell(&args),
    })
}

/// The words of a command as the line of shell that runs it with those words for arguments.
fn line_of_shell(words: &[String]) -> String {
    let quoted: Vec<String> = words.iter().map(|word| single_quoted(word)).collect();
    quoted.join(" ")
}

/// `moon launch --help`: what is started, and where.
pub(super) fn help_text() -> String {
    format!(
        "{PROGRAM} launch <command> [<argument>...]

Starts a program with windows in the moon this shell is a tab of:
  - in a shell of `{PROGRAM} serve`, on the server's desktop. The desktop is started with
    the program when there is none, and every window on that server then opens a pane on it.
  - in a shell of `{PROGRAM} desktop`, on the screen, where its window is put in a pane.
A window among others on a machine's own screen starts no programs: run the command itself.

It returns once the program has started, and does not wait for it to end. A program that
fails as it starts is reported with what it said, and `{PROGRAM} launch` fails with it.

Examples:
  {PROGRAM} launch chromium
  {PROGRAM} launch chromium --incognito \"https://example.com/?a=b c\"

The program is run in the directory this shell is in. Each word after `launch` reaches it
as one argument, quoted here as for any command; none is read as an option of
`{PROGRAM} launch`, so `{PROGRAM} launch xterm --help` is xterm's help.
A desktop started this way has a view of 1280x800 until a pane shows it, and draws its
applications at scale 1; one started from a window's `{PROGRAM} › Applications` draws them
at that window's scale."
    )
}

/// Have this shell's moon start the program, and say where its windows open.
pub(super) fn launch(command: &str) -> Result<()> {
    let folder = std::env::current_dir().context("failed to read the current directory")?;
    let on = instances::launch(command, &folder)?;
    println!("{command} → {on}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::command::parse_command;
    use super::*;

    fn parse(args: &[&str]) -> Result<MoonCommand> {
        parse_command(None, args.iter().map(|arg| arg.to_string()).collect())
    }

    /// Each word is one argument of the program, whatever is in it: the line is what a shell
    /// reads back as those words.
    #[test]
    fn the_words_after_launch_reach_the_program_as_its_arguments() {
        assert_eq!(
            parse(&[
                "launch",
                "chromium",
                "--incognito",
                "https://example.com/?a=b c"
            ])
            .expect("expected it to parse"),
            MoonCommand::Launch {
                command: "'chromium' '--incognito' 'https://example.com/?a=b c'".to_string(),
            }
        );
        // A quote in a word, and a word that is empty, are still that word.
        assert_eq!(
            parse(&["launch", "xmessage", "it's", ""]).expect("expected it to parse"),
            MoonCommand::Launch {
                command: "'xmessage' 'it'\\''s' ''".to_string(),
            }
        );
    }

    /// `--help` is this command's own only ahead of the program.
    #[test]
    fn help_is_asked_ahead_of_the_program_and_is_the_programs_after_it() {
        assert_eq!(
            parse(&["launch", "--help"]).expect("expected it to parse"),
            MoonCommand::LaunchHelp
        );
        assert_eq!(
            parse(&["launch", "-h"]).expect("expected it to parse"),
            MoonCommand::LaunchHelp
        );
        assert_eq!(
            parse(&["launch", "xterm", "--help"]).expect("expected it to parse"),
            MoonCommand::Launch {
                command: "'xterm' '--help'".to_string(),
            }
        );
        assert!(help_text().contains("launch <command>"));
    }

    #[test]
    fn launching_nothing_or_an_option_says_what_is_wanted() {
        let error = parse(&["launch"]).expect_err("expected a refusal");
        assert!(
            format!("{error}").contains("needs a command to start"),
            "got {error}"
        );

        let error = parse(&["launch", "--display=:7", "xterm"]).expect_err("expected a refusal");
        assert!(
            format!("{error}").contains("no options of its own"),
            "got {error}"
        );
    }
}
