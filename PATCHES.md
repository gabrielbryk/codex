# Fork patch manifest

`gabe/fork` is a downstream patch stack on immutable upstream Codex release
`rust-v0.153.4` (`3d2ee51ca2d5db578f328aa75e20aa22c0197c9a`). Fork Fleet owns the
machine-readable contract; this file is the human blast-radius ledger.

- Target: `rust-v0.153.4` (`3d2ee51ca2d5db578f328aa75e20aa22c0197c9a`)
- Source head: `1798ad4dd7028dc37fb264b648a084420a0251bf`
- Source base: `rust-v0.150.1` (`90854393966b21e9ebfd21b122334eb09a20c93d`)
- Immutable plan: `9a41cb2a-68c6-4599-b8c6-0e01cd0dc64f`
- Candidate: `d3938c53-4f89-4e41-80db-519f216f5c87`
- Model: 40 leaves — 17 rework, 23 drop/upstream/quarantine

Each retained leaf below names one failure boundary, its production owner,
focused proof, compatibility/shared impact, and exact retirement condition.
The Fork Fleet plan contains the complete file lists and source-commit mapping.

## Retained-leaf blast radius

### `runtime-alternate-home-isolation` — rework

- Owner: `codex-rs/config/src/loader/mod.rs`; alternate-home load decision only.
- Proof: predicate tests plus full layer discovery/composition across independent
  Primary and Uprising homes, including project-root collisions and symlink aliases.
- Impact/shared: no public or wire change; shares loader plumbing with status config.
- Retire: when upstream has equivalent cross-installation leakage coverage.

### `runtime-stock-updater-policy` — rework

- Owner: private `app-server-daemon/src/stock_updater_policy.rs`; wiring remains in
  `lib.rs`, with process/WebSocket proof in `stock_updater_policy_tests.rs`.
- Proof: all daemon tests, including active-updater refusal, explicit stale-backend
  replacement, no updater start, idempotent matching-ready ensure, and aligned
  operator lifecycle documentation.
- Impact/shared: JSON shape is unchanged, but `autoUpdateEnabled` intentionally
  becomes `false`; an active legacy updater is a cutover precondition.
- Retire: when upstream supports externally managed installs without stock updater
  ownership and covers the same bootstrap/ensure lifecycle.

### `oauth-slack-response-adapter` — rework

- Owner: `rmcp-client/src/slack_oauth_envelope.rs` and the exact OAuth HTTP token
  endpoint hook; generic providers never enter the adapter.
- Proof: adapter unit tests plus exact-endpoint security coverage.
- Impact/shared: no public API or wire change; response bytes remain unchanged for
  non-Slack and lookalike endpoints.
- Retire: when upstream has equivalent provider-bound normalization or Slack emits
  standards-compliant OAuth responses.

### `workload-scoped-command-wrapper` — rework

- Owner: `core` unified-exec wrapper creation/application plus the optional
  `exec-server-protocol::ShellSnapshotRequest.outer_argv_prefix` transport and
  exec-server shell-snapshot forwarding.
- Proof: real unified exec with shell snapshots on/off, process-manager cache
  identity, protocol omission compatibility, exec-server process tests, isolated
  wrapper environment, and remote-prefix exclusion coverage.
- Impact/shared: serialized field is optional with a default, but Rust struct-literal
  users must initialize it; approval, policy, events, network, snapshots, and display
  continue to use original argv. This dependency-coupled leaf spans core/exec-server.
- Retire: when upstream provides the same per-thread attribution without changing
  any original-command semantics.

### `workload-mcp-runtime-attribution` — rework

- Owner: `codex-mcp` server/connection manager and
  `core/src/session/mcp_runtime.rs` reserved-key injection.
- Proof: reserved spoof override, HTTP exclusion, stdio connection-reuse tests,
  and a real core session observing its authoritative thread ID in the child.
- Impact/shared: configured connection identity and non-reserved environment remain
  unchanged; reserved workload keys are authoritative after reuse decisions. This
  adds the public cross-crate Rust method `EffectiveMcpServer::with_stdio_workload_attribution`.
- Retire: when upstream owns equivalent reserved-key attribution and reuse coverage.

### `residency-idle-timeout` — rework

- Owner: config key `thread_unload_delay_secs` and app-server thread lifecycle;
  request processors only report activity to the existing residency owner.
