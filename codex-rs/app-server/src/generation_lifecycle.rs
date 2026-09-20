//! Identity and drain metadata for a host-managed rolling app-server generation.
//!
//! `GenerationLifecycle` owns only replacement-generation bookkeeping and a
//! read/write admission gate; it does not duplicate `turn_admission`'s
//! running-turn tracking. The admission permit returned by
//! [`GenerationLifecycle::acquire_work_permit`] must be held by the caller for
//! the full lifetime of the work it gates (not just until the request is
//! dispatched) so a drain cannot finish while attributed work is still
//! detached from any tracked counter -- see `message_processor.rs` (around
//! line 1005), the only call site that holds it across a spawned task.

use std::collections::BTreeSet;
use std::sync::Arc;

use codex_app_server_protocol::ClientRequest;
use codex_app_server_protocol::JSONRPCErrorError;
use codex_app_server_protocol::ServerDrainResponse;
use codex_app_server_protocol::ServerGenerationState;
use serde_json::json;
use tokio::sync::OwnedRwLockReadGuard;
use tokio::sync::RwLock;

use crate::error_code::INVALID_REQUEST_ERROR_CODE;

const SERVER_DRAINING_ERROR_CODE: i64 = -32002;

#[derive(Clone, Debug)]
pub(crate) struct GenerationLifecycle {
    generation: Option<String>,
    state: Arc<RwLock<GenerationLifecycleState>>,
}

pub(crate) struct WorkAdmissionPermit {
    _guard: OwnedRwLockReadGuard<GenerationLifecycleState>,
}

#[derive(Debug, Default)]
struct GenerationLifecycleState {
    replacement_generation: Option<String>,
    released_thread_ids: BTreeSet<String>,
}

impl GenerationLifecycle {
    pub(crate) fn from_environment() -> Self {
        Self {
            generation: std::env::var("CODEX_APP_SERVER_GENERATION_ID").ok(),
            state: Arc::new(RwLock::new(GenerationLifecycleState::default())),
        }
    }

    pub(crate) async fn begin(
        &self,
        replacement_generation: String,
    ) -> Result<(), JSONRPCErrorError> {
        if replacement_generation.trim().is_empty() {
            return Err(invalid_lifecycle_request(
                "replacement generation must not be empty",
            ));
        }
        if self.generation.as_deref() == Some(replacement_generation.as_str()) {
            return Err(invalid_lifecycle_request(
                "replacement generation must differ from the current generation",
            ));
        }

        let mut state = self.state.write().await;
        match state.replacement_generation.as_deref() {
            Some(existing) if existing != replacement_generation => Err(invalid_lifecycle_request(
                format!("server is already draining to generation {existing}"),
            )),
            Some(_) => Ok(()),
            None => {
                state.replacement_generation = Some(replacement_generation);
                Ok(())
            }
        }
    }

    pub(crate) async fn cancel(&self) -> Result<(), JSONRPCErrorError> {
        let mut state = self.state.write().await;
        if !state.released_thread_ids.is_empty() {
            return Err(invalid_lifecycle_request(
                "drain cannot be cancelled after a thread writer was released",
            ));
        }
        state.replacement_generation = None;
        Ok(())
    }

    /// Records a released thread writer. Callers must only invoke this after
    /// the thread's rollout has been durably persisted (see
    /// `thread_processor::server_drain_status`), so a released thread is
    /// never reported before the replacement generation can actually resume
    /// writing it.
    pub(crate) async fn note_released(&self, thread_id: String) {
        self.state
            .write()
            .await
            .released_thread_ids
            .insert(thread_id);
    }

    pub(crate) async fn is_draining(&self) -> bool {
        self.state.read().await.replacement_generation.is_some()
    }

    /// Admits `request` if it requires drain admission per
    /// [`requires_drain_admission`]. The returned permit must be held by the
    /// caller for the full duration of the work the request starts.
    pub(crate) async fn admit_request(
        &self,
        request: &ClientRequest,
    ) -> Result<Option<WorkAdmissionPermit>, JSONRPCErrorError> {
        if !requires_drain_admission(request) {
            return Ok(None);
        }
        self.acquire_work_permit().await.map(Some)
    }

