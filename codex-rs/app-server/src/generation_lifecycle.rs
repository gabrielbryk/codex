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

    pub(crate) async fn admit_request(
        &self,
        request: &ClientRequest,
    ) -> Result<Option<WorkAdmissionPermit>, JSONRPCErrorError> {
        if !request_starts_new_work(request.method_name()) {
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

fn request_starts_new_work(method: &str) -> bool {
    matches!(
        method,
        "thread/start"
            | "thread/resume"
            | "thread/fork"
            | "thread/archive"
            | "thread/delete"
            | "thread/unarchive"
            | "thread/compact/start"
            | "thread/shellCommand"
            | "thread/rollback"
            | "thread/inject_items"
            | "thread/name/set"
            | "thread/metadata/update"
            | "thread/settings/update"
            | "thread/memoryMode/set"
            | "turn/start"
            | "review/start"
            | "thread/realtime/start"
            | "command/exec"
            | "process/spawn"
    )
}

#[cfg(test)]
#[path = "generation_lifecycle_tests.rs"]
mod tests;
