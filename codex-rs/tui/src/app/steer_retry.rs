//! Retry policy for transient app-server overload while steering an active turn.

use codex_app_server_client::TypedRequestError;
use rand::Rng;
use std::time::Duration;

const MAX_RETRIES: u32 = 3;
const INITIAL_DELAY_MS: u64 = 100;

#[derive(Default)]
pub(super) struct SteerOverloadRetry {
    retries: u32,
}

pub(super) enum SteerRequestOutcome<T> {
    Success(T),
    PersistentOverload(TypedRequestError),
    Failed(TypedRequestError),
}

pub(super) async fn retry_turn_steer<Request, T>(request: Request) -> SteerRequestOutcome<T>
where
    Request: AsyncFnMut() -> Result<T, TypedRequestError>,
{
    retry_turn_steer_with_sleep(request, async |delay| tokio::time::sleep(delay).await).await
}

async fn retry_turn_steer_with_sleep<Request, Sleep, T>(
    mut request: Request,
    mut sleep: Sleep,
) -> SteerRequestOutcome<T>
where
    Request: AsyncFnMut() -> Result<T, TypedRequestError>,
    Sleep: AsyncFnMut(Duration),
{
    let mut retry = SteerOverloadRetry::default();
    loop {
        match request().await {
            Ok(response) => return SteerRequestOutcome::Success(response),
            Err(error) => {
                if let Some(delay) = retry.next_delay(&error) {
                    tracing::warn!(
                        error = %error,
                        delay_ms = delay.as_millis(),
                        "turn/steer overloaded; retrying"
                    );
                    sleep(delay).await;
                } else if retry.is_exhausted_overload(&error) {
                    return SteerRequestOutcome::PersistentOverload(error);
                } else {
                    return SteerRequestOutcome::Failed(error);
                }
            }
        }
    }
}

impl SteerOverloadRetry {
    pub(super) fn next_delay(&mut self, error: &TypedRequestError) -> Option<Duration> {
        if !is_turn_steer_overload(error) || self.retries >= MAX_RETRIES {
            return None;
        }

        let base_ms = INITIAL_DELAY_MS * 2_u64.pow(self.retries);
        self.retries += 1;
        let jitter = rand::rng().random_range(0.9..1.1);
        Some(Duration::from_millis((base_ms as f64 * jitter) as u64))
    }

    pub(super) fn is_exhausted_overload(&self, error: &TypedRequestError) -> bool {
        self.retries == MAX_RETRIES && is_turn_steer_overload(error)
    }
}

fn is_turn_steer_overload(error: &TypedRequestError) -> bool {
    matches!(
        error,
        TypedRequestError::Server { method, source }
            if method == "turn/steer" && source.code == -32001
    )
}

#[cfg(test)]
#[path = "steer_retry_tests.rs"]
mod tests;
