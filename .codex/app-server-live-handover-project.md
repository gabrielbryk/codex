# Live app-server handover project

## Status

Natural-drain v1 was implemented and bootstrapped on 2026-08-12. This is an internal implementation
guide for Gabe's local Codex fork and its source-owned host lifecycle tooling. It deliberately lives
outside the user-facing `docs/` tree.

The implemented first release covers server identity, synchronized drain
admission, existing cross-process thread writer locks, private generation
sockets, a protocol-blind stable router, durable rollout/reconciliation, Fork
Fleet deployment, runtime diagnostics, durable evidence, remote-control ownership transfer, and
retained rollback packages. The one-time zero-active-turn bootstrap is complete: the stable router
owns the canonical socket and the first generation runs behind a private socket.

The host tooling also supports live replacement of the stable router itself. It uses Unix listener
rename semantics rather than descriptor transfer: accepted streams remain attached to their owning
router process, while the replacement binds the canonical pathname for new clients. The persisted
handover controller accounts for each process's sockets through `/proc`, validates PID start time,
socket inode, loaded source hash, routing revision, and a stable-socket RPC probe, and never treats
an unreadable connection inventory as empty. A failed replacement restores the old listener first,
then lets replacement-accepted streams drain before cleanup.

The `/proc/<pid>/fd` accounting reconcile runs in a dedicated user oneshot that deliberately has no
systemd mount namespace. `PrivateTmp`, `ProtectSystem`, and `ProtectHome` make those same-UID
descriptor links unreadable on this host; the broader generation rollout remains sandboxed in its
separate service. Finalization requires two zero-connection samples separated by a bounded
confirmation window. All socket-health writers classify persisted handover and failback phases as
`starting`, preventing a legacy direct app-server from claiming the temporarily absent canonical
pathname.

TUI process replacement, automatic reconnect/reattach, and mid-turn connection migration remain
later phases. Natural-drain v1 keeps each accepted connection on its original server and therefore
preserves live work without changing TUI code. Remote control now transfers exactly once at the
retirement boundary; this is generation ownership transfer, not transport migration of an active
remote client.

### Delivered release boundary

The production-ready natural-drain boundary includes:

- fail-closed, same-UID, connection-affine routing with bounded backend connect and connection
  limits;
- exact package/source/binary identity, sustained candidate health observation, and a stable-route
  proof before accepting a promotion;
- crash-recoverable `drainPending -> draining -> retiring -> retired` transitions, including a
  durable remote-control ownership checkpoint before the old process stops;
- generation-aware liveness, connection, failure-counter, rollout, and code-drift metrics;
- append-only JSONL transition audit plus one atomic evidence document per request;
- automatic retention of at least two retired generations for seven days, with a dry-run prune
  command and path-containment checks;
- exact-SHA build stamping through `CODEX_BUILD_COMMIT`, `GIT_COMMIT`, and `STABLE_GIT_COMMIT`.
- crash-resumable stable-router replacement and active-phase failback without closing accepted
  streams on either router process.

The full roadmap is not complete until the reconnect/reattach and TUI safe-boundary executable
handoff acceptance criteria are proven. Those are P2 follow-on features and are not required to use
natural drain safely.

## Executive decision

Use a hybrid design:

- external tooling owns the stable control socket, app-server generations,
  health checks, promotion, rollback, process scopes, and package pointers;
- Codex app-server adds a small backward-compatible identity and drain API;
- natural-drain v1 makes no TUI or app-server-client changes;
- later phases may add reconnect/reattach and safe-boundary TUI replacement
  without changing the external router contract.

A raw external socket router is sufficient to preserve existing connections
while new connections move to a candidate generation. It is not sufficient to
migrate an accepted WebSocket, restore connection-scoped subscriptions, reject
new work during drain, or upgrade an already-running TUI executable. A
protocol-aware external proxy could emulate those behaviors, but that would
duplicate app-server state and JSON-RPC semantics outside their owning code.

## Problem

The managed local app-server currently owns the canonical Unix socket directly.
The daemon lifecycle implements restart as stop followed by start. Package
activation and daemon replacement are therefore blocked whenever any assistant
turn is active, because stopping the server destroys its in-memory execution
state and all accepted WebSocket connections.

This has two user-visible consequences:

1. A validated fork build cannot become the active server until every turn on
   the host is idle.
2. Restarting the server does not update already-executed TUI processes, so TUI
   fixes remain unavailable in existing panes even after a successful server
   cutover.

The target experience is a rolling local upgrade:

