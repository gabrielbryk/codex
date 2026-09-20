# Validation funnel

Find candidate regressions with the smallest authoritative gate. Read the
[enforced helper workflow](enforced-workflow.md) before execution; its source-owned operational
contract defines plan fields, committed-source receipts, automatic reuse, and phase prerequisites.

## Before expensive commands

Inspect the actual registered command expansion, executable, cwd, timeout, toolchain, target,
profile/features, external inputs, and host requirements. Upstream's six-platform Bazel release
matrix is not a local Linux package gate when it needs remote execution or Windows SDK libraries.
Do not invent zlib/BLAKE3 patches to satisfy an irrelevant gate. A change to registered validation
requires a reviewed plan; helper receipts do not silently replace `forkctl validate` evidence.

Isolate mutable homes, sockets, temp paths, and locks while reusing persistent dependency/build
caches. Never put build caches or cloned repositories on tmpfs. Declare external gate inputs and
environment so receipt reuse is sound; ignored files and remote services are not automatically
fingerprinted. Tool versions are checked from the actual gate cwd.

Join every writer, review and commit intended source, and seal the candidate through Fork Fleet.
Require no staged, unstaged, or untracked files. Source-bound gates enforce this under the existing
heavy lock and verify source/plan/tool inputs afterward. A pass whose inputs changed is diagnostic
only. Do not edit even disjoint candidate files while a gate runs.

## Gate order and reuse

1. Deterministic checks: schemas/lockfiles when affected, `git diff --check`, complete argument-comment
   lint, and other required static checks. Batch all mechanical findings from one bounded artifact.
2. Targeted tests: derive crate/behavior coverage from the dossier and retained invariants. Use the
   repo's `just test` contract and a focused test/debug profile, not release LTO for iteration.
3. At most one approved broad canary: discover new failure classes, not universal host health. The
   complete Rust suite requires repository-mandated user approval.
4. Required fix/fmt: a sole-writer ordinary `gate` with no reusable phase receipt. Review, commit,
   and finalize its changes. Follow repository test-order rules; do not rerun tests solely because
   fix/fmt ran, and never relabel pre-format proof as a pass on a different SHA. If source changed,
   retain the old proof with provenance and resolve required exact-SHA validation explicitly.
5. Behavioral compatibility: actual isolated protocol/env/writer probes, with freshly written,
   source-bound JSON proof. Markers in binary strings or build caches are insufficient.
6. Canonical package once: use the exact-SHA builder and verify its actual manifest/checksums.
   No second Bazel release build unless the reviewed registry explicitly requires it.

Use `next` to select an eligible gate, then `gate --plan <workflow.json> --gate-id <id>` with its exact
declared command. Matching successful receipts are reused. Source or relevant input changes
invalidate proof; session resumption, passive drain, and downstream-only plan edits do not.
Broad/package attempts cannot be repeated on unchanged source within the run. Do not rename gates
or start another run to evade that bound. A failed narrow retry requires a classification supplied
with `--rerun-reason`, not merely “try again.”

## Failure classification

| Class | Evidence | Action |
| --- | --- | --- |
| Candidate regression | Repeatable narrow failure absent on exact target in equivalent environment | Fix candidate; rerun affected narrow gate |
| Upstream regression | Same narrow failure on target | Record; block only if required behavior is violated |
| Test-isolation defect | Probe traffic, shared locks/state, leaked environment, fixed ports | Fix the owned fixture and prove narrow behavior |
| Host/environment | Sandbox/resource/timing issue or drifting failure membership | Record; do not patch product logic |
| Flake | Unchanged narrow test alternates outcomes without target-specific difference | Record retry evidence; do not restart broad suite |

Compare a suspicious narrow failure with the exact target once, not with another complete suite.
Use `compare-failures --before <artifact> --after <artifact>` to classify bounded broad failure sets.
If membership drifts, stop broad retries immediately. An unchanged membership result alone does
not authorize another broad run. Do not kill an in-flight Rust build by PID; let its bounded command
finish for diagnostic/cache value, then resolve the actual blocker.

Fixture checks: isolate terminal/runtime environment and locks; tolerate unrelated localhost
handshakes without terminating a mock server; surface mock-task failures promptly. Split long
reconnect cases under runner deadlines and avoid exact wall-clock equality assertions. Do not
accept new snapshots just to accommodate inherited `TERM`/`TMUX`. Test from reviewed source so
unrelated dirty host-tool changes cannot contaminate results.

## Evidence at handoff

Link receipts and bounded logs; report duration, totals, failed names, classification, and baseline
comparison when needed. Do not stream passing tests or repeated JSON into context. Preserve
partial/unfinished attempts in metrics. After a valid package, waiting for authorized rollout or
fresh runtime proof never justifies rebuilding or repeating validation.
