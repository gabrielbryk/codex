mod streamable_http_test_support;

use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::time::Duration;

use codex_config::types::AuthKeyringBackendKind;
use codex_config::types::OAuthCredentialsStoreMode;
use codex_exec_server::Environment;
use codex_exec_server::ExecServerError;
use codex_exec_server::HttpClient;
use codex_exec_server::HttpRequestParams;
use codex_exec_server::HttpRequestResponse;
use codex_exec_server::HttpResponseBodyStream;
use codex_rmcp_client::RmcpClient;
use futures::FutureExt as _;
use futures::future::BoxFuture;
use futures::future::join_all;
use pretty_assertions::assert_eq;
use serde_json::Value;

use streamable_http_test_support::EnvVarGuard;
use streamable_http_test_support::arm_initialize_post_failure;
use streamable_http_test_support::arm_initialize_post_json_rpc_failure;
use streamable_http_test_support::arm_initialized_notification_post_json_rpc_failure;
use streamable_http_test_support::arm_session_post_failure;
use streamable_http_test_support::arm_session_post_failure_for_method;
use streamable_http_test_support::arm_session_post_json_rpc_failure;
use streamable_http_test_support::call_echo_tool;
use streamable_http_test_support::create_client;
use streamable_http_test_support::create_client_with_http_client;
use streamable_http_test_support::create_oauth_client;
use streamable_http_test_support::expected_echo_result;
use streamable_http_test_support::set_expected_bearer;
use streamable_http_test_support::spawn_streamable_http_server;
use streamable_http_test_support::spawn_streamable_http_server_with_env;
use streamable_http_test_support::write_fallback_oauth_tokens;

const JSON_RPC_INTERNAL_ERROR_CODE: i64 = -32603;
const SIMULATED_NO_RESPONSE_MESSAGE: &str =
    "http/request failed: error sending request for url (simulated no response)";

#[derive(Clone)]
struct FailFirstInitializeHttpClient {
    inner: Arc<dyn HttpClient>,
    failures_remaining: Arc<AtomicUsize>,
    initialize_attempts: Arc<AtomicUsize>,
}

impl FailFirstInitializeHttpClient {
    fn new(inner: Arc<dyn HttpClient>, failures_remaining: usize) -> Self {
        Self {
            inner,
            failures_remaining: Arc::new(AtomicUsize::new(failures_remaining)),
            initialize_attempts: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn initialize_attempts(&self) -> usize {
        self.initialize_attempts.load(Ordering::SeqCst)
    }

    fn fail_next_initialize(&self) {
        self.failures_remaining.store(1, Ordering::SeqCst);
    }
}

impl HttpClient for FailFirstInitializeHttpClient {
    fn http_request(
        &self,
        params: HttpRequestParams,
    ) -> BoxFuture<'_, Result<HttpRequestResponse, ExecServerError>> {
        self.inner.http_request(params)
    }

    fn http_request_stream(
        &self,
        params: HttpRequestParams,
    ) -> BoxFuture<'_, Result<(HttpRequestResponse, HttpResponseBodyStream), ExecServerError>> {
        let inner = Arc::clone(&self.inner);
        let failures_remaining = Arc::clone(&self.failures_remaining);
        let initialize_attempts = Arc::clone(&self.initialize_attempts);

        async move {
            if is_initialize_post(&params) {
                initialize_attempts.fetch_add(1, Ordering::SeqCst);
                if failures_remaining.swap(0, Ordering::SeqCst) > 0 {
                    return Err(ExecServerError::Server {
                        code: JSON_RPC_INTERNAL_ERROR_CODE,
                        message: SIMULATED_NO_RESPONSE_MESSAGE.to_string(),
                    });
                }
            }

            inner.http_request_stream(params).await
        }
        .boxed()
    }
}

