use super::*;
use codex_config::DEFAULT_MCP_SERVER_ENVIRONMENT_ID;
use pretty_assertions::assert_eq;

#[test]
fn remote_http_connections_track_host_headers_but_not_executor_bearer_tokens() {
    let mut config: McpServerConfig = serde_json::from_value(serde_json::json!({
        "url": "https://example.com/mcp",
        "environment_id": "executor-1",
        "bearer_token_env_var": "NODE_REPL_AUTH_TOKEN",
        "env_http_headers": {"X-Api-Key": "PATH"},
    }))
    .expect("remote MCP configuration should deserialize");

    assert_eq!(
        referenced_environment_variables(&config),
        vec![("PATH".to_string(), std::env::var_os("PATH"))],
    );

    let remote_host_bearer: McpServerConfig = serde_json::from_value(serde_json::json!({
        "url": "https://example.com/mcp",
        "environment_id": "executor-1",
        "bearer_token_env_var": "PATH",
    }))
    .expect("host-resolved remote MCP configuration should deserialize");
    assert_eq!(
        referenced_environment_variables(&remote_host_bearer),
        vec![("PATH".to_string(), std::env::var_os("PATH"))],
    );

    config.environment_id = DEFAULT_MCP_SERVER_ENVIRONMENT_ID.to_string();
    assert_eq!(
        referenced_environment_variables(&config),
        vec![
            (
                "NODE_REPL_AUTH_TOKEN".to_string(),
                std::env::var_os("NODE_REPL_AUTH_TOKEN"),
            ),
            ("PATH".to_string(), std::env::var_os("PATH")),
        ],
    );
}

#[test]
fn workload_attribution_changes_launch_env_without_changing_configured_identity() {
    let configured = EffectiveMcpServer::configured(test_server(McpServerTransportConfig::Stdio {
        command: "server".to_string(),
        args: Vec::new(),
        env: Some(HashMap::from([(
            "CONFIGURED".to_string(),
            "value".to_string(),
        )])),
        env_vars: Vec::new(),
        cwd: None,
    }));
    let thread_one = configured
        .clone()
        .with_stdio_workload_attribution("thread-1".to_string());
    let thread_two = configured.with_stdio_workload_attribution("thread-2".to_string());

    assert_eq!(thread_one.config(), thread_two.config());

    let thread_one = thread_one.with_applied_runtime_stdio_env();
    let thread_two = thread_two.with_applied_runtime_stdio_env();
    let McpServerTransportConfig::Stdio {
        env: thread_one_env,
        ..
    } = &thread_one.config().transport
    else {
        panic!("expected stdio transport");
    };
    let McpServerTransportConfig::Stdio {
        env: thread_two_env,
        ..
    } = &thread_two.config().transport
    else {
        panic!("expected stdio transport");
    };

    assert_eq!(
        thread_one_env,
        &Some(HashMap::from([
            ("CONFIGURED".to_string(), "value".to_string()),
            (
                CODEX_WORKLOAD_THREAD_ID_ENV.to_string(),
                "thread-1".to_string(),
            ),
            (CODEX_WORKLOAD_TYPE_ENV.to_string(), "mcp".to_string()),
        ]))
    );
    assert_eq!(
        thread_two_env,
        &Some(HashMap::from([
            ("CONFIGURED".to_string(), "value".to_string()),
            (
                CODEX_WORKLOAD_THREAD_ID_ENV.to_string(),
                "thread-2".to_string(),
            ),
            (CODEX_WORKLOAD_TYPE_ENV.to_string(), "mcp".to_string()),
        ]))
    );
}

#[test]
fn workload_attribution_overrides_configured_reserved_values() {
    let server = EffectiveMcpServer::configured(test_server(McpServerTransportConfig::Stdio {
        command: "server".to_string(),
        args: Vec::new(),
        env: Some(HashMap::from([(
            CODEX_WORKLOAD_THREAD_ID_ENV.to_string(),
            "configured-thread".to_string(),
        )])),
        env_vars: Vec::new(),
        cwd: None,
    }))
    .with_stdio_workload_attribution("runtime-thread".to_string())
    .with_applied_runtime_stdio_env();

    let McpServerTransportConfig::Stdio { env, .. } = &server.config().transport else {
        panic!("expected stdio transport");
    };
    assert_eq!(
        env,
        &Some(HashMap::from([
            (
                CODEX_WORKLOAD_THREAD_ID_ENV.to_string(),
                "runtime-thread".to_string(),
            ),
            (CODEX_WORKLOAD_TYPE_ENV.to_string(), "mcp".to_string()),
        ]))
    );
}

#[test]
fn workload_attribution_is_not_applied_to_http_servers() {
    let server =
        EffectiveMcpServer::configured(test_server(McpServerTransportConfig::StreamableHttp {
            url: "https://example.test/mcp".to_string(),
            bearer_token_env_var: None,
            http_headers: None,
            env_http_headers: None,
            http_headers_helper: None,
        }))
        .with_stdio_workload_attribution("thread-1".to_string())
        .with_applied_runtime_stdio_env();

    assert_eq!(
        server.config().transport,
        McpServerTransportConfig::StreamableHttp {
            url: "https://example.test/mcp".to_string(),
            bearer_token_env_var: None,
            http_headers: None,
            env_http_headers: None,
            http_headers_helper: None,
        }
    );
}

fn test_server(transport: McpServerTransportConfig) -> McpServerConfig {
    McpServerConfig {
        transport,
        auth: McpServerAuth::default(),
        environment_id: DEFAULT_MCP_SERVER_ENVIRONMENT_ID.to_string(),
        enabled: true,
        required: false,
        supports_parallel_tool_calls: false,
        omit_tools_from: None,
        disabled_reason: None,
        startup_timeout_sec: None,
        tool_timeout_sec: None,
        default_tools_approval_mode: None,
        enabled_tools: None,
        disabled_tools: None,
        scopes: None,
        oauth: None,
        oauth_resource: None,
        tools: HashMap::new(),
    }
}
