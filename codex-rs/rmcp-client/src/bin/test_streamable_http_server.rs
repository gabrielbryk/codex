use std::borrow::Cow;
use std::collections::HashMap;
use std::fs;
use std::io::ErrorKind;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::body::Bytes;
use axum::body::to_bytes;
use axum::extract::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::http::HeaderValue;
use axum::http::Method;
use axum::http::Request;
use axum::http::StatusCode;
use axum::http::header::AUTHORIZATION;
use axum::http::header::CONTENT_TYPE;
use axum::http::header::HOST;
use axum::http::header::WWW_AUTHENTICATE;
use axum::middleware;
use axum::middleware::Next;
use axum::response::Response;
use axum::routing::get;
use axum::routing::post;
use rmcp::ErrorData as McpError;
use rmcp::handler::server::ServerHandler;
use rmcp::model::CallToolRequestParams;
use rmcp::model::CallToolResult;
use rmcp::model::JsonObject;
use rmcp::model::ListResourceTemplatesResult;
use rmcp::model::ListResourcesResult;
use rmcp::model::ListToolsResult;
use rmcp::model::PaginatedRequestParams;
use rmcp::model::RawResource;
use rmcp::model::RawResourceTemplate;
use rmcp::model::ReadResourceRequestParams;
use rmcp::model::ReadResourceResult;
use rmcp::model::Resource;
use rmcp::model::ResourceContents;
use rmcp::model::ResourceTemplate;
use rmcp::model::ServerCapabilities;
use rmcp::model::ServerInfo;
use rmcp::model::Tool;
use rmcp::model::ToolAnnotations;
use rmcp::transport::StreamableHttpServerConfig;
use rmcp::transport::StreamableHttpService;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;
use tokio::sync::Mutex;
use tokio::task;
use tokio::time::sleep;

#[derive(Clone)]
struct TestToolServer {
    tools: Arc<Vec<Tool>>,
    resources: Arc<Vec<Resource>>,
    resource_templates: Arc<Vec<ResourceTemplate>>,
}

const MEMO_URI: &str = "memo://codex/example-note";
const MEMO_CONTENT: &str = "This is a sample MCP resource served by the rmcp test server.";
const MCP_SESSION_ID_HEADER: &str = "mcp-session-id";
const SESSION_POST_FAILURE_CONTROL_PATH: &str = "/test/control/session-post-failure";
const INITIALIZE_POST_FAILURE_CONTROL_PATH: &str = "/test/control/initialize-post-failure";
const INITIALIZED_NOTIFICATION_POST_FAILURE_CONTROL_PATH: &str =
    "/test/control/initialized-notification-post-failure";
const EXPECTED_BEARER_CONTROL_PATH: &str = "/test/control/expected-bearer";
const MAX_MCP_POST_BODY_BYTES: usize = 1024 * 1024;

#[derive(Clone, Default)]
struct AppState {
    post_failure: PostFailureState,
    expected_bearer: Arc<Mutex<Option<String>>>,
}

#[derive(Clone, Default)]
struct PostFailureState {
    armed_failure: Arc<Mutex<Option<ArmedFailure>>>,
}

#[derive(Clone, Copy, Debug)]
enum ArmedFailureTarget {
    Initialize,
    InitializedNotification,
    Session,
}

