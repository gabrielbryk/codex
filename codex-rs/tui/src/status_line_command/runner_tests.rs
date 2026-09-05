use super::*;

fn input() -> StatusLineCommandInput {
    StatusLineCommandInput {
        cwd: "/repo".to_string(),
        model: "model".to_string(),
        status: "idle".to_string(),
        input_tokens: 0,
        output_tokens: 0,
        context_remaining_percent: None,
    }
}

fn completion(token: RequestToken, result: Result<&str, &str>) -> Completion {
    Completion {
        token,
        result: result.map(str::to_string).map_err(str::to_string),
    }
}

#[test]
fn lifecycle_rejects_stale_completions_and_retains_last_good() {
    let mut lifecycle = Lifecycle::new(7);
    let now = Instant::now();
    let old = lifecycle.begin(input(), now).expect("generation");
    let mut changed = input();
    changed.status = "working".to_string();
    let newest = lifecycle.begin(changed, now).expect("generation");
    assert_eq!(
        lifecycle.apply(completion(old.token, Ok("old")), now),
        ApplyOutcome::Ignored
    );
    assert_eq!(
        lifecycle.apply(completion(newest.token, Ok("ready")), now),
        ApplyOutcome::Updated
    );
    assert_eq!(lifecycle.last_good(), Some("ready"));
}

#[test]
fn failed_unchanged_input_retries_after_backoff_then_deduplicates_success() {
    let mut lifecycle = Lifecycle::new(7);
    let now = Instant::now();
    let failed = lifecycle.begin(input(), now).expect("initial invocation");
    assert_eq!(
        lifecycle.apply(completion(failed.token, Err("transient")), now),
        ApplyOutcome::RetryAt(now + RETRY_BACKOFF)
    );
    assert_eq!(lifecycle.begin(input(), now), None);
    let retry_at = now + RETRY_BACKOFF;
    let recovered = lifecycle
        .begin(input(), retry_at)
        .expect("retry after backoff");
    assert_eq!(
        lifecycle.apply(completion(recovered.token, Ok("recovered")), retry_at),
        ApplyOutcome::Updated
    );
    assert_eq!(lifecycle.begin(input(), retry_at + RETRY_BACKOFF), None);
    assert_eq!(lifecycle.last_good(), Some("recovered"));
}
