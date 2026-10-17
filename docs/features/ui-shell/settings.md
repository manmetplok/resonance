# Settings

> Verified against master @ `f9438e38` on 2026-10-13.
> Also verified: the Compose section (BUG-70).
> Spec history: `docs/history/specs/command-palette.md` §8 (nav rail);
> autosave from epic #32 / doc #171; code review FU-V3b, FU-M12a.

Per-machine preferences live in one JSON file and are edited in the
**Settings** overlay (⌘, or the ☰ button in the chrome). The overlay has a
nav rail with three pages:

- **General:** project I/O, autosave, arrange, recording, audio latency,
  plugins, MIDI clock.
- **Keyboard:** the keymap editor.
- **MIDI:** control surface, MIDI Learn bindings, controller maps.

## Behaviour

**The file.**

- `settings.json` lives at `<config dir>/resonance/settings.json`, which is
  `~/.config/resonance/` on Linux.
- In a test process (`Resonance::new_for_test*`), every user dir resolves
  under a per-process temp root instead (`user_dirs::enter_hermetic`).
- Loading never blocks boot:
  - a missing file means defaults;
  - an unparsable file is quarantined (`quarantine_corrupt`) and defaults
    are used;
  - the `palette` and `keymap` sections fall back to defaults on their own,
    so one bad section never resets the others.
- Writes are atomic (`atomic_write`). I/O errors are logged and swallowed.

**Sections** (`AppSettings`, every struct `#[serde(default)]`)

| Section | Fields (default) | Edited in |
|---|---|---|
| `autosave` | `enabled` (true), `interval_secs` (30), `backup_retention` (10) | General: toggle and interval picker (15 s – 10 min). Retention has no UI. |
| `media` | `favourites`, `recent_folders` | Media browser |
| `arrange` | `follow_playhead` (true) | General: "Follow playhead" |
| `audio` | `render_threads` (`None`, so the engine picks) | No UI. `RESONANCE_RENDER_THREADS` overrides it; read once at launch. |
| `midi` | `control_surface_input` (`None`) | MIDI page |
| `palette` | `recent` (command keys) | Command palette |
| `keymap` | `preset` ("Resonance"), `overrides` | Keyboard page |

**General page**, top to bottom (`view/settings.rs::view_settings_overlay`):

- **Project:** Open, Save, Save As, then Autosave on/off with an interval.
  Changing autosave writes the file at once, and the next tick uses the new
  value.
- **Arrange:** Follow playhead.
- **Recording:** "Distinct takes per loop pass", with a one-line
  explanation. This is the **project's** `loop_record_mode`, which is
  undoable and saved in the project, not in `settings.json`.
- The General page scrolls (`GENERAL_PAGE_HEIGHT`), opening at the top;
  it outgrew a laptop-height window.
- **Compose:** "Ask again about vocal melodies" forgets the project's
  remembered answer to the chord-change question
  ([vocal lyrics and melody](../compose/vocal-lyrics-and-melody.md#chord-and-length-changes-under-a-melody-bug-70));
  dimmed when nothing is remembered, with a line saying which choice
  applies. Project state like the cycle-record mode: undoable and saved
  with the project.
- **Audio:** input, output and round-trip latency in samples and ms. Opening
  Settings sends `AudioCommand::QueryIoLatency`, and the readout refreshes
  when input devices re-enumerate.
- **Plugins:**
  - a scan summary and "Rescan Plugins" ("Scanning…" while busy);
  - each scan failure on its own line;
  - a "N plugin slot(s) in this project could not be loaded — review" link
    that reopens the missing-plugins dialog.
- **MIDI Clock:** Send MIDI Clock with an output picker, Receive with an
  input picker. The option lists come from `UiViewCaches`, with a stand-in
  entry for a configured device that is unplugged. These are allowed while
  the startup screen is up.

**Keyboard page.** See [keymap-editor.md](keymap-editor.md).

**MIDI page** (`view/settings/midi.rs`). It holds the control-surface port,
every MIDI Learn binding with a per-row clear and Clear all, and saved
controller maps. It belongs to the MIDI Learn feature; see
[midi-learn.md](../hardware-io/midi-learn.md).

**Overlay mechanics**

- `ui.mixer.settings_open` is the open flag, and the overlay is
  `Overlay::Settings`.
- A backdrop click, Close or Esc closes it. Closing also cancels any
  keymap capture or conflict.
- `SettingsTab` (General is the default) picks the page. Switching to MIDI
  reloads the saved controller maps.
- Every page shares one card width (`settings_keyboard::PAGE_WIDTH`), so
  switching tabs doesn't make the card jump.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| File and schema | `resonance-app/src/settings.rs` | `AppSettings`, `AutosaveSettings`, `ArrangeSettings`, `AudioSettings`, `MidiSettings`, `PaletteSettings`, `KeymapSettings`, `load`, `persist`, `lenient` |
| User dirs | `resonance-app/src/user_dirs.rs` | `config_dir`, `data_dir`, `enter_hermetic` |
| Overlay | `resonance-app/src/view/settings.rs` | `view_settings_overlay`, `AUTOSAVE_INTERVALS`, `wide_button`, `toggle_button` |
| Pages | `resonance-app/src/view/settings_keyboard.rs`, `view/settings/midi.rs` | `view_keyboard_page`, `view_midi_page` |
| Open/close, tabs | `resonance-app/src/update/ui.rs`, `update/keymap.rs` | `OpenSettings`, `CloseSettings`, `KeymapMsg::SetTab` |
| Autosave messages | `resonance-app/src/update/project_io/mod.rs` | `ToggleAutosave`, `SetAutosaveInterval` |

## Surfaces

- **GUI:** ⌘, (Open Settings); the ☰ chrome button.
- **Control API / MCP:** none of these settings are exposed. Plugin rescan
  is: `plugins.rescan` → `mcp__resonance__plugins_rescan`.
- **Agent skills:** none.

## How it's tested

| What | Where | Run |
|---|---|---|
| Autosave settings and their UI | `resonance-app/tests/io/autosave_settings.rs`, `io/autosave_settings_ui.rs` | `./scripts/run-tests.py -p resonance-app` |
| Latency readout | `resonance-app/tests/io/io_latency_readout.rs` | same |
| Rescan button | `resonance-app/tests/plugins/settings_plugin_rescan_button.rs` | same |
| Hermetic user dirs | `resonance-app/tests/io/hermetic_user_state.rs`, `tests/hermetic_construction.rs` | same |
| Corrupt keymap section | `resonance-app/tests/performance/shortcut_review_fixes.rs` | same |
| Goldens | `settings_overlay_autosave_row`, `settings_overlay_io_latency`, `settings_midi_page`, `keymap_settings_panel` | `RESONANCE_BLESS=1 …` |

Not covered: corrupt-file quarantine of a whole `settings.json` through the
real config dir. `AudioSettings.render_threads` is only read at launch.

## Decisions and deviations

- Settings that belong to one machine go in `settings.json`. Settings that
  belong to the song (loop-record mode, MIDI Learn bindings) go in the
  project, even when they are edited on this page.
- `settings.rs`'s module comment still says autosave is "the only section".
  The struct has seven.

## Known gaps

- Render threads and backup retention can only be changed by editing the
  file ([intent](../../intents/ui-shell/settings-unexposed-options.md)).
- There is no audio device or sample-rate picker. The engine follows the
  system (PipeWire) setup.
