# Clip inspector

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: epic #18, design doc #153, arch doc #156 (todo #319);
> the WARP section came with the warp UI (review batch W2, de2625d3).

A small card that floats over the top-right of the Arrange area whenever an
audio clip is selected. It gives typed values for what the clip's on-canvas
handles do — fade lengths and curves, gain — plus a reset button and the
clip's warp settings. It is a GUI surface only; the control API edits the same
fields directly.

## Behaviour

- **When it shows.** `ui.interaction.selected_clip` names an audio clip.
  Selecting a MIDI clip, or nothing, hides it. There is no close button and no
  toggle: deselect the clip to dismiss it. It is drawn in `view_main_area`, so
  it appears on the Arrange tab, stacked over the track headers and timeline.
- **Header.** A `CLIP` pill and the clip's name. The name is read-only.
- **FADE IN / FADE OUT.** A whole-millisecond field (digits only; empty means
  0) and a three-way curve picker: `Linear`, `Eq-pow`, `Exp`. Values are
  converted to frames at the project rate and clamped to the clip's audible
  length.
- **GAIN.** `−0.5` / `+0.5` steppers around a dB field (one decimal). Field
  and steppers are limited to −60…+12 dB (`GAIN_MIN_DB`, `GAIN_MAX_DB`); the
  steppers exist because a controlled text field cannot start typing a minus
  sign on its own.
- **Reset to default.** Fades to 0, both curves to equal-power, gain to 0 dB
  (`ResetClipFadeGain`); one undo step. Warp is not reset.
- **WARP.** Always rendered with the same rows so the tempo field keeps focus:
  Off / Warp on, source tempo field + Detect, detection status with a Use
  button, Transient / Tonal, transpose steppers (±1 semitone), marker count +
  Clear, and a caption that says playback does not stretch yet. Details in
  [warp-and-tempo-detection.md](warp-and-tempo-detection.md).
- **Degraded card.** If the clip's track is frozen, or the clip has no source
  (`total_frames == 0`), the fade rows, reset and warp section are replaced by
  a red "Fades unavailable" banner that says why; the gain row stays.
- **Undo.** Every fade/gain edit is its own `Record` entry. The ms and dB
  fields commit on every keystroke, so typing `250` makes three edits
  (2, 25, 250) and three undo steps. The warp tempo field is different: it
  keeps a draft and commits on Enter.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| View | `resonance-app/src/view/clip_inspector.rs` | `view_clip_inspector_flyout`, `editable_flyout`, `degraded_flyout`, `ms_field_row`, `gain_row`, `curve_picker`, `FLYOUT_WIDTH`, `GAIN_MIN_DB`, `GAIN_MAX_DB`, `GAIN_STEP_DB` |
| WARP section | `resonance-app/src/view/clip_inspector/warp.rs` | `warp_section`, `detect_status_row` |
| Placement | `resonance-app/src/view/mod.rs` | `view_main_area` stacks the card top-right |
| Messages / reducers | `resonance-app/src/update/clips.rs`, `update/clip_warp.rs` | `ClipMessage::{SetClipFadeInMs, SetClipFadeOutMs, SetClipGainDb, SetClipFadeInCurve, SetClipFadeOutCurve, ResetClipFadeGain}`, `ClipWarpMessage` |
| State read | `resonance-app/src/state/clips.rs`, `state/clip_warp.rs`, `state/freeze.rs` | `ClipState`, `ClipWarpState`, `TempoDetectStatus`, `WarpBpmDraft` |

The card only reads the `ClipState` mirror and emits the same messages the
canvas handles use, so the two cannot disagree about a value.

## Surfaces

- **GUI:** Arrange tab, top-right of the lanes, while an audio clip is
  selected.
- **Control API / MCP:** none for the card itself. The same edits are
  `clip.set_fade`, `clip.set_gain`, `clip.set_warp`, `clip.set_warp_markers`
  and `clip.detect_tempo` (see [fades-crossfades-clip-gain.md](fades-crossfades-clip-gain.md)
  and [warp-and-tempo-detection.md](warp-and-tempo-detection.md)).
- **Agent skills:** none.

## How it's tested

| What | Where | Run |
|---|---|---|
| Card present/absent, degraded banner (frozen, source-less), reset, gain stepper, curve picker | `resonance-app/tests/timeline/clip_inspector_flyout.rs` | `./scripts/run-tests.py -p resonance-app` |
| The edits it emits | `resonance-app/tests/timeline/clip_fade_gain_handlers.rs` (`inspector_*`) | same |
| WARP section messages | `resonance-app/tests/timeline/clip_warp_ui.rs` | same |
| Golden: WARP section with a detection result | `resonance-app/tests/timeline/clip_warp_snapshot.rs` → `tests/snapshots/clip_warp_inspector_render-wgpu.png` | `RESONANCE_BLESS=1 cargo test -p resonance-app --test timeline` |

Not covered: a golden of the fade/gain part of the card on its own, and
typing into the ms / dB fields.

## Decisions and deviations

- The card is a fixed overlay at the top-right of the arrange area, not
  anchored to the clip, so it can cover clips near that corner.
- The code comment says the ±12 dB gain limit "matches the on-canvas gain drag
  range"; it does not — the drag and the engine allow up to +24 dB.

## Known gaps

- Inspector gain stops at +12 dB while the drag and `clip.set_gain` reach
  +24 dB; a clip set above +12 dB elsewhere snaps down to +12 on the next
  stepper press.
- Each keystroke in the ms and dB fields is a separate undo step.
- No inspector for MIDI clips, and no clip rename.
