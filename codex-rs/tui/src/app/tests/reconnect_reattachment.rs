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

/// Drives the `AppEvent`s a live `App` loop would consume so `transcript_cells`
/// reflects what the transcript actually renders.
fn drain_transcript_events(
    app: &mut App,
    tui: &mut crate::tui::Tui,
    app_event_rx: &mut tokio::sync::mpsc::UnboundedReceiver<AppEvent>,
) {
    while let Ok(event) = app_event_rx.try_recv() {
        match event {
            AppEvent::InsertHistoryCell(cell) => app.insert_history_cell(tui, cell),
            AppEvent::ConsolidateAgentMessage {
                source,
                cwd,
                inline_visualization_context,
                scrollback_reflow,
                deferred_history_cell,
                agent_message_item_id,
            } => app
                .handle_consolidate_agent_message(
                    tui,
                    AgentMessageConsolidation {
                        source,
                        cwd,
                        inline_visualization_context,
                        scrollback_reflow,
                        deferred_history_cell,
                        agent_message_item_id,
                    },
                )
                .expect("agent message consolidation should succeed"),
            _ => {}
        }
    }
}

fn transcript_text(app: &App) -> Vec<String> {
    app.transcript_cells
        .iter()
        .map(|cell| lines_to_single_string(&cell.transcript_lines(/*width*/ 200)))
        .collect()
}

/// Replayed by `reconcile_queued_events_after_session_refresh`: hook notifications
/// deliberately survive a session refresh, and delivering one finalizes the active
/// assistant stream mid-item.
fn reconnect_hook_started_notification(thread_id: ThreadId, turn_id: &str) -> ServerNotification {
    ServerNotification::HookStarted(codex_app_server_protocol::HookStartedNotification {
        thread_id: thread_id.to_string(),
        turn_id: Some(turn_id.to_string()),
        run: codex_app_server_protocol::HookRunSummary {
            id: "user-prompt-submit:0:/hooks.json".to_string(),
            event_name: codex_app_server_protocol::HookEventName::UserPromptSubmit,
            handler_type: codex_app_server_protocol::HookHandlerType::Command,
            execution_mode: codex_app_server_protocol::HookExecutionMode::Sync,
            scope: codex_app_server_protocol::HookScope::Turn,
            source_path: test_path_buf("/hooks.json").abs(),
            source: codex_app_server_protocol::HookSource::User,
            display_order: 0,
            status: codex_app_server_protocol::HookRunStatus::Running,
            status_message: Some("checking input policy".to_string()),
            started_at: 1,
            completed_at: None,
            duration_ms: None,
            entries: Vec::new(),
        },
    })
}

fn agent_message_completed_notification(
    thread_id: ThreadId,
    turn_id: &str,
    item_id: &str,
    text: String,
) -> ServerNotification {
    ServerNotification::ItemCompleted(codex_app_server_protocol::ItemCompletedNotification {
        thread_id: thread_id.to_string(),
        turn_id: turn_id.to_string(),
        completed_at_ms: 0,
        item: ThreadItem::AgentMessage {
            id: item_id.to_string(),
            text,
            phase: None,
            memory_citation: None,
        },
    })
}

struct StreamingReattachmentFixture {
    app: App,
    app_event_rx: tokio::sync::mpsc::UnboundedReceiver<AppEvent>,
    tui: crate::tui::Tui,
    app_server: AppServerSession,
    thread_id: ThreadId,
}

impl StreamingReattachmentFixture {
    async fn new(rollout_stamp: &str, rollout_time: &str) -> Result<Self> {
        let (mut app, app_event_rx, _op_rx) = make_test_app_with_channels().await;
        let tui = crate::tui::test_support::make_test_tui()?;
        let thread_id = ThreadId::from_string(
            &app_test_support::create_fake_rollout(
                app.config.codex_home.as_path(),
                rollout_stamp,
                rollout_time,
                "Reconnect streaming",
                Some(&app.config.model_provider_id),
                /*git_info*/ None,
            )
            .expect("streaming reconnect rollout should be created"),
        )?;
        let mut app_server =
            Box::pin(crate::start_embedded_app_server_for_picker(&app.config)).await?;
        let started = app_server
            .resume_thread(
                app.config.clone(),
                thread_id,
                crate::app_server_session::ResumeModelSettings::RestoreFromThread,
            )
            .await?;
        app.primary_thread_id = Some(thread_id);
        app.active_thread_id = Some(thread_id);
        app.attached_thread_ids.insert(thread_id);
        app.thread_event_channels.insert(
            thread_id,
            ThreadEventChannel::new_with_session(
                /*capacity*/ 16,
                started.session.clone(),
                started.turns.clone(),
            ),
        );
        app.active_thread_rx = app
            .thread_event_channels
            .get_mut(&thread_id)
            .expect("thread channel")
            .receiver
            .take();
        app.chat_widget.handle_thread_session_quiet(started.session);
        Ok(Self {
            app,
            app_event_rx,
            tui,
            app_server,
            thread_id,
        })
    }

    fn drain(&mut self) {
        drain_transcript_events(&mut self.app, &mut self.tui, &mut self.app_event_rx);
    }

