# Shipping and Cutover

Use only with explicit `ship` authority and an exact clean candidate that matches its immutable
plan. Recheck remote state immediately before publication.

Resume shipping from the persisted phase ledger, not from memory. Reuse a completed publish,
validation, or build checkpoint only when the recorded input fingerprint still matches the exact
candidate SHA, plan, package markers, and relevant remote/runtime state. Otherwise invalidate that
checkpoint and rerun only the affected phase; do not repeat valid gates after a session resume.

## Publish safely

1. Create a dated local safety ref for the old fork tip.
2. Publish and verify a remote safety ref before rewriting the maintained branch.
3. Push from the candidate worktree when the maintained checkout does not contain candidate
   objects.
4. Update the maintained branch only with exact `--force-with-lease=<old-tip>`.
5. Never use unleased force push and never disturb unrelated worktrees or stashes.

Run the registered release validation once against the exact candidate SHA:

```bash
forkctl validate <candidate-id> --stage release
```

Do not repeat prior broad suites during shipping unless the candidate SHA changed in a way that
invalidates them.

Record the model/delegate and bounded evidence for each shipping phase. Cheap subagents may collect
preflight evidence or perform mechanical, disjoint checks; keep publication, lease interpretation,
cutover authorization, and ambiguous release decisions with the primary agent or an explicitly
authorized stronger reviewer.

## Preflight and build once

Choose the lifecycle path before building. The commands below using `--rollout`, drain RPCs, and
router generations apply only to a candidate that implements the custom natural-drain contract.
The September 0.153.4 candidate dropped that contract; upstream-native reconnect does not restore
it. For native-server ownership, inspect the host source repository's lifecycle instructions and
plan migration there. Never resurrect unsafe fork lifecycle patches simply to satisfy an old helper.

Probe the exact candidate executable in isolated state: initialization capabilities, required RPCs,
generation environment handling, and truthful writer release. Grepping strings, caches, or package
directories is not behavioral proof. Failure blocks activation, not permission for a hard cutover.

Inspect active turns and rollout state before building:

```bash
/home/gabe/.local/bin/codex-app-server-active-turns
/home/gabe/.local/bin/codex-app-server-rollout status
```

Verify the package wrapper, canonical builder, target triple, candidate cleanliness, persistent
cache, rollout controller, expected markers, and destination before starting the expensive build.
The source-owned wrapper exports `CODEX_REPO_ROOT` from `--repo`; callers must not need an ambient
variable for the canonical builder.

Build the exact SHA once as the eligible package gate, without activation. Supply the custom
protocol markers only for a candidate that implements that contract:

```bash
codex-use-local-build \
  --repo <candidate-worktree> \
  --sha <candidate-sha> \
  --expect 'server/drain/start' \
  --expect 'threadWriterLeaseV1' \
  --expect '<feature-marker>'
```

The helper uses Codex's canonical optimized package builder, immutable provenance variables,
persistent caches, pre/post SHA and cleanliness checks, package metadata/checksums, and atomic
package publication. Without `--activate` or `--rollout`, it leaves the current runtime selection
unchanged. It never restarts or signals an active process. The exact-SHA package build is
authoritative; a second release build is not extra proof unless registered validation requires it.

After behavioral compatibility and separate activation authority, request the already-built package
through the host-owned controller. Do not rerun the builder merely to request rollout. Record
publication/artifact/server/client observations separately in the workflow plan; `next` validates
their identity and completeness, but it does not execute or authorize external actions.

## Natural drain

Use natural drain when the candidate exposes `serverIdentityV1`, `serverDrainV1`, and
`threadWriterLeaseV1` at protocol revision 1. Existing router connections remain pinned while new
connections use the replacement. The controller retires the old server only after its loaded-thread
and connection counts reach zero.

The one-time legacy canonical-socket migration still requires zero active turns. Queue it normally
and record `router-bootstrap-deferred`; never bypass the guard.

After the package is valid and the rollout request is durable, engineering is complete. A draining
generation is passive waiting, not a reason to rebuild, rerun tests, edit code, or quote a bounded
engineering ETA.

Publish machine-readable status with separate `engineeringState: "complete"` and
`rolloutState: "draining"` (or the precise allowed state). Link the latest bounded runtime/preflight
checkpoint containing desired/current SHA, generation identity, connection and loaded-thread counts,
and pending action.
Natural drain remains the safe default: do not kill clients, routers, or active turns to accelerate
rollout, and do not treat deferred handover or draining as a failed engineering phase.

Reconcile and verify with:

```bash
/home/gabe/.local/bin/codex-app-server-rollout apply-pending
/home/gabe/.local/bin/codex-app-server-rollout status
```

Verify desired package/source SHA, CLI version, managed daemon version, app-server version, router
revision, active generation identity/socket, canonical router socket, pending rollout, connection
counts, reconciliation state, and old-generation retirement. CLI version alone does not prove
server cutover. Treat `deferred_router_handover`, `deferred_generation_rollout`, and draining as safe
lifecycle states; never kill clients or routers without separate explicit authority.

Only after publication succeeds, propose registry mappings with
`forkctl registry update --proposal`. Archive the candidate only after package, safety refs,
maintained branch, and registry evidence are durable.

## Completion across profiles and generations

Track source publication, selected package, running app-server, and running TUI clients separately.
Record exact source/package identity, not only semver: custom builds can share the same version.
For every explicitly in-scope Codex home, resolve socket ownership, executable, process ancestry,
service owner, active turns, loaded threads, and old clients. One primary-home check cannot prove
another profile migrated. Use generic host-owned service templates, not account-specific fork code.

An already-running TUI does not upgrade when `current` changes. Preserve active turns and drafts;
client exit/relaunch or disruptive migration needs appropriate authority. Do not kill a shared scope
to retire one daemon. Verify exact old-process retirement and recovery after any authorized cutover.

For reconnect acceptance, use an isolated persisted thread and test-owned server: wait for completed
bootstrap, submit one known message, retain a separate unsent draft, interrupt only the test server,
restart it on the same socket, and assert reattachment plus draft retention and exactly one recorded
message/turn start. An empty-thread or transport-only smoke is insufficient. Do not use production
credentials or disrupt live clients for this test; use fixture-ready signals rather than sleep guesses.
