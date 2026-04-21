#TODO

##General
    - [x] Conformation when a user wants to kill the app, but there are unsaved changes
    - [x] Conformation when a user wants to delete a track which has content.
    - [x] I want to create presets for track (bas guitar, rhythm guitar, solo, etc) we should have user presets, and a number of default presets.

##Arrange tab
    - [x] We need a place for global tracks like tempo track, and signature changes, my idea would an collapsable area between the normal tracks and the time indication
    - [x] Implement tempo track and signature track
    - [x] Selecting a track should highlight it
    - [x] Move the delete track button to the top right position. Its different from the rest of the buttons
    - [x] The solo functionality does not seem to work.

##Mix tab
    - [x] We need a new solution for subtracks. My suggestion would be to make the collapsed view the default. But then make it a bit wider (maybe two slots, and show db meters of all subtracks. When expendanded the user can modify gain etc.
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

    - [x] Deduplicate tempo/signature event removal logic in `update.rs:141-233`.

    - [x] Add debug-build warnings for silent track/clip lookup misses.

    - [x] Promote sub-tracks to an engine concept or document the gap.

    - [x] Document `fetch_max` invariant on peak level atomics.

    - [x] Cap `ClapInstance.pending_params` to prevent unbounded growth.

    - [x] Centralize and document hard-coded limits.
