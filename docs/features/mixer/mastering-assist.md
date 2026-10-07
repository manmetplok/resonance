# Mastering assistant over the control API (`master.assist`)

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: `docs/history/specs/warmth-width-depth.md` §6.3, §7.4 (decision D6).

An agent can ask the app to measure the master mix and suggest settings for
the Resonance mastering plugin, stage by stage, each with its rationale and
the exact parameter writes. The app renders the master offline, runs the
`resonance-mastering-assist` library on it and answers with suggestions —
it **never applies** them. The library itself (analysis, genre and
reference targets, the decision rules, emitted keys) and the plugin's own
Assistant panel are documented in
[plugins-effects/mastering-assist.md](../plugins-effects/mastering-assist.md)
and [plugins-effects/mastering.md](../plugins-effects/mastering.md); this
doc covers only the app side.

## Behaviour

- **Modes.** `mode: "genre"` with `genre` ∈ `rock`, `indie`, `acoustic`,
  `jazz`, `pop`; or `mode: "reference"` with a `pool_asset_id` (the
  reference's integrated loudness becomes the target and its spectrum the
  comparison). Each mode takes exactly its own field.
- **Range.** Defaults to the whole song and is clamped like
  `meter.measure`. The tool advises picking the loudest representative
  section when the song has quiet parts.
- **Rendering and analysis.** The master is rendered offline and measured
  by the engine with the assistant detail set (LTAS on the assistant's
  grid, crest, correlation, true peak, integrated loudness); the handler
  reads that measurement into an `AnalysisResult` and calls
  `decide::build`. A silent master fails the job instead of suggesting.
- **Result** (`AssistResult`): the target (label, target LUFS), the
  plugin id `com.resonance.mastering`, `master_slot` (the index of that
  plugin on the master chain, or `null` if it is not inserted), the
  measured master figures, and the suggestions. The agent applies what it
  agrees with through `master.set_plugin_param`.
- **Job shape.** Like `meter.measure`: validated, registered as a
  `JobToken::Measure` under `master.assist`, and refused `busy` under the
  same offline-render guard (also while the transport rolls). In reference
  mode the app first asks the engine to measure the pooled asset as decoded
  audio (`AudioCommand::MeasureAudio`, read like a placed clip) and only
  then sends the master render, so the job never has two engine commands
  outstanding and a failed decode cannot release the guard while a render
  runs.
- Changes nothing: no project edit, no undo entry, no `revision`.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| App handler | `resonance-app/src/update/control/assist.rs` | `assist`, `resolve_target`, `measured`, `pooled_asset_file` |
| Pending reference state | `resonance-app/src/state/control.rs` | `AssistTarget`, `PendingAssist` |
| Wire types | `resonance-control/src/methods/master.rs` | `AssistParams`, `AssistMode`, `AssistGenre`, `AssistResult`, `AssistSuggestion`, `AssistBandDeviation`, `MASTERING_PLUGIN_ID` |
| Library | `resonance-mastering-assist/src/` | see [plugins-effects/mastering-assist.md](../plugins-effects/mastering-assist.md) |

The app links no plugin crate (ARCHITECTURE.md); the plugin owns the
param-key lookup behind `ParamSink`, and a plugin-side test pins every key
the library emits to a real parameter.

## Surfaces

- **GUI:** none in the app; the mastering plugin's own Assistant panel
  ([plugins-effects/mastering.md](../plugins-effects/mastering.md)).
- **Control API / MCP:** `master.assist` → `mcp__resonance__master_assist`
  (a job; poll with `job_status` / `job_wait`).
- **Agent skills:** `mastering`; `mixing/references/reading-meters.md`.

## How it's tested

| What | Where | Run |
|---|---|---|
| Control method: engine commands sent, payload replays to the panel's Apply writes, reference sequencing, failures, busy | `resonance-app/tests/control/control_master_assist.rs` | `./scripts/run-tests.py -p resonance-app` |
| Engine measurement of a buffer = the panel's `analyze::run` of it | same file, `the_engine_measurement_is_the_panels_analysis` | same |

Not covered: the parity test uses a synthetic buffer; nothing compares the
plugin's live capture against an offline render of the same real mix.

## Decisions and deviations

- The assistant was split out of the plugin into its own library so the
  app can run it without linking a plugin.
- `master.assist` never applies; the plugin panel does. An agent always
  goes through `master.set_plugin_param`.

## Known gaps

- None recorded beyond the library's (see
  [plugins-effects/mastering-assist.md](../plugins-effects/mastering-assist.md#known-gaps)).