1. Start and validate a candidate app-server beside the current generation.
2. Route new clients to the candidate without disturbing existing connections.
3. Let active turns finish on the old generation.
4. Let each old TUI disconnect naturally after its work is complete.
5. Retire the old generation only when it owns no loaded threads or connections.
6. Add active TUI migration and rollback automation in a later phase.

## Existing behavior and reusable foundations

### Transport and connection ownership

- The TUI connects to `app-server-control.sock` with WebSocket RPC over a Unix
  stream through `RemoteAppServerClient`.
- The app-server binds that socket itself and creates one connection-scoped
  transport task per accepted WebSocket.
- The remote client already reconnects to the configured endpoint, repeats the
  `initialize` handshake, and replays the narrowly allowlisted idempotent
  requests it parked during disconnection.
- Reconnection is currently transport-transparent to the TUI. It does not
  automatically issue `thread/resume` for the displayed and side threads.
- Server connection teardown removes the connection from every subscribed
  thread and cleans connection-scoped request, filesystem, command, and process
  state.

### Thread reattachment

`thread/resume` is already the correct reattachment primitive. For a running
thread it rejoins the loaded conversation, associates the new connection,
returns current thread state including the active turn, attaches the listener,
and replays outstanding server requests for that thread. A new
`thread/subscribe` RPC is not required for the first implementation.

### Lifecycle

- The app-server daemon currently tracks one managed PID and one canonical
  socket.
- Restart stops the managed process before spawning its replacement.
- On Huginn, every persistent daemon must run inside its own transient
  `codex-app-server-daemon-*.scope` under `agent-control.slice`.
- Invoking oneshots retain `KillMode=control-group`; persistent children survive
  by entering their own scopes. This invariant must not be weakened.
- Process and socket liveness, PID/start-time identity, initialization health,
  and active turns are distinct proof surfaces.

### Package activation

The canonical package builder already produces immutable versioned release
directories. The current helper has one `standalone/current` pointer and blocks
changing it while turns are active. The new lifecycle needs independent desired
versions for TUI clients and app-server generations.

Splitting those pointers enables three deployment modes:

- client-only: move `client-current` and re-exec capable TUIs against the
  unchanged server after compatibility proof;
- server-only: roll the app-server generation while compatible existing TUIs
  reconnect and resume without replacing their process image;
- combined: prepare both package roles, promote the server generation, then
  re-exec each TUI at its safe boundary.

The lifecycle should choose the narrowest mode supported by the package diff
and declared capabilities. It must fall back to the combined or legacy path
when it cannot prove that a component is unchanged.

## Goals

- Preserve active turns, streaming output, steering, approvals, and server
  requests while a candidate server starts and new connections move to it.
- Allow new and idle clients to use the candidate immediately instead of
  waiting for unrelated active turns.
- Keep active TUI processes usable without requiring them to reconnect or
  replace their executable in v1.
- Preserve the current backward-compatible app-server v2 contract for clients
  that do not understand draining or generations.
- Make promotion, drain, retirement, and rollback observable and auditable.
- Maintain exactly one remote-control owner for a managed Codex home.
- Keep the stable router protocol-agnostic and the app-server as the owner of
  application semantics.
- Support Linux, macOS, and Windows at the Codex protocol and TUI layers. The
  Huginn systemd implementation is Linux-specific host policy.

## Non-goals

- Moving a Rust future, Tokio task, `ThreadManager`, model stream, tool process,
  or accepted WebSocket file descriptor into another server process.
- Serializing an in-flight assistant turn and reconstructing it in a new server.
- Sharing one thread's live in-memory state between two app-server generations.
- Making arbitrary protocol-breaking client/server pairs interoperable.
- Hiding all upgrade transitions from logs and diagnostics.
- Replacing the existing rollout JSONL and SQLite persistence model.
- Adding new functionality to `codex-core` unless later implementation evidence
  proves that no owning app-server or client crate can provide it.

## Architecture

```text
                         stable for the lifetime of the host session
 TUI / desktop bridge ── app-server-control.sock
                                  │
                                  ▼
                          byte-stream router
                           │             │
                existing connections    new connections
                           │             │
                           ▼             ▼
                generations/old.sock   generations/new.sock
                     old app-server      candidate app-server
                       draining             accepting
```

The router selects one backend when it accepts a client and pipes bytes in both
directions for the lifetime of that connection. It does not terminate the
WebSocket or parse JSON-RPC. Existing connections therefore remain pinned to
their original server even after the default generation changes.

Each app-server generation owns:

- an immutable package directory and exact source SHA;
- a private socket path;
- a PID plus process start time;
- its own transient systemd scope;
- generation state and health observations;
- zero or more client connections and loaded threads.

