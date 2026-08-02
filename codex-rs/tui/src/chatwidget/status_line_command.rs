//! External status-line command scheduling and completion application.

use std::path::PathBuf;

use tokio::task::AbortHandle;

use super::ChatWidget;
use crate::app_event::AppEvent;
use crate::status_line_command::process::execute_status_line_command;
use crate::status_line_command::runner::STATUS_LINE_COMMAND_DEBOUNCE;
use crate::status_line_command::runner::StatusLineCommandApplyResult;
use crate::status_line_command::runner::StatusLineCommandCompletion;
use crate::status_line_command::runner::StatusLineCommandLifecycle;
use crate::status_line_command::runner::StatusLineCommandOutcome;
use crate::status_line_command::wire::StatusLineCommandInput;
use crate::status_line_command::wire::StatusLineCommandSessionId;
use crate::terminal_hyperlinks::visible_lines;

pub(super) struct StatusLineCommandRuntime {
    lifecycle: StatusLineCommandLifecycle,
    local_cwd: Option<PathBuf>,
    last_input: Option<StatusLineCommandInput>,
    task: Option<AbortHandle>,
}

impl StatusLineCommandRuntime {
    pub(super) fn new(local_cwd: Option<PathBuf>) -> Self {
        Self {
            lifecycle: StatusLineCommandLifecycle::new(),
            local_cwd,
            last_input: None,
            task: None,
        }
    }

    fn cancel(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }

    pub(super) fn lifecycle_session_id(&self) -> StatusLineCommandSessionId {
        self.lifecycle.session_id().clone()
    }
}

impl Drop for StatusLineCommandRuntime {
    fn drop(&mut self) {
        self.cancel();
    }
}

impl ChatWidget {
    pub(super) fn status_line_command_local_cwd(&self) -> Option<PathBuf> {
        self.status_line_command
            .as_ref()
            .and_then(|runtime| runtime.local_cwd.clone())
    }

    pub(super) fn schedule_status_line_command(&mut self, input: StatusLineCommandInput) {
        let Some(config) = self.config.tui_status_line_command.clone() else {
            return;
        };
        let Some(runtime) = self.status_line_command.as_mut() else {
            return;
        };
        if runtime.last_input.as_ref() == Some(&input) {
            return;
        }
        let Some(local_cwd) = runtime.local_cwd.clone() else {
            tracing::debug!("cannot run status-line command without a local process cwd");
            return;
        };
        let Ok(invocation) = runtime.lifecycle.begin(input.clone()) else {
            tracing::debug!("status-line command generation exhausted");
            return;
        };

        runtime.last_input = Some(input);
        runtime.cancel();
        let app_event_tx = self.app_event_tx.clone();
        let task = tokio::spawn(async move {
            tokio::time::sleep(STATUS_LINE_COMMAND_DEBOUNCE).await;
            let completion = execute_status_line_command(config, &local_cwd, invocation).await;
            app_event_tx.send(AppEvent::StatusLineCommandFinished(completion));
        });
        runtime.task = Some(task.abort_handle());
    }

    pub(crate) fn apply_status_line_command_completion(
        &mut self,
        completion: StatusLineCommandCompletion,
    ) -> bool {
        let failure = match &completion.outcome {
            StatusLineCommandOutcome::Success(_) => None,
            StatusLineCommandOutcome::Failure(failure) => Some(failure.clone()),
        };
        let (apply_result, lines) = {
            let Some(runtime) = self.status_line_command.as_mut() else {
                return false;
            };
            let apply_result = runtime.lifecycle.apply(completion);
            let lines = (apply_result == StatusLineCommandApplyResult::Updated)
                .then(|| runtime.lifecycle.last_good().cloned())
                .flatten()
                .map(|parsed| visible_lines(parsed.lines));
            (apply_result, lines)
        };

        match apply_result {
            StatusLineCommandApplyResult::Updated => {
                self.set_status_lines(lines.unwrap_or_default());
                self.set_status_line_hyperlink(/*url*/ None);
                true
            }
            StatusLineCommandApplyResult::RetainedLastGood => {
                if let Some(failure) = failure {
                    tracing::debug!(
                        kind = ?failure.kind,
                        error = %failure.message,
                        "external status-line command failed"
                    );
                }
                false
            }
            StatusLineCommandApplyResult::Stale => false,
        }
    }
}
