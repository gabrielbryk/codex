# Planning and Reconciliation

Use this playbook for `check`, `prepare`, and the planning portion of `ship`.

## Preservation and target resolution

Start from `/mnt/wd-black/ClonedRepos/codex` and run commands raw, without `rtk`:

```bash
git status --short --branch
git worktree list --porcelain
git stash list
git show-ref --heads --tags
git remote -v
readlink -f ~/.codex/packages/standalone/current
/home/gabe/.local/bin/codex-app-server-active-turns
git fetch --prune --tags origin
git fetch --prune gabrielbryk
forkctl registry validate --json
forkctl plan codex --json
```

Stop mutation if the maintained checkout or candidate is dirty. Inventory unrelated dirty feature
worktrees without touching them. Recheck the same preservation inventory after preparation.

The plan must report the registry's explicit `sourceBaseSha` and bounded `sourceCommitOrder`.
Never replace the declared patch-stack base with a release tag's merge base. Record
`merge-base-expanded-range` if that merge base would expand the range.

Run the installed repo-owned workflow helper once before expensive planning or validation:

```bash
codex-upgrade-workflow preflight \
  --repo /mnt/wd-black/ClonedRepos/codex \
  --fork-id codex > <run-dir>/preflight.json
```

Persist this bounded artifact and use it as the coordinator's source for repository/worktree/stash,
package, runtime, registry, plan, and patch-template evidence. A stale or contradictory preflight is
a hard stop until refreshed; do not compensate by reconstructing the same inventory manually.

Interpret target intent literally:

- An explicit version or stable-release request wins over registry prerelease selection.
- Default to the latest stable `rust-v*` release when the user did not name a target.
- `minimum_version` is a floor, not permission to select a prerelease.
- Use fetched `origin/main` only for an explicit “main” or “latest upstream” request.
- Verify release metadata and report the exact ref and SHA before preparation.

## Logical patch decisions

For every active patch, inspect `maintenance_class`, `required_symbols`, `required_tests`,
`conflict_hotspots`, and `upstream_references`. Record exactly one decision:

- `apply`: behavior is still absent upstream and the patch applies without semantic redesign.
- `rework`: intent remains necessary but upstream architecture or APIs changed.
- `drop`: upstream now provides equivalent behavior and meaningful test coverage.

Similar symbols, nearby refactors, or a clean cherry-pick are not supersession proof. Compare
observable behavior, failure handling, configuration/API compatibility, and tests. When only part
of a patch is superseded, rework the surviving intent rather than dropping the logical patch.

Have Luna populate the helper-generated patch-evidence/template artifact with facts for every patch:
patch ID and source commit, exact symbols/files/tests, upstream equivalent or absence, behavior and
failure-handling comparison, expected conflicts, and uncertainty. Luna returns compact evidence and
bounded artifact links, not a final decision. The primary agent or a stronger reviewer adjudicates
only ambiguous `apply`/`rework`/`drop` cases, partial supersession, and contradictory evidence.
Deterministic evidence collection and mechanical template completion stay on the cheap agent path.

Produce a compact decision table before replay: patch ID, decision, upstream evidence, fork intent
remaining, expected conflicts, required validation, and adjudicator. Every source commit must map
exactly once. A decision cannot be marked complete merely because the patch applies cleanly.

## Candidate preparation

Save the plan JSON and `CandidateDecisionV1[]` outside managed repositories, with one bounded reason
per patch. Then prepare the immutable reviewed plan:

```bash
forkctl prepare codex \
  --plan <plan-id> \
  --digest <plan-digest> \
  --decisions <decision-json>
```

Resolve conflicts only in the candidate worktree. Read both sides and the patch metadata. Never use
wholesale ours/theirs for RMCP, TUI orchestration, app-server daemon, or process-isolation changes.
Candidate rerere has auto-staging disabled; review and stage every reused resolution.

Use the workflow helper's coordinator-owned heavy-gate lock for every expensive build or test. The
primary agent schedules and records those gates; delegated agents may analyze bounded results or
make disjoint mechanical edits, but must not launch competing Cargo, Bazel, Clippy, full-test, or
package commands. After finalization changes the candidate SHA, invalidate only gates that depend
on that SHA and regenerate the relevant bounded evidence; do not rerun unrelated gates.

Continue and inspect with:

```bash
forkctl candidate continue <candidate-id>
forkctl candidate show <candidate-id> --json
forkctl logs show <candidate-id> --json
```

If reviewed reconciliation adds commits after the candidate was ready, use the exact-SHA
`candidate finalize` compare-and-swap and rerun only validation invalidated by that finalization.
Never mutate, switch, reset, clean, or rebase the maintained checkout.
