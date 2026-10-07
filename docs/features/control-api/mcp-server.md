# MCP server (`resonance-mcp`)

> Verified against master @ `86e5c2af` + vocal-params work on 2026-10-07.
> Spec history: ba doc #266 (MCP server design), epic #200 (not in the
> repo). Lead: `resonance-mcp/README.md`, which is partly stale (see
> Decisions and deviations).

`resonance-mcp` is a small stdio program that lets Claude Code, Claude
Desktop or any MCP client drive a running Resonance. It connects to the
app's control socket and publishes one MCP tool per control method, with
schemas generated from the same Rust types the app uses. In Claude Code
the tools appear as `mcp__resonance__<tool>`.

## Behaviour

### Registration

```sh
cargo build --release -p resonance-mcp
claude mcp add --transport stdio resonance -- /absolute/path/to/target/release/resonance-mcp
```

- Keep the server name `resonance`. The agent plugin's skills, the
  lockstep test and permission allowlists all expect
  `mcp__resonance__<tool>`. The agent plugin ships no `.mcp.json` for this
  reason ([agent-plugin.md](agent-plugin.md)).
- If the app's socket is somewhere else, pass the same
  `RESONANCE_CONTROL_SOCKET` to both (`claude mcp add --env …`).
- The server starts even when the app is not running; it connects lazily
  on the first tool call.

### Tool naming and mapping

- Tool name = method name with its **first** `.` turned into `_`:
  `transport.seek` → `transport_seek`, `midi_map.learn` →
  `midi_map_learn`. 184 tools for 184 methods today; the full table is in
  [method-catalogue.md](method-catalogue.md).
- One module per area in `src/tools/`, not always one per namespace:
  `trackmix.rs` holds `track.*`, `mixer.*` and `plugins.*`; `arrange.rs`
  holds `section.*` and `harmony.*`; `compose.rs` holds `generate.*` and
  `notes.*`; `clip.rs` holds `clip.*`, `pool.*`, `arrangement.*` and
  `reference.load`. Each contributes a
  `#[tool_router(router = router_<x>)]` block;
  `ResonanceMcp::combined_router` sums them.
- Deprecated aliases (`methods::deprecated_aliases()`, empty today) get no
  tool, so an agent sees one spelling per operation.
- How a tool calls through:
  - `invoke_structured` for views and most edits: the result is returned
    as structured content with an `output_schema` from the wire type.
  - `invoke` for some edits: the result as pretty JSON text.
  - `invoke_job` for job methods: start, wait up to the tool's budget,
    return the final `JobStatus` ([jobs.md](jobs.md)).
- Tools carry MCP annotations (`read_only_hint`, `destructive_hint`,
  `idempotent_hint`, `open_world_hint = false`).

### Schemas

- Input and output schemas come from `resonance-control` with the
  `schemars` feature, so they cannot drift from the wire types.
- `normalize_schemas` rewrites every published schema: it inlines every
  `$ref` and drops `$defs` (some clients forward schemas without `$defs`
  and then send ids as strings), strips schemars' non-standard numeric
  `format`s (`uint64`, …), and closes `additionalProperties` on every
  input schema.
- `check_arguments` refuses an argument a tool does not declare before
  anything is sent, naming it and the accepted ones (ARCH2-06). A params
  parse failure that rmcp would report as a protocol error is turned into
  a tool error with serde's message, so the model can fix the call.

### Errors

Every recoverable failure is a **tool execution error** with text that
says what to do next (`CallError::actionable_message`), never a protocol
error:

| Failure | What the model is told |
|---|---|
| App not running | start the app (without `RESONANCE_NO_CONTROL=1`) and retry |
| Socket directory not private | refuse; tell the user; do not retry |
| Connection dropped / app unresponsive (read deadline) | the next call reconnects; re-read state with `song_summary` before retrying an edit |
| Handshake rejected / version mismatch | rebuild the app and `resonance-mcp` from the same checkout |
| `not_found` | call a `song_*` view to list valid ids |
| `needs_confirmation` | retry with `confirm: true` (`overwrite: true`) if the loss is intended |
| `busy` | wait for the running job and retry |
| `unsupported` | this app build does not support it |

### Connection

- One connection per server process, behind a mutex, opened lazily. The
  first call (and the first after a drop) connects, checks the socket
  directory is trustworthy, and runs `control.hello`; a version mismatch
  is a handshake error.
