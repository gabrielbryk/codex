//! This file handles all logic related to managing MCP OAuth credentials.
//! All credentials are stored using the keyring crate which uses os-specific keyring services.
//! https://crates.io/crates/keyring
//! macOS: macOS keychain.
//! Windows: Windows Credential Manager
//! Linux: DBus-based Secret Service, the kernel keyutils, and a combo of the two
//! FreeBSD, OpenBSD: DBus-based Secret Service
//!
//! For Linux, we use linux-native-async-persistent which uses both keyutils and async-secret-service (see below) for storage.
//! See the docs for the keyutils_persistent module for a full explanation of why both are used. Because this store uses the
//! async-secret-service, you must specify the additional features required by that store
//!
//! async-secret-service provides access to the DBus-based Secret Service storage on Linux, FreeBSD, and OpenBSD. This is an asynchronous
//! keystore that always encrypts secrets when they are transferred across the bus. If DBus isn't installed the keystore will fall back to the json
//! file because we don't use the "vendored" feature.
//!
//! If the keyring is not available or fails, we fall back to CODEX_HOME/.credentials.json which is consistent with other coding CLI agents.

mod store_lock;

#[cfg(test)]
#[path = "oauth/test_support.rs"]
mod test_support;

use anyhow::Context;
use anyhow::Error;
use anyhow::Result;
use codex_config::types::AuthKeyringBackendKind;
use codex_config::types::OAuthCredentialsStoreMode;
use codex_secrets::LocalSecretsNamespace;
use codex_secrets::SecretName;
use codex_secrets::SecretScope;
use codex_secrets::SecretsBackendKind;
use codex_secrets::SecretsManager;
use oauth2::AccessToken;
use oauth2::RefreshToken;
use oauth2::Scope;
use oauth2::TokenResponse;
use oauth2::basic::BasicTokenType;
use rmcp::transport::auth::OAuthTokenResponse;
use rmcp::transport::auth::VendorExtraTokenFields;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use serde_json::map::Map as JsonMap;
use sha2::Digest;
use sha2::Sha256;
use std::collections::BTreeMap;
use std::fs;
#[cfg(unix)]
use std::fs::OpenOptions;
use std::io::ErrorKind;
#[cfg(unix)]
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;
use tracing::warn;

use self::store_lock::OAuthStore;
use self::store_lock::OAuthStoreLock;
use self::store_lock::OAuthStoreLockFailure;

use codex_keyring_store::DefaultKeyringStore;
use codex_keyring_store::KeyringStore;
use rmcp::transport::auth::AuthError;
use rmcp::transport::auth::AuthorizationManager;
use rmcp::transport::auth::OAuthState;
use tokio::sync::Mutex;
use tokio::sync::MutexGuard as TokioMutexGuard;
use tracing::info;

use crate::oauth_http_client::OAuthHttpClientAdapter;
use codex_utils_home_dir::find_codex_home;

const KEYRING_SERVICE: &str = "Codex MCP Credentials";
const MCP_OAUTH_SECRET_PREFIX: &str = "MCP_OAUTH";
const REFRESH_SKEW_MILLIS: u64 = 30_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StoredOAuthTokens {
    pub server_name: String,
    pub url: String,
    pub client_id: String,
    pub token_response: WrappedOAuthTokenResponse,
    #[serde(default)]
    pub expires_at: Option<u64>,
}

/// Wrap OAuthTokenResponse to allow for partial equality comparison.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WrappedOAuthTokenResponse(pub OAuthTokenResponse);

impl PartialEq for WrappedOAuthTokenResponse {
    fn eq(&self, other: &Self) -> bool {
        match (serde_json::to_string(self), serde_json::to_string(other)) {
            (Ok(s1), Ok(s2)) => s1 == s2,
            _ => false,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum StoredOAuthTokenStatus {
    Missing,
    Usable,
    AuthorizationRequired,
}

pub(crate) fn load_oauth_tokens(
    server_name: &str,
    url: &str,
    store_mode: OAuthCredentialsStoreMode,
    keyring_backend_kind: AuthKeyringBackendKind,
) -> Result<Option<StoredOAuthTokens>> {
    let keyring_store = DefaultKeyringStore;
    match store_mode {
        OAuthCredentialsStoreMode::Auto => load_oauth_tokens_from_keyring_with_fallback_to_file(
            &keyring_store,
            keyring_backend_kind,
            server_name,
            url,
        ),
        OAuthCredentialsStoreMode::File => load_oauth_tokens_from_file(server_name, url),
        OAuthCredentialsStoreMode::Keyring => {
            load_oauth_tokens_from_keyring(&keyring_store, keyring_backend_kind, server_name, url)
                .with_context(|| "failed to read OAuth tokens from keyring".to_string())
        }
    }
}

pub(crate) fn oauth_token_status(
    server_name: &str,
    url: &str,
    store_mode: OAuthCredentialsStoreMode,
    keyring_backend_kind: AuthKeyringBackendKind,
) -> Result<StoredOAuthTokenStatus> {
    Ok(
        match load_oauth_tokens(server_name, url, store_mode, keyring_backend_kind)?.as_ref() {
            None => StoredOAuthTokenStatus::Missing,
            Some(tokens) if oauth_tokens_are_usable(tokens) => StoredOAuthTokenStatus::Usable,
            Some(_) => StoredOAuthTokenStatus::AuthorizationRequired,
        },
    )
}

fn oauth_tokens_are_usable(tokens: &StoredOAuthTokens) -> bool {
    if tokens.client_id.trim().is_empty() {
        return false;
    }

    let token_response = &tokens.token_response.0;
    if token_needs_refresh(tokens.expires_at) {
        return token_response
            .refresh_token()
            .is_some_and(|token| !token.secret().trim().is_empty());
    }

    !token_response.access_token().secret().trim().is_empty()
}

fn refresh_expires_in_from_timestamp(tokens: &mut StoredOAuthTokens) {
    let Some(expires_at) = tokens.expires_at else {
        return;
    };

    match expires_in_from_timestamp(expires_at) {
        Some(seconds) => {
            let duration = Duration::from_secs(seconds);
            tokens.token_response.0.set_expires_in(Some(&duration));
        }
        None => {
            // RMCP treats a missing expiry as unknown and uses the access token
            // as-is. Treat a known-expired timestamp as an explicit zero so
            // startup refreshes the token before the first request.
            tokens
                .token_response
                .0
                .set_expires_in(Some(&Duration::ZERO));
        }
    }
}

fn load_oauth_tokens_from_keyring_with_fallback_to_file<K: KeyringStore + Clone + 'static>(
    keyring_store: &K,
    keyring_backend_kind: AuthKeyringBackendKind,
    server_name: &str,
    url: &str,
) -> Result<Option<StoredOAuthTokens>> {
    match load_oauth_tokens_from_keyring(keyring_store, keyring_backend_kind, server_name, url) {
        Ok(Some(tokens)) => Ok(Some(tokens)),
        Ok(None) => load_oauth_tokens_from_file(server_name, url),
        // A store lock failure means the configured aggregate authority could be changing, or
        // that coordination itself is unavailable. It is not evidence that the keyring backend
        // is unavailable, so consulting File here could replay credentials hidden behind a
        // newer Secrets entry. This is the load-side counterpart of the save guard below.
        Err(error) if error.downcast_ref::<OAuthStoreLockFailure>().is_some() => Err(error),
        Err(error) => {
            warn!("failed to read OAuth tokens from keyring: {error}");
            load_oauth_tokens_from_file(server_name, url)
                .with_context(|| format!("failed to read OAuth tokens from keyring: {error}"))
        }
    }
}

fn load_oauth_tokens_from_keyring<K: KeyringStore + Clone + 'static>(
    keyring_store: &K,
    keyring_backend_kind: AuthKeyringBackendKind,
    server_name: &str,
    url: &str,
) -> Result<Option<StoredOAuthTokens>> {
    match keyring_backend_kind {
        AuthKeyringBackendKind::Direct => {
            load_oauth_tokens_from_direct_keyring(keyring_store, server_name, url)
        }
        AuthKeyringBackendKind::Secrets => {
            load_oauth_tokens_from_secrets_keyring(keyring_store, server_name, url)
        }
    }
}

fn load_oauth_tokens_from_direct_keyring<K: KeyringStore>(
    keyring_store: &K,
    server_name: &str,
    url: &str,
) -> Result<Option<StoredOAuthTokens>> {
    let key = compute_store_key(server_name, url)?;
    match keyring_store.load(KEYRING_SERVICE, &key) {
        Ok(Some(serialized)) => {
            let mut tokens: StoredOAuthTokens = serde_json::from_str(&serialized)
                .context("failed to deserialize OAuth tokens from keyring")?;
            refresh_expires_in_from_timestamp(&mut tokens);
            Ok(Some(tokens))
        }
        Ok(None) => Ok(None),
        Err(error) => Err(Error::new(error.into_error())),
    }
}

fn load_oauth_tokens_from_secrets_keyring<K: KeyringStore + Clone + 'static>(
    keyring_store: &K,
    server_name: &str,
    url: &str,
) -> Result<Option<StoredOAuthTokens>> {
    let _store_lock = OAuthStoreLock::acquire(OAuthStore::Secrets)?;
    let codex_home = find_codex_home()?;
    let manager = SecretsManager::new_with_keyring_store_and_namespace(
        codex_home.to_path_buf(),
        SecretsBackendKind::Local,
        Arc::new(keyring_store.clone()),
        LocalSecretsNamespace::McpOAuth,
    );
    let secret_name = compute_secret_name(server_name, url)?;
    match manager
        .get(&SecretScope::Global, &secret_name)
        .context("failed to load MCP OAuth tokens from encrypted storage")?
    {
        Some(serialized) => {
            let mut tokens: StoredOAuthTokens = serde_json::from_str(&serialized)
                .context("failed to deserialize OAuth tokens from encrypted storage")?;
            refresh_expires_in_from_timestamp(&mut tokens);
            Ok(Some(tokens))
        }
        None => Ok(None),
    }
}

