# Fades, crossfades and clip gain

> Verified against master @ `ef215b08` on 2026-10-15.
> Spec history: epic #18, design doc #153, arch doc #156 (todos #316–#320);
> prototype `docs/history/prototypes/clip-editing-fades-crossfades/index.html`.

Every audio clip has a fade-in, a fade-out (each with a length and a curve)
and a gain in dB. Where two audio clips on the same track overlap, the overlap
crossfades by itself. Every clip edge also gets a 2 ms anti-click ramp. You
set fades and gain by dragging handles on the clip, by typing in the
[clip inspector](clip-inspector.md), or through `clip.set_fade` /
`clip.set_gain`. MIDI clips have none of this.

## Behaviour

### Fades

- Length in frames (`fade_in_frames`, `fade_out_frames`), `0` = no fade.
  A fade is clamped to the clip's audible length.
- Curves (`FadeCurve`): `Linear` (gain = t), `EqualPower` (sin(t·π/2), the
  default) and `Exp` (t²). A fade-out runs the curve backwards.
- On the canvas, the fade beads sit on the clip's top edge. Dragging one sets
  the ramp end to the pointer x (`handle x = ramp end`); fade drags do **not**
  snap. The engine gets one `SetClipFade` on release; one undo step per drag.
- Inspector and API edits are discrete and send the full `SetClipFade` (both
  lengths and both curves) from the mirror.

### Clip gain

- `gain_db`, `0.0` = unity. Applied as one linear factor on every frame.
- The engine clamps to −60…+24 dB (`MIN_CLIP_GAIN_DB`, `MAX_CLIP_GAIN_DB`)
  and turns `NaN` into 0 dB. The app mirror uses the same clamp
  (`clamp_gain_db`).
- On the canvas, the gain bead sits at the clip's top centre. Dragging up
  raises the gain at 0.15 dB per pixel (`GAIN_DB_PER_PX`), measured from the
  grab point so it does not drift. The clip body is tinted by gain and a dB
  tag is drawn in the header once gain is not unity.
- The inspector's field and steppers share the drag's and the API's
  −60…+24 dB range (see [clip-inspector.md](clip-inspector.md)).

### Crossfades

- Not stored. Wherever two audio clips on the same track overlap, the earlier
  clip's tail and the later clip's head each fade over the overlap length.
  If a clip's own fade is longer than the overlap, the longer one wins. The
  overlap fade is capped at the clip's audible length.
- Each clip uses **its own** fade curve over that length. With the default
  equal-power curves the two sides sum to constant power.
- Take clips under comp control are left out of this: they never crossfade
  against ordinary clips (the comp pass renders them, see
  [takes-and-comping.md](takes-and-comping.md)).
- The canvas draws the overlap with two crossing equal-power curves and a `⤬`
  badge.

### Declick

Every clip edge that no fade or crossfade covers ramps over
`CLIP_DECLICK_FRAMES` (96 frames, 2 ms at 48 kHz), capped at half the clip
length so a very short clip's two ramps meet in the middle. This is applied
whether or not the user set a fade; a longer fade or crossfade replaces it.

### Frozen and source-less clips

The canvas draws a frozen track's clips with a diagonal hatch and no fade
beads or fade wedges; gain still applies. The inspector hides the fade
controls for frozen tracks and for clips with `total_frames == 0`.

### Split

A split resets the fade at the cut: the head's fade-out and the tail's
fade-in become 0; the head keeps its fade-in, the tail its fade-out, and both
keep the gain ([clip-editing.md](clip-editing.md)).

### Render

One function, `mix_track_clips_governed`, applies
`fade_in × fade_out × gain` per frame. Live playback and offline bounce/export
both reach it through `render_block`, so they render identically. It does not
allocate.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Model | `resonance-audio/src/types/clip.rs` | `AudioClip::{fade_in_frames, fade_in_curve, fade_out_frames, fade_out_curve, gain_db}`, `FadeCurve::coefficient` |
| Engine handlers | `resonance-audio/src/engine/clips.rs` | `set_clip_fade_in_place`, `set_clip_gain_in_place`, `MIN_CLIP_GAIN_DB`, `MAX_CLIP_GAIN_DB` |
| Render | `resonance-audio/src/mixer/render/clips.rs` | `mix_track_clips_governed`, `clip_crossfade_lengths`, `clip_fade_gain_coef`, `CLIP_DECLICK_FRAMES`, `declick_frames` |
| App mirror | `resonance-app/src/state/clips.rs` | `ClipState` fade/gain fields, `FadeDragState`, `GainDragState` |
| Reducers | `resonance-app/src/update/clips.rs` | `start/update/end_clip_fade_drag`, `*_clip_gain_drag`, `set_clip_fade_in_ms`, `set_clip_fade_out_ms`, `set_clip_gain_db`, `set_clip_fade_curve`, `reset_clip_fade_gain`, `send_clip_fade`, `GAIN_DB_PER_PX` |
| Persistence | `resonance-app/src/project/model.rs` | `ProjectClip::{fade_in_frames, fade_in_curve, fade_out_frames, fade_out_curve, gain_db}` (curves saved as lowercase tags) |
| Canvas draw | `resonance-app/src/view/timeline/draw/clip.rs` | `draw_clip_fades`, `draw_clip_handles`, `draw_crossfades`, `gain_tinted_body` |
| Hit-test | `resonance-app/src/view/timeline/hit_test.rs`, `view/timeline/input/hit_test.rs` | `audio_clip_handles`, `hit_test_audio`, `FADE_HANDLE_RADIUS`, `GAIN_HANDLE_RADIUS`, `clip_handles` |
| Control API | `resonance-control/src/methods/clip.rs`, `resonance-app/src/update/control/clip.rs` | `SetFadeParams`, `FadeResult`, `FadeShape`, `SetGainParams`, `MAX_GAIN_DB`; `set_fade`, `set_gain` |

