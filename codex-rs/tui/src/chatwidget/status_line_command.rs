use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::path::PathBuf;
use std::time::Instant;

use tokio::task::AbortHandle;

use super::ChatWidget;
use super::status_surfaces::approval_mode_display;
use super::status_surfaces::five_hour_status_window;
use super::status_surfaces::permissions_display;
use super::status_surfaces::weekly_status_window;
use crate::app_event::AppEvent;
use crate::status_line_command::process::execute;
use crate::status_line_command::runner::ApplyOutcome;
use crate::status_line_command::runner::Completion;
use crate::status_line_command::runner::Lifecycle;
use crate::status_line_command::runner::RequestToken;
use crate::status_line_command::runner::STATUS_LINE_COMMAND_DEBOUNCE;
use crate::status_line_command::wire::*;
use crate::version::CODEX_CLI_VERSION;
use codex_protocol::config_types::ServiceTier;
use codex_protocol::openai_models::ReasoningEffort as ReasoningEffortConfig;

static NEXT_OWNER: AtomicU64 = AtomicU64::new(1);

pub(super) struct StatusLineCommandRuntime {
    lifecycle: Lifecycle,
    command_task: Option<AbortHandle>,
    retry_task: Option<AbortHandle>,
    in_flight: Option<(RequestToken, StatusLineCommandInput)>,
    local_cwd: Option<PathBuf>,
    session_id: StatusLineCommandSessionId,
}

impl StatusLineCommandRuntime {
    pub(super) fn new(local_cwd: Option<PathBuf>) -> Self {
        Self {
            lifecycle: Lifecycle::new(NEXT_OWNER.fetch_add(1, Ordering::Relaxed)),
            command_task: None,
            retry_task: None,
            in_flight: None,
            local_cwd,
            session_id: StatusLineCommandSessionId::new(),
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

    pub(super) fn lifecycle_session_id(&self) -> StatusLineCommandSessionId {
        self.session_id.clone()
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
        let local_cwd = runtime.local_cwd.clone();
        *runtime = StatusLineCommandRuntime::new(local_cwd);
        self.set_status_line(/*status_line*/ None);
    }

    pub(super) fn status_line_command_local_cwd(&self) -> Option<PathBuf> {
        self.status_line_command
            .as_ref()
            .and_then(|runtime| runtime.local_cwd.clone())
    }

    pub(super) fn schedule_status_line_command(&mut self) {
        let Some(config) = self.config.tui_status_line_command.clone() else {
            return;
        };
        let local_cwd = self
            .status_line_command
            .as_ref()
            .and_then(|runtime| runtime.local_cwd.clone())
            .unwrap_or_else(|| self.config.cwd.to_path_buf());
        let Some(input) = self.status_line_command_input() else {
            return;
        };
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
            tokio::time::sleep(STATUS_LINE_COMMAND_DEBOUNCE).await;
            let completion = execute(config, &local_cwd, invocation).await;
            tx.send(AppEvent::StatusLineCommandFinished(completion));
        });
        runtime.command_task = Some(task.abort_handle());
    }

    pub(crate) fn apply_status_line_command_completion(&mut self, completion: Completion) -> bool {
        let token = completion.token;
        let app_event_tx = self.app_event_tx.clone();
        let (outcome, lines) = {
            let Some(runtime) = self.status_line_command.as_mut() else {
                return false;
            };
            let outcome = runtime.lifecycle.apply(completion, Instant::now());
            let lines = (outcome == ApplyOutcome::Updated)
                .then(|| runtime.lifecycle.last_good().map(|parsed| parsed.lines.clone()))
                .flatten();
            if let ApplyOutcome::RetryAt(retry_at) = outcome {
                let Some((in_flight_token, input)) = runtime.in_flight.as_ref().cloned() else {
                    return false;
                };
                if in_flight_token != token {
                    return false;
                }
                runtime.schedule_retry(retry_at, token, input, app_event_tx);
            }
            (outcome, lines)
        };
        match outcome {
            ApplyOutcome::Ignored => false,
            ApplyOutcome::Updated => {
                self.set_status_hyperlink_lines(lines.unwrap_or_default());
                true
            }
            ApplyOutcome::RetryAt(_) => false,
        }
    }

