# External status-line command proposal

## Status

Revised after source-level design review. This is an internal implementation
guide for the local Codex fork, deliberately outside the user-facing `docs/`
tree. The remaining decisions are recorded below; implementation must not start
until they are settled.

### Decision 2026-08-02: best-effort Unix containment

Containment stays cross-platform and is explicitly **best-effort on Unix**:
cleanup terminates the formatter's process group only, and a formatter that
calls `setsid()` escapes it. Rationale: formatters are short-lived, trusted user
configuration, and keeping macOS (and Unix generally) available outweighs
closing the `setsid()` escape. Windows keeps its Job Object whole-tree
guarantee. Daemonizing / breakaway formatters are documented as unsupported;
stronger sandboxing (bubblewrap, cgroup v2) is Stage 3 future hardening, not
v1.

## Problem and compatibility stance

Codex currently renders `tui.status_line` from fixed built-in items. We want a
locally executed, command-backed status line that receives session data on stdin
and owns the complete rendered area.

This provides **Claude-compatible execution and rendering semantics**, not a
promise that Codex emits every Claude-only datum: Codex uses a cross-platform
argv array rather than a shell string. The input is a Codex-owned, versioned
schema with compatibility aliases for the fields consumed by our exporter.
"Claude-compatible" is therefore not an acceptance criterion for arbitrary
third-party Claude scripts. Stage 1 instead uses a deterministic fixture derived
from exporter revision `e27861d2d5173914cc0d9bad2470d1de6948b43c`, with its
mutable `bunx ccstatusline@latest` renderer replaced by a checked-in stub.
Expanding the envelope later requires a new versioned mapping review.

Codex-only additions live under `codex`. The fixture contract is stable; fields
outside it are documented as best-effort compatibility fields, never invented.
The fixture's required fields are exactly `session_id`, `model.id`,
`model.display_name`, `context_window`, and `version`. `effort`, rate limits,
and `extra_usage` are optional and follow the omission/nullability rules in the
table below. A separate manual smoke test may run the live exporter, but it is
not a deterministic acceptance gate while it invokes `@latest`.

The compact model display remains a separate, smaller built-in item proposal.

## Goals

- Run the formatter on the **local TUI host**, regardless of remote app-server
  or exec-server placement.
- Use versioned JSON on stdin and render the command output as the complete
  external-status-line region.
- Match Claude's execution/output semantics, plus the pinned exporter fixture's
  required input fields, including ANSI, hyperlinks, and multiple lines.
- Keep the TUI responsive with bounded I/O, debounce, cancellation, and
  last-known-good rendering.
- Support Linux, macOS, and Windows without shell parsing.

## Non-goals for v1

- Shell parsing, interpolation, or evaluation. A user who needs shell syntax
  supplies a script as argv[0].
- `/statusline` generating or editing arbitrary command arguments.
- Sending prompts, message contents, environment values, credentials, or raw
  API payloads to the formatter.
- Running the formatter through `WorkspaceCommandExecutor` or app-server RPC.

## Configuration and validation

```toml
[tui.status_line_command]
command = ["/absolute/path/to/statusline"]
timeout_ms = 5000
```

`command` is a non-empty argv array; its first element is the executable and
the rest are literal arguments. It inherits the local TUI environment. The
runner's `current_dir` is the local TUI launch directory, never a remote
workspace path.

The runner debounce is a fixed 300 ms and deliberately separate from execution
timeout. `timeout_ms` defaults to 5000 and is **rejected** outside
250..=30000 ms; it is never silently clamped. A new refresh cancels an
in-flight command regardless of its timeout.

`tui.status_line_command` and `tui.status_line` are mutually exclusive and
must be rejected during configuration resolution. The `/statusline` opening
path and its persistence handler must also refuse to write `tui.status_line`
while command mode is configured. Removing command mode restores built-in
selection behavior.

## Formatter input contract: v1

