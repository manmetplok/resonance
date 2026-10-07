# MIDI editor (piano roll) and quantize

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: none in `docs/history/specs/`. The quantize, humanize and groove work is ba doc #163 / epic #25 (todos #388–#396).

Double-clicking a MIDI clip in the Arrange view opens it in the bottom
editor panel. In the piano roll, users draw, move, resize and delete notes,
select notes with a click, shift-click or marquee, and audition pitches on
the keyboard strip. The Quantize toolbar above the grid quantizes, humanizes
or applies a groove to the selection, or to the whole clip when nothing is
selected. A clip on a vocal track opens in the vocal roll instead, which is
part of the vocal feature (`../compose/`). An agent edits the same notes
through the `notes.*` control methods.

## Behaviour

### Opening and layout

- Ways to open the editor:
  - double-click a MIDI clip body in the timeline (`OpenMidiEditor`);
  - press `Enter` on the selected MIDI clip (`OpenSelectedMidiClip`);
  - choose Open Selected MIDI Clip in the palette.
- `Cmd+Esc` or the "Close clip" button closes it.
- `open_midi_editor` sets `MidiEditorState`:
  - horizontal zoom: `zoom_x` 0.5 px/tick;
  - row height: `zoom_y` 12 px;
  - snap: a 16th note (`snap_ticks` = `TICKS_PER_QUARTER_NOTE / 4`);
  - vertical scroll: 300 px (about C4). The vocal roll starts at 0 instead.
- **None of these can be changed afterwards** except the vertical scroll
  (mouse wheel). There is no zoom control and no snap picker.
- `EditorVariant` (`classify_editor_variant`) picks the piano roll for every
  track type except Vocal.
- The canvas is the clip length plus 4 bars wide, inside a horizontal
  scrollable. It has a 50 px keyboard on the left and a 40 px velocity lane
  at the bottom. The velocity lane only shows velocities; it does not take
  input. The panel is 330 px tall.

### Note editing (`view/midi_editor/input.rs`)

- **Click on empty grid:**
  - With nothing selected, it creates a note: snapped start, length of one
    snap step, velocity 0.8.
  - With a selection, it clears the selection instead.
  - A drag on empty grid is a marquee. Shift adds to the current selection.
- **Click on a note** selects it. Dragging the body moves it, snapped in
  time and free in pitch. Dragging the 6 px right edge resizes it, never
  shorter than one snap step. Shift- or Cmd-click toggles the note in the
  selection and never starts a drag.
- **Right-click on a note** deletes that note.
- **Keys, routed through the keymap and only while the canvas has key
  focus:**
  - Delete/Backspace (with or without Shift or Cmd) deletes the selected
    notes;
  - `Cmd+A` selects all notes;
  - Select Notes in View selects the notes currently visible.
- **Keyboard strip:** pressing a key sends `SendNoteOn` to the track at
  velocity 0.8, and releasing sends note-off.
- Notes are addressed **by index** in a list sorted by start time. While a
  drag moves a note past a neighbour, the input layer re-tracks the dragged
  note with `move_note_resorted`.
- All edits go to the engine (`AddMidiNote`, `MoveMidiNote`,
  `ResizeMidiNote`, `RemoveMidiNote`, `SetMidiNoteVelocity`,
  `SetMidiClipNotes`). The app mirror updates from the `MidiNotesEdited`
  echo. Control writes also update it optimistically.
- Note edits are refused on a **frozen** track
  (`gates::midi_editor_edit_clip` / `midi_editor_edits_open_clip`).

### Quantize, humanize, groove (`view/midi_quantize.rs`)

- Each operation targets the current selection, or every note when nothing
  is selected (`bulk_target`). Each one sends a single bulk command that the
  engine applies at once and echoes once, so each is **one undo step**.
  - **Quantize** (`QuantizeMidiNotes`) settings:
    - grid division, including triplet and dotted feel;
    - strength 0–1 and swing 0–1;
    - Start-only or Start+Length;
    - whether note ends are quantized too;
    - iterative ("soft") mode.
  - **Humanize** (`HumanizeMidiNotes`): timing jitter of at most 240 ticks
    (an eighth note) and velocity jitter of 0–1. Each run draws one new
    seed, so the run is reproducible inside its own undo step.
  - **Groove:** *Apply* (`ApplyGrooveToClip`) applies a built-in groove
    (looked up by name) or a user groove (`"user:<id>"`) at a chosen
    strength. *Extract* (`ExtractGrooveFromClip`) reads the whole clip and
    adds a `UserGroove` to the project's groove library under the typed
    name, or an automatic numbered name.
- While the toolbar is open, the piano roll draws the quantize grid,
  including swing, and "ghost" previews of where the selected notes would
  land (`quantize_grid_steps`, `ghost_targets`).
- The toolbar settings live in `MidiQuantizePanelState`, which is view state:
  it is not undoable and starts from defaults every session.

### Clip-level actions