The lifecycle owns a single active-routing generation and may additionally own
one draining generation. More than two live generations should be rejected in
the first implementation.

## Shared state and thread ownership

Both generations initially use the same `CODEX_HOME`. That means they can see
the same SQLite state, rollout JSONL files, credentials, configuration, plugin
registry, and other process-global resources. SQLite's ability to support
multiple processes is not by itself proof that two complete app-servers are
safe to run concurrently.

Stage 0 must inventory every writable shared resource and classify it as:

- safely multi-process;
- single-writer with an existing lock or lease;
- generation-local and therefore moved beneath the generation directory;
- unsafe until guarded by a new ownership mechanism.

At minimum, the test harness must cover SQLite journal/locking mode, rollout
append ownership, thread metadata writes, configuration watchers, MCP/plugin
startup processes, authentication refresh, analytics flushes, update loops,
command/process managers, and remote-control persistence.

Thread execution follows a strict ownership rule:

- accepting new may load and write a thread only after draining old has crossed
  a durable safe boundary for that thread;
- draining old rejects every operation that could create new work on a migrated
  thread, regardless of whether the caller is local or remote;
- after handoff, old may retain a stale in-memory read view temporarily but may
  not append to that thread's rollout or mutate its runtime settings;
- rollback after candidate work begins is a new resume on old from durable
  history, not simultaneous execution in both generations.

The preferred first implementation reuses existing completion persistence and
`thread/unsubscribe` only if tests prove that the unsubscribe response after an
idle turn is a sufficient durability and ownership barrier. Otherwise add one
narrow `thread/handoff/prepare` RPC that flushes the owning thread, records a
durable history cursor or revision, prevents further work on old, and returns
the proof the candidate must validate before `thread/resume`.

## Runtime layout

```text
~/.codex/packages/standalone/
    current -> releases/local-new
    releases/
        local-old/
        local-new/

~/.codex/app-server-control/
    app-server-control.sock
    generations/
        old/
            app-server.sock
            daemon/app-server.pid
        new/
            app-server.sock
            daemon/app-server.pid

~/.local/state/codex-app-server/
    routing-state.json
    router-status.json
    desired-generation.json
    rollout-events.jsonl
    evidence/rollout-<request-id>.json
```

`current` remains the normal CLI/TUI selection. Each routing-state generation
records its immutable `packagePath`, so changing `current` does not change or
restart a running server generation.

Every state file is written atomically, is owner-only, and records an explicit
schema version. PID records retain the current PID plus process-start-time
identity checks.

## Operator runbook

Inspect before changing anything:

```bash
codex-app-server-active-turns
codex-app-server-rollout status | jq
systemctl --user status codex-app-server-router.service codex-app-server-rollout.timer
```

Prepare a reversible canary, then either roll back by promoting the standby package or complete the
normal promotion:

```bash
codex-app-server-rollout canary-promote --package <immutable-release>
codex-app-server-rollout promote --package <immutable-release>
```

The canonical Fork Fleet path builds, selects the package for new clients, and records a durable
server rollout request:

```bash
codex-use-local-build --repo <candidate-worktree> --sha <full-sha> --rollout \
  --expect serverDrainV1 --expect threadWriterLeaseV1
codex-app-server-rollout apply-pending
codex-app-server-rollout status | jq
```

`desired-generation.json` remains present until the replacement is active and the old server has
proved its drain barrier. A nonzero draining count is safe deferred completion. An error or
`drainPending` count requires reconciliation; do not delete desired state or bypass the barrier.

Retention is automatic during reconciliation. Preview and explicitly apply it with:

```bash
codex-app-server-rollout prune
codex-app-server-rollout prune --apply
```

The defaults are a seven-day retirement age and two retained retired generations. They are
configurable with `CODEX_ROLLOUT_RETIRED_RETENTION_SECONDS` and
`CODEX_ROLLOUT_RETAINED_RETIRED_GENERATIONS`. Candidate health defaults to repeated probes over five
seconds and is controlled by `CODEX_ROLLOUT_HEALTH_OBSERVATION_SECONDS` and
`CODEX_ROLLOUT_HEALTH_PROBE_INTERVAL`.

