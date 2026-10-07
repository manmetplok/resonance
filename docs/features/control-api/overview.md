# Control API: architecture

> Verified against master @ `8eac71b3` + vocal-phase1 fixes (`d1b5c284`) on 2026-10-07.
> Spec history: the original design was ba doc #265 (control protocol) and
> #266 (MCP server), epic #200; ba is retired and those docs are not in the
> repo. Code comments still cite them and the todo numbers (#1147–#1161).

A running Resonance can be driven by another program: an AI agent through
the `resonance-mcp` MCP server, a script, or a test. The app listens on a
per-user unix socket for JSON-RPC requests. Every edit made this way goes
through the same update path as a GUI edit, so it shows up live in the
window and lands in the normal undo history.

```
Claude ──stdio(MCP)──▶ resonance-mcp ──unix socket(JSON-RPC)──▶ resonance-app
                       (ControlClient)                          control_socket → update/control
```

## Behaviour

### The three crates

- **`resonance-control`** is the wire contract. It holds the JSON-RPC
  envelope, newline framing, error kinds, every method name and every
  param/result type, the job types, and the socket-path rule. It has no
  internal dependencies (ARCHITECTURE.md layering table). Both sides
  compile against it.
- **`resonance-app`** hosts the endpoint: the socket threads
  (`control_socket.rs`), the job ledger (`control_jobs.rs`), the per-
  connection state (`state/control.rs`) and the handlers
  (`update/control/**`). The arch-invariants test (ARCH2-11) lets
  `resonance_control` be named only in those files, plus `lib.rs` and
  `engine_events/pool.rs` for two stated reasons.
- **`resonance-mcp`** is a stdio MCP server that turns each control method
  into one MCP tool. See [mcp-server.md](mcp-server.md).

### Socket and threads

- The binary starts with `Resonance::new_with_control()`, which calls
  `start_control_server()`. Tests use `new()`/`new_for_test()`, which never
  bind a socket.
- `RESONANCE_NO_CONTROL=1` turns the endpoint off. A bind failure (another
  live instance owns the socket) logs `control endpoint disabled` and the
  app keeps running.
- Socket path: `$RESONANCE_CONTROL_SOCKET` if set; on macOS the per-user
  temp dir from `confstr(_CS_DARWIN_USER_TEMP_DIR)`; else
  `$XDG_RUNTIME_DIR/resonance/control.sock`; else
  `/tmp/resonance-<uid>/control.sock`. One function,
  `resonance_control::socket::socket_path`, serves both sides.
- The socket's directory must be a real, private (`0700`) directory owned
  by the user. The app creates it with one `mkdir(0700)` and never
  `chmod`s; both the app and the MCP client refuse a directory that is a
  symlink, someone else's, or group/other-accessible (CTL-11).
- One instance per socket path: an exclusive lock on `<socket>.lock` is
  held for the server's lifetime. Only the lock holder may replace a
  stale socket file. On drop, the socket file is removed only if its
  `(dev, inode)` still matches the one this instance bound.
- Threads: one `control-accept` thread, then per connection a reader
  thread and a writer thread. The reader parses lines and forwards each
  request into iced through one unbounded channel (`bridge_stream`,
  subscribed in `update.rs`). The writer drains that connection's
  `ReplySender` channel. The update loop never blocks on socket I/O.
- `job.wait` is the one exception to "everything goes through the update
  loop": the reader thread answers it directly by blocking on the
  `JobBoard` condvar. See [jobs.md](jobs.md).
- A malformed line gets a parse-error reply (id `null`) and the
  connection keeps reading. A frame over 16 MiB (`MAX_FRAME_LEN`) drops
  the connection.

### Dispatch on the update loop

`update::control::handle` receives `Message::Control(ControlMessage)`.
`Connected`/`Disconnected` maintain `ControlEndpointState::sessions`; a
disconnect also orphans the connection's live jobs and drops its pending
plugin-label lookups. Each `Request` runs through `execute`, in this order:

1. `control.hello` is always answered, even on an incompatible
   connection. A version mismatch marks the session `incompatible`, and
   every later request on it is refused `unsupported`. Hello is not
   required before other calls.
2. Ungated calls that do not edit the open project: `song.*` (plus
   `plugins.catalog` and `track.plugin_params`, which `song.rs` also
   answers), `job.*`, `amp_models.*`, `drum_kits.*`,
   `presets.*`, then `project.*` (which create the project the gate
   checks for). `plugins.rescan` is also ungated; it falls through to the
   `plugins` handler below without a compound-undo group.
3. **The mutation gate** (`mutation_gate_error`) for every other known
   method. It answers `busy` when: no project is active; a load or replay
   is pending; an offline mixdown is running (except `meter.*`); a GUI
   drag gesture is open (except the read-only list in
   `reads_open_project_only`, which includes `vocal.get_params`); or a bounce in place, freeze or stem export
   holds the offline renderer.
4. Gated calls run inside `with_compound_undo`, so one call makes at most
   one undo entry and one revision bump (CTL-03). Then the namespace
   handlers are tried in turn (`section`, `harmony`, `generate`, `vocal`,
   `render`, `meter`, `assist`, `reference_track`, `transport`, `track`,
   `plugins`, `external`, `master`, `bus`, `edit`, `notes`, `clip`,
   `arrangement`, `global`, `automation`, `midi_map`).
5. A method in `capabilities()` that no handler took answers
   `unsupported` "method not implemented yet"; an unknown method answers
   JSON-RPC `-32601` with kind `unsupported`.

Handlers never write project or engine state directly. They build an
ordinary domain `Message` and send it through `run_via_update`, which is
`Resonance::update`: the pre-dispatch gates, the frozen-input classifier,
`record_undo`, dispatch and the transaction commit all run, exactly as
for a click.

Every request gets exactly one reply. A handler that must wait for the
engine (a plugin resolving a parameter label with CLAP `text_to_value`)
takes `current_reply`, sets `deferred`, and answers later from
`label_resolved` / `expire_pending_labels`.

### Revision counter

`Resonance::revision()` (`session.revision`) is bumped once per committed
undoable transaction, including undo and redo restores. Every mutating
reply carries it (`MutationAck { revision }`, built only in
`update/control/reply.rs::mutation_ack`), and the `song.*` views carry it
too. A client that sees it jump by more than its own calls knows the user
edited in between. Library calls (`*.set_marks`, `presets.*`) and reads do
not bump it.

### Read views

The `song.*` views and every other wire projection of app state come from
one place, `update/control/view_model/` (`track.rs`, `clip.rs`,
`section.rs`, `lane.rs`, `position.rs`), so two methods cannot report the
same thing two ways (todo #1256).

### Remote indicator

While at least one client is connected, the transport bar shows a small
remote-control chip (`view/remote_indicator.rs`, fed by
`control_client_count()`). It is the only GUI this area has.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Wire contract | `resonance-control/src/lib.rs`, `rpc.rs`, `framing.rs` | `PROTOCOL_VERSION`, `Request`, `Response`, `RpcError`, `ErrorKind`, `MessageReader`, `MAX_FRAME_LEN` |
| Socket path + trust | `resonance-control/src/socket.rs` | `socket_path`, `resolve_socket_path`, `prepare_socket_dir`, `verify_socket_dir` |
| Method names + types | `resonance-control/src/methods/*.rs` | per-namespace `METHODS`, `capabilities()`, `deprecated_aliases()` |
| Socket server | `resonance-app/src/control_socket.rs` | `spawn`, `ControlServer`, `serve_connection`, `job_wait_response`, `bridge_stream` |
| Startup | `resonance-app/src/lib.rs` | `new_with_control`, `start_control_server`, `revision`, `control_client_count` |
| Endpoint state | `resonance-app/src/state/control.rs` | `ControlEndpointState`, `ControlSession`, `ReplySender`, `PendingLabel` |
| Dispatch | `resonance-app/src/update/control/mod.rs` | `handle`, `execute`, `run_via_update` |
| Mutation gate | `resonance-app/src/update/control/gate.rs` | `mutation_gate_error`, `bypasses_mutation_gate`, `reads_open_project_only`, `offline_render_busy_error`, `is_protocol_method` |
| Reply vocabulary | `resonance-app/src/update/control/reply.rs` | `success`, `failure`, `mutation_ack`, `ack_task`, `ack_or_compose_error`, `no_track` … |
| One undo per call | `resonance-app/src/undo/mod.rs` | `with_compound_undo` |
| Wire projections | `resonance-app/src/update/control/view_model/` | shared by every namespace |
| Indicator | `resonance-app/src/view/remote_indicator.rs` | `view` |

Invariants a change must keep:

- A mutating handler goes through `run_via_update`; never mutate state
  directly.
- Never write a `MutationAck` by hand; use `reply::mutation_ack`.
- A new namespace that edits the project must sit below the gate. If it
  edits nothing in the open project, add it to `bypasses_mutation_gate`.
- `state/` must not name `crate::control_socket` (ARCH2-05).

## Surfaces

- **GUI:** the remote-control chip in the transport bar.
- **Control API / MCP:** `control.hello` → `mcp__resonance__control_hello`.
  The full list is in [method-catalogue.md](method-catalogue.md).
- **Agent skills:** every skill in `resonance-agent-plugin/skills/*`
  preflights with `control_hello` ([agent-plugin.md](agent-plugin.md)).
- **Environment:** `RESONANCE_NO_CONTROL=1`, `RESONANCE_CONTROL_SOCKET`.

## How it's tested

| What | Where | Run |
|---|---|---|
| Envelope, framing, socket path, strict params, views | `resonance-control/tests/{envelope,framing,methods,socket_path,unknown_fields,views}.rs` | `./scripts/run-tests.py -p resonance-control` |
| Hello, dispatch skeleton, revision counter | `resonance-app/tests/control/control_endpoint.rs` | `./scripts/run-tests.py -p resonance-app` |
| Real socket round trip, stale socket, removal on drop | `resonance-app/tests/control/control_socket_roundtrip.rs` | same |
| Mutation gate (no project, pending load) | `control_mutation_gate.rs`, `control_mutation_gate_loading.rs` | same |
| One call = one undo = one revision | `control_undo_contract.rs`, `control_edit_undo.rs` | same |
| Shared reply shape and `not_found` wording | `control_reply_contract.rs` | same |
| Every `track.*`/`mixer.*` method reaches a handler | `control_track_dispatch.rs` | same |
| Shared view-model projection | `control_view_model_shared.rs` | same |
| Whole compose flow through control, real engine bounce | `resonance-app/tests/e2e_compose_via_control.rs` (its own test target, on the arch-invariants allow-list; uses the real `Resonance::new`) | `cargo test -p resonance-app --test e2e_compose_via_control` |
| Remote chip golden | `resonance-app/tests/performance/remote_indicator_snapshot.rs`, `tests/snapshots/remote_indicator_active-wgpu.png` | `RESONANCE_BLESS=1 …` to re-bless |
| `resonance_control` stays in the control layer | `tools/arch-invariants/tests/architecture.rs` (ARCH2-11) | `cargo test -p arch-invariants` |

Not covered: the iced subscription bridge itself (tests pump the bridge
channel into `update()` by hand); two app instances racing for the
instance lock; macOS socket-path resolution (unit-tested only through the
pure `resolve_socket_path`).

## Decisions and deviations

- Requests run one at a time on the update loop, in arrival order across
  all connections. That keeps handlers from racing the GUI; the cost is
  that a slow handler delays everyone.
- The gate refuses rather than queues. A `busy` reply is stable and
  retryable; an edit queued behind a pending replay would be wiped by it.
- `control.hello` is advised but not enforced first.
- The mutation gate is enforced once in `execute` instead of per handler,
  because the per-handler convention kept letting new handlers no-op and
  claim success (todo #1161).

## Known gaps

- `render.stems` has a wire type and a handler that answers `unsupported`,
  but is kept out of `capabilities`:
  [render-stems-control](../../intents/control-api/render-range-and-stems.md).
- No server push: clients poll (`song.*`, `job.status`, revision deltas).
  Nobody has asked for notifications yet, so there is no intent.
