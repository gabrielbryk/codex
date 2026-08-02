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
