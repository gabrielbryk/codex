//! Transcript consolidation for finalized streaming agent messages.
//!
//! During streaming, the chat widget emits transient `AgentMessageCell`s so it
//! can animate stable lines into scrollback while keeping the active mutable
//! tail in the bottom pane. Once the answer finishes, the app replaces that
//! trailing run with a single source-backed `AgentMarkdownCell`. This makes the
//! transcript the canonical owner of the raw markdown source used for future
//! resize re-renders.

use std::path::PathBuf;
use std::sync::Arc;

use color_eyre::eyre::Result;

use super::App;
use super::resize_reflow::trailing_run_start;
use crate::app_event::ConsolidationScrollbackReflow;
use crate::history_cell;
use crate::history_cell::HistoryCell;
use crate::inline_visualization::InlineVisualizationContext;
use crate::pager_overlay::Overlay;
use crate::tui;

/// Inputs for one `AppEvent::ConsolidateAgentMessage`.
pub(crate) struct AgentMessageConsolidation {
    pub(crate) source: String,
    pub(crate) cwd: PathBuf,
    pub(crate) inline_visualization_context: Option<InlineVisualizationContext>,
    pub(crate) scrollback_reflow: ConsolidationScrollbackReflow,
    pub(crate) deferred_history_cell: Option<Box<dyn HistoryCell>>,
    /// Item id of the assistant message, when the stream knew it.
    pub(crate) agent_message_item_id: Option<Arc<str>>,
}

impl App {
    pub(super) fn handle_consolidate_agent_message(
        &mut self,
        tui: &mut tui::Tui,
        consolidation: AgentMessageConsolidation,
    ) -> Result<()> {
        let AgentMessageConsolidation {
            source,
            cwd,
            inline_visualization_context,
            scrollback_reflow,
            deferred_history_cell,
            agent_message_item_id,
        } = consolidation;
        // Some finalize paths must preserve a last provisional stream cell long
        // enough for queue ordering, then fold it into the canonical
        // source-backed cell during consolidation.
        if let Some(cell) = deferred_history_cell {
            let cell: Arc<dyn HistoryCell> = cell.into();
            if let Some(Overlay::Transcript(t)) = &mut self.overlay {
                t.insert_cell(cell.clone());
            }
            self.transcript_cells.push(cell);
        }

        // Identify the cells already rendered for this assistant message. When the
        // stream carries an item id, match on that identity: a mid-item transcript
        // write (a replayed hook or approval, a reattachment notice) flushes the
        // stream and leaves a consolidated prefix cell behind, so the run is no
        // longer the contiguous tail. Falling back to the positional scan there
        // would append the authoritative full text next to the orphaned prefix and
        // render the streamed content twice.
        let end = self.transcript_cells.len();
        tracing::debug!(
            "ConsolidateAgentMessage: transcript_cells.len()={end}, source_len={}",
            source.len()
        );
        let start = match agent_message_item_id.as_deref() {
            Some(item_id) => self
                .transcript_cells
                .iter()
                .position(|cell| cell.agent_message_item_id() == Some(item_id))
                .unwrap_or(end),
            None => trailing_run_start::<history_cell::AgentMessageCell>(&self.transcript_cells),
        };
        if start < end {
            tracing::debug!(
                "ConsolidateAgentMessage: replacing cells [{start}..{end}] with AgentMarkdownCell"
            );
            let consolidated: Arc<dyn HistoryCell> = Arc::new(
                history_cell::AgentMarkdownCell::new_with_inline_visualizations(
                    source,
                    &cwd,
                    inline_visualization_context,
                    agent_message_item_id.clone(),
                ),
            );
            // Anything in the replaced span that is not part of this message —
            // notices, hook output, approvals interleaved with the stream — is
            // content the transcript has not shown anywhere else, so keep it
            // rather than trading a duplicate for a drop.
            let replacement = std::iter::once(consolidated.clone())
                .chain(retained_foreign_cells(
                    &self.transcript_cells[start..end],
                    agent_message_item_id.as_deref(),
                ))
                .collect::<Vec<_>>();
            self.transcript_cells
                .splice(start..end, replacement.iter().cloned());

            if let Some(Overlay::Transcript(t)) = &mut self.overlay {
                t.consolidate_cells(start..end, replacement);
                tui.frame_requester().schedule_frame();
            }

            self.finish_agent_message_consolidation(tui, scrollback_reflow)?;
        } else {
            tracing::debug!(
                "ConsolidateAgentMessage: no cells to consolidate(start={start}, end={end})",
            );
            self.maybe_finish_stream_reflow(tui)?;
        }

        Ok(())
    }

    fn finish_agent_message_consolidation(
        &mut self,
        tui: &mut tui::Tui,
        scrollback_reflow: ConsolidationScrollbackReflow,
    ) -> Result<()> {
        match scrollback_reflow {
            ConsolidationScrollbackReflow::IfResizeReflowRan => {
                self.maybe_finish_stream_reflow(tui)?;
            }
            ConsolidationScrollbackReflow::Required => {
                self.finish_required_stream_reflow(tui)?;
            }
        }

        Ok(())
    }
}

/// Returns the cells in `span` that do not belong to the message being consolidated.
///
/// Cells stamped with `item_id` are superseded by the consolidated cell. Streamed
/// `AgentMessageCell`s without a stamp are also part of the run when no item id is
/// known (the legacy positional path already selected only those). Everything else
/// is unrelated transcript content and is preserved.
fn retained_foreign_cells(
    span: &[Arc<dyn HistoryCell>],
    item_id: Option<&str>,
) -> Vec<Arc<dyn HistoryCell>> {
    let Some(item_id) = item_id else {
        return Vec::new();
    };
    span.iter()
        .filter(|cell| {
            cell.agent_message_item_id() != Some(item_id)
                && !(cell.agent_message_item_id().is_none()
                    && cell.as_any().is::<history_cell::AgentMessageCell>())
        })
        .cloned()
        .collect()
}
