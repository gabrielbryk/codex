use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use codex_config::types::TuiStatusLineCommand;
use codex_utils_pty::spawn_pipe_process;
use tokio::sync::mpsc;

use super::MAX_OUTPUT_BYTES;
use super::parse_output;
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
) -> Result<String, String> {
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
    parse_output(&stdout).map_err(str::to_string)
}

async fn collect(mut receiver: mpsc::Receiver<Vec<u8>>) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    while let Some(chunk) = receiver.recv().await {
        if output.len().saturating_add(chunk.len()) > MAX_OUTPUT_BYTES {
            return Err("formatter stream exceeded 4096 bytes".to_string());
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
            )
        })
        .collect()
}

#[cfg(test)]
#[path = "process_tests.rs"]
mod tests;
