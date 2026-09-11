//! Running tier-T1 script commands.
//!
//! # Why not just hand the path to the shell
//!
//! `ShellExecuteW` launches a file and returns immediately; it cannot report
//! what the file printed, which `ScriptMode::Copy` needs. So scripts are spawned
//! rather than shell-executed, and the interpreter is chosen from the extension.
//!
//! # The console window
//!
//! A launcher summoned by a keystroke must not flash a black rectangle on the
//! screen every time it runs a one-line script. `CREATE_NO_WINDOW` is what
//! prevents that, and it is the single most visible difference between a script
//! command that feels built in and one that feels bolted on.

use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Command;

use anyhow::{Context as _, Result};

/// Suppress the console window the child would otherwise be given.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// How long a script may run before it is abandoned.
///
/// Not a kill: the process keeps going, but Lumen stops waiting for its output.
/// A script that hangs must not leak a task that waits on it forever, and the
/// user has already moved on -- the launcher was dismissed the moment they
/// pressed Enter.
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

pub fn run(path: &Path) -> Result<String> {
    let mut command = interpreter_for(path);
    command.creation_flags(CREATE_NO_WINDOW);

    // Run from the script's own folder, so a script can refer to a file beside
    // it without knowing where Lumen happened to be started from.
    if let Some(parent) = path.parent() {
        command.current_dir(parent);
    }

    let mut child = command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .with_context(|| format!("running {}", path.display()))?;

    let started = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() < TIMEOUT => {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Ok(None) => anyhow::bail!(
                "{} is still running after {} seconds; giving up on its output",
                path.display(),
                TIMEOUT.as_secs()
            ),
            Err(error) => return Err(error).context("waiting for the script"),
        }
    }

    let output = child
        .wait_with_output()
        .with_context(|| format!("reading the output of {}", path.display()))?;

    if !output.status.success() {
        let reason = String::from_utf8_lossy(&output.stderr);
        let reason = reason.trim();
        anyhow::bail!(
            "{} exited with {}{}",
            path.display(),
            output.status,
            if reason.is_empty() {
                String::new()
            } else {
                format!(": {reason}")
            }
        );
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Choose how to run a file, from its extension.
///
/// Only the interpreters Windows ships with, plus direct execution. Anything
/// else -- Python, Node -- is reached by giving the script a shebang and an
/// interpreter on `PATH`, or by making it a `.cmd` that calls one. Guessing at
/// interpreters Lumen cannot know are installed would fail later and less
/// clearly than not guessing.
fn interpreter_for(path: &Path) -> Command {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("")
        .to_lowercase();

    match extension.as_str() {
        "ps1" => {
            let mut command = Command::new("powershell.exe");
            // `-NoProfile` because a user's profile can take a second to load
            // and can print things that would end up on the clipboard.
            // `Bypass` applies to this invocation only and does not change the
            // machine's policy.
            command.args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ]);
            command.arg(path);
            command
        }
        "cmd" | "bat" => {
            let mut command = Command::new("cmd.exe");
            command.arg("/C").arg(path);
            command
        }
        _ => Command::new(path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn program_of(path: &str) -> String {
        interpreter_for(Path::new(path))
            .get_program()
            .to_string_lossy()
            .to_lowercase()
    }

    #[test]
    fn powershell_scripts_go_through_powershell() {
        assert_eq!(program_of("C:/scripts/a.ps1"), "powershell.exe");
        // The extension is matched case-insensitively: Windows does not care
        // how the file was named and neither should this.
        assert_eq!(program_of("C:/scripts/A.PS1"), "powershell.exe");
    }

    #[test]
    fn batch_files_go_through_cmd() {
        assert_eq!(program_of("C:/scripts/a.cmd"), "cmd.exe");
        assert_eq!(program_of("C:/scripts/a.bat"), "cmd.exe");
    }

    #[test]
    fn anything_else_is_executed_directly() {
        assert_eq!(program_of("C:/scripts/tool.exe"), "c:/scripts/tool.exe");
        assert_eq!(program_of("C:/scripts/noext"), "c:/scripts/noext");
    }

    #[test]
    fn powershell_is_invoked_without_the_users_profile() {
        let command = interpreter_for(Path::new("C:/scripts/a.ps1"));
        let args: Vec<String> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect();

        // A profile that prints a banner would put the banner on the clipboard.
        assert!(args.iter().any(|arg| arg == "-NoProfile"));
        assert!(args.iter().any(|arg| arg == "-NonInteractive"));
    }

    #[test]
    fn a_script_that_prints_has_its_output_captured() {
        let dir = std::env::temp_dir().join(format!("lumen-run-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("hello.cmd");
        std::fs::write(&path, "@echo off\r\necho hello from lumen\r\n").unwrap();

        let output = run(&path).expect("the script should run");
        assert_eq!(output, "hello from lumen");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failing_script_reports_why() {
        let dir = std::env::temp_dir().join(format!("lumen-fail-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("bad.cmd");
        std::fs::write(&path, "@echo off\r\nexit /b 3\r\n").unwrap();

        let error = run(&path).expect_err("a non-zero exit must be an error");
        assert!(error.to_string().contains("exited with"), "{error}");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
