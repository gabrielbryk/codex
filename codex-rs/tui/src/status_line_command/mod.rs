//! Bounded execution for user-configured TUI status formatters.

pub(crate) mod process;
pub(crate) mod runner;
pub(crate) mod wire;

const MAX_OUTPUT_BYTES: usize = 4 * 1024;
const MAX_STATUS_CELLS: usize = 256;

fn parse_output(output: &[u8]) -> Result<String, &'static str> {
    if output.len() > MAX_OUTPUT_BYTES {
        return Err("formatter output exceeded 4096 bytes");
    }
    let text = std::str::from_utf8(output).map_err(|_| "formatter output was not UTF-8")?;
    let text = text.strip_suffix('\n').unwrap_or(text);
    let text = text.strip_suffix('\r').unwrap_or(text);
    if text.is_empty() {
        return Err("formatter output was empty");
    }
    if text.chars().any(char::is_control) {
        return Err("formatter output contained terminal controls or multiple rows");
    }
    if unicode_width::UnicodeWidthStr::width(text) > MAX_STATUS_CELLS {
        return Err("formatter output exceeded 256 terminal cells");
    }
    Ok(text.to_string())
}

#[cfg(test)]
#[path = "parser_tests.rs"]
mod parser_tests;
