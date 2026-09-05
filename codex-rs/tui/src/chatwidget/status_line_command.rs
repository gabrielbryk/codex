use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Instant;

use ratatui::text::Line;
use tokio::task::AbortHandle;

use super::ChatWidget;
use crate::app_event::AppEvent;
use crate::status_line_command::process::execute;
use crate::status_line_command::runner::ApplyOutcome;
use crate::status_line_command::runner::Completion;
use crate::status_line_command::runner::Lifecycle;
use crate::status_line_command::runner::RequestToken;
use crate::status_line_command::wire::StatusLineCommandInput;

static NEXT_OWNER: AtomicU64 = AtomicU64::new(1);

pub(super) struct StatusLineCommandRuntime {
    lifecycle: Lifecycle,
    command_task: Option<AbortHandle>,
    retry_task: Option<AbortHandle>,
    in_flight: Option<(RequestToken, StatusLineCommandInput)>,
}

impl StatusLineCommandRuntime {
    pub(super) fn new() -> Self {
        Self {
            lifecycle: Lifecycle::new(NEXT_OWNER.fetch_add(1, Ordering::Relaxed)),
            command_task: None,
            retry_task: None,
            in_flight: None,
        }
    }

    fn cancel_tasks(&mut self) {
        if let Some(task) = self.command_task.take() {
            task.abort();
        }
        if let Some(task) = self.retry_task.take() {
            task.abort();
        }
    }

    fn schedule_retry(
        &mut self,
        retry_at: Instant,
        token: RequestToken,
        input: StatusLineCommandInput,
        tx: crate::app_event_sender::AppEventSender,
    ) {
        if let Some(task) = self.retry_task.take() {
            task.abort();
        }
        let task = tokio::spawn(async move {
            tokio::time::sleep_until(retry_at.into()).await;
            tx.send(AppEvent::StatusLineCommandRetry { token, input });
        });
        self.retry_task = Some(task.abort_handle());
    }
}

impl Drop for StatusLineCommandRuntime {
    fn drop(&mut self) {
        self.cancel_tasks();
    }
}

impl ChatWidget {
    pub(super) fn reset_status_line_command(&mut self) {
        let Some(runtime) = self.status_line_command.as_mut() else {
            return;
        };
        *runtime = StatusLineCommandRuntime::new();
        self.set_status_line(/*status_line*/ None);
    }

    pub(super) fn schedule_status_line_command(&mut self) {
        let Some(config) = self.config.tui_status_line_command.clone() else {
            return;
        };
        let cwd = self
            .current_cwd
            .clone()
            .unwrap_or_else(|| self.config.cwd.to_path_buf());
        let input = self.status_line_command_input();
        let Some(runtime) = self.status_line_command.as_mut() else {
            return;
        };
        let Some(invocation) = runtime.lifecycle.begin(input, Instant::now()) else {
            return;
        };
        runtime.cancel_tasks();
        runtime.in_flight = Some((invocation.token, invocation.input.clone()));
        let tx = self.app_event_tx.clone();
        let task = tokio::spawn(async move {
            let completion = execute(config, &cwd, invocation).await;
            tx.send(AppEvent::StatusLineCommandFinished(completion));
        });
        runtime.command_task = Some(task.abort_handle());
    }

    pub(crate) fn apply_status_line_command_completion(&mut self, completion: Completion) -> bool {
        let Some(runtime) = self.status_line_command.as_mut() else {
            return false;
        };
        let token = completion.token;
        match runtime.lifecycle.apply(completion, Instant::now()) {
            ApplyOutcome::Ignored => false,
            ApplyOutcome::Updated => {
                let line = runtime.lifecycle.last_good().map(str::to_string);
                self.set_status_line(line.map(Line::from));
                true
            }
            ApplyOutcome::RetryAt(retry_at) => {
                let Some((in_flight_token, input)) = runtime.in_flight.as_ref().cloned() else {
                    return false;
                };
                if in_flight_token != token {
                    return false;
                }
                runtime.schedule_retry(retry_at, token, input, self.app_event_tx.clone());
                false
            }
        }
    }

    pub(crate) fn retry_status_line_command(
        &mut self,
        token: RequestToken,
        input: StatusLineCommandInput,
    ) -> bool {
        if self.status_line_command_input() != input {
            return false;
        }
        let Some(runtime) = self.status_line_command.as_ref() else {
            return false;
        };
        let Some((in_flight_token, in_flight_input)) = runtime.in_flight.as_ref() else {
            return false;
        };
        if *in_flight_token != token || *in_flight_input != input {
            return false;
        }
        self.schedule_status_line_command();
        true
    }

    fn status_line_command_input(&self) -> StatusLineCommandInput {
        let usage = self.token_info.as_ref().map(|info| &info.last_token_usage);
        let context_remaining_percent = self.token_info.as_ref().and_then(|info| {
            info.model_context_window.map(|window| {
                info.last_token_usage
                    .percent_of_context_window_remaining(window)
            })
        });
        StatusLineCommandInput {
            cwd: self
                .current_cwd
                .as_deref()
                .unwrap_or(self.config.cwd.as_path())
                .to_string_lossy()
                .into_owned(),
            model: self.current_model().to_string(),
            status: self.run_state_status_text().to_lowercase(),
            input_tokens: usage.map_or(0, |usage| usage.input_tokens.max(0) as u64),
            output_tokens: usage.map_or(0, |usage| usage.output_tokens.max(0) as u64),
            context_remaining_percent,
        }
    }
}