pub fn save_oauth_tokens(
    server_name: &str,
    tokens: &StoredOAuthTokens,
    store_mode: OAuthCredentialsStoreMode,
    keyring_backend_kind: AuthKeyringBackendKind,
) -> Result<()> {
    let keyring_store = DefaultKeyringStore;
    match store_mode {
        OAuthCredentialsStoreMode::Auto => save_oauth_tokens_with_keyring_with_fallback_to_file(
            &keyring_store,
            keyring_backend_kind,
            server_name,
            tokens,
        ),
        OAuthCredentialsStoreMode::File => save_oauth_tokens_to_file(tokens),
        OAuthCredentialsStoreMode::Keyring => save_oauth_tokens_with_keyring(
            &keyring_store,
            keyring_backend_kind,
            server_name,
            tokens,
        ),
    }
}

fn save_oauth_tokens_with_keyring<K: KeyringStore + Clone + 'static>(
    keyring_store: &K,
    keyring_backend_kind: AuthKeyringBackendKind,
    server_name: &str,
    tokens: &StoredOAuthTokens,
) -> Result<()> {
    match keyring_backend_kind {
        AuthKeyringBackendKind::Direct => {
            save_oauth_tokens_to_direct_keyring(keyring_store, server_name, tokens)
        }
        AuthKeyringBackendKind::Secrets => {
            save_oauth_tokens_to_secrets_keyring(keyring_store, server_name, tokens)
        }
    }
}

fn save_oauth_tokens_to_direct_keyring<K: KeyringStore>(
    keyring_store: &K,
    server_name: &str,
    tokens: &StoredOAuthTokens,
) -> Result<()> {
    let serialized = serde_json::to_string(tokens).context("failed to serialize OAuth tokens")?;

    let key = compute_store_key(server_name, &tokens.url)?;
    match keyring_store.save(KEYRING_SERVICE, &key, &serialized) {
        Ok(()) => {
            if let Err(error) = delete_oauth_tokens_from_file(&key) {
                warn!("failed to remove OAuth tokens from fallback storage: {error:?}");
            }
            Ok(())
        }
        Err(error) => {
            let message = format!(
                "failed to write OAuth tokens to keyring: {}",
                error.message()
            );
            warn!("{message}");
            Err(Error::new(error.into_error()).context(message))
        }
    }
}

/// Saves one credential while holding the Secrets aggregate-store lock across the mutation.
///
/// The Secrets lock is released before fallback File cleanup to preserve aggregate-lock ordering.
fn save_oauth_tokens_to_secrets_keyring<K: KeyringStore + Clone + 'static>(
    keyring_store: &K,
    server_name: &str,
    tokens: &StoredOAuthTokens,
) -> Result<()> {
    let serialized = serde_json::to_string(tokens).context("failed to serialize OAuth tokens")?;
    {
        let _store_lock = OAuthStoreLock::acquire(OAuthStore::Secrets)?;
        save_oauth_tokens_to_secrets_keyring_with_lock_held(
            keyring_store,
            server_name,
            tokens,
            &serialized,
        )?;
    }

    let key = compute_store_key(server_name, &tokens.url)?;
    if let Err(error) = delete_oauth_tokens_from_file(&key) {
        warn!("failed to remove OAuth tokens from fallback storage: {error:?}");
    }
    Ok(())
}

/// Writes one credential to Secrets. The caller must hold the Secrets aggregate-store lock.
fn save_oauth_tokens_to_secrets_keyring_with_lock_held<K: KeyringStore + Clone + 'static>(
    keyring_store: &K,
    server_name: &str,
    tokens: &StoredOAuthTokens,
    serialized: &str,
) -> Result<()> {
    let codex_home = find_codex_home()?;
    let manager = SecretsManager::new_with_keyring_store_and_namespace(
        codex_home.to_path_buf(),
        SecretsBackendKind::Local,
        Arc::new(keyring_store.clone()),
        LocalSecretsNamespace::McpOAuth,
    );
    let secret_name = compute_secret_name(server_name, &tokens.url)?;
    manager
        .set(&SecretScope::Global, &secret_name, serialized)
        .context("failed to write OAuth tokens to encrypted storage")
}

fn save_oauth_tokens_with_keyring_with_fallback_to_file<K: KeyringStore + Clone + 'static>(
    keyring_store: &K,
    keyring_backend_kind: AuthKeyringBackendKind,
    server_name: &str,
    tokens: &StoredOAuthTokens,
) -> Result<()> {
    match save_oauth_tokens_with_keyring(keyring_store, keyring_backend_kind, server_name, tokens) {
        Ok(()) => Ok(()),
        // As on load, a store lock failure is a coordination failure rather than evidence that
        // the keyring backend is unavailable. Falling back could leave a newer File token hidden
        // behind a stale Secrets entry.
        Err(error) if error.downcast_ref::<OAuthStoreLockFailure>().is_some() => Err(error),
        Err(error) => {
            let message = error.to_string();
            warn!("falling back to file storage for OAuth tokens: {message}");
            save_oauth_tokens_to_file(tokens)
                .with_context(|| format!("failed to write OAuth tokens to keyring: {message}"))
        }
    }
}

pub fn delete_oauth_tokens(
    server_name: &str,
    url: &str,
    store_mode: OAuthCredentialsStoreMode,
    keyring_backend_kind: AuthKeyringBackendKind,
) -> Result<bool> {
    let keyring_store = DefaultKeyringStore;
    delete_oauth_tokens_from_keyring_and_file(
        &keyring_store,
        store_mode,
        keyring_backend_kind,
        server_name,
        url,
    )
}

