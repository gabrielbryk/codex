# Shipping and Cutover

Use only with explicit `ship` authority and an exact clean candidate that matches its immutable
plan. Recheck remote state immediately before publication.

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

## Preflight and build once

Inspect active turns and rollout state before building:

```bash
/home/gabe/.local/bin/codex-app-server-active-turns
/home/gabe/.local/bin/codex-app-server-rollout status
```

Verify the package wrapper, canonical builder, target triple, candidate cleanliness, persistent
cache, rollout controller, expected markers, and destination before starting the expensive build.
The source-owned wrapper exports `CODEX_REPO_ROOT` from `--repo`; callers must not need an ambient
variable for the canonical builder.

Build and request natural drain in one exact-SHA operation from the candidate worktree:

```bash
codex-use-local-build \
  --repo <candidate-worktree> \
  --sha <candidate-sha> \
  --rollout \
  --expect 'server/drain/start' \
  --expect 'threadWriterLeaseV1' \
  --expect '<feature-marker>'
```

The helper uses Codex's canonical optimized package builder, immutable provenance variables,
persistent caches, pre/post SHA and cleanliness checks, package metadata/checksums, and atomic
promotion. It never restarts or signals an active process. The exact-SHA package build is
authoritative; a second release build is not extra proof unless registered validation requires it.

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
