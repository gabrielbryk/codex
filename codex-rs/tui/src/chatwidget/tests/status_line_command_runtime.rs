use super::*;
use crate::chatwidget::status_line_command::StatusLineCommandRuntime;
use crate::chatwidget::status_state::TerminalTitleStatusKind;
use crate::status_line_command::runner::StatusLineCommandCompletion;
use codex_config::types::TuiStatusLineCommand;
use pretty_assertions::assert_eq;

fn install_command(chat: &mut ChatWidget, command: Vec<String>) {
    chat.config.tui_status_line_command = Some(TuiStatusLineCommand {
        command,
        timeout_ms: 1_000,
    });
    chat.status_line_command = Some(StatusLineCommandRuntime::new(std::env::current_dir().ok()));
}

fn switched_thread_session(
    cwd: AbsolutePathBuf,
    runtime_workspace_roots: Vec<AbsolutePathBuf>,
) -> crate::session_state::ThreadSessionState {
    crate::session_state::ThreadSessionState {
        thread_id: ThreadId::new(),
        forked_from_id: None,
        fork_parent_title: None,
        thread_name: None,
        model: "gpt-5.6-sol".to_string(),
        model_provider_id: "openai".to_string(),
        service_tier: None,
        approval_policy: codex_app_server_protocol::AskForApproval::Never,
        approvals_reviewer: ApprovalsReviewer::User,
        permission_profile: PermissionProfile::read_only(),
        active_permission_profile: None,
        runtime_workspace_roots,
        cwd,
        instruction_source_paths: Vec::new(),
        reasoning_effort: None,
        collaboration_mode: None,
        personality: None,
        message_history: None,
        network_proxy: None,
        rollout_path: None,
    }
}

async fn next_completion(
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<AppEvent>,
) -> StatusLineCommandCompletion {
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            match rx.recv().await {
                Some(AppEvent::StatusLineCommandFinished(completion)) => break completion,
                Some(_) => {}
                None => panic!("app event channel closed"),
            }
        }
    })
    .await
    .expect("status-line command completion")
}

#[cfg(unix)]
#[tokio::test]
async fn transient_failures_retry_until_success() {
    let temp = tempfile::tempdir().expect("temp dir");
    let counter = temp.path().join("attempts");
    let (mut chat, mut rx, _op_rx) = make_chatwidget_manual(/*model_override*/ None).await;
    install_command(
        &mut chat,
        vec![
            "/bin/sh".to_string(),
            "-c".to_string(),
            concat!(
                "count_file=$1; ",
                "count=$(cat \"$count_file\" 2>/dev/null || printf 0); ",
                "count=$((count + 1)); printf '%s' \"$count\" > \"$count_file\"; ",
                "if [ \"$count\" -lt 3 ]; then exit 7; fi; printf recovered"
            )
            .to_string(),
            "status-line-test".to_string(),
            counter.to_string_lossy().into_owned(),
        ],
    );

    chat.refresh_status_line();
    assert!(!chat.apply_status_line_command_completion(next_completion(&mut rx).await));
    assert!(!chat.apply_status_line_command_completion(next_completion(&mut rx).await));
    assert!(chat.apply_status_line_command_completion(next_completion(&mut rx).await));

    assert_eq!(status_line_text(&chat), Some("recovered".to_string()));
    assert_eq!(std::fs::read_to_string(counter).unwrap(), "3");
}

#[cfg(unix)]
#[tokio::test]
async fn thread_reset_clears_last_good_and_session_identity() {
    let (mut chat, mut rx, _op_rx) = make_chatwidget_manual(/*model_override*/ None).await;
    install_command(
        &mut chat,
        vec![
            "/bin/sh".to_string(),
            "-c".to_string(),
            "printf old-thread".to_string(),
        ],
    );
    let old_session_id = chat
        .status_line_command
        .as_ref()
        .expect("runtime")
        .lifecycle_session_id();
    chat.refresh_status_line();
    assert!(chat.apply_status_line_command_completion(next_completion(&mut rx).await));
    assert_eq!(status_line_text(&chat), Some("old-thread".to_string()));

    chat.reset_status_line_command_for_thread();

    let new_session_id = chat
        .status_line_command
        .as_ref()
        .expect("runtime")
        .lifecycle_session_id();
    assert_ne!(new_session_id, old_session_id);
    assert_eq!(status_line_text(&chat), None);
}