#[derive(Clone, Debug)]
struct ArmedFailure {
    target: ArmedFailureTarget,
    status: StatusCode,
    remaining: usize,
    /// Raw `WWW-Authenticate` challenge header field values returned with the failure.
    www_authenticate_headers: Vec<HeaderValue>,
    content_type: Option<HeaderValue>,
    body: Option<String>,
    mcp_method: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ArmSessionPostFailureRequest {
    status: u16,
    remaining: usize,
    /// Raw `WWW-Authenticate` challenge header field values to add to the failure.
    #[serde(default)]
    www_authenticate_headers: Vec<String>,
    content_type: Option<String>,
    body: Option<String>,
    #[serde(default)]
    mcp_method: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SetExpectedBearerRequest {
    token: Option<String>,
}

#[derive(Deserialize)]
struct EchoArgs {
    message: String,
    #[allow(dead_code)]
    env_var: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let bind_addr = parse_bind_addr()?;
    let app_state = AppState {
        post_failure: PostFailureState::default(),
        expected_bearer: Arc::new(Mutex::new(
            std::env::var("MCP_EXPECT_BEARER")
                .ok()
                .map(|token| format!("Bearer {token}")),
        )),
    };
    const MAX_BIND_RETRIES: u32 = 20;
    const BIND_RETRY_DELAY: Duration = Duration::from_millis(50);

    let mut bind_retries = 0;
    let listener = loop {
        match tokio::net::TcpListener::bind(&bind_addr).await {
            Ok(listener) => break listener,
            Err(err) if err.kind() == ErrorKind::PermissionDenied => {
                eprintln!(
                    "failed to bind to {bind_addr}: {err}. make sure the process has network access"
                );
                return Ok(());
            }
            Err(err) if err.kind() == ErrorKind::AddrInUse && bind_retries < MAX_BIND_RETRIES => {
                bind_retries += 1;
                sleep(BIND_RETRY_DELAY).await;
            }
            Err(err) => return Err(err.into()),
        }
    };
    let actual_bind_addr = listener.local_addr()?;
    if let Ok(bound_addr_file) = std::env::var("MCP_STREAMABLE_HTTP_BOUND_ADDR_FILE") {
        fs::write(bound_addr_file, actual_bind_addr.to_string())?;
    }
    eprintln!("starting rmcp streamable http test server on http://{actual_bind_addr}/mcp");

    let router = Router::new()
        .route(
            SESSION_POST_FAILURE_CONTROL_PATH,
            post(arm_session_post_failure),
        )
        .route(
            INITIALIZE_POST_FAILURE_CONTROL_PATH,
            post(arm_initialize_post_failure),
        )
        .route(
            INITIALIZED_NOTIFICATION_POST_FAILURE_CONTROL_PATH,
            post(arm_initialized_notification_post_failure),
        )
        .route(EXPECTED_BEARER_CONTROL_PATH, post(set_expected_bearer))
        .route(
            "/.well-known/oauth-authorization-server/mcp",
            get({
                move |headers: HeaderMap| async move {
                    let metadata_base = headers
                        .get(HOST)
                        .and_then(|value| value.to_str().ok())
                        .map(|host| format!("http://{host}"))
                        .unwrap_or_else(|| format!("http://{actual_bind_addr}"));
                    #[expect(clippy::expect_used)]
                    Response::builder()
                        .status(StatusCode::OK)
                        .header(CONTENT_TYPE, "application/json")
                        .body(Body::from(
                            serde_json::to_vec(&json!({
                                "authorization_endpoint": format!("{metadata_base}/oauth/authorize"),
                                "token_endpoint": format!("{metadata_base}/oauth/token"),
                                "scopes_supported": [""],
                            })).expect("failed to serialize metadata"),
                        ))
                        .expect("valid metadata response")
                }
            }),
        )
        .route("/oauth/token", post(oauth_token))
        .nest_service(
            "/mcp",
            StreamableHttpService::new(
                || Ok(TestToolServer::new()),
                Arc::new(LocalSessionManager::default()),
                StreamableHttpServerConfig::default(),
            ),
        )
        .layer(middleware::from_fn_with_state(
            app_state.clone(),
            require_bearer,
        ))
        .layer(middleware::from_fn_with_state(
            app_state.clone(),
            fail_mcp_post_when_armed,
        ))
        .with_state(app_state);

    axum::serve(listener, router).await?;
    task::yield_now().await;
    Ok(())
}

impl ServerHandler for TestToolServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_tool_list_changed()
                .enable_resources()
                .build(),
        )
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: rmcp::service::RequestContext<rmcp::service::RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListToolsResult, McpError>> + Send + '_ {
        let tools = self.tools.clone();
        async move {
            Ok(ListToolsResult {
                tools: (*tools).clone(),
                next_cursor: None,
                meta: None,
            })
        }
    }

    fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: rmcp::service::RequestContext<rmcp::service::RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListResourcesResult, McpError>> + Send + '_ {
        let resources = self.resources.clone();
        async move {
            Ok(ListResourcesResult {
                resources: (*resources).clone(),
                next_cursor: None,
                meta: None,
            })
        }
    }

    async fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: rmcp::service::RequestContext<rmcp::service::RoleServer>,
    ) -> Result<ListResourceTemplatesResult, McpError> {
        Ok(ListResourceTemplatesResult {
            resource_templates: (*self.resource_templates).clone(),
            next_cursor: None,
            meta: None,
        })
    }

    async fn read_resource(
        &self,
        ReadResourceRequestParams { uri, .. }: ReadResourceRequestParams,
        _context: rmcp::service::RequestContext<rmcp::service::RoleServer>,
    ) -> Result<ReadResourceResult, McpError> {
        if uri == MEMO_URI {
            Ok(ReadResourceResult::new(vec![
                ResourceContents::TextResourceContents {
                    uri,
                    mime_type: Some("text/plain".to_string()),
                    text: Self::memo_text().to_string(),
                    meta: None,
                },
            ]))
        } else {
            Err(McpError::resource_not_found(
                "resource_not_found",
                Some(json!({ "uri": uri })),
            ))
        }
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: rmcp::service::RequestContext<rmcp::service::RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        match request.name.as_ref() {
            "echo" => {
                let args: EchoArgs = match request.arguments {
                    Some(arguments) => serde_json::from_value(serde_json::Value::Object(
                        arguments.into_iter().collect(),
                    ))
                    .map_err(|err| McpError::invalid_params(err.to_string(), None))?,
                    None => {
                        return Err(McpError::invalid_params(
                            "missing arguments for echo tool",
                            None,
                        ));
                    }
                };

                let env_snapshot: HashMap<String, String> = std::env::vars().collect();
                let structured_content = json!({
                    "echo": format!("ECHOING: {}", args.message),
                    "env": env_snapshot.get("MCP_TEST_VALUE"),
                });

                let mut result = CallToolResult::success(Vec::new());
                result.structured_content = Some(structured_content);
                Ok(result)
            }
            other => Err(McpError::invalid_params(
                format!("unknown tool: {other}"),
                None,
            )),
        }
    }
}

