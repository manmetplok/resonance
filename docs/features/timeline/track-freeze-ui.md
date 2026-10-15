# Track freeze (UI and app state)

> Verified against master @ `6137b6fb` on 2026-10-15.
> Spec history: `docs/history/prototypes/track-freeze-bounce-in-place/index.html` (design doc #181); ba todos #571–#583 and code review UPD-05, cited in code comments.

Freezing renders an instrument or vocal track's post-FX output to a cache
WAV so playback reads the cache instead of running the synth and plugin
chain. This doc covers what the user sees and the app-side state machine:
the header toggle, menus, batch freeze, progress modal, stale and failed
banners, read-only gating and persistence. The offline render and cache
playback are engine work — see [track-freeze.md](../audio-engine/track-freeze.md).

## Behaviour

**Status machine** (`FreezeStatus`, one per track, absent = `Idle`):
`Idle` → `Freezing { fraction }` → `Frozen { cache_ref }`, or `Failed { message }`.
A frozen track whose inputs change becomes `Stale { cache_ref }` and keeps
playing the old cache until refrozen or unfrozen.

**What can be frozen.** Instrument and vocal tracks that are not sub-tracks
(`freezable`). Audio tracks and sub-tracks are refused with a banner message.
A freeze is also refused while the transport plays, while an offline
measurement (`meter.*`) runs, or when the project was never saved (the cache
needs a directory).

**Actions** (`FreezeMessage`):
- `FreezeTrack` / `UnfreezeTrack` — the snowflake button leading the track
  header's mini-button row toggles between them; also in the track context
  menu. Unfreeze detaches the cache from the engine and deletes the file. On a
  `Failed` track it just clears the status.
- `RefreezeTrack` — re-render in place (the stale banner's primary action).
- `FreezeSelectedTracks` / `FreezeAllTracks` — sequential batch, one track at a
  time (the offline renderer shares plugin instances with the live mixer).
  "Freeze all" is the button in the Tracks header cap (enabled only when some
  freezable track is not already frozen) and a palette command; "Freeze
  selected" is a palette command and context-menu item.
- `CancelFreeze` — cancels the render and abandons the batch. The only message
  the freeze gate lets through while a render runs.
- `RevealFreezeCache` — opens the cache directory in the file manager.

**While rendering.** A progress modal (`view/bounce_progress.rs`) shows the
fraction and, for a batch, "N / M"; every other user message is blocked
(`freeze_blocks_message`), as during a bounce.

**While frozen.**
- Inputs are read-only: note and lyric edits, instrument and plugin-chain
  edits, and plugin parameter changes are dropped before dispatch (no mutation,
  no undo entry) and flip the track to `Stale` instead
  (`frozen_input_edit_target`, ba todo #576). Volume, pan, mute, solo, routing
  and sends stay live.
- Record-arm and monitor buttons lock.
- Audio clips on a frozen track draw the "unsupported" surface (hatched, no
  fade handles); the clip inspector shows a degraded flyout ("Track is frozen —
  fades are disabled until you unfreeze it").
- Edits that bypass the pre-dispatch gate — Compose regeneration, insert/remove
  bars, tempo changes — are caught by a content fingerprint compared after
  every dispatch (`content_baselines`, `revalidate_frozen_content`, code review
  UPD-05). Plugin-parameter automation lanes on the track also mark it stale; a
  fader lane does not.

**Banners** (`view/freeze_banner.rs`): amber "Frozen audio is out of date"
with **Refreeze** / **Unfreeze to edit** for `Stale`; pink "Freeze failed —
<reason>" with **Retry** / **Dismiss** for `Failed`.

**Persistence.** Each `ProjectTrack` carries a `TrackFreezeState` (shared
`resonance_common` type). Only `Frozen` / `Stale` are saved; `Freezing` and
`Failed` load as `Idle`. Caches live in a sibling `<Project>.freeze/` directory
next to the `.rproj` bundle (with a self-ignoring `.gitignore`), not inside it.
On load `rehydrate_frozen_tracks` re-attaches caches; a missing or undecodable
cache downgrades the track to `Stale`.

**Undo.** Freeze, unfreeze, refreeze and batch freeze are each one undo entry;
the rendered cache is not in history. `apply_freeze_restore` reconciles after
undo/redo: it detaches and deletes caches of tracks no longer frozen,
re-attaches caches the engine does not hold, and abandons any batch.

Related but separate: **Bounce in place** (`TrackMessage::BounceInPlace`,
palette "Bounce Selected Track in Place") renders a track to a new audio
track; it shares the progress modal but is not a freeze.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| App state | `resonance-app/src/state/freeze.rs` | `FreezeStatus` (`to_persisted`, `from_persisted`), `FreezeQueue`, `FreezeState` (`statuses`, `queue`, `content_baselines`, `any_in_flight`) |
| Handlers | `resonance-app/src/update/freeze.rs` | `FreezeMessage`, `freeze_one`, `unfreeze_one`, `start_batch`, `advance_freeze_queue`, `freezable`, `freezable_tracks`, `selected_freezable_tracks`, `freeze_cache_dir_for`, `rehydrate_frozen_tracks`, `apply_freeze_restore`, `invalidate_frozen_track`, `freeze_content_fingerprint`, `revalidate_frozen_content` |
| Gates | `resonance-app/src/update/gates.rs` | `freeze_blocks_message`, `frozen_input_edit_target` |
| Engine-event mirror | `resonance-app/src/engine_events/freeze.rs` | `FreezeProgress` / `FreezeCompleted` / `FreezeError` / `FreezeCancelled` |
| Header button | `resonance-app/src/view/track_header/track.rs`, `view/controls.rs` | `freeze_button`, `freeze_toggle_message` |
| Header-cap "Freeze all" | `resonance-app/src/view/track_header/shelf.rs`, `track_header/mod.rs` | `freeze_all_enabled` |
| Context menu | `resonance-app/src/view/menus.rs` | freeze / unfreeze / freeze selected / freeze all / reveal cache items |
| Banners | `resonance-app/src/view/freeze_banner.rs` | `freeze_banner` |
| Progress modal | `resonance-app/src/view/bounce_progress.rs` | Cancel → `CancelFreeze` |
| Frozen clip surface | `resonance-app/src/view/timeline_panel.rs`, `view/clip_inspector.rs` | `frozen_tracks`, `degraded_flyout` |
| Commands | `resonance-app/src/commands/{mod,resolve}.rs` | `FreezeSelectedTracks`, `FreezeAllTracks` |

## Surfaces

- **GUI:** Arrange track header snowflake button; track context menu; Tracks
  header-cap "Freeze all"; command palette ("Freeze Selected Tracks", "Freeze
  All Tracks"); progress modal; stale / failed banners.
- **Control API / MCP:** read-only. `song.tracks` (`TrackDetail`) reports `frozen: true` for a
  track playing from a cache (valid or stale), and every control method that
  edits a frozen track's inputs answers `busy`. No method freezes, unfreezes or
  refreezes. See [intent: freeze control API](../../intents/timeline/freeze-control-api.md).
- **Agent skills:** none.

## How it's tested

| What | Where | Run |
|---|---|---|
| Handlers + undo | `resonance-app/tests/plugins/freeze_handlers.rs` | `./scripts/run-tests.py -p resonance-app` |
| Engine-event mirror, batch advance | `resonance-app/tests/plugins/freeze_event_mirror.rs` | same |
| Persistence + cache lifecycle | `resonance-app/tests/plugins/freeze_persist.rs` | same |
| Read-only gating → stale | `resonance-app/tests/plugins/freeze_readonly.rs` | same |
| Content fingerprint → stale (UPD-05) | `resonance-app/tests/plugins/freeze_stale_on_content.rs` | same |
| Context menu + Freeze all | `resonance-app/tests/plugins/track_freeze_menu.rs` | same |
| Progress modal | `resonance-app/tests/plugins/freeze_progress_modal.rs` | same |
| Frozen render, banners, header button (goldens) | `resonance-app/tests/plugins/{frozen_track_render,freeze_banner_render}.rs`, `tests/timeline/track_header_freeze_button.rs`; `tests/snapshots/{frozen_track_header_and_lane_render,stale_refreeze_banner,freeze_failed_banner,freeze_progress_modal_single,freeze_progress_modal_batch,track_header_freeze_button_idle,track_header_freeze_button_frozen,track_context_menu_frozen}-wgpu.png` | `RESONANCE_BLESS=1 cargo test -p resonance-app --test plugins` |
| Control methods refuse frozen tracks | `resonance-app/tests/control/control_track_frozen.rs` | `./scripts/run-tests.py -p resonance-app` |
| Engine render / playback | `resonance-audio/tests/bounce/{freeze_render_core,freeze_cache_read}.rs`, `tests/engine/freeze_command_plumbing.rs`, `tests/mixer/freeze_playback_substitution.rs` | `./scripts/run-tests.py -p resonance-audio` |

Not covered: a real freeze of a third-party CLAP instrument; "Reveal freeze
cache" opening a file manager.

## Decisions and deviations

- The cache sits **beside** the project bundle so saves never copy large WAVs
  and the cache can be deleted wholesale.
- Freezes run one at a time; parallel renders would interleave `process()`
  calls on shared plugin instances.
- An edit to a frozen input is dropped and the track goes stale, rather than
  auto-unfreezing: nothing changes behind the user's back, and the stale cache
  still plays.

## Known gaps

- No control API to freeze / unfreeze —
  [freeze-control-api](../../intents/timeline/freeze-control-api.md).
