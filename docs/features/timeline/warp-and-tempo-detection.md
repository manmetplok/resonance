# Clip warp and tempo detection

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: doc #166 (warp + tempo detection, engine side, 2026-06-22);
> GUI and control API from the 2026-10-02 review campaign, batch W2
> (FU-A1a1, de2625d3) — `docs/history/reviews/code-review-2026-10-02.md`
> §ARCH2-01 and follow-ups FU-W2a…e.

Warp ("follow tempo") is meant to time-stretch an audio clip so it plays at
the project tempo, optionally with warp markers that pin a sound in the file
to a beat. **Today warp is data only: it is stored, drawn, saved, undoable and
scriptable, but playback and bounce ignore it — a warped clip plays its
source at its own speed.** Tempo detection does work: it estimates a clip's
BPM so it can be entered as the clip's source tempo.

## Behaviour

### Warp settings (per audio clip)

| Field | Range / default | Meaning |
|---|---|---|
| `enabled` | off | Warp on or off. |
| `original_bpm` | unknown; 20–999 BPM | The tempo the source was played at. Unknown means no stretch. |
| `transpose_semitones` | 0; ±48 | Pitch shift on the warp path. |
| `algorithm` | `Transient` | `Transient` (WSOLA, keeps attacks) or `Tonal` (phase vocoder, for sustained material). |
| `markers` | none; at most 1024 | Each pins `source_frame` (a frame of the source file, independent of trim) to `timeline_beat` (beats from the clip start, ≥ 0). Kept sorted by beat; at least 1/16 beat apart; `source_frame` never decreases. |

Marker beats are projected onto the timeline at the project's **base tempo**
(`transport.bpm`), not through the tempo map.

### Editing in the GUI

- **Inspector WARP section** ([clip-inspector.md](clip-inspector.md)):
  Off / Warp on; the tempo field (a draft, committed on Enter; empty clears
  the tempo, junk is dropped); Detect; Transient / Tonal; transpose ±1; the
  marker count and Clear. A caption reads "Saved with the clip — playback
  does not stretch yet."
- **Timeline.** A warped clip gets a `WARP` badge and a marker strip along
  its bottom edge (`theme::WARP_MARKER_STRIP_HEIGHT`) with a hairline per
  marker.
  - Drag a marker: it slides along the grid (snapped), keeps its source
    frame, and is clamped 1/16 beat inside its neighbours and the clip, so the
    order never changes. One undo step; the engine gets the set on release.
  - Double-click the bare strip: adds a marker on the grid that pins the
    source frame playing there now, so adding it changes nothing until it is
    dragged.
  - Double-click or right-click a marker: removes it.
  - A single press on the bare strip falls through to the clip body (move).

### Tempo detection

- Detect (inspector) or `clip.detect_tempo` sends `DetectClipTempo`. The
  engine downmixes the **whole source** (not just the trimmed part) to mono
  and runs `resonance_dsp::tempo::detect_tempo_default`: spectral-flux onset
  envelope, autocorrelation, comb scoring of harmonics, searched in
  60–200 BPM.
- The reply `ClipTempoDetected { bpm, confidence }` changes nothing on the
  clip. The inspector shows "Found N BPM · X% sure" with a Use button (which
  sets `original_bpm`), or "No steady tempo found" when the detector returns
  0.
- `clip.detect_tempo` is a job; it resolves with `{clip_id, bpm, confidence}`
  or fails when no tempo is found or the clip is gone.
- Detection runs on the engine control thread, so a long clip blocks other
  engine commands while it runs.

### Split, save, undo

- Splitting keeps the warp settings on both halves; the tail loses its
  markers.
- The project file stores warp as `ProjectClip::warp` (omitted when
  default; the algorithm saved as a lowercase tag). It rides every undo
  snapshot.
- Discrete edits are one undo entry each; typing in the tempo field and
  running Detect record nothing.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Engine model | `resonance-audio/src/types/clip.rs` | `AudioClip::{warp_enabled, original_bpm, transpose_semitones, warp_algorithm, warp_markers}`, `WarpAlgorithm`, `WarpMarker`, `warp_source_frame` (unused outside tests) |
