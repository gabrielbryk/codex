use super::*;
use crate::app::reconnect_reattachment::reattachment_failure_message;
use crate::app::thread_events::ThreadEventChannel;

#[tokio::test]
async fn reconnect_reattaches_a_live_thread() -> Result<()> {
    let (mut app, _app_event_rx, _op_rx) = make_test_app_with_channels().await;
    let mut app_server = Box::pin(crate::start_embedded_app_server_for_picker(&app.config)).await?;
    let started = app_server.start_thread(&app.config).await?;
    let thread_id = started.session.thread_id;
    app.primary_thread_id = Some(thread_id);
    app.active_thread_id = Some(thread_id);
    app.thread_event_channels.insert(
        thread_id,
        ThreadEventChannel::new_with_session(
            /*capacity*/ 4,
            test_thread_session(thread_id, app.config.cwd.to_path_buf()),
            Vec::new(),
        ),
    );

    app.reattach_after_reconnect(
        &mut app_server,
        /*previous*/ None,
        /*current*/ None,
    )
    .await;

    let session_thread_id = {
        let store = app
            .thread_event_channels
            .get(&thread_id)
            .expect("thread channel")
            .store
            .lock()
            .await;
        store.session.as_ref().map(|session| session.thread_id)
    };
    pretty_assertions::assert_eq!(session_thread_id, Some(thread_id));
    app_server.shutdown().await?;
    Ok(())
}

#[test]
fn reconnect_failure_message_snapshot() {
    insta::assert_snapshot!(
        reattachment_failure_message(&[
            "thread-a: writer still owned by old generation".to_string(),
            "failed to release queued requests: connection closed".to_string(),
        ]),
        @r"
    App-server reconnected, but session reattachment was incomplete. Please retry your message.
    thread-a: writer still owned by old generation
    failed to release queued requests: connection closed
    "
    );
}
