# Engine error taxonomy

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: `docs/history/refactor/refactor-intent.md` (epic C, ARCH-05).

Every engine failure reaches the app with a category as well as a message,
so a control-API job can say *why* it failed (not found, busy, unsupported,
I/O, plugin) and the user still gets a readable banner.

## Behaviour

- `AudioEvent::Error(EngineError)` replaces the old `Error(String)`.
  `EngineError { kind, message }`; `kind` is an `EngineErrorKind`:
  `NotFound`, `Busy`, `Unsupported`, `Io`, `Plugin`, `Internal`.
  Constructors: `EngineError::not_found / busy / unsupported / io / plugin /
  internal`.
- The kinds mirror `resonance_control::ErrorKind` by hand. resonance-audio
  sits below resonance-control in the crate DAG and must not depend on it.
- Each module that can fail has its own `thiserror` type with a
  `From<…> for EngineError` that picks the kind. Examples:
  `EngineInitError` (start-up), `FreezeError` and `FrozenCacheError`
  (freeze), `PartialFileError` (renderer output), `ClipError` /
  `WavIoError` (clip files), `TranscodeError`, `InputStreamError`.
  `resonance_common::WavDecodeError` and `AudioProbeError` map to `Io`.
- Duplicate-id refusals use `Internal`; the bus ceiling (`MAX_BUSSES` = 32)
  uses `Busy`.
- Some paths keep their own typed events:
  - `ExportError { kind: ExportErrorKind, message }` and
    `BounceError { kind: ExportErrorKind, .. }` with `EncoderUnavailable`,
    `Io`, `Cancelled`, `NoAudio`, `TransportRunning`;
  - `TrackBounceError(EngineError)`, `StemExportError(EngineError)`;
  - `PluginLoadFailed { reason }`, `PluginEditorState { failure }` with
    `PluginEditorFailure` (which also maps to a kind via
    `engine_error_kind()`);
  - `FreezeError { message }`, `MixMeasureError { measure_id, message }`,
    `ChainProbeError { probe_id, message }` carry text only.
- App side: `engine_events::transport::error` logs the kind and shows
  `message` as a banner. For control jobs, `update/control/job.rs`
  `export_kind_to_rpc` maps `ExportErrorKind` onto `JobStatus.error.kind`
  (`EncoderUnavailable` → `unsupported`, `TransportRunning` → `busy`,
  the rest → `internal`). `JobError.kind` is optional and still parses a
  bare string from older peers (`resonance-control/src/job.rs`).
- The audio thread never formats or logs: callback-side problems are counted
  in atomics and reported by the engine loop (see
  [rt-safety.md](rt-safety.md)).

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Error type | `resonance-audio/src/types/error.rs` | `EngineError`, `EngineErrorKind` |
| Export/bounce kinds | `resonance-audio/src/types/events.rs` | `ExportErrorKind`, `PluginEditorFailure` |
| Per-module errors | e.g. `engine/audio_engine.rs`, `engine/bounce/freeze.rs`, `engine/bounce/mod.rs`, `io/wav.rs`, `types/clip.rs`, `engine/clips.rs` | `EngineInitError`, `FreezeError`, `PartialFileError`, `WavIoError`, `ClipError`, `TranscodeError` |
| Job error wire | `resonance-control/src/job.rs` | `JobError { message, kind }` |
| App mapping | `resonance-app/src/update/control/job.rs`, `resonance-app/src/engine_events/transport.rs` | `export_kind_to_rpc`, `error` |
| Invariant | `tools/arch-invariants/tests/architecture.rs` | `engine_common_public_fns_dont_return_result_string` (empty allow-lists for audio and common) |

Invariant: no `pub fn` in resonance-audio or resonance-common may return
`Result<_, String>`. The arch-invariants test fails the suite on one.

## Surfaces

- **GUI:** the error banner (`r.banners.error_message`).
- **Control API / MCP:** `job.status` / `job.wait` (`mcp__resonance__job_status`,
  `mcp__resonance__job_wait`) report `error.kind` for failed render jobs.
- **Agent skills:** none directly.

## How it's tested

| What | Where | Run |
|---|---|---|
| Kinds on real handlers | `resonance-audio/tests/engine/engine_error_kind.rs` | `./scripts/run-tests.py -p resonance-audio` |
| Freeze terminal mapping | `resonance-audio/tests/bounce/freeze_render_core.rs` (`freeze_terminal_event`) | same |
| Export error kinds | `resonance-audio/tests/bounce/export_encoders.rs`, `export_normalize.rs` | same |
| Job error wire | `resonance-app/tests/control/control_jobs.rs` | `./scripts/run-tests.py -p resonance-app` |
| No `Result<_, String>` | `tools/arch-invariants/tests/architecture.rs` | `cargo test -p arch-invariants` |

Not covered: a test that every `AudioEvent::Error` site picks a sensible
kind (only a sample is asserted).

## Decisions and deviations

- Epic C's "done when" listed `BounceError`, `TrackBounceError` and
  `StemExportError`; those are typed. `FreezeError`, `MixMeasureError` and
  `ChainProbeError` were left as text. `FreezeError` has a typed source
  (`engine::bounce::FreezeError`) that is flattened to its message in
  `freeze_terminal_event`.
- `TrackBounceError.kind` has no consumer: bounce in place has no control
  surface, so the app only logs it.

## Known gaps

- Measurement and probe failures carry no kind, so `meter.*` jobs cannot
  report one. No intent; small follow-up.
