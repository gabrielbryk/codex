//! Resolves the configured MCP OAuth store and pins that concrete source for one client lifecycle.

use anyhow::Context;
use anyhow::Result;
use codex_config::types::AuthKeyringBackendKind;
use codex_config::types::OAuthCredentialsStoreMode;
use codex_keyring_store::KeyringStore;
use oauth2::TokenResponse;
use tracing::debug;
use tracing::warn;

use super::FallbackTokenEntry;
use super::OAuthKeyringLoadError;
use super::OAuthStore;
use super::OAuthStoreLock;
use super::OAuthStoreLockFailure;
use super::StoredOAuthTokens;
use super::compute_store_key;
use super::delete_oauth_tokens_from_direct_keyring;
use super::delete_oauth_tokens_from_file;
use super::delete_oauth_tokens_from_secrets_keyring;
use super::load_oauth_tokens_from_file;
use super::load_oauth_tokens_from_file_with_lock_held;
use super::load_oauth_tokens_from_keyring;
use super::load_oauth_tokens_from_secrets_keyring_with_lock_held;
use super::read_fallback_file_unlocked;
use super::save_oauth_tokens_to_file;
use super::save_oauth_tokens_with_keyring;
use super::write_fallback_file;

/// Concrete credential store resolved for one MCP OAuth client lifecycle.
///
/// This is intentionally not durable. `Auto` may resolve differently in a later process, but a
/// client that loaded credentials from one store must reread, refresh, persist, and remove only
/// through that store. A mid-lifecycle backend failure is unexpected and must return an error
/// rather than falling back to another possibly stale refresh token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResolvedOAuthCredentialStore {
    File,
    Keyring(AuthKeyringBackendKind),
}

impl ResolvedOAuthCredentialStore {
    /// Loads credentials only from this already-resolved authority.
    ///
    /// Unlike `resolve_oauth_tokens_from_store_policy`, this never evaluates configured
    /// `Auto` fallback policy.
    pub(crate) fn load<K: KeyringStore + Clone + 'static>(
        self,
        keyring_store: &K,
        server_name: &str,
        url: &str,
    ) -> Result<Option<StoredOAuthTokens>> {
        match self {
            Self::File => load_oauth_tokens_from_file(server_name, url)
                .context("failed to reread OAuth tokens from resolved file storage"),
            Self::Keyring(keyring_backend_kind) => load_oauth_tokens_from_keyring(
                keyring_store,
                keyring_backend_kind,
                server_name,
                url,
            )
            .map_err(anyhow::Error::from)
            .context(
                "failed to reread OAuth tokens from resolved keyring storage; refusing file fallback",
            ),
        }
    }

    /// Reads the selected authority without waiting for its aggregate-store lock.
    pub(crate) fn try_load<K: KeyringStore + Clone + 'static>(
        self,
        keyring_store: &K,
        server_name: &str,
        url: &str,
    ) -> Result<Option<StoredOAuthTokens>> {
        match self {
            Self::File => {
                let _store_lock = OAuthStoreLock::try_acquire_for_read(OAuthStore::File)?;
                load_oauth_tokens_from_file_with_lock_held(server_name, url)
                    .context("failed to probe OAuth tokens from resolved file storage")
            }
            Self::Keyring(AuthKeyringBackendKind::Direct) => {
                self.load(keyring_store, server_name, url)
            }
            Self::Keyring(AuthKeyringBackendKind::Secrets) => {
                let _store_lock = OAuthStoreLock::try_acquire_for_read(OAuthStore::Secrets)?;
                load_oauth_tokens_from_secrets_keyring_with_lock_held(
                    keyring_store,
                    server_name,
                    url,
                )
                .map_err(anyhow::Error::from)
            }
        }
    }

    /// Saves credentials only to this already-resolved authority.
    pub(crate) fn save<K: KeyringStore + Clone + 'static>(
        self,
        keyring_store: &K,
        server_name: &str,
        tokens: &StoredOAuthTokens,
    ) -> Result<()> {
        match self {
            Self::File => save_oauth_tokens_to_file(tokens),
            Self::Keyring(keyring_backend_kind) => save_oauth_tokens_with_keyring(
                keyring_store,
                keyring_backend_kind,
                server_name,
                tokens,
            ),
        }
    }

    /// Deletes credentials only from this already-resolved authority.
    pub(crate) fn delete<K: KeyringStore + Clone + 'static>(
        self,
        keyring_store: &K,
        server_name: &str,
        url: &str,
    ) -> Result<bool> {
        match self {
            Self::File => {
                let key = compute_store_key(server_name, url)?;
                delete_oauth_tokens_from_file(&key)
            }
            Self::Keyring(AuthKeyringBackendKind::Direct) => {
                delete_oauth_tokens_from_direct_keyring(keyring_store, server_name, url)
            }
            Self::Keyring(AuthKeyringBackendKind::Secrets) => {
                delete_oauth_tokens_from_secrets_keyring(keyring_store, server_name, url)
            }
        }
    }

    /// Deletes credentials only from this already-resolved authority, refusing to evict a File
    /// entry that still holds a usable refresh token or that no longer matches `expected` (the
    /// token last held in memory).
    ///
    /// This guards the `None` persist branch against a transient in-memory refresh miss wiping a
    /// still-valid on-disk refresh token. Keyring entries are stored per-credential and are not
    /// subject to that aggregate-store race, so their behavior matches [`Self::delete`].
    pub(crate) fn delete_if_stale<K: KeyringStore + Clone + 'static>(
        self,
        keyring_store: &K,
        server_name: &str,
        url: &str,
        expected: Option<&StoredOAuthTokens>,
    ) -> Result<bool> {
        match self {
            Self::File => {
                let key = compute_store_key(server_name, url)?;
                delete_oauth_tokens_from_file_if_stale(&key, expected)
            }
            Self::Keyring(AuthKeyringBackendKind::Direct) => {
                delete_oauth_tokens_from_direct_keyring(keyring_store, server_name, url)
            }
            Self::Keyring(AuthKeyringBackendKind::Secrets) => {
                delete_oauth_tokens_from_secrets_keyring(keyring_store, server_name, url)
            }
        }
    }
}

