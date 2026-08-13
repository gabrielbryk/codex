use std::collections::HashMap;
use std::io::Error as IoError;
use std::io::ErrorKind;
use std::io::Result as IoResult;
use std::time::Duration;

use codex_app_server_protocol::JSONRPCRequest;
use codex_app_server_protocol::RequestId;
use tokio::sync::oneshot;
use tokio::time::Instant;
use tracing::warn;

use crate::RequestResult;

/// Bounds how long a replay-safe request may remain parked across reconnects.
const PARKED_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
/// Prevents a repeatedly failing server from replaying one request forever.
const MAX_REQUEST_REPLAY_ATTEMPTS: u32 = 1;
/// Methods whose protocol-level idempotency makes verbatim replay safe.
const REPLAYABLE_REQUEST_METHODS: &[&str] = &["turn/start"];
/// Wire name of the `TurnStartParams::client_user_message_id` idempotency key.
pub(super) const CLIENT_USER_MESSAGE_ID_FIELD: &str = "clientUserMessageId";

/// Bookkeeping for a request that has been handed to the worker but has not
/// been answered yet.
pub(super) struct PendingRequest {
    pub(super) response_tx: oneshot::Sender<IoResult<RequestResult>>,
    pub(super) replay: Option<ReplayState>,
}

impl PendingRequest {
    fn parked_deadline(&self) -> Option<Instant> {
        Some(self.replay.as_ref()?.parked.as_ref()?.deadline)
    }

    fn give_up_error(&self) -> IoError {
        match self
            .replay
            .as_ref()
            .and_then(|replay| replay.parked.as_ref())
        {
            Some(parked) => IoError::new(parked.err_kind, parked.err_message.clone()),
            None => IoError::new(
                ErrorKind::TimedOut,
                "remote app-server request timed out while reconnecting",
            ),
        }
    }
}

pub(super) struct ReplayState {
    pub(super) request: Box<JSONRPCRequest>,
    pub(super) attempts_remaining: u32,
    pub(super) parked: Option<ParkedRequest>,
}

pub(super) struct ParkedRequest {
    deadline: Instant,
    err_kind: ErrorKind,
    err_message: String,
}

impl ReplayState {
    pub(super) fn for_request(request: &JSONRPCRequest) -> Option<Self> {
        if !REPLAYABLE_REQUEST_METHODS.contains(&request.method.as_str()) {
            return None;
        }
        let has_idempotency_key = request
            .params
            .as_ref()
            .and_then(|params| params.get(CLIENT_USER_MESSAGE_ID_FIELD))
            .and_then(serde_json::Value::as_str)
            .is_some_and(|key| !key.is_empty());
        if !has_idempotency_key {
            return None;
        }
        Some(Self {
            request: Box::new(request.clone()),
            attempts_remaining: MAX_REQUEST_REPLAY_ATTEMPTS,
            parked: None,
        })
    }
}

pub(super) fn earliest_parked_deadline(
    pending_requests: &HashMap<RequestId, PendingRequest>,
) -> Option<Instant> {
    pending_requests
        .values()
        .filter_map(PendingRequest::parked_deadline)
        .min()
}

pub(super) fn park_pending_requests(
    pending_requests: &mut HashMap<RequestId, PendingRequest>,
    err_kind: ErrorKind,
    err_message: &str,
) {
    let now = Instant::now();
    let mut parked = HashMap::new();
    for (request_id, mut pending) in pending_requests.drain() {
        let keep = match pending.replay.as_mut() {
            Some(replay) if replay.attempts_remaining > 0 => {
                let deadline = replay
                    .parked
                    .get_or_insert_with(|| ParkedRequest {
                        deadline: now + PARKED_REQUEST_TIMEOUT,
                        err_kind,
                        err_message: err_message.to_string(),
                    })
                    .deadline;
                deadline > now
            }
            _ => false,
        };
        if keep {
            parked.insert(request_id, pending);
        } else {
            let _ = pending
                .response_tx
                .send(Err(IoError::new(err_kind, err_message.to_string())));
        }
    }
    *pending_requests = parked;
}

pub(super) fn expire_parked_requests(pending_requests: &mut HashMap<RequestId, PendingRequest>) {
    let now = Instant::now();
    let expired: Vec<RequestId> = pending_requests
        .iter()
        .filter(|(_, pending)| {
            pending
                .parked_deadline()
                .is_some_and(|deadline| deadline <= now)
        })
        .map(|(request_id, _)| request_id.clone())
        .collect();
    for request_id in expired {
        let Some(pending) = pending_requests.remove(&request_id) else {
            continue;
        };
        warn!(
            %request_id,
            "giving up on parked remote app-server request after reconnect timeout"
        );
        let give_up_error = pending.give_up_error();
        let _ = pending.response_tx.send(Err(give_up_error));
    }
}

pub(super) fn fail_pending_requests(
    pending_requests: &mut HashMap<RequestId, PendingRequest>,
    err_kind: ErrorKind,
    err_message: &str,
) {
    for (_, pending) in pending_requests.drain() {
        let _ = pending
            .response_tx
            .send(Err(IoError::new(err_kind, err_message.to_string())));
    }
}
