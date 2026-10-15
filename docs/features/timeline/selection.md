# Selection on the Arrange view

> Verified against master @ `6137b6fb` on 2026-10-15.
> Spec history: `docs/history/specs/command-palette.md` (selection-scoped
> commands, §3.4/§4.3); multi-track selection from epic #36 (todo #684).

What is "selected" on the Arrange view decides what the Delete key, the
selection commands (mute/solo/arm selected, split, loop selection, group,
freeze) and the inspector act on. Selection is transient UI state: it is not
saved and never enters the undo history.

## Behaviour

### What can be selected

| Thing | State field | How |
|---|---|---|
| Tracks (many) | `selected_tracks` (click order) + `selected_track` (primary = last) | Click a header or an empty part of a lane. Cmd/Ctrl- or Shift-click toggles a track in the set. |
| One audio clip | `selected_clip` | Press on the clip (move, trim, fade or gain gesture all select it). Also selects its track. |
| One MIDI clip | `selected_midi_clip` | Press on the clip; clears `selected_clip`. Double-click opens it in the [MIDI editor](midi-editor.md). |
| One tempo or signature event | `selected_global_event` | Click it in the expanded global shelf ([tempo-and-signature-map.md](tempo-and-signature-map.md)). |
| One marker | `selected_marker_id` | Click its flag in the ruler ([markers.md](markers.md)). |
| One automation breakpoint | canvas-local `TimelineState::selected_breakpoint` | Click a dot ([automation-lanes.md](automation-lanes.md)). |
| One section placement | Compose state | Click a pill in the section band (see [sections-and-placements.md](../compose/sections-and-placements.md)). |

There is **no multi-clip selection**, no marquee on the Arrange canvas and no
time-range selection.

### Rules

- A plain track click replaces the track selection and drops the clip
  selection. An additive click leaves clip selection alone.
- A press below the last row clears the track selection.
- A press on a group-header lane folds the group instead of selecting
  anything ([track-groups.md](track-groups.md)); a press on an automation or
  take sub-row selects the owning track.
- Selecting a track takes the bus / master selection off (the inspector shows
  one channel), and selecting a bus or the master clears track and clip
  selection.
- Right-clicking a track header selects that track and opens its menu.
- A removed track or clip is dropped from the selection.

### Keyboard ownership

The timeline canvas only reacts to Delete/Backspace while it owns the
keyboard (`focus::KeyFocus`). A press on the canvas takes it; a press on
another canvas gives it away. When a GUI message selects a clip or global
event from elsewhere (e.g. a clip clicked in a list), it bumps
`timeline_key_grant`, and the canvas takes the keyboard on its next event, so
the new selection is deletable without a click (FU-C2). Control-API calls
never bump it. Text typed into the command palette never deletes the clip.

### Commands that act on the selection

| Command (palette name) | Default chord | Acts on |
|---|---|---|
| Delete selection (`TimelineDeleteSelection`) | Delete / Backspace (canvas-local) | global event first, else MIDI clip, else audio clip |
| Mute / Solo / Arm Selected Tracks | M / S / Shift+R | all selected tracks, **one** undo entry; a mixed selection turns the flag on for all, only an all-on selection turns it off; frozen tracks are skipped for arm |
| Monitor selected | — | selected tracks |
| Delete Selected Track | Cmd/Ctrl+Backspace | primary track (canvas Delete wins while the canvas holds a clip selection) |
| Split Clip at Playhead | Cmd/Ctrl+E | selected clip under the playhead, or the take comp under it |
| Loop Selection | Cmd/Ctrl+L | selected audio clip, else MIDI clip, else section placement, else marker region |
| Duplicate Selection | Cmd/Ctrl+D | the selected **section placement** only (places a copy after it) — not clips |
| Group Selected Tracks | Cmd/Ctrl+G | ≥ 2 selected tracks |
| Freeze Selected Tracks | Cmd/Ctrl+F | the **primary** track only (`selected_freezable_tracks` reads `selected_track`, not the multi-selection) |

An unavailable command stays visible in the palette with the reason (e.g.
"Select a clip under the playhead").

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Selection state | `resonance-app/src/state/interaction.rs` | `ClipInteractionState` (`selected_clip`, `selected_midi_clip`, `selected_track(s)`, `select_additive`, `timeline_key_grant`), `select_single_track`, `toggle_track_selection`, `make_primary_track` |
| Channel exclusivity | `resonance-app/src/state/ui_transient.rs` | `UiTransientState::select_track`, `toggle_track_selection`, `clear_channel_selection` |
| Track / bus / master select reducers | `resonance-app/src/update/ui.rs` | `UiMessage::SelectTrack`, `SelectBus`, `SelectMaster`, `OpenTrackMenu`; modifiers → `select_additive` |
| Clip selection on press | `resonance-app/src/update/clips.rs` | `start_clip_drag`, `start_midi_clip_drag`, trim/fade/gain starts |
| Canvas press routing | `resonance-app/src/view/timeline/input/pointer.rs` | `handle_press`, `press_clips` |
| Command targets | `resonance-app/src/commands/resolve.rs` | `timeline_delete`, `selection_range`, `split_target`, `duplicate_target`, availability reasons |
| Bindings | `resonance-app/src/commands/bindings.rs` | `BindingMap::resonance_default` |
| Multi-track toggles | `resonance-app/src/update/track.rs` | `TrackMessage::Toggle{Mute,Solo,Arm}Selected`, `toggle_selected` |
| Keyboard focus | `resonance-app/src/focus.rs` | `KeyFocus` |
| Selection bar | `resonance-app/src/view/selection_bar.rs` | floating "N tracks selected · Group" pill |

## Surfaces

- **GUI:** clicks on the Arrange canvas and header column, the command
  palette (Cmd/Ctrl+K), the chords above.
- **Control API / MCP:** none. The control API addresses everything by id
  (`clip_id`, `track_id`) and never reads or changes the GUI selection.
- **Agent skills:** none.

## How it's tested

| What | Where | Run |
|---|---|---|
| Selection commands, one-undo-entry rule, split, loop selection | `resonance-app/tests/timeline/selection_commands.rs` | `./scripts/run-tests.py -p resonance-app` |
| Key grant / Delete routing | `resonance-app/tests/timeline/timeline_key_grant.rs`, `editor_key_focus.rs` | same |
| Group-from-selection and additive click | `resonance-app/tests/mixer/group_creation_from_selection.rs` | same |
| Selection bar goldens | `tests/timeline/selection_bar.rs` → `tests/snapshots/selection_bar_two_tracks-wgpu.png`, `selection_bar_many_tracks-wgpu.png` | `RESONANCE_BLESS=1 cargo test -p resonance-app --test timeline` |

Not covered: the audio-after-MIDI clip selection case below.

## Decisions and deviations

- **Selection is never undoable.** Undo restores content, not what was
  highlighted (`undo_view_state_keeps_redo.rs` covers that view state does not
  disturb redo).
- **Selection commands are one undo entry** however many tracks they touch.
- **Delete is canvas-local**, so typing in a text field or the palette never
  deletes a clip.

## Known gaps

- No multi-clip selection, marquee, range selection, copy/paste or clip
  duplicate — see
  [../../intents/timeline/clip-multi-select.md](../../intents/timeline/clip-multi-select.md) and [../../intents/timeline/clip-copy-duplicate.md](../../intents/timeline/clip-copy-duplicate.md).