fn delete_oauth_tokens_from_keyring_and_file<K: KeyringStore + Clone + 'static>(
    keyring_store: &K,
    store_mode: OAuthCredentialsStoreMode,
    keyring_backend_kind: AuthKeyringBackendKind,
    server_name: &str,
    url: &str,
) -> Result<bool> {
    let key = compute_store_key(server_name, url)?;
    let keyring_result =
        delete_oauth_tokens_from_keyring(keyring_store, keyring_backend_kind, server_name, url);
    let keyring_removed = match keyring_result {
        Ok(removed) => removed,
        Err(error) => {
            let message = error.to_string();
            warn!("failed to delete OAuth tokens from keyring: {message}");
            match store_mode {
                OAuthCredentialsStoreMode::Auto | OAuthCredentialsStoreMode::Keyring => {
                    return Err(error).context("failed to delete OAuth tokens from keyring");
                }
                OAuthCredentialsStoreMode::File => false,
            }
        }
    };

    let file_removed = delete_oauth_tokens_from_file(&key)?;
    Ok(keyring_removed || file_removed)
}

fn delete_oauth_tokens_from_keyring<K: KeyringStore + Clone + 'static>(
    keyring_store: &K,
    keyring_backend_kind: AuthKeyringBackendKind,
    server_name: &str,
    url: &str,
) -> Result<bool> {
    match keyring_backend_kind {
        AuthKeyringBackendKind::Direct => {
            delete_oauth_tokens_from_direct_keyring(keyring_store, server_name, url)
        }
        AuthKeyringBackendKind::Secrets => {
            let direct_removed =
                delete_oauth_tokens_from_direct_keyring(keyring_store, server_name, url)?;
            let secrets_removed =
                delete_oauth_tokens_from_secrets_keyring(keyring_store, server_name, url)?;
            Ok(direct_removed || secrets_removed)
        }
    }
}

fn delete_oauth_tokens_from_direct_keyring<K: KeyringStore>(
    keyring_store: &K,
    server_name: &str,
    url: &str,
) -> Result<bool> {
    let key = compute_store_key(server_name, url)?;
    keyring_store
        .delete(KEYRING_SERVICE, &key)
        .map_err(|error| Error::new(error.into_error()))
}

fn delete_oauth_tokens_from_secrets_keyring<K: KeyringStore + Clone + 'static>(
    keyring_store: &K,
    server_name: &str,
    url: &str,
) -> Result<bool> {
    let _store_lock = OAuthStoreLock::acquire(OAuthStore::Secrets)?;
    let codex_home = find_codex_home()?;
    let manager = SecretsManager::new_with_keyring_store_and_namespace(
        codex_home.to_path_buf(),
        SecretsBackendKind::Local,
        Arc::new(keyring_store.clone()),
        LocalSecretsNamespace::McpOAuth,
    );
    let secret_name = compute_secret_name(server_name, url)?;
    let secrets_removed = manager
        .delete(&SecretScope::Global, &secret_name)
        .context("failed to delete OAuth tokens from encrypted storage")?;
    Ok(secrets_removed)
}

fn delete_oauth_tokens_if_match(
    server_name: &str,
    url: &str,
    store_mode: OAuthCredentialsStoreMode,
    keyring_backend_kind: AuthKeyringBackendKind,
    expected_tokens: Option<&StoredOAuthTokens>,
) -> Result<bool> {
    let keyring_store = DefaultKeyringStore;
    delete_oauth_tokens_from_keyring_and_file_if_match(
        &keyring_store,
        store_mode,
        keyring_backend_kind,
        server_name,
        url,
        expected_tokens,
    )
}

fn delete_oauth_tokens_from_keyring_and_file_if_match<K: KeyringStore + Clone + 'static>(
    keyring_store: &K,
    store_mode: OAuthCredentialsStoreMode,
    keyring_backend_kind: AuthKeyringBackendKind,
    server_name: &str,
    url: &str,
    expected_tokens: Option<&StoredOAuthTokens>,
) -> Result<bool> {
    let key = compute_store_key(server_name, url)?;
    let keyring_removed = match store_mode {
        OAuthCredentialsStoreMode::File => false,
        OAuthCredentialsStoreMode::Auto | OAuthCredentialsStoreMode::Keyring => {
            let current = load_oauth_tokens_from_keyring(
                keyring_store,
                keyring_backend_kind,
                server_name,
                url,
            )?;
            if let (Some(current), Some(expected)) = (current.as_ref(), expected_tokens)
                && !same_oauth_token_material(current, expected)
            {
                warn!(
                    server_name = %server_name,
                    "skipping MCP OAuth keyring credential delete because stored token changed"
                );
                false
            } else {
                delete_oauth_tokens_from_keyring(
                    keyring_store,
                    keyring_backend_kind,
                    server_name,
                    url,
                )
                .map_err(|error| {
                    warn!("failed to delete OAuth tokens from keyring: {error}");
                    error.context("failed to delete OAuth tokens from keyring")
                })?
            }
        }
    };

    let file_removed = delete_oauth_tokens_from_file_if_match(&key, expected_tokens)?;
    Ok(keyring_removed || file_removed)
}

#[derive(Clone)]
pub(crate) struct OAuthPersistor {
    inner: Arc<OAuthPersistorInner>,
}

struct OAuthPersistorInner {
    server_name: String,
    url: String,
    authorization_manager: Arc<Mutex<AuthorizationManager>>,
    store_mode: OAuthCredentialsStoreMode,
    keyring_backend_kind: AuthKeyringBackendKind,
    last_credentials: Mutex<Option<StoredOAuthTokens>>,
    oauth_http_client: Arc<OAuthHttpClientAdapter>,
    refresh_lock_key: String,
    refresh_lock: Arc<Mutex<()>>,
}

impl OAuthPersistor {
    pub(crate) fn new(
        server_name: String,
        url: String,
        authorization_manager: Arc<Mutex<AuthorizationManager>>,
        store_mode: OAuthCredentialsStoreMode,
        keyring_backend_kind: AuthKeyringBackendKind,
        initial_credentials: Option<StoredOAuthTokens>,
        oauth_http_client: Arc<OAuthHttpClientAdapter>,
    ) -> Self {
        let refresh_lock_key = compute_store_key(&server_name, &url).unwrap_or_else(|error| {
            warn!("failed to compute OAuth refresh lock key for server {server_name}: {error}");
            format!("{server_name}|{url}")
        });
        let refresh_lock = refresh_lock_for_key(&refresh_lock_key);
        Self {
            inner: Arc::new(OAuthPersistorInner {
                server_name,
                url,
                authorization_manager,
                store_mode,
                keyring_backend_kind,
                last_credentials: Mutex::new(initial_credentials),
                oauth_http_client,
                refresh_lock_key,
                refresh_lock,
            }),
        }
    }

