use codex_config::ConfigLayerSource;
use codex_config::ConfigLayerStack;
use codex_config::format_config_layer_source;
use codex_config::types::MAX_TUI_STATUS_LINE_COMMAND_TIMEOUT_MS;
use codex_config::types::MIN_TUI_STATUS_LINE_COMMAND_TIMEOUT_MS;
use codex_config::types::TuiStatusLineCommand;
use std::path::Path;

pub(super) fn resolve_tui_status_line_config(
    tui: Option<&codex_config::types::Tui>,
    config_layer_stack: &ConfigLayerStack,
) -> std::io::Result<(Option<Vec<String>>, Option<TuiStatusLineCommand>)> {
    let Some(tui) = tui else {
        return Ok((None, None));
    };

    let mut status_line = tui.status_line.clone();
    let mut status_line_command = tui.status_line_command.clone();
    if status_line.is_some() && status_line_command.is_some() {
        let source = |key: &str| {
            config_layer_stack
                .layers_high_to_low()
                .enumerate()
                .find(|(_, layer)| {
                    layer
                        .config
                        .get("tui")
                        .and_then(toml::Value::as_table)
                        .is_some_and(|tui| tui.contains_key(key))
                })
                .map(|(precedence, layer)| (precedence, &layer.name))
        };
        match (source("status_line"), source("status_line_command")) {
            (Some((status_precedence, _)), Some((command_precedence, _)))
                if status_precedence < command_precedence =>
            {
                status_line_command = None;
            }
            (Some((status_precedence, _)), Some((command_precedence, _)))
                if command_precedence < status_precedence =>
            {
                status_line = None;
            }
            (status_source, command_source) => {
                let describe = |source: Option<(usize, &ConfigLayerSource)>| {
                    source
                        .map(|(_, source)| {
                            format!(
                                " from {}",
                                format_config_layer_source(source, codex_config::CONFIG_TOML_FILE,)
                            )
                        })
                        .unwrap_or_default()
                };
                let status_source = describe(status_source);
                let command_source = describe(command_source);
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!(
                        "`tui.status_line`{status_source} and `tui.status_line_command`{command_source} are mutually exclusive at the same precedence; remove one"
                    ),
                ));
            }
        }
    }

    let Some(command) = status_line_command.as_ref() else {
        return Ok((status_line, None));
    };
    if command.command.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "`tui.status_line_command.command` must contain at least one argv element",
        ));
    }
    if !Path::new(&command.command[0]).is_absolute() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "`tui.status_line_command.command[0]` must be an absolute executable path",
        ));
    }
    if !(MIN_TUI_STATUS_LINE_COMMAND_TIMEOUT_MS..=MAX_TUI_STATUS_LINE_COMMAND_TIMEOUT_MS)
        .contains(&command.timeout_ms)
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "`tui.status_line_command.timeout_ms` must be between {MIN_TUI_STATUS_LINE_COMMAND_TIMEOUT_MS} and {MAX_TUI_STATUS_LINE_COMMAND_TIMEOUT_MS} milliseconds"
            ),
        ));
    }

    Ok((status_line, status_line_command))
}

#[cfg(test)]
#[path = "external_status_config_tests.rs"]
mod tests;
