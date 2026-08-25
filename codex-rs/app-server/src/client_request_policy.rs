//! Exhaustive lifecycle policy for app-server client requests.

use codex_app_server_protocol::ClientRequest;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ClientRequestPolicy {
    pub(crate) starts_new_work: bool,
    pub(crate) creates_implicit_attachment: bool,
}

impl ClientRequestPolicy {
    const NONE: Self = Self {
        starts_new_work: false,
        creates_implicit_attachment: false,
    };
    const WORK: Self = Self {
        starts_new_work: true,
        creates_implicit_attachment: false,
    };
    const IMPLICIT_ATTACHMENT: Self = Self {
        starts_new_work: false,
        creates_implicit_attachment: true,
    };
    const WORK_AND_IMPLICIT_ATTACHMENT: Self = Self {
        starts_new_work: true,
        creates_implicit_attachment: true,
    };

    pub(crate) fn requires_drain_admission(self) -> bool {
        self.starts_new_work || self.creates_implicit_attachment
    }
}

/// Classifies every v2 client request without a wildcard arm so new protocol methods fail to
/// compile until their drain and attachment behavior is reviewed deliberately.
pub(crate) fn client_request_policy(request: &ClientRequest) -> ClientRequestPolicy {
    match request {
        ClientRequest::ThreadStart { .. }
        | ClientRequest::ThreadResume { .. }
        | ClientRequest::ThreadFork { .. }
        | ClientRequest::ThreadUnarchive { .. } => ClientRequestPolicy::IMPLICIT_ATTACHMENT,

        ClientRequest::ThreadRevert { .. } | ClientRequest::ReviewStart { .. } => {
            ClientRequestPolicy::WORK_AND_IMPLICIT_ATTACHMENT
        }

        ClientRequest::ThreadArchive { .. }
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
        | ClientRequest::ProjectCreate { .. }
        | ClientRequest::ProjectImport { .. }
        | ClientRequest::ProjectUpdate { .. }
        | ClientRequest::ProjectMove { .. }
        | ClientRequest::ProjectDelete { .. }
        | ClientRequest::ThreadSectionMove { .. }
        | ClientRequest::TurnStart { .. }
        | ClientRequest::ThreadRealtimeStart { .. }
        | ClientRequest::OneOffCommandExec { .. }
        | ClientRequest::ProcessSpawn { .. } => ClientRequestPolicy::WORK,

        ClientRequest::Initialize { .. }
        | ClientRequest::ServerDiagnostics { .. }
        | ClientRequest::ServerDrainStart { .. }
        | ClientRequest::ServerDrainStatus { .. }
        | ClientRequest::ServerDrainCancel { .. }
        | ClientRequest::ThreadUnsubscribe { .. }
        | ClientRequest::ThreadIncrementElicitation { .. }
        | ClientRequest::ThreadDecrementElicitation { .. }
        | ClientRequest::ThreadGoalGet { .. }
        | ClientRequest::ThreadQueueList { .. }
        | ClientRequest::MemoryReset { .. }
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
        | ClientRequest::TurnSteer { .. }
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
        | ClientRequest::FuzzyFileSearchSessionStop { .. } => ClientRequestPolicy::NONE,
    }
}

#[cfg(test)]
#[path = "client_request_policy_tests.rs"]
mod tests;
