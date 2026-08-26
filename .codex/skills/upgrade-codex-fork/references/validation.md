# Validation Funnel

The purpose of validation is to find candidate regressions, not to make every host-dependent test
green. Move from cheap deterministic evidence to expensive broad evidence exactly once.

## Preflight

Before a long command:

1. Verify its executable, cwd, required environment, target triple, output path, and timeout.
2. Confirm the candidate is clean and still at the recorded SHA.
3. Isolate mutable `HOME`, `TMPDIR`, sockets, and test state when required, but reuse persistent
   Cargo, Bazel, V8/download, and package build caches keyed by toolchain/candidate.
4. Direct the complete raw log to a bounded artifact. Print only duration, totals, failed test
   names, and a short error excerpt to the terminal/model.
5. Do not run concurrent Cargo, Bazel, Clippy, or package builds against the same cache or host.

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

## Hard stop rules

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
