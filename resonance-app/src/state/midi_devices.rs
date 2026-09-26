//! Hardware MIDI device lists and MIDI clock sync settings (ARCH-06 A6-2).
//!
//! Held as a sub-struct on [`Resonance`](crate::Resonance) so handlers
//! that only care about MIDI I/O configuration can take
//! `&MidiDevices` / `&mut MidiDevices` instead of the whole app.

use resonance_audio::MidiDeviceInfo;

/// Hardware MIDI input/output port lists (refreshed periodically from
/// `Tick` so hot-plugged devices appear) plus the MIDI clock master/slave
/// sync settings, which persist in the project file.
#[derive(Debug, Clone)]
pub struct MidiDevices {
    /// Hardware MIDI input ports advertised by the OS. Refreshed
    /// periodically from `Tick` so hot-plugged devices appear.
    pub midi_input_devices: Vec<MidiDeviceInfo>,
    /// Hardware MIDI output ports.
    pub midi_output_devices: Vec<MidiDeviceInfo>,
    /// Wall-clock instant of the last MIDI device list refresh.
    pub midi_devices_last_refresh: std::time::Instant,
    /// Whether MIDI clock master output is enabled.
    pub midi_clock_send_enabled: bool,
    /// Hardware MIDI output port carrying the master clock.
    pub midi_clock_send_device: Option<String>,
    /// Whether MIDI clock slave (input) is enabled.
    pub midi_clock_recv_enabled: bool,
    /// Hardware MIDI input port carrying the master clock.
    pub midi_clock_recv_device: Option<String>,
}

impl Default for MidiDevices {
    fn default() -> Self {
        Self {
            midi_input_devices: Vec::new(),
            midi_output_devices: Vec::new(),
            midi_devices_last_refresh: std::time::Instant::now(),
            midi_clock_send_enabled: false,
            midi_clock_send_device: None,
            midi_clock_recv_enabled: false,
            midi_clock_recv_device: None,
        }
    }
}
