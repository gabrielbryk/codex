# Fork patch manifest

The `gabe/fork` branch is a downstream patch stack on an immutable upstream
`rust-v*` release tag. Fork Fleet owns the logical patch mapping; this file is
the human review surface for intent, supersession decisions, conflict watches,
and release validation.

Current upstream base: **`rust-v0.149.1`**
(`ff29a44391deccde0aba0f8390337d7f3c319ea4`). The prior source boundary was
**`rust-v0.148.0-alpha.12`**
(`4af6dc74a035b8f177a1588abddef4f6c605464b`).

The 2026-08-24 upgrade reviewed all 49 source commits as 21 logical patches.
No maintained logical patch was fully superseded by `0.149.1`. Seven patches remain clean
downstream applications; fourteen were reworked because upstream now provides
an adjacent primitive or changed the integration seam. The old alpha-only
format/version commits are not maintained behavior: stable normalization is
regenerated from the `0.149.1` tag's dependency graph.

## Maintained logical patches

| Patch | Category | `0.149.1` decision | Current upstream assessment |
|---|---|---|---|
| `remote-turn-resilience` | upstreamable | rework | Upstream now echoes and queues `clientUserMessageId`; the fork still supplies bounded request replay and server-side idempotency across ambiguous reconnects. |
| `claude-history-compact-import` | upstreamable | apply | No upstream equivalent for importing Claude history from the latest compact boundary. |
| `uds-rendezvous-hardening` | local environment | apply | Secure sticky rendezvous-directory support remains downstream-only. |
| `disable-stock-fork-updater` | local environment | apply | Required so stock update logic cannot replace the packaged fork runtime. |
| `mcp-oauth-recovery-hardening` | defensive | rework | Upstream now provides issuer binding/tracking, locked atomic fallback-token writes, and `invalid_grant` classification. The fork still supplies stale compare-delete, refresh-token preservation, the host/executor collision guard, reactive live-401 retry, and Slack envelope normalization. |
| `orphan-tool-output-persistence` | upstreamable | rework | Upstream repairs orphan outputs in request snapshots but does not persist the repair to durable rollout history. |
| `agent-wait-gone-failure` | upstreamable | apply | Upstream returns immediate `NotFound`; the fork additionally preserves an explicit model-visible failure contract. |
| `scoped-command-runtime` | fork feature | apply | Per-thread command and MCP child attribution remains downstream-only. |
| `idle-thread-residency` | fork feature | rework | Retains the five-minute idle bound and explicit-versus-implicit subscription semantics on the current listener lifecycle. |
| `tui-transport-recovery` | upstreamable | rework | Retains non-fatal app-server request failures and recovery messaging on the current TUI event flow. |
| `tui-steer-overload-retry` | upstreamable | rework | Retains bounded steering retry and composer-input requeue around upstream overload handling. |
| `alternate-codex-home-isolation` | local environment | apply | Required for independent Primary and Uprising homes. |
| `contained-pty-spawning` | upstreamable | rework | Retains supervised PTY containment while using current upstream process primitives. |
| `external-status-line` | fork feature | rework | Upstream expanded native status configuration; the fork retains only its bounded external command runner, parser, wire format, and config seam. |
| `alpha-workspace-normalization` | release chore | rework | Alpha-specific edits are superseded; the replacement is stable `0.149.1` lock, schema, and snapshot normalization from the tag lockfile. |
| `live-app-server-handover` | fork feature | rework | No upstream equivalent. The drain classifier is now exhaustive over `ClientRequest` and explicitly reviews `0.149.1` queue, project, and thread-revert RPCs. |
| `agent-request-observability` | upstreamable | rework | Retains thread/turn correlation and per-request latency/token outcomes without duplicating upstream telemetry. |
| `tui-thread-ownership-isolation` | defensive | rework | Retains isolation from unrelated broadcast threads on the current TUI ownership model. |
| `tui-reconnect-correctness` | fork feature | rework | Retains replay gating, authoritative reattachment, event reconciliation, and streamed-item identity on current upstream TUI transitions. |
| `fork-patch-manifest-maintenance` | release chore | rework | This manifest and internal handover ledger are regenerated for the stable release. |
| `tmp-inode-test-hygiene` | defensive | apply | The fork's known temporary-directory leak sites remain present upstream. |

## Important integration invariants

- `turn/start` replay is allowed only with a non-empty
  `clientUserMessageId`, is bounded, and is not released until owned threads
  reattach after reconnect.
- Drain admission must be exhaustive over `ClientRequest`. New request variants
  must choose both named policy flags: whether they start new work and whether
  they can create an implicit attachment. Existing-turn continuation operations
  such as steering, interrupting, writing to a command/process, and stopping
  realtime remain available while draining.
- Idle auto-attached threads do not count as retaining subscribers; explicit
  clients do. Active threads never unload.
- The stable router owns generation selection. TUI reconnect logic reattaches
  owned threads but does not independently select or promote generations.
- OAuth recovery uses the shared upstream token-store locks. The fork's
  compare-and-delete function must remain behaviorally aligned with the
  upstream unconditional delete path except for its intentional stale-token
  and executor-ownership guards.
- External status-line execution stays bounded and contained. Do not duplicate
  upstream native status rendering or use raw `unicode_width` for TUI cell
  arithmetic.
- App-server protocol changes require stable and experimental schema fixture
  regeneration. The root `write-app-server-schema` recipe invokes
  `app-server-protocol/scripts/write_schema_fixtures.py`.
- Stable release normalization starts from the tag's `Cargo.lock` dependency
  selections. Never use an unconstrained lockfile regeneration to manufacture
  a clean diff.

## Superseded historical patches

No maintained logical patch was fully superseded by `0.149.1`. The following
entries describe historical components or release artifacts that are no longer
independent downstream behavior.

- The old monolithic MCP OAuth patch was superseded by upstream's modular OAuth
  implementation. In `0.149.1`, upstream also supersedes its issuer
  binding/tracking, locked and atomic fallback-token writes, and
  `invalid_grant` classification components. The downstream patch retains only
  stale compare-delete, refresh-token preservation, the host/executor collision
  guard, reactive live-401 retry, and Slack envelope normalization.
- MCP connection-manager reaping was superseded by upstream connection reuse;
  replaying it would tear down active connections.
- The standalone TUI `turn/start` error patch was upstreamed. Its fork-specific
  transport/replay residue lives in the remote resilience and TUI recovery
  patches.
- Intermediate alpha.9 and alpha.12 lock/version restamps and standalone
  rustfmt commits are release artifacts, not maintained behavior. Regenerate
  one final stable normalization per release.

## Per-upgrade verification

1. Verify the recorded source base and target tag by full SHA; do not infer the
   boundary from ancestry between release tags.
2. Review every logical patch for full, partial, or absent upstream coverage.
3. Replay into a fresh Fork Fleet worktree and resolve conflicts by behavior,
   never wholesale `ours` or `theirs` selection.
4. Regenerate config/app-server schemas and review every TUI snapshot change.
5. Validate locked Cargo metadata, Bazel lock parity, argument comments, scoped
   crate tests, the complete `just test` suite, and the release build.
6. Attribute every file in `git diff rust-v0.149.1..HEAD` to a logical patch or
   stable release normalization.
7. Publish a dated safety ref before updating `gabe/fork`, use an exact
   `--force-with-lease`, create the annotated `fork/0.149.1` tag, package the
   exact published SHA, and use natural drain for the live rollout.