Never restart the router merely because installed code differs from its running source hash. Use
`codex-app-server-router-handover prepare`. The handover renames the bound listener, starts the
replacement at the canonical pathname, keeps old accepted streams open, and reconciles until the
old process has zero kernel-observed established sockets. Server-generation mutation is suspended
during that interval because only the old router can account for its pinned backend connections.
Finalization moves the replacement listener to a temporary hold path before stopping old, preventing
old shutdown cleanup from unlinking the new router's pathname, then restores the canonical name.
If readiness or health fails while the old router is still available, failback first returns that
old listener to the canonical pathname. The rejected replacement keeps its renamed listener and
accepted streams until its kernel-observed connection count reaches zero. The controller blocks
server-generation changes throughout `preparing`, `active`, `finalizing`, `failbackPreparing`, and
`failingBack` so neither router's hidden connections can be omitted from a retirement decision.

## Server identity and compatibility

The initialize response should add optional, backward-compatible fields:

```json
{
  "userAgent": "codex_cli_rs/0.148.0-alpha.9",
  "codexHome": "/home/gabe/.codex",
  "platformFamily": "unix",
  "platformOs": "linux",
  "serverIdentity": {
    "instanceId": "01...",
    "generation": "local-...",
    "sourceSha": "b27edf...",
    "protocolRevision": 1,
    "capabilities": [
      "serverDrainV1",
      "threadWriterLeaseV1"
    ]
  }
}
```

`instanceId` changes on every process start. `generation` identifies the
immutable package deployment. `protocolRevision` is a deliberately reviewed
compatibility number, not a replacement for feature capabilities or schema
fixtures. Missing identity means legacy behavior: do not attempt automatic
migration and retain the existing all-turns-idle cutover gate.

The lifecycle supplies generation and source identity from the already-verified
immutable package through a narrow startup argument or environment contract.
The server must not derive its identity from the mutable `current`,
`client-current`, or `server-current` links. The process-generated `instanceId`
is then combined with that package identity in initialize and diagnostics.

Promotion requires:

- a successful bounded socket and initialize probe;
- expected Codex home and platform;
- a supported protocol revision and required capabilities;
- the expected package generation, source SHA, and binary checksum;
- candidate health sustained for a configurable short observation window;
- no unmanaged process owning the private generation socket.

Semver equality alone is not a compatibility proof.

## Drain protocol

Add backward-compatible, capability-gated app-server v2 methods:

- `server/drain/start`
- `server/drain/status`
- `server/drain/cancel`

Natural-drain v1 uses polling and does not add notifications. States are:

```text
Accepting
Draining { generation, target_generation, started_at }
```

The status response reports at minimum:

- process instance and generation identity;
- accepting/draining state;
- running assistant turn count;
- attached connection count;
- loaded subscribed-thread count;
- pending server-request count;
- active pairing or other non-thread elicitation count when relevant.

### Admission behavior while draining

The server must keep accepting operations required to finish existing work:

- `turn/steer` and `turn/interrupt`;
- approval and elicitation responses;
- server-request responses;
- thread reads, history reads, and status requests;
- explicit drain status and cancellation operations.

It must reject requests that create new durable work, including at least:

- `thread/start` unless needed for a documented internal recovery path;
- `turn/start`;
- review start;
- compaction start;
- fork or other operations that start background agent execution;
- new long-running command/process facilities not owned by an existing turn.

Rejected requests return a structured, retryable `serverDraining` error with
the target generation. This admission check must occur at the owning request
processor immediately before work creation, not only in external tooling. That
closes the race where an active-turn probe reaches zero just before a new turn
starts.

Drain status unloads each non-active thread through the existing bounded
shutdown/removal path, which releases its `$CODEX_HOME/thread-writer-locks`
file lock. Retirement requires both no loaded threads and no router connections.
Cancellation is allowed only before the first writer is released.

Because v1 does not migrate connections, `thread/resume` is rejected on a
draining generation. A TUI whose transport fails during drain may need to wait
for the old ownership to release and resume through the new generation; seamless
mid-drain reconnect is part of the later reattachment phase.

## Remote client reconnect behavior

The remote client should preserve its existing transport retry and idempotent
request replay. Add a distinct successful-reconnect event containing the old
and new server identities:

```text
AppServerEvent::Reconnected { previous, current }
```

Also add an explicit client command that closes and reconnects at the TUI's
chosen safe boundary. Do not make an upgrade reconnect indistinguishable from a
network failure.

After reconnect:

1. Repeat `initialize` and validate the new identity.
2. Replay only the existing allowlisted idempotent requests.
3. Surface `Reconnected` to the TUI.
4. Let the TUI issue `thread/resume` for its primary, displayed, and side
   threads.
5. Deduplicate the reconstructed snapshot against already-rendered item IDs and
   accept the authoritative active-turn state from the new subscription.

If reconnect lands on the same instance, reattachment is still required
because connection-scoped subscriptions were removed. If it lands on a new
generation, the TUI must verify that the desired thread exists there before it
detaches from an old server that still owns active work.