    pub(crate) fn retry_status_line_command(
        &mut self,
        token: RequestToken,
        input: StatusLineCommandInput,
    ) -> bool {
        if self.status_line_command_input().as_ref() != Some(&input) {
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

    pub(super) fn status_line_command_input(&self) -> Option<StatusLineCommandInput> {
        let local_process_cwd = self.status_line_command_local_cwd()?;
        let session_cwd = self.status_line_cwd().to_string_lossy().into_owned();
        let project_dir = self.config.cwd.to_string_lossy().into_owned();
        let added_dirs = self
            .config
            .workspace_roots
            .iter()
            .filter(|root| *root != &self.config.cwd)
            .map(|root| root.to_string_lossy().into_owned())
            .collect();
        let effort = self.effective_reasoning_effort();
        let thinking_enabled = effort
            .as_ref()
            .is_some_and(|effort| effort != &ReasoningEffortConfig::None);
        let effort = effort.and_then(|effort| {
            (effort != ReasoningEffortConfig::None).then(|| StatusLineCommandEffort {
                level: effort.as_str().to_string(),
            })
        });
        let current_usage = self.token_info.as_ref().map(|info| &info.last_token_usage);
        let latest_usage = current_usage.cloned().unwrap_or_default();
        let context_window_size = self
            .status_line_context_window_size()
            .unwrap_or_default()
            .max(0) as u64;
        let (used_percentage, remaining_percentage) = if self.token_info.is_some() {
            (
                self.status_line_context_used_percent()
                    .map(|value| value as f64),
                self.status_line_context_remaining_percent()
                    .map(|value| value as f64),
            )
        } else {
            (None, None)
        };
        let git_summary = self.status_line_git_summary.as_ref();
        let pull_request = git_summary.and_then(|summary| summary.pull_request.as_ref());
        let pr = pull_request.map(|pull_request| StatusLineCommandPullRequest {
            number: pull_request.number,
            url: pull_request.url.clone(),
            review_state: None,
        });
        let branch_changes = git_summary
            .and_then(|summary| summary.branch_change_stats.as_ref())
            .map(|stats| StatusLineCommandBranchChanges {
                additions: stats.additions,
                deletions: stats.deletions,
            });
        let rate_limits = self
            .rate_limit_snapshots_by_limit_id
            .get("codex")
            .and_then(|snapshot| {
                let to_wire = |window: &crate::status::RateLimitWindowDisplay| {
                    window.resets_at_epoch_seconds.map(|resets_at| {
                        StatusLineCommandRateLimitWindow {
                            used_percentage: window.used_percent,
                            resets_at,
                        }
                    })
                };
                let five_hour =
                    five_hour_status_window(snapshot).and_then(|(window, _)| to_wire(window));
                let seven_day =
                    weekly_status_window(snapshot).and_then(|(window, _)| to_wire(window));
                (five_hour.is_some() || seven_day.is_some()).then_some(
                    StatusLineCommandRateLimits {
                        five_hour,
                        seven_day,
                    },
                )
            });

        Some(StatusLineCommandInput {
            cwd: session_cwd.clone(),
            session_id: self.status_line_command.as_ref()?.lifecycle_session_id(),
            session_name: self.thread_name.clone(),
            model: StatusLineCommandModel {
                id: self.current_model().to_string(),
                display_name: self.model_display_name().to_string(),
            },
            workspace: StatusLineCommandWorkspace {
                current_dir: session_cwd,
                project_dir: Some(project_dir),
                added_dirs,
                // The current native git summary does not expose repository identity.
                repo: None,
            },
            version: CODEX_CLI_VERSION.to_string(),
            fast_mode: self.current_service_tier() == Some(ServiceTier::Fast.request_value()),
            exceeds_200k_tokens: latest_usage.total_tokens.max(0) > 200_000,
            effort,
            thinking: StatusLineCommandThinking {
                enabled: thinking_enabled,
            },
            context_window: StatusLineCommandContextWindow {
                total_input_tokens: latest_usage.input_tokens.max(0) as u64,
                total_output_tokens: latest_usage.output_tokens.max(0) as u64,
                context_window_size,
                used_percentage,
                remaining_percentage,
                current_usage: current_usage.map(|usage| StatusLineCommandCurrentUsage {
                    input_tokens: usage.input_tokens.max(0) as u64,
                    output_tokens: usage.output_tokens.max(0) as u64,
                    // The target TUI model does not retain provider cache-write tokens.
                    cache_creation_input_tokens: 0,
                    cache_read_input_tokens: usage.cached_input_tokens.max(0) as u64,
                }),
            },
            rate_limits,
            extra_usage: None,
            pr,
            codex: StatusLineCommandCodex {
                schema_version: STATUS_LINE_COMMAND_SCHEMA_VERSION,
                local_process_cwd: local_process_cwd.to_string_lossy().into_owned(),
                status: self.run_state_status_text().to_lowercase(),
                permissions: permissions_display(&self.config),
                approval_mode: approval_mode_display(&self.config),
                service_tier: self.current_service_tier().unwrap_or("default").to_string(),
                workspace_headline: self
                    .status_line_workspace_headline
                    .as_deref()
                    .map(bounded_workspace_headline),
                task_progress: None,
                git_branch: self.status_line_branch.clone(),
                branch_changes,
            },
        })
    }
}

fn bounded_workspace_headline(headline: &str) -> String {
    let mut end = headline.len().min(MAX_STATUS_LINE_WORKSPACE_HEADLINE_BYTES);
    while !headline.is_char_boundary(end) {
        end = end.saturating_sub(/*rhs*/ 1);
    }
    headline[..end].to_string()
}
