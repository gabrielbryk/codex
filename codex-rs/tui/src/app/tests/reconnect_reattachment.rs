use super::*;
use crate::app::reconnect_reattachment::reattachment_failure_message;
use crate::app::thread_events::ThreadEventChannel;

#[tokio::test]
async fn reconnect_ignores_visible_but_unattached_thread_channels() {
    let mut app = make_test_app().await;
    let attached = ThreadId::new();
    let visible_only = ThreadId::new();
    app.attached_thread_ids.insert(attached);
    app.thread_event_channels
        .insert(visible_only, ThreadEventChannel::new(/*capacity*/ 4));

    pretty_assertions::assert_eq!(app.reconnect_thread_ids(), vec![attached]);
}

#[tokio::test]
async fn reconnect_reattaches_a_live_thread() -> Result<()> {
    let (mut app, _app_event_rx, _op_rx) = make_test_app_with_channels().await;
    let thread_id = ThreadId::from_string(
        &app_test_support::create_fake_rollout(
            app.config.codex_home.as_path(),
            "2026-08-12T20-00-00",
            "2026-08-12T20:00:00Z",
            "Reconnect primary",
            Some(&app.config.model_provider_id),
            /*git_info*/ None,
        )
        .expect("primary reconnect rollout should be created"),
    )?;
    let other_thread_id = ThreadId::from_string(
        &app_test_support::create_fake_rollout(
            app.config.codex_home.as_path(),
            "2026-08-12T20-00-01",
            "2026-08-12T20:00:01Z",
            "Reconnect navigation target",
            Some(&app.config.model_provider_id),
            /*git_info*/ None,
        )
        .expect("navigation reconnect rollout should be created"),
    )?;
    let mut app_server = Box::pin(crate::start_embedded_app_server_for_picker(&app.config)).await?;
    let started = app_server
        .resume_thread(
            app.config.clone(),
            thread_id,
            crate::app_server_session::ResumeModelSettings::RestoreFromThread,
        )
        .await?;
    let other_started = app_server
        .resume_thread(
            app.config.clone(),
            other_thread_id,
            crate::app_server_session::ResumeModelSettings::RestoreFromThread,
        )
        .await?;
    app.primary_thread_id = Some(thread_id);
    app.active_thread_id = Some(thread_id);
    app.attached_thread_ids.insert(thread_id);
    app.thread_event_channels.insert(
        thread_id,
        ThreadEventChannel::new_with_session(/*capacity*/ 4, started.session, started.turns),
    );
    app.agent_navigation.upsert(
        other_thread_id,
        Some("reconnect navigation target".to_string()),
        Some("test".to_string()),
        /*is_closed*/ false,
    );
    app.thread_event_channels.insert(
        other_thread_id,
        ThreadEventChannel::new_with_session(
            /*capacity*/ 4,
            other_started.session,
            other_started.turns,
        ),
    );
    app.active_thread_rx = app
        .thread_event_channels
        .get_mut(&thread_id)
        .expect("thread channel")
        .receiver
        .take();
    let stale_notification = codex_app_server_protocol::ServerNotification::TurnStarted(
        codex_app_server_protocol::TurnStartedNotification {
            thread_id: thread_id.to_string(),
            turn: codex_app_server_protocol::Turn {
                id: "stale-turn".to_string(),
                items: Vec::new(),
                items_view: codex_app_server_protocol::TurnItemsView::Full,
                status: codex_app_server_protocol::TurnStatus::InProgress,
                error: None,
                started_at: Some(1),
                completed_at: None,
                duration_ms: None,
            },
        },
    );
    {
        let channel = app
            .thread_event_channels
            .get(&thread_id)
            .expect("thread channel");
        channel
            .store
            .lock()
            .await
            .push_notification_ref(&stale_notification);
        channel
            .sender
            .send(
                crate::app::thread_events::ThreadBufferedEvent::Notification(Box::new(
                    stale_notification,
                )),
            )
            .await
            .expect("stale event should enqueue");
    }

    let failures = app
        .reattach_after_reconnect(
            &mut app_server,
            codex_app_server_client::ReconnectEpoch::FIRST,
            /*previous*/ None,
            /*current*/ None,
        )
        .await;
    pretty_assertions::assert_eq!(failures, Vec::<String>::new());

    let (session_thread_id, buffer_contains_stale_turn, active_turn_id) = {
        let store = app
            .thread_event_channels
            .get(&thread_id)
            .expect("thread channel")
            .store
            .lock()
            .await;
        (
            store.session.as_ref().map(|session| session.thread_id),
            store.buffer.iter().any(|event| {
                matches!(
                    event,
                    crate::app::thread_events::ThreadBufferedEvent::Notification(notification)
                        if matches!(
                            notification.as_ref(),
                            codex_app_server_protocol::ServerNotification::TurnStarted(notification)
                                if notification.turn.id == "stale-turn"
                        )
                )
            }),
            store.active_turn_id().map(str::to_string),
        )
    };
    let queued_event_count = app
        .active_thread_rx
        .as_mut()
        .expect("active test channel receiver")
        .len();
    pretty_assertions::assert_eq!(session_thread_id, Some(thread_id));
    pretty_assertions::assert_eq!(buffer_contains_stale_turn, false);
    pretty_assertions::assert_eq!(active_turn_id, None);
    pretty_assertions::assert_eq!(queued_event_count, 0);
    let mut tui = crate::tui::test_support::make_test_tui()?;
    app.select_agent_thread_with_reason(
        &mut tui,
        &mut app_server,
        other_thread_id,
        super::super::displayed_thread_transition::DisplayedThreadTransitionReason::AgentPickerSelection,
    )
    .await?;
    pretty_assertions::assert_eq!(app.current_displayed_thread_id(), Some(other_thread_id));
    app.select_agent_thread_with_reason(
        &mut tui,
        &mut app_server,
        thread_id,
        super::super::displayed_thread_transition::DisplayedThreadTransitionReason::AgentPickerSelection,
    )
    .await?;
    pretty_assertions::assert_eq!(app.current_displayed_thread_id(), Some(thread_id));
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
