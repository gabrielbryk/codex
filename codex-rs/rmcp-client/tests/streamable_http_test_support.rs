//! Shared helpers for Streamable HTTP RMCP integration tests.
//!
//! This support module starts the test HTTP server, launches a real
//! `exec-server` when remote coverage is needed, and provides small helpers for
//! creating RMCP clients and asserting round-trip behavior.

// This support module is included by multiple integration-test crates. Each
// crate uses a different subset of the helpers, so dead-code warnings would
// otherwise depend on which test file compiled the module.
#![allow(dead_code)]

use std::ffi::OsString;
use std::net::TcpListener;
use std::path::Path;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use anyhow::Context as _;
use codex_config::types::AuthKeyringBackendKind;
use codex_config::types::OAuthCredentialsStoreMode;
use codex_exec_server::Environment;
use codex_exec_server::ExecServerClient;
use codex_exec_server::HttpClient;
use codex_exec_server::RemoteExecServerConnectArgs;
use codex_rmcp_client::ElicitationAction;
use codex_rmcp_client::ElicitationResponse;
use codex_rmcp_client::RmcpClient;
use codex_rmcp_client::StoredOAuthTokens;
use codex_rmcp_client::WrappedOAuthTokenResponse;
use codex_rmcp_client::save_oauth_tokens;
use codex_utils_cargo_bin::CargoBinError;
use futures::FutureExt as _;
use oauth2::AccessToken;
use oauth2::RefreshToken;
use oauth2::Scope;
use oauth2::basic::BasicTokenType;
use pretty_assertions::assert_eq;
use rmcp::model::CallToolResult;
use rmcp::model::ClientCapabilities;
use rmcp::model::ElicitationCapability;
use rmcp::model::FormElicitationCapability;
use rmcp::model::Implementation;
use rmcp::model::InitializeRequestParams;
use rmcp::model::ProtocolVersion;
use rmcp::transport::auth::OAuthTokenResponse;
use rmcp::transport::auth::VendorExtraTokenFields;
use serde_json::json;
use tempfile::TempDir;
use tokio::io::AsyncBufReadExt;
use tokio::io::BufReader;
use tokio::net::TcpStream;
use tokio::process::Child;
use tokio::process::Command;
use tokio::time::sleep;

const SESSION_POST_FAILURE_CONTROL_PATH: &str = "/test/control/session-post-failure";
const INITIALIZE_POST_FAILURE_CONTROL_PATH: &str = "/test/control/initialize-post-failure";
const INITIALIZED_NOTIFICATION_POST_FAILURE_CONTROL_PATH: &str =
    "/test/control/initialized-notification-post-failure";

fn streamable_http_server_bin() -> Result<PathBuf, CargoBinError> {
    codex_utils_cargo_bin::cargo_bin("test_streamable_http_server")
}

fn init_params() -> InitializeRequestParams {
    let mut capabilities = ClientCapabilities::default();
    capabilities.elicitation = Some(ElicitationCapability {
        form: Some(FormElicitationCapability {
            schema_validation: None,
        }),
        url: None,
    });
    InitializeRequestParams::new(
        capabilities,
        Implementation::new("codex-test", "0.0.0-test").with_title("Codex rmcp recovery test"),
    )
    .with_protocol_version(ProtocolVersion::V_2025_06_18)
}

pub(crate) fn expected_echo_result(message: &str) -> CallToolResult {
    let mut result = CallToolResult::success(Vec::new());
    result.structured_content = Some(json!({
        "echo": format!("ECHOING: {message}"),
        "env": null,
    }));
    result
}

pub(crate) async fn create_client(base_url: &str) -> anyhow::Result<RmcpClient> {
    create_client_with_http_client(base_url, Environment::default_for_tests().get_http_client())
        .await
}

pub(crate) async fn create_client_with_http_client(
    base_url: &str,
    http_client: Arc<dyn HttpClient>,
) -> anyhow::Result<RmcpClient> {
    let client = RmcpClient::new_streamable_http_client(
        "test-streamable-http",
        &format!("{base_url}/mcp"),
        Some("test-bearer".to_string()),
        /*http_headers*/ None,
        /*env_http_headers*/ None,
        OAuthCredentialsStoreMode::File,
        AuthKeyringBackendKind::default(),
        http_client,
        /*auth_provider*/ None,
    )
    .await?;

    initialize_client(&client).await?;

    Ok(client)
}

