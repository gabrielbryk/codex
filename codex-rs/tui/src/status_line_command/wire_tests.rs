use pretty_assertions::assert_eq;

use super::StatusLineCommandInput;

#[test]
fn input_is_one_bounded_json_line() {
    let input = StatusLineCommandInput {
        cwd: "/repo".to_string(),
        model: "gpt-5.6-sol".to_string(),
        status: "working".to_string(),
        input_tokens: 10,
        output_tokens: 2,
        context_remaining_percent: Some(80),
    };
    let bytes = input.to_json_line().expect("bounded input");
    assert_eq!(bytes.last(), Some(&b'\n'));
    assert_eq!(bytes.iter().filter(|byte| **byte == b'\n').count(), 1);
    assert!(serde_json::from_slice::<serde_json::Value>(&bytes).is_ok());
    let oversized = StatusLineCommandInput {
        cwd: "x".repeat(9_000),
        model: String::new(),
        status: String::new(),
        input_tokens: 0,
        output_tokens: 0,
        context_remaining_percent: None,
    };
    assert!(oversized.to_json_line().is_err());
}
