use codex_app_server_client::AppServerEvent;

use super::*;
use pretty_assertions::assert_eq;

/// A fatal exit must tell the user how to get back into the session, not just print the transport
/// error (openai/codex#33976).
#[tokio::test]
async fn disconnected_fatal_exit_includes_resume_hint() -> Result<()> {
    let (mut app, mut app_event_rx, _op_rx) = Box::pin(make_test_app_with_channels()).await;
    let app_server = Box::pin(crate::start_embedded_app_server_for_picker(
        app.chat_widget.config_ref(),
    ))
    .await?;
    let thread_id = ThreadId::new();
    app.active_thread_id = Some(thread_id);
    while app_event_rx.try_recv().is_ok() {}

    Box::pin(app.handle_app_server_event(
        &app_server,
        AppServerEvent::Disconnected {
            message: "app-server connection closed".to_string(),
        },
    ))
    .await;

    let mut fatal_messages = Vec::new();
    while let Ok(event) = app_event_rx.try_recv() {
        if let AppEvent::FatalExitRequest(message) = event {
            fatal_messages.push(message);
        }
    }
    assert_eq!(
        fatal_messages,
        vec![format!(
            "app-server connection closed\nResume this session with: codex resume {thread_id}"
        )]
    );

    app_server.shutdown().await?;
    Ok(())
}
