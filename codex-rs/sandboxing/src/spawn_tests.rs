use super::scope_wrapped_command;

fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|part| (*part).to_string()).collect()
}

#[test]
fn prefixes_wrapper_and_separator() {
    assert_eq!(
        scope_wrapped_command(&argv(&["/bin/zsh", "-lc", "make"]), Some("/w/scope"), false),
        argv(&["/w/scope", "--", "/bin/zsh", "-lc", "make"]),
    );
}

#[test]
fn leaves_command_untouched_without_wrapper() {
    let command = argv(&["/bin/zsh", "-lc", "make"]);
    assert_eq!(scope_wrapped_command(&command, None, false), command);
    assert_eq!(scope_wrapped_command(&command, Some(""), false), command);
}

#[test]
fn leaves_arg0_override_untouched() {
    let command = argv(&["codex-linux-sandbox", "--flag"]);
    assert_eq!(
        scope_wrapped_command(&command, Some("/w/scope"), true),
        command
    );
}
