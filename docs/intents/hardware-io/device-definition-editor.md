# Device definition editor and portability

> Stage: **intent**
> Status: **partly built**
> Area: hardware-io
> Source: `docs/intents/prototypes/device-presets-midi-automation/index.html` (epic #40, doc #201) compared with [device presets](../../features/hardware-io/device-presets.md).

## Why

- Only the Moog Muse ships. Any other synth needs a hand-written JSON file
  in the user data folder, with no validation feedback in the app beyond a
  skipped file.
- User definitions live on the machine, not with the project. A project
  that uses one opens on another machine with its device id pointing at
  nothing.
- A track without a definition cannot automate a raw CC by number, which the
  design offered for any synth.
- RPN-bound parameters are accepted but never sent
  (`emit_device_param_automation` skips `MidiBinding::Rpn`).
- No control method selects a device preset or a named patch.

## What it would do

- An editor: duplicate a definition, edit params (name, group, CC/NRPN/RPN
  binding, range, curve) and patch banks, validate, save.
- Copy the definitions a project uses into the project folder on save, and
  prefer them on load.
- "CC by number" automation lanes for any external track.
- Send RPN like NRPN.
- `external.set_device` and patch-by-name in `external.set_patch`.

## What exists to build on

- `DeviceDefinition::validate`, `DeviceDefinitionRegistry::save_to_path`
  (`resonance-common/src/device_registry.rs`), unused by the app today.
- `RescanDefinitions` / `RevealUserDefinitionsFolder` in
  `resonance-app/src/update/external_instrument.rs`.
- `encode_nrpn` in `resonance-audio/src/midi_hardware.rs`; RPN differs only
  in the controller numbers (101/100 instead of 99/98).

## Open decisions

1. Project-local definitions: copy on save, or reference by content hash?
   Recommended: copy the JSON into `<project>/device_definitions/`.
2. Raw CC lanes: a new `AutomationTarget` variant or a synthetic
   definition? Recommended: a synthetic "Generic CC" definition, so lanes
   stay `DeviceParam`.

## Slices

1. RPN emission (engine + test in `device_param_automation.rs`).
2. Control: `external.set_device`, patch by name.
3. Project-local copies of used definitions.
4. Generic CC definition.
5. Editor UI.

## How we'd know it works

Tests per slice; a project using a user definition opens on a clean data
directory with names and lanes intact.
