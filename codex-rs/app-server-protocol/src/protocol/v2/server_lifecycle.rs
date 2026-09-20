use crate::JsonSchema;
use crate::TS;
use serde::Deserialize;
use serde::Serialize;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ServerDrainStartParams {
    /// Generation that should receive newly accepted client connections.
    pub replacement_generation: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ServerDrainStatusParams {}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ServerDrainCancelParams {}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub enum ServerGenerationState {
    Accepting,
    Draining,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ServerDrainResponse {
    /// Managed generation id, or null for an unmanaged app-server process.
    pub generation: Option<String>,
    pub state: ServerGenerationState,
    pub replacement_generation: Option<String>,
    /// Loaded threads that still have an active turn or elicitation.
    pub active_thread_ids: Vec<String>,
    /// Threads that remain loaded in this process.
    pub loaded_thread_ids: Vec<String>,
    /// Threads whose writer ownership was released during this drain.
    pub released_thread_ids: Vec<String>,
    /// False after any thread writer has been released to the replacement.
    pub cancellation_allowed: bool,
}
