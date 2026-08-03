use pretty_assertions::assert_eq;
use tokio::sync::mpsc;

use super::*;
use crate::status_line_command::runner::StatusLineCommandApplyResult;
use crate::status_line_command::runner::StatusLineCommandLifecycle;
use crate::status_line_command::wire::*;

const EXPORTER_V1_FULL_FIXTURE: &str = include_str!("fixtures/exporter_v1_full.json");

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

fn exporter_fixture_input() -> StatusLineCommandInput {
    let mut input = input();
    input.cwd = "/remote/workspace".to_string();
    input.session_id = uuid::Uuid::parse_str("11111111-2222-4333-8444-555555555555")
        .expect("valid UUID")
        .into();
    input.session_name = Some("Status work".to_string());
    input.model = StatusLineCommandModel {
        id: "gpt-5.6-terra".to_string(),
        display_name: "Terra".to_string(),
    };
    input.workspace = StatusLineCommandWorkspace {
        current_dir: "/remote/workspace".to_string(),
        project_dir: Some("/remote".to_string()),
        added_dirs: vec!["/remote/shared".to_string()],
        repo: Some(StatusLineCommandRepository {
            host: "github.com".to_string(),
            owner: "openai".to_string(),
            name: "codex".to_string(),
        }),
    };
    input.fast_mode = true;
    input.effort = Some(StatusLineCommandEffort {
        level: "high".to_string(),
    });
    input.thinking.enabled = true;
    input.context_window.total_input_tokens = 10;
    input.context_window.total_output_tokens = 20;
    input.context_window.used_percentage = Some(25.0);
    input.context_window.remaining_percentage = Some(75.0);
    input.context_window.current_usage = Some(StatusLineCommandCurrentUsage {
        input_tokens: 100,
        output_tokens: 20,
        cache_creation_input_tokens: 10,
        cache_read_input_tokens: 5,
    });
    input.rate_limits = Some(StatusLineCommandRateLimits {
        five_hour: Some(StatusLineCommandRateLimitWindow {
            used_percentage: 10.0,
            resets_at: 1_800_000_000,
        }),
        seven_day: Some(StatusLineCommandRateLimitWindow {
            used_percentage: 20.0,
            resets_at: 1_800_500_000,
        }),
    });
    input.extra_usage = Some(StatusLineCommandExtraUsage {
        enabled: true,
        used: 2.5,
        limit: 50.0,
    });
    input.pr = Some(StatusLineCommandPullRequest {
        number: 42,
        url: "https://github.com/openai/codex/pull/42".to_string(),
        review_state: Some("approved".to_string()),
    });
    input.codex.local_process_cwd = "/local/codex".to_string();
    input.codex.status = "working".to_string();
    input.codex.permissions = "workspace-write".to_string();
    input.codex.service_tier = "fast".to_string();
    input.codex.workspace_headline = Some("Implementing status line".to_string());
    input.codex.task_progress = Some(StatusLineCommandTaskProgress {
        completed: 2,
        total: 3,
    });
    input.codex.git_branch = Some("feature/status-line".to_string());
    input.codex.branch_changes = Some(StatusLineCommandBranchChanges {
        additions: 12,
        deletions: 3,
    });
    input
}

fn invocation() -> StatusLineCommandInvocation {
    StatusLineCommandLifecycle::new()
        .begin(input())
        .expect("generation available")
}

#[cfg(unix)]
fn shell_config(script: impl Into<String>, args: Vec<String>) -> TuiStatusLineCommand {
    TuiStatusLineCommand {
        command: [
            vec!["/bin/sh".to_string(), "-c".to_string(), script.into()],
            args,
        ]
        .concat(),
        timeout_ms: 3_000,
    }
}

fn failure_kind(completion: StatusLineCommandCompletion) -> StatusLineCommandFailureKind {
    let StatusLineCommandOutcome::Failure(failure) = completion.outcome else {
        panic!("expected formatter failure");
    };
    failure.kind
}