## TUI safe-boundary handoff

Server handover alone does not activate new TUI code. Existing panes must
replace their running executable.

When a capable TUI learns that its server is draining:

1. Continue rendering the active turn and accepting steering, interruption,
   approvals, and server-request responses against the old connection.
2. Queue user input that would start a new turn instead of sending it to the
   draining server.
3. Wait for the current thread to become idle and for pending client-owned
   server requests to resolve.
4. Cross the tested thread durability/ownership barrier on old, then unsubscribe
   the migrating connection.
5. Write a bounded owner-only handoff envelope that includes the returned
   durable history revision when the barrier provides one.
6. On Unix, replace the process image with the target `client-current` Codex
   binary through `execve`, preserving the terminal process identity.
7. On Windows, use a platform-specific spawn-and-replace path with an explicit
   readiness handshake.
8. Connect through the stable socket, initialize, resume subscribed threads,
   restore local state, delete the envelope, then submit queued input.

The handoff envelope contains only bounded local presentation state:

- schema version and nonce;
- expected old and target server identities;
- durable history revision or handoff proof when required;
- primary and displayed thread IDs;
- subscribed side-thread IDs;
- effective profile, cwd, and explicit CLI/config overrides needed to recreate
  the session;
- composer draft, queued steer/input, and selected UI mode;
- terminal restoration metadata that is not already recoverable at startup.

It must not contain credentials, arbitrary environment variables, model
response bodies already persisted in rollout history, or unbounded transcript
data. The new process validates ownership, mode, age, nonce, target binary, and
server identity before consuming it. Failed handoff leaves the file for a
bounded recovery window and prints a manual resume command.

## Remote-control ownership

Only one server generation may own the persisted remote-control connection for
a Codex home.

Initial promotion sequence:

1. Start the candidate with remote control disabled ephemerally.
2. Route new local connections to the candidate.
3. Drain local and remote work on the old generation.
4. Disable and verify remote control on old.
5. Enable and verify persisted enrollment on the candidate.
6. Mark the candidate as the remote-control owner.
7. Stop old only after ownership and visibility proof succeeds.

Failure during transfer keeps or restores old ownership. The lifecycle must
never start two uncontrolled remote-control-enabled generations and hope that
enrollment resolves the race.

## External lifecycle state machine

```text
Stable(old)
  -> Preparing(new)
  -> CandidateReady(old,new)
  -> RoutingNewToCandidate(old drainPending,new active)
  -> DrainingOld(old draining,new active)
  -> TransferringRemoteControl(old,new)
  -> RetiringOld(old retiring,new active)
  -> Stable(new)

Any pre-retirement failure
  -> RollbackRoutingToOld
  -> StopCandidateWhenSafe
  -> Stable(old)
```

Every transition is compare-and-swap against the persisted state generation,
audited with request ID, old/new package identities, PIDs, socket paths, active
turns, connection counts, and outcome. A crash-recovered reconciler reads the
state and performs only idempotent transitions.

`retiring` is the durable checkpoint after connection/thread quiescence and remote-control transfer
but before process stop. If stopping the old daemon or the controller process fails, the next
reconcile resumes from that checkpoint without repeating ownership transfer or claiming the server
was already retired.

The stable router must fail closed:

- it never routes to an unverified or absent backend;
- if the selected backend disappears before connect, it may fall back only to
  a verified generation explicitly permitted by routing state;
- existing byte streams are not silently moved between backends;
- malformed or unreadable routing state does not trigger a daemon restart;
- socket-file existence alone is never treated as liveness.

## Source ownership

### Codex fork

Implemented code surfaces:

- `codex-rs/app-server-protocol/src/protocol/v1.rs`: additive initialize
  identity, unless a compatibility-preserving v2-owned representation can be
  exposed through the existing initialize response.
- `codex-rs/app-server-protocol/src/protocol/v2/server_lifecycle.rs`: drain
  request and response types.
- `codex-rs/app-server-protocol/src/protocol/common.rs`: method mappings.
- `codex-rs/app-server/src/generation_lifecycle.rs`: focused process-local drain state and
  admission decisions.
- `codex-rs/app-server/src/request_processors/`: drain RPC processor and narrow
  admission checks at work-creation owners.
- `codex-rs/app-server-daemon/`: generation-aware native status only where it
  belongs in cross-platform lifecycle reporting; host-specific routing policy
  remains external.

Do not grow `codex-core` or central TUI orchestration files with the new
concepts. Use focused private modules and explicitly export only required crate
API.

### Host lifecycle tooling