impl TestToolServer {
    fn new() -> Self {
        let tools = vec![Self::echo_tool()];
        let resources = vec![Self::memo_resource()];
        let resource_templates = vec![Self::memo_template()];
        Self {
            tools: Arc::new(tools),
            resources: Arc::new(resources),
            resource_templates: Arc::new(resource_templates),
        }
    }

    fn echo_tool() -> Tool {
        #[expect(clippy::expect_used)]
        let schema: JsonObject = serde_json::from_value(json!({
            "type": "object",
            "properties": {
                "message": { "type": "string" },
                "env_var": { "type": "string" }
            },
            "required": ["message"],
            "additionalProperties": false
        }))
        .expect("echo tool schema should deserialize");

        let mut tool = Tool::new(
            Cow::Borrowed("echo"),
            Cow::Borrowed("Echo back the provided message and include environment data."),
            Arc::new(schema),
        );
        #[expect(clippy::expect_used)]
        let output_schema: JsonObject = serde_json::from_value(json!({
            "type": "object",
            "properties": {
                "echo": { "type": "string" },
                "env": {
                    "anyOf": [
                        { "type": "string" },
                        { "type": "null" }
                    ]
                }
            },
            "required": ["echo", "env"],
            "additionalProperties": false
        }))
        .expect("echo tool output schema should deserialize");
        tool.output_schema = Some(Arc::new(output_schema));
        tool.annotations = Some(ToolAnnotations::new().read_only(true));
        tool
    }

    fn memo_resource() -> Resource {
        let raw = RawResource {
            uri: MEMO_URI.to_string(),
            name: "example-note".to_string(),
            title: Some("Example Note".to_string()),
            description: Some("A sample MCP resource exposed for integration tests.".to_string()),
            mime_type: Some("text/plain".to_string()),
            size: None,
            icons: None,
            meta: None,
        };
        Resource::new(raw, None)
    }

    fn memo_template() -> ResourceTemplate {
        let raw = RawResourceTemplate {
            uri_template: "memo://codex/{slug}".to_string(),
            name: "codex-memo".to_string(),
            title: Some("Codex Memo".to_string()),
            description: Some(
                "Template for memo://codex/{slug} resources used in tests.".to_string(),
            ),
            mime_type: Some("text/plain".to_string()),
            icons: None,
        };
        ResourceTemplate::new(raw, None)
    }

    fn memo_text() -> &'static str {
        MEMO_CONTENT
    }
}

fn parse_bind_addr() -> Result<SocketAddr, Box<dyn std::error::Error>> {
    let default_addr = "127.0.0.1:3920";
    let bind_addr = std::env::var("MCP_STREAMABLE_HTTP_BIND_ADDR")
        .or_else(|_| std::env::var("BIND_ADDR"))
        .unwrap_or_else(|_| default_addr.to_string());
    Ok(bind_addr.parse()?)
}

