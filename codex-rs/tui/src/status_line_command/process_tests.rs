use pretty_assertions::assert_eq;
use tokio::sync::mpsc;

use super::*;
use crate::status_line_command::runner::StatusLineCommandLifecycle;
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

fn invocation() -> StatusLineCommandInvocation {
    StatusLineCommandLifecycle::new()
        .begin(input())
        .expect("generation available")
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