Canonical owner: `~/workspace/personal/tooling/claude-process-guard`.

Implemented additions and changes:

- new `bin/codex-app-server-router`;
- new `bin/codex-app-server-router-handover`, installed from the same source-owned repository;
- new `bin/codex-app-server-rollout` and reconciliation timer;
- immutable per-generation package, socket, and daemon directories;
- user-systemd router unit and transient generation scope integration;
- router/controller hardening, generation-aware Prometheus metrics, JSONL audit, and request
  evidence;
- source-owned manifests and tests;
- compatibility migration from the current one-PID state.

Do not move these files into chezmoi or patch installed copies as the source of
truth.

### Fork Fleet and package tooling

Canonical owner: the Worklens `libs/fork-fleet` source tree.

Extend package activation with an explicit `--rollout` staged
deployment operation:

```text
prepare package
promote client-current when allowed
prepare candidate server generation
verify compatibility and health
promote routing
drain and migrate
retire or roll back
record durable report
```

The maintain/upgrade skill must continue to use the old active-turn-deferred
path until the runtime reports all required capabilities.

## Staged delivery

Natural-drain v1 deliberately combines the smallest safe subset of the earlier
stages: existing writer-lock proof, initialize identity, server drain/admission,
the blind router, generation lifecycle, and Fork Fleet deployment. The TUI and
remote-client portions below remain the roadmap rather than acceptance criteria
for v1.

Each stage is independently reviewable and should remain below the repository's
change-size guidance. Avoid combining Codex protocol, TUI process replacement,
and host router implementation in one branch.

### Stage 0: invariants and test harness

- Record exact connection, active-turn, pending-request, and process identity
  observations needed by later gates.
- Add a two-generation integration harness with private temporary sockets and
  deterministic mocked responses.
- Inventory and test every shared `CODEX_HOME` writer under two simultaneous
  generations; move unsafe process-local resources into generation state or add
  explicit ownership.
- Prove whether turn completion plus `thread/unsubscribe` is a sufficient
  durable ownership barrier; specify `thread/handoff/prepare` if it is not.
- Prove baseline behavior: transport reconnect initializes but does not restore
  thread subscriptions automatically.

Exit gate: the harness can detect lost subscription, duplicate events, missing
completion, wrong-generation connection routing, concurrent rollout writers,
and SQLite or process-resource ownership violations.

### Stage 1: reconnect identity and thread reattachment

- Add optional initialize server identity and capabilities.
- Surface successful reconnects from the remote client.
- Have the TUI resume all owned threads after same-generation reconnect.
- Reconcile snapshots without duplicating already-rendered items.
- Replay outstanding approval/server requests through existing server support.

Exit gate: deliberately severing the TUI socket during a streaming turn does
not exit the TUI; it resumes the thread, receives completion exactly once, and
can answer a pending approval.

This stage improves ordinary transient disconnect behavior independently of
rolling upgrades.

### Stage 2: server drain protocol

- Add drain state, status, notifications, cancellation, and structured
  `serverDraining` rejection.
- Add admission checks for all work-producing RPCs.
- Expose required drain-completion gauges.
- Update app-server API documentation and generated schemas.

Exit gate: an active turn can be steered and completed while new turn creation
is consistently rejected; drain completes without a probe/start race.

### Stage 3: stable router and generation lifecycle

- Introduce the byte-stream router and private generation sockets.
- Start candidate processes in dedicated scopes with remote control disabled.
- Add immutable routing state, candidate health observation, promotion, and
  rollback.
- Split `client-current` from `server-current`.
- Retain legacy single-server mode as a fail-safe until migration is complete.

Exit gate: existing connections remain on old, new connections reach new, and
rollback affects only new connections while both server processes remain
healthy and correctly scoped.

### Stage 4: TUI live executable handoff

- Add safe-boundary input queuing.
- Add the bounded handoff envelope and validation.
- Add Unix `execve` and Windows spawn-and-replace implementations.
- Restore thread subscriptions, composer state, and queued input.
- Add visible progress and a manual recovery path.

Exit gate: the same terminal pane changes `/proc/<pid>/exe` on Unix, resumes the
same thread on the candidate, preserves queued input, and does not lose or
duplicate transcript events.

### Stage 5: remote-control transfer and automated retirement

- Transfer remote-control ownership after local drain.
- Retire old when all gates are satisfied.
- Add rollback before and during ownership transfer.
- Teach Fork Fleet to drive and report the complete deployment.

Exit gate: exactly one remote-control owner is visible throughout; an injected
candidate or transfer failure returns routing and ownership to old without
interrupting its active work.

