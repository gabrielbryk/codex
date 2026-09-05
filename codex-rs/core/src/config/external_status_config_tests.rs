use super::*;
use crate::config::Config;
use crate::config::ConfigOverrides;
use codex_config::ConfigLayerEntry;
use codex_config::ConfigLayerSource;
use codex_config::ConfigLayerStack;
use codex_config::config_toml::ConfigToml;
use codex_exec_server::LOCAL_FS;
use core_test_support::PathExt;
use core_test_support::TempDirExt;
use pretty_assertions::assert_eq;
use tempfile::tempdir;

#[tokio::test]
async fn runtime_config_resolves_status_line_command_with_default_timeout() {
    let executable = std::env::current_exe().expect("current executable");
    let executable_toml = toml::Value::String(executable.to_string_lossy().into_owned());
    let cfg: ConfigToml = toml::from_str(&format!(
        r#"
[tui.status_line_command]
command = [{executable_toml}, "--compact"]
"#,
    ))
    .expect("status line command should deserialize");

    let expected = TuiStatusLineCommand {
        command: vec![
            executable.to_string_lossy().into_owned(),
            "--compact".to_string(),
        ],
        timeout_ms: 5_000,
    };
    assert_eq!(
        cfg.tui
            .as_ref()
            .and_then(|tui| tui.status_line_command.as_ref()),
        Some(&expected)
    );

    let config = Config::load_from_base_config_with_overrides(
        cfg,
        ConfigOverrides::default(),
        tempdir().expect("tempdir").abs(),
    )
    .await
    .expect("status line command should resolve");

    assert_eq!(config.tui_status_line_command, Some(expected));
}

#[tokio::test]
async fn runtime_config_rejects_invalid_status_line_command() {
    let executable = toml::Value::String(
        std::env::current_exe()
            .expect("current executable")
            .to_string_lossy()
            .into_owned(),
    );
    let cases = vec![
        (
            "empty argv",
            r#"
[tui.status_line_command]
command = []
"#
            .to_string(),
            "`tui.status_line_command.command` must contain at least one argv element",
        ),
        (
            "relative executable",
            r#"
[tui.status_line_command]
command = ["statusline"]
"#
            .to_string(),
            "`tui.status_line_command.command[0]` must be an absolute executable path",
        ),
        (
            "timeout too small",
            format!(
                r#"
[tui.status_line_command]
command = [{executable}]
timeout_ms = 249
"#,
            ),
            "`tui.status_line_command.timeout_ms` must be between 250 and 30000 milliseconds",
        ),
        (
            "timeout too large",
            format!(
                r#"
[tui.status_line_command]
command = [{executable}]
timeout_ms = 30001
"#,
            ),
            "`tui.status_line_command.timeout_ms` must be between 250 and 30000 milliseconds",
        ),
    ];

    for (name, toml, expected_error) in cases {
        let cfg: ConfigToml = toml::from_str(&toml)
            .unwrap_or_else(|error| panic!("{name} should deserialize: {error}"));
        let error = Config::load_from_base_config_with_overrides(
            cfg,
            ConfigOverrides::default(),
            tempdir().expect("tempdir").abs(),
        )
        .await
        .expect_err(name);

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData, "{name}");
        assert_eq!(error.to_string(), expected_error, "{name}");
    }
}

#[tokio::test]
async fn runtime_config_rejects_status_line_and_command_together() {
    let cfg: ConfigToml = toml::from_str(
        r#"
[tui]
status_line = ["model"]

[tui.status_line_command]
command = ["/statusline"]
"#,
    )
    .expect("conflicting status line settings should deserialize");

    let error = Config::load_from_base_config_with_overrides(
        cfg,
        ConfigOverrides::default(),
        tempdir().expect("tempdir").abs(),
    )
    .await
    .expect_err("conflicting status line settings should be rejected");

    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    assert_eq!(
        error.to_string(),
        "`tui.status_line` and `tui.status_line_command` are mutually exclusive at the same precedence; remove one"
    );
}

#[tokio::test]
async fn higher_priority_external_command_overrides_project_builtin_status_line() {
    let codex_home = tempdir().expect("tempdir");
    let project_dot_codex = codex_home.path().join("project/.codex").abs();
    let executable = std::env::current_exe()
        .expect("current executable")
        .to_string_lossy()
        .into_owned();
    let config_layer_stack = ConfigLayerStack::new(
        vec![
            ConfigLayerEntry::new(
                ConfigLayerSource::Project {
                    dot_codex_folder: project_dot_codex,
                },
                toml::toml! {
                    [tui]
                    status_line = ["model"]
                }
                .into(),
            ),
            ConfigLayerEntry::new(
                ConfigLayerSource::LegacyManagedConfigTomlFromMdm,
                toml::Value::Table(toml::map::Map::from_iter([(
                    "tui".to_string(),
                    toml::Value::Table(toml::map::Map::from_iter([(
                        "status_line_command".to_string(),
                        toml::Value::Table(toml::map::Map::from_iter([(
                            "command".to_string(),
                            toml::Value::Array(vec![toml::Value::String(executable.clone())]),
                        )])),
                    )])),
                )])),
            ),
        ],
        Default::default(),
        Default::default(),
    )
    .expect("config layers should be valid before effective-value validation");
    let cfg: ConfigToml = config_layer_stack
        .effective_config()
        .try_into()
        .expect("merged config should deserialize");

    let config = Config::load_config_with_layer_stack(
        LOCAL_FS.as_ref(),
        cfg,
        ConfigOverrides::default(),
        codex_home.abs(),
        config_layer_stack,
    )
    .await
    .expect("higher priority command should win");

    assert_eq!(config.tui_status_line, None);
    assert_eq!(
        config.tui_status_line_command,
        Some(TuiStatusLineCommand {
            command: vec![executable],
            timeout_ms: 5_000,
        })
    );
}
