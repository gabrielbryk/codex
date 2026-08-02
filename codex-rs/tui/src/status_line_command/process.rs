//! Local process execution for external status-line formatters.

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use codex_config::types::TuiStatusLineCommand;
use codex_utils_pty::spawn_piped_contained_process;
use tokio::sync::mpsc;

use super::parser::MAX_STATUS_LINE_COMMAND_BYTES;
use super::parser::parse_status_line_command_output;
use super::runner::StatusLineCommandCompletion;
use super::runner::StatusLineCommandFailure;
use super::runner::StatusLineCommandFailureKind;
use super::runner::StatusLineCommandInvocation;
use super::runner::StatusLineCommandOutcome;

/// Execute a formatter on the local TUI host and return its owned completion.
pub(crate) async fn execute_status_line_command(
    config: TuiStatusLineCommand,
    local_cwd: &Path,
    invocation: StatusLineCommandInvocation,
) -> StatusLineCommandCompletion {
    let token = invocation.token;
    let outcome = execute(config, local_cwd, invocation.input).await;
    StatusLineCommandCompletion { token, outcome }
}

async fn execute(
    config: TuiStatusLineCommand,
    local_cwd: &Path,
    input: super::wire::StatusLineCommandInput,
) -> StatusLineCommandOutcome {
    let Some((program, args)) = config.command.split_first() else {
        return failure(StatusLineCommandFailureKind::Spawn, "empty command argv");
    };
    let input = match input.to_json_line() {
        Ok(input) => input,
        Err(err) => return failure(StatusLineCommandFailureKind::Stdin, err.to_string()),
    };
    let env = std::env::vars().collect::<HashMap<_, _>>();
    let spawned = match spawn_piped_contained_process(
        program,
        args,
        local_cwd,
        &env,
        /*arg0*/ &None,
        /*inherited_fds*/ &[],
    )
    .await
    {
        Ok(spawned) => spawned,
        Err(err) => return failure(StatusLineCommandFailureKind::Spawn, err.to_string()),
    };

    let codex_utils_pty::SpawnedProcess {
        session,
        stdout_rx,
        stderr_rx,
        exit_rx,
    } = spawned;
    let writer = session.writer_sender();
    if let Err(err) = writer.send(input).await {
        session.terminate();
        return failure(StatusLineCommandFailureKind::Stdin, err.to_string());
    }
    drop(writer);
    session.close_stdin();

    let timeout = Duration::from_millis(config.timeout_ms);
    let collected = tokio::time::timeout(timeout, async {
        tokio::try_join!(
            collect_bounded(stdout_rx),
            collect_bounded(stderr_rx),
            async {
                exit_rx.await.map_err(|err| StreamFailure {
                    kind: StatusLineCommandFailureKind::ExitStatus,
                    message: err.to_string(),
                })
            }
        )
    })
    .await;

    let (stdout, _stderr, exit_code) = match collected {
        Err(_) => {
            session.terminate();
            return failure(StatusLineCommandFailureKind::Timeout, "formatter timed out");
        }
        Ok(Err(err)) => {
            session.terminate();
            return failure(err.kind, err.message);
        }
        Ok(Ok(output)) => output,
    };
    if exit_code != 0 {
        return failure(
            StatusLineCommandFailureKind::ExitStatus,
            format!("formatter exited with status {exit_code}"),
        );
    }

    match parse_status_line_command_output(&stdout) {
        Ok(parsed) => StatusLineCommandOutcome::Success(parsed),
        Err(err) => failure(StatusLineCommandFailureKind::Parse, err.to_string()),
    }
}

#[derive(Debug)]
struct StreamFailure {
    kind: StatusLineCommandFailureKind,
    message: String,
}

async fn collect_bounded(mut receiver: mpsc::Receiver<Vec<u8>>) -> Result<Vec<u8>, StreamFailure> {
    let mut output = Vec::new();
    while let Some(chunk) = receiver.recv().await {
        if output.len().saturating_add(chunk.len()) > MAX_STATUS_LINE_COMMAND_BYTES {
            return Err(StreamFailure {
                kind: StatusLineCommandFailureKind::OutputLimit,
                message: format!("formatter stream exceeded {MAX_STATUS_LINE_COMMAND_BYTES} bytes"),
            });
        }
        output.extend_from_slice(&chunk);
    }
    Ok(output)
}

fn failure(
    kind: StatusLineCommandFailureKind,
    message: impl Into<String>,
) -> StatusLineCommandOutcome {
    StatusLineCommandOutcome::Failure(StatusLineCommandFailure {
        kind,
        message: message.into(),
    })
}

#[cfg(test)]
#[path = "process_tests.rs"]
mod tests;
