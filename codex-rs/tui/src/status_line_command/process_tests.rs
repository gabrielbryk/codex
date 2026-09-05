use std::collections::HashMap;

use pretty_assertions::assert_eq;

use super::*;
use crate::status_line_command::runner::Lifecycle;
use crate::status_line_command::wire::*;

#[test]
fn formatter_environment_excludes_credentials_and_proxy_configuration() {
    let environment = formatter_environment([
        ("PATH".to_string(), "/bin".to_string()),
        ("HOME".to_string(), "/home/test".to_string()),
        ("CODEX_HOME".to_string(), "/home/test/.codex-uprising".to_string()),
        ("CCSTATUSLINE_BIN".to_string(), "/opt/ccstatusline".to_string()),
        ("OPENAI_API_KEY".to_string(), "secret".to_string()),
        ("AWS_SECRET_ACCESS_KEY".to_string(), "secret".to_string()),
        ("HTTPS_PROXY".to_string(), "secret".to_string()),
        ("CODEX_HOME".to_string(), "/private/codex".to_string()),
    ]);
    assert_eq!(
        environment,
        HashMap::from([
            ("PATH".into(), "/bin".into()),
            ("HOME".into(), "/home/test".into()),
            ("CODEX_HOME".into(), "/home/test/.codex-uprising".into()),
            ("CCSTATUSLINE_BIN".into(), "/opt/ccstatusline".into()),
        ])
    );
}

fn invocation() -> Invocation {
    Lifecycle::new(1)
        .begin(
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
                    total_input_tokens: 10,
                    total_output_tokens: 2,
                    context_window_size: 100,
                    used_percentage: Some(20.0),
                    remaining_percentage: Some(80.0),
                    current_usage: None,
                },
                rate_limits: None,
                extra_usage: None,
                pr: None,
                codex: StatusLineCommandCodex {
                    schema_version: STATUS_LINE_COMMAND_SCHEMA_VERSION,
                    local_process_cwd: "/local".to_string(),
                    status: "working".to_string(),
                    permissions: "read-only".to_string(),
                    approval_mode: "on-request".to_string(),
                    service_tier: "default".to_string(),
                    workspace_headline: None,
                    task_progress: None,
                    git_branch: None,
                    branch_changes: None,
                },
            },
            std::time::Instant::now(),
        )
        .expect("generation")
}

fn shell(script: &str, args: &[String], timeout_ms: u64) -> TuiStatusLineCommand {
    let mut command = vec!["/bin/sh".to_string(), "-c".to_string(), script.to_string()];
    command.extend_from_slice(args);
    TuiStatusLineCommand {
        command,
        timeout_ms,
    }
}

#[cfg(unix)]
#[tokio::test]
async fn command_receives_json_and_returns_one_row() {
    let config = shell(
        "IFS= read -r line; case \"$line\" in *working*) printf ready;; *) exit 9;; esac",
        &[],
        1_000,
    );
    let completion = execute(config, &std::env::current_dir().expect("cwd"), invocation()).await;
    assert_eq!(
        completion
            .result
            .expect("valid output")
            .lines[0]
            .line
            .to_string(),
        "ready"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn timeout_and_output_overflow_fail_closed() {
    let cwd = std::env::current_dir().expect("cwd");
    let timeout = shell("/bin/sleep 1", &[], 250);
    assert_eq!(
        execute(timeout, &cwd, invocation()).await.result,
        Err("formatter timed out".to_string())
    );
    let overflow = shell(
        "/usr/bin/head -c 8193 /dev/zero | /usr/bin/tr '\\0' x",
        &[],
        1_000,
    );
    assert_eq!(
        execute(overflow, &cwd, invocation()).await.result,
        Err("formatter stream exceeded 8192 bytes".to_string())
    );
}

#[cfg(unix)]
#[tokio::test]
async fn timeout_terminates_descendants_in_the_formatter_process_group() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let marker = tmp.path().join("descendant-finished");
    let config = shell(
        "(/bin/sleep 1; : > \"$1\") & wait",
        &[
            "status-line-command".to_string(),
            marker.to_string_lossy().into_owned(),
        ],
        250,
    );

    assert_eq!(
        execute(config, tmp.path(), invocation()).await.result,
        Err("formatter timed out".to_string())
    );
    tokio::time::sleep(Duration::from_millis(1_100)).await;
    assert!(!marker.exists(), "timed out descendant wrote its marker");
}
