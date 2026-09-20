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
fn reserved_workload_key_overwrites_configured_spoof_value() {
    let spoofed = EffectiveMcpServer::configured(test_server(McpServerTransportConfig::Stdio {
        command: "server".to_string(),
        args: Vec::new(),
        env: Some(HashMap::from([(
            "CODEX_WORKLOAD_THREAD_ID".to_string(),
            "attacker-supplied".to_string(),
        )])),
        env_vars: Vec::new(),
        cwd: None,
    }))
    .with_stdio_runtime_env("CODEX_WORKLOAD_THREAD_ID", "real-thread".to_string());

    let McpServerTransportConfig::Stdio { env, .. } = &spoofed.config().transport else {
        panic!("expected stdio transport");
    };
    assert_eq!(
        env.as_ref()
            .and_then(|env| env.get("CODEX_WORKLOAD_THREAD_ID")),
        Some(&"real-thread".to_string()),
        "the reserved key must overwrite any configured value, never merge with it"
    );
}

#[test]
fn connection_identity_ignores_reserved_workload_attribution() {
    let base = McpServerTransportConfig::Stdio {
        command: "server".to_string(),
        args: Vec::new(),
        env: Some(HashMap::from([(
            "UNRELATED_KEY".to_string(),
            "kept".to_string(),
        )])),
        env_vars: Vec::new(),
        cwd: None,
    };
    let mut with_attribution_a = base.clone();
    let mut with_attribution_b = base.clone();
    if let McpServerTransportConfig::Stdio { env, .. } = &mut with_attribution_a {
        env.get_or_insert_with(HashMap::new).insert(
            "CODEX_WORKLOAD_THREAD_ID".to_string(),
            "thread-a".to_string(),
        );
    }
    if let McpServerTransportConfig::Stdio { env, .. } = &mut with_attribution_b {
        env.get_or_insert_with(HashMap::new).insert(
            "CODEX_WORKLOAD_THREAD_ID".to_string(),
            "thread-b".to_string(),
        );
    }

    let identity_a = transport_for_identity(&with_attribution_a);
    let identity_b = transport_for_identity(&with_attribution_b);
    assert_eq!(
        identity_a, identity_b,
        "differing workload attribution alone must not change connection identity"
    );
    let McpServerTransportConfig::Stdio { env, .. } = &identity_a else {
        panic!("expected stdio transport");
    };
    assert_eq!(
        env,
        &Some(HashMap::from([(
            "UNRELATED_KEY".to_string(),
            "kept".to_string()
        )])),
        "non-reserved configured env keys must survive identity stripping"
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
            http_headers_helper: None,
        }))
        .with_stdio_runtime_env("CODEX_WORKLOAD_THREAD_ID", "thread-1".to_string());
    assert_eq!(
        http.config().transport,
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
