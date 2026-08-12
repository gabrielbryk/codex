//! Restores connection-scoped thread subscriptions after app-server reconnects.

use super::App;
use crate::app::thread_events::ThreadEventAttachment;
use crate::app_server_session::AppServerSession;
use codex_app_server_protocol::ServerIdentity;
use codex_protocol::ThreadId;
use std::time::Duration;

const REATTACH_RETRY_DELAYS: [Duration; 3] = [
    Duration::from_millis(100),
    Duration::from_millis(250),
    Duration::from_millis(500),
];

impl App {
    pub(super) async fn reattach_after_reconnect(
        &mut self,
        app_server: &mut AppServerSession,
        previous: Option<ServerIdentity>,
        current: Option<ServerIdentity>,
    ) {
        tracing::info!(
            ?previous,
            ?current,
            "reattaching TUI threads after app-server reconnect"
        );
        let mut thread_ids: Vec<ThreadId> = self
            .thread_event_channels
            .iter()
            .filter_map(|(thread_id, channel)| {
                (channel.attachment() == ThreadEventAttachment::Live).then_some(*thread_id)
            })
            .collect();
        thread_ids.sort_by_key(ToString::to_string);
        thread_ids.dedup();

        let mut failures = Vec::new();
        for thread_id in thread_ids {
            let mut last_error = None;
            for retry_delay in REATTACH_RETRY_DELAYS {
                match app_server
                    .resume_thread(self.config.clone(), thread_id, self.resume_model_settings())
                    .await
                {
                    Ok(started) => {
                        let channel = self.ensure_thread_channel(thread_id);
                        let mut store = channel.store.lock().await;
                        store.set_session(started.session.clone(), started.turns);
                        drop(store);
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

        if let Err(err) = app_server.finish_reconnect().await {
            failures.push(format!("failed to release queued requests: {err}"));
        }
        if !failures.is_empty() {
            self.chat_widget
                .add_error_message(reattachment_failure_message(&failures));
        }
    }
}

pub(super) fn reattachment_failure_message(failures: &[String]) -> String {
    format!(
        "App-server reconnected, but session reattachment was incomplete. Please retry your message.\n{}",
        failures.join("\n")
    )
}
