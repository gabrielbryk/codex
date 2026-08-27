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

The coordinator owns every heavy gate (Cargo, Bazel, Clippy, full test, package build, or other
resource-intensive command) through this form:

```bash
codex-upgrade-workflow gate \
  --run-dir <run-dir> \
  --cwd <candidate-worktree> \
  --timeout <seconds> \
  -- <executable> <literal-args...>
```

The helper holds the host-wide lock through completion and result capture, writes a capped private
redacted log, and emits a compact outcome plus `inputFingerprint`. That fingerprint covers the gate
invocation (cwd, argv, selected environment, timeout, and log limit); it does **not** prove that the
candidate source stayed unchanged.

Before a read-only gate, record the candidate HEAD and staged index tree after reviewing all edits.
Require no unstaged or untracked source files, then freeze the candidate until the result is
captured. After the gate, verify the same HEAD/index tree and the same absence of unstaged or
untracked source files. Pair this candidate snapshot with the helper's invocation fingerprint in the
checkpoint. A delegated agent may prepare the command or inspect its bounded artifact, but may not
start a competing heavy gate. Read-only analysis may continue; no agent or command may edit any
candidate file, even one outside the crate under test.

From the candidate worktree, the minimal snapshot evidence is:

```bash
git rev-parse HEAD
git write-tree
git diff --quiet
git ls-files --others --exclude-standard
```

The last command must print nothing. Record the first two values, run the final two checks again
after the gate, and compare the HEAD/tree values exactly. Do not use a blind `git add -A` to make
these checks pass; review every intended file before staging it.

For a formatter, fixer, generator, or other intentionally mutating gate, the command must be the
candidate's sole writer. Its pre-command snapshot is invalidated by design: review and stage its
output and record a new frozen snapshot before accepting any later read-only gate.

If source changes during a read-only gate, let the command finish rather than killing a Rust build,
record the candidate-input drift in the gate note, and do not count its pass or failure as
authoritative. Once all writers have joined, rerun only the affected narrow gate on the frozen
candidate. Do not restart a broad canary merely because its input drifted.

## Gate order

1. **Deterministic repository gates**
   - generated schema and lockfile consistency when affected;
   - `git diff --check`;
   - argument-comment lint, captured completely in one invocation;
   - other registered static checks that do not execute the product.
2. **Targeted behavior tests**
   - derive affected crates and maintained fork behaviors from the target diff and patch metadata;
   - run the specific crate/test commands required by Codex `AGENTS.md`;
   - retain integration/snapshot coverage for changed user-visible behavior.
3. **One broad canary at most**
   - run only when required by reviewed validation or explicitly approved under `AGENTS.md`;
   - its job is to discover new failure classes, not establish universal host health.
4. **Final lint/fmt**
   - run scoped `just fix -p <project>` or the shared equivalent required by `AGENTS.md`;
   - run final `just fmt` automatically;
   - do not rerun tests solely because fix/fmt ran.
5. **Release artifact**
   - use the canonical exact-SHA package builder once;
   - do not add a redundant Bazel release build unless the reviewed registry requires it.

Batch every mechanical lint finding from the first complete artifact. When delegation is authorized,
give that complete set to one Luna agent with disjoint file ownership; do not discover and repair
one truncated batch per full lint invocation.

For patch reconciliation, use `patchDecisionEvidence` and `patchDecisionTemplate` from the preflight
artifact. Have Luna fill observable facts, exact file/symbol/test references, behavior comparisons,
and uncertainty. The primary agent (or a deliberately stronger reviewer) adjudicates only ambiguous
`apply`/`rework`/`drop` cases, partial supersession, or conflicting evidence. Do not spend a stronger
model on mechanically complete evidence or deterministic lint repairs.

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

## Compact evidence

For each gate record: exact command, candidate SHA, duration, pass/fail/skip counts, failed names,
classification, baseline comparison when performed, and artifact path. Never paste all passing
test lines or an entire JSON status payload into agent context. Poll long-running commands using a
small progress summary, not an ever-growing tail.
