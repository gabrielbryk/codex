//! External status-line command scheduling and completion application.

use std::path::PathBuf;

use tokio::task::AbortHandle;

use super::ChatWidget;
use super::next_status_line_async_owner;
use super::status_surfaces::approval_mode_display;
use super::status_surfaces::five_hour_status_window;
use super::status_surfaces::permissions_display;
use super::status_surfaces::weekly_status_window;
use crate::app_event::AppEvent;
use crate::status_line_command::process::execute_status_line_command;
use crate::status_line_command::runner::STATUS_LINE_COMMAND_DEBOUNCE;
use crate::status_line_command::runner::StatusLineCommandApplyResult;
use crate::status_line_command::runner::StatusLineCommandCompletion;
use crate::status_line_command::runner::StatusLineCommandLifecycle;
use crate::status_line_command::runner::StatusLineCommandOutcome;
use crate::status_line_command::wire::MAX_STATUS_LINE_WORKSPACE_HEADLINE_BYTES;
use crate::status_line_command::wire::STATUS_LINE_COMMAND_SCHEMA_VERSION;
use crate::status_line_command::wire::StatusLineCommandBranchChanges;
use crate::status_line_command::wire::StatusLineCommandCodex;
use crate::status_line_command::wire::StatusLineCommandContextWindow;
use crate::status_line_command::wire::StatusLineCommandCurrentUsage;
use crate::status_line_command::wire::StatusLineCommandEffort;
use crate::status_line_command::wire::StatusLineCommandInput;
use crate::status_line_command::wire::StatusLineCommandModel;
use crate::status_line_command::wire::StatusLineCommandPullRequest;
use crate::status_line_command::wire::StatusLineCommandRateLimitWindow;
use crate::status_line_command::wire::StatusLineCommandRateLimits;
use crate::status_line_command::wire::StatusLineCommandRepository;
use crate::status_line_command::wire::StatusLineCommandSessionId;
use crate::status_line_command::wire::StatusLineCommandThinking;
use crate::status_line_command::wire::StatusLineCommandWorkspace;
use crate::version::CODEX_CLI_VERSION;
use codex_protocol::config_types::ServiceTier;
use codex_protocol::openai_models::ReasoningEffort as ReasoningEffortConfig;

const MAX_STATUS_LINE_COMMAND_ATTEMPTS: u8 = 3;

pub(super) struct StatusLineCommandRuntime {
    lifecycle: StatusLineCommandLifecycle,
    local_cwd: Option<PathBuf>,
    last_input: Option<StatusLineCommandInput>,
    retry_input: Option<StatusLineCommandInput>,
    attempts: u8,
    task: Option<AbortHandle>,
}

impl StatusLineCommandRuntime {
    pub(super) fn new(local_cwd: Option<PathBuf>) -> Self {
        Self {
            lifecycle: StatusLineCommandLifecycle::new(),
            local_cwd,
            last_input: None,
            retry_input: None,
            attempts: 0,
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
    pub(super) fn reset_status_line_command_for_thread(&mut self) {
        let Some(runtime) = self.status_line_command.as_mut() else {
            return;
        };
        let local_cwd = runtime.local_cwd.clone();
        *runtime = StatusLineCommandRuntime::new(local_cwd);
        self.status_line_async_owner = next_status_line_async_owner();
        self.status_line_branch = None;
        self.status_line_branch_cwd = None;
        self.status_line_branch_pending = false;
        self.status_line_branch_lookup_complete = false;
        self.status_line_git_summary = None;
        self.status_line_git_summary_cwd = None;
        self.status_line_git_summary_pending = false;
        self.status_line_git_summary_lookup_complete = false;
        self.status_line_workspace_headline = None;
        self.status_line_workspace_headline_pending_request_id = None;
        self.status_line_workspace_headline_last_requested_at = None;
        self.status_line_workspace_messages_disabled = false;
        self.set_status_hyperlink_lines(Vec::new());
        self.set_status_line_hyperlink(/*url*/ None);
    }

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
        if runtime.retry_input.as_ref() != Some(&input) {
            runtime.retry_input = Some(input.clone());
            runtime.attempts = 0;
        }
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
        runtime.attempts = runtime.attempts.saturating_add(1);
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
        let (apply_result, lines, retry_input) = {
            let Some(runtime) = self.status_line_command.as_mut() else {
                return false;
            };
            let apply_result = runtime.lifecycle.apply(completion);
            let lines = (apply_result == StatusLineCommandApplyResult::Updated)
                .then(|| runtime.lifecycle.last_good().cloned())
                .flatten();
            let retry_input = if apply_result == StatusLineCommandApplyResult::RetainedLastGood
                && runtime.attempts < MAX_STATUS_LINE_COMMAND_ATTEMPTS
            {
                runtime.last_input.take()
            } else {
                None
            };
            if apply_result != StatusLineCommandApplyResult::Stale {
                runtime.task = None;
            }
            (apply_result, lines, retry_input)
        };

        let updated = match apply_result {
            StatusLineCommandApplyResult::Updated => {
                self.set_status_hyperlink_lines(lines.map_or_else(Vec::new, |parsed| parsed.lines));
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
        };
        if let Some(input) = retry_input {
            self.schedule_status_line_command(input);
        }
        updated
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
        let repo = git_summary
            .and_then(|summary| summary.repository.as_ref())
            .map(|repository| StatusLineCommandRepository {
                host: repository.host.clone(),
                owner: repository.owner.clone(),
                name: repository.name.clone(),
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
                repo,
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
                    cache_creation_input_tokens: usage.cache_write_input_tokens.max(0) as u64,
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
