use std::time::Duration;

use anyhow::Result;
use app_test_support::DEFAULT_CLIENT_NAME;
use app_test_support::MockResponsesConfig;
use app_test_support::TestAppServer;
use app_test_support::create_mock_responses_server_repeating_assistant;
use codex_app_server_protocol::ClientInfo;
use codex_app_server_protocol::ClientRequest;
use codex_app_server_protocol::InitializeCapabilities;
use codex_app_server_protocol::JSONRPCMessage;
use codex_app_server_protocol::RequestId;
use codex_app_server_protocol::ServerDrainResponse;
use codex_app_server_protocol::ServerDrainStartParams;
use codex_app_server_protocol::ServerGenerationState;
use codex_app_server_protocol::ThreadStartParams;
use pretty_assertions::assert_eq;
use tempfile::TempDir;
use tokio::time::timeout;

const READ_TIMEOUT: Duration = Duration::from_secs(20);

#[tokio::test]
async fn drain_releases_idle_thread_and_rejects_new_work() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;
    let codex_home = TempDir::new()?;
    MockResponsesConfig::new(&server.uri()).write(codex_home.path())?;
    let mut app_server = TestAppServer::builder()
        .with_codex_home(codex_home.path())
        .build()
        .await?;
    let initialization = app_server
        .initialize_with_capabilities(
            ClientInfo {
                name: DEFAULT_CLIENT_NAME.to_string(),
                title: None,
                version: "0.1.0".to_string(),
            },
            Some(InitializeCapabilities {
                experimental_api: true,
                ..Default::default()
            }),
        )
        .await?;
    assert!(matches!(initialization, JSONRPCMessage::Response(_)));
    let thread = app_server
        .start_thread(ThreadStartParams::default())
        .await?
        .thread;

    let drain: ServerDrainResponse = app_server
        .request(|request_id| ClientRequest::ServerDrainStart {
            request_id,
            params: ServerDrainStartParams {
                replacement_generation: "replacement".to_string(),
            },
        })
        .await?;

    assert_eq!(drain.state, ServerGenerationState::Draining);
    assert_eq!(drain.active_thread_ids, Vec::<String>::new());
    assert_eq!(drain.loaded_thread_ids, Vec::<String>::new());
    assert_eq!(drain.released_thread_ids, vec![thread.id]);
    assert!(!drain.cancellation_allowed);

    let request_id = app_server
        .send_raw_request(
            "thread/start",
            Some(serde_json::to_value(ThreadStartParams::default())?),
        )
        .await?;
    let error = timeout(
        READ_TIMEOUT,
        app_server.read_stream_until_error_message(RequestId::Integer(request_id)),
    )
    .await??;
    assert_eq!(error.error.code, -32002);
    assert_eq!(
        error.error.data.as_ref().and_then(|data| data.get("type")),
        Some(&serde_json::json!("serverDraining"))
    );

    let cancel_id = app_server
        .send_raw_request("server/drain/cancel", Some(serde_json::json!({})))
        .await?;
    let cancel_error = timeout(
        READ_TIMEOUT,
        app_server.read_stream_until_error_message(RequestId::Integer(cancel_id)),
    )
    .await??;
    assert_eq!(cancel_error.error.code, -32600);
    assert_eq!(
        cancel_error.error.message,
        "drain cannot be cancelled after a thread writer was released"
    );

    Ok(())
}
