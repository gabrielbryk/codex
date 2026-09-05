use super::*;
use crate::status_line_command::wire::STATUS_LINE_COMMAND_SCHEMA_VERSION;
use crate::status_line_command::wire::StatusLineCommandCodex;
use crate::status_line_command::wire::StatusLineCommandContextWindow;
use crate::status_line_command::wire::StatusLineCommandModel;
use crate::status_line_command::wire::StatusLineCommandSessionId;
use crate::status_line_command::wire::StatusLineCommandThinking;
use crate::status_line_command::wire::StatusLineCommandWorkspace;

fn input() -> StatusLineCommandInput {
    StatusLineCommandInput {
        cwd: "/remote".to_string(),
        session_id: StatusLineCommandSessionId::new(),
        session_name: None,
        model: StatusLineCommandModel {
            id: "model".to_string(),
            display_name: "Model".to_string(),
        },
        workspace: StatusLineCommandWorkspace {
            current_dir: "/remote".to_string(),
            project_dir: None,
            added_dirs: Vec::new(),
            repo: None,
        },
        version: "0.153.4".to_string(),
        fast_mode: false,
        exceeds_200k_tokens: false,
        effort: None,
        thinking: StatusLineCommandThinking { enabled: false },
        context_window: StatusLineCommandContextWindow {
            total_input_tokens: 0,
            total_output_tokens: 0,
            context_window_size: 0,
            used_percentage: None,
            remaining_percentage: None,
            current_usage: None,
        },
        rate_limits: None,
        extra_usage: None,
        pr: None,
        codex: StatusLineCommandCodex {
            schema_version: STATUS_LINE_COMMAND_SCHEMA_VERSION,
            local_process_cwd: "/local".to_string(),
            status: "idle".to_string(),
            permissions: "read-only".to_string(),
            approval_mode: "on-request".to_string(),
            service_tier: "default".to_string(),
            workspace_headline: None,
            task_progress: None,
            git_branch: None,
            branch_changes: None,
        },
    }
}

fn completion(token: RequestToken, result: Result<&str, &str>) -> Completion {
    Completion {
        token,
        result: result
            .map(|value| {
                crate::status_line_command::parser::parse_status_line_command_output(
                    value.as_bytes(),
                )
                .expect("valid output")
            })
            .map_err(str::to_string),
    }
}

#[test]
fn lifecycle_rejects_stale_completions_and_retains_last_good() {
    let mut lifecycle = Lifecycle::new(7);
    let now = Instant::now();
    let old = lifecycle.begin(input(), now).expect("generation");
    let mut changed = input();
    changed.codex.status = "working".to_string();
    let newest = lifecycle.begin(changed, now).expect("generation");
    assert_eq!(
        lifecycle.apply(completion(old.token, Ok("old")), now),
        ApplyOutcome::Ignored
    );
    assert_eq!(
        lifecycle.apply(completion(newest.token, Ok("ready")), now),
        ApplyOutcome::Updated
    );
    assert_eq!(
        lifecycle.last_good().map(|parsed| parsed.lines[0].line.to_string()),
        Some("ready".to_string())
    );
}

#[test]
fn failed_unchanged_input_retries_after_backoff_then_deduplicates_success() {
    let mut lifecycle = Lifecycle::new(7);
    let now = Instant::now();
    let input = input();
    let failed = lifecycle
        .begin(input.clone(), now)
        .expect("initial invocation");
    assert_eq!(
        lifecycle.apply(completion(failed.token, Err("transient")), now),
        ApplyOutcome::RetryAt(now + RETRY_BACKOFF)
    );
    assert_eq!(lifecycle.begin(input.clone(), now), None);
    let retry_at = now + RETRY_BACKOFF;
    let recovered = lifecycle
        .begin(input.clone(), retry_at)
        .expect("retry after backoff");
    assert_eq!(
        lifecycle.apply(completion(recovered.token, Ok("recovered")), retry_at),
        ApplyOutcome::Updated
    );
    assert_eq!(lifecycle.begin(input, retry_at + RETRY_BACKOFF), None);
    assert_eq!(
        lifecycle.last_good().map(|parsed| parsed.lines[0].line.to_string()),
        Some("recovered".to_string())
    );
}