/// Deletes the fallback File entry for `key`, but only when it is genuinely safe to evict.
///
/// The `None` branch of `OAuthPersistor::persist_if_needed` (`oauth.rs`) fires whenever the
/// in-memory `AuthorizationManager` reports no credentials — including a transient refresh miss
/// right after a startup 401. Because `last_credentials` is seeded from disk (with the still-valid
/// refresh token), an unconditional delete would wipe a usable credential and force a full
/// re-login.
///
/// This performs an authoritative, lock-held reread and refuses to remove an on-disk entry that
/// either (a) still carries a usable refresh token, or (b) no longer matches the token we are
/// evicting (another process rotated it). Only a genuinely dead entry — no usable refresh token
/// and still matching the evicted token — is removed. The whole read-compare-delete runs under the
/// same `OAuthStoreLock(File)` used by the load/save/delete paths, preserving lock ordering.
pub(crate) fn delete_oauth_tokens_from_file_if_stale(
    key: &str,
    expected: Option<&StoredOAuthTokens>,
) -> Result<bool> {
    let _store_lock = OAuthStoreLock::acquire_for_write(OAuthStore::File)?;
    let mut store = match read_fallback_file_unlocked()? {
        Some(store) => store,
        None => return Ok(false),
    };

    // Mirror the host/executor collision guard `delete_oauth_tokens_from_file` applies. This
    // function is the delete path `persist_if_needed` actually takes once wired, so without the
    // same check an executor-keyed eviction could remove a host-owned credential — exactly what
    // upstream's fail-closed environment isolation is meant to prevent.
    if key.starts_with("executor:")
        && !key.contains('|')
        && store.get(key).is_some_and(|entry| !entry.executor_owned)
    {
        anyhow::bail!("executor OAuth credential key conflicts with a host-owned credential");
    }

    let Some(entry) = store.get(key) else {
        return Ok(false);
    };

    if entry
        .refresh_token
        .as_deref()
        .is_some_and(|token| !token.trim().is_empty())
    {
        debug!(
            server_name = %entry.server_name,
            "skipping MCP OAuth file credential delete because the on-disk entry still holds a usable refresh token"
        );
        return Ok(false);
    }

    if let Some(expected) = expected
        && !file_entry_matches_expected(entry, expected)
    {
        debug!(
            server_name = %entry.server_name,
            "skipping MCP OAuth file credential delete because the on-disk entry no longer matches the evicted token"
        );
        return Ok(false);
    }

    let removed = store.remove(key).is_some();
    if removed {
        write_fallback_file(&store)?;
    }
    Ok(removed)
}