    /// Persists the latest stored credentials if they have changed.
    /// Deletes the credentials if they are no longer present.
    #[expect(
        clippy::await_holding_invalid_type,
        reason = "AuthorizationManager async access must be serialized through its mutex"
    )]
    pub(crate) async fn persist_if_needed(&self) -> Result<()> {
        let (client_id, maybe_credentials) = {
            let manager = self.inner.authorization_manager.clone();
            let guard = manager.lock().await;
            guard.get_credentials().await
        }?;

        match maybe_credentials {
            Some(credentials) => {
                let mut last_credentials = self.inner.last_credentials.lock().await;
                let new_token_response = WrappedOAuthTokenResponse(credentials.clone());
                let same_token = last_credentials
                    .as_ref()
                    .map(|prev| prev.token_response == new_token_response)
                    .unwrap_or(false);
                let expires_at = if same_token {
                    last_credentials.as_ref().and_then(|prev| prev.expires_at)
                } else {
                    compute_expires_at_millis(&credentials)
                };
                let stored = StoredOAuthTokens {
                    server_name: self.inner.server_name.clone(),
                    url: self.inner.url.clone(),
                    client_id,
                    token_response: new_token_response,
                    expires_at,
                };
                if last_credentials.as_ref() != Some(&stored) {
                    save_oauth_tokens(
                        &self.inner.server_name,
                        &stored,
                        self.inner.store_mode,
                        self.inner.keyring_backend_kind,
                    )?;
                    *last_credentials = Some(stored);
                }
            }
            None => {
                let mut last_serialized = self.inner.last_credentials.lock().await;
                if last_serialized.take().is_some()
                    && let Err(error) = delete_oauth_tokens(
                        &self.inner.server_name,
                        &self.inner.url,
                        self.inner.store_mode,
                        self.inner.keyring_backend_kind,
                    )
                {
                    warn!(
                        "failed to remove OAuth tokens for server {}: {error}",
                        self.inner.server_name
                    );
                }
            }
        }

        Ok(())
    }

    pub(crate) async fn refresh_if_needed(&self) -> Result<()> {
        let _refresh_guard = self.lock_refresh().await?;
        let expires_at = {
            let guard = self.inner.last_credentials.lock().await;
            guard.as_ref().and_then(|tokens| tokens.expires_at)
        };

        if !token_needs_refresh(expires_at) {
            return Ok(());
        }

        if let PersistedCredentialReloadStatus::Fresh =
            self.reload_persisted_credentials_status().await?
        {
            return Ok(());
        }

        self.refresh_locked().await
    }

    pub(crate) async fn refresh(&self) -> Result<()> {
        let _refresh_guard = self.lock_refresh().await?;
        if let PersistedCredentialReloadStatus::Fresh =
            self.reload_persisted_credentials_status().await?
        {
            return Ok(());
        }
        self.refresh_locked().await
    }

    async fn lock_refresh(&self) -> Result<RefreshLockGuard<'_>> {
        info!(
            server_name = %self.inner.server_name,
            lock_key = %self.inner.refresh_lock_key,
            "acquiring MCP OAuth refresh lock"
        );
        let file_guard = CrossProcessRefreshLockGuard::acquire(
            self.inner.refresh_lock_key.clone(),
            self.inner.server_name.clone(),
        )
        .await?;
        let process_guard = self.inner.refresh_lock.lock().await;
        Ok(RefreshLockGuard {
            _process_guard: process_guard,
            _file_guard: file_guard,
        })
    }

    async fn refresh_locked(&self) -> Result<()> {
        info!(
            server_name = %self.inner.server_name,
            "refreshing MCP OAuth credentials"
        );
        match self.refresh_authorization_manager().await {
            Ok(()) => {
                self.persist_if_needed().await?;
                info!(
                    server_name = %self.inner.server_name,
                    "refreshed MCP OAuth credentials"
                );
                Ok(())
            }
            Err(error) => self
                .handle_refresh_failure_after_reload(error)
                .await
                .with_context(|| {
                    format!(
                        "failed to refresh OAuth tokens for server {}",
                        self.inner.server_name
                    )
                }),
        }
    }

    async fn reload_persisted_credentials_status(&self) -> Result<PersistedCredentialReloadStatus> {
        if !self.reload_persisted_credentials_if_changed().await? {
            return Ok(PersistedCredentialReloadStatus::Unchanged);
        }

        let expires_at = {
            let guard = self.inner.last_credentials.lock().await;
            guard.as_ref().and_then(|tokens| tokens.expires_at)
        };
        if token_needs_refresh(expires_at) {
            return Ok(PersistedCredentialReloadStatus::NeedsRefresh);
        }

        info!(
            server_name = %self.inner.server_name,
            "using newer persisted MCP OAuth credentials"
        );
        Ok(PersistedCredentialReloadStatus::Fresh)
    }

    async fn handle_refresh_failure_after_reload(&self, error: AuthError) -> Result<()> {
        warn!(
            server_name = %self.inner.server_name,
            error = %error,
            "MCP OAuth refresh failed; checking persisted credentials"
        );

        match self.reload_persisted_credentials_status().await? {
            PersistedCredentialReloadStatus::Fresh => return Ok(()),
            PersistedCredentialReloadStatus::NeedsRefresh => {
                return match self.refresh_authorization_manager().await {
                    Ok(()) => {
                        self.persist_if_needed().await?;
                        info!(
                            server_name = %self.inner.server_name,
                            "refreshed MCP OAuth credentials after persisted credential reload"
                        );
                        Ok(())
                    }
                    Err(error) => self.handle_refresh_grant_failure(error).await,
                };
            }
            PersistedCredentialReloadStatus::Unchanged => {}
        }

        self.handle_refresh_grant_failure(error).await
    }

    async fn handle_refresh_grant_failure(&self, error: AuthError) -> Result<()> {
        if !refresh_grant_requires_reauthorization(&error) {
            return Err(error.into());
        }

        warn!(
            server_name = %self.inner.server_name,
            "MCP OAuth refresh grant is no longer usable; deleting stored credentials"
        );
        let failed_credentials = {
            let mut last_credentials = self.inner.last_credentials.lock().await;
            last_credentials.take()
        };
        let _ = delete_oauth_tokens_if_match(
            &self.inner.server_name,
            &self.inner.url,
            self.inner.store_mode,
            self.inner.keyring_backend_kind,
            failed_credentials.as_ref(),
        )?;
        Err(error).with_context(|| {
            format!(
                "OAuth credentials for MCP server {} are no longer valid; reauthorization is required",
                self.inner.server_name
            )
        })
    }

    #[expect(
        clippy::await_holding_invalid_type,
        reason = "AuthorizationManager async access must be serialized through its mutex"
    )]
    async fn refresh_authorization_manager(&self) -> std::result::Result<(), AuthError> {
        {
            let manager = self.inner.authorization_manager.clone();
            let guard = manager.lock().await;
            guard.refresh_token().await?;
        }
        Ok(())
    }

    async fn reload_persisted_credentials_if_changed(&self) -> Result<bool> {
        let latest = load_oauth_tokens(
            &self.inner.server_name,
            &self.inner.url,
            self.inner.store_mode,
            self.inner.keyring_backend_kind,
        )?;
        let latest = match latest {
            Some(tokens) => tokens,
            None => return Ok(false),
        };

        {
            let last_credentials = self.inner.last_credentials.lock().await;
            if last_credentials
                .as_ref()
                .is_some_and(|last| same_oauth_token_material(last, &latest))
            {
                return Ok(false);
            }
        }

        let mut oauth_state = OAuthState::new_with_oauth_http_client(
            self.inner.url.clone(),
            self.inner.oauth_http_client.clone(),
        )
        .await?;
        oauth_state
            .set_credentials(&latest.client_id, latest.token_response.0.clone())
            .await?;
        let replacement = oauth_state.into_authorization_manager().ok_or_else(|| {
            anyhow::anyhow!("failed to rebuild OAuth authorization manager from stored credentials")
        })?;

        {
            let mut manager = self.inner.authorization_manager.lock().await;
            *manager = replacement;
        }
        {
            let mut last_credentials = self.inner.last_credentials.lock().await;
            *last_credentials = Some(latest);
        }

        Ok(true)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PersistedCredentialReloadStatus {
    Unchanged,
    Fresh,
    NeedsRefresh,
}

struct RefreshLockGuard<'a> {
    _process_guard: TokioMutexGuard<'a, ()>,
    _file_guard: CrossProcessRefreshLockGuard,
}

#[cfg(unix)]
struct CrossProcessRefreshLockGuard {
    file: fs::File,
}

#[cfg(unix)]
impl CrossProcessRefreshLockGuard {
    async fn acquire(lock_key: String, server_name: String) -> Result<Self> {
        let path = refresh_lock_file_path(&lock_key)?;
        tokio::task::spawn_blocking(move || {
            info!(
                server_name = %server_name,
                path = %path.display(),
                "acquiring cross-process MCP OAuth refresh lock"
            );
            Self::acquire_blocking(path)
        })
        .await
        .context("failed to join MCP OAuth refresh lock task")?
    }

