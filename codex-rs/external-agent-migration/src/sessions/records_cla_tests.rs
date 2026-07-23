use super::*;
use pretty_assertions::assert_eq;
use tempfile::TempDir;

#[test]
fn reads_session_import_in_one_pass() {
    let root = TempDir::new().expect("tempdir");
    let path = root.path().join("session.jsonl");
    let contents = [
        serde_json::json!({
            "type": "user",
            "cwd": root.path(),
            "timestamp": "2026-06-03T12:00:00Z",
            "message": { "content": "<user_query>\nfirst request\n</user_query>" },
        })
        .to_string(),
        "not json".to_string(),
        serde_json::json!({
            "type": "ai-title",
            "aiTitle": "generated title",
        })
        .to_string(),
        serde_json::json!({
            "type": "custom-title",
            "customTitle": "custom title",
        })
        .to_string(),
    ]
    .join("\n");
    std::fs::write(&path, &contents).expect("session");

    let parsed = read_session_import(&path).expect("parse session");

    assert_eq!(parsed.cwd.as_deref(), Some(root.path()));
    assert_eq!(parsed.custom_title.as_deref(), Some("custom title"));
    assert_eq!(parsed.ai_title.as_deref(), Some("generated title"));
    assert_eq!(parsed.messages.len(), 1);
    assert_eq!(parsed.messages[0].text, "first request");
    assert_eq!(
        parsed.content_sha256,
        format!("{:x}", Sha256::digest(contents))
    );
}

fn message_record(role: &str, text: &str, cwd: &Path) -> String {
    serde_json::json!({
        "type": role,
        "cwd": cwd,
        "timestamp": "2026-06-03T12:00:00Z",
        "message": { "content": text },
    })
    .to_string()
}

fn compact_boundary_record() -> String {
    serde_json::json!({
        "type": "system",
        "subtype": "compact_boundary",
    })
    .to_string()
}

#[test]
fn reads_only_messages_after_claude_compact_boundary() {
    let root = TempDir::new().expect("tempdir");
    let path = root.path().join("session.jsonl");
    let contents = [
        message_record("user", "old request", root.path()),
        message_record("assistant", "old answer", root.path()),
        compact_boundary_record(),
        message_record("user", "compacted summary", root.path()),
        message_record("assistant", "new answer", root.path()),
    ]
    .join("\n");
    std::fs::write(&path, &contents).expect("session");

    let parsed = read_session_import(&path).expect("parse session");
    let texts = parsed
        .messages
        .iter()
        .map(|message| message.text.as_str())
        .collect::<Vec<_>>();

    assert_eq!(texts, vec!["compacted summary", "new answer"]);
    assert_eq!(
        parsed.content_sha256,
        format!("{:x}", Sha256::digest(contents))
    );
}

#[test]
fn reads_only_messages_after_last_claude_compact_boundary() {
    let root = TempDir::new().expect("tempdir");
    let path = root.path().join("session.jsonl");
    let contents = [
        message_record("user", "first request", root.path()),
        compact_boundary_record(),
        message_record("user", "first summary", root.path()),
        message_record("assistant", "middle answer", root.path()),
        compact_boundary_record(),
        message_record("user", "second summary", root.path()),
    ]
    .join("\n");
    std::fs::write(&path, &contents).expect("session");

    let parsed = read_session_import(&path).expect("parse session");
    let texts = parsed
        .messages
        .iter()
        .map(|message| message.text.as_str())
        .collect::<Vec<_>>();

    assert_eq!(texts, vec!["second summary"]);
}

#[test]
fn preserves_titles_across_claude_compact_boundary() {
    let root = TempDir::new().expect("tempdir");
    let path = root.path().join("session.jsonl");
    let contents = [
        serde_json::json!({
            "type": "custom-title",
            "customTitle": "kept title",
        })
        .to_string(),
        message_record("user", "old request", root.path()),
        compact_boundary_record(),
        message_record("user", "summary", root.path()),
    ]
    .join("\n");
    std::fs::write(&path, &contents).expect("session");

    let parsed = read_session_import(&path).expect("parse session");

    assert_eq!(parsed.custom_title.as_deref(), Some("kept title"));
    let texts = parsed
        .messages
        .iter()
        .map(|message| message.text.as_str())
        .collect::<Vec<_>>();
    assert_eq!(texts, vec!["summary"]);
}
