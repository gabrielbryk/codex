# Validation Funnel

The purpose of validation is to find candidate regressions, not to make every host-dependent test
green. Move from cheap deterministic evidence to expensive broad evidence exactly once.

## Preflight

Before a long command:

1. Verify its executable, cwd, required environment, target triple, output path, and timeout.
2. Confirm every writer lane has joined and the candidate is still at the recorded HEAD. Review and
   stage every intended change; require no unstaged or untracked source files.
3. Isolate mutable `HOME`, `TMPDIR`, sockets, and test state when required, but reuse persistent
   Cargo, Bazel, V8/download, and package build caches keyed by toolchain/candidate.
4. Direct the complete raw log to a bounded artifact. Print only duration, totals, failed test
   names, and a short error excerpt to the terminal/model.
5. Do not run concurrent Cargo, Bazel, Clippy, or package builds against the same cache or host.

Use the installed repo-owned workflow helper for one bounded preflight JSON written outside the
managed checkout:

```bash
codex-upgrade-workflow preflight \
  --repo /mnt/wd-black/ClonedRepos/codex \
  --fork-id codex > <run-dir>/preflight.json
```

The result contains bounded repository/worktree/stash, package, runtime, registry, plan, patch
template, and stable source-plan/runtime fingerprints. Treat a missing, stale, or contradictory
result as a preflight failure. Preflight may inspect state, but must not mutate the candidate,
acquire the heavy lock, or start a gate.

The registered `upgrade-workflow-audit` action is documentation/repository coherence evidence only.
It checks the five canonical skill files, section-scoped contract markers, and static target
leakage. It does not inspect or report candidate finalization state, nor prove live registry, plan,
ownership, dependency, replacement-surface, size, or semantic-test adequacy. Fork Fleet owns those
runtime checks through fresh `forkctl registry validate`, `forkctl plan`, `forkctl prepare`, and
immutable-plan-aware `forkctl candidate finalize`; reviewers adjudicate semantics.

Both `upgrade-workflow-audit` and `upgrade-workflow-tests` are registered release evidence. The
first checks document shape; the second runs every contract-marker mutation test. Neither proves
that manual evidence or reviewer judgments are adequate. The registered test stage
uses an explicit tool path. It contains the pinned user Cargo location and system tool directories
and does not inherit an arbitrary `PATH` from the validator launcher.

Require preflight to prove operator/runtime registry freshness as described in the planning
playbook: a fresh `forkctl plan` must reload the canonical registry or fail closed, and its target
SHA, digest, patch count, and leaf contracts must match the validated registry view. Record a
bounded generated-CLI warning only when the resolved launcher uses an artifact whose source/build
provenance may be stale; never turn a registry mismatch into a warning.

## Heavy-gate placement

Before the first Cargo, Bazel, Clippy, workspace-test, or release-build gate, record the launcher's
actual cgroup and its effective `MemoryHigh`, `MemoryMax`, and swap limit. Heavy work must enter
`agent-workloads.slice`; inheriting `worklens.service`, `agent-control.slice`, or another bounded
control-plane service is a preflight failure even when the host still has free RAM. Do not infer
capacity from host-wide memory utilization because a child can be throttled by an ancestor cgroup.

On Huginn, launch coordinator-owned heavy gates through `agent-slice-exec`:

```bash
agent-slice-exec codex-upgrade-workflow gate \
  --run-dir <run-dir> \
  --cwd <candidate-worktree> \
  --timeout <seconds> \
  -- <executable> <literal-args...>
```

Treat registered `forkctl validate` results as authoritative only after verifying that Fork Fleet's
validation child enters the workload slice rather than inheriting the daemon's service cgroup. If
that placement contract is absent or fails, stop before the broad gate and repair the source-owned
Fork Fleet process launcher; a manually relocated gate is diagnostic evidence but does not update
candidate validation state.

The coordinator owns every heavy gate through this form:

```bash
agent-slice-exec codex-upgrade-workflow gate \
  --run-dir <run-dir> \
  --cwd <candidate-worktree> \
  --timeout <seconds> \
  -- <executable> <literal-args...>
```

The helper holds the host-wide lock, writes a capped private redacted log, and emits an invocation
`inputFingerprint`; it does not prove that candidate source stayed unchanged.

Before a read-only gate, record the reviewed HEAD/index tree, require no unstaged or untracked
source, and freeze all writers. Afterward verify the same snapshot and pair it with the invocation
fingerprint. No concurrent agent or command may edit the candidate.

From the candidate worktree, the minimal snapshot evidence is:

```bash
git rev-parse HEAD
git write-tree
git diff --quiet
git ls-files --others --exclude-standard
```

The last command prints nothing. A mutating fix/fmt/generator is the sole writer and requires a new
reviewed snapshot. If a read-only gate drifts, let it finish for diagnostics but do not count it;
rerun only the affected narrow gate after writers join.