- Proof: config/default tests and app-server zero/one-second active-turn unload cases.
- Impact/shared: additive public `Config` field and config key; default remains 60
  seconds. Shares core config files and sample literals with status config.
- Retire: drop the code as soon as the target contains upstream `5e26f7621c`; keep
  any local five-minute deployment value outside this source leaf.

### `tui-external-status-config-contract` — rework

- Owner: private `core/src/config/external_status_config.rs` and sibling tests;
  additive config types and thin loader/core wiring only.
- Proof: absolute executable, bounded timeout, project-source rejection, trusted
  precedence, and same-layer conflict tests.
- Impact/shared: additive public `Tui.status_line_command` and `Config` field; shares
  loader/core wiring and sample literals with alternate-home/residency leaves.
- Retire: when upstream owns equivalent trusted-layer validation and conflicts.

### `tui-external-status-wire-v1-compatibility` — rework

- Owner: `tui/src/status_line_command/wire.rs`, its fixture/tests, and Bazel
  compile-data declaration.
- Proof: full/minimal deep equality and streaming rejection above 64 KiB.
- Impact/shared: preserves the installed formatter's version-1 model, workspace,
  context, rate-limit, PR, and Codex metadata contract.
- Retire: after every installed formatter migrates to an upstream-owned versioned wire.

### `tui-external-status-rich-output-runtime` — rework

- Owner: private parser module and safety tests.
- Proof: safe SGR and HTTPS OSC 8 parsing; invalid controls, invisible text, links,
  escapes, UTF-8, and excess output are rejected; only three rows are retained.
- Impact/shared: semantic spans and links reach presentation, never raw escapes.
- Retire: when upstream owns equivalent safe parsing or a structured output contract.

### `tui-external-status-command-runtime` — rework

- Owner: private process and runner modules plus the TUI crate dependency.
- Proof: exact nonsecret environment allowlist, credential/proxy rejection,
  output/timeout failure, Unix descendant termination, 300 ms coalescing,
  last-good ownership, and bounded retry tests.
- Impact/shared: preserves `PATH`, `CODEX_HOME`, XDG, and documented statusline
  controls required by the installed formatter; Windows containment is best-effort.
- Retire: when upstream owns equivalent environment compatibility, containment,
  debounce, and last-good execution.

### `tui-external-status-multiline-footer` — rework

- Owner: bottom-pane composer/footer height and rendering plus dedicated snapshots.
- Proof: bounded three-row reservation, instructional-footer suppression, right-badge
  preservation, styled spans, and semantic hyperlink marking.
- Impact/shared: high-touch footer files receive only presentation wiring.
- Retire: when upstream owns equivalent bounded rich footer presentation.

### `tui-external-status-integration` — rework

- Owner: `chatwidget/status_line_command.rs`, event dispatch, status-surface wiring,
  v1 input mapping, and focused runtime tests.
- Proof: owner/thread switching, stale result/timer suppression, automatic retry,
  local process cwd versus session wire cwd, and parsed multiline application.
- Impact/shared: user-visible TUI behavior; current upstream lacks a repository
  identity producer, so the optional v1 `workspace.repo` remains absent.
- Retire: when upstream owns equivalent v1 input mapping and owner-bound scheduling.

### `maintenance-repo-local-upgrade-workflow` — rework

- Owner: `.codex/skills/upgrade-codex-fork/` entrypoint and five playbooks.
- Proof: workflow contract mutation tests, exact-reference audit, and mandatory
  workload-slice placement checks before heavy validation.
- Impact/shared: maintainer policy only; no product/runtime surface.
- Retire: when an upstream/shared workflow provides the same Codex-specific leaf,
  target, preservation, gate, and authorization guarantees.

### `maintenance-workflow-contract-validation` — rework

- Owner: workflow/manifest coherence portions of
  `scripts/fork_fleet_validation.py` and its unit tests.
- Proof: malformed sections, extra/missing references, spoofed markers, dependency
  order, source accounting, retained-evidence rejection, target derivation,
  subprocess propagation, exact Just-only formatter adjudication, and the
  fail-closed differential-workspace contract markers and unit cases, including
  isolated registered-stage tool discovery and process-start cleanup.
- Impact/shared: shares the two validation scripts only with target-format policy.
- Retire: when Fork Fleet natively provides this Codex workflow/leaf audit.

### `maintenance-target-bound-format-validation` — rework

