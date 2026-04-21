#TODO

##General
    - [x] Conformation when a user wants to kill the app, but there are unsaved changes
    - [x] Conformation when a user wants to delete a track which has content.
    - I want to create presets for track (bas guitar, rhythm guitar, solo, etc) we should have user presets, and a number of default presets.

##Arrange tab
    - [x] We need a place for global tracks like tempo track, and signature changes, my idea would an collapsable area between the normal tracks and the time indication
    - [x] Implement tempo track and signature track
    - [x] Selecting a track should highlight it
    - [x] Move the delete track button to the top right position. Its different from the rest of the buttons
    - [x] The solo functionality does not seem to work.

##Mix tab
    -We need a new solution for subtracks. My suggestion would be to make the collapsed view the default. But then make it a bit wider (maybe two slots, and show db meters of all subtracks. When expendanded the user can modify gain etc.
    - [x] Something goes wrong when saving with busses. When opening a saved project the bus is there, but does not work. When removing the bus, audio can be heard again.

##Compose tab
    - [x] We need a solution about the editing of instruments, the current view is to small so its hard to pick the correct notes. We need to brainstorm about this.
    

##Plugins general
    - We need better controls, the current control boxes run out of screen, maybe knobs are better then slider. Also the visuals (scopes etc) can be smaller, and maybe stacked. Lets come up with a coherent design for each plugin and inplement it.

##Delay plugin
    - [x] Feedback range is to large afaik


##Drum plugin
    - [x] We should be able to download the drumkit as a zip from a server. (see https://resonance.plok.org/index.json)
    - [x] Check if round robin is executed correctly (also add unit tests)
    - [x] Implement all pads/parts found in /home/jorrit/Documents/Guitar/drummica
    - [x] Add ability to delete installed drumkits from the download panel

##Architecture / code health (from code review)

    - Break up the `Resonance` god object (`resonance-app/src/main.rs:25-82`).
      The struct holds 30+ fields and every message routes through a single 1000-line
      `dispatch()` in `update.rs`. Give each sub-state (`TransportState`, `MixerUiState`,
      `ClipInteractionState`, etc.) its own `handle()` method that takes the relevant
      message variant and an `&AudioEngine`, so `dispatch()` becomes a thin router.
      This keeps Iced's single-state-tree but makes ownership clear.

    - Full engine replay on undo is expensive for large projects
      (`resonance-app/src/undo.rs` — `replay_loaded_project`).
      Every undo/redo tears down all engine state and re-instantiates plugins.
      Short-term: diff old and new snapshots and only replay changed tracks/plugins.
      Long-term: consider command-based undo with inverse operations.

    - Deduplicate tempo/signature event removal logic in `update.rs:141-233`.
      AddTempoEvent, RemoveTempoEvent, AddSignatureEvent, RemoveSignatureEvent,
      and DeleteSelectedEvent all repeat the same pattern (validate index > 0,
      remove from vec, rebuild, sync display). Extract a generic helper like
      `remove_global_event(events, index, rebuild_fn)`.

    - Add debug-build warnings for silent track/clip lookup misses.
      Throughout `update.rs`, calls like `self.with_track_mut(id, |t| ...)` silently
      no-op if the track doesn't exist. Add `debug_assert!` or `tracing::warn!` on
      the `None` path so undo/redo desync issues are immediately visible during
      development. Keep silent in release builds.

    - Promote sub-tracks to an engine concept or document the gap.
      Sub-tracks (multi-output instrument routing) are created only in the UI
      when `PluginAdded` events arrive (`resonance-app/src/engine_events.rs`).
      The audio engine has no "sub-track" concept — they're regular tracks with
      `sub_track_of` set. If the UI-side creation fails, audio routing is wrong.
      Either make the engine emit `SubTrackCreated` events, or add defensive
      checks and a clear doc comment explaining the abstraction boundary.

    - Document `fetch_max` invariant on peak level atomics.
      `resonance-audio/src/mixer.rs:260-265` uses `fetch_max` on bit-punned
      `AtomicU32` for peak metering. This only works because peak values are always
      non-negative (`.abs()` is applied). Add a comment at the `fetch_max` site
      explaining this invariant, or use a wrapper that does
      `loop { CAS with f32::max }` for correctness regardless.

    - Cap `ClapInstance.pending_params` to prevent unbounded growth.
      `resonance-audio/src/clap_host.rs` — `pending_params: Vec<(u32, f64)>`
      grows without limit if the GUI automates many parameters between process
      calls. Cap at 128 entries, deduplicating by param_id (keep last value).

    - Centralize and document hard-coded limits.
      `MAX_PLUGIN_OUTPUT_PORTS = 8` (mixer.rs:23), `MAX_BUSSES = 32`
      (engine/mod.rs:16), `MAX_INPUT_CHANNELS = 32`, undo history = 200
      (undo.rs:23), metronome = 16 beats/buffer. Add a `limits.rs` or a
      doc comment block that lists all limits in one place, and add runtime
      validation (log a warning) when a limit is hit rather than silently
      truncating.
