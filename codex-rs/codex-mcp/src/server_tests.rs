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
fn runtime_attribution_is_added_only_to_stdio_servers() {
    let stdio = EffectiveMcpServer::configured(test_server(McpServerTransportConfig::Stdio {
        command: "server".to_string(),
        args: Vec::new(),
        env: None,
        env_vars: Vec::new(),
        cwd: None,
    }))
    .with_stdio_runtime_env("CODEX_WORKLOAD_THREAD_ID", "thread-1".to_string());

    let McpServerTransportConfig::Stdio { env, .. } = &stdio.config().transport else {
        panic!("expected stdio transport");
    };
    assert_eq!(
        env,
        &Some(HashMap::from([(
            "CODEX_WORKLOAD_THREAD_ID".to_string(),
            "thread-1".to_string(),
        )]))
    );

    let http =
        EffectiveMcpServer::configured(test_server(McpServerTransportConfig::StreamableHttp {
            url: "https://example.test/mcp".to_string(),
            bearer_token_env_var: None,
            http_headers: None,
            env_http_headers: None,
        }))
        .with_stdio_runtime_env("CODEX_WORKLOAD_THREAD_ID", "thread-1".to_string());
    assert_eq!(
        http.config().transport,
        McpServerTransportConfig::StreamableHttp {
            url: "https://example.test/mcp".to_string(),
            bearer_token_env_var: None,
            http_headers: None,
            env_http_headers: None,
        }
    );
}

fn test_server(transport: McpServerTransportConfig) -> McpServerConfig {
    McpServerConfig {
        transport,
        auth: McpServerAuth::default(),
        environment_id: codex_config::DEFAULT_MCP_SERVER_ENVIRONMENT_ID.to_string(),
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
