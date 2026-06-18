# MCP OAuth 401 Recovery Scope

## Problem

Long-running Streamable HTTP MCP clients can keep an OAuth-backed transport alive after the access token expires. When the MCP server returns a Bearer `invalid_token` challenge, Codex currently surfaces the 401 instead of refreshing credentials, recreating the transport, and retrying the operation.

This is especially visible with providers that intentionally issue short-lived access tokens and longer-lived refresh grants, such as Cloudflare Access Managed OAuth.

## Minimal Patch

Goal: prove the recovery approach locally with the smallest behavior change.

Scope:

- Recover only when the active Streamable HTTP client has OAuth state.
- Detect transport-level `AuthRequired` errors from a 401 `WWW-Authenticate` challenge.
- Force-refresh through the existing `AuthorizationManager`.
- Persist refreshed credentials through the existing `OAuthPersistor`.
- Recreate the Streamable HTTP transport through the existing session-recovery path.
- Retry the failed operation once.
- Preserve existing behavior for non-OAuth static bearer-token clients.

Acceptance checks:

- Existing 404 session-expiry recovery still passes.
- Existing static bearer-token 401 test still fails twice without recovery.
- New OAuth-backed 401 test proves Codex calls the token endpoint, recreates the transport, and retries successfully.

Non-goals:

- No changes to app-server protocol, UI, config shape, or credential-store format.
- No provider-specific parsing for Cloudflare, Notion, Sourcegraph, or other OAuth providers.
- No automatic user reauthorization flow when the refresh grant itself is invalid.

## Robust Upstream-Quality Patch

Goal: make OAuth-backed MCP recovery correct across startup, runtime expiry, concurrent sessions, and stale credential stores.

Scope:

- Refresh expired stored credentials before initial MCP initialize, so startup does not fail with an already-expired access token.
- Treat `401` with Bearer `invalid_token` as refreshable for OAuth-backed transports.
- Avoid refreshing on static bearer-token/header-auth clients unless they are explicitly OAuth-backed.
- Coordinate concurrent refresh attempts per MCP server so parallel tool calls do not rotate refresh tokens against each other.
- Reload persisted credentials before refresh when the in-memory refresh token fails, so another Codex process can update the credential chain without poisoning this session.
- On `invalid_grant`, delete or mark stale credentials and surface a clear reauth-required error instead of repeatedly retrying.
- Add structured logs for refresh attempt, refresh success, refresh failure, recovery reinitialize, and one-shot retry failure.
- Add tests for:
  - startup with expired access token and valid refresh token;
  - runtime 401 `invalid_token`;
  - non-OAuth 401 no recovery;
  - 403 `insufficient_scope` remains non-refresh recovery;
  - refresh-token rotation;
  - stale in-memory refresh token after external credential update;
  - `invalid_grant` requiring reauth;
  - concurrent OAuth-backed tool calls sharing one refresh.

Open design questions:

- Whether the recovery lock should be per `RmcpClient` only or global per credential key.
- Whether `AuthClient` should own more of this logic upstream in `rmcp`, or whether Codex should keep provider/session recovery in `codex-rs/rmcp-client`.
- Whether Codex should expose a user-facing MCP auth-recovery event in app-server surfaces, or keep it as internal transport behavior.
- Whether forced refresh should parse Bearer challenge errors explicitly or rely on `AuthRequired` plus active OAuth state.

Rollout plan:

1. Prove the minimal patch with deterministic local tests.
2. Test the patched local binary against the `agent_html` Cloudflare Access Managed OAuth server.
3. Expand tests to cover startup expiry and refresh-token rotation.
4. Prepare an upstream PR with the robust behavior and a smaller compatibility story.
