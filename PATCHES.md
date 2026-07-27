# Fork patch manifest

The `gabe/fork` branch is a patch stack rebased onto an upstream `rust-v*`
release tag. Ordered **bottom → top**: upstreamable fixes first, local-env
patches next, then defensive, then the regenerated lock chore last. Keep each
commit atomic (one fix + its tests) so it can be dropped when upstream covers
it, or `format-patch`ed to a PR.

Current base: **rust-v0.146.0-alpha.10.1**.

| # | Commit subject | Category | Files | Upstream status |
|---|---|---|---|---|
| 1 | `fix: reconnect remote app-server client` | upstreamable | `app-server-client/{lib,remote}.rs` | Fork-only; general resilience — candidate to PR upstream. |
| 2 | `fix: import Claude history from last compact boundary` | upstreamable | `external-agent-migration/src/sessions/records.rs` | Fork-only; general feature — candidate to PR upstream. |
| 3 | `fix(config): isolate alternate Codex homes` | local-env | `config/src/loader/{mod,tests}.rs` | Fork-only forever — specific to the dual `.codex` / `.codex-uprising` setup. |
| 4 | `fix(uds): accept sticky rendezvous directories` | local-env | `uds/src/{lib,lib_tests}.rs` | Fork-only forever — specific to the shared sticky `/tmp` socket dir. |
| 5 | `fix(app-server-daemon): disable stock auto-updater for local fork` | local-env | `app-server-daemon/src/lib.rs` | Fork-only forever — the managed-fork deploy must not let the stock updater snap `standalone/current` back to upstream stock. |
| 6 | `fix(rmcp-client): classify invalid_grant startup errors as reauth` | defensive | `rmcp-client/src/startup_error.rs` | Fork-only; largely redundant since 0.145.0's `refresh_transaction` maps `invalid_grant`→`AuthorizationRequired`. Re-evaluate each release; drop if a raw path can no longer surface it. |
| 7 | `fix(rmcp-client): harden MCP OAuth reauth (delete guard + reactive 401 recovery)` | defensive | `rmcp-client/src/oauth.rs`, `rmcp-client/src/oauth/resolved_store.rs`, `rmcp-client/src/oauth/refresh_transaction.rs`, `rmcp-client/src/rmcp_client.rs` | Fork-only; strong candidate to PR upstream. One coherent "harden MCP OAuth reauth" patch closing two holes 0.145.0's oauth rewrite left. **(a) Compare-and-delete guard:** `persist_if_needed`'s `None` branch deleted the resolved-store entry unconditionally whenever the in-memory `AuthorizationManager` reported no credentials — including a transient refresh miss after a startup 401 — wiping a still-valid on-disk refresh token and forcing a full re-login. Adds `ResolvedOAuthCredentialStore::delete_if_stale` + `delete_oauth_tokens_from_file_if_stale`: a lock-held authoritative reread that refuses to evict a File entry still holding a usable refresh token or one that no longer matches the evicted token. **(b) Reactive 401 recovery:** 0.145.0 replaced the old patch's live-401 recovery with proactive expiry-only pre-refresh, so a runtime `401 AuthRequired` in `run_service_operation` fell straight through to `Err` — no refresh, no retry — and expiry-gated pre-refresh can't cover `expires_at == None` (`token_needs_refresh` returns false), early revocation, or clock skew. Adds `is_auth_required_401` + a single-retry recovery arm mirroring the `is_session_expired_404` path, driven by `OAuthPersistor::refresh_after_unauthorized` (a `RefreshTrigger::Unauthorized` transaction that forces the authoritative locked reread / adopt-newer / fail-closed refresh regardless of cached expiry, reusing guard (a) so a failed refresh can't wipe a valid token). Scoped to the File store (this deploy's `mcp_oauth_credentials_store = "file"`); keyring paths unchanged. Re-evaluate each release; drop once upstream's `None` branch stops deleting unconditionally AND upstream recovers live 401s. |
| 8 | `fix(tui): keep thread ops non-fatal when the app-server request fails` | upstreamable | `tui/src/app/thread_routing.rs`, `tui/src/app/tests/turn_submission.rs` | Fork-only; candidate to PR upstream. Upstream PR #34636 made only `turn/start` non-fatal (see patch #10 below); the remaining arms of `try_submit_active_thread_op_via_app_server` still propagate app-server request failures with `?`, so the same transport blip still kills the TUI through `/compact`, `/rename`, `/review`, background-terminal cleanup, `!` shell commands, config reload, or approving a guardian-denied action. Each of those arms now logs `tracing::warn!` + renders a chat error naming the operation and the full cause chain, and returns `Ok(true)`. `Review` skips the review-thread bookkeeping entirely on failure so no partial state is recorded (the invalid-review-thread-id parse stays fatal — a malformed response is not retry-recoverable). Deliberately unchanged: the `Interrupt` turn-mismatch race and the `UserTurn` steer-race paths, which are protocol races rather than transport failures. Regression test lives in upstream's `app/tests/turn_submission.rs` harness. Re-evaluate each release; drop once upstream stops exiting the TUI on failed thread ops. |
| 9 | `feat(tui): include a resume hint in the fatal exit message` | upstreamable | `tui/src/app/app_server_events.rs`, `tui/src/app/tests.rs`, `tui/src/app/tests/fatal_exit.rs` | Fork-only; candidate to PR upstream (requested by openai/codex#33976). After patches #8/#10 the only remaining fatal TUI exit is `AppServerEvent::Disconnected` → `AppEvent::FatalExitRequest` (the single sender in the tree), whose message `main.rs` prints as `ERROR: <transport error>` with no way back into the session — the existing `AppExitInfo::resume_hint` line only appears when the rollout file is already on disk and resumable. The send site now appends `\nResume this session with: codex resume <thread-id>` using `codex_utils_cli::resume_command` and the displayed/primary thread id, so the printed fatal error always carries the recovery command when a thread id is known. Re-evaluate each release; drop once upstream prints a resume hint on fatal exit. |
| 10 | `fix(tui): treat turn/start transport failures as non-fatal too` | upstreamable | `tui/src/app/event_dispatch.rs` | Fork-only residue of a dropped patch; candidate to PR upstream. Upstream PR #34636 (`handle_turn_start_rejection` + `app/tests/turn_submission.rs`, in 0.146.0-alpha.1+) superseded the fork's own `turn/start` patch, but its guard matches only `TypedRequestError::Server`. This fork carries patch #1's reconnect path, which calls `fail_pending_requests()` on every websocket drop before reconnecting, so an in-flight `turn/start` routinely fails as `TypedRequestError::Transport` and would still kill the session. Extracts `is_recoverable_turn_start_failure` and widens it to `Transport` for `turn/start` only; every other method and every non-typed error stays fatal. Unit tests in `event_dispatch::tests`. Re-evaluate each release; drop once upstream's guard covers transport failures. |
| 11 | `chore: refresh workspace lock for 0.146.0-alpha.10.1` | chore | `codex-rs/Cargo.lock` | Regenerated every release (release tags bump `Cargo.toml` but ship a `0.0.0` lock; the first `cargo` run stamps the real version). Drop + recreate each upgrade. |

## History note — dropped patches

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

## Triage checklist (each upgrade)

1. `git log --oneline <newtag>..HEAD` — review the carried stack.
2. For each patch above: is the behavior + its tests now upstream? If yes, drop.
   If partially, rework against the new upstream shape. Else reapply.
3. Rebase with `--onto <newtag> <oldtag>` (tags are not linear ancestors).
4. Validate, rebuild, install, reconcile via the `upgrade-codex-fork` skill.
5. Tag `fork/<newversion>`; update this file's base + any status changes.
