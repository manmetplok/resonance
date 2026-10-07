# Track freeze (engine side)

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: prototype `docs/history/prototypes/track-freeze-bounce-in-place/`;
> "doc #187", "todo #571 / #573 / #577" and code-review findings ENG-07,
> ENG-08, ENG-13, FU-G3a cited in code.

Freezing renders a track's instrument and insert effects once to a cache
file, then plays the cache instead of running the plugins, to save CPU. The
track's fader, pan, mute/solo, routing and sends stay live. This page covers
the render, the cache format and playback substitution; the freeze buttons,
banner, staleness and batch freeze are in [track-freeze-ui.md](../timeline/track-freeze-ui.md).

## Behaviour

### Rendering the cache (`FreezeTrack { track_id, cache_path }`)

- Runs on a `freeze-render` worker with the offline-render rules (gate,
  transport must be stopped, offline mode, plugin reset, PDC trim): see
  [offline-render-and-export.md](offline-render-and-export.md).
- Renders the track and its sub-tracks only (`in_filter`), ignoring its live
  mute/solo state.
- Range `[0, latest clip end + 2 s tail]` across the whole project, starting
  at sample 0 so the cache is timeline-aligned and needs no stored offset.
- **`freeze_raw` capture:** the track renders at unity gain with no pan and
  is routed straight to master, so the cache is fader- and route-independent;
  sub-tracks keep their own constant fader and pan (the kit's internal
  balance is baked in) and are also forced to master. Master FX and volume
  are excluded.
- Automation: plugin-parameter lanes of the frozen tracks' plugins are baked
  in (`baked_automation`); gain, pan and mute lanes stay live on playback.
- Output: 32-bit float stereo WAV, written to `<path>.partial` and renamed
  when complete, so a failed or cancelled refreeze keeps the old cache.
- Events: `FreezeProgress { fraction }` (at most once per percent), then
  exactly one of `FreezeCompleted { cache_ref }`, `FreezeCancelled` (after
  `CancelFreeze`, partial file removed) or `FreezeError { message }`.
- `FreezeCacheRef` records the file name, rate, bit depth, a render
  fingerprint (the frozen tracks' notes, the source chain and instrument,
  the baked automation) and status. The app computes its own fingerprint to
  decide staleness.

### Where the cache lives

The app writes caches to a sibling directory of the project bundle,
`<project>.freeze/freeze_<track_id>.wav` (`Song.rproj` → `Song.freeze/`),
with a self-ignoring `.gitignore`. A project never bundles its caches.

### Playback substitution

- `SetTrackFrozenSource { track_id, source: Some(FrozenSource) }` attaches a
  decoded cache; `None` or `UnfreezeTrack` detaches it. The app decodes the
  file with `resonance_audio::read_freeze_cache` (32-bit float stereo; int
  PCM accepted as a fallback; missing or wrong files return an error so the
  track loads as stale).
- A cache at another sample rate is converted to the engine rate on a worker
  (`FrozenSource::at_rate`, `PendingFrozenConversion`) before it is
  published; until then the track plays unfrozen. The mixer only ever reads
  a cache frame for frame.
- In the track job, a track with a frozen source fills its buffer from the
  cache (`fill_from_frozen_source`) instead of running its instrument and
  insert chain; frames past the end are silent. Its sub-tracks get nothing
  (the fan-out is baked into the parent's cache). Everything after the
  source stage (PDC, fader, pan, mute/solo, routing, **aux sends**) still
  runs. This path is shared by playback, export and stems.
- A frozen track's chain contributes 0 to PDC.
- A stem of a sub-track whose parent is frozen is refused.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Render | `resonance-audio/src/engine/bounce/freeze.rs` | `to_freeze_cache`, `baked_automation`, `compute_track_fingerprint`, `FreezeError`, `FREEZE_CANCELLED_MSG` |
| Decode for load | same file, re-exported from `resonance-audio/src/lib.rs` | `read_freeze_cache`, `FrozenCacheError` |
| Spawn + terminal event | `resonance-audio/src/engine/bounce/mod.rs` | `to_freeze_cache_spawn`, `freeze_terminal_event` |
| Strategy | `resonance-audio/src/mixer/render/strategy.rs` | `Bounce { freeze_raw }`, `force_master_route` |
| Attach / detach / rate conversion | `resonance-audio/src/engine/tracks.rs` | `handle_set_track_frozen_source`, `settle_frozen_conversions`, `PendingFrozenConversion` |
| Playback | `resonance-audio/src/mixer/render/frozen.rs`, `track_pass.rs` | `fill_from_frozen_source`, `render_track_source` |
| Types | `resonance-audio/src/types/freeze.rs`, `resonance-common/src/freeze.rs` | `FrozenSource`, `FreezeCacheRef`, `FreezeCacheStatus` |
| App | `resonance-app/src/update/freeze.rs`, `state/freeze.rs`, `engine_events/freeze.rs` | `freeze_cache_dir_for`, `cache_filename` |

## Surfaces

- **GUI:** track header freeze button, track menu, "freeze selected / all",
  progress modal, stale banner ([track-freeze-ui.md](../timeline/track-freeze-ui.md)).
- **Control API / MCP:** no freeze method. `song.tracks` reports
  `frozen: true` (`mcp__resonance__song_tracks`), and mutations of a frozen
  track's inputs are refused.
- **Agent skills:** `spatial` (`references/depth.md` mentions frozen
  tracks).

## How it's tested

| What | Where | Run |
|---|---|---|
| Render core, terminal events, cancel | `resonance-audio/tests/bounce/freeze_render_core.rs` | `./scripts/run-tests.py -p resonance-audio` |
| Cache decode | `tests/bounce/freeze_cache_read.rs` | same |
| Playback substitution | `tests/mixer/freeze_playback_substitution.rs` | same |
| Command plumbing | `tests/engine/freeze_command_plumbing.rs` | same |
| App | `resonance-app/tests/plugins/freeze_*.rs`, `frozen_track_render.rs`, `track_freeze_menu.rs`, `track_header_freeze_button.rs`; `tests/control/control_track_frozen.rs` | `./scripts/run-tests.py -p resonance-app` |

Not covered: a frozen track with aux sends (no test renders sends through a
freeze), and "frozen playback is sample-identical to unfrozen" for a track
with sends.

## Decisions and deviations

- The cache is captured pre-fader and pre-routing so moving the fader or
  re-routing does not require a refreeze.
- Caches live outside the project bundle as build artifacts.

## Known gaps

- **Aux sends are applied twice for a frozen track.** The freeze render
  passes no send filter, so the track's sends are tapped (at unity, since
  `freeze_raw` gains are 1.0), the return busses render, and their wet
  output is summed into the cache. On playback the reduction applies the
  same sends again from the cache. Read from the code, not yet measured:
  [freeze-aux-send-double-apply](../../intents/audio-engine/freeze-aux-send-double-apply.md).
