---
name: upgrade-codex-fork
description: Safely check, prepare, ship, or improve Gabe's OpenAI Codex fork with Fork Fleet. Use for syncing to upstream or a rust-v release, validating the patch stack, preparing a candidate, building an exact-SHA package, authorized runtime cutover, or workflow improvement.
---

# Upgrade Codex Fork

Fork Fleet owns patch intent and candidates. Never rebase the active checkout, infer its stack from
a target-dependent merge base, or edit installed caches. This directory owns the skill; Worklens
`libs/fork-fleet` owns helpers.

## Choose One Mode

Announce the mode before commands:

- `check`: read-only preservation, target, patch, and distance evidence.
- `prepare`: an isolated, reviewed, validated candidate; the upgrade default.
- `ship`: requires explicit authority to publish, install, activate, deploy, or cut over.
- `improve`: evidence-gated workflow changes; it grants no history or runtime authority.

“Update” or “upgrade” alone never authorizes `ship`.

## Read Before Acting

Read each selected playbook completely. Read the candidate's root and applicable nested `AGENTS.md`
before mutation, delegation, validation, or build.

- Every run: [planning and reconciliation](references/planning-and-reconciliation.md)
- Validation/build: [validation](references/validation.md)
- Every `ship`: [shipping and cutover](references/shipping-and-cutover.md)
- Delegation/long work: [coordination and status](references/coordination-and-status.md)
- Reports/`improve`: [improvement and reporting](references/improvement-and-reporting.md)

Before mutation, start and retain one `codex-upgrade-report --mode <mode> --status running` run.

## Mandatory Gates

Follow this order; reopen phases only for new evidence.

1. **Preservation:** inventory every checkout, ref, stash, package, and runtime in a bounded
   `codex-upgrade-workflow preflight` artifact. Never discard or overwrite unrelated work.
2. **Target:** resolve the request to an immutable SHA. Stable requests override prerelease
   defaults. Reject plans or leaf decisions adjudicated against another SHA.
3. **Leaves:** audit every logical leaf's ownership, upstream replacement, supersession, shared
   surfaces, dependencies, tests, and retirement trigger. Record `apply`, `rework`, or `drop`; do not
   preserve upstream-fixed behavior or combine unrelated boundaries.
4. **Candidate:** mutate only the Fork Fleet candidate worktree. Join all writers, review/stage the
   intended tree, record HEAD plus index tree, then freeze it before a read-only heavy gate.
5. **Validation:** run deterministic checks, candidate-coherence compile, and each retained leaf's
   tests. Serialize heavy commands through `codex-upgrade-workflow gate`; reject results if
   HEAD/index changed. The complete Rust suite requires repository-mandated user approval.
6. **Final tail:** after semantic leaves and fix/fmt, generate and commit release metadata as the
   last isolated leaf with exact allowed paths. Revalidate target, diff, trailer, generated outputs,
   and final SHA; never hide semantic edits in this tail.
7. **Authorization:** `prepare` stops before publication. Only `ship` may move safety refs, publish,
   build the exact SHA, install, or request managed drain. Each destructive/runtime step needs
   explicit authority; verify the live generation after drain.

## Operating Rules

- Keep bounded logs; load only summaries and novel failures into context.
- Follow validation stop rules; capture lint once, batch fixes, and rerun once.
- Isolate runtime state. Never overlap writers with a gate; mutators run alone.
- Resume checkpoints only when SHA, invocation fingerprint, and artifacts match.
- Build from the final SHA; do not rebuild while draining.

## Finish

Update the same report with all SHAs, leaf decisions/supersession, gates, safety refs,
package/source markers, live generation, preserved work, and blockers. Report engineering
separately from passive rollout drain.