fn is_initialize_post(params: &HttpRequestParams) -> bool {
    params.method.eq_ignore_ascii_case("POST")
        && params
            .body
            .as_ref()
            .and_then(|body| serde_json::from_slice::<Value>(&body.0).ok())
            .and_then(|body| {
                body.get("method")
                    .and_then(Value::as_str)
                    .map(|method| method == "initialize")
            })
            .unwrap_or(false)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn streamable_http_initialize_retries_remote_no_response_error() -> anyhow::Result<()> {
    let (_server, base_url) = spawn_streamable_http_server().await?;
    let http_client = FailFirstInitializeHttpClient::new(
        Environment::default_for_tests().get_http_client(),
        /*failures_remaining*/ 1,
    );

    let client = create_client_with_http_client(&base_url, Arc::new(http_client.clone())).await?;
    let result = call_echo_tool(&client, "after-init-retry").await?;

    assert_eq!(http_client.initialize_attempts(), 2);
    assert_eq!(result, expected_echo_result("after-init-retry"));

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn streamable_http_initialize_retries_transient_http_status() -> anyhow::Result<()> {
    let (_server, base_url) = spawn_streamable_http_server().await?;

    arm_initialize_post_failure(&base_url, /*status*/ 502, /*remaining*/ 1).await?;

    let client = create_client(&base_url).await?;
    let result = call_echo_tool(&client, "after-status-retry").await?;

    assert_eq!(result, expected_echo_result("after-status-retry"));

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn streamable_http_initialize_retries_json_rpc_transient_status() -> anyhow::Result<()> {
    let (_server, base_url) = spawn_streamable_http_server().await?;

    arm_initialize_post_json_rpc_failure(&base_url, /*status*/ 502, /*remaining*/ 1).await?;

    let client = create_client(&base_url).await?;
    let result = call_echo_tool(&client, "after-json-status-retry").await?;

    assert_eq!(result, expected_echo_result("after-json-status-retry"));

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn streamable_http_retries_initialized_notification_status() -> anyhow::Result<()> {
    let (_server, base_url) = spawn_streamable_http_server().await?;

    arm_initialized_notification_post_json_rpc_failure(
        &base_url, /*status*/ 502, /*remaining*/ 1,
    )
    .await?;

    let client = create_client(&base_url).await?;
    let result = call_echo_tool(&client, "after-notification-status-retry").await?;

    assert_eq!(
        result,
        expected_echo_result("after-notification-status-retry")
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn streamable_http_tools_list_retries_transient_http_status() -> anyhow::Result<()> {
    let (_server, base_url) = spawn_streamable_http_server().await?;
    let client = create_client(&base_url).await?;

    let expected = client
        .list_tools(
            /*params*/ None,
            /*timeout*/ Some(Duration::from_secs(5)),
        )
        .await?;
    arm_session_post_failure(
        &base_url,
        /*status*/ 502,
        /*remaining*/ 1,
        /*www_authenticate_headers*/ &[],
    )
    .await?;

    let result = client
        .list_tools(
            /*params*/ None,
            /*timeout*/ Some(Duration::from_secs(5)),
        )
        .await?;

    assert_eq!(result, expected);

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn streamable_http_tools_list_retries_json_rpc_transient_status() -> anyhow::Result<()> {
    let (_server, base_url) = spawn_streamable_http_server().await?;
    let client = create_client(&base_url).await?;

    let expected = client
        .list_tools(
            /*params*/ None,
            /*timeout*/ Some(Duration::from_secs(5)),
        )
        .await?;
    arm_session_post_json_rpc_failure(&base_url, /*status*/ 502, /*remaining*/ 1).await?;

    let result = client
        .list_tools(
            /*params*/ None,
            /*timeout*/ Some(Duration::from_secs(5)),
        )
        .await?;

    assert_eq!(result, expected);

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn streamable_http_404_session_expiry_recovers_and_retries_once() -> anyhow::Result<()> {
    let (_server, base_url) = spawn_streamable_http_server().await?;
    let client = create_client(&base_url).await?;

    let warmup = call_echo_tool(&client, "warmup").await?;
    assert_eq!(warmup, expected_echo_result("warmup"));

    arm_session_post_failure(
        &base_url,
        /*status*/ 404,
        /*remaining*/ 1,
        /*www_authenticate_headers*/ &[],
    )
    .await?;

    let recovered = call_echo_tool(&client, "recovered").await?;
    assert_eq!(recovered, expected_echo_result("recovered"));

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn streamable_http_session_recovery_retries_initialize_failure() -> anyhow::Result<()> {
    let (_server, base_url) = spawn_streamable_http_server().await?;
    let http_client = FailFirstInitializeHttpClient::new(
        Environment::default_for_tests().get_http_client(),
        /*failures_remaining*/ 0,
    );
    let client = create_client_with_http_client(&base_url, Arc::new(http_client.clone())).await?;

    let warmup = call_echo_tool(&client, "warmup").await?;
    assert_eq!(warmup, expected_echo_result("warmup"));

    arm_session_post_failure(
        &base_url,
        /*status*/ 404,
        /*remaining*/ 1,
        /*www_authenticate_headers*/ &[],
    )
    .await?;
    http_client.fail_next_initialize();

    let recovered = call_echo_tool(&client, "recovered-after-retry").await?;
    assert_eq!(http_client.initialize_attempts(), 3);
    assert_eq!(recovered, expected_echo_result("recovered-after-retry"));

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
#[serial_test::serial]
async fn streamable_http_oauth_401_refreshes_recovers_and_retries_once() -> anyhow::Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let token_count_file = temp_dir.path().join("token-count");
    let token_count_file = token_count_file
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("token count file path must be utf-8"))?
        .to_string();
    let (_server, base_url) = spawn_streamable_http_server_with_env(&[
        ("MCP_EXPECT_BEARER", "initial-access-token"),
        (
            "MCP_OAUTH_REFRESH_ACCESS_TOKEN",
            "token-after-refresh-required",
        ),
        ("MCP_OAUTH_REFRESH_TOKEN", "refreshed-refresh-token"),
        ("MCP_OAUTH_TOKEN_COUNT_FILE", &token_count_file),
    ])
    .await?;
    let oauth_client =
        create_oauth_client(&base_url, "initial-access-token", "initial-refresh-token").await?;

    let warmup = call_echo_tool(&oauth_client.client, "warmup").await?;
    assert_eq!(warmup, expected_echo_result("warmup"));
    set_expected_bearer(&base_url, Some("token-after-refresh-required")).await?;

    arm_session_post_failure_for_method(
        &base_url,
        /*status*/ 401,
        /*remaining*/ 1,
        /*www_authenticate_headers*/
        &[r#"Bearer realm="OAuth", error="invalid_token", error_description="expired""#],
        Some("tools/call"),
    )
    .await?;

    let recovered = call_echo_tool(&oauth_client.client, "recovered").await?;
    assert_eq!(recovered, expected_echo_result("recovered"));
    assert_eq!(std::fs::read_to_string(token_count_file)?.trim(), "1");
    let credentials =
        std::fs::read_to_string(oauth_client.codex_home_path().join(".credentials.json"))?;
    assert!(credentials.contains("refreshed-refresh-token"));

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
#[serial_test::serial]
async fn streamable_http_oauth_401_reloads_newer_persisted_credentials_after_invalid_grant()
-> anyhow::Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let token_count_file = temp_dir.path().join("token-count");
    let token_count_file = token_count_file
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("token count file path must be utf-8"))?
        .to_string();
    let (_server, base_url) = spawn_streamable_http_server_with_env(&[
        ("MCP_EXPECT_BEARER", "initial-access-token"),
        ("MCP_OAUTH_TOKEN_ERROR", "invalid_grant"),
        ("MCP_OAUTH_TOKEN_COUNT_FILE", &token_count_file),
    ])
    .await?;
    let oauth_client =
        create_oauth_client(&base_url, "initial-access-token", "initial-refresh-token").await?;

    let warmup = call_echo_tool(&oauth_client.client, "warmup").await?;
    assert_eq!(warmup, expected_echo_result("warmup"));

    let server_url = format!("{base_url}/mcp");
    write_fallback_oauth_tokens(
        oauth_client.codex_home_path(),
        "test-streamable-http-oauth",
        &server_url,
        "externally-refreshed-access-token",
        "externally-refreshed-refresh-token",
        None,
    )?;
    set_expected_bearer(&base_url, Some("externally-refreshed-access-token")).await?;

    arm_session_post_failure_for_method(
        &base_url,
        /*status*/ 401,
        /*remaining*/ 1,
        /*www_authenticate_headers*/
        &[r#"Bearer realm="OAuth", error="invalid_token", error_description="expired""#],
        Some("tools/call"),
    )
    .await?;

    let recovered =
        call_echo_tool(&oauth_client.client, "recovered-after-external-refresh").await?;
    assert_eq!(
        recovered,
        expected_echo_result("recovered-after-external-refresh")
    );
    assert!(
        !std::path::Path::new(&token_count_file).exists(),
        "newer persisted credentials should be used without a refresh attempt"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
#[serial_test::serial]
async fn streamable_http_oauth_expired_startup_invalid_grant_deletes_credentials_and_requires_reauth()
-> anyhow::Result<()> {
    let codex_home = tempfile::tempdir()?;
    let (_server, base_url) = spawn_streamable_http_server_with_env(&[
        ("MCP_EXPECT_BEARER", "initial-access-token"),
        ("MCP_OAUTH_TOKEN_ERROR", "invalid_grant"),
    ])
    .await?;
    let server_url = format!("{base_url}/mcp");
    write_fallback_oauth_tokens(
        codex_home.path(),
        "test-streamable-http-oauth",
        &server_url,
        "expired-access-token",
        "invalid-refresh-token",
        Some(1),
    )?;
    let _codex_home_guard = EnvVarGuard::set("CODEX_HOME", codex_home.path().as_os_str());

    let error = match RmcpClient::new_streamable_http_client(
        "test-streamable-http-oauth",
        &server_url,
        /*bearer_token*/ None,
        /*http_headers*/ None,
        /*env_http_headers*/ None,
        OAuthCredentialsStoreMode::File,
        AuthKeyringBackendKind::default(),
        Environment::default_for_tests().get_http_client(),
        /*auth_provider*/ None,
    )
    .await
    {
        Ok(_) => anyhow::bail!("expected startup refresh to require reauthorization"),
        Err(error) => error,
    };
    let error_chain = format!("{error:#}");
    assert!(
        error_chain.contains("reauthorization is required"),
        "expected reauthorization-required error, got: {error:#}"
    );
    let credentials_path = codex_home.path().join(".credentials.json");
    if credentials_path.exists() {
        let credentials = std::fs::read_to_string(credentials_path)?;
        assert!(!credentials.contains("test-streamable-http-oauth"));
    }

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
#[serial_test::serial]
async fn streamable_http_oauth_concurrent_401_recovery_refreshes_once() -> anyhow::Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let token_count_file = temp_dir.path().join("token-count");
    let token_count_file = token_count_file
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("token count file path must be utf-8"))?
        .to_string();
    let (_server, base_url) = spawn_streamable_http_server_with_env(&[
        ("MCP_EXPECT_BEARER", "initial-access-token"),
        ("MCP_OAUTH_REFRESH_ACCESS_TOKEN", "initial-access-token"),
        (
            "MCP_OAUTH_REFRESH_TOKEN",
            "concurrent-refreshed-refresh-token",
        ),
        ("MCP_OAUTH_TOKEN_COUNT_FILE", &token_count_file),
    ])
    .await?;
    let oauth_client =
        create_oauth_client(&base_url, "initial-access-token", "initial-refresh-token").await?;

    let warmup = call_echo_tool(&oauth_client.client, "warmup").await?;
    assert_eq!(warmup, expected_echo_result("warmup"));

    arm_session_post_failure_for_method(
        &base_url,
        /*status*/ 401,
        /*remaining*/ 1,
        /*www_authenticate_headers*/
        &[r#"Bearer realm="OAuth", error="invalid_token", error_description="expired""#],
        Some("tools/call"),
    )
    .await?;

    let results = join_all([
        call_echo_tool(&oauth_client.client, "concurrent-a"),
        call_echo_tool(&oauth_client.client, "concurrent-b"),
        call_echo_tool(&oauth_client.client, "concurrent-c"),
    ])
    .await;
    for result in results {
        result?;
    }
    assert_eq!(std::fs::read_to_string(token_count_file)?.trim(), "1");

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn streamable_http_401_does_not_trigger_recovery() -> anyhow::Result<()> {
    let (_server, base_url) = spawn_streamable_http_server().await?;
    let client = create_client(&base_url).await?;

    let warmup = call_echo_tool(&client, "warmup").await?;
    assert_eq!(warmup, expected_echo_result("warmup"));

    arm_session_post_failure(
        &base_url,
        /*status*/ 401,
        /*remaining*/ 2,
        /*www_authenticate_headers*/ &[],
    )
    .await?;

    let first_error = call_echo_tool(&client, "unauthorized").await.unwrap_err();
    assert!(first_error.to_string().contains("401"));

    let second_error = call_echo_tool(&client, "still-unauthorized")
        .await
        .unwrap_err();
    assert!(second_error.to_string().contains("401"));

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn streamable_http_403_scope_challenge_returns_insufficient_scope() -> anyhow::Result<()> {
    let (_server, base_url) = spawn_streamable_http_server().await?;
    let client = create_client(&base_url).await?;

    let warmup = call_echo_tool(&client, "warmup").await?;
    assert_eq!(warmup, expected_echo_result("warmup"));

    arm_session_post_failure(
        &base_url,
        /*status*/ 403,
        /*remaining*/ 1,
        /*www_authenticate_headers*/
        &[r#"Bearer error="insufficient_scope", scope="files:read files:write""#],
    )
    .await?;

    let error = call_echo_tool(&client, "forbidden").await.unwrap_err();
    assert!(
        error.to_string().contains("Insufficient scope"),
        "expected insufficient-scope transport error, got: {error:#}"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn streamable_http_403_finds_bearer_challenge_in_later_header_value() -> anyhow::Result<()> {
    let (_server, base_url) = spawn_streamable_http_server().await?;
    let client = create_client(&base_url).await?;

    let warmup = call_echo_tool(&client, "warmup").await?;
    assert_eq!(warmup, expected_echo_result("warmup"));

    arm_session_post_failure(
        &base_url,
        /*status*/ 403,
        /*remaining*/ 1,
        /*www_authenticate_headers*/
        &[
            r#"Basic realm="example""#,
            r#"Bearer error="insufficient_scope", scope="files:read""#,
        ],
    )
    .await?;

    let error = call_echo_tool(&client, "forbidden").await.unwrap_err();
    assert!(
        error.to_string().contains("Insufficient scope"),
        "expected insufficient-scope transport error, got: {error:#}"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn streamable_http_404_recovery_only_retries_once() -> anyhow::Result<()> {
    let (_server, base_url) = spawn_streamable_http_server().await?;
    let client = create_client(&base_url).await?;

    let warmup = call_echo_tool(&client, "warmup").await?;
    assert_eq!(warmup, expected_echo_result("warmup"));

    arm_session_post_failure(
        &base_url,
        /*status*/ 404,
        /*remaining*/ 2,
        /*www_authenticate_headers*/ &[],
    )
    .await?;

    let error = call_echo_tool(&client, "double-404").await.unwrap_err();
    let error_message = error.to_string();
    assert!(
        error_message.contains("404") || error_message.contains("session expired"),
        "expected session-expiry error, got: {error:#}"
    );

    let recovered = call_echo_tool(&client, "after-double-404").await?;
    assert_eq!(recovered, expected_echo_result("after-double-404"));

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn streamable_http_non_session_failure_does_not_trigger_recovery() -> anyhow::Result<()> {
    let (_server, base_url) = spawn_streamable_http_server().await?;
    let client = create_client(&base_url).await?;

    let warmup = call_echo_tool(&client, "warmup").await?;
    assert_eq!(warmup, expected_echo_result("warmup"));

    arm_session_post_failure(
        &base_url,
        /*status*/ 500,
        /*remaining*/ 2,
        /*www_authenticate_headers*/ &[],
    )
    .await?;

    let first_error = call_echo_tool(&client, "server-error").await.unwrap_err();
    assert!(first_error.to_string().contains("500"));

    let second_error = call_echo_tool(&client, "still-server-error")
        .await
        .unwrap_err();
    assert!(second_error.to_string().contains("500"));

    Ok(())
}