async fn require_bearer(
    State(state): State<AppState>,
    mut request: Request<Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    if request.uri().path().contains("/.well-known/")
        || request.uri().path().starts_with("/oauth/")
        || request.uri().path().starts_with("/test/control/")
        || (request.uri().path() == "/mcp"
            && !request.headers().contains_key(MCP_SESSION_ID_HEADER))
    {
        return Ok(next.run(request).await);
    }
    let expected = state.expected_bearer.lock().await.clone();
    let Some(expected) = expected else {
        return Ok(next.run(request).await);
    };
    if request.uri().path() == "/mcp"
        && request.method() == Method::POST
        && request.headers().contains_key(MCP_SESSION_ID_HEADER)
    {
        let (preserved_request, is_initialized_notification) =
            match request_matches_mcp_method(request, "notifications/initialized").await {
                Ok(result) => result,
                Err(response) => return Ok(response),
            };
        request = preserved_request;
        if is_initialized_notification {
            return Ok(next.run(request).await);
        }
    }
    if request
        .headers()
        .get(AUTHORIZATION)
        .is_some_and(|value| value.as_bytes() == expected.as_bytes())
    {
        Ok(next.run(request).await)
    } else {
        #[expect(clippy::expect_used)]
        Ok(Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .header(
                WWW_AUTHENTICATE,
                r#"Bearer realm="OAuth", error="invalid_token""#,
            )
            .body(Body::from("missing or invalid bearer token"))
            .expect("valid bearer challenge response"))
    }
}

async fn oauth_token(body: Bytes) -> Response {
    if let Ok(count_file) = std::env::var("MCP_OAUTH_TOKEN_COUNT_FILE") {
        let count = fs::read_to_string(&count_file)
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok())
            .unwrap_or(0)
            + 1;
        if let Err(err) = fs::write(&count_file, count.to_string()) {
            eprintln!("failed to write OAuth token count file {count_file}: {err}");
        }
    }

    let body = String::from_utf8_lossy(&body);
    if let Ok(required_refresh_token) = std::env::var("MCP_OAUTH_VALID_REFRESH_TOKEN")
        && !body.contains(&format!("refresh_token={required_refresh_token}"))
    {
        return oauth_error_response("invalid_grant", "refresh token is invalid");
    }
    if let Ok(error) = std::env::var("MCP_OAUTH_TOKEN_ERROR") {
        let description = std::env::var("MCP_OAUTH_TOKEN_ERROR_DESCRIPTION")
            .unwrap_or_else(|_| "forced OAuth token error".to_string());
        return oauth_error_response(&error, &description);
    }

    let access_token = std::env::var("MCP_OAUTH_REFRESH_ACCESS_TOKEN")
        .or_else(|_| std::env::var("MCP_EXPECT_BEARER"))
        .unwrap_or_else(|_| "test-bearer".to_string());
    let refresh_token = std::env::var("MCP_OAUTH_REFRESH_TOKEN")
        .unwrap_or_else(|_| "refreshed-refresh-token".to_string());

    #[expect(clippy::expect_used)]
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({
                "access_token": access_token,
                "refresh_token": refresh_token,
                "token_type": "bearer",
                "expires_in": 3600,
            }))
            .expect("failed to serialize OAuth token response"),
        ))
        .expect("valid OAuth token response")
}

async fn arm_session_post_failure(
    State(state): State<AppState>,
    Json(request): Json<ArmSessionPostFailureRequest>,
) -> Result<StatusCode, StatusCode> {
    arm_post_failure(state.post_failure, request, ArmedFailureTarget::Session).await
}

async fn arm_initialize_post_failure(
    State(state): State<AppState>,
    Json(request): Json<ArmSessionPostFailureRequest>,
) -> Result<StatusCode, StatusCode> {
    arm_post_failure(state.post_failure, request, ArmedFailureTarget::Initialize).await
}

async fn arm_initialized_notification_post_failure(
    State(state): State<AppState>,
    Json(request): Json<ArmSessionPostFailureRequest>,
) -> Result<StatusCode, StatusCode> {
    arm_post_failure(
        state.post_failure,
        request,
        ArmedFailureTarget::InitializedNotification,
    )
    .await
}