    fn acquire_blocking(path: PathBuf) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "failed to create MCP OAuth refresh lock directory at {}",
                    parent.display()
                )
            })?;
        }

        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)
            .with_context(|| {
                format!(
                    "failed to open MCP OAuth refresh lock file at {}",
                    path.display()
                )
            })?;

        loop {
            let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
            if result == 0 {
                return Ok(Self { file });
            }

            let error = std::io::Error::last_os_error();
            if error.kind() == ErrorKind::Interrupted {
                continue;
            }

            return Err(error).with_context(|| {
                format!(
                    "failed to lock MCP OAuth refresh lock file at {}",
                    path.display()
                )
            });
        }
    }
}

#[cfg(unix)]
impl Drop for CrossProcessRefreshLockGuard {
    fn drop(&mut self) {
        let result = unsafe { libc::flock(self.file.as_raw_fd(), libc::LOCK_UN) };
        if result != 0 {
            let error = std::io::Error::last_os_error();
            warn!("failed to release MCP OAuth refresh lock: {error}");
        }
    }
}

#[cfg(not(unix))]
struct CrossProcessRefreshLockGuard;

#[cfg(not(unix))]
impl CrossProcessRefreshLockGuard {
    async fn acquire(_lock_key: String, _server_name: String) -> Result<Self> {
        Ok(Self)
    }
}

fn refresh_lock_for_key(key: &str) -> Arc<Mutex<()>> {
    use std::collections::HashMap;
    use std::sync::Mutex as StdMutex;
    use std::sync::OnceLock;

    static REFRESH_LOCKS: OnceLock<StdMutex<HashMap<String, Arc<Mutex<()>>>>> = OnceLock::new();
    let locks = REFRESH_LOCKS.get_or_init(StdMutex::default);
    let mut locks = locks
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    locks
        .entry(key.to_string())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

fn refresh_lock_file_path(key: &str) -> Result<PathBuf> {
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    let digest = hasher.finalize();
    let hex = format!("{digest:x}");
    let truncated = &hex[..16];
    Ok(find_codex_home()?
        .join(".locks")
        .join(format!("mcp-oauth-refresh-{truncated}.lock"))
        .to_path_buf())
}

fn same_oauth_token_material(left: &StoredOAuthTokens, right: &StoredOAuthTokens) -> bool {
    left.server_name == right.server_name
        && left.url == right.url
        && left.client_id == right.client_id
        && left.token_response.0.access_token().secret()
            == right.token_response.0.access_token().secret()
        && left
            .token_response
            .0
            .refresh_token()
            .map(oauth2::RefreshToken::secret)
            == right
                .token_response
                .0
                .refresh_token()
                .map(oauth2::RefreshToken::secret)
        && scopes_as_json(&left.token_response.0) == scopes_as_json(&right.token_response.0)
}

fn scopes_as_json(token_response: &OAuthTokenResponse) -> Option<String> {
    token_response
        .scopes()
        .and_then(|scopes| serde_json::to_string(scopes).ok())
}

fn refresh_grant_requires_reauthorization(error: &AuthError) -> bool {
    match error {
        AuthError::AuthorizationRequired => true,
        AuthError::TokenRefreshFailed(message) | AuthError::OAuthError(message) => {
            message.contains("invalid_grant") || message.contains("No refresh token available")
        }
        _ => false,
    }
}

const FALLBACK_FILENAME: &str = ".credentials.json";
const MCP_SERVER_TYPE: &str = "http";

type FallbackFile = BTreeMap<String, FallbackTokenEntry>;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FallbackTokenEntry {
    server_name: String,
    server_url: String,
    client_id: String,
    access_token: String,
    #[serde(default)]
    expires_at: Option<u64>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    scopes: Vec<String>,
}

fn load_oauth_tokens_from_file(server_name: &str, url: &str) -> Result<Option<StoredOAuthTokens>> {
    let _store_lock = OAuthStoreLock::acquire(OAuthStore::File)?;
    let Some(store) = read_fallback_file_unlocked()? else {
        return Ok(None);
    };

    let key = compute_store_key(server_name, url)?;

    for entry in store.values() {
        let entry_key = compute_store_key(&entry.server_name, &entry.server_url)?;
        if entry_key != key {
            continue;
        }

        return Ok(Some(fallback_entry_to_stored_tokens(entry)));
    }

    Ok(None)
}

fn fallback_entry_to_stored_tokens(entry: &FallbackTokenEntry) -> StoredOAuthTokens {
    let mut token_response = OAuthTokenResponse::new(
        AccessToken::new(entry.access_token.clone()),
        BasicTokenType::Bearer,
        VendorExtraTokenFields::default(),
    );

    if let Some(refresh) = entry.refresh_token.clone() {
        token_response.set_refresh_token(Some(RefreshToken::new(refresh)));
    }

    let scopes = entry.scopes.clone();
    if !scopes.is_empty() {
        token_response.set_scopes(Some(scopes.into_iter().map(Scope::new).collect()));
    }

    let mut stored = StoredOAuthTokens {
        server_name: entry.server_name.clone(),
        url: entry.server_url.clone(),
        client_id: entry.client_id.clone(),
        token_response: WrappedOAuthTokenResponse(token_response),
        expires_at: entry.expires_at,
    };
    refresh_expires_in_from_timestamp(&mut stored);
    stored
}

/// Saves one credential while holding the File aggregate-store lock across the full
/// read-modify-write operation.
fn save_oauth_tokens_to_file(tokens: &StoredOAuthTokens) -> Result<()> {
    let _store_lock = OAuthStoreLock::acquire(OAuthStore::File)?;
    save_oauth_tokens_to_file_with_lock_held(tokens)
}

/// Updates the fallback File. The caller must hold the File aggregate-store lock.
fn save_oauth_tokens_to_file_with_lock_held(tokens: &StoredOAuthTokens) -> Result<()> {
    let key = compute_store_key(&tokens.server_name, &tokens.url)?;
    let mut store = read_fallback_file_unlocked()?.unwrap_or_default();

    let token_response = &tokens.token_response.0;
    let expires_at = tokens
        .expires_at
        .or_else(|| compute_expires_at_millis(token_response));
    let refresh_token = token_response
        .refresh_token()
        .map(|token| token.secret().to_string());
    let scopes = token_response
        .scopes()
        .map(|s| s.iter().map(|s| s.to_string()).collect())
        .unwrap_or_default();
    let entry = FallbackTokenEntry {
        server_name: tokens.server_name.clone(),
        server_url: tokens.url.clone(),
        client_id: tokens.client_id.clone(),
        access_token: token_response.access_token().secret().to_string(),
        expires_at,
        refresh_token,
        scopes,
    };

    store.insert(key, entry);
    write_fallback_file(&store)
}

fn delete_oauth_tokens_from_file(key: &str) -> Result<bool> {
    let _store_lock = OAuthStoreLock::acquire(OAuthStore::File)?;
    delete_oauth_tokens_from_file_unlocked(key)
}

fn delete_oauth_tokens_from_file_if_match(
    key: &str,
    expected_tokens: Option<&StoredOAuthTokens>,
) -> Result<bool> {
    let _store_lock = OAuthStoreLock::acquire(OAuthStore::File)?;
    let mut store = match read_fallback_file_unlocked()? {
        Some(store) => store,
        None => return Ok(false),
    };

    if let Some(expected) = expected_tokens
        && let Some(current) = store.get(key).map(fallback_entry_to_stored_tokens)
        && !same_oauth_token_material(&current, expected)
    {
        warn!(
            server_name = %expected.server_name,
            "skipping MCP OAuth fallback credential delete because stored token changed"
        );
        return Ok(false);
    }

    delete_oauth_tokens_from_file_store(&mut store, key)
}

fn delete_oauth_tokens_from_file_unlocked(key: &str) -> Result<bool> {
    let mut store = match read_fallback_file_unlocked()? {
        Some(store) => store,
        None => return Ok(false),
    };

    delete_oauth_tokens_from_file_store(&mut store, key)
}

fn delete_oauth_tokens_from_file_store(store: &mut FallbackFile, key: &str) -> Result<bool> {
    let removed = store.remove(key).is_some();

    if removed {
        write_fallback_file(store)?;
    }

    Ok(removed)
}

pub(crate) fn compute_expires_at_millis(response: &OAuthTokenResponse) -> Option<u64> {
    let expires_in = response.expires_in()?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0));
    let expiry = now.checked_add(expires_in)?;
    let millis = expiry.as_millis();
    if millis > u128::from(u64::MAX) {
        Some(u64::MAX)
    } else {
        Some(millis as u64)
    }
}

