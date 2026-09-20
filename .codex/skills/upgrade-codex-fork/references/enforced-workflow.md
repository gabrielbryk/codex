# Enforced helper workflow

Read the source-owned operational contract completely before creating an evidence plan or running
source-bound gates:

`/home/gabe/workspace/personal/tooling/worklens/libs/fork-fleet/ops/codex/WORKFLOW.md`

That file owns the command/schema examples; do not duplicate them in this skill. The installed
`codex-upgrade-workflow` is linked to that source directory. Verify `--help` exposes `dossier`,
`template`, `next`, `gate --plan`, and `metrics`; missing commands require a source-owned helper
installation repair, not fallback to manual repeated discovery. Never edit the installed copy.

Use the existing upgrade report directory for dossiers, reviewed `workflow.json`, gate receipts,
bounded logs, and completion observations. The workflow plan is run-local evidence derived from
Fork Fleet's immutable plan, not another permanent patch registry or replacement candidate state.

Review candidate source and all intended edits before committing/finalizing. Source-bound gates
require a clean committed candidate; the helper no longer relies on manually comparing a staged
tree. Changing source, toolchain, declared external inputs, or gate settings invalidates the affected
receipt. Changing only a downstream gate or adding runtime observations preserves earlier proof.
Do not create a new run or rename a gate to evade broad/package attempt limits.

Compatibility probe scripts must exercise initialization/RPC/environment/writer behavior on isolated
test-owned processes and freshly emit the documented proof. A string grep is not a probe. Package
proof comes from the canonical builder's actual manifest and checksum verification. Source-bound
helper receipts do not replace registered `forkctl validate` evidence or silently narrow its policy.

Use [September lessons](2026-09-upgrade-lessons.md) only for the relevant historical failure. The
executable regression scenarios now live in Worklens
`libs/fork-fleet/tests/codex-upgrade-enforcement.test.ts` and exercise the real helper in disposable
repositories. These are deterministic workflow tests, not an LLM-based evaluation of skill following.
