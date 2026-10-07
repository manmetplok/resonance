# Overlays, dialogs and the status area

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: `docs/history/specs/command-palette.md` §3.2 (overlay gate,
> Esc order); code review UX-01/04/05/07/13 (status area, project switch).

At most one window-level overlay shows at a time: a modal dialog, a popover
or the palette. The app picks it from state in one fixed priority order.
The keyboard gate and the renderer read the same function, so the overlay a
user sees is always the one the shortcuts are gated on. Esc closes it.
Above the main area, a status area carries the audio-engine health line,
the autosave-failure line and the dismissable error banner.

## Behaviour

**One root overlay.** `Resonance::root_overlay()` returns the first match in
this order:

| # | Overlay | Shown when | Esc / backdrop |
|---|---|---|---|
| 1 | `Recovery` | `io.recovery_prompt` is set (drawn over the startup screen if nothing is open) | Cancel |
| 2 | `Startup` | `!io.has_active_project` | cannot close |
| 3 | `BounceProgress` | a bounce in place is running | Esc does nothing; Cancel button |
| 4 | `MixdownProgress` | `io.bouncing` (WAV mixdown) | same |
| 5 | `FreezeProgress` | any freeze in flight | same |
| 6 | `Palette` | `ui.palette` is open | close |
| 7 | `ConfirmQuit` | window close requested with unsaved changes | Cancel |
| 8 | `ConfirmProjectSwitch` | GUI Open / Open Recent / New over unsaved changes | Cancel |
| 9 | `ConfirmDeleteTrack` | deleting a track that has clips | Cancel |
| 10 | `BounceDialog` | bounce-in-place setup for an external instrument | Cancel |
| 11 | `ExportDialog` | Export (⇧⌘E) | Close |
| 12 | `ImportDialog` | MIDI import | Cancel |
| 13 | `ImportProgress` | audio import transcoding | Dismiss |
| 14 | `MissingPlugins` | a project loaded with missing plugin slots | Dismiss |
| 15 | `Relink` | a project loaded with missing audio files | Dismiss |
| 16 | `Settings` | `ui.mixer.settings_open` | Close |
| 17 | `PresetBrowser` | the host preset browser is open | Esc reverts the audition; backdrop keeps it |
| 18 | `AddTrackMenu` | the add-track popover | Close |
| 19 | `MarkersOverview` | the markers popover | Close |
| 20 | `DrumGroupsManager` | Compose drum-groups manager | Close |
| 21 | `MarkerMenu` | marker context menu or inline rename | Close or cancel the rename |
| 22 | `MidiMenu` | the right-click MIDI Learn menu | Close |
| 23 | `TrackMenu` | track context menu or its "Save as preset…" prompt | Close |
| 24 | `SelectionBar` | Arrange with two or more tracks selected | non-modal |

Every overlay except `SelectionBar` is modal (`blocks_keys`). While a modal
overlay shows:

- only ⌘/Ctrl chords and Esc reach the registry;
- the canvases ignore keys (`canvas_keys_blocked`);
- the palette can open over it, closing it first, except over rows 1–5
  (`allows_palette`).

`view()` stacks the chosen overlay over the base view, Performance mode
included. Two layers are drawn outside the overlay chain:

- `TrackMenu` is drawn inside the Arrange main area, because its anchor is
  in arrange-area coordinates.
- The generic plugin window floats between the base view and the root
  overlay, and is hidden while the track menu is up
  ([plugin-windows.md](plugin-windows.md)).

**Message gates.** These are separate from the overlay, in
`update/gates.rs`:

- While the startup screen is up, project-mutating messages are swallowed.
- While a bounce, mixdown or freeze runs, messages that would disturb the
  engine are swallowed.
- A command run from the palette or a shortcut meets the same gates.

**Esc order.** See [command-registry-and-shortcuts.md](command-registry-and-shortcuts.md).
In short, Esc goes to the first of these that applies:

1. A rebind capture.
2. The palette.
3. The preset drag or preset browser.
4. An inline rename, then the CHAIN transient state.
5. The topmost modal overlay.
6. The generic plugin window, then an armed MIDI Learn.
7. Exit Performance mode.