async fn arm_post_failure(
    state: PostFailureState,
    request: ArmSessionPostFailureRequest,
    target: ArmedFailureTarget,
) -> Result<StatusCode, StatusCode> {
    let status = StatusCode::from_u16(request.status).map_err(|_| StatusCode::BAD_REQUEST)?;
    let www_authenticate_headers = request
        .www_authenticate_headers
        .into_iter()
        .map(|value| HeaderValue::from_str(&value).map_err(|_| StatusCode::BAD_REQUEST))
        .collect::<Result<Vec<_>, _>>()?;
    let content_type = request
        .content_type
        .map(|value| HeaderValue::from_str(&value).map_err(|_| StatusCode::BAD_REQUEST))
        .transpose()?;
    let armed_failure = if request.remaining == 0 {
        None
    } else {
        Some(ArmedFailure {
            target,
            status,
            remaining: request.remaining,
            www_authenticate_headers,
            content_type,
            body: request.body,
            mcp_method: request.mcp_method,
        })
    };
    *state.armed_failure.lock().await = armed_failure;
    Ok(StatusCode::NO_CONTENT)
}

async fn set_expected_bearer(
    State(state): State<AppState>,
    Json(request): Json<SetExpectedBearerRequest>,
) -> Result<StatusCode, StatusCode> {
    *state.expected_bearer.lock().await = request.token.map(|token| format!("Bearer {token}"));
    Ok(StatusCode::NO_CONTENT)
}

async fn fail_mcp_post_when_armed(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if request.uri().path() != "/mcp" || request.method() != Method::POST {
        return next.run(request).await;
    }
    let (parts, body) = request.into_parts();
    let body_bytes = match to_bytes(body, MAX_MCP_POST_BODY_BYTES).await {
        Ok(body_bytes) => body_bytes,
        Err(_) => {
            let mut response = Response::new(Body::from("failed to read request body"));
            *response.status_mut() = StatusCode::BAD_REQUEST;
            return response;
        }
    };
    let has_session_id = parts.headers.contains_key(MCP_SESSION_ID_HEADER);
    let mcp_method = request_mcp_method(&body_bytes);

    {
        let mut armed_failure = state.post_failure.armed_failure.lock().await;
        if let Some(failure) = armed_failure.as_mut()
            && failure.remaining > 0
            && match failure.target {
                ArmedFailureTarget::Initialize => !has_session_id,
                ArmedFailureTarget::InitializedNotification => {
                    has_session_id && mcp_method.as_deref() == Some("notifications/initialized")
                }
                ArmedFailureTarget::Session => {
                    has_session_id
                        && mcp_method.as_deref() != Some("notifications/initialized")
                        && failure
                            .mcp_method
                            .as_deref()
                            .is_none_or(|expected| Some(expected) == mcp_method.as_deref())
                }
            }
        {
            failure.remaining -= 1;
            let status = failure.status;
            let www_authenticate_headers = failure.www_authenticate_headers.clone();
            let content_type = failure.content_type.clone();
            let body = failure
                .body
                .clone()
                .unwrap_or_else(|| format!("forced session failure with status {status}"));
            if failure.remaining == 0 {
                *armed_failure = None;
            }
            let mut response = Response::new(Body::from(body));
            *response.status_mut() = status;
            if let Some(content_type) = content_type {
                response.headers_mut().insert(CONTENT_TYPE, content_type);
            }
            for www_authenticate_header in www_authenticate_headers {
                response
                    .headers_mut()
                    .append(WWW_AUTHENTICATE, www_authenticate_header);
            }
            return response;
        }
    }

    next.run(Request::from_parts(parts, Body::from(body_bytes)))
        .await
}

fn request_mcp_method(body: &[u8]) -> Option<String> {
    serde_json::from_slice::<Value>(body)
        .ok()?
        .get("method")?
        .as_str()
        .map(ToString::to_string)
}

async fn request_matches_mcp_method(
    request: Request<Body>,
    expected_method: &str,
) -> Result<(Request<Body>, bool), Response> {
    let (parts, body) = request.into_parts();
    let bytes = match to_bytes(body, MAX_MCP_POST_BODY_BYTES).await {
        Ok(bytes) => bytes,
        Err(error) => {
            let mut response = Response::new(Body::from(format!(
                "failed to inspect MCP request body: {error}"
            )));
            *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
            return Err(response);
        }
    };
    let matches_method = request_mcp_method(&bytes).is_some_and(|method| method == expected_method);
    Ok((
        Request::from_parts(parts, Body::from(bytes)),
        matches_method,
    ))
}

fn oauth_error_response(error: &str, error_description: &str) -> Response {
    #[expect(clippy::expect_used)]
    Response::builder()
        .status(StatusCode::BAD_REQUEST)
        .header(CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({
                "error": error,
                "error_description": error_description,
            }))
            .expect("failed to serialize OAuth error response"),
        ))
        .expect("valid OAuth error response")
}
