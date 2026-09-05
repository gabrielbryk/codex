//! Versioned JSON input written to an external status-line command.
//!
//! Version 1 intentionally models only data Codex can source without exposing
//! prompts, environment variables, credentials, or raw API payloads. Required
//! compatibility fields are always present. Optional objects use omission,
//! while the percentage and current-usage fields explicitly permit JSON null.

use serde::Serialize;
use serde::ser::Error as _;
use std::io;
use std::io::Write;
use uuid::Uuid;

/// Current formatter input schema version.
pub(crate) const STATUS_LINE_COMMAND_SCHEMA_VERSION: u8 = 1;
/// Maximum serialized JSON-line payload accepted for formatter stdin.
pub(crate) const MAX_STATUS_LINE_COMMAND_INPUT_BYTES: usize = 64 * 1024;
/// Maximum backend-provided workspace headline retained in formatter input.
pub(crate) const MAX_STATUS_LINE_WORKSPACE_HEADLINE_BYTES: usize = 1024;

/// Stable identity generated once for the lifetime of a command-mode widget.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub(crate) struct StatusLineCommandSessionId(String);

impl StatusLineCommandSessionId {
    pub(crate) fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }
}

impl From<Uuid> for StatusLineCommandSessionId {
    fn from(value: Uuid) -> Self {
        Self(value.to_string())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct StatusLineCommandInput {
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
        let mut writer =
            BoundedJsonWriter::new(MAX_STATUS_LINE_COMMAND_INPUT_BYTES.saturating_sub(/*rhs*/ 1));
        let result = serde_json::to_writer(&mut writer, self);
        if writer.limit_exceeded {
            return Err(serde_json::Error::custom(format!(
                "status-line command input exceeds {MAX_STATUS_LINE_COMMAND_INPUT_BYTES} bytes"
            )));
        }
        result?;

        let mut bytes = writer.bytes;
        bytes.push(b'\n');
        Ok(bytes)
    }
}

/// A JSON sink whose retained allocation cannot grow beyond `max_bytes`.
struct BoundedJsonWriter {
    bytes: Vec<u8>,
    max_bytes: usize,
    limit_exceeded: bool,
}

impl BoundedJsonWriter {
    fn new(max_bytes: usize) -> Self {
        Self {
            bytes: Vec::new(),
            max_bytes,
            limit_exceeded: false,
        }
    }
}

impl Write for BoundedJsonWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.len() > self.max_bytes.saturating_sub(self.bytes.len()) {
            self.limit_exceeded = true;
            return Err(io::Error::other("status-line command input exceeds limit"));
        }

        self.bytes.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
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
    /// Always serialized; `null` means the review state is unavailable in schema version 1.
    pub(crate) review_state: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct StatusLineCommandCodex {
    /// Codex-owned input schema version. Compatibility aliases remain top-level.
    pub(crate) schema_version: u8,
    /// Working directory of the local TUI process that launches the formatter.
    pub(crate) local_process_cwd: String,
    pub(crate) status: String,
    pub(crate) permissions: String,
    pub(crate) approval_mode: String,
    pub(crate) service_tier: String,
    pub(crate) workspace_headline: Option<String>,
    pub(crate) task_progress: Option<StatusLineCommandTaskProgress>,
    pub(crate) git_branch: Option<String>,
    pub(crate) branch_changes: Option<StatusLineCommandBranchChanges>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct StatusLineCommandTaskProgress {
    pub(crate) completed: u64,
    pub(crate) total: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct StatusLineCommandBranchChanges {
    pub(crate) additions: u64,
    pub(crate) deletions: u64,
}

#[cfg(test)]
#[path = "wire_tests.rs"]
mod tests;
