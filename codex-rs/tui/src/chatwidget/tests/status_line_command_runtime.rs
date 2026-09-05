use super::*;
use crate::chatwidget::status_line_command::StatusLineCommandRuntime;
use crate::status_line_command::runner::Completion;
use crate::status_line_command::runner::RequestToken;
use crate::status_line_command::wire::StatusLineCommandInput;
use codex_config::types::TuiStatusLineCommand;
use tokio::sync::mpsc::UnboundedReceiver;

async fn next_completion(events: &mut UnboundedReceiver<AppEvent>) -> Completion {
    loop {
        if let Some(AppEvent::StatusLineCommandFinished(completion)) = events.recv().await {
            return completion;
        }
    }
}

async fn next_retry(
    events: &mut UnboundedReceiver<AppEvent>,
) -> (RequestToken, StatusLineCommandInput) {
    loop {
        if let Some(AppEvent::StatusLineCommandRetry { token, input }) = events.recv().await {
            return (token, input);
        }
    }
}

fn thread_settings_for_status_command(
    thread_id: ThreadId,
    cwd: codex_utils_absolute_path::AbsolutePathBuf,
) -> codex_app_server_protocol::ThreadSettingsUpdatedNotification {
    codex_app_server_protocol::ThreadSettingsUpdatedNotification {
        thread_id: thread_id.to_string(),
        thread_settings: codex_app_server_protocol::ThreadSettings {
            cwd,
            approval_policy: AskForApproval::OnRequest,
            approvals_reviewer: codex_app_server_protocol::ApprovalsReviewer::User,
            sandbox_policy: codex_app_server_protocol::SandboxPolicy::ReadOnly {
                network_access: false,
            },
            active_permission_profile: None,
            model: "gpt-5.6-sol".to_string(),
            model_provider: "openai".to_string(),
            service_tier: None,
            effort: None,
            summary: None,
            collaboration_mode: CollaborationMode {
                mode: ModeKind::Default,
                settings: codex_protocol::config_types::Settings {
                    model: "gpt-5.6-sol".to_string(),
                    reasoning_effort: None,
                    developer_instructions: None,
                },
            },
            multi_agent_mode: Default::default(),
            personality: None,
        },
    }
}

fn thread_session_for_status_command(
    thread_id: ThreadId,
    cwd: codex_utils_absolute_path::AbsolutePathBuf,
) -> crate::session_state::ThreadSessionState {
    crate::session_state::ThreadSessionState {
        thread_id,
        forked_from_id: None,
        fork_parent_title: None,
        thread_name: None,
        model: "gpt-5.6-sol".to_string(),
        model_provider_id: "openai".to_string(),
        service_tier: None,
        approval_policy: AskForApproval::Never,
        approvals_reviewer: ApprovalsReviewer::User,
        permission_profile: PermissionProfile::read_only(),
        active_permission_profile: None,
        cwd: cwd.clone(),
        runtime_workspace_roots: vec![cwd],
        instruction_source_paths: Vec::new(),
        reasoning_effort: None,
        collaboration_mode: None,
        personality: None,
        message_history: None,
        network_proxy: None,
        rollout_path: None,
    }
}

#[cfg(unix)]
#[tokio::test]
async fn external_command_updates_the_existing_single_line_footer() {
    let (mut chat, mut events, _ops) = make_chatwidget_manual(/*model_override*/ None).await;
    let cwd = std::env::current_dir().expect("cwd");
    chat.current_cwd = Some(cwd.clone());
    chat.config.cwd = cwd.abs();
    chat.config.tui_status_line_command = Some(TuiStatusLineCommand {
        command: vec![
            "/bin/sh".to_string(),
            "-c".to_string(),
            "IFS= read -r line; case \"$line\" in *gpt-5.6-sol*) printf 'external ready';; *) exit 9;; esac"
                .to_string(),
        ],
        timeout_ms: 1_000,
    });
    chat.status_line_command = Some(StatusLineCommandRuntime::new(std::env::current_dir().ok()));

    chat.refresh_status_line();
    let completion = next_completion(&mut events).await;
    assert!(chat.apply_status_line_command_completion(completion));
    assert_chatwidget_snapshot!(
        "external_status_line_layout",
        render_bottom_popup(&chat, /*width*/ 80)
    );
}

#[tokio::test]
async fn command_input_uses_latest_context_usage_instead_of_session_total() {
    let (mut chat, _events, _ops) = make_chatwidget_manual(/*model_override*/ None).await;
    chat.status_line_command = Some(StatusLineCommandRuntime::new(std::env::current_dir().ok()));
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
                output_tokens: 12_000,
                total_tokens: 187_000,
                ..TokenUsage::default()
            },
            model_context_window: Some(272_000),
        }),
    );

    let input = chat.status_line_command_input().expect("command input");
    assert!(!input.exceeds_200k_tokens);
    assert_eq!(
        input.context_window.current_usage,
        Some(
            crate::status_line_command::wire::StatusLineCommandCurrentUsage {
                input_tokens: 175_000,
                output_tokens: 12_000,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 120_000,
            }
        )
    );
}

