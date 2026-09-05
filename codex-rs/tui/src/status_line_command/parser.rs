//! Bounded parser for formatter stdout.
//!
//! Raw escape sequences never reach ratatui. The parser accepts a deliberately
//! small SGR color/style language and OSC 8 links with HTTPS destinations, and
//! rejects every other terminal control sequence.

use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Span;
use thiserror::Error;
use url::Url;

use crate::terminal_hyperlinks::HyperlinkLine;
use crate::width::char_width;
use crate::width::display_width;

pub(crate) const MAX_STATUS_LINE_COMMAND_BYTES: usize = 8 * 1024;
pub(crate) const MAX_STATUS_LINE_COMMAND_ROWS: usize = 3;
const MAX_HYPERLINK_URL_BYTES: usize = 2048;
const MAX_HYPERLINK_LABEL_CELLS: usize = 512;
const MAX_OSC_PARAMS_BYTES: usize = 128;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ParsedStatusLine {
    pub(crate) lines: Vec<HyperlinkLine>,
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub(crate) enum StatusLineCommandParseError {
    #[error("formatter stdout exceeded {MAX_STATUS_LINE_COMMAND_BYTES} bytes")]
    OutputTooLarge,
    #[error("formatter stdout was not valid UTF-8")]
    InvalidUtf8,
    #[error("formatter stdout did not contain visible text")]
    EmptyOutput,
    #[error("formatter stdout contained a bare carriage return")]
    BareCarriageReturn,
    #[error("formatter stdout contained a forbidden control character U+{0:04X}")]
    ForbiddenControl(u32),
    #[error("formatter stdout contained a forbidden invisible character U+{0:04X}")]
    ForbiddenInvisible(u32),
    #[error("formatter stdout contained an unsupported escape sequence")]
    UnsupportedEscape,
    #[error("formatter stdout contained a malformed ANSI SGR sequence")]
    MalformedSgr,
    #[error("formatter stdout contained an unsupported ANSI SGR parameter {0}")]
    UnsupportedSgr(u16),
    #[error("formatter stdout contained a malformed OSC 8 hyperlink")]
    MalformedHyperlink,
    #[error("formatter stdout contained a non-HTTPS hyperlink")]
    UnsafeHyperlink,
    #[error("formatter stdout hyperlink URL exceeded {MAX_HYPERLINK_URL_BYTES} bytes")]
    HyperlinkUrlTooLong,
    #[error("formatter stdout hyperlink label exceeded {MAX_HYPERLINK_LABEL_CELLS} cells")]
    HyperlinkLabelTooLong,
    #[error("formatter stdout ended with an open OSC 8 hyperlink")]
    UnterminatedHyperlink,
}

/// Parse bounded formatter stdout into visible lines plus semantic hyperlinks.
pub(crate) fn parse_status_line_command_output(
    output: &[u8],
) -> Result<ParsedStatusLine, StatusLineCommandParseError> {
    if output.len() > MAX_STATUS_LINE_COMMAND_BYTES {
        return Err(StatusLineCommandParseError::OutputTooLarge);
    }
    let output =
        std::str::from_utf8(output).map_err(|_| StatusLineCommandParseError::InvalidUtf8)?;
    Parser::new(output).parse()
}

#[derive(Debug)]
struct Parser<'a> {
    input: &'a str,
    offset: usize,
    style: Style,
    hyperlink: Option<String>,
    hyperlink_label: String,
    lines: Vec<HyperlinkLine>,
    current_line: HyperlinkLine,
    current_text: String,
    current_text_style: Style,
    current_text_hyperlink: Option<String>,
    saw_visible_text: bool,
    ended_with_newline: bool,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            input,
            offset: 0,
            style: Style::default(),
            hyperlink: None,
            hyperlink_label: String::new(),
            lines: Vec::new(),
            current_line: HyperlinkLine::default(),
            current_text: String::new(),
            current_text_style: Style::default(),
            current_text_hyperlink: None,
            saw_visible_text: false,
            ended_with_newline: false,
        }
    }

    fn parse(mut self) -> Result<ParsedStatusLine, StatusLineCommandParseError> {
        while self.offset < self.input.len() {
            let remaining = &self.input[self.offset..];
            if remaining.starts_with("\r\n") {
                self.offset += 2;
                self.finish_line();
                self.ended_with_newline = true;
                continue;
            }

            let Some(ch) = remaining.chars().next() else {
                return Err(StatusLineCommandParseError::InvalidUtf8);
            };
            match ch {
                '\r' => return Err(StatusLineCommandParseError::BareCarriageReturn),
                '\n' => {
                    self.offset += 1;
                    self.finish_line();
                    self.ended_with_newline = true;
                }
                '\u{1b}' => self.parse_escape()?,
                _ => {
                    self.offset += ch.len_utf8();
                    self.push_char(ch)?;
                    self.ended_with_newline = false;
                }
            }
        }

        if self.hyperlink.is_some() {
            return Err(StatusLineCommandParseError::UnterminatedHyperlink);
        }
        if !self.ended_with_newline || self.lines.is_empty() {
            self.finish_line();
        }
        if !self.saw_visible_text {
            return Err(StatusLineCommandParseError::EmptyOutput);
        }

        Ok(ParsedStatusLine { lines: self.lines })
    }

    fn push_char(&mut self, ch: char) -> Result<(), StatusLineCommandParseError> {
        if ch.is_control() || ('\u{80}'..='\u{9f}').contains(&ch) || ch == '\u{7f}' {
            return Err(StatusLineCommandParseError::ForbiddenControl(ch.into()));
        }
        if is_forbidden_invisible(ch) {
            return Err(StatusLineCommandParseError::ForbiddenInvisible(ch.into()));
        }

        // Measure with the crate's own width helpers, not raw `unicode-width`. Ratatui reserves a
        // cell for halfwidth sound marks (U+FF9E/U+FF9F) where `unicode-width` reports zero, so
        // raw measurement would undercount a label's rendered cells and let it exceed the bound.
        let width = char_width(ch);
        if self.hyperlink.is_some() {
            self.hyperlink_label.push(ch);
            if display_width(&self.hyperlink_label) > MAX_HYPERLINK_LABEL_CELLS {
                return Err(StatusLineCommandParseError::HyperlinkLabelTooLong);
            }
        }
        if width > 0 {
            self.saw_visible_text = true;
        }
        self.switch_run_if_needed();
        self.current_text.push(ch);
        Ok(())
    }

    fn parse_escape(&mut self) -> Result<(), StatusLineCommandParseError> {
        let bytes = self.input.as_bytes();
        match bytes.get(self.offset + 1) {
            Some(b'[') => self.parse_sgr_escape(),
            Some(b']') => self.parse_osc_escape(),
            _ => Err(StatusLineCommandParseError::UnsupportedEscape),
        }
    }

    fn parse_sgr_escape(&mut self) -> Result<(), StatusLineCommandParseError> {
        let sequence_start = self.offset + 2;
        let bytes = self.input.as_bytes();
        let Some(relative_end) = bytes[sequence_start..]
            .iter()
            .position(|byte| (0x40..=0x7e).contains(byte))
        else {
            return Err(StatusLineCommandParseError::MalformedSgr);
        };
        let sequence_end = sequence_start + relative_end;
        if bytes[sequence_end] != b'm' {
            return Err(StatusLineCommandParseError::UnsupportedEscape);
        }
        let params = &self.input[sequence_start..sequence_end];
        let parsed = if params.is_empty() {
            vec![0]
        } else {
            params
                .split(';')
                .map(|param| {
                    param
                        .parse::<u16>()
                        .map_err(|_| StatusLineCommandParseError::MalformedSgr)
                })
                .collect::<Result<Vec<_>, _>>()?
        };

        self.flush_run();
        apply_sgr(&mut self.style, &parsed)?;
        self.offset = sequence_end + 1;
        Ok(())
    }

    fn parse_osc_escape(&mut self) -> Result<(), StatusLineCommandParseError> {
        let payload_start = self.offset + 2;
        let bytes = self.input.as_bytes();
        let mut cursor = payload_start;
        let payload_end = loop {
            match bytes.get(cursor) {
                Some(0x07) => break cursor,
                Some(0x1b) if bytes.get(cursor + 1) == Some(&b'\\') => break cursor,
                Some(_) => cursor += 1,
                None => return Err(StatusLineCommandParseError::MalformedHyperlink),
            }
        };
        let terminator_len = if bytes[payload_end] == 0x07 { 1 } else { 2 };
        let payload = &self.input[payload_start..payload_end];
        let mut parts = payload.splitn(3, ';');
        if parts.next() != Some("8") {
            return Err(StatusLineCommandParseError::UnsupportedEscape);
        }
        let params = parts
            .next()
            .ok_or(StatusLineCommandParseError::MalformedHyperlink)?;
        let destination = parts
            .next()
            .ok_or(StatusLineCommandParseError::MalformedHyperlink)?;
        if params.len() > MAX_OSC_PARAMS_BYTES
            || params
                .chars()
                .any(|ch| ch.is_control() || ('\u{80}'..='\u{9f}').contains(&ch))
        {
            return Err(StatusLineCommandParseError::MalformedHyperlink);
        }

        self.flush_run();
        if destination.is_empty() {
            if self.hyperlink.take().is_none() {
                return Err(StatusLineCommandParseError::MalformedHyperlink);
            }
            self.hyperlink_label.clear();
        } else {
            if self.hyperlink.is_some() {
                return Err(StatusLineCommandParseError::MalformedHyperlink);
            }
            if destination.len() > MAX_HYPERLINK_URL_BYTES {
                return Err(StatusLineCommandParseError::HyperlinkUrlTooLong);
            }
            if destination.chars().any(|ch| {
                ch.is_control() || ('\u{80}'..='\u{9f}').contains(&ch) || is_forbidden_invisible(ch)
            }) {
                return Err(StatusLineCommandParseError::MalformedHyperlink);
            }
            let url = Url::parse(destination)
                .map_err(|_| StatusLineCommandParseError::MalformedHyperlink)?;
            if url.scheme() != "https" {
                return Err(StatusLineCommandParseError::UnsafeHyperlink);
            }
            self.hyperlink = Some(url.to_string());
            self.hyperlink_label.clear();
        }
        self.offset = payload_end + terminator_len;
        Ok(())
    }

    fn switch_run_if_needed(&mut self) {
        if self.current_text.is_empty() {
            self.current_text_style = self.style;
            self.current_text_hyperlink.clone_from(&self.hyperlink);
        } else if self.current_text_style != self.style
            || self.current_text_hyperlink != self.hyperlink
        {
            self.flush_run();
            self.current_text_style = self.style;
            self.current_text_hyperlink.clone_from(&self.hyperlink);
        }
    }

    fn flush_run(&mut self) {
        if self.current_text.is_empty() {
            return;
        }
        let text = std::mem::take(&mut self.current_text);
        self.current_line.push_span(
            Span::from(text).style(self.current_text_style),
            self.current_text_hyperlink.as_deref(),
        );
    }

    fn finish_line(&mut self) {
        self.flush_run();
        if self.lines.len() < MAX_STATUS_LINE_COMMAND_ROWS {
            self.lines.push(std::mem::take(&mut self.current_line));
        } else {
            self.current_line = HyperlinkLine::default();
        }
    }
}

