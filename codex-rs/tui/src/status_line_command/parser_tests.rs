use pretty_assertions::assert_eq;
use ratatui::style::Color;
use ratatui::style::Modifier;

use super::*;

fn visible_text(parsed: &ParsedStatusLine) -> Vec<String> {
    parsed
        .lines
        .iter()
        .map(|line| line.line.to_string())
        .collect()
}

#[test]
fn parses_crlf_multiple_rows_and_discards_rows_after_limit() {
    let parsed =
        parse_status_line_command_output(b"one\r\ntwo\nthree\nfour").expect("valid status line");
    assert_eq!(visible_text(&parsed), vec!["one", "two", "three"]);
}

#[test]
#[allow(clippy::disallowed_methods)]
fn parses_sgr_styles_without_retaining_escape_bytes() {
    let parsed = parse_status_line_command_output(
        b"plain \x1b[1;31mbold red\x1b[22;39m \x1b[38;2;1;2;3mrgb\x1b[0m",
    )
    .expect("valid status line");
    assert_eq!(visible_text(&parsed), vec!["plain bold red rgb"]);
    let spans = &parsed.lines[0].line.spans;
    assert_eq!(spans[1].content.as_ref(), "bold red");
    assert!(spans[1].style.add_modifier.contains(Modifier::BOLD));
    assert_eq!(spans[1].style.fg, Some(Color::Red));
    assert_eq!(spans[3].style.fg, Some(Color::Rgb(1, 2, 3)));
}

#[test]
fn parses_https_osc8_as_semantic_hyperlink() {
    let parsed = parse_status_line_command_output(
        b"PR \x1b]8;;https://github.com/openai/codex/pull/1\x1b\\#1\x1b]8;;\x1b\\",
    )
    .expect("valid status line");
    assert_eq!(visible_text(&parsed), vec!["PR #1"]);
    assert_eq!(parsed.lines[0].hyperlinks.len(), 1);
    assert_eq!(parsed.lines[0].hyperlinks[0].columns, 3..5);
    assert_eq!(
        parsed.lines[0].hyperlinks[0].destination,
        "https://github.com/openai/codex/pull/1"
    );
}

#[test]
fn permits_emoji_joiners_and_variation_selectors() {
    let parsed =
        parse_status_line_command_output("👩\u{200d}💻 ❤️".as_bytes()).expect("valid status line");
    assert_eq!(visible_text(&parsed), vec!["👩\u{200d}💻 ❤️"]);
}

#[test]
fn rejects_terminal_controls_and_bidi_controls() {
    assert_eq!(
        parse_status_line_command_output(b"bad\ttext"),
        Err(StatusLineCommandParseError::ForbiddenControl(9))
    );
    assert_eq!(
        parse_status_line_command_output("left\u{202e}right".as_bytes()),
        Err(StatusLineCommandParseError::ForbiddenInvisible(0x202e))
    );
    assert_eq!(
        parse_status_line_command_output(b"\x1b[2Jclear"),
        Err(StatusLineCommandParseError::UnsupportedEscape)
    );
}

#[test]
fn rejects_unsafe_or_unterminated_hyperlinks() {
    assert_eq!(
        parse_status_line_command_output(b"\x1b]8;;http://example.com\x07label\x1b]8;;\x07"),
        Err(StatusLineCommandParseError::UnsafeHyperlink)
    );
    assert_eq!(
        parse_status_line_command_output(b"\x1b]8;;https://example.com\x07label"),
        Err(StatusLineCommandParseError::UnterminatedHyperlink)
    );
    assert_eq!(
        parse_status_line_command_output(
            "\x1b]8;;https://example.com/\u{202e}spoof\x07label\x1b]8;;\x07".as_bytes()
        ),
        Err(StatusLineCommandParseError::MalformedHyperlink)
    );
}

#[test]
fn rejects_bare_cr_invalid_utf8_empty_and_oversized_output() {
    assert_eq!(
        parse_status_line_command_output(b"left\rright"),
        Err(StatusLineCommandParseError::BareCarriageReturn)
    );
    assert_eq!(
        parse_status_line_command_output(&[0xff]),
        Err(StatusLineCommandParseError::InvalidUtf8)
    );
    assert_eq!(
        parse_status_line_command_output(b"\n"),
        Err(StatusLineCommandParseError::EmptyOutput)
    );
    assert_eq!(
        parse_status_line_command_output(&vec![b'a'; MAX_STATUS_LINE_COMMAND_BYTES + 1]),
        Err(StatusLineCommandParseError::OutputTooLarge)
    );
}

#[test]
fn rejects_hyperlink_labels_over_cell_limit() {
    let output = format!(
        "\x1b]8;;https://example.com\x07{}\x1b]8;;\x07",
        "x".repeat(MAX_HYPERLINK_LABEL_CELLS + 1)
    );
    assert_eq!(
        parse_status_line_command_output(output.as_bytes()),
        Err(StatusLineCommandParseError::HyperlinkLabelTooLong)
    );
}

#[test]
fn hyperlink_label_limit_uses_grapheme_aware_terminal_width() {
    let label = "👩\u{200d}💻".repeat(MAX_HYPERLINK_LABEL_CELLS / 2);
    let output = format!("\x1b]8;;https://example.com\x07{label}\x1b]8;;\x07");
    let parsed = parse_status_line_command_output(output.as_bytes()).expect("512-cell label");
    assert_eq!(parsed.lines[0].line.to_string(), label);
}
