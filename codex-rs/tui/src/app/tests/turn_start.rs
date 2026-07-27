use codex_app_server_protocol::AskForApproval;
use codex_app_server_protocol::UserInput;

use super::*;

/// A failed `turn/start` must stay recoverable: the app-server websocket can drop and reconnect
/// underneath an in-flight request, and that must not tear down the TUI.
#[tokio::test]
async fn failed_turn_start_reports_error_instead_of_exiting() -> Result<()> {
    let (mut app, mut app_event_rx, _op_rx) = Box::pin(make_test_app_with_channels()).await;
    let mut app_server = Box::pin(crate::start_embedded_app_server_for_picker(
        app.chat_widget.config_ref(),
    ))
    .await?;
    // The app-server never started this thread, so `turn/start` fails at the request level.
    let thread_id = ThreadId::new();
    let op = Op::user_turn(
        vec![UserInput::Text {
            text: "hello".to_string(),
            text_elements: Vec::new(),
        }],
        app.chat_widget.config_ref().cwd.to_path_buf(),
        AskForApproval::Never,
        /*active_permission_profile*/ None,
        "gpt-5.1-codex".to_string(),
        /*effort*/ None,
        /*summary*/ None,
        /*service_tier*/ None,
        /*final_output_json_schema*/ None,
        /*collaboration_mode*/ None,
        /*personality*/ None,
    );
    while app_event_rx.try_recv().is_ok() {}

    Box::pin(app.submit_thread_op(&mut app_server, thread_id, op)).await?;

    let mut rendered_cells = Vec::new();
    while let Ok(event) = app_event_rx.try_recv() {
        if let AppEvent::InsertHistoryCell(cell) = event {
            rendered_cells.push(lines_to_single_string(&cell.display_lines(/*width*/ 200)));
        }
    }
    assert!(
        rendered_cells
            .iter()
            .any(|cell| cell.contains("Failed to start turn")
                && cell.contains("please resend your message")),
        "expected a turn/start failure message, got {rendered_cells:?}"
    );

    app_server.shutdown().await?;
    Ok(())
}

/// The other app-server thread ops in `try_submit_active_thread_op_via_app_server` must be
/// recoverable too: a failed request renders a chat error instead of exiting the TUI.
#[tokio::test]
async fn failed_thread_compact_start_reports_error_instead_of_exiting() -> Result<()> {
    let (mut app, mut app_event_rx, _op_rx) = Box::pin(make_test_app_with_channels()).await;
    let mut app_server = Box::pin(crate::start_embedded_app_server_for_picker(
        app.chat_widget.config_ref(),
    ))
    .await?;
    // The app-server never started this thread, so `thread/compact/start` fails at the request
    // level.
    let thread_id = ThreadId::new();
    while app_event_rx.try_recv().is_ok() {}

    Box::pin(app.submit_thread_op(&mut app_server, thread_id, Op::Compact)).await?;

    let mut rendered_cells = Vec::new();
    while let Ok(event) = app_event_rx.try_recv() {
        if let AppEvent::InsertHistoryCell(cell) = event {
            rendered_cells.push(lines_to_single_string(&cell.display_lines(/*width*/ 200)));
        }
    }
    assert!(
        rendered_cells
            .iter()
            .any(|cell| cell.contains("Failed to compact the conversation")
                && cell.contains("please try again")),
        "expected a thread/compact/start failure message, got {rendered_cells:?}"
    );

    app_server.shutdown().await?;
    Ok(())
}
