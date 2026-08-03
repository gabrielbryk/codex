use pretty_assertions::assert_eq;

use super::*;
use crate::status_line_command::parser::parse_status_line_command_output;
use crate::status_line_command::wire::*;

fn input() -> StatusLineCommandInput {
    StatusLineCommandInput {
        cwd: "/remote".to_string(),
        session_id: StatusLineCommandSessionId::new(),
        session_name: None,
        model: StatusLineCommandModel {
            id: "gpt-5.6-sol".to_string(),
            display_name: "Sol".to_string(),
        },
        workspace: StatusLineCommandWorkspace {
            current_dir: "/remote".to_string(),
            project_dir: None,
            added_dirs: Vec::new(),
            repo: None,
        },
        version: "0.146.0".to_string(),
        fast_mode: false,
        exceeds_200k_tokens: false,
        effort: None,
        thinking: StatusLineCommandThinking { enabled: false },
        context_window: StatusLineCommandContextWindow {
            total_input_tokens: 0,
            total_output_tokens: 0,
            context_window_size: 272_000,
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

fn success(token: StatusLineCommandRequestToken, text: &[u8]) -> StatusLineCommandCompletion {
    StatusLineCommandCompletion {
        token,
        outcome: StatusLineCommandOutcome::Success(
            parse_status_line_command_output(text).expect("valid output"),
        ),
    }
}

#[test]
fn lifecycle_owns_session_identity_and_increments_generations() {
    let mut lifecycle = StatusLineCommandLifecycle::new();
    let session_id = lifecycle.session_id().clone();
    let first = lifecycle.begin(input()).expect("generation available");
    let second = lifecycle.begin(input()).expect("generation available");

    assert_eq!(first.input.session_id, session_id);
    assert_eq!(second.input.session_id, session_id);
    assert_eq!(first.token.owner_id, second.token.owner_id);
    assert_eq!(first.token.generation, StatusLineCommandGeneration(1));
    assert_eq!(second.token.generation, StatusLineCommandGeneration(2));
}

#[test]
fn stale_generation_cannot_replace_newest_result() {
    let mut lifecycle = StatusLineCommandLifecycle::new();
    let first = lifecycle.begin(input()).expect("generation available");
    let second = lifecycle.begin(input()).expect("generation available");

    assert_eq!(
        lifecycle.apply(success(first.token, b"old")),
        StatusLineCommandApplyResult::Stale
    );
    assert_eq!(
        lifecycle.apply(success(second.token, b"new")),
        StatusLineCommandApplyResult::Updated
    );
    assert_eq!(
        lifecycle.last_good().expect("last good").lines[0]
            .line
            .to_string(),
        "new"
    );
}

#[test]
fn foreign_owner_cannot_update_replacement_widget() {
    let mut old = StatusLineCommandLifecycle::new();
    let old_request = old.begin(input()).expect("generation available");
    let mut replacement = StatusLineCommandLifecycle::new();
    replacement.begin(input()).expect("generation available");

    assert_eq!(
        replacement.apply(success(old_request.token, b"late")),
        StatusLineCommandApplyResult::Stale
    );
    assert_eq!(replacement.last_good(), None);
}

#[test]
fn newest_failure_retains_last_good_result() {
    let mut lifecycle = StatusLineCommandLifecycle::new();
    let first = lifecycle.begin(input()).expect("generation available");
    assert_eq!(
        lifecycle.apply(success(first.token, b"good")),
        StatusLineCommandApplyResult::Updated
    );
    let second = lifecycle.begin(input()).expect("generation available");
    let failure = StatusLineCommandCompletion {
        token: second.token,
        outcome: StatusLineCommandOutcome::Failure(StatusLineCommandFailure {
            kind: StatusLineCommandFailureKind::Timeout,
            message: "timed out".to_string(),
        }),
    };

    assert_eq!(
        lifecycle.apply(failure),
        StatusLineCommandApplyResult::RetainedLastGood
    );
    assert_eq!(
        lifecycle.last_good().expect("last good").lines[0]
            .line
            .to_string(),
        "good"
    );
}
