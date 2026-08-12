use pretty_assertions::assert_eq;

use super::GenerationLifecycle;
use codex_app_server_protocol::ServerGenerationState;

#[tokio::test]
async fn drain_is_idempotent_for_the_same_replacement() {
    let lifecycle = GenerationLifecycle {
        generation: Some("old".to_string()),
        state: Default::default(),
    };

    lifecycle.begin("new".to_string()).await.expect("begin");
    lifecycle
        .begin("new".to_string())
        .await
        .expect("idempotent begin");

    assert_eq!(
        lifecycle.response(Vec::new(), Vec::new()).await,
        codex_app_server_protocol::ServerDrainResponse {
            generation: Some("old".to_string()),
            state: ServerGenerationState::Draining,
            replacement_generation: Some("new".to_string()),
            active_thread_ids: Vec::new(),
            loaded_thread_ids: Vec::new(),
            released_thread_ids: Vec::new(),
            cancellation_allowed: true,
        }
    );
}

#[tokio::test]
async fn drain_cannot_be_cancelled_after_a_writer_is_released() {
    let lifecycle = GenerationLifecycle {
        generation: Some("old".to_string()),
        state: Default::default(),
    };
    lifecycle.begin("new".to_string()).await.expect("begin");
    lifecycle.note_released("thread-1".to_string()).await;

    let error = lifecycle.cancel().await.expect_err("cancel must fail");

    assert_eq!(error.code, crate::error_code::INVALID_REQUEST_ERROR_CODE);
    assert_eq!(
        lifecycle
            .response(Vec::new(), Vec::new())
            .await
            .released_thread_ids,
        vec!["thread-1".to_string()]
    );
}

#[tokio::test]
async fn drain_waits_for_admitted_work_to_cross_its_creation_boundary() {
    let lifecycle = GenerationLifecycle {
        generation: Some("old".to_string()),
        state: Default::default(),
    };
    let permit = lifecycle.acquire_work_permit().await.expect("work permit");
    let lifecycle_for_drain = lifecycle.clone();
    let drain = tokio::spawn(async move { lifecycle_for_drain.begin("new".to_string()).await });

    tokio::task::yield_now().await;
    assert!(!drain.is_finished());
    drop(permit);
    drain.await.expect("drain task").expect("begin drain");
    assert!(lifecycle.is_draining().await);
}