## Gate order

1. **Deterministic repository gates**
   - generated schema and lockfile consistency when affected;
   - `git diff --check`;
   - argument-comment lint, captured completely in one invocation;
   - other registered static checks that do not execute the product.
2. **Candidate coherence compile**
   - run the registered `workspace-check` action after the frozen semantic review and before tests;
   - use the workspace action when the stack crosses crate boundaries, otherwise check every
     affected crate and its test targets;
   - treat non-exhaustive matches, stale tests/signatures, unused fork-only seams, or generated API
     drift as reconciliation failures, not mechanical test failures;
   - run a warning-denying scoped Clippy gate early enough to catch dead integration residue; the
     final `fix` step remains separate.
3. **Targeted behavior tests**
   - derive affected crates and maintained fork behaviors from the target diff and patch metadata;
   - run the specific crate/test commands required by Codex `AGENTS.md`;
   - retain integration/snapshot coverage for changed user-visible behavior.
   - update every retained leaf's implementation/test ledger with the exact command, candidate
     SHA, result, and artifact; fail the leaf when a required validation ID or focused behavior
     test has no current entry.
4. **One broad canary at most**
   - run only when required by reviewed validation or explicitly approved under `AGENTS.md`;
   - its job is to discover new failure classes, not establish universal host health.
5. **Final lint/fmt**
   - run scoped warning-denying `just fix -p <project>` for every changed Rust crate, using the
     repository-supported warning-denial environment; avoid an unscoped workspace fixer unless
     shared crates require it;
   - run final `just fmt` automatically;
   - do not rerun tests solely because fix/fmt ran.
6. **Release artifact**
   - use the canonical exact-SHA package builder once;
   - do not add a redundant Bazel release build unless the reviewed registry requires it.

Compile does not replace semantic proof. Lifecycle leaves require exact symbols, focused integration
test IDs and artifacts, and independent semantic review; a sequential same-process mock cannot prove
a restart, race, or generation handover.

Batch the first complete lint artifact once; do not repair truncated batches across repeated runs.

Use preflight `patchDecisionEvidence` and `patchDecisionTemplate` for facts, symbols, tests,
comparisons, and uncertainty. Reserve stronger review for ambiguous decisions or conflicts.

## Exact-target differential workspace gate

The registered `workspace-tests` action is differential. It runs every workspace-test command on
the candidate and accepts a successful command immediately without spending a second target run.
When a command fails, it binds the baseline through the manifest, candidate history, and
`FORK_FLEET_TARGET_SHA`, materializes that exact target in a disposable independent local clone,
then runs the identical command under the same environment policy there. It never adds a worktree
to the managed checkout or mutates shared Git worktree metadata. Use a fresh clone and mutable
state for each failed command; a setup failure blocks that comparison but does not prevent later
candidate commands from running.

Before freezing the target's pre-test snapshot, run offline Cargo lock normalization in that fresh
clone using full offline Cargo metadata; do not use a dependency-eliding metadata mode that leaves
the stale lock untouched. First prove HEAD, index, and worktree are pristine at the exact target.
Then accept only local
source-less workspace packages restamped from `0.0.0` to the target's `workspace.package` version.
Compute that exact expected textual transformation from the original lock and require the result to
match byte-for-byte; comments, whitespace, or key order changes are not normalization. Retain the
structural package-set check and reject any dependency, source, checksum, package-set, metadata,
index, or non-lockfile change. Record the prepared target snapshot and run both candidate and target
workspace commands with `--locked` so the identical argv cannot rewrite either lock. Require the
target's post-test snapshot to exactly equal its prepared target snapshot. Emit a redacted bounded
delta on any normalization or snapshot violation; every such diagnostic is a redacted bounded delta.

Both sides run with `INSTA_UPDATE=no`, but use private per-side `HOME`, `TMPDIR`,
`CARGO_TARGET_DIR`, `CODEX_HOME`, and XDG cache paths so candidate artifacts cannot make the target
pass or fail. The validator parses normalized nextest failure names from the complete command
output while emitting only explicit summaries, complete failure membership, and bounded useful
excerpts. Large command logs, Cargo outputs, and Git diagnostics are streamed to disk before they
are buffered, using disk-backed private validation storage rather than RAM-backed temporary
filesystems. A shared hard output cap terminates the process group on overflow or timeout and fails
closed, and every emitted excerpt is redacted. Redaction must cover Authorization
and Cookie headers through the end of their line, quoted JSON credentials, sensitive assignments,
and sensitive values supplied through the environment.

