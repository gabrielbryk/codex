# Coordination and Status

Use this playbook when the user authorizes delegation or the upgrade contains long-running work.

## Delegation and model routing

Use cheap-first delegation. Luna is the default for every safely separable mechanical or evidence
task:

- inventory, searches, commit mapping, and patch evidence collection;
- enumerate and fix a complete static-lint finding set;
- update snapshots or generated artifacts after intent is established;
- audit disjoint logical patches for exact symbols, tests, or upstream equivalents;
- make repetitive call-site migrations with explicit file ownership;
- classify logs, collect structured status, and perform other bounded, deterministic checks.

Use Terra for one bounded semantic conflict, narrow-test diagnosis, or review of a Luna result when
the evidence does not settle the behavior. Reserve Sol/frontier for high-risk ambiguity: partial
upstream supersession, cross-cutting architecture conflicts, ambiguous regression classification,
release/publish/cutover decisions, and final safety adjudication. Keep semantic conflict resolution,
partial-supersession decisions, release classification, and final ship authority with the primary
agent unless explicitly escalated.

Escalate only when a concrete evidence-backed uncertainty remains after a Luna attempt, when the
decision can drop preserved fork behavior, corrupt history, publish the wrong SHA, or interrupt the
runtime, or when independent evidence conflicts. Do not escalate merely because a task is long;
split it into smaller mechanical units first. Deterministic checks and tests should verify mechanical
work instead of spending frontier tokens on review. The coordinator owns the heavy-command queue:
agents may prepare commands and inspect results, but Cargo, Bazel, Clippy, full-test, and package
builds run serially through that queue.

Use at most 2–4 concurrent Luna lanes, each with disjoint file ownership, while reserving capacity
for coordination. Give every agent a bounded deliverable, exact candidate path/SHA, and this compact
return schema:

```text
Task/status: <id> / <done|blocked|escalate>
Files: <changed or inspected paths>
Evidence: <commands, counts, tests, and relevant symbols>
Failures: <novel failures only, or none>
Uncertainty/decision: <none, or the precise question for escalation>
```

Avoid overlapping “review everything” agents and never stream raw logs; the coordinator owns
cross-agent synthesis and final status.

## Critical-path ledger

Maintain one compact ledger and update it only when state changes:

```text
Target: <ref> @ <sha>
Patches: <decided>/<total> (<apply>/<rework>/<drop>)
Candidate: <id> @ <sha> (<clean|dirty>)
Current gate: <exact gate and progress>
Real blockers: <candidate regressions only>
Classified noise: <count by upstream/environment/flake>
Package: <pending|building|ready path>
Rollout: <not requested|staged|draining|current>
ETA: <remaining active command time>; natural drain is unbounded passive waiting
```

Do not call a deferred drain state a blocker. Do not mix already elapsed work into ETA. If a command
has a stable duration history, give its measured range; otherwise name the uncertainty rather than
offering a wide unsupported estimate.

## Token and output discipline

- Persist full command logs to bounded artifacts and inspect them locally with narrow searches.
- Return counts, duration, failed names, and only novel excerpts to the primary agent.
- Do not paste full Fork Fleet JSON, rollout JSON, passing-test streams, or repeated unchanged state.
- Poll at intervals appropriate to the command and request compact progress, not cumulative output.
- On a user status request, answer from the ledger immediately before doing more work.
- Report which models were delegated and what each owns when the user asks or has expressed a model
  preference.

Repeated user status requests are evidence that the ledger or boundary explanation is inadequate;
improve the next update rather than restating the previous one.