#[tokio::test]
async fn bounded_stream_accepts_exact_limit() {
    let (sender, receiver) = mpsc::channel(2);
    sender
        .send(vec![b'a'; MAX_STATUS_LINE_COMMAND_BYTES])
        .await
        .expect("receiver alive");
    drop(sender);

    assert_eq!(
        collect_bounded(receiver).await.expect("within limit").len(),
        MAX_STATUS_LINE_COMMAND_BYTES
    );
}

#[tokio::test]
async fn bounded_stream_rejects_overflow_across_chunks() {
    let (sender, receiver) = mpsc::channel(2);
    sender
        .send(vec![b'a'; MAX_STATUS_LINE_COMMAND_BYTES])
        .await
        .expect("receiver alive");
    sender.send(vec![b'b']).await.expect("receiver alive");
    drop(sender);

    let err = collect_bounded(receiver).await.expect_err("over limit");
    assert_eq!(err.kind, StatusLineCommandFailureKind::OutputLimit);
}

#[cfg(unix)]
#[tokio::test]
async fn command_receives_json_and_returns_styled_output() {
    let config = TuiStatusLineCommand {
        command: vec![
            "/bin/sh".to_string(),
            "-c".to_string(),
            concat!(
                "IFS= read -r input; ",
                "case \"$input\" in ",
                "*gpt-5.6-sol*) printf '\\033[32mok\\033[0m' ;; ",
                "*) exit 9 ;; esac"
            )
            .to_string(),
        ],
        timeout_ms: 1_000,
    };

    let completion =
        execute_status_line_command(config, &std::env::current_dir().unwrap(), invocation()).await;
    let StatusLineCommandOutcome::Success(parsed) = completion.outcome else {
        panic!("expected successful formatter output");
    };
    assert_eq!(parsed.lines[0].line.to_string(), "ok");
}

#[cfg(unix)]
#[tokio::test]
async fn command_runs_in_local_process_cwd_while_json_reports_remote_cwd() {
    let local = tempfile::tempdir().expect("local cwd");
    let local = local.path().canonicalize().expect("canonical local cwd");
    let config = shell_config(
        concat!(
            "IFS= read -r input; ",
            "case \"$input\" in ",
            "*'\"cwd\":\"/remote\"'*) pwd ;; ",
            "*) exit 9 ;; esac"
        ),
        Vec::new(),
    );

    let completion = execute_status_line_command(config, &local, invocation()).await;
    let StatusLineCommandOutcome::Success(parsed) = completion.outcome else {
        panic!("expected successful formatter output");
    };
    assert_eq!(
        parsed.lines[0].line.to_string(),
        local.to_string_lossy().into_owned()
    );
}

