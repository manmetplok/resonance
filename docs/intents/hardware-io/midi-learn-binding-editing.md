# MIDI Learn: binding editing and more targets

> Status: **partly built**
> Area: hardware-io
> Source: code reading for [MIDI Learn](../../features/hardware-io/midi-learn.md); architecture doc #167.
> Register items covered: FU-W1a, FU-W1b, FU-W1g (review 2026-10-02).

## Why

- The binding model supports a sub-range, inversion, soft takeover (Pickup,
  Scale) and relative encoders, but nothing edits them: every learned
  binding is absolute, full range, Jump. An endless encoder learned by
  moving it is read as absolute and jumps.
- Transport targets (Play, Stop, Record, Loop) can be bound only over the
  API or from a hand-edited controller map; the transport buttons have no
  "Learn MIDI" menu. Bus and master faders cannot be learned at all.
- An agent cannot choose the control-surface port, load or save a
  controller map, or bind a known CC without the user moving it.

## What it would do

- A binding row in Settings → MIDI expands to edit min/max, invert,
  takeover and CC mode (absolute / three relative encodings).
- Learn on the transport buttons, bus faders/pans and the master fader
  (new `MidiTarget` variants for bus/master).
- Control methods: `midi_map.set_binding` (explicit source, no learn),
  `midi_map.update` (range/mode/takeover), `midi_map.maps` /
  `load_map` / `save_map`, and setting the control-surface port.

## What exists to build on

- `resonance_common::midi_map` has every field and all the math
  (`cc_to_norm`, `decode_relative`, `apply_delta`, `takeover_value`), tested
  in `resonance-common/tests/midi_map.rs`.
- The app already applies range, mode and takeover in
  `update/midi_map.rs::continuous`.
- `learnable(...)` in `view/midi_learn.rs` wraps any control; the menu and
  outline come with it.

## Open decisions

1. Auto-detect relative encoders during learn (look at the first few
   values)? Recommended: no; offer the mode in the editor, default absolute.
2. Bus/master targets: extend `MidiTarget` or map onto automation targets?
   Recommended: new variants mirroring `AutomationTarget::{BusGain, BusPan,
   MasterGain}`.

## Slices

1. Binding editor in Settings → MIDI (+ undo, golden).
2. Learn wrappers on transport buttons; bus/master targets end to end.
3. Control methods for explicit binding, editing, maps and the port.

## How we'd know it works

App tests in `resonance-app/tests/midi/` for editing and the new targets;
control tests; a live session with an encoder in relative mode and a
fader with Pickup.