Codex writes one UTF-8 JSON object plus newline, then closes stdin. Values not
available in Codex use `null` only where Claude permits null; optional Claude
objects are absent when unavailable. The serialized wire type, not display
types, owns this contract.

```json
{
  "cwd": "/session-or-remote/workspace",
  "session_id": "stable-codex-session-uuid",
  "model": { "id": "gpt-5.6-terra", "display_name": "Terra" },
  "workspace": {
    "current_dir": "/session-or-remote/workspace",
    "project_dir": "/session/project-root-or-null",
    "added_dirs": [],
    "repo": { "host": "github.com", "owner": "openai", "name": "codex" }
  },
  "version": "0.146.0",
  "fast_mode": true,
  "exceeds_200k_tokens": false,
  "effort": { "level": "medium" },
  "thinking": { "enabled": true },
  "context_window": {
    "total_input_tokens": 0,
    "total_output_tokens": 0,
    "context_window_size": 272000,
    "used_percentage": 0.0,
    "remaining_percentage": 100.0,
    "current_usage": null
  },
  "rate_limits": {
    "five_hour": { "used_percentage": 0.0, "resets_at": 0 },
    "seven_day": { "used_percentage": 0.0, "resets_at": 0 }
  },
  "extra_usage": { "enabled": false, "used": 0.0, "limit": 0.0 },
  "pr": { "number": 123, "url": "https://...", "review_state": null },
  "codex": {
    "local_process_cwd": "/local/tui/launch-dir",
    "status": "working",
    "permissions": "workspace-write",
    "approval_mode": "on-request",
    "service_tier": "fast",
    "workspace_headline": null,
    "task_progress": null
  }
}
```

`session_id` is always a non-empty `StatusLineCommandSessionId`, generated once
when the command-mode widget is constructed and retained for that widget's
entire lifetime. It is never replaced by a server thread ID. The initial mapping
table is:

| Input field | Codex source / rule |
| --- | --- |
| `model`, `thinking.enabled` | active model and reasoning settings; `thinking.enabled` is true exactly when reasoning is enabled for the active model |
| `effort` | omit the entire object when the active model does not support reasoning effort; otherwise emit `level` |
| `fast_mode` | selected fast/priority service tier |
| `session_name`, `workspace.repo`, `rate_limits` windows, `pr` | omit the object/field when unavailable; do not serialize `null` where Claude uses absence |
| `exceeds_200k_tokens` | computed from raw context totals |
| `context_window`, `rate_limits`, `pr` | raw protocol/status data, never display strings; `context_window.used_percentage` and `remaining_percentage` may be `null` before usable usage data exists |
| `extra_usage` | omit the entire object when Codex has no corresponding extra-usage data; never synthesize zero/false values |
| `codex.*` | Codex-specific local/session state |

`cwd` and `workspace.current_dir` describe the session workspace, which may be
remote. `codex.local_process_cwd` is the local process directory. These values
must never be conflated. Codex must not run local Git to fill remote workspace
fields: branch, PR, summary, and workspace headline continue to use the
existing remote `WorkspaceCommandExecutor` paths.

Wire types must encode the table above now; no nullability/omission decision is
deferred. `context_window.current_usage`, `used_percentage`, and
`remaining_percentage` may be `null`; `session_name`, `effort`, `rate_limits`,
its windows, `extra_usage`, `pr`, and `workspace.repo` are omitted when
unavailable. Preserve raw epoch seconds for `resets_at`; do not derive them
from `RateLimitWindowDisplay`, which only keeps formatted reset text.

## Rendering and process safety

The command is trusted user configuration, but stdout is still untrusted
terminal input. The runner must:

- pipe stdin/stdout/stderr, serialize then close stdin, and cap each stream at
  8 KiB with concurrent bounded reads; overflow kills and drains the process;
