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

## History note — dropped patches

- **MCP OAuth 401-recovery** (`harden local mcp oauth recovery`): **upstreamed
  in rust-v0.145.0** as the `rmcp-client/src/oauth/` module (refresh_lock,
  refresh_transaction, resolved_store, store_lock + recovery tests). Dropped
  from the stack — do not reintroduce the old monolithic `oauth.rs` patch.

## Triage checklist (each upgrade)

1. `git log --oneline <newtag>..HEAD` — review the carried stack.
2. For each patch above: is the behavior + its tests now upstream? If yes, drop.
   If partially, rework against the new upstream shape. Else reapply.
3. Rebase with `--onto <newtag> <oldtag>` (tags are not linear ancestors).
4. Validate, rebuild, install, reconcile via the `upgrade-codex-fork` skill.
5. Tag `fork/<newversion>`; update this file's base + any status changes.