/// Reports whether a persisted fallback entry still carries the exact token material we intend to
/// evict. Compares identity plus the access and refresh secrets; expiry/scope drift is ignored so a
/// re-serialized-but-equivalent credential still matches.
fn file_entry_matches_expected(entry: &FallbackTokenEntry, expected: &StoredOAuthTokens) -> bool {
    let expected_response = &expected.token_response.0;
    let expected_access = expected_response.access_token().secret().as_str();
    let expected_refresh = expected_response
        .refresh_token()
        .map(|token| token.secret().as_str());
    entry.server_name == expected.server_name
        && entry.server_url == expected.url
        && entry.client_id == expected.client_id
        && entry.access_token.as_str() == expected_access
        && entry.refresh_token.as_deref() == expected_refresh
}

#[derive(Debug)]
pub(crate) struct ResolvedOAuthTokens {
    pub(crate) tokens: StoredOAuthTokens,
    pub(crate) store: ResolvedOAuthCredentialStore,
}

pub(crate) fn resolve_oauth_tokens_from_store_policy<K: KeyringStore + Clone + 'static>(
    keyring_store: &K,
    server_name: &str,
    url: &str,
    store_mode: OAuthCredentialsStoreMode,
    keyring_backend_kind: AuthKeyringBackendKind,
) -> Result<Option<ResolvedOAuthTokens>> {
    match store_mode {
        OAuthCredentialsStoreMode::Auto => {
            // Auto remains keyring-first at lifecycle startup. The returned source is then pinned
            // by the client transport recipe and OAuth persistor so retries, recovery, and
            // refresh work cannot hot-switch stores.
            // TODO(stevenlee): Different processes can still resolve Auto to different stores
            // when keyring availability differs. Solving that safely requires durable backend
            // selection or reconciliation of legacy entries and is intentionally outside this
            // stack.
            match load_oauth_tokens_from_keyring(
                keyring_store,
                keyring_backend_kind,
                server_name,
                url,
            ) {
                Ok(Some(tokens)) => Ok(Some(ResolvedOAuthTokens {
                    tokens,
                    store: ResolvedOAuthCredentialStore::Keyring(keyring_backend_kind),
                })),
                Ok(None) => Ok(
                    load_oauth_tokens_from_file(server_name, url)?.map(|tokens| {
                        ResolvedOAuthTokens {
                            tokens,
                            store: ResolvedOAuthCredentialStore::File,
                        }
                    }),
                ),
                // Auto may fall back when the keyring backend is unavailable, but a Secrets
                // aggregate-lock failure means authority may be changing. Consulting File in
                // that state could replay credentials hidden behind a newer Secrets entry.
                Err(OAuthKeyringLoadError::StoreLock(error)) => Err(error.into()),
                Err(error) => {
                    warn!("failed to read OAuth tokens from keyring: {error}");
                    Ok(load_oauth_tokens_from_file(server_name, url)
                        .with_context(|| {
                            format!("failed to read OAuth tokens from keyring: {error}")
                        })?
                        .map(|tokens| ResolvedOAuthTokens {
                            tokens,
                            store: ResolvedOAuthCredentialStore::File,
                        }))
                }
            }
        }
        OAuthCredentialsStoreMode::File => Ok(load_oauth_tokens_from_file(server_name, url)?.map(
            |tokens| ResolvedOAuthTokens {
                tokens,
                store: ResolvedOAuthCredentialStore::File,
            },
        )),
        OAuthCredentialsStoreMode::Keyring => Ok(load_oauth_tokens_from_keyring(
            keyring_store,
            keyring_backend_kind,
            server_name,
            url,
        )
        .map_err(anyhow::Error::from)
        .context("failed to read OAuth tokens from keyring")?
        .map(|tokens| ResolvedOAuthTokens {
            tokens,
            store: ResolvedOAuthCredentialStore::Keyring(keyring_backend_kind),
        })),
    }
}