#[cfg(unix)]
#[tokio::test]
async fn thread_reset_during_debounce_prevents_formatter_spawn() {
    let temp = tempfile::tempdir().expect("temp dir");
    let old_cwd = temp.path().join("old-thread");
    let new_cwd = temp.path().join("new-thread");
    std::fs::create_dir_all(&old_cwd).expect("old cwd");
    std::fs::create_dir_all(&new_cwd).expect("new cwd");
    let old_cwd = old_cwd.abs();
    let new_cwd = new_cwd.abs();
    let sentinel = temp.path().join("old-formatter-spawned");
    let (mut chat, mut rx, _op_rx) = make_chatwidget_manual(/*model_override*/ None).await;
    install_command(
        &mut chat,
        vec![
            "/bin/sh".to_string(),
            "-c".to_string(),
            concat!(
                "IFS= read -r input; ",
                "if printf '%s' \"$input\" | grep -F \"\\\"cwd\\\":\\\"$1\\\"\" >/dev/null; then ",
                "printf spawned > \"$3\"; printf old-thread; ",
                "elif printf '%s' \"$input\" | grep -F \"\\\"cwd\\\":\\\"$2\\\"\" >/dev/null; then ",
                "printf new-thread; else exit 9; fi"
            )
            .to_string(),
            "status-line-test".to_string(),
            old_cwd.to_string_lossy().into_owned(),
            new_cwd.to_string_lossy().into_owned(),
            sentinel.to_string_lossy().into_owned(),
        ],
    );
    chat.thread_id = Some(ThreadId::new());
    chat.current_cwd = Some(old_cwd.to_path_buf());
    chat.config.cwd = old_cwd.clone();
    chat.config.workspace_roots = vec![old_cwd];

    chat.refresh_status_line();
    chat.handle_thread_session(switched_thread_session(new_cwd.clone(), vec![new_cwd]));
    let completion = next_completion(&mut rx).await;

    assert!(!sentinel.exists(), "formatter spawned after thread reset");
    assert!(chat.apply_status_line_command_completion(completion));
    assert_eq!(status_line_text(&chat), Some("new-thread".to_string()));
}

#[cfg(unix)]
#[tokio::test]
async fn stale_completion_does_not_detach_newer_process_from_cancellation() {
    let temp = tempfile::tempdir().expect("temp dir");
    let old_cwd = temp.path().join("old-thread");
    let new_cwd = temp.path().join("new-thread");
    std::fs::create_dir_all(&old_cwd).expect("old cwd");
    std::fs::create_dir_all(&new_cwd).expect("new cwd");
    let old_cwd = old_cwd.abs();
    let new_cwd = new_cwd.abs();
    let sentinel = temp.path().join("newer-completed");
    let (mut chat, mut rx, _op_rx) = make_chatwidget_manual(/*model_override*/ None).await;
    install_command(
        &mut chat,
        vec![
            "/bin/sh".to_string(),
            "-c".to_string(),
            concat!(
                "IFS= read -r input; ",
                "if printf '%s' \"$input\" | grep -F \"\\\"cwd\\\":\\\"$2\\\"\" >/dev/null; then ",
                "printf new-thread; ",
                "elif printf '%s' \"$input\" | grep -F '\"status\":\"working\"' >/dev/null; then ",
                "sleep 1; printf done > \"$3\"; printf newer; ",
                "else printf older; fi"
            )
            .to_string(),
            "status-line-test".to_string(),
            old_cwd.to_string_lossy().into_owned(),
            new_cwd.to_string_lossy().into_owned(),
            sentinel.to_string_lossy().into_owned(),
        ],
    );
    chat.thread_id = Some(ThreadId::new());
    chat.current_cwd = Some(old_cwd.to_path_buf());
    chat.config.cwd = old_cwd.clone();
    chat.config.workspace_roots = vec![old_cwd];
    chat.status_state.terminal_title_status_kind = TerminalTitleStatusKind::Working;
    chat.refresh_status_line();
    let older = next_completion(&mut rx).await;

    chat.bottom_pane.set_task_running(/*running*/ true);
    chat.set_status_header("Working".to_string());
    tokio::time::sleep(std::time::Duration::from_millis(450)).await;
    assert!(!chat.apply_status_line_command_completion(older));
    chat.handle_thread_session(switched_thread_session(new_cwd.clone(), vec![new_cwd]));
    let new_thread = next_completion(&mut rx).await;
    assert!(chat.apply_status_line_command_completion(new_thread));
    tokio::time::sleep(std::time::Duration::from_millis(1_200)).await;

    assert!(!sentinel.exists());
    assert_eq!(status_line_text(&chat), Some("new-thread".to_string()));
}