fn apply_sgr(style: &mut Style, params: &[u16]) -> Result<(), StatusLineCommandParseError> {
    let mut index = 0;
    while index < params.len() {
        let param = params[index];
        match param {
            0 => *style = Style::default(),
            1 => *style = style.add_modifier(Modifier::BOLD),
            2 => *style = style.add_modifier(Modifier::DIM),
            3 => *style = style.add_modifier(Modifier::ITALIC),
            4 => *style = style.add_modifier(Modifier::UNDERLINED),
            7 => *style = style.add_modifier(Modifier::REVERSED),
            9 => *style = style.add_modifier(Modifier::CROSSED_OUT),
            22 => *style = style.remove_modifier(Modifier::BOLD | Modifier::DIM),
            23 => *style = style.remove_modifier(Modifier::ITALIC),
            24 => *style = style.remove_modifier(Modifier::UNDERLINED),
            27 => *style = style.remove_modifier(Modifier::REVERSED),
            29 => *style = style.remove_modifier(Modifier::CROSSED_OUT),
            30..=37 => style.fg = Some(ansi_color(param - 30, /*bright*/ false)),
            39 => style.fg = None,
            40..=47 => style.bg = Some(ansi_color(param - 40, /*bright*/ false)),
            49 => style.bg = None,
            90..=97 => style.fg = Some(ansi_color(param - 90, /*bright*/ true)),
            100..=107 => style.bg = Some(ansi_color(param - 100, /*bright*/ true)),
            38 | 48 => {
                let color = parse_extended_color(params, &mut index)?;
                if param == 38 {
                    style.fg = Some(color);
                } else {
                    style.bg = Some(color);
                }
            }
            _ => return Err(StatusLineCommandParseError::UnsupportedSgr(param)),
        }
        index += 1;
    }
    Ok(())
}

