//! Restores connection-scoped thread subscriptions after app-server reconnects.

use super::App;
use crate::app_server_session::AppServerSession;
use codex_app_server_client::ReconnectEpoch;
use codex_app_server_protocol::ServerIdentity;
use codex_protocol::ThreadId;
use std::time::Duration;

const REATTACH_RETRY_DELAYS: [Duration; 3] = [
    Duration::from_millis(100),
    Duration::from_millis(250),
    Duration::from_millis(500),
];

impl App {
    pub(super) fn reconnect_thread_ids(&self) -> Vec<ThreadId> {
        let mut thread_ids: Vec<ThreadId> = self.attached_thread_ids.iter().copied().collect();
        thread_ids.sort_by_key(ToString::to_string);
        thread_ids
    }

    pub(super) async fn reattach_after_reconnect(
        &mut self,
        app_server: &mut AppServerSession,
        epoch: ReconnectEpoch,
        previous: Option<ServerIdentity>,
        current: Option<ServerIdentity>,
    ) -> Vec<String> {
        tracing::info!(
            reconnect_epoch = epoch.get(),
            previous_instance_id = previous
                .as_ref()
                .map(|identity| identity.instance_id.as_str()),
            previous_generation = previous
                .as_ref()
                .map(|identity| identity.generation.as_str()),
            current_instance_id = current
                .as_ref()
                .map(|identity| identity.instance_id.as_str()),
            current_generation = current
                .as_ref()
                .map(|identity| identity.generation.as_str()),
            "reattaching TUI threads after app-server reconnect"
        );
        let thread_ids = self.reconnect_thread_ids();

        let mut failures = Vec::new();
        for thread_id in thread_ids {
            let mut last_error = None;
            for retry_delay in REATTACH_RETRY_DELAYS {
                match app_server
                    .resume_thread(self.config.clone(), thread_id, self.resume_model_settings())
                    .await
                {
                    Ok(started) => {
                        let snapshot_turn_count = started.turns.len();
                        let snapshot_last_turn_id =
                            started.turns.last().map(|turn| turn.id.clone());
                        let snapshot_last_item_id = started
                            .turns
                            .iter()
                            .rev()
                            .find_map(|turn| turn.items.last())
                            .map(|item| item.id().to_string());
                        let snapshot_revision = format!(
                            "{snapshot_turn_count}:{}:{}",
                            snapshot_last_turn_id.as_deref().unwrap_or("none"),
                            snapshot_last_item_id.as_deref().unwrap_or("none"),
                        );
                        let channel = self.ensure_thread_channel(thread_id);
                        let mut store = channel.store.lock().await;
                        let buffered_events_before = store.buffer.len();
                        store.set_session(started.session.clone(), started.turns);
                        store.rebase_buffer_after_session_refresh();
                        let buffered_events_after = store.buffer.len();
                        drop(store);
                        let (queued_events_before, queued_events_preserved) =
                            self.reconcile_queued_events_after_session_refresh(thread_id);
                        tracing::info!(
                            reconnect_epoch = epoch.get(),
                            %thread_id,
                            %snapshot_revision,
                            snapshot_turn_count,
                            snapshot_last_turn_id = snapshot_last_turn_id.as_deref(),
                            snapshot_last_item_id = snapshot_last_item_id.as_deref(),
                            buffered_events_before,
                            buffered_events_after,
                            queued_events_before,
                            queued_events_preserved,
                            "reattached TUI thread from authoritative app-server snapshot"
                        );
                        if self.current_displayed_thread_id() == Some(thread_id) {
                            self.chat_widget
                                .handle_thread_session_quiet(started.session);
                        }
                        last_error = None;
                        break;
                    }
                    Err(err) => {
                        last_error = Some(err);
                        tokio::time::sleep(retry_delay).await;
                    }
                }
            }
            if let Some(err) = last_error {
                tracing::warn!(%thread_id, error = %err, "failed to reattach thread after app-server reconnect");
                failures.push(format!("{thread_id}: {err}"));
            }
        }

        if failures.is_empty()
            && let Err(err) = app_server.finish_reconnect(epoch).await
        {
            failures.push(format!("failed to release queued requests: {err}"));
        } else if !failures.is_empty() {
            failures.push(
                "queued requests remain parked because not every live thread reattached"
                    .to_string(),
            );
        }
        if !failures.is_empty() {
            self.chat_widget
                .add_error_message(reattachment_failure_message(&failures));
        }
        failures
    }

    /// Reconnect events are handled serially by the main app loop. Therefore,
    /// every event already in a thread receiver at this point was routed before
    /// `thread/resume` returned its authoritative snapshot; post-snapshot
    /// notifications cannot be routed until this handler yields. Discard
    /// snapshot-covered events and immediately deliver the small set of
    /// interactive/hook events that the store refresh policy preserves.
    fn reconcile_queued_events_after_session_refresh(
        &mut self,
        thread_id: ThreadId,
    ) -> (usize, usize) {
        let is_active = self.active_thread_id == Some(thread_id);
        let receiver = if is_active {
            self.active_thread_rx.as_mut()
        } else {
            self.thread_event_channels
                .get_mut(&thread_id)
                .and_then(|channel| channel.receiver.as_mut())
        };
        let Some(receiver) = receiver else {
            return (0, 0);
        };

        let mut queued_events_before = 0;
        let mut preserved = Vec::new();
        while let Ok(event) = receiver.try_recv() {
            queued_events_before += 1;
            if crate::app::thread_events::ThreadEventStore::event_survives_session_refresh(&event) {
                preserved.push(event);
            }
        }

        let queued_events_preserved = preserved.len();
        if is_active {
            for event in preserved {
                self.handle_thread_event_now(event);
            }
        }
        (queued_events_before, queued_events_preserved)
    }
}

pub(super) fn reattachment_failure_message(failures: &[String]) -> String {
    format!(
        "App-server reconnected, but session reattachment was incomplete. Please retry your message.\n{}",
        failures.join("\n")
    )
}
