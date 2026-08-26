---
name: upgrade-codex-fork
description: Safely check, prepare, ship, or improve Gabe's local OpenAI Codex fork upgrade workflow with Fork Fleet. Use for syncing /mnt/wd-black/ClonedRepos/codex to upstream/main or a rust-v release, validating the logical patch stack, preparing an isolated candidate, building an exact-SHA canonical package, performing an explicitly authorized runtime cutover, or improving this process from recorded friction.
---

# Upgrade Codex Fork

Use Fork Fleet as the authority for patch intent and isolated candidates. Never rebase the active
Codex checkout in place, infer the stack from a target-dependent merge base, or edit installed skill
caches. This repo-local directory is the durable skill source. Fork Fleet owns the external
installer, package helper, and report writer under `libs/fork-fleet` in the Worklens repository.

## Select One Mode

Announce the mode and why before commands:

- `check`: read-only preservation, target, patch, and release-distance evidence.
- `prepare`: `check` plus one isolated reviewed candidate and required validation. This is the
  default when an upgrade request does not authorize publication or cutover.
- `ship`: only after explicit publish, install, activate, deploy, or cutover authority.
- `improve`: post-run, evidence-gated changes to source-owned workflow or documentation.

“Update” or “upgrade” alone does not authorize `ship`. Improving the workflow does not authorize
changing Codex history or runtime state.

## Read the Relevant Playbooks

Read each selected reference completely before acting:

- Every `check`, `prepare`, or `ship`: [planning and reconciliation](references/planning-and-reconciliation.md)
- Every validation or build: [validation funnel](references/validation.md)
- Every `ship`: [shipping and cutover](references/shipping-and-cutover.md)
- When delegation is authorized or work is long-running: [coordination and status](references/coordination-and-status.md)
- Every run report and every `improve`: [improvement and reporting](references/improvement-and-reporting.md)

Repository `AGENTS.md` remains authoritative for Codex code style and test ordering.

## Start a Durable Run

Before mutation, create one report and retain its run ID:

```bash
codex-upgrade-report \
  --mode <check|prepare|ship|improve> \
  --status running \
  --repo /mnt/wd-black/ClonedRepos/codex
```

Update that same report at terminal boundaries. Keep raw logs in bounded artifacts, not in the
report or model context.

## Execution Contract

Use this order and do not reopen an earlier phase without new evidence:

1. Preserve and inventory every checkout, ref, stash, installed package, and active runtime.
2. Resolve and verify the exact requested target; explicit stable requests override prerelease
   registry defaults.
3. Decide `apply`, `rework`, or `drop` for every logical patch using behavior and test evidence.
4. Prepare and resolve conflicts only in the Fork Fleet candidate worktree.
5. Run deterministic gates, targeted behavior tests, at most one broad canary, then final fix/fmt.
6. In `ship`, publish with safety refs and exact leases, build the exact SHA once, and request the
   managed natural-drain rollout.
7. Separate “engineering complete” from passive drain waiting; verify the live generation after
   the controller finishes.

## Non-Negotiable Efficiency Rules

- Do not run the complete Rust suite without the repository-required user approval.
- Never repeat a broad suite merely because its failure set changed. Follow the validation
  classification and stop rules in the linked playbook.
- Capture complete lint output once, batch all mechanical repairs, then rerun the lint once.
- Reuse persistent Cargo/Bazel/download caches while isolating mutable runtime state.
- Serialize heavy Rust/Bazel/package builds; parallelize only independent analysis and mechanical
  edits with disjoint ownership.
- Load only summaries and novel failures into agent context. Do not stream passing-test output.
- The canonical exact-SHA package build is the release build unless reviewed registry validation
  explicitly requires another build system.
- Never rebuild or retest while a valid package is merely waiting for natural drain.

## Finish

Report mode/run path, source/base/target/candidate SHAs, patch decisions, compact gate outcomes,
safety refs, package/source markers, live generation state, preserved dirty worktrees, and only real
remaining blockers. For a waiting rollout, report the exact drain state and no engineering ETA.
