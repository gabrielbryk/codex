use pretty_assertions::assert_eq;
use ratatui::style::Color;
use ratatui::style::Modifier;
use serde_json::json;
use uuid::Uuid;

use super::*;
use crate::status_line_command::parser::parse_status_line_command_output;

const EXPORTER_V1_FULL_FIXTURE: &str = include_str!("fixtures/exporter_v1_full.json");

fn minimal_input() -> StatusLineCommandInput {
    StatusLineCommandInput {
        cwd: "/remote/workspace".to_string(),
        session_id: Uuid::parse_str("11111111-2222-4333-8444-555555555555")
            .expect("valid UUID")
            .into(),
        session_name: None,
        model: StatusLineCommandModel {
            id: "gpt-5.6-terra".to_string(),
            display_name: "Terra".to_string(),
        },
        workspace: StatusLineCommandWorkspace {
            current_dir: "/remote/workspace".to_string(),
            project_dir: None,
            added_dirs: Vec::new(),
            repo: None,
        },
        version: "0.146.0".to_string(),
        fast_mode: true,
        exceeds_200k_tokens: false,
        effort: None,
        thinking: StatusLineCommandThinking { enabled: false },
        context_window: StatusLineCommandContextWindow {
            total_input_tokens: 10,
            total_output_tokens: 20,
            context_window_size: 272_000,
            used_percentage: None,
            remaining_percentage: None,
            current_usage: None,
        },
        rate_limits: None,
        extra_usage: None,
        pr: None,
        codex: StatusLineCommandCodex {
            schema_version: STATUS_LINE_COMMAND_SCHEMA_VERSION,
            local_process_cwd: "/local/codex".to_string(),
            status: "working".to_string(),
            permissions: "workspace-write".to_string(),
            approval_mode: "on-request".to_string(),
            service_tier: "fast".to_string(),
            workspace_headline: None,
            task_progress: None,
            git_branch: None,
            branch_changes: None,
        },
    }
}

#[test]
fn minimal_contract_uses_null_only_for_declared_nullable_fields() {
    let input = minimal_input();
    let actual = serde_json::to_value(input).expect("serialize input");

    assert_eq!(
        actual,
        json!({
            "cwd": "/remote/workspace",
            "session_id": "11111111-2222-4333-8444-555555555555",
            "model": { "id": "gpt-5.6-terra", "display_name": "Terra" },
            "workspace": {
                "current_dir": "/remote/workspace",
                "project_dir": null,
                "added_dirs": []
            },
            "version": "0.146.0",
            "fast_mode": true,
            "exceeds_200k_tokens": false,
            "thinking": { "enabled": false },
            "context_window": {
                "total_input_tokens": 10,
                "total_output_tokens": 20,
                "context_window_size": 272000,
                "used_percentage": null,
                "remaining_percentage": null,
                "current_usage": null
            },
            "codex": {
                "schema_version": 1,
                "local_process_cwd": "/local/codex",
                "status": "working",
                "permissions": "workspace-write",
                "approval_mode": "on-request",
                "service_tier": "fast",
                "workspace_headline": null,
                "task_progress": null,
                "git_branch": null,
                "branch_changes": null
            }
        })
    );
}

#[test]
fn optional_usage_objects_and_raw_reset_epochs_are_preserved() {
    let mut input = minimal_input();
    input.effort = Some(StatusLineCommandEffort {
        level: "high".to_string(),
    });
    input.rate_limits = Some(StatusLineCommandRateLimits {
        five_hour: Some(StatusLineCommandRateLimitWindow {
            used_percentage: 42.5,
            resets_at: 1_800_000_123,
        }),
        seven_day: None,
    });
    input.extra_usage = Some(StatusLineCommandExtraUsage {
        enabled: true,
        used: 12.25,
        limit: 100.0,
    });

    let actual = serde_json::to_value(input).expect("serialize input");
    assert_eq!(actual["effort"], json!({ "level": "high" }));
    assert_eq!(
        actual["rate_limits"],
        json!({
            "five_hour": {
                "used_percentage": 42.5,
                "resets_at": 1_800_000_123
            }
        })
    );
    assert_eq!(
        actual["extra_usage"],
        json!({ "enabled": true, "used": 12.25, "limit": 100.0 })
    );
}

#[test]
fn unavailable_pr_review_state_is_explicitly_null() {
    let mut input = minimal_input();
    input.pr = Some(StatusLineCommandPullRequest {
        number: 42,
        url: "https://github.com/openai/codex/pull/42".to_string(),
        review_state: None,
    });

    let actual = serde_json::to_value(input).expect("serialize input");
    assert_eq!(actual["pr"]["review_state"], serde_json::Value::Null);
}

#[test]
fn json_line_ends_with_one_newline() {
    let bytes = minimal_input().to_json_line().expect("serialize input");
    assert_eq!(bytes.last(), Some(&b'\n'));
    assert_eq!(bytes.iter().filter(|byte| **byte == b'\n').count(), 1);
}

