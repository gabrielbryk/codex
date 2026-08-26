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

Produce a compact decision table before replay: patch ID, decision, upstream evidence, fork intent
remaining, expected conflicts, and required validation. Every source commit must map exactly once.

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

Continue and inspect with:

```bash
forkctl candidate continue <candidate-id>
forkctl candidate show <candidate-id> --json
forkctl logs show <candidate-id> --json
```

If reviewed reconciliation adds commits after the candidate was ready, use the exact-SHA
`candidate finalize` compare-and-swap and rerun only validation invalidated by that finalization.
Never mutate, switch, reset, clean, or rebase the maintained checkout.