Creating, deleting, moving and trimming MIDI clips on the timeline
(`MidiClipMessage`) is covered in [clip-editing.md](./clip-editing.md).
**The Arrange view has no gesture that creates an empty MIDI clip.** Clips
come from Compose generators, MIDI recording, the MIDI import dialog (see
[audio-pool-and-import.md](./audio-pool-and-import.md)), or
`notes.create_clip`.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Editor messages + handlers | `resonance-app/src/update/midi_editor.rs` | `MidiEditorMessage`, `bulk_target`, `lookup_groove`, `remove_selected_notes` |
| Open editor | `resonance-app/src/update/clips.rs` | `open_midi_editor`, `MidiEditorState` |
| Selection model | `resonance-app/src/state/interaction.rs` | `MidiEditorState::{select_single, toggle_note, apply_marquee}`, `MidiQuantizePanelState`, `HUMANIZE_TIMING_MAX_TICKS` |
| Project quantize state | `resonance-app/src/state/quantize.rs` | `QuantizeState`, `UserGroove`, `GrooveSelection`, `QuantizeSettings` |
| Canvas | `resonance-app/src/view/midi_editor/{mod,input,draw}.rs` | `PianoRollCanvas`, `QuantizePreview`, `quantize_grid_steps`, `ghost_targets` |
| Shared roll geometry | `resonance-app/src/view/piano_roll.rs` | `PianoRollLayout`, `PianoRollViewport`, `hit_test_note`, `notes_in_marquee` (shared with Compose's expanded editor) |
| Panel chrome | `resonance-app/src/view/editor_panel.rs` | `view_midi_editor_panel`, `EditorVariant` |
| Quantize toolbar | `resonance-app/src/view/midi_quantize.rs` | `view` |
| Engine edits + quantize maths | `resonance-audio/src/quantize`, engine MIDI handlers | `quantize_notes`, `stock_grooves`, `move_note_resorted` |
| Event mirror | `resonance-app/src/engine_events/midi.rs` | `optimistic_set_notes`, `groove_extracted` |
| Frozen gate | `resonance-app/src/update/gates.rs` | `midi_editor_edit_clip`, `midi_editor_edits_open_clip` |
| Control | `resonance-app/src/update/control/notes.rs`, `update/control/import_midi.rs` | handlers for `notes.*` |

## Surfaces

- **GUI:** the bottom editor panel in Arrange (and Compose); the palette
  commands `OpenSelectedMidiClip` (`Enter`), `CloseMidiEditor` (`Cmd+Esc`),
  `DeleteSelectedNotes`, `SelectAllNotes` (`Cmd+A`) and `SelectNotesInView`.
- **Control API / MCP** (notes are addressed by `index` from `song.notes`):
  - `notes.insert` → `mcp__resonance__notes_insert`
  - `notes.edit` (pitch, start, length, velocity) → `mcp__resonance__notes_edit`
  - `notes.delete` → `mcp__resonance__notes_delete`
  - `notes.insert_many` (one undo entry) → `mcp__resonance__notes_insert_many`
  - `notes.replace_all` (needs `confirm` on a clip that already has notes) → `mcp__resonance__notes_replace_all`
  - `notes.create_clip` → `mcp__resonance__notes_create_clip`
  - `notes.move_clip` → `mcp__resonance__notes_move_clip`
  - `notes.import_midi` → `mcp__resonance__notes_import_midi`
  - Reads go through `song.notes` (`../control-api/`).
  - There is **no** quantize, humanize or groove method. An agent quantizes
    by rewriting the notes.
- **Agent skills:** `arranging` (`notes_insert_many`, `notes_replace_all`,
  `notes_edit`, `notes_delete`, `notes_import_midi`) and `drumming`.

## How it's tested

| What | Where | Run |
|---|---|---|
| Selection model, marquee | `resonance-app/tests/midi/{midi_editor_selection,midi_marquee}.rs` | `./scripts/run-tests.py -p resonance-app` |
| Drag re-tracking | `resonance-app/tests/midi/note_drag_reorder.rs` | same |
| Quantize/humanize/groove handlers + undo | `resonance-app/tests/midi/{midi_quantize_handlers,midi_groove_ui,midi_quantize_panel,midi_bulk_edit_mirror}.rs` | same |
| Grid/ghost geometry | `resonance-app/tests/midi/midi_quantize_overlay.rs` | same |
| Key focus (Delete hits the right surface) | `resonance-app/tests/timeline/editor_key_focus.rs` | same |
| Groove library + settings persistence | `resonance-app/tests/timeline/quantize_persistence.rs` | same |
| `notes.*` control | `resonance-app/tests/control/{control_notes,control_notes_bulk,control_notes_move_clip,control_notes_read_your_writes,control_notes_create_insert_race,control_notes_confirm_caps,control_import_midi}.rs` | same |
| Engine bulk edits / quantize maths | `resonance-audio/tests/engine/midi_bulk_edits.rs`, `resonance-audio/tests/types/quantize_engine.rs` | `./scripts/run-tests.py -p resonance-audio` |

**Not covered:**

- There is no golden image of the piano roll or the quantize toolbar.
- Nothing checks the undo granularity of a pointer drag, which is
  per-event; see Known gaps.
- Audible note preview.

## Decisions and deviations

- Notes are addressed by index, not by a stable id. The control API
  documents this ("re-read after edits that reorder notes").
- Built-in grooves live in code (`stock_grooves`) and are referenced by
  index or name, so they are never copied into the project file.
- `QuantizeState::settings` ("last-used quantize settings", todo #395) is
  saved and restored with the project, but nothing writes or reads it: the
  toolbar uses `MidiQuantizePanelState` defaults. The persistence test only
  covers the round trip.
- `RemoveSelectedNotes` sends one `RemoveMidiNote` per note, highest index
  first, rather than a single bulk command. It is still one undo entry.

## Known gaps

- Moving or resizing a note by drag records **one undo entry per pointer-move
  event**. `MoveNote` and `ResizeNote` are `UndoAction::Record` and are sent
  on every `CursorMoved`:
  [piano-roll-drag-undo](../../intents/timeline/piano-roll-drag-undo.md).
- The velocity lane can't be edited, there is no zoom, no snap picker, and
  no way to create a MIDI clip in Arrange:
  [piano-roll-editing-controls](../../intents/timeline/piano-roll-editing-controls.md).
- The quantize toolbar doesn't remember its last settings:
  [quantize-remember-settings](../../intents/timeline/quantize-remember-settings.md).
- Quantize, humanize and groove are not available to agents.