#[tokio::test]
async fn same_cwd_thread_reset_rejects_old_dependency_results() {
    let (mut chat, _rx, _op_rx) = make_chatwidget_manual(/*model_override*/ None).await;
    install_command(&mut chat, vec!["formatter".to_string()]);
    let cwd = PathBuf::from("/same-cwd");
    let old_owner = chat.status_line_async_owner;
    chat.status_line_branch = Some("old-branch".to_string());
    chat.status_line_git_summary = Some(StatusLineGitSummary::default());
    chat.status_line_workspace_headline = Some("old headline".to_string());
    chat.status_line_workspace_headline_pending_request_id = Some(10);

    chat.reset_status_line_command_for_thread();
    assert_ne!(chat.status_line_async_owner, old_owner);
    assert_eq!(chat.status_line_branch, None);
    assert!(chat.status_line_git_summary.is_none());
    assert_eq!(chat.status_line_workspace_headline, None);
    assert_eq!(chat.status_line_workspace_headline_pending_request_id, None);

    chat.status_line_branch_cwd = Some(cwd.clone());
    chat.status_line_branch_pending = true;
    chat.status_line_git_summary_cwd = Some(cwd.clone());
    chat.status_line_git_summary_pending = true;
    chat.status_line_workspace_headline_pending_request_id = Some(11);
    chat.set_status_line_branch(old_owner, cwd.clone(), Some("stale-branch".to_string()));
    chat.set_status_line_git_summary(old_owner, cwd, StatusLineGitSummary::default());
    assert!(!chat.set_status_line_workspace_headline(
        old_owner,
        10,
        Ok(
            crate::workspace_messages::WorkspaceHeadlineFetchResult::Available(Some(
                "stale headline".to_string(),
            )),
        ),
    ));

    assert_eq!(chat.status_line_branch, None);
    assert!(chat.status_line_git_summary.is_none());
    assert_eq!(chat.status_line_workspace_headline, None);
    assert!(chat.status_line_branch_pending);
    assert!(chat.status_line_git_summary_pending);
    assert_eq!(
        chat.status_line_workspace_headline_pending_request_id,
        Some(11)
    );
}

#[tokio::test]
async fn replacement_widget_rejects_old_widgets_same_cwd_dependency_results() {
    let (old_chat, _old_rx, _old_op_rx) = make_chatwidget_manual(/*model_override*/ None).await;
    let old_owner = old_chat.status_line_async_owner;
    let (mut replacement, _rx, _op_rx) = make_chatwidget_manual(/*model_override*/ None).await;
    install_command(&mut replacement, vec!["formatter".to_string()]);
    assert_ne!(replacement.status_line_async_owner, old_owner);
    let cwd = PathBuf::from("/same-cwd");
    replacement.status_line_branch_cwd = Some(cwd.clone());
    replacement.status_line_branch_pending = true;
    replacement.status_line_git_summary_cwd = Some(cwd.clone());
    replacement.status_line_git_summary_pending = true;

    replacement.set_status_line_branch(
        old_owner,
        cwd.clone(),
        Some("old-widget-branch".to_string()),
    );
    replacement.set_status_line_git_summary(old_owner, cwd, StatusLineGitSummary::default());

    assert_eq!(replacement.status_line_branch, None);
    assert!(replacement.status_line_git_summary.is_none());
    assert!(replacement.status_line_branch_pending);
    assert!(replacement.status_line_git_summary_pending);
}