    pub(crate) async fn acquire_work_permit(
        &self,
    ) -> Result<WorkAdmissionPermit, JSONRPCErrorError> {
        let guard = Arc::clone(&self.state).read_owned().await;
        if guard.replacement_generation.is_none() {
            return Ok(WorkAdmissionPermit { _guard: guard });
        }
        Err(self.server_draining_error(&guard))
    }

    pub(crate) async fn response(
        &self,
        mut active_thread_ids: Vec<String>,
        mut loaded_thread_ids: Vec<String>,
    ) -> ServerDrainResponse {
        active_thread_ids.sort();
        loaded_thread_ids.sort();
        let state = self.state.read().await;
        let draining = state.replacement_generation.is_some();
        ServerDrainResponse {
            generation: self.generation.clone(),
            state: if draining {
                ServerGenerationState::Draining
            } else {
                ServerGenerationState::Accepting
            },
            replacement_generation: state.replacement_generation.clone(),
            active_thread_ids,
            loaded_thread_ids,
            released_thread_ids: state.released_thread_ids.iter().cloned().collect(),
            cancellation_allowed: state.released_thread_ids.is_empty(),
        }
    }

    fn server_draining_error(&self, state: &GenerationLifecycleState) -> JSONRPCErrorError {
        JSONRPCErrorError {
            code: SERVER_DRAINING_ERROR_CODE,
            message: "server generation is draining; retry on the replacement generation"
                .to_string(),
            data: Some(json!({
                "type": "serverDraining",
                "generation": self.generation,
                "replacementGeneration": state.replacement_generation,
                "retryable": true,
            })),
        }
    }
}

fn invalid_lifecycle_request(message: impl Into<String>) -> JSONRPCErrorError {
    JSONRPCErrorError {
        code: INVALID_REQUEST_ERROR_CODE,
        message: message.into(),
        data: None,
    }
}

