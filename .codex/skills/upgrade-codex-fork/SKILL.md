---
name: upgrade-codex-fork
description: Check, prepare, ship, or improve Gabe's Codex fork using Fork Fleet, incremental Git evidence, source-bound validation receipts, and explicit runtime authority.
---

# Upgrade Codex Fork

This skill coordinates; deterministic helpers collect evidence and enforce gate eligibility. Fork
Fleet owns registry intent and candidates. This directory owns Codex-specific instructions; Worklens
`libs/fork-fleet/ops/codex` owns helpers. Never edit installed caches or rebase the maintained checkout.

## Mode and authority

Announce one mode: `check` (read-only), `prepare` (isolated candidate and approved validation),
`ship` (only explicitly authorized publication/install/cutover), or `improve` (source-owned process
changes). “Upgrade” alone means prepare, not ship. No mode raises registry authority or permits
interrupting active work without explicit authorization. “No PR” means direct repository commits.

Before acting, read the complete applicable root/more-specific `AGENTS.md`. Read playbooks only
when entering their phase, completely once selected:

- Planning: [planning and reconciliation](references/planning-and-reconciliation.md).
- Patch evidence: [release-history dossier](references/release-history.md).
- Prepare/ship execution: [enforced helper workflow](references/enforced-workflow.md).
- Tests/builds: [validation funnel](references/validation.md).
- Shipping: [shipping and cutover](references/shipping-and-cutover.md).
- Delegation/long-running work: [coordination and status](references/coordination-and-status.md).
- Reports/improve: [improvement and reporting](references/improvement-and-reporting.md).

Historical postmortems are reference material for a matching failure or improve task, not mandatory
context for every upgrade. Start from current receipts, not full transcripts.

## One run, one next action

Before mutation create a durable report with `codex-upgrade-report --mode <mode> --status running
--repo <source-repo>`; retain its run directory across resumes. Then:

1. Preserve and inventory using one bounded `codex-upgrade-workflow preflight`. Resolve the explicit
   source base, pinned target, publication destination/authority, and installed lifecycle contract.
2. Generate `dossier` from the immutable Fork Fleet plan. On retarget use `--previous`; retain valid
   family evidence, not old-SHA test claims. Generate an unapproved `template` for the evidence plan.
3. Review every family's invariant/test mapping and apply/rework/drop decision. Rework requires
   target-native implementation stages. Dropped host APIs require a host-owned migration plan.
4. Run `codex-upgrade-workflow next --run-dir <run> --plan <workflow.json>`. Address its precise
   blocker or execute its eligible action. Candidate mutations still use Fork Fleet; never raw
   `git cherry-pick --continue`. Record the actual coordinator SHA after continue/finalize.
5. Run eligible read-only gates with `gate --plan <workflow.json> --gate-id <id>`. Require committed,
   clean source and no writers. Successful matching receipts are reused automatically. Ordinary
   unbound gates are for sole-writer formatting/fixing and never count as reusable phase proof.
6. Keep validation, behavioral compatibility, and one canonical package build separate. Inspect
   actual gate expansion/host prerequisites before any expensive command. Full Rust suites require
   repo-mandated approval; never repeat broad suites for drifting environmental failures.
7. In ship, verify publication, selected artifact, every scoped live server, and existing clients
   separately. Refresh stale runtime proof without rebuilding. Preserve drafts and active turns.

The helper is an evidence boundary, not permission to execute external actions. Its JSON checks
cannot establish the truth of an arbitrary probe script; review probe behavior and source ownership.
Missing helpers or receipts are blockers, not reasons to substitute memory or disable enforcement.

## Keep the coordinator small

Use scripts for inventory, diffs, logs, and mechanical collection. Delegate only a bounded unresolved
semantic question or genuinely useful disjoint implementation; include paths, authority, and a small
evidence packet, not full histories. No blanket agent fan-out. Serialize heavy gates and join writers.

Finish with commit/run/artifact links, compact results, separate engineering/runtime states, and
real remaining blockers. Include `metrics` counts for executed/reused/invalidated/duplicate gates,
review reuse, and recorded abandoned work. Token cost is unknown without authoritative request data;
never add cached input to total input or sum cumulative transcript counters.
