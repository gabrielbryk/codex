# Improvement and Reporting

Use for every durable report and for all `improve` work.

## Bounded run evidence

Reports live under
`${FORK_FLEET_STATE_ROOT:-~/.local/state/fork-fleet}/codex-upgrades/<run-id>/` as `report.json` and
`report.md`. Update the same run ID at meaningful/terminal boundaries with exact SHAs, compact gate
results, and bounded notes.

Persist phase checkpoints through the report helper:

```bash
codex-upgrade-report <run arguments> \
  --checkpoint <exact-sha> <phase> <status> <artifact> <bounded-note> <64-hex-input-fingerprint>
```

The helper adds update/completion timestamps. A resumed session may reuse a completed checkpoint
only when its SHA, fingerprint, and required artifacts still match; legacy checkpoints without a
fingerprint are evidence but are never reusable. Otherwise invalidate and rerun only that phase.
Never repeat a completed gate solely because the session resumed or the report was reopened.

Persist justified delegation with `--delegation <model> <task> <outcome>`; record the uncertainty and
bounded evidence path. Scripts own routine collection. Do not delegate a dossier or inventory that
the helper already computes. Escalate only for concrete semantic ambiguity or safety adjudication.

Source-bound `gate --plan --gate-id` receipts are the reuse authority; older report checkpoints are
historical context unless independently matched by the helper. Run `metrics` at handoff and record
its bounded artifact: duplicates, reuse, invalidation, unfinished attempts, elapsed gate time, review
reuse, and explicitly recorded abandoned work. Preserve unknown token cost without request evidence.

Reports expose machine-readable `engineeringState` and `rolloutState` separately. Engineering may
be `pending`, `running`, `ready`, `blocked`, `failed`, or `complete`; rollout may be
`not_requested`, `staged`, `draining`, `current`, `blocked`, or `failed`. Link the bounded preflight
or rollout checkpoint artifact containing exact current generation, desired SHA, drain/connection
counts, and pending action. Prose must not imply an engineering ETA while rollout is merely draining.

Allowed friction codes are:

- `merge-base-expanded-range`
- `release-floor-blocked`
- `package-head-drift`
- `package-manifest-invalid`
- `skill-install-conflict`
- `active-turn-cutover-deferred`
- `router-bootstrap-deferred`
- `generation-drain-deferred`
- `generation-contract-incompatible`
- `validation-environment-failure`

Do not put raw logs, environment dumps, credentials, tokens, authenticated URLs, or arbitrary error
output in reports. Store a bounded artifact path and a one-line classification instead.

Final reports separate:

- mode and run path;
- source tip, explicit source base, target ref/SHA, candidate ID/SHA;
- applied/reworked/dropped patches and supersession evidence;
- compact exact validation results and classifications;
- safety refs and branch movement;
- package path and `fork-build.json` source SHA;
- current link, CLI/daemon/app-server versions, active generation and drain state;
- tree cleanliness, preserved unrelated work, and real remaining blockers;
- improvement friction, regression test, source fix, or why no source change was justified.

## Improve mode

Self-improvement normally occurs only after an upgrade reaches a terminal boundary. If the user
explicitly requests an immediate correction to a concrete workflow defect during an active run,
limit the change to the source-owned skill, keep the candidate and runtime untouched, record the
improve run separately, and resume the active upgrade afterward.

For the normal terminal improvement pass:

1. Read the terminal report, bounded candidate logs, transcript summary, and exact failed command.
   Read historical postmortems only for the matching failure; they are not required upgrade context.
2. Classify each issue with an allowed friction code and distinguish deterministic tooling defects
   from external state or one-off operator mistakes.
3. Change source only when a regression test reproduces a deterministic defect or the same friction
   appears in at least two independent reports.
4. Route ownership first: this skill and Codex behavior belong in the Codex fork. Fork Fleet
   contracts, helpers, reports, and installers belong in Worklens `libs/fork-fleet`. Never edit
   plugin caches, installed bundles, or generated deployment copies.
5. Add the smallest regression test first and validate the owning repository. Use Codex's
   `AGENTS.md` contract for repo-local skill or product changes; for Fork Fleet tooling run
   `moon run fork-fleet:check`, then `moon run :check --concurrency 4`.
6. Reproduce the former failure in a disposable home/worktree. Do not publish, activate, restart,
   force-push, or rewrite registry intent merely to test an improvement.
7. Update the improve run report with validation and the fix summary.

For external state, ambiguous patch intent, isolated operator error, or outages, improve the
playbook/classification only when supported by completed-run evidence; do not invent automation
that broadens mutation authority.