pub(crate) async fn initialize_client(client: &RmcpClient) -> anyhow::Result<()> {
    client
        .initialize(
            init_params(),
            Some(Duration::from_secs(5)),
            Box::new(|_, _| {
                async {
                    Ok(ElicitationResponse {
                        action: ElicitationAction::Accept,
                        content: Some(json!({})),
                        meta: None,
                    })
                }
                .boxed()
            }),
        )
        .await?;
    Ok(())
}

pub(crate) struct OAuthTestClient {
    pub(crate) client: RmcpClient,
    _codex_home: TempDir,
    _codex_home_guard: EnvVarGuard,
}

pub(crate) async fn create_oauth_client(
    base_url: &str,
    access_token: &str,
    refresh_token: &str,
) -> anyhow::Result<OAuthTestClient> {
    create_oauth_client_with_expires_at(base_url, access_token, refresh_token, None).await
}

pub(crate) async fn create_oauth_client_with_expires_at(
    base_url: &str,
    access_token: &str,
    refresh_token: &str,
    expires_at: Option<u64>,
) -> anyhow::Result<OAuthTestClient> {
    let server_name = "test-streamable-http-oauth";
    let server_url = format!("{base_url}/mcp");
    let codex_home = TempDir::new()?;
    write_fallback_oauth_tokens(
        codex_home.path(),
        server_name,
        &server_url,
        access_token,
        refresh_token,
        expires_at,
    )?;
    let codex_home_guard = EnvVarGuard::set("CODEX_HOME", codex_home.path().as_os_str());
    let client = RmcpClient::new_streamable_http_client(
        server_name,
        &server_url,
        /*bearer_token*/ None,
        /*http_headers*/ None,
        /*env_http_headers*/ None,
        OAuthCredentialsStoreMode::File,
        AuthKeyringBackendKind::default(),
        Environment::default_for_tests().get_http_client(),
        /*auth_provider*/ None,
    )
    .await?;

    client
        .initialize(
            init_params(),
            Some(Duration::from_secs(5)),
            Box::new(|_, _| {
                async {
                    Ok(ElicitationResponse {
                        action: ElicitationAction::Accept,
                        content: Some(json!({})),
                        meta: None,
                    })
                }
                .boxed()
            }),
        )
        .await?;

    Ok(OAuthTestClient {
        client,
        _codex_home: codex_home,
        _codex_home_guard: codex_home_guard,
    })
}

impl OAuthTestClient {
    pub(crate) fn codex_home_path(&self) -> &Path {
        self._codex_home.path()
    }
}

/// Creates a Streamable HTTP RMCP client that sends traffic through the remote
/// runtime HTTP API.
pub(crate) async fn create_remote_client(
    base_url: &str,
    http_client: ExecServerClient,
) -> anyhow::Result<RmcpClient> {
    let client = RmcpClient::new_streamable_http_client(
        "test-streamable-http-remote",
        &format!("{base_url}/mcp"),
        Some("test-bearer".to_string()),
        /*http_headers*/ None,
        /*env_http_headers*/ None,
        OAuthCredentialsStoreMode::File,
        AuthKeyringBackendKind::default(),
        Arc::new(http_client),
        /*auth_provider*/ None,
    )
    .await?;

    client
        .initialize(
            init_params(),
            Some(Duration::from_secs(5)),
            Box::new(|_, _| {
                async {
                    Ok(ElicitationResponse {
                        action: ElicitationAction::Accept,
                        content: Some(json!({})),
                        meta: None,
                    })
                }
                .boxed()
            }),
        )
        .await?;

    Ok(client)
}

pub(crate) async fn call_echo_tool(
    client: &RmcpClient,
    message: &str,
) -> anyhow::Result<CallToolResult> {
    client
        .call_tool(
            "echo".to_string(),
            Some(json!({ "message": message })),
            /*meta*/ None,
            Some(Duration::from_secs(5)),
        )
        .await
}

pub(crate) async fn arm_session_post_failure(
    base_url: &str,
    status: u16,
    remaining: usize,
    www_authenticate_headers: &[&str],
) -> anyhow::Result<()> {
    arm_session_post_failure_for_method(
        base_url,
        status,
        remaining,
        www_authenticate_headers,
        /*mcp_method*/ None,
    )
    .await
}

pub(crate) async fn arm_session_post_failure_for_method(
    base_url: &str,
    status: u16,
    remaining: usize,
    www_authenticate_headers: &[&str],
    mcp_method: Option<&str>,
) -> anyhow::Result<()> {
    let response = reqwest::Client::new()
        .post(format!("{base_url}{SESSION_POST_FAILURE_CONTROL_PATH}"))
        .json(&json!({
            "status": status,
            "remaining": remaining,
            "www_authenticate_headers": www_authenticate_headers,
            "mcp_method": mcp_method,
        }))
        .send()
        .await?;

    assert_eq!(response.status(), reqwest::StatusCode::NO_CONTENT);
    Ok(())
}

