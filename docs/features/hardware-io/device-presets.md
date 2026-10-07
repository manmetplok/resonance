# Device presets and hardware parameter automation

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: `docs/intents/prototypes/device-presets-midi-automation/index.html` (epic #40, architecture doc #201). Leads: ba todos #718, #723.

A device preset (device definition) describes one hardware synth model: its
parameters with their CC or NRPN addresses and ranges, and its named
patches. Picking one on an external-instrument track gives the patch picker
real names and makes the synth's parameters automatable from the timeline;
the lanes are sent to the synth over its MIDI output during playback and
during a realtime bounce.

## Behaviour

### Definitions

- A `DeviceDefinition` has an `id`, manufacturer, model, schema version, a
  list of `DeviceParam` (id, name, optional group, binding, integer
  `min`/`max`/`default`, curve) and a list of `PatchEntry` (bank MSB/LSB,
  program, name, optional category).
- Bindings: `Cc { cc }`, `Nrpn { msb, lsb, fourteen_bit }`,
  `Rpn { msb, lsb, fourteen_bit }`. Curves: Linear, Exponential,
  Logarithmic. `lane_value_to_binding_value` / `binding_value_to_lane` are
  the one mapping between a normalized lane value and the integer sent.
- **Bundled:** one definition, Moog Muse (`moog-muse.json`, 102 params,
  256 patches), compiled into `resonance-common`
  (`BUNDLED_DEFINITION_JSON`).
- **User:** every `.json` file in `<data dir>/resonance/device_definitions/`
  (`user_definitions_dir`, e.g. `~/.local/share/resonance/device_definitions`
  on Linux). A user file with a bundled id replaces it (last wins by id).
  Invalid files are skipped and recorded as scan errors.
- Scanned once at startup (bundled only in tests). The inspector offers
  "Reveal folder" (`RevealUserDefinitionsFolder`, creates the
  folder) and "Re-scan" (`RescanDefinitions`), which rebuild the list without
  a restart.
- There is no in-app editor; definitions are written by hand.

### On a track

- The inspector's device-preset picker stores the id on the track's
  external config (`device_id`, saved in the project, undoable) and sends the
  definition's params to the engine (`SetTrackDeviceParams`); the engine
  confirms with `TrackDeviceParamsApplied`. Clearing the picker sends an
  empty set.
- **Named patches:** the Patch card lists the definition's patches; picking
  one sets bank and program together (one Bank Select + Program Change).
- **Automation:** device parameters appear in the timeline's automation-lane
  picker as `AutomationTarget::DeviceParam { track, param_id }` lanes
  (drawing and editing lanes is the timeline area). The engine thread
  evaluates every enabled device lane on each iteration (~16 ms) and sends a CC or
  NRPN only when the integer value changed (`emit_device_param_automation`),
  so a slow sweep does not flood the port. The realtime bounce runs the
  same function, so live and bounce send the same messages.
- RPN-bound params are accepted in a definition but never sent (the emitter
  skips `Rpn`).
- Offline renders have no hardware output and send nothing.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Model + mapping | `resonance-common/src/device_definition.rs` | `DeviceDefinition`, `DeviceParam`, `MidiBinding`, `ParamCurve`, `PatchEntry`, `lane_value_to_binding_value` |
| Registry | `resonance-common/src/device_registry.rs`, `resonance-common/bundled/device_definitions/` | `DeviceDefinitionRegistry::{scan_bundled, scan_dir, list}`, `user_definitions_dir` |
| Engine params | `resonance-audio/src/engine/midi/hardware.rs` | `handle_set_track_device_params`, `set_track_device_params_in_place` |
| Emission | `resonance-audio/src/engine/midi/outbound.rs`, `midi_hardware.rs` | `emit_device_param_automation`, `poll_device_param_automation`, `MidiOutputRegistry::send_nrpn`, `encode_nrpn` |
| App | `resonance-app/src/update/external_instrument.rs`, `state/devices.rs`, `state/ui_caches.rs` | `SetDevice`, `SetPatch`, `RescanDefinitions`, `DeviceState::registry`, `rebuild_device_choices` |
| View | `resonance-app/src/view/mixer/inspector/external_instrument.rs` | device picker, patch card |

## Surfaces

- **GUI:** Mix inspector → TRACK → EXTERNAL INSTRUMENT → device preset,
  patch by name, definitions folder / re-scan; Arrange automation-lane
  picker lists the device's params.
- **Control API / MCP:** no method selects a device preset or names a
  patch. Device lanes are listed and written through the automation
  methods (`automation.lanes` / `set_lane` / `add_points` …, target kind
  `device`); see [../timeline/automation-control-api.md](../timeline/automation-control-api.md).

## How it's tested

| What | Where | Run |
|---|---|---|
| Model, validation, mapping | `resonance-common/tests/device_definition.rs` | `./scripts/run-tests.py -p resonance-common` |
| Registry scan, last-wins, errors | `resonance-common/tests/device_registry.rs`, `bundled_moog_muse.rs` | same |
| Engine params, CC/NRPN emission | `resonance-audio/tests/engine/device_params_handler.rs`, `resonance-audio/tests/midi_hw/device_param_automation.rs`, `midi_hardware_emit.rs` | `./scripts/run-tests.py -p resonance-audio` |
| App: preset picker, persistence, patch by name, rescan | `resonance-app/tests/plugins/external_instrument_device_preset.rs`, `external_instrument_device_persistence.rs`, `external_instrument_patch_by_name.rs`, `resonance-app/tests/io/user_definitions_rescan.rs`; golden `external_instrument_named_patch_picker-wgpu.png` | `./scripts/run-tests.py -p resonance-app` |
| Device lanes in the timeline | `resonance-app/tests/timeline/automation_device_params.rs`, `timeline_automation_device_lanes.rs` | same |

Not covered: sending to a real synth; RPN (not built).

## Decisions and deviations

Compared with the prototype:

- Built: bundled + user presets, named patch picker, device-param lanes
  that emit CC/NRPN live and during the realtime bounce, change-only
  emission.
- Not built: the definition editor (duplicate/edit, add params and patches),
  "Validate" and `.rdev` import/export; definitions saved alongside the
  project (user definitions are per machine, so a project that uses a user
  definition does not carry it); the "raw CC by number" lanes for a track
  without a preset; the "patch number not named" hint; the Prophet-6 and
  Minilogue presets shown in the mock.
- Files are `.json`, not `.rdev`.

## Known gaps

- Editor, project-local definitions, raw-CC lanes, RPN:
  [device-definition-editor](../../intents/hardware-io/device-definition-editor.md).