| Engine handlers | `resonance-audio/src/engine/clips.rs` | `set_clip_warp_in_place`, `set_clip_warp_markers_in_place`, `detect_clip_tempo_in_place` |
| Stretcher (not wired) | `resonance-dsp/src/timestretch/mod.rs` | `TimeStretch`, `StretchAlgorithm` |
| Tempo detector | `resonance-dsp/src/tempo.rs` | `detect_tempo`, `detect_tempo_default`, `TempoConfig`, `fold_bpm` |
| App mirror | `resonance-app/src/state/clip_warp.rs` | `ClipWarpState`, `source_frame_at_beat`, `markers_with_added`, `markers_without`, `MIN/MAX_WARP_BPM`, `MAX_TRANSPOSE_SEMITONES`, `MAX_WARP_MARKERS`, `MIN_WARP_MARKER_GAP_BEATS`, `TempoDetectStatus`, `WarpBpmDraft` |
| Reducers | `resonance-app/src/update/clip_warp.rs` | `ClipWarpMessage`, `set_warp`, `set_warp_markers`, `commit_bpm_draft`, `detect_tempo`, marker drag fns |
| Echoes | `resonance-app/src/engine_events/clips.rs` | `warp_changed`, `warp_markers_changed`, `tempo_detected` |
| Persistence | `resonance-app/src/project/model.rs`, `update/project_io/serialize.rs` | `ProjectClipWarp`, `ProjectWarpMarker` |
| Inspector | `resonance-app/src/view/clip_inspector/warp.rs` | `warp_section`, `detect_status_row` |
| Canvas | `resonance-app/src/view/timeline/draw/warp.rs`, `input/pointer.rs`, `input/hover.rs` | `WarpHit`, `warp_hit_in_rect`, `press_warp_strip`, `handle_right_press` |
| Control API | `resonance-control/src/methods/clip.rs`, `resonance-app/src/update/control/clip/warp.rs` | `SetWarpParams`, `SetWarpMarkersParams`, `WarpMarkerSpec`, `WarpResult`, `DetectTempoParams`, `DetectTempoResult`; `set_warp`, `set_warp_markers`, `detect_tempo`, `tempo_detected` |

## Surfaces

- **GUI:** clip inspector WARP section; marker strip on warped clips in the
  Arrange tab.
- **Control API / MCP:**
  - `clip.set_warp` → `mcp__resonance__clip_set_warp` — any of `enabled`,
    `original_bpm` (20–999), `clear_original_bpm`, `transpose_semitones`
    (±48), `algorithm` (`transient` / `tonal`); omitted fields keep their
    value; a call that sets nothing is rejected.
  - `clip.set_warp_markers` → `mcp__resonance__clip_set_warp_markers` —
    the full marker set as `{source_frame, beat}`; validated (beat ≥ 0,
    `source_frame` within the source, 1/16-beat spacing, non-decreasing
    source) and sorted.
  - `clip.detect_tempo` → `mcp__resonance__clip_detect_tempo` (job).
  - The MCP tool descriptions say playback does not stretch yet.
- **Agent skills:** none in `resonance-agent-plugin/skills/*`.

## How it's tested

| What | Where | Run |
|---|---|---|
| Warp mapping math (`warp_source_frame`) | `resonance-audio/tests/types/clip_warp.rs` | `./scripts/run-tests.py -p resonance-audio` |
| Engine warp handlers | `resonance-audio/tests/engine/clip_warp_handlers.rs` | same |
| Tempo detector, time-stretcher | `resonance-dsp/tests/tempo.rs`, `resonance-dsp/tests/timestretch.rs` | `./scripts/run-tests.py -p resonance-dsp` |
| GUI: inspector, Detect round trip, marker drag/add/remove, undo, split | `resonance-app/tests/timeline/clip_warp_ui.rs` | `./scripts/run-tests.py -p resonance-app` |
| `clip.set_warp`, `clip.set_warp_markers`, `clip.detect_tempo` | `resonance-app/tests/control/control_clip_warp.rs` | same |
| Goldens | `resonance-app/tests/timeline/clip_warp_snapshot.rs` → `tests/snapshots/clip_warp_timeline_render-wgpu.png`, `clip_warp_inspector_render-wgpu.png` | `RESONANCE_BLESS=1 cargo test -p resonance-app --test timeline` |

Not covered: anything audible — there is no render path to test.

## Decisions and deviations

- The engine side (doc #166) shipped in June 2026 with no caller; the GUI and
  API were added in the October review campaign so the data would be
  reachable, before the render path existed.
- Marker beats use the project's base tempo rather than the tempo map, in the
  app mirror and in the engine's `warp_source_frame` alike.
- The app and the engine disagree on where an unmarked warp starts reading:
  `ClipWarpState::source_frame_at_beat` starts at `trim_start_frames`, the
  engine's `warp_source_frame` at frame 0 relative to the clip. This only
  matters once a render path exists.

## Known gaps

- Warp is not rendered, live or in bounce; `TimeStretch` is called only from
  tests (review FU-W2a). → [warp-render-path.md](../../intents/audio-engine/clip-warp-render.md)
- Tempo detection runs on the engine control thread over the whole source
  (FU-W2b). → same intent.
- `song.tracks` does not report warp state (FU-W2c).
  → [clip-state-readback.md](../../intents/timeline/clip-state-readback.md)
- A marker within the hit width of a clip's bottom corner shadows the trim
  handle there (FU-W2e).
