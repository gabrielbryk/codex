use std::collections::HashMap;

use pretty_assertions::assert_eq;

use super::*;
use crate::status_line_command::runner::Lifecycle;
use crate::status_line_command::wire::StatusLineCommandInput;

#[test]
fn formatter_environment_excludes_credentials_and_proxy_configuration() {
    let environment = formatter_environment([
        ("PATH".to_string(), "/bin".to_string()),
        ("HOME".to_string(), "/home/test".to_string()),
        ("OPENAI_API_KEY".to_string(), "secret".to_string()),
        ("AWS_SECRET_ACCESS_KEY".to_string(), "secret".to_string()),
        ("HTTPS_PROXY".to_string(), "secret".to_string()),
        ("CODEX_HOME".to_string(), "/private/codex".to_string()),
    ]);
    assert_eq!(
        environment,
        HashMap::from([("HOME".into(), "/home/test".into())])
    );
}

fn invocation() -> Invocation {
    Lifecycle::new(1)
        .begin(
            StatusLineCommandInput {
                cwd: "/repo".to_string(),
                model: "model".to_string(),
                status: "working".to_string(),
                input_tokens: 10,
                output_tokens: 2,
                context_remaining_percent: Some(80),
            },
            std::time::Instant::now(),
        )
        .expect("generation")
}

fn shell(script: &str, args: &[String], timeout_ms: u64) -> TuiStatusLineCommand {
    let mut command = vec!["/bin/sh".to_string(), "-c".to_string(), script.to_string()];
    command.extend_from_slice(args);
    TuiStatusLineCommand {
        command,
        timeout_ms,
    }
}

#[cfg(unix)]
#[tokio::test]
async fn command_receives_json_and_returns_one_row() {
    let config = shell(
        "IFS= read -r line; case \"$line\" in *working*) printf ready;; *) exit 9;; esac",
        &[],
        1_000,
    );
    let completion = execute(config, &std::env::current_dir().expect("cwd"), invocation()).await;
    assert_eq!(completion.result, Ok("ready".to_string()));
}

#[cfg(unix)]
#[tokio::test]
async fn timeout_and_output_overflow_fail_closed() {
    let cwd = std::env::current_dir().expect("cwd");
    let timeout = shell("/bin/sleep 1", &[], 250);
    assert_eq!(
        execute(timeout, &cwd, invocation()).await.result,
        Err("formatter timed out".to_string())
    );
    let overflow = shell(
        "/usr/bin/head -c 4097 /dev/zero | /usr/bin/tr '\\0' x",
        &[],
        1_000,
    );
    assert_eq!(
        execute(overflow, &cwd, invocation()).await.result,
        Err("formatter stream exceeded 4096 bytes".to_string())
    );
}

#[cfg(unix)]
#[tokio::test]
async fn timeout_terminates_descendants_in_the_formatter_process_group() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let marker = tmp.path().join("descendant-finished");
    let config = shell(
        "(/bin/sleep 1; : > \"$1\") & wait",
        &[
            "status-line-command".to_string(),
            marker.to_string_lossy().into_owned(),
        ],
        250,
    );

    assert_eq!(
        execute(config, tmp.path(), invocation()).await.result,
        Err("formatter timed out".to_string())
    );
    tokio::time::sleep(Duration::from_millis(1_100)).await;
    assert!(!marker.exists(), "timed out descendant wrote its marker");
}