- Deadlines: 120 s read for ordinary calls, 630 s for `job.wait` (the
  app caps a wait at 600 s), 30 s write (`CallTimeouts`). A timed-out
  connection is dropped and never reused, so a late reply can never be
  paired with a new request. RPC errors keep the connection.
- stdout carries only MCP JSON-RPC; logs go to stderr (`RUST_LOG`,
  default `info`).

### Server instructions

`get_info` sends a fixed `INSTRUCTIONS` text to the client: read before
writing, ids come from the `song_*` views, the revision contract, jobs,
verify every edit by reading it back (the agent cannot hear),
`control_hello` as the authority on what the running build supports,
`confirm`/`overwrite`, position units, and how automation tools fit
together.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Binary | `resonance-mcp/src/main.rs` | stderr logging, `ControlClient::new(socket_path())`, `serve(stdio())` |
| Server + router | `resonance-mcp/src/server.rs` | `ResonanceMcp`, `combined_router`, `invoke`, `invoke_structured`, `invoke_job`, `normalize_schemas`, `inline_defs`, `check_arguments`, `INSTRUCTIONS` |
| Socket client | `resonance-mcp/src/client.rs` | `ControlClient`, `call_blocking`, `connect`, `wait_job`, `CallTimeouts`, `CallError`, `JOB_WAIT_SLICE`, `MAX_JOB_WAIT` |
| Tools | `resonance-mcp/src/tools/*.rs` | one `#[tool_router]` per module; tool descriptions are the agent's only documentation |

Invariants: a new tool module must be added to `combined_router`; a tool
name must equal its method with the first `.` replaced; descriptions must
name real plugin ids and real fields (checked by `doc_hygiene.rs`).

## Surfaces

- **Control API / MCP:** all 184 tools; see the catalogue.
- **Agent skills:** `resonance-agent-plugin/skills/*` name these tools,
  checked by the lockstep test.

## How it's tested

All in `resonance-mcp/tests/`, run with `./scripts/run-tests.py -p resonance-mcp`.

| What | Where |
|---|---|
| Request mapping, structured results, error hints, handshake mismatch, reconnect, job waiting, one tool per method, unknown arguments, closed input schemas | `translation.rs` (fake socket server in `tests/common/`) |
| Read deadlines, dropped-on-timeout connection, `job.wait` long deadline | `client_timeout.rs` |
| Socket directory trust | `socket_trust.rs` |
| No non-standard formats, no boolean subschemas, no `$ref`/`$defs`, ids inline to integer | `schema_hygiene.rs` |
| Descriptions: real plugin ids, no padding runs, known traps stay documented | `doc_hygiene.rs` |
| Plugin-parameter tools publish `choices`, accept number or label | `plugin_param_schema.rs` |
| Skills name only real tools; protocol pin | `agent_plugin_lockstep.rs` ([agent-plugin.md](agent-plugin.md)) |

Not covered: a real Claude Code client against a real app (done by hand:
`claude mcp list`, `/mcp`, then `control_hello`); tool description
accuracy beyond the hygiene checks.

## Decisions and deviations

- Stateless translation layer: no caching, no tool that is not a method.
  Anything an agent needs must exist as a control method first.
- Errors as tool results, not protocol errors (ba doc #266 §3), because
  clients render protocol errors opaquely and the model cannot recover.
- One shared connection, so `job.wait` is sliced (CTL-07).
- `resonance-mcp/README.md` is out of date: it lists `track_…plugins` (now
  `plugins_catalog`) and `render_stems` (not published), describes only
  the Linux socket path, says `job_status` results carry a `revision`
  (they do not), and points at ba doc #267, which no longer exists. The
  tool list there covers only the first epic's namespaces.
- The `control_hello` tool description says to map a tool to its method
  by "turning the first underscore into a dot". That is wrong for
  `midi_map_*`, `drum_kits_*` and `amp_models_*` (`midi_map_learn` is
  `midi_map.learn`, not `midi.map_learn`). The rule only works in the
  method → tool direction.

## Known gaps

- No MCP resources or prompts; tools only.
- Drum-kit pad/note mapping is not reachable through any tool:
  [drum-pad-map-mcp](../../intents/control-api/drum-pad-map-mcp.md).
