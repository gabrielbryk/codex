use super::parser::ParsedStatusLine;
use super::wire::StatusLineCommandInput;
use std::time::Duration;
use std::time::Instant;

pub(crate) const STATUS_LINE_COMMAND_DEBOUNCE: Duration = Duration::from_millis(300);
const RETRY_BACKOFF: Duration = Duration::from_millis(250);
const MAX_CONSECUTIVE_FAILURES: u8 = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RequestToken {
    owner: u64,
    generation: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Invocation {
    pub(crate) token: RequestToken,
    pub(crate) input: StatusLineCommandInput,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Completion {
    pub(crate) token: RequestToken,
    pub(crate) result: Result<ParsedStatusLine, String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ApplyOutcome {
    Ignored,
    Updated,
    RetryAt(Instant),
}

#[derive(Debug)]
pub(crate) struct Lifecycle {
    owner: u64,
    generation: u64,
    last_good: Option<ParsedStatusLine>,
    last_input: Option<StatusLineCommandInput>,
    consecutive_failures: u8,
    retry_not_before: Option<Instant>,
}

impl Lifecycle {
    pub(crate) fn new(owner: u64) -> Self {
        Self {
            owner,
            generation: 0,
            last_good: None,
            last_input: None,
            consecutive_failures: 0,
            retry_not_before: None,
        }
    }

    pub(crate) fn begin(
        &mut self,
        input: StatusLineCommandInput,
        now: Instant,
    ) -> Option<Invocation> {
        if self.last_input.as_ref() == Some(&input) {
            if self.consecutive_failures == 0
                || self.consecutive_failures >= MAX_CONSECUTIVE_FAILURES
                || self.retry_not_before.is_some_and(|deadline| now < deadline)
            {
                return None;
            }
        } else {
            self.consecutive_failures = 0;
            self.retry_not_before = None;
            self.last_input = Some(input.clone());
        }
        self.generation = self.generation.checked_add(1)?;
        Some(Invocation {
            token: RequestToken {
                owner: self.owner,
                generation: self.generation,
            },
            input,
        })
    }

    pub(crate) fn apply(&mut self, completion: Completion, now: Instant) -> ApplyOutcome {
        if completion.token.owner != self.owner || completion.token.generation != self.generation {
            return ApplyOutcome::Ignored;
        }
        match completion.result {
            Ok(value) => {
                self.last_good = Some(value);
                self.consecutive_failures = 0;
                self.retry_not_before = None;
                ApplyOutcome::Updated
            }
            Err(_) => {
                self.consecutive_failures = self.consecutive_failures.saturating_add(1);
                let retry_at = *self.retry_not_before.insert(now + RETRY_BACKOFF);
                if self.consecutive_failures < MAX_CONSECUTIVE_FAILURES {
                    ApplyOutcome::RetryAt(retry_at)
                } else {
                    ApplyOutcome::Ignored
                }
            }
        }
    }

    pub(crate) fn last_good(&self) -> Option<&ParsedStatusLine> {
        self.last_good.as_ref()
    }
}

#[cfg(test)]
#[path = "runner_tests.rs"]
mod tests;