pub(crate) async fn set_expected_bearer(base_url: &str, token: Option<&str>) -> anyhow::Result<()> {
    let response = reqwest::Client::new()
        .post(format!("{base_url}/test/control/expected-bearer"))
        .json(&json!({
            "token": token,
        }))
        .send()
        .await?;

    assert_eq!(response.status(), reqwest::StatusCode::NO_CONTENT);
    Ok(())
}

pub(crate) async fn arm_session_post_json_rpc_failure(
    base_url: &str,
    status: u16,
    remaining: usize,
) -> anyhow::Result<()> {
    let response = reqwest::Client::new()
        .post(format!("{base_url}{SESSION_POST_FAILURE_CONTROL_PATH}"))
        .json(&json!({
            "status": status,
            "remaining": remaining,
            "content_type": "application/json",
            "body": json!({
                "jsonrpc": "2.0",
                "id": 1,
                "error": {
                    "code": -32000,
                    "message": "transient session failure",
                },
            }).to_string(),
        }))
        .send()
        .await?;

    assert_eq!(response.status(), reqwest::StatusCode::NO_CONTENT);
    Ok(())
}

pub(crate) async fn arm_initialized_notification_post_json_rpc_failure(
    base_url: &str,
    status: u16,
    remaining: usize,
) -> anyhow::Result<()> {
    let response = reqwest::Client::new()
        .post(format!(
            "{base_url}{INITIALIZED_NOTIFICATION_POST_FAILURE_CONTROL_PATH}"
        ))
        .json(&json!({
            "status": status,
            "remaining": remaining,
            "content_type": "application/json",
            "body": json!({
                "jsonrpc": "2.0",
                "id": 1,
                "error": {
                    "code": -32000,
                    "message": "transient session failure",
                },
            }).to_string(),
        }))
        .send()
        .await?;

    assert_eq!(response.status(), reqwest::StatusCode::NO_CONTENT);
    Ok(())
}

pub(crate) async fn arm_initialize_post_failure(
    base_url: &str,
    status: u16,
    remaining: usize,
) -> anyhow::Result<()> {
    let response = reqwest::Client::new()
        .post(format!("{base_url}{INITIALIZE_POST_FAILURE_CONTROL_PATH}"))
        .json(&json!({
            "status": status,
            "remaining": remaining,
        }))
        .send()
        .await?;

    assert_eq!(response.status(), reqwest::StatusCode::NO_CONTENT);
    Ok(())
}

pub(crate) async fn arm_initialize_post_json_rpc_failure(
    base_url: &str,
    status: u16,
    remaining: usize,
) -> anyhow::Result<()> {
    let response = reqwest::Client::new()
        .post(format!("{base_url}{INITIALIZE_POST_FAILURE_CONTROL_PATH}"))
        .json(&json!({
            "status": status,
            "remaining": remaining,
            "content_type": "application/json",
            "body": json!({
                "jsonrpc": "2.0",
                "id": 1,
                "error": {
                    "code": -32000,
                    "message": "transient initialize failure",
                },
            }).to_string(),
        }))
        .send()
        .await?;

    assert_eq!(response.status(), reqwest::StatusCode::NO_CONTENT);
    Ok(())
}

pub(crate) async fn spawn_streamable_http_server() -> anyhow::Result<(Child, String)> {
    spawn_streamable_http_server_with_env(&[]).await
}

pub(crate) async fn spawn_streamable_http_server_with_env(
    env: &[(&str, &str)],
) -> anyhow::Result<(Child, String)> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    drop(listener);

    let bind_addr = format!("127.0.0.1:{port}");
    let base_url = format!("http://{bind_addr}");
    let mut command = Command::new(streamable_http_server_bin()?);
    command
        .kill_on_drop(true)
        .env("MCP_STREAMABLE_HTTP_BIND_ADDR", &bind_addr);
    for (name, value) in env {
        command.env(name, value);
    }
    let mut child = command.spawn()?;

    wait_for_streamable_http_server(&mut child, &bind_addr, Duration::from_secs(5)).await?;
    Ok((child, base_url))
}

