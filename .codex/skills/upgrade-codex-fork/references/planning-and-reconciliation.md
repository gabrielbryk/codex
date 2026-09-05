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

### Operator and runtime registry freshness

The preflight must prove that the operator-facing `forkctl` invocation and the workflow helper read
the same current registry on this invocation. Record the resolved `forkctl` launcher, canonical
registry path, registry validation generation time, plan ID/digest, and registry/source-plan
fingerprint. Then require a newly executed `forkctl plan` to either reload that registry or fail
closed. A plan retained by a daemon, process cache, old environment override, or prior preflight is
not current evidence, even when its plan ID still resolves.

Stop before replay if the registry path, target SHA, plan digest, patch count, or leaf contracts
differ between registry validation, the fresh plan, and the preflight summary. Do not reuse an old
plan or copy its decisions into a new file to work around the mismatch.

Also identify whether the resolved `forkctl` launcher executes a generated CLI artifact. If it
does, compare that artifact with its source-owned CLI inputs or recorded build provenance and emit
a bounded warning when it may be stale. The current source-tree launcher may legitimately execute
an up-to-date generated `dist` artifact; that is not itself a warning. Generated-CLI freshness is
diagnostic: it never downgrades contradictory registry or plan state from a hard failure.

Interpret target intent literally:

- An explicit version or stable-release request wins over registry prerelease selection.
- Default to the latest stable `rust-v*` release when the user did not name a target.
- `minimum_version` is a floor, not permission to select a prerelease.
- Use fetched `origin/main` only for an explicit “main” or “latest upstream” request.
- Verify release metadata and report the exact ref and SHA before preparation.

Inventory tags at the target, sibling stable or prerelease tags, and release/package records that
claim its SHA. Record each tag object/commit and artifact source marker; stop on disagreement.

## Logical patch decisions

For a `leaf-v1` plan, first verify that every active leaf's `adjudicatedTargetSha` exactly equals the
fresh plan's resolved `targetSha`. Any mismatch invalidates the leaf's recommendation and decision
basis. Refresh target-specific upstream evidence and the source-owned registry before replay; never
carry forward a static decision table from a previous target merely because paths still exist.

Perform an exact ownership audit before replay:

1. Map every source commit and changed path to exactly one leaf `sourceSurface`, including hunks
   hidden in broad historical commits.
2. Separately map every proposed target-native edit to that leaf's `replacementSurface`. Source
   surfaces authorize source accounting; they do not authorize replacement writes.
3. Require overlapping effective replacement surfaces to be declared symmetrically in
   `sharedSurfaces` by every participating leaf. One-sided sharing is a registry defect.
4. For every source-free active leaf, require non-empty `supersedes`.
   Only source-backed, active, reviewed-drop quarantine leaves qualify.
   Prove each is intentionally dropped without replay;
   upstream, inactive, or ordinary feature leaves are invalid anchors.
5. Validate dependencies after dispositions: retained leaves may depend on applied, reworked, or
   upstream-satisfied leaves, never on an intentionally dropped leaf.

Treat `upstream` as a proved no-replay disposition: record the upstream owner, behavior, and tests,
then pass an explicit reviewed `drop` to the stable prepare API. Treat intentional `drop` the same
way for replay selection, but keep its omission rationale distinct. Neither disposition may
contribute a source commit, empty placeholder, or replacement edit. In leaf-v1, `rework` also skips
all source commits and is completed only by target-native replacement commits carrying exact
`Fork-Fleet-Rework` trailers.

For every active patch, inspect `maintenance_class`, `required_symbols`, `required_tests`,
`conflict_hotspots`, and `upstream_references`. Record exactly one decision:

- `apply`: behavior is still absent upstream and the patch applies without semantic redesign.
- `rework`: intent remains necessary but upstream architecture or APIs changed.
- `drop`: upstream now provides equivalent behavior and meaningful test coverage.

Similar symbols, nearby refactors, or a clean cherry-pick are not supersession proof. Compare
observable behavior, failure handling, configuration/API compatibility, and tests. When only part
of a patch is superseded, rework the surviving intent rather than dropping the logical patch.

