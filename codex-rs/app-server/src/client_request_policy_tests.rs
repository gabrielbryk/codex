use codex_app_server_protocol::ClientRequest;
use codex_app_server_protocol::ProjectCreateParams;
use codex_app_server_protocol::ProjectListParams;
use codex_app_server_protocol::RequestId;
use codex_app_server_protocol::ThreadGoalClearParams;
use codex_app_server_protocol::ThreadGoalSetParams;
use codex_app_server_protocol::ThreadQueueStartParams;
use codex_app_server_protocol::ThreadRevertParams;
use codex_app_server_protocol::ThreadSectionMoveParams;
use pretty_assertions::assert_eq;

use super::ClientRequestPolicy;
use super::client_request_policy;

fn request_id() -> RequestId {
    RequestId::Integer(1)
}

#[test]
fn target_added_mutations_require_drain_admission() {
    let requests = [
        ClientRequest::ThreadQueueStart {
            request_id: request_id(),
            params: ThreadQueueStartParams {
                thread_id: "thread".to_string(),
                queued_submission_id: None,
            },
        },
        ClientRequest::ProjectCreate {
            request_id: request_id(),
            params: ProjectCreateParams {
                name: "project".to_string(),
                roots: Vec::new(),
                metadata: None,
                idempotency_key: "key".to_string(),
            },
        },
        ClientRequest::ThreadGoalSet {
            request_id: request_id(),
            params: ThreadGoalSetParams {
                thread_id: "thread".to_string(),
                objective: Some("ship the upgrade".to_string()),
                status: None,
                token_budget: None,
            },
        },
        ClientRequest::ThreadGoalClear {
            request_id: request_id(),
            params: ThreadGoalClearParams {
                thread_id: "thread".to_string(),
            },
        },
        ClientRequest::ThreadSectionMove {
            request_id: request_id(),
            params: ThreadSectionMoveParams {
                thread_id: "thread".to_string(),
                section_id: Some("section".to_string()),
                before_thread_id: None,
            },
        },
    ];

    assert!(requests.iter().all(|request| {
        let policy = client_request_policy(request);
        policy.starts_new_work && policy.requires_drain_admission()
    }));
}

#[test]
fn thread_revert_is_work_and_can_replace_an_attached_runtime() {
    let request = ClientRequest::ThreadRevert {
        request_id: request_id(),
        params: ThreadRevertParams {
            thread_id: "thread".to_string(),
            before_turn_id: "turn".to_string(),
        },
    };

    assert_eq!(
        client_request_policy(&request),
        ClientRequestPolicy {
            starts_new_work: true,
            creates_implicit_attachment: true,
        }
    );
}

#[test]
fn target_added_reads_remain_available_during_drain() {
    let request = ClientRequest::ProjectList {
        request_id: request_id(),
        params: ProjectListParams {
            cursor: None,
            limit: None,
        },
    };

    assert_eq!(client_request_policy(&request), ClientRequestPolicy::NONE);
}
