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

First generate the [release-history dossier](release-history.md). Pin the target for this run;
a newly discovered hotfix requires an explicit retarget decision, an endpoint delta, and a fresh
immutable plan, not a restart of all evidence collection. Reuse unaffected patch-family evidence
with its original provenance, but do not claim old-SHA validation proves the new candidate.

Before replay, record the publication destination and authority separately from the upstream fetch
remote. A prepare-only registry ceiling or missing fork write destination is an early shipping
blocker, not a reason to discover the problem after a release build. Do not silently change policy.
Record the installed host lifecycle contract and every in-scope Codex home/client generation too.

For every active patch, inspect `maintenance_class`, `required_symbols`, `required_tests`,
`conflict_hotspots`, and `upstream_references`. Record exactly one decision:

- `apply`: behavior is still absent upstream and the patch applies without semantic redesign.
- `rework`: intent remains necessary but upstream architecture or APIs changed.
- `drop`: upstream now provides equivalent behavior and meaningful test coverage.

Similar symbols, nearby refactors, or a clean cherry-pick are not supersession proof. Compare
observable behavior, failure handling, configuration/API compatibility, and tests. When only part
of a patch is superseded, rework the surviving intent rather than dropping the logical patch.

`rework` is not permission to replay historical commits and resolve conflicts until they compile.
Before preparation, require a target-native design, retained invariant, test mapping, and bounded
implementation stages for each reworked family. Test upstream's replacement first where possible.
If a dropped patch supplies an API used by the host router/controller, either retain that contract
or record an authorized host migration as a release dependency. Do not discover this at activation.

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
primary agent schedules and records those gates. Delegated agents may analyze bounded results at
any time and may make disjoint mechanical edits only during an explicit edit phase; they must not
launch competing Cargo, Bazel, Clippy, full-test, or package commands. Before a gate, join every
writer lane, review and stage the intended edits, record the candidate snapshot, and freeze the
candidate until the result is captured. After finalization changes the candidate SHA, invalidate
only gates that depend on that SHA and regenerate the relevant bounded evidence; do not rerun
unrelated gates.

Continue and inspect with:

```bash
forkctl candidate continue <candidate-id>
forkctl candidate show <candidate-id> --json
forkctl logs show <candidate-id> --json
```

Never follow Git's generic `git cherry-pick --continue` hint inside a coordinator-owned candidate.
Stage the reviewed resolution, then use `forkctl candidate continue`. A persisted/worktree SHA
mismatch is a coordinator-state defect, not a test-environment failure; preserve it and stop replay.

If reviewed reconciliation adds commits after the candidate was ready, use the exact-SHA
`candidate finalize` compare-and-swap and rerun only validation invalidated by that finalization.
Never mutate, switch, reset, clean, or rebase the maintained checkout.