#[tokio::test]
async fn command_input_uses_latest_context_usage_instead_of_session_total() {
    let (mut chat, _rx, _op_rx) = make_chatwidget_manual(/*model_override*/ None).await;
    install_command(&mut chat, vec!["formatter".to_string()]);
    handle_token_count(
        &mut chat,
        Some(TokenUsageInfo {
            total_token_usage: TokenUsage {
                input_tokens: 900_000,
                output_tokens: 80_000,
                total_tokens: 980_000,
                ..TokenUsage::default()
            },
            last_token_usage: TokenUsage {
                input_tokens: 175_000,
                cached_input_tokens: 120_000,
                cache_write_input_tokens: 4_000,
                output_tokens: 12_000,
                total_tokens: 187_000,
                ..TokenUsage::default()
            },
            model_context_window: Some(272_000),
        }),
    );

    let input = chat.status_line_command_input().expect("command input");
    assert!(!input.exceeds_200k_tokens);
    assert_eq!(input.context_window.total_input_tokens, 175_000);
    assert_eq!(input.context_window.total_output_tokens, 12_000);
    assert_eq!(
        input.context_window.current_usage,
        Some(
            crate::status_line_command::wire::StatusLineCommandCurrentUsage {
                input_tokens: 175_000,
                output_tokens: 12_000,
                cache_creation_input_tokens: 4_000,
                cache_read_input_tokens: 120_000,
            }
        )
    );
}

#[tokio::test]
async fn command_input_reports_origin_repository_without_a_pull_request() {
    let (mut chat, _rx, _op_rx) = make_chatwidget_manual(/*model_override*/ None).await;
    install_command(&mut chat, vec!["formatter".to_string()]);
    chat.status_line_git_summary = Some(StatusLineGitSummary {
        repository: Some(crate::branch_summary::StatusLineRepository {
            host: "github.com".to_string(),
            owner: "openai".to_string(),
            name: "codex".to_string(),
        }),
        pull_request: None,
        branch_change_stats: None,
    });

    let input = chat.status_line_command_input().expect("command input");
    assert_eq!(
        input.workspace.repo,
        Some(
            crate::status_line_command::wire::StatusLineCommandRepository {
                host: "github.com".to_string(),
                owner: "openai".to_string(),
                name: "codex".to_string(),
            }
        )
    );
    assert_eq!(input.pr, None);
}

#[tokio::test]
async fn formatter_stdin_reports_resumed_workspace_without_local_cwd_leak() {
    let temp = tempfile::tempdir().expect("temp dir");
    let local_formatter_cwd = temp.path().join("local-formatter-cwd");
    let project_dir = temp.path().join("remote-resumed-project");
    let shared_dir = temp.path().join("remote-shared");
    let tools_dir = temp.path().join("remote-tools");
    for path in [&local_formatter_cwd, &project_dir, &shared_dir, &tools_dir] {
        std::fs::create_dir_all(path).expect("workspace directory");
    }
    let local_formatter_cwd =
        dunce::canonicalize(local_formatter_cwd).expect("canonical local formatter cwd");
    let captured_input = temp.path().join("captured-input.json");
    let captured_cwd = temp.path().join("captured-cwd.txt");
    #[cfg(unix)]
    let command = vec![
        "/bin/sh".to_string(),
        "-c".to_string(),
        concat!(
            "IFS= read -r input; printf '%s\\n' \"$input\" > \"$1\"; ",
            "pwd > \"$2\"; printf ok"
        )
        .to_string(),
        "status-line-test".to_string(),
        captured_input.to_string_lossy().into_owned(),
        captured_cwd.to_string_lossy().into_owned(),
    ];
    #[cfg(windows)]
    let command = vec![
        "powershell.exe".to_string(),
        "-NoLogo".to_string(),
        "-NoProfile".to_string(),
        "-NonInteractive".to_string(),
        "-Command".to_string(),
        concat!(
            "& { param($captured_input, $captured_cwd) ",
            "$input_json = [Console]::In.ReadToEnd(); ",
            "$utf8 = [Text.UTF8Encoding]::new($false); ",
            "[IO.File]::WriteAllText($captured_input, $input_json, $utf8); ",
            "[IO.File]::WriteAllText($captured_cwd, (Get-Location).Path, $utf8); ",
            "[Console]::Out.Write('ok') }"
        )
        .to_string(),
        captured_input.to_string_lossy().into_owned(),
        captured_cwd.to_string_lossy().into_owned(),
    ];
    let (mut chat, mut rx, _op_rx) = make_chatwidget_manual(/*model_override*/ None).await;
    install_command(&mut chat, command);
    chat.status_line_command = Some(StatusLineCommandRuntime::new(Some(
        local_formatter_cwd.clone(),
    )));
    let project_dir = project_dir.abs();
    let shared_dir = shared_dir.abs();
    let tools_dir = tools_dir.abs();
    chat.handle_thread_session(switched_thread_session(
        project_dir.clone(),
        vec![project_dir.clone(), shared_dir.clone(), tools_dir.clone()],
    ));
    let completion = next_completion(&mut rx).await;
    assert!(chat.apply_status_line_command_completion(completion));
    assert_eq!(status_line_text(&chat), Some("ok".to_string()));

    let stdin = std::fs::read(captured_input).expect("captured formatter stdin");
    let input: serde_json::Value = serde_json::from_slice(&stdin).expect("formatter stdin JSON");
    assert_eq!(input["cwd"], project_dir.to_string_lossy().into_owned());
    assert_eq!(
        input["workspace"],
        serde_json::json!({
            "current_dir": project_dir.to_string_lossy().into_owned(),
            "project_dir": project_dir.to_string_lossy().into_owned(),
            "added_dirs": [
                shared_dir.to_string_lossy().into_owned(),
                tools_dir.to_string_lossy().into_owned(),
            ],
        })
    );
    assert_eq!(
        input["codex"]["local_process_cwd"],
        local_formatter_cwd.to_string_lossy().into_owned()
    );
    assert_ne!(
        input["workspace"]["current_dir"],
        input["codex"]["local_process_cwd"]
    );
    let actual_formatter_cwd =
        std::fs::read_to_string(captured_cwd).expect("captured formatter cwd");
    assert_eq!(
        dunce::canonicalize(actual_formatter_cwd.trim_end()).expect("canonical formatter cwd"),
        local_formatter_cwd
    );
}

