# Coordination and Status

Use this playbook when the user authorizes delegation or the upgrade contains long-running work.

## Delegation

Prefer Luna agents for bounded mechanical work:

- enumerate and fix a complete static-lint finding set;
- update snapshots or generated artifacts after intent is established;
- audit disjoint logical patches for exact symbols/tests/upstream equivalents;
- make repetitive call-site migrations with explicit file ownership.

Keep semantic conflict resolution, partial-supersession decisions, release classification, and final
ship authority with the primary agent. Give every agent a bounded deliverable, disjoint files, exact
candidate path/SHA, and a return schema containing evidence rather than raw logs. Avoid overlapping
“review everything” agents.

Parallelize read-only analysis and disjoint edits. Serialize Cargo, Bazel, Clippy, full test, and
package builds: parallel build agents contend for CPU, locks, memory, and caches and can make the host
appear pegged without shortening the critical path.

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