#[tokio::test]
async fn external_command_blocks_the_builtin_status_picker() {
    let (mut chat, mut events, _ops) = make_chatwidget_manual(/*model_override*/ None).await;
    chat.config.tui_status_line_command = Some(TuiStatusLineCommand {
        command: vec!["/formatter".to_string()],
        timeout_ms: 1_000,
    });

    chat.open_status_line_setup();

    let cell = loop {
        if let Some(AppEvent::InsertHistoryCell(cell)) = events.recv().await {
            break cell;
        }
    };
    insta::assert_snapshot!(
        lines_to_single_string(&cell.display_lines(/*width*/ 100)),
        @"• An external status command is configured. Remove `tui.status_line_command` to edit built-in status items."
    );
}

#[cfg(unix)]
#[tokio::test]
async fn unchanged_failure_automatically_retries_after_backoff() {
    let (mut chat, mut events, _ops) = make_chatwidget_manual(/*model_override*/ None).await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let attempts = tmp.path().join("attempts");
    chat.current_cwd = Some(tmp.path().to_path_buf());
    chat.config.cwd = tmp.path().to_path_buf().abs();
    chat.config.tui_status_line_command = Some(TuiStatusLineCommand {
        command: vec![
            "/bin/sh".to_string(),
            "-c".to_string(),
            "n=$(/bin/cat \"$1\" 2>/dev/null || printf 0); n=$((n+1)); printf '%s' \"$n\" > \"$1\"; [ \"$n\" -gt 1 ] && printf recovered"
                .to_string(),
            "status-line-command".to_string(),
            attempts.to_string_lossy().into_owned(),
        ],
        timeout_ms: 1_000,
    });
    chat.status_line_command = Some(StatusLineCommandRuntime::new(std::env::current_dir().ok()));

    chat.refresh_status_line();
    assert!(!chat.apply_status_line_command_completion(next_completion(&mut events).await));
    assert!(
        tokio::time::timeout(Duration::from_millis(50), events.recv())
            .await
            .is_err(),
        "retry timer fired before backoff"
    );
    let (token, input) = next_retry(&mut events).await;
    assert!(chat.retry_status_line_command(token, input));
    assert!(chat.apply_status_line_command_completion(next_completion(&mut events).await));
    assert_eq!(chat.status_line_text().as_deref(), Some("recovered"));
    assert_eq!(
        std::fs::read_to_string(attempts).expect("attempt count"),
        "2"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn stale_input_and_widget_retry_timers_are_suppressed() {
    let (mut chat, mut events, _ops) = make_chatwidget_manual(/*model_override*/ None).await;
    let tmp = tempfile::tempdir().expect("tempdir");
    chat.current_cwd = Some(tmp.path().to_path_buf());
    chat.config.tui_status_line_command = Some(TuiStatusLineCommand {
        command: vec![
            "/bin/sh".to_string(),
            "-c".to_string(),
            "exit 9".to_string(),
        ],
        timeout_ms: 1_000,
    });
    chat.status_line_command = Some(StatusLineCommandRuntime::new(std::env::current_dir().ok()));

    chat.refresh_status_line();
    assert!(!chat.apply_status_line_command_completion(next_completion(&mut events).await));
    let (token, input) = next_retry(&mut events).await;
    chat.current_cwd = Some(tmp.path().join("changed"));
    assert!(!chat.retry_status_line_command(token, input.clone()));

    chat.current_cwd = Some(tmp.path().to_path_buf());
    chat.reset_status_line_command();
    assert!(!chat.retry_status_line_command(token, input));
    assert!(
        tokio::time::timeout(Duration::from_millis(50), events.recv())
            .await
            .is_err(),
        "stale retry launched a formatter"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn settings_and_thread_switch_keep_formatter_in_local_process_cwd() {
    let (mut chat, mut events, _ops) = make_chatwidget_manual(/*model_override*/ None).await;
    let local_cwd = std::env::current_dir().expect("local cwd");
    let tmp = tempfile::tempdir().expect("tempdir");
    let settings_cwd = tmp.path().join("settings-cwd");
    let switched_cwd = tmp.path().join("switched-cwd");
    std::fs::create_dir_all(&settings_cwd).expect("settings cwd");
    std::fs::create_dir_all(&switched_cwd).expect("switched cwd");
    chat.config.tui_status_line_command = Some(TuiStatusLineCommand {
        command: vec!["/bin/sh".to_string(), "-c".to_string(), "pwd".to_string()],
        timeout_ms: 1_000,
    });
    chat.status_line_command = Some(StatusLineCommandRuntime::new(std::env::current_dir().ok()));

    let thread_id = ThreadId::new();
    chat.thread_id = Some(thread_id);
    let settings = thread_settings_for_status_command(thread_id, settings_cwd.clone().abs());
    chat.handle_server_notification(
        ServerNotification::ThreadSettingsUpdated(settings),
        /*replay_kind*/ None,
    );
    assert!(chat.apply_status_line_command_completion(next_completion(&mut events).await));
    assert_eq!(
        chat.status_line_text().as_deref(),
        Some(local_cwd.to_string_lossy().as_ref())
    );

    chat.handle_thread_session(thread_session_for_status_command(
        ThreadId::new(),
        switched_cwd.clone().abs(),
    ));
    assert!(chat.apply_status_line_command_completion(next_completion(&mut events).await));
    assert_eq!(
        chat.status_line_text().as_deref(),
        Some(local_cwd.to_string_lossy().as_ref())
    );
}