/// Exhaustive classifier for whether a client request must hold a drain
/// admission permit for as long as its work is in flight. No wildcard arm, so
/// a new `ClientRequest` variant fails to compile here until its drain
/// behavior is reviewed deliberately.
///
/// `server/drain/*` itself, and `remoteControl/status/*`, are intentionally
/// `false`: the drain RPCs and remote-control status surface must stay
/// answerable while a generation is draining.
pub(crate) fn requires_drain_admission(request: &ClientRequest) -> bool {
    match request {
        ClientRequest::ThreadStart { .. }
        | ClientRequest::ThreadResume { .. }
        | ClientRequest::ThreadFork { .. }
        | ClientRequest::ThreadUnarchive { .. }
        | ClientRequest::ThreadRevert { .. }
        | ClientRequest::ReviewStart { .. }
        | ClientRequest::ThreadArchive { .. }
        | ClientRequest::ThreadDelete { .. }
        | ClientRequest::ThreadQueueAdd { .. }
        | ClientRequest::ThreadQueueUpdate { .. }
        | ClientRequest::ThreadQueueDelete { .. }
        | ClientRequest::ThreadQueueReorder { .. }
        | ClientRequest::ThreadQueueStart { .. }
        | ClientRequest::ThreadSetName { .. }
        | ClientRequest::ThreadMetadataUpdate { .. }
        | ClientRequest::ThreadSettingsUpdate { .. }
        | ClientRequest::ThreadMemoryModeSet { .. }
        | ClientRequest::ThreadGoalSet { .. }
        | ClientRequest::ThreadGoalClear { .. }
        | ClientRequest::ThreadCompactStart { .. }
        | ClientRequest::ThreadShellCommand { .. }
        | ClientRequest::ThreadRollback { .. }
        | ClientRequest::ThreadInjectItems { .. }
        | ClientRequest::ThreadAttachmentAdd { .. }
        | ClientRequest::ThreadAttachmentRemove { .. }
        | ClientRequest::ProjectCreate { .. }
        | ClientRequest::ProjectImport { .. }
        | ClientRequest::ProjectUpdate { .. }
        | ClientRequest::ProjectMove { .. }
        | ClientRequest::ProjectDelete { .. }
        | ClientRequest::ThreadSectionMove { .. }
        | ClientRequest::TurnStart { .. }
        | ClientRequest::TurnSettingsUpdate { .. }
        | ClientRequest::ThreadRealtimeStart { .. }
        | ClientRequest::OneOffCommandExec { .. }
        | ClientRequest::ProcessSpawn { .. }
        | ClientRequest::PluginReconcile { .. }
        | ClientRequest::McpServerEventStreamStart { .. } => true,

        ClientRequest::TurnSteer { .. }
        | ClientRequest::Initialize { .. }
        | ClientRequest::ServerDiagnostics { .. }
        | ClientRequest::ServerDrainStart { .. }
        | ClientRequest::ServerDrainStatus { .. }
        | ClientRequest::ServerDrainCancel { .. }
        | ClientRequest::ThreadUnsubscribe { .. }
        | ClientRequest::ThreadIncrementElicitation { .. }
        | ClientRequest::ThreadDecrementElicitation { .. }
        | ClientRequest::ThreadGoalGet { .. }
        | ClientRequest::ThreadQueueList { .. }
        | ClientRequest::ThreadAttachmentList { .. }
        | ClientRequest::MemoryStatus { .. }
        | ClientRequest::MemoryReset { .. }
        | ClientRequest::UserVerificationStatus { .. }
        | ClientRequest::UserVerificationEnroll { .. }
        | ClientRequest::UserVerificationVerify { .. }
        | ClientRequest::UserVerificationCancel { .. }
        | ClientRequest::UserVerificationDelete { .. }
        | ClientRequest::ThreadApproveGuardianDeniedAction { .. }
        | ClientRequest::ThreadBackgroundTerminalsClean { .. }
        | ClientRequest::ThreadBackgroundTerminalsList { .. }
        | ClientRequest::ThreadBackgroundTerminalsTerminate { .. }
        | ClientRequest::ThreadList { .. }
        | ClientRequest::ProjectList { .. }
        | ClientRequest::ProjectRead { .. }
        | ClientRequest::ThreadSectionList { .. }
        | ClientRequest::ThreadSectionCreate { .. }
        | ClientRequest::ThreadSectionUpdate { .. }
        | ClientRequest::ThreadSectionDelete { .. }
        | ClientRequest::ThreadSearch { .. }
        | ClientRequest::ThreadSearchOccurrences { .. }
        | ClientRequest::ThreadLoadedList { .. }
        | ClientRequest::ThreadRead { .. }
        | ClientRequest::ThreadTurnsList { .. }
        | ClientRequest::ThreadItemsList { .. }
        | ClientRequest::ThreadTimelineList { .. }
        | ClientRequest::SkillsList { .. }
        | ClientRequest::SkillsExtraRootsSet { .. }
        | ClientRequest::HooksList { .. }
        | ClientRequest::MarketplaceAdd { .. }
        | ClientRequest::MarketplaceRemove { .. }
        | ClientRequest::MarketplaceUpgrade { .. }
        | ClientRequest::PluginList { .. }
        | ClientRequest::PluginSearch { .. }
        | ClientRequest::PluginInstalled { .. }
        | ClientRequest::PluginRead { .. }
        | ClientRequest::PluginSkillRead { .. }
        | ClientRequest::PluginShareSave { .. }
        | ClientRequest::PluginShareUpdateTargets { .. }
        | ClientRequest::PluginShareList { .. }
        | ClientRequest::PluginShareCheckout { .. }
        | ClientRequest::PluginShareDelete { .. }
        | ClientRequest::AppsRead { .. }
        | ClientRequest::AppsList { .. }
        | ClientRequest::AppsInstalled { .. }
        | ClientRequest::FsReadFile { .. }
        | ClientRequest::FsWriteFile { .. }
        | ClientRequest::FsCreateDirectory { .. }
        | ClientRequest::FsGetMetadata { .. }
        | ClientRequest::FsReadDirectory { .. }
        | ClientRequest::FsRemove { .. }
        | ClientRequest::FsCopy { .. }
        | ClientRequest::FsWatch { .. }
        | ClientRequest::FsUnwatch { .. }
        | ClientRequest::SkillsConfigWrite { .. }
        | ClientRequest::PluginInstall { .. }
        | ClientRequest::PluginUninstall { .. }
        | ClientRequest::TurnInterrupt { .. }
        | ClientRequest::ThreadRealtimeAppendAudio { .. }
        | ClientRequest::ThreadRealtimeAppendText { .. }
        | ClientRequest::ThreadRealtimeAppendSpeech { .. }
        | ClientRequest::ThreadRealtimeStop { .. }
        | ClientRequest::ThreadRealtimeListVoices { .. }
        | ClientRequest::ModelList { .. }
        | ClientRequest::ModelProviderCapabilitiesRead { .. }
        | ClientRequest::ExperimentalFeatureList { .. }
        | ClientRequest::PermissionProfileList { .. }
        | ClientRequest::ExperimentalFeatureEnablementSet { .. }
        | ClientRequest::RemoteControlEnable { .. }
        | ClientRequest::RemoteControlDisable { .. }
        | ClientRequest::RemoteControlStatusRead { .. }
        | ClientRequest::RemoteControlPairingStart { .. }
        | ClientRequest::RemoteControlPairingStatus { .. }
        | ClientRequest::RemoteControlClientsList { .. }
        | ClientRequest::RemoteControlClientsRevoke { .. }
        | ClientRequest::CollaborationModeList { .. }
        | ClientRequest::MockExperimentalMethod { .. }
        | ClientRequest::EnvironmentAdd { .. }
        | ClientRequest::EnvironmentInfo { .. }
        | ClientRequest::EnvironmentStatus { .. }
        | ClientRequest::McpServerOauthLogin { .. }
        | ClientRequest::McpServerRefresh { .. }
        | ClientRequest::McpServerStatusList { .. }
        | ClientRequest::McpResourceRead { .. }
        | ClientRequest::McpServerEventStreamStop { .. }
        | ClientRequest::McpServerToolCall { .. }
        | ClientRequest::WindowsSandboxSetupStart { .. }
        | ClientRequest::WindowsSandboxReadiness { .. }
        | ClientRequest::LoginAccount { .. }
        | ClientRequest::BedrockDiscover { .. }
        | ClientRequest::BedrockSetup { .. }
        | ClientRequest::CancelLoginAccount { .. }
        | ClientRequest::LogoutAccount { .. }
        | ClientRequest::GetAccountRateLimits { .. }
        | ClientRequest::ConsumeAccountRateLimitResetCredit { .. }
        | ClientRequest::GetAccountTokenUsage { .. }
        | ClientRequest::GetWorkspaceMessages { .. }
        | ClientRequest::SendAddCreditsNudgeEmail { .. }
        | ClientRequest::FeedbackUpload { .. }
        | ClientRequest::CommandExecWrite { .. }
        | ClientRequest::CommandExecTerminate { .. }
        | ClientRequest::CommandExecResize { .. }
        | ClientRequest::ProcessWriteStdin { .. }
        | ClientRequest::ProcessKill { .. }
        | ClientRequest::ProcessResizePty { .. }
        | ClientRequest::ConfigRead { .. }
        | ClientRequest::ExternalAgentConfigDetect { .. }
        | ClientRequest::ExternalAgentConfigImport { .. }
        | ClientRequest::ExternalAgentConfigImportHistoryRecord { .. }
        | ClientRequest::ExternalAgentConfigImportHistoriesRead { .. }
        | ClientRequest::ConfigValueWrite { .. }
        | ClientRequest::ConfigBatchWrite { .. }
        | ClientRequest::ConfigRequirementsRead { .. }
        | ClientRequest::GetAccount { .. }
        | ClientRequest::GetConversationSummary { .. }
        | ClientRequest::GitDiffToRemote { .. }
        | ClientRequest::GetAuthStatus { .. }
        | ClientRequest::FuzzyFileSearch { .. }
        | ClientRequest::FuzzyFileSearchSessionStart { .. }
        | ClientRequest::FuzzyFileSearchSessionUpdate { .. }
        | ClientRequest::FuzzyFileSearchSessionStop { .. } => false,
    }
}

#[cfg(test)]
#[path = "generation_lifecycle_tests.rs"]
mod tests;
