use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use codex_config::types::TuiStatusLineCommand;
use codex_utils_pty::spawn_pipe_process;
use tokio::sync::mpsc;

use super::parser::MAX_STATUS_LINE_COMMAND_BYTES;
use super::parser::parse_status_line_command_output;
use super::runner::Completion;
use super::runner::Invocation;

/// Runs one formatter invocation with bounded input, output, and elapsed time.
///
/// On Unix, timeout termination targets the formatter's process group. On Windows, the pipe
/// backend attempts Job Object containment but may fall back to terminating only the root process.
pub(crate) async fn execute(
    config: TuiStatusLineCommand,
    cwd: &Path,
    invocation: Invocation,
) -> Completion {
    let token = invocation.token;
    let result = execute_inner(config, cwd, invocation.input).await;
    Completion { token, result }
}

async fn execute_inner(
    config: TuiStatusLineCommand,
    cwd: &Path,
    input: super::wire::StatusLineCommandInput,
) -> Result<super::parser::ParsedStatusLine, String> {
    let (program, args) = config
        .command
        .split_first()
        .ok_or_else(|| "empty formatter command".to_string())?;
    let stdin = input.to_json_line().map_err(str::to_string)?;
    let env = formatter_environment(std::env::vars());
    let spawned = spawn_pipe_process(program, args, cwd, &env, &None, &[])
        .await
        .map_err(|error| error.to_string())?;
    let codex_utils_pty::SpawnedProcess {
        session,
        stdout_rx,
        stderr_rx,
        exit_rx,
    } = spawned;
    let writer = session.writer_sender();
    writer
        .send(stdin)
        .await
        .map_err(|error| error.to_string())?;
    drop(writer);
    session.close_stdin();

    let collected = tokio::time::timeout(Duration::from_millis(config.timeout_ms), async {
        tokio::try_join!(collect(stdout_rx), collect(stderr_rx), async {
            exit_rx.await.map_err(|error| error.to_string())
        })
    })
    .await;
    let (stdout, _stderr, exit_code) = match collected {
        Err(_) => {
            session.terminate();
            return Err("formatter timed out".to_string());
        }
        Ok(Err(error)) => {
            session.terminate();
            return Err(error);
        }
        Ok(Ok(output)) => output,
    };
    if exit_code != 0 {
        return Err(format!("formatter exited with status {exit_code}"));
    }
    parse_status_line_command_output(&stdout).map_err(|error| error.to_string())
}

async fn collect(mut receiver: mpsc::Receiver<Vec<u8>>) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    while let Some(chunk) = receiver.recv().await {
        if output.len().saturating_add(chunk.len()) > MAX_STATUS_LINE_COMMAND_BYTES {
            return Err(format!(
                "formatter stream exceeded {MAX_STATUS_LINE_COMMAND_BYTES} bytes"
            ));
        }
        output.extend_from_slice(&chunk);
    }
    Ok(output)
}

fn formatter_environment(
    environment: impl IntoIterator<Item = (String, String)>,
) -> HashMap<String, String> {
    environment
        .into_iter()
        .filter(|(name, _)| {
            matches!(
                name.to_ascii_uppercase().as_str(),
                "SYSTEMROOT"
                    | "COMSPEC"
                    | "WINDIR"
                    | "PATH"
                    | "HOME"
                    | "USERPROFILE"
                    | "HOMEDRIVE"
                    | "HOMEPATH"
                    | "TMPDIR"
                    | "TMP"
                    | "TEMP"
                    | "LANG"
                    | "LANGUAGE"
                    | "LC_ALL"
                    | "LC_CTYPE"
                    | "TERM"
                    | "COLORTERM"
                    | "NO_COLOR"
                    | "XDG_CACHE_HOME"
                    | "XDG_CONFIG_HOME"
                    | "XDG_DATA_HOME"
                    | "XDG_STATE_HOME"
                    | "XDG_RUNTIME_DIR"
                    | "CODEX_HOME"
                    | "CCSTATUSLINE_BIN"
                    | "CODEX_STATUSLINE_CONFIG"
                    | "CODEX_STATUSLINE_HOME_BADGE"
                    | "AGENT_LIVE_STATUSLINE_BADGE"
                    | "AGENT_LIVE_STATUSLINE_OWNER_PID"
                    | "AGENT_LIVE_STATUSLINE_OWNER_STARTED_AT"
                    | "AGENT_LIVE_STATUSLINE_RELEASE_AT"
                    | "AGENT_LIVE_STATUSLINE_RELEASE_PATH"
                    | "STATUSLINE_RENDER_TIMEOUT_MS"
                    | "STATUSLINE_HELPER_TIMEOUT_MS"
                    | "STATUSLINE_TELEMETRY_PATH"
                    | "STATUSLINE_TELEMETRY_MAX_BYTES"
                    | "STATUSLINE_TELEMETRY_WRITE_TIMEOUT_MS"
                    | "STATUSLINE_VERSION_SKEW_BADGE"
                    | "STATUSLINE_VERSION_SKEW_CACHE_PATH"
                    | "STATUSLINE_VERSION_SKEW_CACHE_TTL_SECONDS"
                    | "STATUSLINE_WORKTREE_ABBREV"
            )
        })
        .collect()
}

#[cfg(test)]
#[path = "process_tests.rs"]
mod tests;