Invariants:

- The render path is on the audio thread: no allocation, `O(1)` per frame,
  the overlap scan once per clip per block.
- The engine echoes `ClipFadeChanged` / `ClipGainChanged`; the mirror is
  written first and the echo confirms it.
- `clip.set_fade` sends up to four messages inside `with_compound_undo`, so
  one call is one revision bump and one undo step.

## Surfaces

- **GUI:** Arrange tab. Fade beads on the clip's top corners (shown when a
  fade is set or the clip is selected), gain bead at top centre (shown when
  gain is not unity or the clip is selected). Numeric control in the
  [clip inspector](clip-inspector.md).
- **Control API / MCP:** `clip.set_fade` → `mcp__resonance__clip_set_fade`
  (`fade_in`, `fade_out` as `AmountSpec`; `fade_in_shape`, `fade_out_shape`
  as `linear` / `equal_power` / `exp`; a call that sets nothing is
  rejected; result reports the clamped lengths). `clip.set_gain` →
  `mcp__resonance__clip_set_gain` (finite `gain_db`, clamped to +24 dB in the
  handler and −60 dB in the engine). Audio clips only.
- **Agent skills:** none in `resonance-agent-plugin/skills/*`.

## How it's tested

| What | Where | Run |
|---|---|---|
| Render: ramps, gain, crossfade seam, declick, short-clip cap | `resonance-audio/tests/mixer/clip_fade_gain_render.rs` | `./scripts/run-tests.py -p resonance-audio` |
| Curve math | `resonance-audio/tests/types/fade_curve.rs` | same |
| Engine handlers (clamps, echoes) | `resonance-audio/tests/engine/clip_fade_gain_handlers.rs` | same |
| App reducers (drags, inspector edits, clamps) | `resonance-app/tests/timeline/clip_fade_gain_handlers.rs` | `./scripts/run-tests.py -p resonance-app` |
| Hit-testing of beads | `resonance-app/tests/timeline/clip_fade_gain_hit_test.rs` | same |
| Draw helpers | `resonance-app/tests/timeline/clip_fade_gain_draw.rs` | same |
| Mirror from echoes | `resonance-app/tests/timeline/clip_fade_gain_mirror.rs` | same |
| Save / load | `resonance-app/tests/timeline/clip_fade_gain_persistence.rs` | same |
| `clip.set_fade` / `clip.set_gain` | `resonance-app/tests/control/control_clip_place.rs` (`sets_fade_lengths_and_shapes`, `set_fade_..._one_atomic_undo_transaction`, `a_fade_is_clamped_to_the_clip_length`, `clip_gain_is_clamped_and_rejects_nonsense`, …) | same |
| Golden: fades, beads, gain tint, crossfade, frozen hatch | `resonance-app/tests/timeline/clip_fade_gain_snapshot.rs` → `tests/snapshots/clip_fades_gain_crossfade_render-wgpu.png` | `RESONANCE_BLESS=1 cargo test -p resonance-app --test timeline` |

Not covered: bounce output with fades compared to live playback beyond the
shared-function argument.

## Decisions and deviations

- Crossfades are derived from overlap, never stored (design doc #153). There
  is no per-crossfade curve or length; to shape one, set the clips' fades.
- The canvas always draws the crossfade as equal-power curves, but the audio
  uses each clip's own fade curve. They match only while both clips keep the
  default curve.
- The anti-click ramp was not in the original design; it was added later
  (fef17197, "Declick clip edges so a trimmed splice cannot click"). It
  changes the first and last 2 ms of every clip edge with no fade.

## Known gaps

- `song.tracks` does not report a clip's fades or gain.
  → [clip-state-readback.md](../../intents/timeline/clip-state-readback.md)
- No fade or gain on MIDI clips.
