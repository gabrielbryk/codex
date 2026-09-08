# Coordination and Status

Use this playbook when the user authorizes delegation or the upgrade contains long-running work.

## Delegation and model routing

Run deterministic collection through scripts first: dossier, template, next, metrics, and bounded
log extraction. Do not create an agent for work those commands already perform. Delegate only when
there is a concrete semantic uncertainty, independent review value, or useful disjoint implementation.
Give the agent the affected family packet and precise question, not a complete transcript or diff.
Choose the least costly adequate model; escalate only with an evidence-backed reason. Keep release
authority and ambiguous safety decisions with the primary agent. A long task alone is not a reason
to escalate or fan out. The coordinator owns the heavy-command queue:
agents may prepare commands and inspect results, but Cargo, Bazel, Clippy, full-test, and package
builds run serially through that queue.

Use at most 2–4 justified lanes, each with disjoint file ownership, while reserving capacity
for coordination. Every delegated editing, review, or testing prompt must include:

- the exact candidate path and recorded HEAD;
- an instruction to read the candidate's complete root `AGENTS.md` and every more-specific
  `AGENTS.md` that governs owned paths before acting;
- disjoint file ownership and whether writes are allowed;
- an explicit prohibition on starting Cargo, Bazel, Clippy, test, formatter, generator, or package
  commands unless the coordinator assigned that exact command;
- an explicit prohibition on staging, committing, finalizing, publishing, or cutover unless that
  authority was separately assigned.

Do not rely on inherited conversation context for these constraints, especially when a cheaper
model is spawned with limited or no forked turns. Give every agent a bounded deliverable and this
compact return schema:

```text
Task/status: <id> / <done|blocked|escalate>
Files: <changed or inspected paths>
Evidence: <commands, counts, tests, and relevant symbols>
Failures: <novel failures only, or none>
Uncertainty/decision: <none, or the precise question for escalation>
```

Avoid overlapping “review everything” agents and never stream raw logs; the coordinator owns
cross-agent synthesis and final status.

## Edit/gate phase barrier

Use a strict phase barrier for candidate work:

1. **Edit:** run only disjoint writer lanes. The coordinator does not start a heavy gate.
2. **Join:** wait for every agent with candidate write authority to finish. Do not send an editor a
   follow-up that can write while a gate is running.
3. **Review and seal:** inspect returned diffs, commit only intended changes, and continue/finalize
   through Fork Fleet. No staged, unstaged, or untracked source files may remain.
4. **Freeze:** record the coordinator SHA and mark all writer lanes inactive.
5. **Gate:** the coordinator starts one serialized command. Read-only evidence agents may continue,
   but nothing else may write anywhere in the candidate, including disjoint files.
6. **Capture:** the source-bound gate verifies source/plan/tool inputs and stores a receipt; only an
   authoritative receipt may advance `next`. Review failures rather than accepting process exit alone.

A mutating formatter, fixer, or generator is itself the sole writer during step 5. Review and stage
its output, then establish a new frozen snapshot before the next read-only gate.

If overlap is discovered after a valid long-running command starts, do not interrupt it. Let it
finish for cache warming and diagnostic evidence, record that its candidate input drifted and its
result is non-authoritative, join the writers, and rerun only the exact narrow gate required for the
frozen snapshot. Never respond by rerunning the broad suite.

## Critical-path ledger

Maintain one compact ledger and update it only when state changes:

```text
Target: <ref> @ <sha>
Patches: <decided>/<total> (<apply>/<rework>/<drop>)
Candidate: <id> @ <sha> (<clean|dirty>)
Candidate snapshot: <committed coordinator SHA + verified receipt, or unfrozen>
Writer lanes: <active count and owners, or joined>
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
