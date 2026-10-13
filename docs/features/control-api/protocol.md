# Control protocol and conventions

> Verified against master @ `cb3b1d6a` + voicebank registry (vocal slices 4–5) on 2026-10-07.
> Spec history: ba doc #265 (not in the repo). Code-review findings cited
> below (CTL-01, CTL-03, ARCH2-06, …) are in
> `docs/history/reviews/code-review-todo.md`.

The wire contract every control client speaks: what a message looks like,
how versions and capabilities are checked, how errors, ids, positions and
destructive calls work, and the rules for adding a method. The
`resonance-control` crate is the single source of truth; this page
describes it.

## Behaviour

### Envelope and framing

- JSON-RPC 2.0. A request is
  `{"jsonrpc":"2.0","id":1,"method":"song.summary","params":{...}}`;
  `jsonrpc` may be omitted. Every request has an id (number or string)
  and gets exactly one response with the same id. No batches, no
  notifications.
- One UTF-8 JSON message per `\n`-terminated line over a unix stream
  socket. Blank lines are skipped. A line longer than 16 MiB
  (`MAX_FRAME_LEN`) poisons the connection and it is dropped.
- Absent `params` is read as `null`, then as `{}`, so a method whose
  params are all optional can be called without them.

### Errors

`error` is `{code, message, data: {kind, detail}}`. Clients branch on
`data.kind`, never on `code` or `message`:

| `kind` | Code | Meaning |
|---|---|---|
| `not_found` | 1000 | A referenced id (track, clip, section, chord, job …) does not exist. |
| `invalid_params` | -32602 (parse errors -32700) | Missing, wrong type, out of range, or an unknown field. |
| `needs_confirmation` | 1001 | A destructive call needs `confirm: true` (`overwrite: true` for a file); `detail` says what would be lost. |
| `busy` | 1002 | The app cannot take this now (no project, load in progress, render running, GUI drag open). Retry. |
| `unsupported` | 1003 (unknown method -32601) | Unknown method, a method not built yet, or an incompatible connection. |
| `voicebank_missing` | 1004 | A vocal render names a voicebank, or the vocoder it needs, that is not installed; the message names it and where to get it. Also a job's `error.kind`. Added without a protocol bump: an older client reads it as `unknown`. |
| `internal` | -32603 | Unexpected failure. |

`unknown` exists only so an older client can parse a newer server's
kinds; it is never sent.

### Versioning and capabilities

- `PROTOCOL_VERSION` is `2` (bumped from 1 for the vocal params wire: snake_case vocal enums, per-lane `song.vocal` lines and params, voicebank ids). `control.hello {protocol_version}` returns
  `{app_version, protocol_version, capabilities}`. The app compares
  versions with `!=`: a mismatch marks the connection incompatible.
- `capabilities` is `methods::capabilities()`: every method this build
  answers, in namespace order (187 today). It is a contract. A name that
  would always answer `unsupported` is kept out of it (as `render.stems`
  was until it was built).
- What bumps the version: removing or renaming a method or field, or
  changing what an existing field means (CTL-01 changed what `beat`
  counts in 6/8; that kind of change needs a bump or a capability the
  client checks). Adding methods or optional fields does not.