pub(crate) fn write_fallback_oauth_tokens(
    home: &Path,
    server_name: &str,
    server_url: &str,
    access_token: &str,
    refresh_token: &str,
    expires_at: Option<u64>,
) -> anyhow::Result<()> {
    let expires_at = match expires_at {
        Some(expires_at) => expires_at,
        None => SystemTime::now()
            .checked_add(Duration::from_secs(3600))
            .ok_or_else(|| anyhow::anyhow!("failed to compute expiry time"))?
            .duration_since(UNIX_EPOCH)?
            .as_millis() as u64,
    };

    let mut token_response = OAuthTokenResponse::new(
        AccessToken::new(access_token.to_string()),
        BasicTokenType::Bearer,
        VendorExtraTokenFields::default(),
    );
    token_response.set_refresh_token(Some(RefreshToken::new(refresh_token.to_string())));
    token_response.set_scopes(Some(vec![Scope::new("profile".to_string())]));
    let tokens = StoredOAuthTokens {
        server_name: server_name.to_string(),
        url: server_url.to_string(),
        client_id: "test-client-id".to_string(),
        token_response: WrappedOAuthTokenResponse(token_response),
        expires_at: Some(expires_at),
    };
    let _home_guard = EnvVarGuard::set("CODEX_HOME", home.as_os_str());
    save_oauth_tokens(
        server_name,
        &tokens,
        OAuthCredentialsStoreMode::File,
        AuthKeyringBackendKind::default(),
    )?;
    Ok(())
}

pub(crate) struct EnvVarGuard {
    key: &'static str,
    original: Option<OsString>,
}

impl EnvVarGuard {
    pub(crate) fn set(key: &'static str, value: &std::ffi::OsStr) -> Self {
        let original = std::env::var_os(key);
        unsafe {
            std::env::set_var(key, value);
        }
        Self { key, original }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        unsafe {
            match &self.original {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }
}

/// Owns the exec-server process used by the remote-client integration test.
pub(crate) struct ExecServerProcess {
    _codex_home: TempDir,
    child: Child,
    pub(crate) client: ExecServerClient,
}

impl Drop for ExecServerProcess {
    /// Stops the local exec-server process best-effort when the test exits.
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

/// Starts a local exec-server and connects an initialized `ExecServerClient`.
pub(crate) async fn spawn_exec_server() -> anyhow::Result<ExecServerProcess> {
    let codex_home = TempDir::new()?;
    let mut child = Command::new(codex_utils_cargo_bin::cargo_bin("codex")?)
        .args(["exec-server", "--listen", "ws://127.0.0.1:0"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .env("CODEX_HOME", codex_home.path())
        .spawn()?;

    let websocket_url = read_exec_server_listen_url(&mut child).await?;
    let client = ExecServerClient::connect_websocket(RemoteExecServerConnectArgs::new(
        websocket_url,
        "rmcp-client-remote-http-test".to_string(),
    ))
    .await?;

    Ok(ExecServerProcess {
        _codex_home: codex_home,
        child,
        client,
    })
}

/// Reads the websocket URL printed by `codex exec-server --listen`.
async fn read_exec_server_listen_url(child: &mut Child) -> anyhow::Result<String> {
    let stdout = child
        .stdout
        .take()
        .context("failed to capture exec-server stdout")?;
    let mut lines = BufReader::new(stdout).lines();
    let deadline = Instant::now() + Duration::from_secs(10);

    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            anyhow::bail!("timed out waiting for exec-server listen URL");
        }

        let line = tokio::time::timeout(remaining, lines.next_line())
            .await
            .context("timed out waiting for exec-server stdout")??
            .context("exec-server stdout closed before emitting listen URL")?;
        let listen_url = line.trim();
        if listen_url.starts_with("ws://") {
            return Ok(listen_url.to_string());
        }
    }
}

async fn wait_for_streamable_http_server(
    server_child: &mut Child,
    address: &str,
    timeout: Duration,
) -> anyhow::Result<()> {
    let deadline = Instant::now() + timeout;

    loop {
        if let Some(status) = server_child.try_wait()? {
            return Err(anyhow::anyhow!(
                "streamable HTTP server exited early with status {status}"
            ));
        }

        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(anyhow::anyhow!(
                "timed out waiting for streamable HTTP server at {address}: deadline reached"
            ));
        }

        match tokio::time::timeout(remaining, TcpStream::connect(address)).await {
            Ok(Ok(_)) => return Ok(()),
            Ok(Err(error)) => {
                if Instant::now() >= deadline {
                    return Err(anyhow::anyhow!(
                        "timed out waiting for streamable HTTP server at {address}: {error}"
                    ));
                }
            }
            Err(_) => {
                return Err(anyhow::anyhow!(
                    "timed out waiting for streamable HTTP server at {address}: connect call timed out"
                ));
            }
        }

        sleep(Duration::from_millis(50)).await;
    }
}
