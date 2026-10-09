use std::{
    env,
    io::Read,
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

#[cfg(unix)]
use std::os::unix::process::CommandExt;

use anyhow::{Context, Result, anyhow, bail};

use crate::{
    api::{AgentKind, AgentLog, append_to_agent_log},
    comments::DispatchJob,
};

pub(crate) fn run_agent_dispatch(job: &DispatchJob) -> Result<String> {
    if job.cancel_token.load(Ordering::SeqCst) {
        bail!("agent dispatch stopped");
    }

    let prompt = build_agent_prompt(job);
    match job.agent {
        AgentKind::None => Ok(String::new()),
        AgentKind::Claude => run_claude(prompt, job),
        AgentKind::Codex => run_codex(prompt, job),
        AgentKind::OpenCode => run_opencode(prompt, job),
        AgentKind::Pi => run_pi(prompt, job),
    }
}

fn build_agent_prompt(job: &DispatchJob) -> String {
    if job.targets.len() == 1 {
        let target = &job.targets[0];
        return format!(
            concat!(
                "Moon Review note\n",
                "=================\n",
                "Please fix this code issue.\n\n",
                "Repository: {}\n",
                "File: {}\n",
                "Hunk: {}\n\n",
                "Selected code:\n{}\n\n",
                "Issue:\n{}\n",
            ),
            job.repo_path.display(),
            target.file_path,
            target.header,
            target.selection,
            target.comment,
        );
    }

    let mut prompt = format!(
        concat!(
            "Moon Review batch\n",
            "=================\n",
            "Please address all of the following code review comments in one pass.\n\n",
            "Repository: {}\n\n",
            "Comments:\n\n",
        ),
        job.repo_path.display(),
    );

    for (index, target) in job.targets.iter().enumerate() {
        prompt.push_str(&format!(
            "{}. File: {}\nHunk: {}\nSelected code:\n{}\n\nIssue:\n{}\n\n",
            index + 1,
            target.file_path,
            target.header,
            target.selection,
            target.comment,
        ));
    }

    prompt
}

fn run_claude(prompt: String, job: &DispatchJob) -> Result<String> {
    let mut command = crate::shell_path::installed_tool("claude")?;
    command
        .current_dir(&job.repo_path)
        .args(["-p", "--permission-mode", "bypassPermissions"]);
    configure_agent_command(&mut command);
    let output = command
        .spawn()
        .context("failed to start Claude")?
        .wait_with_streamed_output_from_stdin(
            prompt.as_bytes(),
            "failed to write prompt to Claude",
            "[moonreview] Claude stdout: ",
            "[moonreview] Claude stderr: ",
            Arc::clone(&job.cancel_token),
            Arc::clone(&job.log),
        )?;

    summarize_agent_output("Claude", output)
}

fn run_codex(prompt: String, job: &DispatchJob) -> Result<String> {
    let mut command = crate::shell_path::installed_tool("codex")?;
    command
        .current_dir(&job.repo_path)
        .args(["exec", "--full-auto", "-"]);
    configure_agent_command(&mut command);
    let output = command
        .spawn()
        .context("failed to start Codex")?
        .wait_with_streamed_output_from_stdin(
            prompt.as_bytes(),
            "failed to write prompt to Codex",
            "[moonreview] Codex stdout: ",
            "[moonreview] Codex stderr: ",
            Arc::clone(&job.cancel_token),
            Arc::clone(&job.log),
        )?;

    summarize_agent_output("Codex", output)
}

fn run_opencode(prompt: String, job: &DispatchJob) -> Result<String> {
    let mut command = crate::shell_path::installed_tool("opencode")?;
    command
        .current_dir(&job.repo_path)
        .args(["run", "--dangerously-skip-permissions"])
        .arg(prompt);
    configure_agent_command(&mut command);
    let output = command
        .spawn()
        .context("failed to start OpenCode")?
        .wait_with_streamed_output_from_stdin(
            b"",
            "failed to write prompt to OpenCode",
            "[moonreview] OpenCode stdout: ",
            "[moonreview] OpenCode stderr: ",
            Arc::clone(&job.cancel_token),
            Arc::clone(&job.log),
        )?;

    summarize_agent_output("OpenCode", output)
}

fn run_pi(prompt: String, job: &DispatchJob) -> Result<String> {
    let mut command = crate::shell_path::installed_tool("pi")?;
    command.current_dir(&job.repo_path).arg("--print");
    configure_agent_command(&mut command);
    let output = command
        .spawn()
        .context("failed to start Pi")?
        .wait_with_streamed_output_from_stdin(
            prompt.as_bytes(),
            "failed to write prompt to Pi",
            "[moonreview] Pi stdout: ",
            "[moonreview] Pi stderr: ",
            Arc::clone(&job.cancel_token),
            Arc::clone(&job.log),
        )?;
    summarize_agent_output("Pi", output)
}

/// The pipes an agent is talked to through, and a process group of its own, which is what
/// stopping it ends - see [`stop_process_group`]. Who it runs as and the PATH it is found on
/// are the command's already - see [`crate::shell_path::installed_tool`].
fn configure_agent_command(command: &mut Command) {
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(unix)]
    unsafe {
        command.pre_exec(|| {
            if libc::setpgid(0, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

fn summarize_agent_output(agent: &str, output: std::process::Output) -> Result<String> {
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    if !output.status.success() {
        let detail = if stderr.is_empty() {
            stdout.clone()
        } else {
            stderr.clone()
        };
        bail!("{agent} failed: {}", detail.trim());
    }

    let summary = if stdout.is_empty() { stderr } else { stdout };
    Ok(if summary.is_empty() {
        format!("Sent to {agent}.")
    } else {
        format!("{}: {}", agent, summarize_text(&summary, 240))
    })
}

fn summarize_text(value: &str, max_len: usize) -> String {
    let compact = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() <= max_len {
        return compact;
    }

    compact
        .chars()
        .take(max_len.saturating_sub(1))
        .collect::<String>()
        + "…"
}

pub(crate) trait ChildExt {
    fn wait_with_output_from_stdin(
        self,
        input: &[u8],
        write_error: &str,
    ) -> Result<std::process::Output>;
    fn wait_with_streamed_output_from_stdin(
        self,
        input: &[u8],
        write_error: &str,
        stdout_prefix: &'static str,
        stderr_prefix: &'static str,
        cancel_token: Arc<AtomicBool>,
        log: AgentLog,
    ) -> Result<std::process::Output>;
}

impl ChildExt for std::process::Child {
    fn wait_with_output_from_stdin(
        mut self,
        input: &[u8],
        write_error: &str,
    ) -> Result<std::process::Output> {
        use std::io::Write;

        if let Some(stdin) = self.stdin.as_mut() {
            stdin.write_all(input).context(write_error.to_string())?;
        }
        self.wait_with_output()
            .context("failed to wait for process")
    }

    fn wait_with_streamed_output_from_stdin(
        mut self,
        input: &[u8],
        write_error: &str,
        stdout_prefix: &'static str,
        stderr_prefix: &'static str,
        cancel_token: Arc<AtomicBool>,
        log: AgentLog,
    ) -> Result<std::process::Output> {
        use std::io::Write;

        if let Some(stdin) = self.stdin.as_mut() {
            stdin.write_all(input).context(write_error.to_string())?;
        }
        drop(self.stdin.take());

        let stdout = self
            .stdout
            .take()
            .ok_or_else(|| anyhow!("process stdout was not piped"))?;
        let stderr = self
            .stderr
            .take()
            .ok_or_else(|| anyhow!("process stderr was not piped"))?;

        let stdout_log = Arc::clone(&log);
        let stdout_thread = thread::spawn(move || stream_reader(stdout, stdout_prefix, stdout_log));
        let stderr_thread = thread::spawn(move || stream_reader(stderr, stderr_prefix, log));

        let status = loop {
            if cancel_token.load(Ordering::SeqCst) {
                let _ = stop_process_group(&mut self);
                break self.wait().context("failed to wait for stopped process")?;
            }
            if let Some(status) = self.try_wait().context("failed to wait for process")? {
                break status;
            }
            thread::sleep(Duration::from_millis(100));
        };
        let stdout = stdout_thread
            .join()
            .map_err(|_| anyhow!("stdout stream thread panicked"))??;
        let stderr = stderr_thread
            .join()
            .map_err(|_| anyhow!("stderr stream thread panicked"))??;

        Ok(std::process::Output {
            status,
            stdout,
            stderr,
        })
    }
}

#[cfg(unix)]
fn stop_process_group(child: &mut Child) -> std::io::Result<()> {
    let pid = child.id() as libc::pid_t;
    let group_result = unsafe { libc::kill(-pid, libc::SIGKILL) };
    if group_result == -1 {
        child.kill()
    } else {
        Ok(())
    }
}

#[cfg(not(unix))]
fn stop_process_group(child: &mut Child) -> std::io::Result<()> {
    child.kill()
}

fn stream_reader<R: Read>(mut reader: R, prefix: &'static str, log: AgentLog) -> Result<Vec<u8>> {
    let mut collected = Vec::new();
    let mut buffer = [0u8; 4096];

    loop {
        let bytes_read = reader
            .read(&mut buffer)
            .context("failed to read process output")?;
        if bytes_read == 0 {
            break;
        }

        let chunk = &buffer[..bytes_read];
        collected.extend_from_slice(chunk);
        let text = String::from_utf8_lossy(chunk);
        append_to_agent_log(&log, &text);
        eprint!("{prefix}{text}");
    }

    Ok(collected)
}

pub(crate) fn command_exists(command: &str) -> Result<bool> {
    let path = crate::shell_path::tools_path_of_the_current_user()?;
    Ok(command_exists_on(path.as_ref(), command))
}

/// Whether one of the folders of `path`, a `PATH`, holds a program of this name.
fn command_exists_on(path: &std::ffi::OsStr, command: &str) -> bool {
    env::split_paths(path).any(|dir| {
        let candidate = dir.join(command);
        std::fs::metadata(candidate)
            .map(|meta| meta.is_file())
            .unwrap_or(false)
    })
}

/// Which agents the user moon runs as has installed.
pub(crate) fn detect_agent_availability() -> crate::api::AgentAvailability {
    detect_agent_availability_on(crate::shell_path::installed_tools_path().as_ref())
}

/// Which agents are found on `path`, the login `PATH` of whoever would run them - a person's
/// own, on a server that gives each a Unix user: see `crate::agent_sessions::availability_in`.
pub(crate) fn detect_agent_availability_on(
    path: &std::ffi::OsStr,
) -> crate::api::AgentAvailability {
    crate::api::AgentAvailability {
        claude: command_exists_on(path, "claude"),
        codex: command_exists_on(path, "codex"),
        opencode: command_exists_on(path, "opencode"),
        pi: command_exists_on(path, "pi"),
    }
}

pub(crate) fn agent_is_available(
    availability: crate::api::AgentAvailability,
    agent: crate::api::AgentKind,
) -> bool {
    match agent {
        crate::api::AgentKind::None => true,
        crate::api::AgentKind::Claude => availability.claude,
        crate::api::AgentKind::Codex => availability.codex,
        crate::api::AgentKind::OpenCode => availability.opencode,
        crate::api::AgentKind::Pi => availability.pi,
    }
}

pub(crate) fn agent_options(
    availability: crate::api::AgentAvailability,
) -> Vec<crate::api::AgentOption> {
    [
        (crate::api::AgentKind::None, "No agent"),
        (crate::api::AgentKind::Claude, "Claude"),
        (crate::api::AgentKind::Codex, "Codex"),
        (crate::api::AgentKind::OpenCode, "OpenCode"),
        (crate::api::AgentKind::Pi, "Pi"),
    ]
    .into_iter()
    .map(|(kind, label)| crate::api::AgentOption {
        kind,
        label: label.to_string(),
        available: agent_is_available(availability, kind),
        // Said for one person, by `crate::agent_sessions::agent_options_for`.
        logged_in: None,
        log_in_command: None,
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn pi_selection_tracks_availability_and_serializes() {
        let availability = crate::api::AgentAvailability {
            pi: true,
            ..Default::default()
        };
        let option = agent_options(availability)
            .into_iter()
            .find(|option| option.kind == AgentKind::Pi)
            .unwrap();
        assert_eq!(option.label, "Pi");
        assert!(option.available);
        assert!(!agent_is_available(Default::default(), AgentKind::Pi));
        assert_eq!(serde_json::to_string(&AgentKind::Pi).unwrap(), "\"pi\"");
        assert_eq!(
            serde_json::from_str::<AgentKind>("\"pi\"").unwrap(),
            AgentKind::Pi
        );
        let old: crate::api::AgentAvailability =
            serde_json::from_str(r#"{"claude":true,"codex":false,"opencode":false}"#).unwrap();
        assert!(!old.pi);
    }

    #[test]
    fn wait_with_streamed_output_from_stdin_captures_stdout_and_stderr() {
        let cancel_token = Arc::new(AtomicBool::new(false));
        let output = Command::new("sh")
            .args([
                "-c",
                "cat >/dev/null; printf stdout-value; printf stderr-value >&2",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn test shell")
            .wait_with_streamed_output_from_stdin(
                b"input",
                "write stdin",
                "[test] stdout: ",
                "[test] stderr: ",
                cancel_token,
                Arc::new(std::sync::Mutex::new(String::new())),
            )
            .expect("wait for child");

        assert!(output.status.success());
        assert_eq!(output.stdout, b"stdout-value");
        assert_eq!(output.stderr, b"stderr-value");
    }

    #[test]
    fn wait_with_streamed_output_from_stdin_kills_cancelled_child() {
        let cancel_token = Arc::new(AtomicBool::new(true));
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 10 & wait"]);
        configure_agent_command(&mut command);

        let output = command
            .spawn()
            .expect("spawn test shell")
            .wait_with_streamed_output_from_stdin(
                b"",
                "write stdin",
                "[test] stdout: ",
                "[test] stderr: ",
                cancel_token,
                Arc::new(std::sync::Mutex::new(String::new())),
            )
            .expect("wait for stopped child");

        assert!(!output.status.success());
    }

    #[test]
    fn wait_with_streamed_output_from_stdin_kills_cancelled_descendant() {
        let cancel_token = Arc::new(AtomicBool::new(true));
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 10 &"]);
        configure_agent_command(&mut command);

        let output = command
            .spawn()
            .expect("spawn test shell")
            .wait_with_streamed_output_from_stdin(
                b"",
                "write stdin",
                "[test] stdout: ",
                "[test] stderr: ",
                cancel_token,
                Arc::new(std::sync::Mutex::new(String::new())),
            )
            .expect("wait for stopped child process group");

        assert!(!output.status.success());
    }
}