pub(crate) fn try_resolve_oauth_tokens_from_store_policy<K: KeyringStore + Clone + 'static>(
    keyring_store: &K,
    server_name: &str,
    url: &str,
    store_mode: OAuthCredentialsStoreMode,
    keyring_backend_kind: AuthKeyringBackendKind,
) -> Result<Option<ResolvedOAuthTokens>> {
    let load = |store: ResolvedOAuthCredentialStore| {
        store
            .try_load(keyring_store, server_name, url)
            .map(|tokens| tokens.map(|tokens| ResolvedOAuthTokens { tokens, store }))
    };
    let keyring = ResolvedOAuthCredentialStore::Keyring(keyring_backend_kind);
    match store_mode {
        OAuthCredentialsStoreMode::File => load(ResolvedOAuthCredentialStore::File),
        OAuthCredentialsStoreMode::Keyring => load(keyring),
        OAuthCredentialsStoreMode::Auto => match load(keyring) {
            Ok(Some(tokens)) => Ok(Some(tokens)),
            Ok(None) => load(ResolvedOAuthCredentialStore::File),
            Err(error) if error.downcast_ref::<OAuthStoreLockFailure>().is_some() => Err(error),
            Err(error) => {
                warn!("failed to read OAuth tokens from keyring: {error}");
                load(ResolvedOAuthCredentialStore::File)
                    .with_context(|| format!("failed to read OAuth tokens from keyring: {error}"))
            }
        },
    }
}

#[cfg(test)]
mod delete_if_stale_tests {
    use codex_keyring_store::tests::MockKeyringStore;
    use oauth2::AccessToken;
    use oauth2::RefreshToken;
    use oauth2::Scope;
    use oauth2::TokenResponse;
    use oauth2::basic::BasicTokenType;
    use rmcp::transport::auth::OAuthTokenResponse;
    use rmcp::transport::auth::VendorExtraTokenFields;

    use super::*;
    use crate::oauth::FallbackTokenEntry;
    use crate::oauth::WrappedOAuthTokenResponse;
    use crate::oauth::load_oauth_tokens_from_file;
    use crate::oauth::save_oauth_tokens_to_file;
    use crate::oauth::test_support::TempCodexHome;

    fn sample_tokens() -> StoredOAuthTokens {
        let mut response = OAuthTokenResponse::new(
            AccessToken::new("access-token".to_string()),
            BasicTokenType::Bearer,
            VendorExtraTokenFields::default(),
        );
        response.set_refresh_token(Some(RefreshToken::new("refresh-token".to_string())));
        response.set_scopes(Some(vec![
            Scope::new("scope-a".to_string()),
            Scope::new("scope-b".to_string()),
        ]));
        let expires_in = std::time::Duration::from_secs(3600);
        response.set_expires_in(Some(&expires_in));
        StoredOAuthTokens {
            server_name: "delete-if-stale-server".to_string(),
            url: "https://example.com/mcp".to_string(),
            issuer: None,
            client_id: "client".to_string(),
            token_response: WrappedOAuthTokenResponse(response),
            expires_at: Some(9_999_999_999_999),
        }
    }

    fn assert_tokens_match_without_expiry(
        actual: &StoredOAuthTokens,
        expected: &StoredOAuthTokens,
    ) {
        let actual_response = &actual.token_response.0;
        let expected_response = &expected.token_response.0;
        assert_eq!(actual.server_name, expected.server_name);
        assert_eq!(actual.url, expected.url);
        assert_eq!(actual.client_id, expected.client_id);
        assert_eq!(
            actual_response.access_token().secret(),
            expected_response.access_token().secret()
        );
        assert_eq!(
            actual_response.refresh_token().map(RefreshToken::secret),
            expected_response.refresh_token().map(RefreshToken::secret),
        );
    }

