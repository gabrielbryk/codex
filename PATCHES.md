# Fork patch manifest

The `gabe/fork` branch is a patch stack rebased onto an upstream `rust-v*`
release tag. Ordered **bottom → top**: upstreamable fixes first, local-env
patches next, then defensive, then the regenerated lock chore last. Keep each
commit atomic (one fix + its tests) so it can be dropped when upstream covers
it, or `format-patch`ed to a PR. Fixes added between rebases land on top of the
lock chore (#12+ below); the next rebase reorders them back under it.

Current base: **rust-v0.147.0-alpha.4** (`8bf9d5d124`).

> **STALE — restored 2026-08-14, not yet reconciled.** This file was added by
> `8d67d3f1f0` and then lost from `gabe/fork` during the v2 normalization
> rebase; it survived only on `safety/codex-fork-v2-normalized-exact` and was
> recovered from there. The table below therefore describes the stack as of
> **rust-v0.147.0-alpha.4**, while `gabe/fork` is now based on
> **rust-v0.148.0-alpha.12** (`902bd9e0`) and carries 38 local commits.
>
> Entirely absent from the table: the live app-server handover stack (draining
> generations, passive daemon status, private generation sockets), request
> latency/token telemetry, MCP runtime thread attribution, scoped-command
> display, and the TUI reconnect/reattachment subset (`d8e876801f`, `c1a7b10f96`,
> `d954575147`, `2e666c2836`, `6be0e6749a`).
>
> Reconcile this table against the current stack **before** starting the
> rust-v0.148.0-alpha.18 upgrade — the drop/accept/rework/reapply triage that
> `CLAUDE.md` prescribes has no other authoritative source.

| # | Commit subject | Category | Files | Upstream status |
|---|---|---|---|---|
| 1 | `fix: reconnect remote app-server client` | upstreamable | `app-server-client/{lib,remote}.rs` | Fork-only; general resilience — candidate to PR upstream. |
| 2 | `fix: import Claude history from last compact boundary` | upstreamable | `external-agent-migration/src/sessions/{records_cla,records_cla_tests}.rs` | Fork-only; general feature — candidate to PR upstream. **Reworked at 0.147.0-alpha.4:** upstream split `sessions/records.rs` into per-agent readers (`records_cla.rs` for Claude, `records_cur.rs` for Cursor, shared `records_common.rs`), deleting the file this patch modified. Ported to `records_cla.rs` — the Claude-specific reader, which is where a `compact_boundary` record belongs — against upstream's renamed `read_session_import` (was `read_session_import_with_cwd`; the `fallback_timestamp` parameter is gone). Tests moved to `records_cla_tests.rs` and rewritten against upstream's inline-JSON style, since upstream's split dropped the `session_record` helper the fork's tests used. Only the `read_session_import` loop is patched; `summarize_session` is deliberately untouched, matching the original patch's scope. |
| 3 | `fix(config): isolate alternate Codex homes` | local-env | `config/src/loader/{mod,tests}.rs` | Fork-only forever — specific to the dual `.codex` / `.codex-uprising` setup. |
| 4 | `fix(uds): accept sticky rendezvous directories` | local-env | `uds/src/{lib,lib_tests}.rs` | Fork-only forever — specific to the shared sticky `/tmp` socket dir. |
| 5 | `fix(app-server-daemon): disable stock auto-updater for local fork` | local-env | `app-server-daemon/src/lib.rs` | Fork-only forever — the managed-fork deploy must not let the stock updater snap `standalone/current` back to upstream stock. |
| 6 | `fix(rmcp-client): classify invalid_grant startup errors as reauth` | defensive | `rmcp-client/src/startup_error.rs` | Fork-only; largely redundant since 0.145.0's `refresh_transaction` maps `invalid_grant`→`AuthorizationRequired`. **0.147.0-alpha.4:** upstream `61de0d8fe8` (rmcp 3.0.0-beta.3) split `TokenRefreshFailed` from `TokenRefreshRejected`, narrowing what can still reach this path — the strongest drop candidate in the stack. Re-evaluate next upgrade; drop if no raw path can surface it. |
| 7 | `fix(rmcp-client): harden MCP OAuth reauth (delete guard + reactive 401 recovery)` | defensive | `rmcp-client/src/oauth.rs`, `rmcp-client/src/oauth/resolved_store.rs`, `rmcp-client/src/oauth/refresh_transaction.rs`, `rmcp-client/src/rmcp_client.rs` | Fork-only; strong candidate to PR upstream. One coherent "harden MCP OAuth reauth" patch closing two holes 0.145.0's oauth rewrite left. **(a) Compare-and-delete guard:** `persist_if_needed`'s `None` branch deleted the resolved-store entry unconditionally whenever the in-memory `AuthorizationManager` reported no credentials — including a transient refresh miss after a startup 401 — wiping a still-valid on-disk refresh token and forcing a full re-login. Adds `ResolvedOAuthCredentialStore::delete_if_stale` + `delete_oauth_tokens_from_file_if_stale`: a lock-held authoritative reread that refuses to evict a File entry still holding a usable refresh token or one that no longer matches the evicted token. **(b) Reactive 401 recovery:** 0.145.0 replaced the old patch's live-401 recovery with proactive expiry-only pre-refresh, so a runtime `401 AuthRequired` in `run_service_operation` fell straight through to `Err` — no refresh, no retry — and expiry-gated pre-refresh can't cover `expires_at == None` (`token_needs_refresh` returns false), early revocation, or clock skew. Adds `is_auth_required_401` + a single-retry recovery arm mirroring the `is_session_expired_404` path, driven by `OAuthPersistor::refresh_after_unauthorized` (a `RefreshTrigger::Unauthorized` transaction that forces the authoritative locked reread / adopt-newer / fail-closed refresh regardless of cached expiry, reusing guard (a) so a failed refresh can't wipe a valid token). Scoped to the File store (this deploy's `mcp_oauth_credentials_store = "file"`); keyring paths unchanged. **(c) Host/executor collision guard (`74774cc43d`, added at 0.147.0-alpha.4):** upstream #36310 (`164b3bfeab`) added a fail-closed check to `delete_oauth_tokens_from_file` refusing an `executor:`-prefixed key that resolves to an entry lacking the `executor_owned` marker. Because guard (a) reroutes `persist_if_needed`'s eviction through the fork's own `delete_oauth_tokens_from_file_if_stale`, upstream's guard was unreachable on the path this deploy takes — the fork had silently reopened the executor/host boundary upstream had just closed. Mirrored at the same position as the upstream sibling. **Recurring hazard: whenever upstream hardens `delete_oauth_tokens_from_file`, check whether the fork's `_if_stale` twin needs the same change** — no test, range-diff, or symbol-presence check catches a guard that upstream added and the fork's divergent copy never received. Re-evaluate each release; drop once upstream's `None` branch stops deleting unconditionally AND upstream recovers live 401s. |
| 8 | `fix(tui): keep thread ops non-fatal when the app-server request fails` | upstreamable | `tui/src/app/thread_routing.rs`, `tui/src/app/tests/turn_submission.rs` | Fork-only; candidate to PR upstream. Upstream PR #34636 made only `turn/start` non-fatal (see patch #10 below); the remaining arms of `try_submit_active_thread_op_via_app_server` still propagate app-server request failures with `?`, so the same transport blip still kills the TUI through `/compact`, `/rename`, `/review`, background-terminal cleanup, `!` shell commands, config reload, or approving a guardian-denied action. Each of those arms now logs `tracing::warn!` + renders a chat error naming the operation and the full cause chain, and returns `Ok(true)`. `Review` skips the review-thread bookkeeping entirely on failure so no partial state is recorded (the invalid-review-thread-id parse stays fatal — a malformed response is not retry-recoverable). Deliberately unchanged: the `Interrupt` turn-mismatch race and the `UserTurn` steer-race paths, which are protocol races rather than transport failures. Regression test lives in upstream's `app/tests/turn_submission.rs` harness. Re-evaluate each release; drop once upstream stops exiting the TUI on failed thread ops. |
| 9 | `feat(tui): include a resume hint in the fatal exit message` | upstreamable | `tui/src/app/app_server_events.rs`, `tui/src/app/tests.rs`, `tui/src/app/tests/fatal_exit.rs` | Fork-only; candidate to PR upstream (requested by openai/codex#33976). After patches #8/#10 the only remaining fatal TUI exit is `AppServerEvent::Disconnected` → `AppEvent::FatalExitRequest` (the single sender in the tree), whose message `main.rs` prints as `ERROR: <transport error>` with no way back into the session — the existing `AppExitInfo::resume_hint` line only appears when the rollout file is already on disk and resumable. The send site now appends `\nResume this session with: codex resume <thread-id>` using `codex_utils_cli::resume_command` and the displayed/primary thread id, so the printed fatal error always carries the recovery command when a thread id is known. Re-evaluate each release; drop once upstream prints a resume hint on fatal exit. |
| 10 | `fix(tui): treat turn/start transport failures as non-fatal too` | upstreamable | `tui/src/app/event_dispatch.rs` | Fork-only residue of a dropped patch; candidate to PR upstream. Upstream PR #34636 (`handle_turn_start_rejection` + `app/tests/turn_submission.rs`, in 0.146.0-alpha.1+) superseded the fork's own `turn/start` patch, but its guard matches only `TypedRequestError::Server`. This fork carries patch #1's reconnect path, which calls `fail_pending_requests()` on every websocket drop before reconnecting, so an in-flight `turn/start` routinely fails as `TypedRequestError::Transport` and would still kill the session. Extracts `is_recoverable_turn_start_failure` and widens it to `Transport` for `turn/start` only; every other method and every non-typed error stays fatal. Unit tests in `event_dispatch::tests`. Re-evaluate each release; drop once upstream's guard covers transport failures. |
| 11 | `chore: refresh workspace lock for 0.147.0-alpha.4` | chore | `codex-rs/Cargo.lock` | Regenerated every release (release tags bump `Cargo.toml` but ship a `0.0.0` lock; the first `cargo` run stamps the real version). Drop + recreate each upgrade. `just bazel-lock-check` passes at this base, so `MODULE.bazel.lock` needed no companion update. |
| 12 | `fix(core): persist synthetic outputs for orphaned tool calls` | upstreamable | `core/src/context_manager/{normalize,history,history_tests}.rs`, `core/src/session/{mod,turn}.rs` | Fork-only; strong candidate to PR upstream. Observed live: a turn wedged on a dead `wait_agent` custom tool call logged `Custom tool call output is missing for call id: call_…` on every retry for hours, and kept doing so across turn boundaries. `ContextManager::for_prompt` consumes a *throwaway snapshot*, so the synthetic `"aborted"` output `ensure_call_outputs_present` injects only ever reaches the outgoing request; the durable history keeps the orphaned call forever and every later request re-detects, re-logs, and re-injects it (a resume replays the same orphan). `ensure_call_outputs_present` now returns the injected outputs, `ContextManager::backfill_missing_call_outputs` applies the repair in place, and `Session::backfill_missing_call_outputs` records them into history + rollout before each prompt build in `run_turn` and the sampling retry loop. **Conflict watch (0.147.0-alpha.4):** the file to watch is `core/src/context_manager/history.rs`, changed +47/−16 by upstream `4f6d06d485`; `normalize.rs` is untouched upstream, so that half stays clean. Upstream's existing `remove_orphan_outputs` (`normalize.rs`) handles the *inverse* case — an output with no call — and is unrelated to this patch. The missing-custom-tool-output log drops from `error_or_panic` to `warn!` because request-only normalization paths (compaction, prompt debug) can still legitimately see the same orphan — the corresponding `..._panics_in_debug` test is removed and the previously `cfg(not(debug_assertions))`-gated positive test now runs everywhere. Re-evaluate each release; drop once upstream persists the repair. |
| 13 | `fix(core): fail wait_agent fast when a target thread is gone` | upstreamable | `core/src/tools/handlers/multi_agents/wait.rs`, `core/src/tools/handlers/multi_agents_tests.rs`, `core/tests/suite/agent_execution.rs` | Fork-only; candidate to PR upstream. Observed live: a turn repeatedly called `multi_agent_v1__wait_agent` on a sub-agent whose spawn had already failed with `no thread with id: …`; each call burned the full `timeout_ms` and returned `{"status":{},"timed_out":true}`, a success payload carrying no liveness information, so the model just waited again. Two gaps: (a) a target that is neither live nor in the agent registry was folded into the result as a plain `not_found` status entry instead of a failure — `close_agent` already makes the known/unknown distinction, `wait_agent` now does too and returns a tool error naming the dead targets; (b) a target can vanish mid-wait without waking its status watch (the `watch::Sender` lives in the thread object and can outlive the registry entry), so the wait runs to the deadline and reports an empty status map — the timeout path now re-checks the registry and errors when every target has gone missing. Still-starting targets keep the existing wait behaviour. **0.147.0-alpha.4: this patch got more valuable, not less.** Upstream `8a1c941439` raises the recommended `wait_agent` timeout to minute-scale, so an un-fast-failed dead target now burns a minute per poll instead of seconds. `e597169e9a` (thread-to-path index) removes one *cause* of registry/thread desync but still does not fail the wait. Re-evaluate each release; drop once upstream fails wait on definitively-missing targets. |
| 14 | `fix(rmcp-client): accept Slack Web-API envelope in OAuth token responses` | upstreamable | `rmcp-client/src/{lib,oauth_http_client,slack_oauth_envelope,slack_oauth_envelope_tests}.rs` | Fork-only; strong candidate to PR upstream — though the real fix arguably belongs further up the stack, in the `oauth2` crate's `endpoint_response` or in rmcp's `AuthorizationManager`, neither of which this fork owns. Observed live: Slack MCP OAuth refreshes had failed continuously for ~25 days with `OAuth token refresh failed: Failed to parse server response`. Slack's discovery document points `token_endpoint` at `https://slack.com/api/oauth.v2.user.access`, a Slack Web API method rather than an RFC 6749 token endpoint: it answers HTTP 200 for both success and failure and wraps the payload in Slack's own envelope (`{"ok":true,"access_token":…}` / `{"ok":false,"error":"invalid_refresh_token"}`). The `oauth2` crate accepts only RFC 6749 shapes, so *every* Slack token response — success or failure alike — fell out as `RequestTokenError::Parse`, whose `Display` is the useless `Failed to parse server response`; a rejected refresh token was indistinguishable from a malformed body, so the actual cause never reached the logs. The fix goes in at the seam the fork does own: `OAuthHttpClientAdapter`, the `OAuthHttpClient` impl every OAuth HTTP request funnels through. On POST responses only (token exchange + refresh; discovery GETs are never touched), a body that parses as JSON with a boolean `ok` field is rewritten before `oauth2` sees it — `ok: true` becomes a standard token response (`token_type` forced to `bearer`, since Slack reports the token *audience* `user`/`bot` where RFC 6749 wants the HTTP auth scheme; `refresh_token`/`expires_in`/comma-to-space-normalized `scope` carried through, with the nested `authed_user` object as fallback source), and `ok: false` becomes HTTP 400 `{"error":"<slack error>"}` so `oauth2` yields a real `ServerResponse` error naming the cause. Anything that is not a Slack envelope — RFC-compliant token and error bodies, discovery documents, non-JSON payloads — passes through byte-identical. **Build constraint (0.147.0-alpha.4): `rmcp-client` lost its `reqwest` exception in `deny.toml`** — this patch must keep using `codex-http-client` types or `cargo deny check bans` fails. Verified clean at this base. **Reworked at 0.147.0-alpha.4:** upstream PRs #35806/#35814 ("Route/Use configured HTTP clients for all MCP OAuth requests") rewrote the tail of `OAuthHttpClientAdapter::execute_request`, replacing the inline `OAuthHttpClientError::new(error.to_string())` mapping with a shared `oauth_http_client_error` helper. Conflict resolved by keeping upstream's helper and re-attaching the fork's `is_post` normalization on top; the `slack_oauth_envelope` module and its tests replayed unchanged. Re-evaluate each release; drop once `oauth2`/rmcp tolerate the Slack envelope upstream, or once Slack advertises a conformant token endpoint. |
| 15 | `feat: replay in-flight turn/start across a reconnect` (3 commits: `app-server` idempotency, `app-server-client` replay, `tui` idempotency key) | upstreamable | `app-server/src/request_processors/turn_processor.rs`, `app-server/tests/suite/v2/turn_start.rs`, `app-server-client/src/{lib,remote}.rs`, `tui/src/app_server_session.rs` | Fork-only; candidate to PR upstream (see openai/codex#13949). **Pairs with patch #1** — that patch reconnects the remote transport after a websocket drop, but it calls `fail_pending_requests()` first, so an in-flight `turn/start` still surfaced a transport error even though the connection recovered moments later, costing the user the prompt they had just sent (patch #10 keeps that failure non-fatal, but the submission is still lost). Three commits close the loop end to end. **(a) app-server:** `client_user_message_id` was carried through purely as a correlation label on the resulting user message item; every `turn/start` unconditionally started or steered a new turn, making the request unsafe to re-send. It is now treated as an idempotency key — a bounded, process-wide, per-thread cache records the `TurnStartResponse` each keyed submission produced and returns it verbatim on a repeat of the same key, rather than starting a second turn; the cache evicts oldest-first, is fixed-cost regardless of live thread count, and only has to outlive a client reconnect. Requests without a key keep their previous behavior. **(b) app-server-client:** replay-safe requests are parked across the reconnect instead of failed. A request qualifies only if its method is on a small allowlist (`turn/start`) *and* it carries a `clientUserMessageId` — without the key the server cannot recognize the replay as a duplicate. Parked requests are re-sent on the freshly initialized stream under their original JSON-RPC ids so responses route back to waiting callers through the normal path. Give-up semantics are deliberately conservative: replayed at most once, abandoned after 15s, failing with the transport error that parked it; every other in-flight request keeps the previous fail-fast behavior. **(c) tui:** stamps a fresh UUID on each `turn/start` it sends, which is what arms (a) and (b). Net effect: a momentary app-server restart no longer costs the user a prompt. **Reworked at 0.147.0-alpha.4:** upstream boxed the `AppServerEvent::ServerNotification` payload (`Box<ServerNotification>`), which the rebase auto-merged silently and only surfaced as a `cargo check` type error in the fork's own reconnect test. The assertion now matches upstream's own shape at `app-server-client/src/lib.rs` — bind the payload and test it through `notification.as_ref()`. Production replay logic was unaffected. Re-evaluate each release; drop once upstream makes `turn/start` idempotent and replays it across reconnects. |
| 16 | `feat(tui): external status line command mode` (`bfbc6ea6da`, squashed) | fork-feature | `tui/src/status_line_command/{mod,parser,parser_tests,process,process_tests,runner,runner_tests,wire,wire_tests}.rs` (new), `tui/src/chatwidget/status_line_command.rs` (new) + `tui/src/chatwidget/{status_controls,status_surfaces,constructor,session_flow,tests}.rs`, `tui/src/chatwidget/tests/{status_and_layout,status_line_command_runtime,status_surface_previews}.rs`, `tui/src/app/{event_dispatch,background_requests,tests}.rs`, `tui/src/app_event.rs`, `tui/src/bottom_pane/{chat_composer,footer,mod}.rs` + `chat_composer/footer_state.rs` + `chat_composer_status_line_tests.rs`, `tui/src/branch_summary.rs`, `tui/src/status/{rate_limits,tests}.rs`, `tui/src/terminal_hyperlinks.rs`, `tui/src/token_usage.rs`, `tui/src/lib.rs`, `config/src/{loader/mod,types}.rs`, `core/src/config/{mod,config_tests,config_loader_tests}.rs`, `core/config.schema.json`, plus new snapshot fixtures under `tui/src/chatwidget/snapshots/` and `tui/src/bottom_pane/snapshots/`. | Fork-only feature; not upstreamable as-is — the wire schema for the external status-line command is Codex-owned protocol surface upstream hasn't shipped. Re-evaluate if upstream ships its own command-backed statusline (then rework/drop in favor of it). Adds a `/statusline`-guarded external command mode: user-configured command runs locally, its stdout is parsed per `wire.rs`'s contract and rendered into the footer/status surfaces (supports multiline, preserves hyperlinks, derives repo from origin). Includes the former `fix(config): ignore project status line commands` step — security-relevant: project-level config cannot silently inject an external status-line command (loader change + `config_loader_tests.rs` coverage), closing an arbitrary-local-command-execution vector from an untrusted repo's config. Containment decision (best-effort process-group on Unix — `setsid()` escape is documented, not fixed; full Job Object tree-kill on Windows) recorded 2026-08-02 in `.codex/status-line-command-proposal.md` (tracked as part of this commit). **Cell-semantics hazard (`40338f8f32`, fixed at 0.147.0-alpha.4):** ratatui 0.30.2 reserves a terminal cell for halfwidth sound marks U+FF9E/U+FF9F; the `unicode-width` crate reports **zero** for both. `parser.rs` measured the `MAX_HYPERLINK_LABEL_CELLS` budget with raw `UnicodeWidthStr::width`, so a label built from sound marks scored zero cells and was admitted at any length, then rendered past the bound; `saw_visible_text` mis-classified a sound-mark-only run as invisible. **Neither produced a compile error** — the upgrade silently changed what the old measurement meant. Both sites now use `crate::width::{display_width, char_width}`. **Standing rule: any column/cell arithmetic in fork-owned TUI code must use `crate::width::*`, never `unicode_width` directly.** **Reworked at 0.147.0-alpha.4:** upstream's `terminal_hyperlinks.rs` refactor dropped the `UnicodeWidthStr` import this patch's `remap_hyperlinks_to_visible_line` relied on for `common_prefix.width()`. Switched to the crate's own `crate::width::display_width`, which was already imported in the upstream file and additionally corrects for halfwidth sound marks the way Ratatui accounts for columns — the more correct measure here, since the value indexes hyperlink *column* ranges. |
| 17 | `feat(utils-pty): add contained process spawn API` (`b08492404d`, squashed) | upstreamable | `utils/pty/Cargo.toml`, `utils/pty/src/{lib,pipe,tests}.rs`, `utils/pty/src/win/{job,mod,suspended}.rs`, `utils/pty/src/windows_tests.rs` | Fork-only; candidate to PR upstream as a general-purpose primitive (a spawn API that atomically returns a killable process handle, not statusline-specific). Adds `spawn_contained_process` / `spawn_piped_contained_process` (renamed from `spawn_process_tree` / `spawn_piped_process_tree` during development, which also renamed `PipeSpawnMode::ContainedProcessTree` → `Contained` — naming/doc-only, no behavior change, done to stop overclaiming whole-tree kill on Unix) plus a Windows suspended-spawn path (`win/suspended.rs`) that creates the child suspended, assigns it to a Job Object before first instruction, then resumes — closing the TOCTOU window where a fast-exiting child could be missed by the containment handle. Consumed by patch #16's `status_line_command/process.rs`. **0.147.0-alpha.4 — orthogonal and synergistic with upstream `6b23635a7e`:** upstream changed *signal dispatch* (Windows non-TTY `Interrupt` now routes to `kill()`, terminator consumed after success, new `ProcessDriver.tty` field) while this patch changes *containment establishment*. Combined, Ctrl-C on a contained Windows process now terminates the whole job tree. **Caveat to check if the runner ever grows soft-signal expectations: Windows `Interrupt` on a pipe-backed process is now an unconditional hard tree-terminate.** Linux-only deploy here, so not currently load-bearing. |
| 18 | `test(tui): restore upstream status surface snapshots` (`f151d915e2`) | fork-local, drop candidate | `tui/src/chatwidget/snapshots/*.snap` (5 files) | Not upstreamable — fixes machine-baked values this fork's own `37bd8ed002` upgrade-session re-record introduced (host `/tmp` resolved as a git repo root, baking literal `"tmp"` in place of upstream's `"my-project"`/`"project"` placeholders; also stamps the fork's release version into a narrow-terminal snapshot missed because `insta` stops at the first failing assertion). Drop at the next rebase if upstream's own snapshots regenerate cleanly against a normal (non-`/tmp`) checkout — this patch exists only to undo fork-environment contamination, not to change upstream behavior. |

**Squash status:** rows 16–18 were squashed at the rust-v0.147.0-alpha.4 rebase (30 statusline commits → `bfbc6ea6da`; 3 utils-pty commits → `b08492404d`; row 18 left as its own commit so it stays visible as a drop candidate). The squash was verified content-identical: `git diff fork/pre-0.147-squash HEAD -- . ':!PATCHES.md'` is empty. `utils/pty` was deliberately NOT folded into the statusline commit — it is a standalone primitive intended for separate upstreaming.

## History note — dropped patches

- **Stale rebase lock chore** (`chore: rebase fork onto rust-v0.146.0`,
  `888f445642`): **dropped at rust-v0.147.0-alpha.4** and not replaced. It was a
  pure restamp of `MODULE.bazel.lock` + `codex-rs/Cargo.lock` + this file's base
  line for the 0.146.0 base, so replaying it onto a newer tag only produced
  lockfile conflicts against values that patch #11 immediately overwrites. The
  equivalent work for each new base belongs in patch #11 alone. Note the fork
  had already moved to stable `rust-v0.146.0` via this commit even though the
  manifest's base line still read `0.146.0-alpha.10.1` — verify the real base
  with `git merge-base --is-ancestor <tag> HEAD` rather than trusting the header.

- **MCP OAuth 401-recovery** (`harden local mcp oauth recovery`): **mostly
  upstreamed in rust-v0.145.0** as the `rmcp-client/src/oauth/` module
  (refresh_lock, refresh_transaction, resolved_store, store_lock + recovery
  tests). Dropped from the stack — do not reintroduce the old monolithic
  `oauth.rs` patch. **Caveat — two pieces the rewrite dropped, both reapplied as
  patch #7 above, reworked to the new module shape (not the old monolith):**
  1. **Compare-and-delete guard.** 0.145.0 did **not** port the old
     `delete_oauth_tokens_from_file_if_match` +
     `delete_oauth_tokens_if_match_keeps_newer_fallback_token` test. The new
     `persist_if_needed` `None` branch deletes the store entry unconditionally,
     reopening the credential-loss race. Reapplied as `delete_if_stale` on
     `ResolvedOAuthCredentialStore`.
  2. **Reactive live-401 recovery.** The old patch classified a live `401
     AuthRequired`, refreshed, and retried the op once. 0.145.0 kept only
     proactive expiry-based pre-refresh, leaving a runtime 401 to surface to the
     user (and pre-refresh can't see `expires_at == None`, early revocation, or
     clock skew). Reapplied as `is_auth_required_401` + a single-retry arm in
     `run_service_operation` driven by `OAuthPersistor::refresh_after_unauthorized`.

- **MCP connection-manager reap** (`fix(mcp): reap superseded MCP connection
  managers on refresh`): **superseded in rust-v0.146.0-alpha.x** and dropped —
  do not reapply. Upstream PR #34952 ("Reuse MCP connections across runtime
  refreshes") rebuilt the whole seam the patch hooked: `McpConnectionManager` is
  now `McpConnectionSet`, `SessionServices::publish_mcp_runtime` moved into
  `core/src/session/mcp_runtime.rs`, and `McpRuntime::replace` builds the new set
  from the previous one so unchanged servers keep the *same*
  `Arc<McpServerConnection>` instead of respawning. #34952 also added
  `impl Drop for McpServerConnection` (cancels the client's `cancel_token`) —
  exactly the backstop the fork patch added, now at connection granularity, so a
  non-reused connection's stdio child dies when the superseded set drops. PR
  #34957 ("Replace closed MCP connections during reconciliation") hardens the
  same path. **Reapplying the old drain would now be a regression:** it called
  `superseded.shutdown()`, and under the reuse design the superseded set shares
  `Arc<McpServerConnection>` values with the live one, so that would tear down
  connections the current runtime is actively using.

- **turn/start non-fatal in the TUI** (`fix(tui): surface turn/start failure in
  chat instead of exiting`): **upstreamed in rust-v0.146.0-alpha.1** as PR #34636
  (`ChatWidget::handle_turn_start_rejection` + the `event_dispatch` guard +
  `app/tests/turn_submission.rs`). Dropped; the fork's regression test was
  duplicative and removed. **Caveat:** upstream's guard only matches
  `TypedRequestError::Server`, which is not enough for this fork — reapplied
  narrowly as patch #10 above.

## Upstream watch list (recorded at rust-v0.147.0-alpha.4)

Things that are not fork patches but will cost time at a future upgrade if
forgotten. Delete a line once it has been absorbed or has stopped being true.

- **Release artifact renamed AND recompressed.** `3d1d26915a` stopped publishing
  `codex-<target>-bundle.tar.zst`. The replacement is
  `codex-package-<target>.tar.gz` — different name *and* gzip instead of zstd,
  so install tooling needs both the name and the `zstd -d` → `gzip` step
  changed. `.github/dotslash-config.json` still matches `codex-package-*.tar.zst`;
  do **not** infer the extension from that file.
- **Code mode is out-of-process only.** `97576b1794` removed the embedded V8
  fallback, so a standalone release must ship `codex-code-mode-host` beside
  `codex`. `codex-use-local-build` already enumerates and copies referenced
  sibling binaries, so this is satisfied today — re-check if that helper changes.
  The same commit deleted `CODEX_CODE_MODE_HOST_PATH` and made
  `effective_tool_mode` silently downgrade `CodeMode → Direct` when the host
  binary is missing.
- **Protocol types now ship precomputed.** `acd540f158` / `4642370542` made
  `ts-rs` + `schemars` dev-only and embed the schemas as zstd artifacts. Any
  fork patch adding a protocol type must run `just write-app-server-schema` or
  the embedded-vs-generated equality test fails.
- **`PlannedTools` is gone.** `89a0eed93c` replaced it with a trust-tiered
  `ToolRegistry` (`register_trusted` panics on duplicate; `register_external`
  warns and skips). If the fork ever carries custom tool registration, treat
  `b293412c24 + 9a46fd33a0 + 89a0eed93c + c126f206da + 66ebeb7037 + 385fe95ce1`
  as a single upstream squash and re-author against `finalize_tool_router`
  rather than resolving hunk-by-hunk — several of them edit identical hunks and
  will conflict more than once.
- **`isPinned` was removed outright** from thread metadata and filters
  (`85c6da1c79`), replaced by persisted sections. Relevant only if fork code
  ever reads thread metadata.
- **Divergent-copy hazard.** Where a fork patch introduces a *parallel* version
  of an upstream function rather than editing it, upstream hardening lands only
  on the original. `delete_oauth_tokens_from_file_if_stale` already hit this
  (see patch #7c). No test, range-diff, or symbol-presence check detects it —
  it must be looked for deliberately, by diffing the fork twin against its
  upstream sibling.

### Known-failing tests in this environment (not regressions)

`codex-tui ide_context::ipc::tests` — 5 failures plus 1 timeout, all reporting
`IDE context socket directory is writable by other users`. Cause is the
machine's `umask 002`: temp dirs are created `0775` and upstream's check rejects
any `mode() & 0o022`. `tui/src/ide_context/` is byte-identical to upstream, so
this is an upstream/environment interaction, not a fork defect. Do not
re-investigate each upgrade; do re-confirm the file is still untouched.

Run tests through `just test`, not raw `cargo test` — the justfile sets
`rust_min_stack = 8388608`, and at least one upstream test
(`codex-app-server-client tests::typed_request_roundtrip_works`) overflows the
default stack without it.

## Triage checklist (each upgrade)

1. `git log --oneline <newtag>..HEAD` — review the carried stack. **Verify the
   real base first** with `git merge-base --is-ancestor <tag> HEAD`; this file's
   header has drifted from reality before.
2. For each patch above: is the behavior + its tests now upstream? If yes, drop.
   If partially, rework against the new upstream shape. Else reapply.
3. Rebase with `--onto <newtag> <oldtag>` (tags are not linear ancestors).
4. Validate, rebuild, install, reconcile via the `upgrade-codex-fork` skill.
5. Tag `fork/<newversion>`; update this file's base + any status changes.
6. Post-rebase verification beyond "tests pass" — each catches a distinct class:
   - `git range-diff <pre-rebase-tag>...HEAD` — every commit should be `=`;
     audit each `!` and account for every dropped/added entry. A patch whose
     file was renamed upstream shows as unpaired, not dropped — confirm by
     grepping for its symbols.
   - Grep for each patch's key symbols and each named regression test. A test
     silently lost in a conflict resolution still reports "all passing".
   - Attribute every file in `git diff --name-only <newtag>..HEAD` to a patch;
     anything unattributable is an accidental upstream revert.
   - Diff any fork twin of an upstream function against its sibling (see
     divergent-copy hazard above).
