# MIDI Learn and controller mapping

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: none in `docs/history/specs/`. Leads: architecture doc #167 (§1–§3, W1), epic #21.

Map knobs, faders, encoders and pads on a hardware controller to mixer
controls, plugin parameters and transport buttons. Right-click a control,
choose "Learn MIDI", move a knob: the binding is made. Bindings belong to
the project and undo like any edit; a set of bindings can be saved as a
controller map and loaded into other projects. A hardware move takes the
same path as a mouse drag, so it lands in undo and automation the same way.

## Behaviour

### The control-surface port

- One MIDI input port, chosen in Settings → MIDI → CONTROL SURFACE, feeds
  every binding. It is a per-machine user setting
  (`AppSettings::midi.control_surface_input`), not project state.
- With no port chosen, learn captures nothing and no binding fires. A port
  that is not plugged in is remembered and opened when it appears (the 2 s
  MIDI poll reconciles it).
- This port is separate from the per-track MIDI inputs
  ([midi-hardware-io.md](midi-hardware-io.md)).

### Learning

- Learnable in the GUI: track fader, pan, mute and solo on the mixer strip
  (right-click), every non-hidden, writable parameter in a plugin's generic
  parameter window (right-click a row), and the Mix inspector's TRACK →
  MIDI CONTROL picker ("Learn MIDI for…"), which also lists the track's aux
  sends and its plugins' parameters.
- Transport targets (Play, Stop, Record, Loop) exist but have no GUI learn
  entry; they come from `midi_map.learn` or a controller map. Bus and
  master faders are not learnable.
- Arming learn outlines the control (the fader badge reads `LEARN`); Esc,
  "Cancel learn" or arming the same target again disarms.
- The engine captures the first CC or the first note-on from the port
  (`engine::midi_map`). A CC is always captured as **absolute**. The new
  binding gets the full 0–1 range, no inversion and "jump" takeover
  (`MidiBinding::new`).
- One physical control drives one target: binding a control removes any
  other binding on the same CC or note, in the app and the engine.

### What a move does

The engine only matches: it sends `ControlSurfaceMoved { binding, value }`
and the app turns it into the same message a mouse edit sends
(`update::midi_map::move_message`), so undo coalescing, the dirty flag and
automation write apply.

- **Continuous** (volume, pan, send level, plugin parameter): absolute CC
  maps 0–127 onto the binding's `min..max` (optionally inverted) and goes
  through the takeover rule: Jump, Pickup (wait until within ~1.5 CC steps
  of the current value) or Scale (move half-way each message). A relative
  CC (two's complement, signed bit or binary offset) adds a delta, scaled so
  127 counts cover the range. Volume and send level use the fader's dB
  curve; a stepped plugin parameter is rounded.
- **Toggles** (mute, solo, loop): a note flips on every note-on; a CC switch
  sets on at 64 and above, off below.
- **Triggers** (play, stop, record): fire on note-on or a CC ≥ 64.
- Note-offs are dropped. A binding whose target is gone does nothing.
- Range, inversion, takeover and relative mode are honoured but can only be
  set by editing a controller-map file; neither the GUI nor the control API
  edits them.

### Bindings list and controller maps

- Settings → MIDI lists every binding (target name and source, e.g. `CC7`)
  with a delete button and "Clear all".
- "Save bindings as map" writes the current bindings under a name to
  `<data dir>/resonance/controller_maps.json` (replacing a map of that name).
  Saved maps can be loaded (replaces the project's bindings, one undo entry)
  or deleted. Maps refer to tracks and plugins by id, so they fit projects
  built the same way; transport bindings fit any project.
- Project bindings are saved as `midi_bindings` in `project.json` and sent
  to the engine wholesale on load and undo (`SetControllerMap`).

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Model + mapping math | `resonance-common/src/midi_map.rs` | `MidiBinding`, `ControlSource`, `CcMode`, `RelativeEnc`, `MidiTarget`, `Takeover`, `ControllerMap`, `cc_to_norm`, `decode_relative`, `apply_delta`, `takeover_value` |
| Engine match + learn | `resonance-audio/src/engine/midi_map.rs` | `ActiveMidiMap`, `handle_set_midi_binding`, `handle_set_controller_map`, `handle_set_control_surface_input` |
| Port | `resonance-audio/src/midi_hardware.rs` | `ControlSurfaceInput`, `LiveControlEvent` |
| App mirror + apply | `resonance-app/src/state/midi_map.rs`, `update/midi_map.rs`, `engine_events/midi_map.rs` | `MidiMapState`, `MidiMapMessage`, `move_message`, `hardware_moved`, `set_all` |
| View | `resonance-app/src/view/midi_learn.rs`, `view/settings/midi.rs`, `view/mixer/inspector/midi.rs`, `view/plugin_window.rs` | `learnable`, `target_label`, `view_midi_page`, `learn_choices` |
| Control | `resonance-app/src/update/control/midi_map.rs` | target resolution |

The engine holds the live binding set and echoes every change
(`MidiBindingChanged` / `MidiBindingCleared`); the app's mirror is a
projection of those echoes plus its own optimistic edit.

## Surfaces

- **GUI:** right-click on strip fader/pan/M/S and on plugin parameter rows;
  Mix inspector → TRACK → MIDI CONTROL; Settings → MIDI (port, bindings,
  controller maps).
- **Control API / MCP:** `midi_map.bindings` → `mcp__resonance__midi_map_bindings`
  (all bindings, armed target, chosen port); `midi_map.learn` →
  `mcp__resonance__midi_map_learn` (target: `track_id` with `control`
  volume/pan/mute/solo, `send_id` or `param` [+ `plugin_id`, `occurrence`],
  or `transport` play/stop/record/loop); `midi_map.cancel_learn` →
  `mcp__resonance__midi_map_cancel_learn`; `midi_map.clear` →
  `mcp__resonance__midi_map_clear`; `midi_map.clear_all` →
  `mcp__resonance__midi_map_clear_all`. The user still has to move the
  control; the API cannot pick the port or load a controller map.

## How it's tested

| What | Where | Run |
|---|---|---|
| Mapping math | `resonance-common/tests/midi_map.rs` | `./scripts/run-tests.py -p resonance-common` |
| Engine learn/match, command plumbing | `resonance-audio/tests/midi_hw/midi_map_engine.rs`, `resonance-audio/tests/engine/midi_map_command_plumbing.rs`, `midi_hw/control_surface_parse.rs` | `./scripts/run-tests.py -p resonance-audio` |
| App learn flow, mirror | `resonance-app/tests/midi/midi_learn.rs`, `midi_map_mirror.rs` | `./scripts/run-tests.py -p resonance-app` |
| Mixer UI | `resonance-app/tests/mixer/midi_learn_ui.rs`; goldens `mixer_midi_learn_state-wgpu.png`, `mixer_midi_menu-wgpu.png`, `settings_midi_page-wgpu.png` | same |
| Control | `resonance-app/tests/control/control_midi_map.rs` | same |

Not covered: a real controller end to end.

## Decisions and deviations

- The engine matches and the app applies, so a hardware move needs no
  separate path into undo, automation or the plugin's range.
- The control-surface port is a machine setting because it names the user's
  hardware; bindings are project state.

## Known gaps

- No editing of range/invert/takeover/relative, no GUI learn for transport,
  bus or master, no API for maps or the port:
  [midi-learn-binding-editing](../../intents/hardware-io/midi-learn-binding-editing.md).
