//! Versioned JSON input written to an external status-line command.
//!
//! Version 1 intentionally models only data Codex can source without exposing
//! prompts, environment variables, credentials, or raw API payloads. Required
//! compatibility fields are always present. Optional objects use omission,
//! while the percentage and current-usage fields explicitly permit JSON null.

use serde::Serialize;
use uuid::Uuid;

/// Current formatter input schema version.
pub(crate) const STATUS_LINE_COMMAND_SCHEMA_VERSION: u8 = 1;

/// Stable identity generated once for the lifetime of a command-mode widget.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub(crate) struct StatusLineCommandSessionId(String);

impl StatusLineCommandSessionId {
    pub(crate) fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<Uuid> for StatusLineCommandSessionId {
    fn from(value: Uuid) -> Self {
        Self(value.to_string())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct StatusLineCommandInput {
    /// Codex-owned schema version. Claude-compatible aliases remain top-level.
    pub(crate) schema_version: u8,
    /// Session workspace, which may be on a remote app-server host.
    pub(crate) cwd: String,
    pub(crate) session_id: StatusLineCommandSessionId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) session_name: Option<String>,
    pub(crate) model: StatusLineCommandModel,
    pub(crate) workspace: StatusLineCommandWorkspace,
    /// Local Codex client version.
    pub(crate) version: String,
    pub(crate) fast_mode: bool,
    pub(crate) exceeds_200k_tokens: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) effort: Option<StatusLineCommandEffort>,
    pub(crate) thinking: StatusLineCommandThinking,
    pub(crate) context_window: StatusLineCommandContextWindow,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) rate_limits: Option<StatusLineCommandRateLimits>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) extra_usage: Option<StatusLineCommandExtraUsage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) pr: Option<StatusLineCommandPullRequest>,
    pub(crate) codex: StatusLineCommandCodex,
}

impl StatusLineCommandInput {
    /// Serialize one JSON object followed by a newline, ready for command stdin.
    pub(crate) fn to_json_line(&self) -> Result<Vec<u8>, serde_json::Error> {
        let mut bytes = serde_json::to_vec(self)?;
        bytes.push(b'\n');
        Ok(bytes)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct StatusLineCommandModel {
    pub(crate) id: String,
    pub(crate) display_name: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct StatusLineCommandWorkspace {
    /// Session workspace, which may be remote and need not exist locally.
    pub(crate) current_dir: String,
    /// Session project root, if the session reports one.
    pub(crate) project_dir: Option<String>,
    pub(crate) added_dirs: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) repo: Option<StatusLineCommandRepository>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct StatusLineCommandRepository {
    pub(crate) host: String,
    pub(crate) owner: String,
    pub(crate) name: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct StatusLineCommandEffort {
    pub(crate) level: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct StatusLineCommandThinking {
    pub(crate) enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct StatusLineCommandContextWindow {
    pub(crate) total_input_tokens: u64,
    pub(crate) total_output_tokens: u64,
    pub(crate) context_window_size: u64,
    pub(crate) used_percentage: Option<f64>,
    pub(crate) remaining_percentage: Option<f64>,
    pub(crate) current_usage: Option<StatusLineCommandCurrentUsage>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct StatusLineCommandCurrentUsage {
    pub(crate) input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) cache_creation_input_tokens: u64,
    pub(crate) cache_read_input_tokens: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct StatusLineCommandRateLimits {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) five_hour: Option<StatusLineCommandRateLimitWindow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) seven_day: Option<StatusLineCommandRateLimitWindow>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct StatusLineCommandRateLimitWindow {
    pub(crate) used_percentage: f64,
    /// Raw Unix epoch seconds from the protocol response.
    pub(crate) resets_at: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct StatusLineCommandExtraUsage {
    pub(crate) enabled: bool,
    pub(crate) used: f64,
    pub(crate) limit: f64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct StatusLineCommandPullRequest {
    pub(crate) number: u64,
    pub(crate) url: String,
    pub(crate) review_state: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct StatusLineCommandCodex {
    /// Working directory of the local TUI process that launches the formatter.
    pub(crate) local_process_cwd: String,
    pub(crate) status: String,
    pub(crate) permissions: String,
    pub(crate) approval_mode: String,
    pub(crate) service_tier: String,
    pub(crate) workspace_headline: Option<String>,
    pub(crate) task_progress: Option<StatusLineCommandTaskProgress>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct StatusLineCommandTaskProgress {
    pub(crate) completed: u64,
    pub(crate) total: u64,
}

#[cfg(test)]
#[path = "wire_tests.rs"]
mod tests;