#[tokio::test]
async fn command_input_bounds_backend_workspace_headline_on_utf8_boundaries() {
    let (mut chat, _rx, _op_rx) = make_chatwidget_manual(/*model_override*/ None).await;
    install_command(&mut chat, vec!["formatter".to_string()]);
    chat.status_line_workspace_headline = Some("é".repeat(1_000));

    let headline = chat
        .status_line_command_input()
        .expect("command input")
        .codex
        .workspace_headline
        .expect("workspace headline");
    assert_eq!(
        headline.len(),
        crate::status_line_command::wire::MAX_STATUS_LINE_WORKSPACE_HEADLINE_BYTES
    );
    assert_eq!(headline.chars().count(), 512);
}

#[cfg(unix)]
#[tokio::test]
async fn run_state_change_refreshes_command_input() {
    let (mut chat, mut rx, _op_rx) = make_chatwidget_manual(/*model_override*/ None).await;
    install_command(
        &mut chat,
        vec![
            "/bin/sh".to_string(),
            "-c".to_string(),
            concat!(
                "IFS= read -r input; ",
                "case \"$input\" in ",
                "*'\"status\":\"working\"'*) printf working ;; ",
                "*) printf ready ;; esac"
            )
            .to_string(),
        ],
    );
    chat.status_state.terminal_title_status_kind = TerminalTitleStatusKind::Working;
    chat.refresh_status_line();
    assert!(chat.apply_status_line_command_completion(next_completion(&mut rx).await));
    assert_eq!(status_line_text(&chat), Some("ready".to_string()));

    chat.bottom_pane.set_task_running(/*running*/ true);
    chat.set_status_header("Working".to_string());
    assert!(chat.apply_status_line_command_completion(next_completion(&mut rx).await));

    assert_eq!(status_line_text(&chat), Some("working".to_string()));
}

#[tokio::test]
async fn command_input_contains_raw_rate_limit_reset_epochs() {
    let (mut chat, _rx, _op_rx) = make_chatwidget_manual(/*model_override*/ None).await;
    install_command(&mut chat, vec!["formatter".to_string()]);
    chat.on_rate_limit_snapshot(Some(RateLimitSnapshot {
        limit_id: None,
        limit_name: None,
        primary: Some(RateLimitWindow {
            used_percent: 25,
            window_duration_mins: Some(300),
            resets_at: Some(1_900_000_001),
        }),
        secondary: Some(RateLimitWindow {
            used_percent: 50,
            window_duration_mins: Some(10_080),
            resets_at: Some(1_900_000_002),
        }),
        credits: None,
        individual_limit: None,
        spend_control_reached: None,
        plan_type: None,
        rate_limit_reached_type: None,
    }));

    let limits = chat
        .status_line_command_input()
        .expect("command input")
        .rate_limits
        .expect("rate limits");
    assert_eq!(
        limits.five_hour.expect("five-hour window").resets_at,
        1_900_000_001
    );
    assert_eq!(
        limits.seven_day.expect("seven-day window").resets_at,
        1_900_000_002
    );
}