fn expires_in_from_timestamp(expires_at: u64) -> Option<u64> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0));
    let now_ms = now.as_millis() as u64;

    if expires_at <= now_ms {
        None
    } else {
        Some((expires_at - now_ms) / 1000)
    }
}

fn token_needs_refresh(expires_at: Option<u64>) -> bool {
    let Some(expires_at) = expires_at else {
        return false;
    };

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0))
        .as_millis() as u64;

    now.saturating_add(REFRESH_SKEW_MILLIS) >= expires_at
}

fn compute_store_key(server_name: &str, server_url: &str) -> Result<String> {
    let mut payload = JsonMap::new();
    payload.insert(
        "type".to_string(),
        Value::String(MCP_SERVER_TYPE.to_string()),
    );
    payload.insert("url".to_string(), Value::String(server_url.to_string()));
    payload.insert("headers".to_string(), Value::Object(JsonMap::new()));

    let truncated = sha_256_prefix(&Value::Object(payload))?;
    Ok(format!("{server_name}|{truncated}"))
}

/// Derive a valid secret-store name from the MCP OAuth store key.
///
/// `compute_store_key` intentionally includes readable identity components and
/// a pipe separator, but `SecretName` only allows `A-Z`, `0-9`, and `_`.
/// Re-hashing keeps the secret key deterministic while satisfying that
/// restricted alphabet.
fn compute_secret_name(server_name: &str, server_url: &str) -> Result<SecretName> {
    let key = compute_store_key(server_name, server_url)?;
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    let digest = hasher.finalize();
    let hex = format!("{digest:X}");
    SecretName::new(&format!("{MCP_OAUTH_SECRET_PREFIX}_{}", &hex[..32]))
}

fn fallback_file_path() -> Result<PathBuf> {
    Ok(find_codex_home()?.join(FALLBACK_FILENAME).to_path_buf())
}

fn read_fallback_file_unlocked() -> Result<Option<FallbackFile>> {
    let path = fallback_file_path()?;
    let contents = match fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(err).context(format!(
                "failed to read credentials file at {}",
                path.display()
            ));
        }
    };

    match serde_json::from_str::<FallbackFile>(&contents) {
        Ok(store) => Ok(Some(store)),
        Err(e) => Err(e).context(format!(
            "failed to parse credentials file at {}",
            path.display()
        )),
    }
}

fn write_fallback_file(store: &FallbackFile) -> Result<()> {
    let path = fallback_file_path()?;

    if store.is_empty() {
        if path.exists() {
            fs::remove_file(path)?;
        }
        return Ok(());
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let serialized = serde_json::to_string(store)?;
    fs::write(&path, serialized)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(0o600);
        fs::set_permissions(&path, perms)?;
    }

    Ok(())
}

