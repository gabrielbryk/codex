use super::CommandScopeIdentity;
use super::command_scope_prefix;
use pretty_assertions::assert_eq;

fn identity() -> CommandScopeIdentity {
    CommandScopeIdentity {
        thread_id: "thread-1".to_string(),
        turn_id: "turn-1".to_string(),
        call_id: "call-1".to_string(),
        profile: "/home/gabe/.codex".to_string(),
    }
}

#[test]
fn preserves_exact_identity_prefix() {
    let prefix = command_scope_prefix(
        "/opt/guard/codex-command-scope-exec".to_string(),
        identity(),
    );

    assert_eq!(
        prefix,
        vec![
            "/opt/guard/codex-command-scope-exec".to_string(),
            "--thread-id".to_string(),
            "thread-1".to_string(),
            "--turn-id".to_string(),
            "turn-1".to_string(),
            "--call-id".to_string(),
            "call-1".to_string(),
            "--profile".to_string(),
            "/home/gabe/.codex".to_string(),
            "--".to_string(),
        ]
    );
}

#[test]
fn empty_wrapper_has_no_prefix() {
    assert_eq!(
        command_scope_prefix(String::new(), identity()),
        Vec::<String>::new()
    );
}
