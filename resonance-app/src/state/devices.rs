//! Hardware/external-device state (ARCH-06 second tier, A-12f survey): the
//! audio input and MIDI device lists, the device-definition registry,
//! per-track external-instrument config, and the MIDI-learn binding mirror
//! — previously five loose fields on [`Resonance`](crate::Resonance).
//!
//! Grouped together per the A-12 survey
//! (`docs/design/A-12-resonance-fields.md`): all five are the
//! hardware/external-device domain. `input_devices` and `midi_devices` are
//! themselves already sub-states (A-12a); nested here they'd stutter as
//! `r.devices.input_devices` / `r.devices.midi_devices`, so the fields are
//! renamed `input` / `midi` on the way in — the types keep their names,
//! only the field on [`DeviceState`] is shortened. `device_registry`
//! becomes `registry` for the same reason. `external_instruments` and
//! `midi_map` read fine as-is (`r.devices.external_instruments`,
//! `r.devices.midi_map`) and keep their names.

use crate::state;

/// Hardware device lists, the device-definition registry, per-track
/// external-instrument config, and the MIDI-learn binding mirror.
#[derive(Debug, Clone, Default)]
pub struct DeviceState {
    /// Hardware audio input device list and the OS default (ARCH-06 A6-2).
    /// See `state::InputDevices`.
    pub input: state::InputDevices,
    /// Hardware MIDI device lists and clock sync settings (ARCH-06 A6-2).
    /// See `state::MidiDevices`.
    pub midi: state::MidiDevices,
    /// Device-definition registry (epic #40, doc #201 §2): the bundled
    /// device presets plus any user-authored ones, scanned once at startup.
    /// The External-Instrument inspector's device-preset picker reads
    /// `list()`; selecting a preset resolves its `params` (via `get(id)`)
    /// into the `SetTrackDeviceParams` command. Read-only after
    /// construction (a rescan/reload is a later todo).
    pub registry: resonance_common::DeviceDefinitionRegistry,
    /// External-instrument tracks: per-track bank/program/latency config plus
    /// runtime device-offline flags (doc #169, epic #39). Absence means the
    /// track is a plain track. The MIDI-out / audio-return / monitor / arm
    /// fields live on the track itself; this map holds only the
    /// external-specific bits. Config (not the offline flags) round-trips
    /// undo via `ProjectTrack::external_instrument` in the snapshot's file.
    pub external_instruments: crate::state::ExternalInstrumentMap,
    /// MIDI Learn / hardware control-surface mapping, mirrored from the
    /// engine's active binding set. A pure projection of `MidiBinding*` /
    /// `ControlSurface*` events — see `state::MidiMapState`.
    pub midi_map: state::MidiMapState,
}
