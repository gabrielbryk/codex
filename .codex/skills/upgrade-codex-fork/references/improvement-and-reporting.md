# Improvement and Reporting

Use for every durable report and for all `improve` work.

## Bounded run evidence

Reports live under
`${FORK_FLEET_STATE_ROOT:-~/.local/state/fork-fleet}/codex-upgrades/<run-id>/` as `report.json` and
`report.md`. Update the same run ID at meaningful/terminal boundaries with exact SHAs, compact gate
results, and bounded notes.

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

Self-improvement occurs only after an upgrade reaches a terminal boundary:

1. Read the terminal report, bounded candidate logs, transcript summary, and exact failed command.
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