    #[test]
    fn delete_if_stale_keeps_entry_with_valid_refresh_token() -> Result<()> {
        let _env = TempCodexHome::new();
        let store = MockKeyringStore::default();
        // The evicted in-memory snapshot and the on-disk entry share the same still-valid refresh
        // token, mirroring a transient refresh miss after a startup 401.
        let tokens = sample_tokens();
        save_oauth_tokens_to_file(&tokens)?;

        let removed = ResolvedOAuthCredentialStore::File.delete_if_stale(
            &store,
            &tokens.server_name,
            &tokens.url,
            Some(&tokens),
        )?;

        assert!(
            !removed,
            "entry with a usable refresh token must not be deleted"
        );
        let loaded = load_oauth_tokens_from_file(&tokens.server_name, &tokens.url)?
            .expect("credential with a valid refresh token must remain on disk");
        assert_tokens_match_without_expiry(&loaded, &tokens);
        Ok(())
    }

    #[test]
    fn delete_if_stale_removes_dead_matching_entry() -> Result<()> {
        let _env = TempCodexHome::new();
        let store = MockKeyringStore::default();
        // A genuinely dead credential: expired with no refresh token, matching what we evicted.
        let mut dead = sample_tokens();
        dead.token_response.0.set_refresh_token(None);
        dead.expires_at = Some(0);
        save_oauth_tokens_to_file(&dead)?;

        let removed = ResolvedOAuthCredentialStore::File.delete_if_stale(
            &store,
            &dead.server_name,
            &dead.url,
            Some(&dead),
        )?;

        assert!(
            removed,
            "a dead entry with no usable refresh token must still be removed"
        );
        assert!(
            load_oauth_tokens_from_file(&dead.server_name, &dead.url)?.is_none(),
            "dead credential should be gone from disk"
        );
        Ok(())
    }

    #[test]
    fn delete_if_stale_keeps_entry_that_no_longer_matches_evicted_token() -> Result<()> {
        let _env = TempCodexHome::new();
        let store = MockKeyringStore::default();
        // Both the evicted snapshot and the on-disk entry are unrefreshable, but another process
        // rotated the on-disk access token, so the entry no longer matches and must be preserved.
        let mut evicted = sample_tokens();
        evicted.token_response.0.set_refresh_token(None);
        evicted.expires_at = Some(0);

        let mut on_disk = evicted.clone();
        on_disk
            .token_response
            .0
            .set_access_token(AccessToken::new("rotated-access-token".to_string()));
        save_oauth_tokens_to_file(&on_disk)?;

        let removed = ResolvedOAuthCredentialStore::File.delete_if_stale(
            &store,
            &evicted.server_name,
            &evicted.url,
            Some(&evicted),
        )?;

        assert!(
            !removed,
            "entry that no longer matches the evicted token must not be deleted"
        );
        assert!(
            load_oauth_tokens_from_file(&on_disk.server_name, &on_disk.url)?.is_some(),
            "rotated credential should remain on disk"
        );
        Ok(())
    }

    #[test]
    fn delete_if_stale_refuses_executor_key_colliding_with_host_credential() -> Result<()> {
        let _env = TempCodexHome::new();
        // A legacy host entry (no `executor_owned` marker) parked under an executor-shaped key,
        // and otherwise perfectly deletable: no refresh token left to protect it. Only the
        // host/executor collision guard should stop the eviction.
        let url = "https://example.com/mcp";
        let key = compute_store_key("executor:colliding-server", url)?;
        let mut store = std::collections::BTreeMap::new();
        store.insert(
            key.clone(),
            FallbackTokenEntry {
                server_name: "executor:colliding-server".to_string(),
                server_url: url.to_string(),
                issuer: None,
                client_id: "client".to_string(),
                access_token: "access".to_string(),
                expires_at: Some(0),
                refresh_token: None,
                scopes: Vec::new(),
                executor_owned: false,
            },
        );
        write_fallback_file(&store)?;

        let error = delete_oauth_tokens_from_file_if_stale(&key, None)
            .expect_err("executor-keyed delete must fail closed against a host-owned entry");
        assert!(
            error.to_string().contains("conflicts with a host-owned"),
            "unexpected error: {error}"
        );
        assert!(
            read_fallback_file_unlocked()?.is_some_and(|store| store.contains_key(&key)),
            "host-owned credential must survive a refused executor-keyed delete"
        );
        Ok(())
    }
}