### Stage 6: one-time production bootstrap

- Build and stage the router-capable release.
- Wait for the existing all-turns-idle safety gate.
- Stop the current direct socket owner.
- Start the stable router and one generation behind its private socket.
- Verify local TUI, desktop bridge, remote-control, process scope, package
  identity, reconnect, and rollback surfaces.

Exit gate: the router owns the canonical socket and ordinary later deployments
no longer require a host-global zero-active-turn window.

## Verification plan

### Codex protocol and server

- Initialize decoding remains compatible when identity fields are absent.
- Capability and protocol-revision comparisons are exhaustive.
- Drain methods and notifications serialize with the intended camelCase wire
  names and generated TypeScript location.
- Every work-producing method is classified as admitted or rejected while
  draining; use an exhaustive owner-side classification where practical.
- Drain cancellation returns to accepting only before irreversible retirement.
- Connection close and resume preserve active turn state and replay pending
  server requests.
- Thread handoff proves a durable last-written history revision before the
  candidate may create new work.
- App-server schema fixtures and `app-server/README.md` are updated.

Run at minimum:

```text
just write-app-server-schema
just write-app-server-schema --experimental   # if an experimental surface is used
just test -p codex-app-server-protocol
just test -p codex-app-server-client
just test -p codex-app-server
```

### TUI

- Socket loss during streaming output reconnects without fatal exit.
- Same-generation and changed-generation reconnects take distinct paths.
- Primary, displayed, and side threads reattach exactly once.
- A pending approval is presented after reattachment.
- New input is queued while draining and submitted only after candidate resume.
- Handoff rejects stale, foreign-owner, wrong-mode, wrong-generation, oversized,
  and already-consumed envelopes.
- Failed process replacement leaves a usable old TUI or a precise recovery
  command.
- UI changes include `insta` snapshot coverage.

Run `just test -p codex-tui`, inspect pending snapshots, and accept only the
intentional updates.

### Host tooling

- Router maintains per-connection backend affinity.
- Backend EOF affects only that connection and is observable.
- Invalid routing state fails closed.
- Promotion is atomic under concurrent connects.
- Reconciler crash recovery is idempotent at every state transition.
- PID/start-time checks reject reused PIDs.
- Candidate processes remain in their transient scopes after the invoking
  oneshot exits.
- `KillMode=control-group` continues to reap environment-import strays.
- Liveness distinguishes absent, starting, responsive, unresponsive, unmanaged,
  and indeterminate generations.
- Cold boot, not only restart, is covered.

### End-to-end fault matrix

Inject failures at each boundary:

- candidate does not bind;
- initialize times out;
- identity or protocol revision mismatches;
- router dies and restarts;
- old or new backend dies;
- TUI disconnects during assistant delta delivery;
- approval is outstanding during disconnect;
- a new turn races drain start;
- TUI handoff exec fails;
- candidate becomes unhealthy after promotion;
- remote-control disable or enable fails;
- host reboots with one draining generation recorded.
- both generations attempt to open or mutate each classified shared
  `CODEX_HOME` resource.

For every case assert routing, server ownership, active turns, client-visible
behavior, persisted state, audit outcome, and recovery action.

Before finalizing each large Rust stage, run scoped `just fix -p ...`, then
`just fmt`; per repository policy do not rerun tests after fix or format. If a
stage changes common, core, or protocol in a way requiring the complete suite,
ask before running the complete `just test`.

## Observability and operations

Expose and record:

- router PID, age, current generation, accepted connections by generation, and
  routing-state revision;
- each server's PID/start time, exact executable, source SHA, socket, scope,
  health classification, active turns, connected clients, pending requests,
  drain state, and remote-control ownership;
- each migration attempt's thread ID, client PID, old/new instance IDs, safe
  boundary, result, and duration without logging prompt or response content;
- package pointers and immutable release checksums;
- last transition request and rollback reason.

The runtime doctor, server-status helper, metrics exporter, and Fork Fleet
report should consume the same source-owned generation state rather than infer
it independently.

Operational commands need dry-run and JSON modes. No helper may interpret a
socket path's existence as proof of a live server or automatically restart an
unresponsive live PID.

## Security and correctness constraints

- Control and private generation sockets remain in owner-only directories and
  use mode `0600`.
- State and handoff files are atomic, bounded, schema-versioned, and mode
  `0600`.
- The router passes bytes without logging JSON-RPC payloads.
- The handoff envelope never copies credentials or broad environment state.
- Generation selection uses canonical absolute paths and verified immutable
  package metadata.
