use super::*;
use codex_app_server_protocol::JSONRPCErrorError;
use pretty_assertions::assert_eq;
use std::cell::Cell;
use std::collections::VecDeque;
use std::io;

fn server_error(method: &str, code: i64) -> TypedRequestError {
    TypedRequestError::Server {
        method: method.to_string(),
        source: JSONRPCErrorError {
            code,
            message: "server busy".to_string(),
            data: None,
        },
    }
}

#[test]
fn retries_turn_steer_overload_three_times_with_bounded_backoff() {
    let error = server_error("turn/steer", -32001);
    let mut retry = SteerOverloadRetry::default();

    let delays = (0..MAX_RETRIES)
        .map(|_| retry.next_delay(&error).expect("overload should retry"))
        .collect::<Vec<_>>();

    for (delay, base_ms) in delays.into_iter().zip([100_u64, 200, 400]) {
        assert!(delay >= Duration::from_millis(base_ms * 9 / 10));
        assert!(delay < Duration::from_millis(base_ms * 11 / 10));
    }
    assert_eq!(retry.next_delay(&error), None);
    assert!(retry.is_exhausted_overload(&error));
}

#[test]
fn does_not_retry_unrelated_failures() {
    let errors = [
        server_error("turn/steer", -32602),
        server_error("turn/start", -32001),
        TypedRequestError::Transport {
            method: "turn/steer".to_string(),
            source: io::Error::other("connection closed"),
        },
    ];

    for error in errors {
        let mut retry = SteerOverloadRetry::default();
        assert_eq!(retry.next_delay(&error), None);
        assert!(!retry.is_exhausted_overload(&error));
    }
}

#[tokio::test]
async fn overload_then_success_submits_once_without_duplication() {
    let attempts = Cell::new(0);
    let mut responses = VecDeque::from([Err(server_error("turn/steer", -32001)), Ok("accepted")]);
    let delays = Cell::new(0);

    let outcome = retry_turn_steer_with_sleep(
        async || {
            attempts.set(attempts.get() + 1);
            responses.pop_front().expect("one response per attempt")
        },
        async |_| {
            delays.set(delays.get() + 1);
        },
    )
    .await;

    assert!(matches!(outcome, SteerRequestOutcome::Success("accepted")));
    assert_eq!(attempts.get(), 2);
    assert_eq!(delays.get(), 1);
}

#[tokio::test]
async fn persistent_overload_returns_nonfatal_outcome_after_bounded_retries() {
    let attempts = Cell::new(0);
    let delays = Cell::new(0);

    let outcome = retry_turn_steer_with_sleep(
        async || {
            attempts.set(attempts.get() + 1);
            Err::<(), _>(server_error("turn/steer", -32001))
        },
        async |_| {
            delays.set(delays.get() + 1);
        },
    )
    .await;

    assert!(matches!(
        outcome,
        SteerRequestOutcome::PersistentOverload(_)
    ));
    assert_eq!(attempts.get(), 4);
    assert_eq!(delays.get(), 3);
}
