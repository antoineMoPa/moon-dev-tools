//! Agent programs - the command line each agent answers one prompt with, and the flag it is
//! told a model with.
//!
//! One row per agent, so what differs between them is laid out here rather than spelled out
//! in a branch for each wherever one is run. A comment dispatch runs them with tools and
//! permissions of their own - see [`crate::agent`] - and only takes the model flag from here.

use crate::api::AgentKind;

/// How one agent is run to read a prompt on stdin, answer it, and exit.
pub(crate) struct AgentProgram {
    pub(crate) agent: AgentKind,
    /// The program on the person's PATH.
    pub(crate) program: &'static str,
    /// What comes ahead of the model: the switch that makes it answer and exit.
    answer_args: &'static [&'static str],
    /// What comes after the model - codex's `-`, which is what has it read stdin.
    after_model: &'static [&'static str],
    /// The flag a model is named with, as the next argument.
    model_flag: &'static str,
}

/// No tools, no permissions to skip: an answer read off a prompt is all these are asked for.
const AGENT_PROGRAMS: [AgentProgram; 4] = [
    AgentProgram {
        agent: AgentKind::Claude,
        program: "claude",
        answer_args: &["-p"],
        after_model: &[],
        model_flag: "--model",
    },
    AgentProgram {
        agent: AgentKind::Codex,
        program: "codex",
        answer_args: &["exec"],
        after_model: &["-"],
        model_flag: "--model",
    },
    AgentProgram {
        agent: AgentKind::OpenCode,
        program: "opencode",
        answer_args: &["run"],
        after_model: &[],
        model_flag: "--model",
    },
    AgentProgram {
        agent: AgentKind::Pi,
        program: "pi",
        answer_args: &["--print"],
        after_model: &[],
        model_flag: "--model",
    },
];

/// The row of an agent that runs. [`AgentKind::None`] has none: asking for it is a run that
/// should never have been started.
pub(crate) fn program_of(agent: AgentKind) -> &'static AgentProgram {
    AGENT_PROGRAMS
        .iter()
        .find(|program| program.agent == agent)
        .expect("every agent that runs has a program")
}

/// The arguments that tell `agent` to run with `model`, none when no model was picked and the
/// agent runs with its own default.
pub(crate) fn model_args(agent: AgentKind, model: Option<&str>) -> Vec<&str> {
    match model {
        Some(model) => vec![program_of(agent).model_flag, model],
        None => Vec::new(),
    }
}

/// The whole command line that has `agent` answer the prompt on its stdin, with `model`.
pub(crate) fn answer_args(agent: AgentKind, model: Option<&str>) -> Vec<&str> {
    let program = program_of(agent);
    let mut args = program.answer_args.to_vec();
    args.extend(model_args(agent, model));
    args.extend(program.after_model);
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picked_model_goes_ahead_of_what_reads_the_prompt_and_no_pick_names_none() {
        assert_eq!(
            answer_args(AgentKind::Codex, Some("gpt-5.5")),
            ["exec", "--model", "gpt-5.5", "-"]
        );
        assert_eq!(answer_args(AgentKind::OpenCode, None), ["run"]);
    }
}