- No destructive cleanup uses unresolved globs or broad directory roots.
- Old releases remain recoverable until post-cutover soak succeeds.
- The server never reports drain complete while a pending approval or other
  server request still depends on an attached client.

## Rollout and rollback

Before promotion, preserve:

- old package and pointer targets;
- old generation PID/start-time and socket identity;
- routing state and remote-control owner;
- active-turn and connection inventory;
- candidate package manifest and checksums.

Rollback before old retirement is a routing-state update back to old followed
by candidate drain/stop when safe. Never move existing connections between
servers during rollback.

After old retirement, rollback is a new rolling deployment using the preserved
old package as the candidate. Do not pretend that stopped in-memory turns can be
recovered by repointing a symlink.

The default retention policy keeps at least two retired generations and never removes one until it
has been retired for seven days. Cleanup refuses paths outside the generation and immutable-release
roots and never deletes the current package. Operators can lengthen the soak without code changes.

## Natural-drain v1 acceptance

The delivered v1 is accepted when all of these are simultaneously true:

- the router owns the canonical socket and its PID/start identity, status freshness, routing
  revision, and loaded source hash are valid;
- the candidate package manifest and all declared file checksums validate before process start;
- repeated health probes pass for the configured observation window;
- a stable-socket initialize reaches the exact promoted generation before drain begins;
- the old server reports `draining` with the exact replacement and cannot create new work;
- retirement waits for both an empty loaded-thread set and zero live routed connections;
- remote control has zero or one owner throughout and ownership is durably checkpointed before old
  stops;
- injected drain, transfer, stop, PID-reuse, stale-router, and path-containment failures leave a
  retryable or rolled-back state;
- metrics, alerts, audit JSONL, and per-request evidence identify incomplete and failed rollouts;
- Fork Fleet embeds the exact source SHA and performs a genuinely source-distinct canary/build.

These criteria do not claim that an already-running TUI process changes executables or that a
broken socket is reattached mid-turn. Those remain in the full-project criteria below.

## Acceptance criteria

This project is complete only when all of the following are proven:

- The stable router, not an app-server generation, owns the canonical socket.
- A candidate can start, validate, and receive new connections while another
  generation has active turns.
- Existing active turns continue on old without missing or duplicate terminal
  transcript events.
- Draining old cannot accept a new turn after its drain admission barrier.
- A capable TUI reattaches after ordinary reconnect and after generation
  migration.
- An existing TUI process activates the new binary at a safe boundary while
  preserving its thread and queued input.
- Pending approvals and server requests survive reconnection or block migration
  with an explicit reason.
- Remote control has exactly one verified owner.
- Candidate failure before retirement rolls new connections back to old.
- Lifecycle state recovers correctly after process crash and host reboot.
- Runtime status proves exact package, binary, PID/start time, socket, scope,
  routing generation, active turns, and connected clients.
- Fork Fleet performs and reports the deployment without bypassing safety
  checks.
- The legacy all-turns-idle deployment remains available when either side lacks
  the required capabilities.
- Two-generation operation has no unclassified shared writer and never permits
  both generations to append work to the same thread.

## Open decisions

The remaining decisions concern the P2 reconnect/TUI roadmap. Natural-drain decisions are recorded
inline as resolved so they are not reopened accidentally:

1. **Resolved for v1:** server identity is additive in the initialize response.
2. **Resolved for v1:** drain is fork-private and capability-gated at protocol revision 1.
3. **Resolved for v1:** work-producing RPC admission is exhaustively owned by the server lifecycle
   classifier and covered by server tests.
4. Whether existing completion persistence plus `thread/unsubscribe` is a
   sufficient thread ownership barrier or a new `thread/handoff/prepare` RPC is
   required.
5. **Resolved for v1:** thread writer locks protect shared rollout writers; generation-local daemon
   state isolates process ownership; remote control is explicitly transferred. Re-audit if a P2
   feature introduces another shared writer.
6. Whether idle side threads migrate in the first TUI handoff or lazily on
   navigation.
7. How much composer and modal state is safe and worthwhile to preserve across
   process replacement.
8. The Windows process-replacement mechanism and terminal ownership handshake.
9. **Resolved for v1:** five seconds of repeated health probes, configurable by environment; seven
   days before retirement cleanup.
10. **Resolved for v1:** remote-control ownership transfers only after natural drain; active remote
    connection migration is deferred.
11. **Resolved for v1:** retain at least two retired generations and require seven days of age;
    both limits are configurable.

These decisions do not change the architectural boundary: transport routing and
process generations remain external; connection reattachment, admission
control, and TUI state replacement remain in their owning Codex components.
