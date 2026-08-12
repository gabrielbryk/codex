use super::CommandScopeIdentity;
use super::wrap_command_with_scope;

fn identity() -> CommandScopeIdentity {
    CommandScopeIdentity {
        thread_id: "thread-1".to_string(),
        turn_id: "turn-1".to_string(),
        call_id: "call-1".to_string(),
        profile: "/home/gabe/.codex".to_string(),
    }
}

#[test]
fn preserves_exact_argv_after_identity_prefix() {
    let command = vec![
        "/usr/bin/zsh".to_string(),
        "-c".to_string(),
        "printf '%s' 'hello world'".to_string(),
    ];
    let wrapped = wrap_command_with_scope(
        command.clone(),
        "/opt/guard/codex-command-scope-exec".to_string(),
        identity(),
    );

    assert_eq!(
        &wrapped[..11],
        &[
            "/opt/guard/codex-command-scope-exec",
            "--thread-id",
            "thread-1",
            "--turn-id",
            "turn-1",
            "--call-id",
            "call-1",
            "--profile",
            "/home/gabe/.codex",
            "--",
            "/usr/bin/zsh",
        ]
    );
    assert_eq!(&wrapped[10..], command);
}

#[test]
fn disabled_wrapper_leaves_command_unchanged() {
    let command = vec!["echo".to_string(), "hello".to_string()];
    assert_eq!(
        wrap_command_with_scope(command.clone(), String::new(), identity()),
        command
    );
}