// External formatter output owns these colors, so preserving its explicit
// 256-color/RGB SGR semantics is preferable to theme-derived substitutions.
#[allow(clippy::disallowed_methods)]
fn parse_extended_color(
    params: &[u16],
    index: &mut usize,
) -> Result<Color, StatusLineCommandParseError> {
    match params.get(*index + 1) {
        Some(5) => {
            let value = *params
                .get(*index + 2)
                .ok_or(StatusLineCommandParseError::MalformedSgr)?;
            let value =
                u8::try_from(value).map_err(|_| StatusLineCommandParseError::MalformedSgr)?;
            *index += 2;
            Ok(Color::Indexed(value))
        }
        Some(2) => {
            let red = color_component(params.get(*index + 2))?;
            let green = color_component(params.get(*index + 3))?;
            let blue = color_component(params.get(*index + 4))?;
            *index += 4;
            Ok(Color::Rgb(red, green, blue))
        }
        _ => Err(StatusLineCommandParseError::MalformedSgr),
    }
}

fn color_component(value: Option<&u16>) -> Result<u8, StatusLineCommandParseError> {
    value
        .copied()
        .and_then(|value| u8::try_from(value).ok())
        .ok_or(StatusLineCommandParseError::MalformedSgr)
}

fn ansi_color(index: u16, bright: bool) -> Color {
    match (index, bright) {
        (0, false) => Color::Black,
        (1, false) => Color::Red,
        (2, false) => Color::Green,
        (3, false) => Color::Yellow,
        (4, false) => Color::Blue,
        (5, false) => Color::Magenta,
        (6, false) => Color::Cyan,
        (7, false) => Color::Gray,
        (0, true) => Color::DarkGray,
        (1, true) => Color::LightRed,
        (2, true) => Color::LightGreen,
        (3, true) => Color::LightYellow,
        (4, true) => Color::LightBlue,
        (5, true) => Color::LightMagenta,
        (6, true) => Color::LightCyan,
        (7, true) => Color::White,
        _ => unreachable!("ANSI color index is constrained to 0..=7"),
    }
}

fn is_forbidden_invisible(ch: char) -> bool {
    matches!(
        ch,
        '\u{00AD}'
            | '\u{034F}'
            | '\u{061C}'
            | '\u{180E}'
            | '\u{200B}'
            | '\u{200C}'
            | '\u{200E}'
            | '\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{206F}'
            | '\u{FEFF}'
            | '\u{FFF9}'..='\u{FFFB}'
            | '\u{1BCA0}'..='\u{1BCA3}'
    )
}

#[cfg(test)]
#[path = "parser_tests.rs"]
mod tests;