- Deprecation window: a renamed method can keep its old spelling for a
  release, listed in `methods::deprecated_aliases()` (in `capabilities`,
  but with no MCP tool). Removing it at the end of the window does not
  bump the version. A method that was never deprecated still bumps. The
  list is empty today; `track.plugins` → `plugins.catalog` was its only
  entry (removed in todo #1240).
- A bump refuses every client not rebuilt, including the installed
  `resonance-mcp`, and forces a re-pin of the agent plugin's
  `lockstep.json` ([agent-plugin.md](agent-plugin.md)).

### Strict requests, tolerant results

Every `*Params` type and every request-only spec nested in one
(`PositionSpec`, `AutomationTargetSpec`, `PresetFilter`, …) is
`#[serde(deny_unknown_fields)]`, so `beats` for `beat` is
`invalid_params` naming the field, not a silent no-op (ARCH2-06). This
works through `#[serde(flatten)]` only because every flattened part is a
plain struct; never flatten an enum or a map into params. Result and view
types accept unknown fields so an older client can read a newer app.

### Ids, positions and units

- Ids (`TrackId`, `ClipId`, `SectionDefinitionId`, `JobId`, …) are the
  app's real `u64` ids as plain JSON numbers (`ids.rs`). They come from
  the `song.*` views (or the result of the call that created the entity)
  and stay valid until the entity is deleted.
- Timeline positions are `PositionSpec`: `{bar, beat}` or `{sample}`.
  Bars and beats are 1-based, and `beat` counts the bar's time-signature
  beat (an eighth in 6/8, a half in 2/2), across signature changes. A
  reported `SongPosition` can be sent straight back. A beat past the
  bar's last one is refused.
- Clip- and section-relative `*_beat` / `*_beats` fields are 0-based
  quarter notes whatever the meter.
- Bar counts are capped at `MAX_BARS` = 100 000 (`check_max_bars`,
  CTL-05). Pitch 60 is C4.

### Mutations, revision and undo

- Every mutating result carries `revision` (a bare `MutationAck`, or a
  richer result with its own `revision` field). It is bumped once per
  call, however many internal edits the call fans out into.
- One call is one undo entry: `edit.undo` takes back a whole call. The
  undo stack is shared with the user's GUI edits, so a client should read
  `edit.status` before undoing.
- Per-user library state (amp models, drum kits, plugin presets) is not
  project state: no undo entry, no revision bump, no open project needed.

### Destructive calls

Deletes, overwriting files, and `project.new` / `project.open` over
unsaved changes refuse with `needs_confirmation` and a summary until the
call carries `confirm: true` (`overwrite: true` for render targets).

### Long-running calls

`project.*`, `vocal.render`, `render.mixdown`, `meter.*`,
`master.assist`, `pool.import`, `clip.place` and `clip.detect_tempo`
return `{job_id}`; the client polls `job.status` or blocks on `job.wait`.
See [jobs.md](jobs.md).

## The vertical-slice rule

One control method is one MCP tool, and they land together. A new
method is not done until the same change has:

1. the wire type and method constant in `resonance-control` (added to its
   namespace's `METHODS`),
2. the app handler in `resonance-app/src/update/control/`,
3. the MCP tool in `resonance-mcp/src/tools/`,
4. tests: the app-side test in `resonance-app/tests/control/`, and the
   MCP side kept green by `combined_router_exposes_every_control_method`.

The step-by-step rules are in
[docs/guides/control-api-conventions.md](../../guides/control-api-conventions.md).

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Version + crate rules | `resonance-control/src/lib.rs` | `PROTOCOL_VERSION`, module doc (evolution rules) |
| Envelope + errors | `resonance-control/src/rpc.rs` | `Request::params`, `Response`, `RpcError`, `ErrorKind`, `codes` |
| Framing | `resonance-control/src/framing.rs` | `write_message`, `MessageReader`, `MAX_FRAME_LEN`, `FramingError::Oversized` |
| Handshake | `resonance-control/src/methods/control.rs` | `HELLO`, `HelloParams`, `HelloResult` |
| Capabilities | `resonance-control/src/methods/mod.rs` | `capabilities`, `deprecated_aliases` |
| Shared primitives | `resonance-control/src/common.rs`, `ids.rs` | `MutationAck`, `PositionSpec`, `SongPosition`, `MAX_BARS`, id newtypes |
| Server-side hello | `resonance-app/src/update/control/mod.rs`, `gate.rs` | `hello`, `is_protocol_method` (in `gate.rs`) |

## Surfaces

- **Control API / MCP:** `control.hello` → `mcp__resonance__control_hello`.
- **Agent skills:** each skill checks the methods it needs against
  `capabilities` in its preflight.

## How it's tested

| What | Where | Run |
|---|---|---|
| Envelope, error kinds, unknown-field tolerance on results | `resonance-control/tests/envelope.rs`, `views.rs` | `./scripts/run-tests.py -p resonance-control` |
| Framing, oversize cap | `resonance-control/tests/framing.rs` | same |
| Every `*Params` denies unknown fields | `resonance-control/tests/unknown_fields.rs` (scans the sources) | same |
| Capability list, confirm flags, round-trips | `resonance-control/tests/methods.rs` | same |
| Socket-path rule | `resonance-control/tests/socket_path.rs` | same |
| Hello + incompatible connection | `resonance-app/tests/control/control_endpoint.rs` | `./scripts/run-tests.py -p resonance-app` |
| Position beat unit in and out | `resonance-app/tests/control/control_position_beats.rs` | same |
| One method = one tool | `resonance-mcp/tests/translation.rs::combined_router_exposes_every_control_method` | `./scripts/run-tests.py -p resonance-mcp` |

Not covered: nothing checks that a change of a field's meaning comes with
a version bump; that is a review rule.

## Decisions and deviations

- `!=` rather than `>=` on the version: a client either matches or is
  refused, so a mismatch is loud. This is why bumps are expensive and the
  deprecation window exists.
- `capabilities` lists implemented methods only (ba doc #275 P4).
  Before that, `render.stems` was listed and always failed, which made
  the list useless for deciding anything.
- `JobStatus.error` changed from a string to `{message, kind}` (epic C,
  C-2) without a bump; the deserializer still accepts the old bare
  string.

## Known gaps

