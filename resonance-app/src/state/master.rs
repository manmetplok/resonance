//! Master-bus state (ARCH-06 A6-3, last of the A-12 batches).
//!
//! Held as a sub-struct on [`Resonance`](crate::Resonance) so handlers
//! that only care about the master bus can take `&MasterState` /
//! `&mut MasterState` instead of the whole app. Field names match
//! [`crate::state::BusState`] (`volume`, `level_l`, `level_r`,
//! `fx_bypassed`, `plugins`) since the master bus is the same shape as a
//! regular bus minus the id/name/pan/mute/routing fields that only make
//! sense for a bus that lives in `TrackRegistry`.

use crate::state::PluginSlotState;

/// The app's view of the master bus: its fader, meters, FX chain and
/// FX-chain bypass.
#[derive(Debug, Clone, Default)]
pub struct MasterState {
    pub volume: f32,
    pub level_l: f32,
    pub level_r: f32,
    /// FX plugins inserted on the master bus, rendered after every
    /// track and bus has been summed.
    pub plugins: Vec<PluginSlotState>,
    /// When true, the master FX chain is bypassed — the master fader
    /// and metering still run, but no master-bus plugins are processed.
    pub fx_bypassed: bool,
}