Treat upstream ownership as stronger evidence than textual equivalence. For every lifecycle patch,
compare source and target and record the exact production symbols, authoritative producer -> owner
-> consumer -> completion path, focused integration test IDs and artifacts, and which owner wins.
An independent semantic reviewer must verify observable behavior, concurrency boundaries, failure
handling, persistence, privacy, and model-context bounds. The ledger and prose audit can prove that
these fields exist, not that the selected owner or test is adequate; unknown adequacy means `rework`.

Produce a compact decision table before replay: patch ID, decision, upstream evidence, fork intent
remaining, expected conflicts, required validation, and adjudicator. Every source commit must map
exactly once. A decision cannot be marked complete merely because the patch applies cleanly.

Maintain a per-leaf implementation/test ledger: target SHA, disposition, apply-only source commits,
rework-only replacement commit and paths, supersession/sharing, validation IDs, exact commands,
outcomes, and artifacts. Upstream/drop leaves record zero replay and replacement commits. Behavior
leaves also record the lifecycle trace and focused integration test in a target- and candidate-SHA
checkpoint. This is explicit SHA-bound manual evidence; an independent reviewer judges its semantics.

Empty replayed commits are positive supersession evidence, not harmless history. Reopen the owning
logical decision whenever a commit becomes empty, when its producer survives but its consumer was
dropped, or when tests still name removed fields/methods. Either drop the obsolete behavior or
rework the complete end-to-end path. Do not retain empty reconnect, generated-artifact, formatting,
or release-normalization commits in the final candidate.

## Candidate preparation

Save the plan JSON and `CandidateDecisionV1[]` outside managed repositories, with one bounded reason
per patch. Then prepare the immutable reviewed plan:

```bash
forkctl prepare codex \
  --plan <plan-id> \
  --digest <plan-digest> \
  --decisions <decision-json>
```

Before invoking prepare, regenerate the decision file from the fresh immutable plan. Do not reuse
static decisions from a prior plan or target. Confirm that selected replay commits contain only
`apply` leaves; `rework`, `upstream`, and intentional `drop` leaves must select no source commits.

Before staging or finalizing a leaf, compute its aggregate per-leaf diff from the target across all
of that leaf's commits with `git diff --numstat`. The non-mechanical limit is 800 changed lines; keep
complex logic below 500 changed lines. Record additions plus deletions and classification. Commit
splitting never resets the budget. This aggregate is manual, SHA-bound reviewer evidence; Fork Fleet
does not currently enforce the size limit.

Resolve conflicts only in the candidate worktree. Read both sides and the patch metadata. Never use
wholesale ours/theirs for RMCP, TUI orchestration, app-server daemon, or process-isolation changes.
Candidate rerere has auto-staging disabled; review and stage every reused resolution.

Use the coordinator-owned heavy-gate lock. Join writers, review and stage, freeze the candidate
snapshot, and forbid competing Cargo/Bazel/Clippy/test/package work. After finalization changes the
SHA, invalidate only SHA-dependent gates.

Continue and inspect with:

```bash
forkctl candidate continue <candidate-id>
forkctl candidate show <candidate-id> --json
forkctl logs show <candidate-id> --json
```

If reviewed reconciliation adds commits after the candidate was ready, use the exact-SHA
`candidate finalize` compare-and-swap and rerun only validation invalidated by that finalization.
Never mutate, switch, reset, clean, or rebase the maintained checkout.

For a leaf with `phase = "final"`, its replacement commit owns only the declared final/generated
surfaces and must follow every semantic rework commit. If any repair commit lands after that tail,
regenerate and commit a newer final tail after the repair; the earlier final evidence is stale.
Fork Fleet alone enforces final-phase semantic ordering through immutable-plan-aware `forkctl
candidate finalize`; the documentation/repository coherence audit never infers candidate state.

## Frozen candidate coherence review

Freeze the ready candidate and review the full target diff once. Trace new API to callers,
completion, and tests; compare lifecycle owners; inspect concurrency, persistence, context bounds,
and privacy; attribute every path to one leaf; regenerate current manifests; remove unexplained
empty commits and stale artifacts.

For a large or lifecycle-sensitive stack, use independent read-only reviewers for external API and
breaking behavior, model-visible context, testing, and maintainability. All reviewers must receive
the same frozen HEAD/tree and exact upstream base. Resolve every finding before counting compile or
test evidence.