Require exactly one terminal summary, the expected nextest test-failure exit code for a failing
run, and exact equality between the summary's declared failure count and the unique parsed terminal
failure names, plus exact equality between its declared timed-out count and unique timeout names.
Recognize the explicit nextest terminal failure statuses `FAIL` and `FL+LK`; the latter is a failed
retry that also leaked. Parse the terminal timeout status `TMT` into its own category. Reject multiple
summaries, count mismatches, unknown terminal failure statuses, and format drift rather than guessing
at membership. A validator process timeout remains separate fail-closed evidence and never becomes
a parsed test name.

Candidate failures are accepted only when they are a subset of exact-target failures for that same
command, and candidate timeouts only when they are a subset of exact-target timeouts. Compare these
categories separately: a target failure cannot cover a candidate timeout or vice versa. Any
candidate-only failed test names or candidate-only timed-out test names block release. A target
setup error, process timeout,
truncation, or unparseable nonzero result also blocks; it is not evidence that the candidate is
equivalent. Apply this comparison independently to every workspace-test command, including the
separate V8 sandbox command, and always remove the temporary clone.

Record candidate HEAD, index tree, and untracked state before the gate and compare all three after
cleanup. Record the target HEAD, index tree, and untracked state before and after each comparison
too. Any drift invalidates the result even when every differential comparison would otherwise pass.
This name-set comparison does not prove semantic equivalence: the same test name can fail for a new
reason. Retained-leaf and changed-path targeted tests remain mandatory. The differential gate only
classifies broad exact-target failure membership; it does not waive a target failure that violates
a required retained-fork behavior.

## Failure classification

Classify each failure before changing code:

| Class | Evidence | Action |
| --- | --- | --- |
| Candidate regression | Repeatable narrow failure, absent at target/baseline, on a changed behavior path | Fix candidate and rerun the narrow gate |
| Upstream regression | Same narrow failure on the exact target in the same isolated environment | Record; block only if it violates a required fork behavior |
| Test-isolation defect | Product request is confused with probes, shared listeners/state, fixed ports, or leaked environment | Harden the owned fixture once and prove repeated narrow passes |
| Host/environment failure | Sandbox, localhost watcher, temp path, resource pressure, or timing; failure membership drifts | Record `validation-environment-failure`; do not patch product logic |
| Flake | Same unchanged narrow test alternates pass/fail without target-specific difference | Record retry evidence; do not restart the broad suite |

For a repeatable suspicious failure, compare the exact narrow test against the target/baseline once.
Do not run a second complete suite to obtain that comparison.

After a broad canary, compare its bounded failure membership with the prior recorded set:

```bash
codex-upgrade-workflow compare-failures \
  --before <previous-failure-set> \
  --after <current-failure-set> \
  [--repeat <additional-recorded-set>] > <run-dir>/failure-comparison.json
```

The comparison must normalize test names and classify added, removed, and unchanged failures. If
material membership drift is detected, record `validation-environment-failure`, stop broad reruns,
and continue only with narrow reproductions that can distinguish candidate behavior from host
instability. The helper must not turn drift into a retry loop or declare a regression without the
required target/baseline evidence.

## Hard stop rules

- Do not start a read-only gate while any agent retains candidate write authority or while the
  candidate has unreviewed, unstaged, or untracked source changes.
- Do not dispatch or follow up candidate-writing work until the current read-only gate result and
  post-gate snapshot are captured.
- Never run more than one broad canary for an unchanged candidate SHA.
- A broad rerun requires both a candidate change relevant to a deterministic prior failure and a
  reviewed reason that targeted tests cannot cover it.
- If two existing broad results have materially different failure membership, classify the broad
  environment as unstable immediately. Continue only with narrow reproductions.
- If targeted maintained-path tests pass and remaining failures are classified upstream,
  environmental, or flaky, they are not cutover blockers.
- Do not rewrite production code to satisfy a host probe, nested sandbox limitation, fixed socket,
  or timing-sensitive fixture.
- Do not keep a large run alive merely to increase the pass count after its remaining failures are
  already classified.
- Never restart a completed validation phase while the package is only waiting for natural drain.
- Do not begin targeted tests while the manifest names an old target, the candidate contains
  unexplained empty commits, or a surviving lifecycle producer has no production consumer.

## Compact evidence

For each gate record: exact command, candidate SHA, duration, pass/fail/skip counts, failed names,
classification, baseline comparison when performed, and artifact path. Never paste all passing
test lines or an entire JSON status payload into agent context. Poll long-running commands using a
small progress summary, not an ever-growing tail.

Before accepting validation, reconcile those gate records against the per-leaf implementation/test
ledger. Prove upstream and intentional-drop leaves contributed no replay or replacement commit,
and prove every applied or reworked leaf has current target-bound implementation and test evidence.
For behavior leaves, an independent reviewer records the producer -> owner -> consumer -> completion
trace, exact symbols, and focused end-to-end test IDs/artifacts in SHA-bound evidence. Its existence
and semantic adequacy are manual review obligations, not claims made by the prose audit or Fork Fleet.
