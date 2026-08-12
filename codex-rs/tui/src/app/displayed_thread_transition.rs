//! Structured telemetry for changes to the transcript currently shown by the TUI.

use codex_protocol::ThreadId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DisplayedThreadTransitionReason {
    AdjacentThreadNavigation,
    AgentPickerSelection,
    AutomaticClosedThreadFailover,
    SessionLineageAttachment,
    PromptEditAttachment,
    SideConversationCleanupRecovery,
    SideConversationReturn,
    SideConversationStart,
    SideConversationToggle,
}

impl DisplayedThreadTransitionReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::AdjacentThreadNavigation => "adjacent_thread_navigation",
            Self::AgentPickerSelection => "agent_picker_selection",
            Self::AutomaticClosedThreadFailover => "automatic_closed_thread_failover",
            Self::SessionLineageAttachment => "session_lineage_attachment",
            Self::PromptEditAttachment => "prompt_edit_attachment",
            Self::SideConversationCleanupRecovery => "side_conversation_cleanup_recovery",
            Self::SideConversationReturn => "side_conversation_return",
            Self::SideConversationStart => "side_conversation_start",
            Self::SideConversationToggle => "side_conversation_toggle",
        }
    }
}

pub(super) fn log_displayed_thread_transition(
    previous_thread_id: Option<ThreadId>,
    current_thread_id: Option<ThreadId>,
    reason: DisplayedThreadTransitionReason,
) {
    if previous_thread_id == current_thread_id {
        return;
    }

    tracing::info!(
        previous_thread_id = previous_thread_id.map(|thread_id| thread_id.to_string()),
        current_thread_id = current_thread_id.map(|thread_id| thread_id.to_string()),
        reason = reason.as_str(),
        "displayed TUI thread changed"
    );
}
