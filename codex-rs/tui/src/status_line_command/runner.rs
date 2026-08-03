//! Request ownership and last-known-good state for formatter runs.
//!
//! The process implementation consumes [`StatusLineCommandInvocation`] after a
//! fixed debounce. Completion tokens contain both a per-widget owner ID and a
//! monotonically increasing generation, so late work from a replaced widget or
//! superseded snapshot cannot update the visible footer.

use std::time::Duration;

use thiserror::Error;
use uuid::Uuid;

use super::parser::ParsedStatusLine;
use super::wire::StatusLineCommandInput;
use super::wire::StatusLineCommandSessionId;

pub(crate) const STATUS_LINE_COMMAND_DEBOUNCE: Duration = Duration::from_millis(300);

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct StatusLineCommandOwnerId(Uuid);

impl StatusLineCommandOwnerId {
    fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct StatusLineCommandGeneration(u64);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StatusLineCommandRequestToken {
    pub(crate) owner_id: StatusLineCommandOwnerId,
    pub(crate) generation: StatusLineCommandGeneration,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct StatusLineCommandInvocation {
    pub(crate) token: StatusLineCommandRequestToken,
    pub(crate) input: StatusLineCommandInput,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StatusLineCommandFailureKind {
    Spawn,
    Stdin,
    ExitStatus,
    Timeout,
    OutputLimit,
    Parse,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StatusLineCommandFailure {
    pub(crate) kind: StatusLineCommandFailureKind,
    pub(crate) message: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum StatusLineCommandOutcome {
    Success(ParsedStatusLine),
    Failure(StatusLineCommandFailure),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StatusLineCommandCompletion {
    pub(crate) token: StatusLineCommandRequestToken,
    pub(crate) outcome: StatusLineCommandOutcome,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StatusLineCommandApplyResult {
    Updated,
    RetainedLastGood,
    Stale,
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[error("status-line command generation exhausted")]
pub(crate) struct StatusLineCommandGenerationExhausted;

#[derive(Clone, Debug)]
pub(crate) struct StatusLineCommandLifecycle {
    owner_id: StatusLineCommandOwnerId,
    session_id: StatusLineCommandSessionId,
    latest_generation: StatusLineCommandGeneration,
    last_good: Option<ParsedStatusLine>,
}

impl StatusLineCommandLifecycle {
    pub(crate) fn new() -> Self {
        Self {
            owner_id: StatusLineCommandOwnerId::new(),
            session_id: StatusLineCommandSessionId::new(),
            latest_generation: StatusLineCommandGeneration(0),
            last_good: None,
        }
    }

    pub(crate) fn session_id(&self) -> &StatusLineCommandSessionId {
        &self.session_id
    }

    /// Mark a new immutable snapshot as newest and return its ownership token.
    pub(crate) fn begin(
        &mut self,
        mut input: StatusLineCommandInput,
    ) -> Result<StatusLineCommandInvocation, StatusLineCommandGenerationExhausted> {
        let next_generation = self
            .latest_generation
            .0
            .checked_add(1)
            .ok_or(StatusLineCommandGenerationExhausted)?;
        self.latest_generation = StatusLineCommandGeneration(next_generation);
        input.session_id = self.session_id.clone();
        Ok(StatusLineCommandInvocation {
            token: StatusLineCommandRequestToken {
                owner_id: self.owner_id.clone(),
                generation: self.latest_generation,
            },
            input,
        })
    }

    /// Apply only the newest completion owned by this widget.
    ///
    /// Failures intentionally preserve `last_good`; diagnostics are emitted by
    /// the caller, which owns per-session/config-revision log suppression.
    pub(crate) fn apply(
        &mut self,
        completion: StatusLineCommandCompletion,
    ) -> StatusLineCommandApplyResult {
        if completion.token.owner_id != self.owner_id
            || completion.token.generation != self.latest_generation
        {
            return StatusLineCommandApplyResult::Stale;
        }

        match completion.outcome {
            StatusLineCommandOutcome::Success(parsed) => {
                self.last_good = Some(parsed);
                StatusLineCommandApplyResult::Updated
            }
            StatusLineCommandOutcome::Failure(_) => StatusLineCommandApplyResult::RetainedLastGood,
        }
    }

    pub(crate) fn last_good(&self) -> Option<&ParsedStatusLine> {
        self.last_good.as_ref()
    }
}

#[cfg(test)]
#[path = "runner_tests.rs"]
mod tests;