- Owner: target derivation and exact-target baseline portions of the validation scripts.
- Proof: missing/mismatched manifest target, injected-target mismatch, history target,
  Just-only baseline, multiple-formatter failure tests, and workspace pass-fast,
  subset acceptance, candidate-only rejection, target setup/timeout/parse failure,
  terminal-summary count and format rejection, explicit `FAIL`/`FL+LK` failure and
  `TMT` timeout membership parsing, category-preserving subset comparison, streaming
  output cap/redaction,
  per-command isolation, candidate/target immutability, byte-exact target lock
  normalization acceptance and mutation rejection, locked production command argv,
  real full-offline Cargo metadata restamping, and multi-command continuation tests.
- Impact/shared: updated Fork Fleet injects immutable target identity; standalone use
  independently derives history and rejects any provided mismatch. Only an unchanged
  exact-target `justfile` failure may be tolerated after all language groups pass.
  Workspace failures are dynamically accepted only when their normalized terminal
  nextest membership is a subset of a freshly executed exact-target result; there is
  no static failure allowlist. Per-side Cargo outputs, Git diagnostics, logs, and the
  disposable clone use private disk-backed, hard-capped storage rather than tmpfs.
  Name-set equivalence is
  classification evidence only; retained-leaf and changed-path tests still own
  semantic proof. Emitted evidence redacts credential-bearing headers through their
  line plus assignments, quoted JSON credentials, and sensitive environment values.
  Each fresh target clone is normalized offline only for the byte-for-byte expected
  local `0.0.0` package version-line restamps; comments, whitespace, key order, and
  all other drift fail closed. Both sides then run identical locked workspace-test
  argv.
- Retire: when Fork Fleet natively owns exact-target baseline comparison.

### `maintenance-validation-environment-fixtures` — rework

- Owner: only `protocol/src/permission_profile_intersection_tests.rs`, placing its
  temporary directory beneath the explicit test cwd.
- Proof: focused permission-profile intersection tests in the isolated validator.
- Impact/shared: test-only; no product, public, config, or wire change.
- Retire: when the upstream fixture passes unchanged in the isolated environment.

### `maintenance-generated-release-tail` — rework, final

- Owner: only `PATCHES.md`, regenerated `codex-rs/Cargo.lock`, and regenerated
  `codex-rs/core/config.schema.json` for this target.
- Proof: schema generation, locked metadata, Bazel-lock reproducibility, diff check,
  and exact Fork Fleet accounting. Semantic UI snapshots stay with their UI leaf.
- Impact/shared: Cargo regeneration restamps internal packages from `0.0.0` to
  `0.153.4`; the only new dependency edge is the TUI runtime dependency.
- Retire: when the frozen semantic candidate produces none of these target-specific
  artifacts or upstream packaging owns them reproducibly.

## Dropped, upstream-owned, or quarantined

The remaining 23 leaves are evidence, not candidate code:

- Upstream owns legacy turn/reconnect recovery, agent-gone wait failure, contained
  PTY behavior, and server-request ownership.
- Compaction epochs, orphan-output persistence, retaining-kind eviction, steer
  retry, UDS rendezvous, daemon identity, stored-child resume, handover lifecycle,
  and legacy telemetry were dropped after concrete ordering, race, security,
  compatibility, or ownership failures.
- The mixed OAuth source patch is quarantined; unsafe token transaction/reactive
  retry stay deferred, while only the provider-scoped Slack adapter survives.
- The broad 4.3k-line external-status source patch is quarantined behind six
  coherent config, wire, parser, process, footer, and integration leaves above.
- Tmp nesting is dropped because `TempDir::keep()` still leaks the same inode count
  and no repository-owned reaper proves bounded cleanup.
- Historical validation and generated transition commits are quarantined; their
  retained behavior is assigned to the atomic maintenance leaves above.

## Release invariants

- Every source commit is owned exactly once; source-free reworks supersede an
  explicit reviewed quarantine.
- No retained leaf exceeds 800 changed lines. The two source-compatible wire/parser
  ports exceed 500 only with their safety fixtures/tests; their production logic
  remains bounded and their retirement owners stay independent.
- The generated tail is last and contains no product logic or semantic snapshots.
- The complete workspace test suite still requires explicit user authorization.
- Publication, branch movement, packaging, deployment, and runtime cutover remain
  separately authorized operations and are not part of this prepare run.
