# Fork patch manifest

The `gabe/fork` branch is a patch stack rebased onto an upstream `rust-v*`
release tag. Ordered **bottom → top**: upstreamable fixes first, local-env
patches next, then defensive, then the regenerated lock chore last. Keep each
commit atomic (one fix + its tests) so it can be dropped when upstream covers
it, or `format-patch`ed to a PR.

Current base: **rust-v0.145.0**.

| # | Commit subject | Category | Files | Upstream status |
|---|---|---|---|---|
| 1 | `fix: reconnect remote app-server client` | upstreamable | `app-server-client/{lib,remote}.rs` | Fork-only; general resilience — candidate to PR upstream. |
| 2 | `fix: import Claude history from last compact boundary` | upstreamable | `external-agent-migration/src/sessions/records.rs` | Fork-only; general feature — candidate to PR upstream. |
| 3 | `fix(config): isolate alternate Codex homes` | local-env | `config/src/loader/{mod,tests}.rs` | Fork-only forever — specific to the dual `.codex` / `.codex-uprising` setup. |
| 4 | `fix(uds): accept sticky rendezvous directories` | local-env | `uds/src/{lib,lib_tests}.rs` | Fork-only forever — specific to the shared sticky `/tmp` socket dir. |
| 5 | `fix(app-server-daemon): disable stock auto-updater for local fork` | local-env | `app-server-daemon/src/lib.rs` | Fork-only forever — the managed-fork deploy must not let the stock updater snap `standalone/current` back to upstream stock. |
| 6 | `fix(rmcp-client): classify invalid_grant startup errors as reauth` | defensive | `rmcp-client/src/startup_error.rs` | Fork-only; largely redundant since 0.145.0's `refresh_transaction` maps `invalid_grant`→`AuthorizationRequired`. Re-evaluate each release; drop if a raw path can no longer surface it. |
| 7 | `chore: refresh workspace lock for 0.145.0` | chore | `codex-rs/Cargo.lock` | Regenerated every release (release tags ship `0.0.0`; first `cargo` run stamps the real version). Drop + recreate each upgrade. |
| 8 | `fix(rmcp-client): harden MCP OAuth reauth (delete guard + reactive 401 recovery)` | defensive | `rmcp-client/src/oauth.rs`, `rmcp-client/src/oauth/resolved_store.rs`, `rmcp-client/src/oauth/refresh_transaction.rs`, `rmcp-client/src/rmcp_client.rs` | Fork-only; strong candidate to PR upstream. One coherent "harden MCP OAuth reauth" patch closing two holes 0.145.0's oauth rewrite left. **(a) Compare-and-delete guard:** `persist_if_needed`'s `None` branch deleted the resolved-store entry unconditionally whenever the in-memory `AuthorizationManager` reported no credentials — including a transient refresh miss after a startup 401 — wiping a still-valid on-disk refresh token and forcing a full re-login. Adds `ResolvedOAuthCredentialStore::delete_if_stale` + `delete_oauth_tokens_from_file_if_stale`: a lock-held authoritative reread that refuses to evict a File entry still holding a usable refresh token or one that no longer matches the evicted token. **(b) Reactive 401 recovery:** 0.145.0 replaced the old patch's live-401 recovery with proactive expiry-only pre-refresh, so a runtime `401 AuthRequired` in `run_service_operation` fell straight through to `Err` — no refresh, no retry — and expiry-gated pre-refresh can't cover `expires_at == None` (`token_needs_refresh` returns false), early revocation, or clock skew. Adds `is_auth_required_401` + a single-retry recovery arm mirroring the `is_session_expired_404` path, driven by `OAuthPersistor::refresh_after_unauthorized` (a `RefreshTrigger::Unauthorized` transaction that forces the authoritative locked reread / adopt-newer / fail-closed refresh regardless of cached expiry, reusing guard (a) so a failed refresh can't wipe a valid token). Scoped to the File store (this deploy's `mcp_oauth_credentials_store = "file"`); keyring paths unchanged. Re-evaluate each release; drop once upstream's `None` branch stops deleting unconditionally AND upstream recovers live 401s. |

## History note — dropped patches

- **MCP OAuth 401-recovery** (`harden local mcp oauth recovery`): **mostly
  upstreamed in rust-v0.145.0** as the `rmcp-client/src/oauth/` module
  (refresh_lock, refresh_transaction, resolved_store, store_lock + recovery
  tests). Dropped from the stack — do not reintroduce the old monolithic
  `oauth.rs` patch. **Caveat — two pieces the rewrite dropped, both reapplied as
  patch #9 above, reworked to the new module shape (not the old monolith):**
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

## Triage checklist (each upgrade)

1. `git log --oneline <newtag>..HEAD` — review the carried stack.
2. For each patch above: is the behavior + its tests now upstream? If yes, drop.
   If partially, rework against the new upstream shape. Else reapply.
3. Rebase with `--onto <newtag> <oldtag>` (tags are not linear ancestors).
4. Validate, rebuild, install, reconcile via the `upgrade-codex-fork` skill.
5. Tag `fork/<newversion>`; update this file's base + any status changes.