**The dialogs** (one file each in `resonance-app/src/view/`)

- `confirm_quit.rs`:
  - "Unsaved changes" (Cancel / Discard & Quit / Save & Quit).
  - The project-switch dialog "Save the changes to “X” before opening “Y”"
    (Cancel / Don't Save / Save, or Save As… for an untitled project).
  - Both share `unsaved_changes_dialog`.
- `confirm_delete_track.rs`: confirms deleting a track that has audio or
  MIDI clips. An empty track is deleted at once, still as one undo entry.
- `missing_plugins_dialog.rs`: one row per dead plugin slot, naming the
  plugin, its chain and its position, and one Rescan action. The fixes for
  a single slot live on the slot itself, in the mixer CHAIN. Settings ›
  General keeps a "N plugin slot(s) … could not be loaded — review" link
  back to the dialog.
- `export_dialog.rs`: Audio stems and MIDI tabs. The MIDI tab is disabled: the
  engine-side writer `resonance_audio::midi_io::write_midi_project` exists and is
  tested, but nothing calls it ([midi-project-export](../../intents/audio-engine/midi-project-export.md)).
  The export itself is documented in
  [stem-export.md](../audio-engine/stem-export.md) and
  [offline-render-and-export.md](../audio-engine/offline-render-and-export.md).
- `bounce_dialog.rs`, `bounce_progress.rs`: bounce-in-place setup, and the
  blocking progress modals for bounce, freeze and mixdown, each with
  Cancel.
- `import_dialog.rs`, `import_progress_dialog.rs`, `relink_dialog.rs`: the
  MIDI import, audio transcode and missing-media flows
  ([smf-import.md](../hardware-io/smf-import.md), [audio-pool-and-import.md](../timeline/audio-pool-and-import.md), [relink.md](../timeline/relink.md)).
- `menus.rs`:
  - The add-track popover: Audio, Instrument, Ext Instrument and Vocal,
    then built-in track presets, then user presets with a two-step delete.
  - The marker menu: Convert, Rename, Recolor, Loop to section, Play from
    here, Delete.
  - The track menu: Freeze/Unfreeze track, Freeze selected tracks, Freeze
    all tracks (with registry chord hints), Save as preset…, Reveal freeze
    cache….
- `selection_bar.rs`: the floating "Group selected" bar.

**Quit and switch flow.**

- `iced::window::close_requests()` becomes `WindowCloseRequested`. The
  window is created with `exit_on_close_request: false`.
- With unsaved changes on an open project, the app asks. Otherwise it closes
  the recovery session, shuts the engine down (150 ms) and closes the
  window.
- Save & Quit saves first; an untitled project becomes a Save As. If an edit
  lands during the save, the app asks again. A failed save cancels the quit
  and shows a "Save failed" banner.
- Open and New go through `request_switch`, which works the same way
  through `switch_after_save`.

**Status area** (`view/mod.rs::view_status_area`). It sits between the
transport and the main area and shows, worst first:

1. **AUDIO**, in `BAD`: the engine stream was lost, or the engine thread
   died (`EngineHealth`, polled every tick). It clears itself when the
   condition clears.
2. **AUTOSAVE**, in `WARM`: three or more autosaves in a row failed
   (`AUTOSAVE_FAILURES_BEFORE_INDICATOR`). Any successful save clears it.
3. **ERROR**: the transient banner (`Banners::error_message`) with a ×
   dismiss button. A newer message replaces it.

The status column is always a child of the layout, even when it has zero
height, so a banner appearing or disappearing never shifts `main_area`'s
child index. A shift would reset its scroll offsets, key focus and drags.

**Undo notice.** After an undo or redo, the chrome shows "Undid …" or
"Redid …" for 2.5 s (`HISTORY_NOTICE_DURATION`). It sits in a fixed 200 px
slot, so it never moves the view tabs.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Overlay choice and gate | `resonance-app/src/state/overlay.rs` | `Overlay`, `root_overlay`, `modal_overlay`, `blocks_keys`, `allows_palette`, `dismiss_message`, `canvas_keys_blocked` |
| Rendering | `resonance-app/src/view/mod.rs` | `view`, `view_root_overlay`, `view_status_area`, `status_line`, `error_bar` |
| Modal flags | `resonance-app/src/state/modal.rs` | `confirm_quit`, `confirm_switch`, `switch_after_save`, `quit_after_save`, `confirm_delete_track`, `bounce_dialog`, `export_dialog`, `ProjectSwitch` |
| Banners | `resonance-app/src/state/banners.rs` | `Banners`, `EngineHealth`, `autosave_failing`, `HistoryNotice` |
| Message gates | `resonance-app/src/update/gates.rs` | `is_gated_message`, `gates_message` |
| Quit and switch | `resonance-app/src/update.rs` (`WindowCloseRequested`), `update/project_io/mod.rs` | `request_switch`, `answer_switch`, `perform_switch` |
| Esc dispatch | `resonance-app/src/update/shortcuts.rs` | `dismiss_overlay` |

Invariant: add a new overlay to **both** the `Overlay` enum and
`root_overlay`, and give it a `dismiss_message`. Otherwise Esc can't close
it and the keyboard gate can't see it.

## Surfaces

- **GUI:** everywhere.
- **Control API / MCP:** control methods never open dialogs. Destructive
  calls take a `confirm` parameter instead. While the startup screen is up,
  control mutations fail with "no active project — open or create one first".
- **Agent skills:** none.

## How it's tested

| What | Where | Run |
|---|---|---|
| Modal gate, Esc before Performance exit, startup can't be closed | `resonance-app/tests/performance/shortcut_dispatch.rs` | `./scripts/run-tests.py -p resonance-app` |
| Track menu gates bare keys, Esc closes it | `resonance-app/tests/performance/shortcut_review_fixes.rs` | same |
| Project switch confirm, untitled chrome | `resonance-app/tests/io/project_lifecycle_ux.rs` | same |
| Quit flow (`ConfirmSaveAndQuit`, `ConfirmDiscardAndQuit`, `quit_after_save`) | `resonance-app/tests/io/crash_detection.rs` | same |
| Delete-track confirm and undo | `resonance-app/tests/mixer/delete_track_confirm_undo.rs` | same |
| Status area keeps widget state; engine-health line | `resonance-app/tests/timeline/status_area_keeps_widget_state.rs`, `mixer/tick_gating.rs` | same |
| Banner contrast | `resonance-app/tests/mixer/theme_contrast.rs` | same |
| Goldens | `status_area_all_lines`, `project_switch_confirm`, `untitled_chrome_not_saved`, `selection_bar_*`, `export_dialog_*`, `import_*`, `relink_modal_*`, `freeze_progress_modal_*`, `mixdown_progress_modal`, `markers_overview_popover_renders`, `track_context_menu_*`, `add_track_menu_preset_delete_armed` in `resonance-app/tests/snapshots/` | `RESONANCE_BLESS=1 …` |

Not covered: there are no goldens for the quit confirm, the delete-track
confirm or the missing-plugins dialog. No test checks that every `Overlay`
variant has a reachable state.

## Decisions and deviations

- Esc never closes the startup screen or a progress modal. Stopping a render
  takes the explicit Cancel button.
- The bounce, mixdown and freeze progress modals sit above everything except
  recovery and startup, because input during a render could change state
  the engine is not ready for.
- The project-switch confirm (code review UX-01) replaced the earlier rule
  that New Project was "unavailable while there are unsaved changes". That
  rule is still in the command-palette spec's status line and in older
  notes, but the code now asks.
- The error banner was restyled from a solid `BAD` fill (no text colour
  reached 4.5:1 on it) to a tinted wash with an "ERROR" chip (UX-07).

## Known gaps

- The MIDI tab of the export dialog is disabled although `write_midi_project` exists (no caller).
- Whether project-lifecycle and quit messages should stay blocked while a
  render runs is an open product question (`update/gates.rs` comments).