#[cfg(unix)]
#[tokio::test]
async fn exporter_fixture_round_trips_through_real_command_stdin_and_stdout() {
    let temp = tempfile::tempdir().expect("temp dir");
    let captured_input = temp.path().join("captured-input.json");
    let config = shell_config(
        concat!(
            "IFS= read -r input; printf '%s\\n' \"$input\" > \"$1\"; ",
            "printf '\\033[1;36mTerra\\033[0m (high) · 25%%\\n",
            "feature/status-line · \\033]8;;https://github.com/openai/codex/pull/42\\007",
            "PR #42\\033]8;;\\007'"
        ),
        vec![
            "status-line-test".to_string(),
            captured_input.to_string_lossy().into_owned(),
        ],
    );

    let outcome = execute(config, temp.path(), exporter_fixture_input()).await;
    let StatusLineCommandOutcome::Success(parsed) = outcome else {
        panic!("expected successful exporter fixture output");
    };
    let expected_value = serde_json::from_str::<serde_json::Value>(EXPORTER_V1_FULL_FIXTURE)
        .expect("valid exporter fixture");
    let mut expected_bytes = serde_json::to_vec(&expected_value).expect("serialize fixture");
    expected_bytes.push(b'\n');

    assert_eq!(std::fs::read(captured_input).unwrap(), expected_bytes);
    assert_eq!(
        parsed
            .lines
            .iter()
            .map(|line| line.line.to_string())
            .collect::<Vec<_>>(),
        vec!["Terra (high) · 25%", "feature/status-line · PR #42"]
    );
    assert_eq!(
        parsed.lines[1].hyperlinks[0].destination,
        "https://github.com/openai/codex/pull/42"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn real_stdout_and_stderr_overflow_are_reported() {
    let local_cwd = std::env::current_dir().expect("current dir");
    let overflow = format!(
        "i=0; while [ \"$i\" -le {MAX_STATUS_LINE_COMMAND_BYTES} ]; do printf x; i=$((i + 1)); done"
    );
    let stdout = execute_status_line_command(
        shell_config(overflow.clone(), Vec::new()),
        &local_cwd,
        invocation(),
    )
    .await;
    let stderr = execute_status_line_command(
        shell_config(format!("{{ {overflow}; }} >&2"), Vec::new()),
        &local_cwd,
        invocation(),
    )
    .await;

    assert_eq!(
        (failure_kind(stdout), failure_kind(stderr)),
        (
            StatusLineCommandFailureKind::OutputLimit,
            StatusLineCommandFailureKind::OutputLimit
        )
    );
}

#[cfg(unix)]
#[tokio::test]
async fn real_runner_failure_retains_last_good_output() {
    let local_cwd = std::env::current_dir().expect("current dir");
    let mut lifecycle = StatusLineCommandLifecycle::new();
    let first = lifecycle
        .begin(input())
        .expect("first generation available");
    let first =
        execute_status_line_command(shell_config("printf good", Vec::new()), &local_cwd, first)
            .await;
    assert_eq!(
        lifecycle.apply(first),
        StatusLineCommandApplyResult::Updated
    );

    let overflow = format!(
        "i=0; while [ \"$i\" -le {MAX_STATUS_LINE_COMMAND_BYTES} ]; do printf x; i=$((i + 1)); done"
    );
    let second = lifecycle
        .begin(input())
        .expect("second generation available");
    let second =
        execute_status_line_command(shell_config(overflow, Vec::new()), &local_cwd, second).await;
    assert_eq!(
        lifecycle.apply(second),
        StatusLineCommandApplyResult::RetainedLastGood
    );
    assert_eq!(
        lifecycle.last_good().expect("last good").lines[0]
            .line
            .to_string(),
        "good"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn timeout_terminates_formatter_process_group() {
    let temp = tempfile::tempdir().expect("temp dir");
    let sentinel = temp.path().join("group-child-survived");
    let mut config = shell_config(
        "(sleep 1; printf survived > \"$1\") & wait",
        vec![
            "status-line-test".to_string(),
            sentinel.to_string_lossy().into_owned(),
        ],
    );
    config.timeout_ms = 250;

    let completion = execute_status_line_command(
        config,
        &std::env::current_dir().expect("current dir"),
        invocation(),
    )
    .await;
    assert_eq!(
        failure_kind(completion),
        StatusLineCommandFailureKind::Timeout
    );
    tokio::time::sleep(Duration::from_millis(1_200)).await;
    assert!(
        !sentinel.exists(),
        "formatter process-group child survived timeout"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn command_timeout_is_reported() {
    let config = TuiStatusLineCommand {
        command: vec![
            "/bin/sh".to_string(),
            "-c".to_string(),
            "sleep 2".to_string(),
        ],
        timeout_ms: 250,
    };

    let completion =
        execute_status_line_command(config, &std::env::current_dir().unwrap(), invocation()).await;
    let StatusLineCommandOutcome::Failure(failure) = completion.outcome else {
        panic!("expected timeout failure");
    };
    assert_eq!(failure.kind, StatusLineCommandFailureKind::Timeout);
}
