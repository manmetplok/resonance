# resonance-mcp

An [MCP](https://modelcontextprotocol.io) **stdio** server that lets an
AI client (Claude Code, Claude Desktop, or any MCP-capable tool) drive a
**running** resonance DAW: create a project, write harmony, generate and
edit parts, set lyrics, play, and export a WAV — with every edit flowing
through the app's normal undo path and appearing live in the GUI.

It is a thin translation layer: each MCP tool maps 1:1 onto a method of
the `resonance-control` unix-socket JSON-RPC protocol
([protocol doc](../docs/features/control-api/protocol.md)). The
running app hosts the control endpoint; this binary connects to it over
the socket and re-exposes the surface as typed MCP tools. Param and
result schemas are generated from the exact `resonance-control` wire
types, so the published tool schemas can never drift from the protocol.

## Install

```sh
cargo build --release -p resonance-mcp
# binary at target/release/resonance-mcp
```

## Register with Claude Code

```sh
# name = "resonance"; everything after -- is the server command line
claude mcp add --transport stdio resonance -- /absolute/path/to/target/release/resonance-mcp
```

The app resolves the control socket at
`$XDG_RUNTIME_DIR/resonance/control.sock` (fallback
`/tmp/resonance-<uid>/control.sock`); on macOS it uses the per-user temp
directory (`confstr(_CS_DARWIN_USER_TEMP_DIR)`, normally what `$TMPDIR`
names). If yours differs, pass it through:

```sh
claude mcp add --env RESONANCE_CONTROL_SOCKET=/run/user/1000/resonance/control.sock \
  --transport stdio resonance -- /absolute/path/to/target/release/resonance-mcp
```

Both sides insist the socket's directory is private: a real directory
(not a symlink) owned by you with mode `0700`. The app creates it that
way and refuses to serve from one that isn't; `resonance-mcp` refuses to
connect to one that isn't. Point `RESONANCE_CONTROL_SOCKET` into a
private directory, not straight into `$HOME` or a project folder.

Verify with `claude mcp list` (expect `✔ Connected`) and `/mcp` inside a
Claude Code session to list the tools.

## Tool surface

One tool per control method, snake_case and namespaced (Claude Code shows
them as `mcp__resonance__<tool>`). There are ~180, across the namespaces
`song`, `project`, `edit`, `transport`, `global`, `arrangement`, `section`,
`harmony`, `generate`, `notes`, `clip`, `pool`, `automation`, `track`,
`mixer`, `bus`, `master`, `meter`, `reference`, `render`, `vocal`,
`external`, `midi_map`, `plugins`, `presets`, `drum_kits`, `amp_models`,
`job` and `control`. The full list, with the method each tool calls and the
feature doc that describes it, is
[docs/features/control-api/method-catalogue.md](../docs/features/control-api/method-catalogue.md).

Call the read-only `song_*` views first: they never mutate. Every
*mutating* result carries a `revision` counter (job status does not).
`control_hello` returns the running app's `capabilities` — the
authoritative list of methods it implements.

All ids (`track_id`, `clip_id`, `section_id`, `placement_id`,
`chord_id`) come from the `song_*` views and feed straight back into the
mutation tools. Destructive tools (`*_delete`, overwriting files,
`project_new`/`project_open` with unsaved changes) refuse until you pass
`confirm: true` (`overwrite: true` for render targets). Long operations
(project I/O, `vocal_render`, `render_mixdown`, `meter_*` measurements) run as jobs: the tool waits a
bounded time and returns the final status; if it is still running, poll
`job_status` or block on `job_wait` with the returned `job_id`.

## Behaviour notes

- **The app must be running.** Tools return an actionable "resonance is
  not running: could not connect to the control socket at …" error (not a crash) until it is,
  and the server reconnects automatically on the next call after a
  dropped socket.
- **Protocol version is checked at the handshake** (`control.hello`); a
  version-incompatible app yields a clear "update the app and/or this
  binary" error.
- **stdout carries only MCP JSON-RPC.** All logging goes to stderr
  (`RUST_LOG` controls the level, default `info`); Claude Code captures
  it (`claude --debug`).

## Composing a song with Claude

The walkthrough is in the top-level [README](../README.md#mcp-server); the
craft knowledge (mixing, mastering, arranging, …) ships as skills in
[resonance-agent-plugin](../resonance-agent-plugin/README.md).

The same sequence is exercised headlessly, without the GUI, by the
acceptance test `resonance-app/tests/e2e_compose_via_control.rs` (epic
#200, todo #1160): it drives the control protocol through the whole flow
and asserts the real AudioEngine bounce writes a non-empty WAV. Run it
with `cargo test -p resonance-app --test e2e_compose_via_control`.
