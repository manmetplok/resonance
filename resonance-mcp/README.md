# resonance-mcp

An [MCP](https://modelcontextprotocol.io) **stdio** server that lets an
AI client (Claude Code, Claude Desktop, or any MCP-capable tool) drive a
**running** resonance DAW: create a project, write harmony, generate and
edit parts, set lyrics, play, and export a WAV — with every edit flowing
through the app's normal undo path and appearing live in the GUI.

It is a thin translation layer: each MCP tool maps 1:1 onto a method of
the `resonance-control` unix-socket JSON-RPC protocol (ba doc #265). The
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
`/tmp/resonance-<uid>/control.sock`). If yours differs, pass it through:

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
them as `mcp__resonance__<tool>`):

- **Read-only introspection** (call these first — they never mutate and
  every result carries a `revision` counter):
  `song_summary`, `song_sections`, `song_tracks`, `song_notes`,
  `song_vocal`, `job_status`.
- **Project**: `project_new`, `project_open`, `project_save`,
  `project_save_as` (explicit paths, run as jobs).
- **Transport**: `transport_play/stop/pause/seek/loop_set/loop_toggle`,
  `transport_set_tempo/set_time_signature/set_key`.
- **Tracks & mixer**: `track_add/rename/delete/add_instrument/add_effect/
  plugins`, `mixer_set_volume/set_pan/set_mute/set_solo`.
- **Structure & harmony**: `section_create/rename/resize/delete/place/
  remove_placement/set_scale`, `harmony_add_chord/edit_chord/delete_chord/
  apply_progression`.
- **Composition**: `generate_part`, `generate_drums`,
  `notes_insert/edit/delete/create_clip`.
- **Vocals**: `vocal_set_lyrics/set_line/set_pronunciation/
  clear_pronunciation/render`.
- **Render & jobs**: `render_mixdown`, `render_stems`, `job_status`,
  `job_wait`.

All ids (`track_id`, `clip_id`, `section_id`, `placement_id`,
`chord_id`) come from the `song_*` views and feed straight back into the
mutation tools. Destructive tools (`*_delete`, overwriting files,
`project_new`/`project_open` with unsaved changes) refuse until you pass
`confirm: true` (`overwrite: true` for render targets). Long operations
(project I/O, `vocal_render`, `render_*`) run as jobs: the tool waits a
bounded time and returns the final status; if it is still running, poll
`job_status` or block on `job_wait` with the returned `job_id`.

## Behaviour notes

- **The app must be running.** Tools return an actionable "resonance is
  not running — start the app and retry" error (not a crash) until it is,
  and the server reconnects automatically on the next call after a
  dropped socket.
- **Protocol version is checked at the handshake** (`control.hello`); a
  version-incompatible app yields a clear "update the app and/or this
  binary" error.
- **stdout carries only MCP JSON-RPC.** All logging goes to stderr
  (`RUST_LOG` controls the level, default `info`); Claude Code captures
  it (`claude --debug`).

## Composing a song with Claude

See ba doc #267 "Composing with Claude" for the full end-to-end
walkthrough — `claude mcp add` registration, the tool-by-tool compose
flow (project → harmony → generate → notes → vocals → play → export WAV →
save), and troubleshooting.

The same sequence is exercised headlessly, without the GUI, by the
acceptance test `resonance-app/tests/e2e_compose_via_control.rs` (epic
#200, todo #1160): it drives the control protocol through the whole flow
and asserts the real AudioEngine bounce writes a non-empty WAV. Run it
with `cargo test -p resonance-app --test e2e_compose_via_control`.
