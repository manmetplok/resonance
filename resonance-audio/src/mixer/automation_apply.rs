//! Per-block application of parameter-automation lanes inside the shared
//! render core (architecture doc #162 §2, epic #14).
//!
//! The engine thread publishes an [`AutomationSnapshot`] of every enabled
//! lane; this module turns that snapshot into the concrete values the
//! render block applies:
//!
//! - **Gain / pan** lanes become per-block stereo-gain *ramp endpoints*
//!   (`(start, end)` per channel), evaluated at the block's first and
//!   next-block-start frames so the existing per-sample ramp
//!   ([`super::common::ramped_gain`]) produces a click-free sweep — the
//!   same machinery a fader drag already uses. Evaluating at the block
//!   boundaries (not just the start) means the offline bounce, which
//!   doesn't carry a "previous block gain", still ramps smoothly.
//! - **Mute** lanes become a per-block boolean, OR-ed into the track's
//!   static mute so the existing mute fade-out applies.
//! - **Plugin-param** lanes are queued onto the plugin instance via
//!   `set_param` right before it processes the block — the same path as
//!   `SetPluginParam`.
//!
//! Everything here is allocation-free: lookups are `HashMap::get`, value
//! sampling is a binary search ([`resonance_common::sample_lane`]).

use resonance_common::{lane_value_to_plugin_param, AutomationTarget, PluginInstanceId};

use crate::clap_host::SyncClapInstance;
use crate::engine::AutomationSnapshot;

/// Convert a gain lane's decibel value to a linear coefficient. A value
/// at (or below) the lane floor maps to exact silence so a full fade-out
/// lands on zero rather than `-60 dBFS` of residue.
#[inline]
fn db_to_lin(db: f32) -> f32 {
    if db <= resonance_common::GAIN_MIN_DB {
        0.0
    } else {
        10f32.powf(db / 20.0)
    }
}

/// Stereo gains for an automated `(volume, pan)` pair — the same balance
/// law [`super::common::track_stereo_gains`] applies to the static
/// values, so a pan lane and a pan knob mean the same thing.
#[inline]
fn stereo_gains(volume: f32, pan: f32) -> (f32, f32) {
    let (pan_l, pan_r) = resonance_dsp::stereo_balance(pan);
    (volume * pan_l, volume * pan_r)
}

/// Resolve the automated stereo-gain ramp for one target across a block,
/// or `None` when neither a gain nor a pan lane targets it (caller then
/// uses the static gains). Returns `((gain_l_start, gain_l_end),
/// (gain_r_start, gain_r_end))`.
///
/// `static_volume` / `static_pan` fill in whichever of the two lanes is
/// absent, so automating only gain keeps the static pan (and vice-versa).
#[inline]
pub fn auto_gain_ramp(
    snap: &AutomationSnapshot,
    gain_target: AutomationTarget,
    pan_target: AutomationTarget,
    static_volume: f32,
    static_pan: f32,
    start: u64,
    end: u64,
) -> Option<((f32, f32), (f32, f32))> {
    let gain_lane = snap.mix_lanes.get(&gain_target);
    let pan_lane = snap.mix_lanes.get(&pan_target);
    if gain_lane.is_none() && pan_lane.is_none() {
        return None;
    }
    let volume_at = |frame: u64| {
        gain_lane
            .map(|l| db_to_lin(l.real_value_at(frame)))
            .unwrap_or(static_volume)
    };
    let pan_at = |frame: u64| pan_lane.map(|l| l.real_value_at(frame)).unwrap_or(static_pan);
    let (gl_start, gr_start) = stereo_gains(volume_at(start), pan_at(start));
    let (gl_end, gr_end) = stereo_gains(volume_at(end), pan_at(end));
    Some(((gl_start, gl_end), (gr_start, gr_end)))
}

/// The VOLUME half of [`auto_gain_ramp`]: linear gain at the block's two
/// evaluation frames, with no pan law applied.
///
/// Used for the group trim a multi-output instrument's parent fader
/// applies to its sub-track taps (ba doc #275 P1.1). Pan is deliberately
/// left out — the taps sum to master individually, each with its own pan,
/// so there is nothing left for the parent's pan to act on.
#[inline]
pub fn auto_volume_ramp(
    snap: &AutomationSnapshot,
    gain_target: AutomationTarget,
    static_volume: f32,
    start: u64,
    end: u64,
) -> (f32, f32) {
    match snap.mix_lanes.get(&gain_target) {
        Some(lane) => (
            db_to_lin(lane.real_value_at(start)),
            db_to_lin(lane.real_value_at(end)),
        ),
        None => (static_volume, static_volume),
    }
}

/// Automated master volume (linear) at `frame`, or `None` when no
/// master-gain lane is set (caller uses the static master volume). The
/// master pass ramps from the previous block's volume to this value, so
/// successive blocks chain into a smooth sweep.
#[inline]
pub fn auto_master_volume(snap: &AutomationSnapshot, frame: u64) -> Option<f32> {
    snap.mix_lanes
        .get(&AutomationTarget::MasterGain)
        .map(|l| db_to_lin(l.real_value_at(frame)))
}

/// Automated mute state at `frame`, or `None` when no mute lane targets
/// it (caller falls back to the static mute flag).
#[inline]
pub fn auto_muted(
    snap: &AutomationSnapshot,
    mute_target: AutomationTarget,
    frame: u64,
) -> Option<bool> {
    snap.mix_lanes
        .get(&mute_target)
        .map(|l| l.real_value_at(frame) >= 0.5)
}

/// Queue every plugin-param lane targeting `instance` onto `inst`,
/// sampled at `frame` and mapped into the plugin's own range. A no-op
/// when no param lane targets the instance. Must be called while holding
/// the plugin lock, before `process()`.
#[inline]
pub(crate) fn apply_plugin_params(
    inst: &mut SyncClapInstance,
    snap: &AutomationSnapshot,
    instance: PluginInstanceId,
    frame: u64,
) {
    if let Some(params) = snap.plugin_params.get(&instance) {
        for p in params {
            let real = lane_value_to_plugin_param(p.lane.sample(frame), p.min, p.max);
            inst.0.set_param(p.param_id, real);
        }
    }
}