- timeout, cancel, and clean up the formatter through a new public,
  encapsulated `codex-utils-pty` spawn-and-terminate API, whose containment
  strength is deliberately platform-specific. **Unix: process-group termination
  only** — the child leads its own group and the group is signalled; this is
  best-effort, and a formatter that calls `setsid()` (or otherwise leaves the
  group) escapes cleanup. Daemonizing/breakaway formatters are unsupported.
  **Windows: whole process tree** — the child is created suspended, assigned to
  a breakaway-forbidden Job Object, then resumed, eliminating the
  spawn/assignment race. `kill_on_drop(true)` alone is insufficient on either
  platform;
- accept valid UTF-8 only. At most **three rows** are rendered, each truncated
  to the available footer width in terminal cells; excess rows are discarded;
- parse only an explicit ANSI SGR subset and validated OSC 8 hyperlinks into
  ratatui spans. OSC 8 permits only `https` URLs up to 2048 bytes and labels up
  to 512 cells. Reject all other C0/C1 controls, DEL, malformed escape
  sequences, and bidi controls; retain normal emoji shaping controls such as
  ZWJ and variation selectors;
- normalize CRLF safely, preserve intentional line boundaries up to the line
  cap, and never print stderr in the status line or transcript.

Reuse the terminal-title character policy as the baseline sanitizer, but keep
ANSI parsing separate from text normalization. The parser must be a dedicated,
reviewed module with snapshots; it must not pass raw escape codes to ratatui.

## Lifecycle and ownership

Refreshes are event-driven and debounced for 300 ms. Each command-mode widget
has a non-reused owner token plus a monotonically increasing generation. The
same owner token is attached to every command-mode branch, summary, PR, and
headline request/result; a replacement widget rejects all older-owner data
before it can create a formatter snapshot.

1. A refresh builds an immutable wire snapshot and marks the newest generation
   dirty.
2. After debounce, run at most one formatter. New input during a run terminates
   the in-flight formatter (process group on Unix, job on Windows) and retains
   only the newest snapshot.
3. `AppEvent::StatusLineCommandFinished` includes both owner token and
   generation. The dispatcher accepts it only for the live widget and newest
   generation.
4. Thread/session replacement and widget drop abort the debounce task and
   terminate the running formatter before installing the replacement. A late
   event is harmless.
5. Success replaces the entire command region. Spawn, stdin, UTF-8, parser,
   nonzero-exit, timeout, and output-limit failures retain the last good result
   and log one debug diagnostic per session/config revision.

Command mode independently declares its data dependencies when constructing
`StatusSurfaceSelections`; otherwise current built-in-item gating clears git
and headline caches. It must request the same remote branch, summary, PR, and
headline data needed by its v1 snapshot.

## Implementation shape

Add a focused `status_line_command/` module tree, not one large file: `wire.rs`
owns the serialized schema, `parser.rs` owns bounded ANSI/OSC8 parsing,
`runner.rs` owns scheduling and bounded streams, and `platform.rs` owns
formatter spawn and termination through the new `codex-utils-pty` API. Do not expose raw
Windows process handles or `JobObject::assign_process`; do not grow
`chatwidget.rs` with standalone helpers.

Likely files:

- `codex-rs/config/src/types.rs` and `codex-rs/core/src/config/mod.rs`:
  resolved command config and mutual-exclusion validation.
- `codex-rs/utils/pty`: a tested public contained spawn-and-terminate API;
  on Unix it owns process-group setup and group termination (best-effort), and
  on Windows it owns suspended spawn, job assignment, and resume atomically.
- `codex-rs/core/config.schema.json`: regenerate with `just write-config-schema`.
- `codex-rs/tui/src/status_line_command/{mod,wire,parser,runner,platform}.rs`
  and sibling tests.
- `codex-rs/tui/src/status/rate_limits.rs`: retain/expose raw reset epochs for
  the wire snapshot without changing display formatting.
- `codex-rs/tui/src/chatwidget/status_surfaces.rs`: command dependencies,
  snapshot construction, scheduling, and result application.