#[test]
fn json_line_rejects_payloads_over_the_input_limit() {
    let mut input = minimal_input();
    input.session_name = Some("x".repeat(MAX_STATUS_LINE_COMMAND_INPUT_BYTES * 4));

    let err = input.to_json_line().expect_err("oversized input");
    assert_eq!(
        err.to_string(),
        "status-line command input exceeds 65536 bytes"
    );
}

#[test]
fn json_line_writer_does_not_retain_bytes_beyond_its_limit() {
    let max_bytes = 128;
    let mut writer = BoundedJsonWriter::new(max_bytes);
    let oversized_value = "x".repeat(MAX_STATUS_LINE_COMMAND_INPUT_BYTES * 4);

    serde_json::to_writer(&mut writer, &oversized_value).expect_err("oversized input");

    assert!(writer.limit_exceeded);
    assert!(writer.bytes.len() <= max_bytes);
}

#[test]
fn fully_populated_exporter_fixture_is_stable() {
    let mut input = minimal_input();
    input.session_name = Some("Status work".to_string());
    input.workspace.project_dir = Some("/remote".to_string());
    input.workspace.added_dirs = vec!["/remote/shared".to_string()];
    input.workspace.repo = Some(StatusLineCommandRepository {
        host: "github.com".to_string(),
        owner: "openai".to_string(),
        name: "codex".to_string(),
    });
    input.effort = Some(StatusLineCommandEffort {
        level: "high".to_string(),
    });
    input.thinking.enabled = true;
    input.context_window.used_percentage = Some(25.0);
    input.context_window.remaining_percentage = Some(75.0);
    input.context_window.current_usage = Some(StatusLineCommandCurrentUsage {
        input_tokens: 100,
        output_tokens: 20,
        cache_creation_input_tokens: 10,
        cache_read_input_tokens: 5,
    });
    input.rate_limits = Some(StatusLineCommandRateLimits {
        five_hour: Some(StatusLineCommandRateLimitWindow {
            used_percentage: 10.0,
            resets_at: 1_800_000_000,
        }),
        seven_day: Some(StatusLineCommandRateLimitWindow {
            used_percentage: 20.0,
            resets_at: 1_800_500_000,
        }),
    });
    input.extra_usage = Some(StatusLineCommandExtraUsage {
        enabled: true,
        used: 2.5,
        limit: 50.0,
    });
    input.pr = Some(StatusLineCommandPullRequest {
        number: 42,
        url: "https://github.com/openai/codex/pull/42".to_string(),
        review_state: Some("approved".to_string()),
    });
    input.codex.workspace_headline = Some("Implementing status line".to_string());
    input.codex.task_progress = Some(StatusLineCommandTaskProgress {
        completed: 2,
        total: 3,
    });
    input.codex.git_branch = Some("feature/status-line".to_string());
    input.codex.branch_changes = Some(StatusLineCommandBranchChanges {
        additions: 12,
        deletions: 3,
    });

    let expected = serde_json::from_str::<serde_json::Value>(EXPORTER_V1_FULL_FIXTURE)
        .expect("valid exporter fixture");
    assert_eq!(
        serde_json::to_value(input).expect("serialize full fixture"),
        expected
    );
}

/// Deterministic stand-in for the pinned exporter's mutable `ccstatusline@latest`
/// subprocess. It deliberately consumes only fields covered by the v1 fixture.
fn render_exporter_fixture(input: &serde_json::Value) -> String {
    let model = input["model"]["display_name"]
        .as_str()
        .expect("model display name");
    let effort = input["effort"]["level"].as_str().expect("effort level");
    let used = input["context_window"]["used_percentage"]
        .as_f64()
        .expect("used percentage");
    let branch = input["codex"]["git_branch"].as_str().expect("git branch");
    let pr_number = input["pr"]["number"].as_u64().expect("PR number");
    let pr_url = input["pr"]["url"].as_str().expect("PR URL");

    format!(
        "\x1b[1;36m{model}\x1b[0m ({effort}) · {used:.0}%\n{branch} · \x1b]8;;{pr_url}\x1b\\PR #{pr_number}\x1b]8;;\x1b\\"
    )
}

#[test]
#[allow(clippy::disallowed_methods)]
fn exporter_fixture_stub_renders_accepted_status_line() {
    let input = serde_json::from_str::<serde_json::Value>(EXPORTER_V1_FULL_FIXTURE)
        .expect("valid exporter fixture");
    let rendered = render_exporter_fixture(&input);
    let parsed = parse_status_line_command_output(rendered.as_bytes()).expect("accepted output");

    assert_eq!(
        parsed
            .lines
            .iter()
            .map(|line| line.line.to_string())
            .collect::<Vec<_>>(),
        vec!["Terra (high) · 25%", "feature/status-line · PR #42"]
    );
    assert_eq!(parsed.lines[0].line.spans[0].style.fg, Some(Color::Cyan));
    assert!(
        parsed.lines[0].line.spans[0]
            .style
            .add_modifier
            .contains(Modifier::BOLD)
    );
    assert_eq!(parsed.lines[1].hyperlinks.len(), 1);
    assert_eq!(
        parsed.lines[1].hyperlinks[0].destination,
        "https://github.com/openai/codex/pull/42"
    );
}
