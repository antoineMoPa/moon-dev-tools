//! What Codex is started with in a moon terminal so its model can show visualizations.
//!
//! The Codex app teaches its model the inline visualization contract; the Codex CLI does not
//! ship those instructions, and its model left alone draws ASCII. So moon hands them over as
//! developer instructions - [`VISUALIZE_INSTRUCTIONS`] - and lets the sandbox write the folder
//! the fragments go in.
//!
//! Both have to be added to what Codex would have had, not put in its place. A `-c` on Codex's
//! command line replaces a config value outright: `-c developer_instructions=…` hides the
//! person's own `developer_instructions` from `config.toml`, and when two are given the last
//! wins, so a task's brief and these instructions cannot be two of them. So everything is
//! joined into one - the config's, the task's brief, then these. The folder goes through
//! `--add-dir`, which adds a writable root rather than replacing
//! `sandbox_workspace_write.writable_roots`.

use std::path::Path;

use anyhow::{Context, Result};

/// What the model is told about showing a visualization, restating Codex's contract.
pub(crate) const VISUALIZE_INSTRUCTIONS: &str =
    include_str!("../../assets/codex_visualization/instructions.md");

const DEVELOPER_INSTRUCTIONS: &str = "developer_instructions=";

/// The arguments Codex is started with: developer instructions carrying the visualization
/// contract, the visualizations folder as a writable root, then `args` - less the developer
/// instructions it had, which are folded into the one given here.
///
/// Both lead: `-c` and `--add-dir` are options of `codex` itself, and `args` may go on to a
/// subcommand such as `resume`.
pub(crate) fn codex_arguments(codex_home: &Path, args: &[String]) -> Result<Vec<String>> {
    let (briefs, rest) = take_developer_instructions(args);
    let configured = configured_developer_instructions(codex_home)?;
    let instructions = configured
        .into_iter()
        .chain(briefs)
        .chain([VISUALIZE_INSTRUCTIONS.to_string()])
        .filter(|text| !text.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");

    // A writable root that is not there is no root at all to the sandbox, and the model is the
    // one that creates the thread's folder inside it.
    let visualizations_dir = codex_home.join("visualizations");
    std::fs::create_dir_all(&visualizations_dir)
        .with_context(|| format!("could not create {}", visualizations_dir.display()))?;

    let mut arguments = vec![
        "-c".to_string(),
        // Quoted as a TOML string: `-c` reads its value as TOML first and only falls back to
        // the raw text when that fails, so free text that happened to parse would be read as
        // something else.
        format!(
            "{DEVELOPER_INSTRUCTIONS}{}",
            toml::Value::String(instructions)
        ),
        "--add-dir".to_string(),
        visualizations_dir.display().to_string(),
    ];
    arguments.extend(rest);
    Ok(arguments)
}

/// The developer instructions a `-c` in `args` gives, and `args` without it.
fn take_developer_instructions(args: &[String]) -> (Vec<String>, Vec<String>) {
    let mut briefs = Vec::new();
    let mut rest = Vec::new();
    let mut arguments = args.iter();
    while let Some(argument) = arguments.next() {
        if argument == "-c" {
            let value = arguments
                .next()
                .expect("`-c` is always followed by the value it sets");
            match value.strip_prefix(DEVELOPER_INSTRUCTIONS) {
                Some(brief) => briefs.push(brief.to_string()),
                None => rest.extend([argument.clone(), value.clone()]),
            }
        } else {
            rest.push(argument.clone());
        }
    }
    assert!(
        briefs.len() <= 1,
        "moon starts Codex with one set of developer instructions at most"
    );
    (briefs, rest)
}

/// The `developer_instructions` of the person's own `config.toml`, if they set any.
fn configured_developer_instructions(codex_home: &Path) -> Result<Option<String>> {
    let config_path = codex_home.join("config.toml");
    let text = match std::fs::read_to_string(&config_path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| format!("could not read {}", config_path.display()));
        }
    };
    let config: toml::Table = text
        .parse()
        .with_context(|| format!("could not read {} as TOML", config_path.display()))?;
    Ok(match config.get("developer_instructions") {
        None => None,
        Some(toml::Value::String(instructions)) => Some(instructions.clone()),
        Some(other) => anyhow::bail!(
            "developer_instructions in {} is a {}, where Codex reads a string",
            config_path.display(),
            other.type_str()
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codex_home_with_config(config: Option<&str>) -> std::path::PathBuf {
        let codex_home = std::env::temp_dir().join(format!(
            "moon-codex-home-{}",
            crate::moontasks::store::new_uuid()
        ));
        std::fs::create_dir_all(&codex_home).unwrap();
        if let Some(config) = config {
            std::fs::write(codex_home.join("config.toml"), config).unwrap();
        }
        codex_home
    }

    /// The text a `-c developer_instructions=…` gives Codex, read the way Codex reads it.
    fn instructions_in(arguments: &[String]) -> String {
        let value = arguments[1].strip_prefix(DEVELOPER_INSTRUCTIONS).unwrap();
        let table: toml::Table = format!("value = {value}").parse().unwrap();
        table["value"].as_str().unwrap().to_string()
    }

    #[test]
    fn the_person_s_instructions_and_the_brief_come_before_the_contract() {
        let codex_home =
            codex_home_with_config(Some("developer_instructions = \"Answer in French.\"\n"));
        let args = [
            "-c".to_string(),
            "developer_instructions=You are working on a task. \"Quoted\" = {braces}".to_string(),
            "resume".to_string(),
            "--last".to_string(),
        ];

        let arguments = codex_arguments(&codex_home, &args).unwrap();

        assert_eq!(arguments[0], "-c");
        assert_eq!(
            instructions_in(&arguments),
            format!(
                "Answer in French.\n\nYou are working on a task. \"Quoted\" = {{braces}}\n\n{VISUALIZE_INSTRUCTIONS}"
            )
        );
        assert_eq!(
            &arguments[2..],
            [
                "--add-dir".to_string(),
                codex_home.join("visualizations").display().to_string(),
                "resume".to_string(),
                "--last".to_string(),
            ]
        );
        assert!(codex_home.join("visualizations").is_dir());
    }

    #[test]
    fn a_codex_with_no_config_is_given_the_contract_alone() {
        let codex_home = codex_home_with_config(None);

        let arguments = codex_arguments(&codex_home, &[]).unwrap();

        assert_eq!(instructions_in(&arguments), VISUALIZE_INSTRUCTIONS);
        assert_eq!(arguments.len(), 4);
    }

    #[test]
    fn other_config_overrides_are_kept_where_they_were() {
        let codex_home = codex_home_with_config(Some("model = \"gpt-5.5\"\n"));
        let args = ["-c".to_string(), "model=\"o3\"".to_string()];

        let arguments = codex_arguments(&codex_home, &args).unwrap();

        assert_eq!(instructions_in(&arguments), VISUALIZE_INSTRUCTIONS);
        assert_eq!(&arguments[4..], args);
    }

    #[test]
    fn a_config_codex_could_not_read_either_is_refused() {
        let codex_home = codex_home_with_config(Some("developer_instructions = 3\n"));

        assert!(codex_arguments(&codex_home, &[]).is_err());
    }
}
