//! Hardware audio input device list and the OS default (ARCH-06 A6-2).
//!
//! Held as a sub-struct on [`state::DeviceState`](crate::state::DeviceState)
//! (A-12f) — itself a field on [`Resonance`](crate::Resonance) — so handlers
//! that only care about audio input configuration can take
//! `&InputDevices` / `&mut InputDevices` instead of the whole app.

use resonance_audio::types::InputDeviceInfo;

/// Hardware audio input ports advertised by the OS, refreshed from
/// engine device-list events, plus the OS-reported default input's name
/// (used to preselect a device in pickers).
///
/// Named `devices`/`default_name` rather than `input_devices`/
/// `default_input_device_name` to avoid stuttering through this
/// struct's own name (`r.devices.input.devices`, not
/// `r.devices.input.input_devices`) — the one field pair in this batch
/// where keeping the original names verbatim reads worse than
/// shortening them.
#[derive(Debug, Clone, Default)]
pub struct InputDevices {
    /// Hardware audio input ports advertised by the OS.
    pub devices: Vec<InputDeviceInfo>,
    /// The OS-reported default input device's name, if any.
    pub default_name: Option<String>,
}