fn sha_256_prefix(value: &Value) -> Result<String> {
    let serialized =
        serde_json::to_string(&value).context("failed to serialize MCP OAuth key payload")?;
    let mut hasher = Sha256::new();
    hasher.update(serialized.as_bytes());
    let digest = hasher.finalize();
    let hex = format!("{digest:x}");
    let truncated = &hex[..16];
    Ok(truncated.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use codex_keyring_store::tests::MockKeyringStore;
    use codex_secrets::compute_keyring_account;
    use keyring::Error as KeyringError;
    use pretty_assertions::assert_eq;
    use std::sync::Arc;

    use super::test_support::TempCodexHome;

    #[test]
    fn load_oauth_tokens_reads_from_keyring_when_available() -> Result<()> {
        let _env = TempCodexHome::new();
        let store = MockKeyringStore::default();
        let tokens = sample_tokens();
        let expected = tokens.clone();
        let serialized = serde_json::to_string(&tokens)?;
        let key = super::compute_store_key(&tokens.server_name, &tokens.url)?;
        store.save(KEYRING_SERVICE, &key, &serialized)?;

        let loaded = super::load_oauth_tokens_from_keyring(
            &store,
            AuthKeyringBackendKind::Direct,
            &tokens.server_name,
            &tokens.url,
        )?
        .expect("tokens should load from keyring");
        assert_tokens_match_without_expiry(&loaded, &expected);
        Ok(())
    }

    #[test]
    fn load_oauth_tokens_falls_back_when_missing_in_keyring() -> Result<()> {
        let _env = TempCodexHome::new();
        let store = MockKeyringStore::default();
        let tokens = sample_tokens();
        let expected = tokens.clone();

        super::save_oauth_tokens_to_file(&tokens)?;

        let loaded = super::load_oauth_tokens_from_keyring_with_fallback_to_file(
            &store,
            AuthKeyringBackendKind::Direct,
            &tokens.server_name,
            &tokens.url,
        )?
        .expect("tokens should load from fallback");
        assert_tokens_match_without_expiry(&loaded, &expected);
        Ok(())
    }

    #[test]
    fn load_oauth_tokens_falls_back_when_keyring_errors() -> Result<()> {
        let _env = TempCodexHome::new();
        let store = MockKeyringStore::default();
        let tokens = sample_tokens();
        let expected = tokens.clone();
        let key = super::compute_store_key(&tokens.server_name, &tokens.url)?;
        store.set_error(&key, KeyringError::Invalid("error".into(), "load".into()));

        super::save_oauth_tokens_to_file(&tokens)?;

        let loaded = super::load_oauth_tokens_from_keyring_with_fallback_to_file(
            &store,
            AuthKeyringBackendKind::Direct,
            &tokens.server_name,
            &tokens.url,
        )?
        .expect("tokens should load from fallback");
        assert_tokens_match_without_expiry(&loaded, &expected);
        Ok(())
    }

    #[test]
    fn save_oauth_tokens_prefers_keyring_when_available() -> Result<()> {
        let _env = TempCodexHome::new();
        let store = MockKeyringStore::default();
        let tokens = sample_tokens();
        let key = super::compute_store_key(&tokens.server_name, &tokens.url)?;

        super::save_oauth_tokens_to_file(&tokens)?;

        super::save_oauth_tokens_with_keyring_with_fallback_to_file(
            &store,
            AuthKeyringBackendKind::Direct,
            &tokens.server_name,
            &tokens,
        )?;

        let fallback_path = super::fallback_file_path()?;
        assert!(!fallback_path.exists(), "fallback file should be removed");
        let stored = store.saved_value(&key).expect("value saved to keyring");
        assert_eq!(serde_json::from_str::<StoredOAuthTokens>(&stored)?, tokens);
        Ok(())
    }

    #[test]
    fn save_oauth_tokens_writes_fallback_when_keyring_fails() -> Result<()> {
        let _env = TempCodexHome::new();
        let store = MockKeyringStore::default();
        let tokens = sample_tokens();
        let key = super::compute_store_key(&tokens.server_name, &tokens.url)?;
        store.set_error(&key, KeyringError::Invalid("error".into(), "save".into()));

        super::save_oauth_tokens_with_keyring_with_fallback_to_file(
            &store,
            AuthKeyringBackendKind::Direct,
            &tokens.server_name,
            &tokens,
        )?;

        let fallback_path = super::fallback_file_path()?;
        assert!(fallback_path.exists(), "fallback file should be created");
        let saved = super::read_fallback_file_unlocked()?.expect("fallback file should load");
        let key = super::compute_store_key(&tokens.server_name, &tokens.url)?;
        let entry = saved.get(&key).expect("entry for key");
        assert_eq!(entry.server_name, tokens.server_name);
        assert_eq!(entry.server_url, tokens.url);
        assert_eq!(entry.client_id, tokens.client_id);
        assert_eq!(
            entry.access_token,
            tokens.token_response.0.access_token().secret().as_str()
        );
        assert!(store.saved_value(&key).is_none());
        Ok(())
    }

    #[test]
    fn save_oauth_tokens_with_secrets_backend_writes_encrypted_storage() -> Result<()> {
        let env = TempCodexHome::new();
        let store = MockKeyringStore::default();
        let tokens = sample_tokens();
        let key = super::compute_store_key(&tokens.server_name, &tokens.url)?;
        let serialized = serde_json::to_string(&tokens)?;
        store.save(KEYRING_SERVICE, &key, &serialized)?;
        super::save_oauth_tokens_to_file(&tokens)?;

        super::save_oauth_tokens_with_keyring_with_fallback_to_file(
            &store,
            AuthKeyringBackendKind::Secrets,
            &tokens.server_name,
            &tokens,
        )?;

        let manager = SecretsManager::new_with_keyring_store_and_namespace(
            env.path().to_path_buf(),
            SecretsBackendKind::Local,
            Arc::new(store.clone()),
            LocalSecretsNamespace::McpOAuth,
        );
        let secret_name = super::compute_secret_name(&tokens.server_name, &tokens.url)?;
        let stored = manager
            .get(&SecretScope::Global, &secret_name)?
            .expect("tokens should be saved to encrypted storage");
        assert_eq!(serde_json::from_str::<StoredOAuthTokens>(&stored)?, tokens);
        assert_eq!(store.saved_value(&key), Some(serialized));
        assert!(env.path().join("secrets").join("mcp_oauth.age").exists());
        assert!(!env.path().join("secrets").join("local.age").exists());
        assert!(!super::fallback_file_path()?.exists());
        Ok(())
    }

    #[test]
    fn load_oauth_tokens_with_secrets_backend_reads_encrypted_storage() -> Result<()> {
        let _env = TempCodexHome::new();
        let store = MockKeyringStore::default();
        let tokens = sample_tokens();
        let expected = tokens.clone();

        super::save_oauth_tokens_with_keyring(
            &store,
            AuthKeyringBackendKind::Secrets,
            &tokens.server_name,
            &tokens,
        )?;

        let loaded = super::load_oauth_tokens_from_keyring(
            &store,
            AuthKeyringBackendKind::Secrets,
            &tokens.server_name,
            &tokens.url,
        )?
        .expect("tokens should load from encrypted storage");
        assert_tokens_match_without_expiry(&loaded, &expected);
        Ok(())
    }

    #[test]
    fn load_oauth_tokens_with_secrets_backend_ignores_direct_entry() -> Result<()> {
        let _env = TempCodexHome::new();
        let store = MockKeyringStore::default();
        let tokens = sample_tokens();
        let key = super::compute_store_key(&tokens.server_name, &tokens.url)?;
        let serialized = serde_json::to_string(&tokens)?;
        store.save(KEYRING_SERVICE, &key, &serialized)?;

        let loaded = super::load_oauth_tokens_from_keyring(
            &store,
            AuthKeyringBackendKind::Secrets,
            &tokens.server_name,
            &tokens.url,
        )?;

        assert!(loaded.is_none());
        Ok(())
    }

    #[test]
    fn save_oauth_tokens_with_secrets_backend_falls_back_to_file_when_keyring_fails() -> Result<()>
    {
        let env = TempCodexHome::new();
        let store = MockKeyringStore::default();
        store.set_error(
            &compute_keyring_account(env.path()),
            KeyringError::Invalid("error".into(), "save".into()),
        );
        let tokens = sample_tokens();

        super::save_oauth_tokens_with_keyring_with_fallback_to_file(
            &store,
            AuthKeyringBackendKind::Secrets,
            &tokens.server_name,
            &tokens,
        )?;

        let saved = super::read_fallback_file_unlocked()?.expect("fallback file should load");
        let key = super::compute_store_key(&tokens.server_name, &tokens.url)?;
        assert!(saved.contains_key(&key));
        Ok(())
    }

    #[test]
    fn delete_oauth_tokens_with_secrets_backend_removes_secrets_and_file() -> Result<()> {
        let env = TempCodexHome::new();
        let store = MockKeyringStore::default();
        let tokens = sample_tokens();
        let serialized = serde_json::to_string(&tokens)?;
        let key = super::compute_store_key(&tokens.server_name, &tokens.url)?;
        store.save(KEYRING_SERVICE, &key, &serialized)?;
        super::save_oauth_tokens_with_keyring(
            &store,
            AuthKeyringBackendKind::Secrets,
            &tokens.server_name,
            &tokens,
        )?;
        store.save(KEYRING_SERVICE, &key, &serialized)?;
        super::save_oauth_tokens_to_file(&tokens)?;

        let removed = super::delete_oauth_tokens_from_keyring_and_file(
            &store,
            OAuthCredentialsStoreMode::Auto,
            AuthKeyringBackendKind::Secrets,
            &tokens.server_name,
            &tokens.url,
        )?;

        let manager = SecretsManager::new_with_keyring_store_and_namespace(
            env.path().to_path_buf(),
            SecretsBackendKind::Local,
            Arc::new(store.clone()),
            LocalSecretsNamespace::McpOAuth,
        );
        let secret_name = super::compute_secret_name(&tokens.server_name, &tokens.url)?;
        assert!(removed);
        assert!(manager.get(&SecretScope::Global, &secret_name)?.is_none());
        assert!(store.saved_value(&key).is_none());
        assert!(!super::fallback_file_path()?.exists());
        Ok(())
    }

    #[test]
    fn delete_oauth_tokens_removes_all_storage() -> Result<()> {
        let _env = TempCodexHome::new();
        let store = MockKeyringStore::default();
        let tokens = sample_tokens();
        let serialized = serde_json::to_string(&tokens)?;
        let key = super::compute_store_key(&tokens.server_name, &tokens.url)?;
        store.save(KEYRING_SERVICE, &key, &serialized)?;
        super::save_oauth_tokens_to_file(&tokens)?;

        let removed = super::delete_oauth_tokens_from_keyring_and_file(
            &store,
            OAuthCredentialsStoreMode::Auto,
            AuthKeyringBackendKind::Direct,
            &tokens.server_name,
            &tokens.url,
        )?;
        assert!(removed);
        assert!(!store.contains(&key));
        assert!(!super::fallback_file_path()?.exists());
        Ok(())
    }

    #[test]
    fn delete_oauth_tokens_file_mode_removes_keyring_only_entry() -> Result<()> {
        let _env = TempCodexHome::new();
        let store = MockKeyringStore::default();
        let tokens = sample_tokens();
        let serialized = serde_json::to_string(&tokens)?;
        let key = super::compute_store_key(&tokens.server_name, &tokens.url)?;
        store.save(KEYRING_SERVICE, &key, &serialized)?;
        assert!(store.contains(&key));

        let removed = super::delete_oauth_tokens_from_keyring_and_file(
            &store,
            OAuthCredentialsStoreMode::Auto,
            AuthKeyringBackendKind::Direct,
            &tokens.server_name,
            &tokens.url,
        )?;
        assert!(removed);
        assert!(!store.contains(&key));
        assert!(!super::fallback_file_path()?.exists());
        Ok(())
    }

    #[test]
    fn delete_oauth_tokens_if_match_keeps_newer_fallback_token() -> Result<()> {
        let _env = TempCodexHome::new();
        let old_tokens = sample_tokens();
        let mut newer_tokens = old_tokens.clone();
        newer_tokens.token_response.0 = sample_token_response("new-access", Some("new-refresh"));
        let expires_in = Duration::from_secs(3600);
        newer_tokens
            .token_response
            .0
            .set_expires_in(Some(&expires_in));
        newer_tokens.expires_at = super::compute_expires_at_millis(&newer_tokens.token_response.0);
        super::save_oauth_tokens_to_file(&newer_tokens)?;

        let removed = super::delete_oauth_tokens_if_match(
            &old_tokens.server_name,
            &old_tokens.url,
            OAuthCredentialsStoreMode::File,
            AuthKeyringBackendKind::Direct,
            Some(&old_tokens),
        )?;

        assert!(!removed, "newer token should not be deleted");
        let loaded = super::load_oauth_tokens_from_file(&old_tokens.server_name, &old_tokens.url)?
            .expect("newer fallback token should remain");
        assert_tokens_match_without_expiry(&loaded, &newer_tokens);
        Ok(())
    }

    #[test]
    fn delete_oauth_tokens_propagates_keyring_errors() -> Result<()> {
        let _env = TempCodexHome::new();
        let store = MockKeyringStore::default();
        let tokens = sample_tokens();
        let key = super::compute_store_key(&tokens.server_name, &tokens.url)?;
        store.set_error(&key, KeyringError::Invalid("error".into(), "delete".into()));
        super::save_oauth_tokens_to_file(&tokens).unwrap();

        let result = super::delete_oauth_tokens_from_keyring_and_file(
            &store,
            OAuthCredentialsStoreMode::Auto,
            AuthKeyringBackendKind::Direct,
            &tokens.server_name,
            &tokens.url,
        );
        assert!(result.is_err());
        assert!(super::fallback_file_path().unwrap().exists());
        Ok(())
    }

    #[test]
    fn refresh_expires_in_from_timestamp_restores_future_durations() {
        let mut tokens = sample_tokens();
        let expires_at = tokens.expires_at.expect("expires_at should be set");

        tokens.token_response.0.set_expires_in(None);
        super::refresh_expires_in_from_timestamp(&mut tokens);

        let actual = tokens
            .token_response
            .0
            .expires_in()
            .expect("expires_in should be restored")
            .as_secs();
        let expected = super::expires_in_from_timestamp(expires_at)
            .expect("expires_at should still be in the future");
        let diff = actual.abs_diff(expected);
        assert!(diff <= 1, "expires_in drift too large: diff={diff}");
    }

    #[test]
    fn refresh_expires_in_from_timestamp_marks_expired_tokens() {
        let mut tokens = sample_tokens();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_else(|_| Duration::from_secs(0));
        let expired_at = now.as_millis() as u64;
        tokens.expires_at = Some(expired_at.saturating_sub(1000));

        let duration = Duration::from_secs(600);
        tokens.token_response.0.set_expires_in(Some(&duration));

        super::refresh_expires_in_from_timestamp(&mut tokens);

        assert_eq!(tokens.token_response.0.expires_in(), Some(Duration::ZERO));
    }

    #[test]
    fn oauth_tokens_are_usable_when_expiry_is_unknown() {
        let mut tokens = sample_tokens();
        tokens.expires_at = None;
        tokens.token_response.0.set_refresh_token(None);

        assert!(super::oauth_tokens_are_usable(&tokens));
    }

    #[test]
    fn oauth_tokens_are_usable_when_unexpired_without_refresh_token() {
        let mut tokens = sample_tokens();
        tokens.token_response.0.set_refresh_token(None);

        assert!(super::oauth_tokens_are_usable(&tokens));
    }

    #[test]
    fn oauth_tokens_are_usable_when_expired_but_refreshable() {
        let mut tokens = sample_tokens();
        tokens.expires_at = Some(0);

        assert!(super::oauth_tokens_are_usable(&tokens));
    }

    #[test]
    fn oauth_tokens_are_not_usable_when_expired_and_unrefreshable() {
        let mut tokens = sample_tokens();
        tokens.expires_at = Some(0);
        tokens.token_response.0.set_refresh_token(None);

        assert!(!super::oauth_tokens_are_usable(&tokens));
    }

    #[test]
    fn oauth_tokens_are_not_usable_when_near_expiry_and_unrefreshable() {
        let mut tokens = sample_tokens();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_else(|_| Duration::from_secs(0))
            .as_millis() as u64;
        tokens.expires_at = Some(now.saturating_add(REFRESH_SKEW_MILLIS - 1));
        tokens.token_response.0.set_refresh_token(None);

        assert!(!super::oauth_tokens_are_usable(&tokens));
    }

    #[test]
    fn oauth_tokens_are_not_usable_when_client_id_is_blank() {
        let mut tokens = sample_tokens();
        tokens.client_id = " ".to_string();

        assert!(!super::oauth_tokens_are_usable(&tokens));
    }

    #[test]
    fn oauth_tokens_are_not_usable_when_access_token_is_blank() {
        let mut tokens = sample_tokens();
        tokens
            .token_response
            .0
            .set_access_token(AccessToken::new(" ".to_string()));

        assert!(!super::oauth_tokens_are_usable(&tokens));
    }

    #[test]
    fn oauth_tokens_are_not_usable_when_required_refresh_token_is_blank() {
        let mut tokens = sample_tokens();
        tokens.expires_at = Some(0);
        tokens
            .token_response
            .0
            .set_refresh_token(Some(RefreshToken::new(" ".to_string())));

        assert!(!super::oauth_tokens_are_usable(&tokens));
    }

    fn assert_tokens_match_without_expiry(
        actual: &StoredOAuthTokens,
        expected: &StoredOAuthTokens,
    ) {
        assert_eq!(actual.server_name, expected.server_name);
        assert_eq!(actual.url, expected.url);
        assert_eq!(actual.client_id, expected.client_id);
        assert_eq!(actual.expires_at, expected.expires_at);
        assert_token_response_match_without_expiry(
            &actual.token_response,
            &expected.token_response,
        );
    }

    fn assert_token_response_match_without_expiry(
        actual: &WrappedOAuthTokenResponse,
        expected: &WrappedOAuthTokenResponse,
    ) {
        let actual_response = &actual.0;
        let expected_response = &expected.0;

        assert_eq!(
            actual_response.access_token().secret(),
            expected_response.access_token().secret()
        );
        assert_eq!(actual_response.token_type(), expected_response.token_type());
        assert_eq!(
            actual_response.refresh_token().map(RefreshToken::secret),
            expected_response.refresh_token().map(RefreshToken::secret),
        );
        assert_eq!(actual_response.scopes(), expected_response.scopes());
        assert_eq!(
            actual_response.extra_fields().0,
            expected_response.extra_fields().0
        );
        assert_eq!(
            actual_response.expires_in().is_some(),
            expected_response.expires_in().is_some()
        );
    }

    fn sample_tokens() -> StoredOAuthTokens {
        let mut response = sample_token_response("access-token", Some("refresh-token"));
        response.set_scopes(Some(vec![
            Scope::new("scope-a".to_string()),
            Scope::new("scope-b".to_string()),
        ]));
        let expires_in = Duration::from_secs(3600);
        response.set_expires_in(Some(&expires_in));
        let expires_at = super::compute_expires_at_millis(&response);

        StoredOAuthTokens {
            server_name: "test-server".to_string(),
            url: "https://example.test".to_string(),
            client_id: "client-id".to_string(),
            token_response: WrappedOAuthTokenResponse(response),
            expires_at,
        }
    }

    fn sample_token_response(
        access_token: &str,
        refresh_token: Option<&str>,
    ) -> OAuthTokenResponse {
        let mut response = OAuthTokenResponse::new(
            AccessToken::new(access_token.to_string()),
            BasicTokenType::Bearer,
            VendorExtraTokenFields::default(),
        );
        if let Some(refresh_token) = refresh_token {
            response.set_refresh_token(Some(RefreshToken::new(refresh_token.to_string())));
        }
        response
    }
}
