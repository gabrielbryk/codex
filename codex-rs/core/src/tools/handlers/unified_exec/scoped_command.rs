const CODEX_COMMAND_SCOPE_EXEC_ENV: &str = "CODEX_COMMAND_SCOPE_EXEC";

pub(super) struct CommandScopeIdentity {
    pub(super) thread_id: String,
    pub(super) turn_id: String,
    pub(super) call_id: String,
    pub(super) profile: String,
}

pub(super) fn wrap_local_command_for_scope(
    command: Vec<String>,
    identity: CommandScopeIdentity,
) -> Vec<String> {
    let Ok(wrapper) = std::env::var(CODEX_COMMAND_SCOPE_EXEC_ENV) else {
        return command;
    };
    wrap_command_with_scope(command, wrapper, identity)
}

fn wrap_command_with_scope(
    command: Vec<String>,
    wrapper: String,
    identity: CommandScopeIdentity,
) -> Vec<String> {
    if wrapper.is_empty() {
        return command;
    }

    let mut wrapped = vec![
        wrapper,
        "--thread-id".to_string(),
        identity.thread_id,
        "--turn-id".to_string(),
        identity.turn_id,
        "--call-id".to_string(),
        identity.call_id,
        "--profile".to_string(),
        identity.profile,
        "--".to_string(),
    ];
    wrapped.extend(command);
    wrapped
}

#[cfg(test)]
#[path = "scoped_command_tests.rs"]
mod tests;