    fn start_turn(&mut self, turn_id: &str) {
        self.app.chat_widget.handle_server_notification(
            ServerNotification::TurnStarted(TurnStartedNotification {
                thread_id: self.thread_id.to_string(),
                turn: test_turn(turn_id, TurnStatus::InProgress, Vec::new()),
            }),
            /*replay_kind*/ None,
        );
        self.drain();
        // Drop the session/turn framing cells so assertions describe the streamed item.
        self.app.transcript_cells.clear();
    }

    fn stream_counts(
        &mut self,
        turn_id: &str,
        item_id: &str,
        counts: std::ops::RangeInclusive<u32>,
    ) {
        for n in counts {
            self.app.chat_widget.handle_server_notification(
                super::agent_message_delta_notification(
                    self.thread_id,
                    turn_id,
                    item_id,
                    &format!("{n}\n"),
                ),
                /*replay_kind*/ None,
            );
        }
        self.drain();
    }

    fn complete_item(
        &mut self,
        turn_id: &str,
        item_id: &str,
        counts: std::ops::RangeInclusive<u32>,
    ) {
        let text: String = counts.map(|n| format!("{n}\n")).collect();
        self.app.chat_widget.handle_server_notification(
            agent_message_completed_notification(self.thread_id, turn_id, item_id, text),
            /*replay_kind*/ None,
        );
        self.drain();
    }
}

/// A transport sever mid-item must not make the transcript render the streamed
/// prefix a second time when the authoritative item completion arrives.
///
/// Reattachment replays the interactive/hook events that
/// `event_survives_session_refresh` preserves. Delivering one finalizes the live
/// assistant stream mid-item, which used to leave the already-rendered prefix
/// outside the trailing streaming run that consolidation replaces, so the
/// authoritative full text landed next to it.
#[tokio::test]
async fn reattachment_renders_a_streamed_item_exactly_once() -> Result<()> {
    let mut fixture =
        StreamingReattachmentFixture::new("2026-08-15T10-00-00", "2026-08-15T10:00:00Z").await?;
    fixture.start_turn("turn-stream");
    fixture.stream_counts("turn-stream", "item-stream", 1..=9);
    pretty_assertions::assert_eq!(
        transcript_text(&fixture.app).join("\n").contains("9"),
        true,
        "the streamed prefix should already be rendered before the sever"
    );

    let failures = fixture
        .app
        .reattach_after_reconnect(
            &mut fixture.app_server,
            codex_app_server_client::ReconnectEpoch::FIRST,
            /*previous*/ None,
            /*current*/ None,
        )
        .await;
    pretty_assertions::assert_eq!(failures, Vec::<String>::new());
    fixture.drain();

    fixture.app.handle_thread_event_now(
        crate::app::thread_events::ThreadBufferedEvent::Notification(Box::new(
            reconnect_hook_started_notification(fixture.thread_id, "turn-stream"),
        )),
    );
    fixture.drain();

    fixture.stream_counts("turn-stream", "item-stream", 10..=15);
    fixture.complete_item("turn-stream", "item-stream", 1..=15);

    let expected: String = (1..=15)
        .map(|n| format!("{n}"))
        .collect::<Vec<_>>()
        .join("\n");
    let rendered = transcript_text(&fixture.app);
    let assistant_cells = rendered
        .iter()
        .filter(|text| text.contains('1') && text.contains('9'))
        .count();
    pretty_assertions::assert_eq!(
        assistant_cells,
        1,
        "the streamed item must be rendered once, saw: {rendered:#?}"
    );
    pretty_assertions::assert_eq!(
        rendered
            .last()
            .map(|text| text.replace("• ", "").replace("  ", "")),
        Some(expected),
        "the surviving cell must carry the full authoritative message"
    );
    fixture.app_server.shutdown().await?;
    Ok(())
}

/// The de-duplication must not swing into dropping transcript content: an
/// unrelated notice written while the item was still streaming is not part of
/// the message and has been rendered nowhere else, so it must survive.
#[tokio::test]
async fn reattachment_keeps_transcript_notices_interleaved_with_a_streamed_item() -> Result<()> {
    let mut fixture =
        StreamingReattachmentFixture::new("2026-08-15T10-00-01", "2026-08-15T10:00:01Z").await?;
    fixture.start_turn("turn-notice");
    fixture.stream_counts("turn-notice", "item-notice", 1..=5);

    fixture
        .app
        .chat_widget
        .add_info_message("App-server reconnected".to_string(), /*hint*/ None);
    fixture.drain();

    fixture.stream_counts("turn-notice", "item-notice", 6..=8);
    fixture.complete_item("turn-notice", "item-notice", 1..=8);

    let rendered = transcript_text(&fixture.app);
    pretty_assertions::assert_eq!(
        rendered
            .iter()
            .filter(|text| text.contains("App-server reconnected"))
            .count(),
        1,
        "the interleaved notice must survive consolidation, saw: {rendered:#?}"
    );
    let assistant_cells = rendered
        .iter()
        .filter(|text| text.contains('8') && text.contains('1'))
        .count();
    pretty_assertions::assert_eq!(
        assistant_cells,
        1,
        "the streamed item must be rendered once, saw: {rendered:#?}"
    );
    fixture.app_server.shutdown().await?;
    Ok(())
}
