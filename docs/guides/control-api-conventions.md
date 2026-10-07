# Control API conventions

Always-true rules for adding or changing a control method. The
background is in [docs/features/control-api/](../features/control-api/).

## One method, one tool, one change

A control method and its MCP tool land together, as one vertical slice:

1. **Wire** — `resonance-control/src/methods/<namespace>.rs`: a
   `pub const NAME: &str = "<namespace>.<name>";` with a doc comment
   (`/// \`ns.name\` — what it does ([\`Params\`] -> [\`Result\`]).`), the
   constant added to that file's `METHODS`, and the param/result types.
   A new namespace also goes into `methods::capabilities()`.
2. **Handler** — `resonance-app/src/update/control/<namespace>.rs`,
   reached from `execute` in `update/control/mod.rs`.
3. **Tool** — `resonance-mcp/src/tools/<module>.rs`, named the method with
   its first `.` replaced by `_`. A new module is added to
   `ResonanceMcp::combined_router`.
4. **Tests** — an app-side test module under
   `resonance-app/tests/control/`, registered in `tests/control.rs` (never
   a new top-level test file). The MCP side is held by
   `combined_router_exposes_every_control_method`.

Do not land a method without its tool, or a tool without its method. Do
not add a tool that is not a method.

## Wire types

- Every `*Params` type, and every request-only struct nested in one, is
  `#[serde(deny_unknown_fields)]` and derives
  `#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]`.
  `resonance-control/tests/unknown_fields.rs` enforces the first.
- Never `#[serde(flatten)]` an enum or a map into params.
- Optional fields are `#[serde(default, skip_serializing_if = …)]`.
- Result and view types stay tolerant of unknown fields.
- Ids use the newtypes in `ids.rs`. Timeline positions take
  `PositionSpec` and report `SongPosition`; `beat` is the time-signature
  beat. Relative `*_beat(s)` fields are 0-based quarter notes. Bar counts
  go through `check_max_bars`.
- A mutating result carries `revision`: return `MutationAck`, or embed a
  `revision: u64` in a richer result.
- A result that is "any JSON" needs an explicit object schema; schemars'
  boolean `true` is rejected by strict MCP clients.

## Versioning

- Adding a method or an optional field: no version bump.
- Removing or renaming a method or field, or changing what a field
  means: bump `PROTOCOL_VERSION` (or add a capability clients check), and
  re-pin `resonance-agent-plugin/lockstep.json`.
- Renaming: keep the old name answering for a release, listed in
  `methods::deprecated_aliases()` with no tool. Dropping it after that
  window needs no bump.
- `capabilities` lists only what the build implements. A method that
  would always answer `unsupported` stays out of `METHODS` until it works.

## Handlers

- Mutate only by building a domain `Message` and calling
  `run_via_update`. Never write app or engine state directly.
- Reply with the helpers in `update/control/reply.rs` (`success`,
  `failure`, `ack`, `ack_task`, `ack_or_compose_error`, `reject`,
  `no_track`, …). `MutationAck` is built only in `reply::mutation_ack`.
- Project edits sit below the mutation gate. A method that does not
  touch the open project (a per-user library, the plugin catalog) goes in
  `bypasses_mutation_gate`. A gated method that only reads goes in
  `reads_open_project_only` so it stays answerable during a GUI drag.
- One call must be one undo entry; `execute` already wraps gated calls in
  `with_compound_undo`. Do not open a second history entry yourself.
- Destructive calls take `confirm: bool` (`overwrite: bool` for files) and
  answer `needs_confirmation` with a summary of what would be lost.
- Validate and refuse; do not clamp. Use the shared validators
  (`validate_bpm`, `validate_time_signature`).
- Wire projections of app state come from `update/control/view_model/`.
- Long operations return `JobStarted` from `start_control_job` with a
  `JobToken`, resolved from the real completion event, never guessed.
- `resonance_control` may be named only in the control layer
  (arch-invariants ARCH2-11).

## Tools

- Read-only tools return structured content with an `output_schema` and
  `read_only_hint = true`; destructive ones set `destructive_hint = true`.
- Job tools call `invoke_job` with a wait budget.
- The description is the agent's only documentation: name real plugin
  ids and real field names (checked by `doc_hygiene.rs`), and say what
  the call refuses.
- If an agent skill should use the method, name the tool in the skill
  and add the method to its preflight; `agent_plugin_lockstep.rs` checks
  both.

## Before merging

```sh
./scripts/run-tests.py -p resonance-control
./scripts/run-tests.py -p resonance-mcp
./scripts/run-tests.py -p resonance-app
cargo test -p arch-invariants
```