- `codex-rs/tui/src/chatwidget.rs`: only minimal state ownership.
- `codex-rs/tui/src/app_event.rs`, `app/event_dispatch.rs`, and session
  replacement paths: owner-token completion dispatch and cancellation.
- `codex-rs/tui/src/chatwidget/status_controls.rs`,
  `bottom_pane/status_line_setup.rs`, and the persistence handler: show command
  mode and block conflicting built-in writes.
- `codex-rs/tui/src/bottom_pane/{footer,chat_composer,mod}.rs`: replace the
  single `Option<Line>` status value with bounded `Vec<Line>` ownership;
  calculate footer height from accepted rows; truncate each row independently;
  keep instructional overlays and right-side badges higher priority than the
  external region.

## Staged delivery

### Stage 1: compatible, safe command path

Implement config validation (including both `/statusline` guards), fixture-
compatible JSON v1, local argv execution, contained formatter cancellation,
multiline/plain/ANSI/OSC8 parsing, multi-row footer plumbing, dependencies, and
last-good behavior. Add the checked-in deterministic exporter-derived fixture
and assert its enumerated required fields, omission rules, and rendered output;
this—not broad third-party Claude
compatibility—is the Stage 1 compatibility gate.

### Stage 2: setup UX

Make `/statusline` accurately display command mode and provide a disable/reset
path. Its Stage 1 opening/persistence guards already prevent conflicting writes.

### Stage 3: optional enhancements

Consider a refresh interval, richer Codex-only fields, and a command editor
only after v1 metrics show the runner is safe and responsive.

Also deferred here: stronger Unix containment that would close the `setsid()`
escape left open by the 2026-08-02 best-effort decision — a bubblewrap-backed
launcher on Linux and/or a cgroup v2 freezer/kill scope per formatter run, with
a documented fallback wherever neither is available (notably macOS). This is
future hardening, explicitly out of scope for v1.

## Verification plan

- Config/core tests: empty argv, rejected timeout, mutual exclusion, schema
  generation, and config removal. Cover both cross-layer conflict directions
  (user/project/profile/CLI) and assert diagnostics name the effective keys.
- Wire tests: exact Claude-compatible fixture shape; nullability/omission for
  every fixture field; raw rate-limit epochs; local versus remote cwd; stable
  command-session identity before and after server thread creation.
- Runner tests: timeout, nonzero exit, malformed UTF-8, empty output, CRLF,
  bare CR, C1 CSI, bidi controls, huge unterminated stdout/stderr, wide
  graphemes, valid ANSI/OSC8, invalid escape sequences, and background children
  in the formatter's process group.
- `codex-utils-pty` tests: Unix group termination kills same-group background
  children; Windows suspended-spawn/Job behavior terminates the whole tree
  without exposing a raw-handle assignment race. No test asserts Unix cleanup of
  processes that left the group — that escape is a documented limitation.
- Async/TUI tests: debounce versus execution timeout, one child tree at a time,
  thread switching during debounce and execution, stale owner/generation for
  formatter **and dependency** results, data collection in command mode,
  last-good retention, and command removal.
- TUI snapshots: configured command mode, multiline/ANSI output, and restored
  built-in mode.
- Cross-platform coverage: a foreign-OS remote session confirms local execution
  remains local while session-workspace JSON remains accurate.

Run `just fmt`; then the affected config/core and TUI package tests and scoped
`just fix -p` checks. Regenerate the schema. Because `codex-core` changes, ask
for the required complete `just test` decision after scoped tests pass.

## Re-review gate

Before code starts, re-review this document against the current TUI event and
session-replacement paths, the actual Claude status-line contract, and the
existing exporter. The reviewer must confirm: compatibility claim, two-CWD
semantics, owner-token cancellation, independent data dependencies, raw rate
limit data, platform-specific formatter cleanup and its documented Unix limits,
parser bounds, setup write guards,
and test coverage.
