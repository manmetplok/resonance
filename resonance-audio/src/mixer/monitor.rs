//! Monitor input handling: de-interleave the cpal input stream, route
//! each track's chosen channel(s) into its stereo L/R pair, and run
//! the track's plugin chain on the result.
//!
//! Used by the no-playback / count-in branches in `mix_audio` (via
//! [`mix_monitor_passthrough`]), which keep every audible monitored
//! track flowing through to the master so the performer can hear
//! themselves; the playing-back timeline path mixes monitor input
//! inside `render_core` instead.

use indexmap::IndexMap;

use crate::clap_host::SyncClapInstance;
use crate::types::*;

use super::common::{
    latch_transport, ramped_stereo_peaks, sum_to_output, track_stereo_gains, TransportSnap,
};

/// De-interleave monitor input into track buffers and process through plugins.
/// Returns the number of frames written. `monitor_temp` is interleaved
/// multi-channel input audio (the raw stream straight from the device);
/// `input_channels` tells us how many channels are in each frame, and the
/// track's own `input_port` picks which channel(s) to route into its
/// stereo L/R pair.
#[allow(clippy::too_many_arguments)]
fn process_monitor_track(
    track: &Track,
    monitor_temp: &[f32],
    monitor_frames: usize,
    max_frames: usize,
    input_channels: usize,
    track_buf_l: &mut [f32],
    track_buf_r: &mut [f32],
    plugins_guard: &IndexMap<PluginInstanceId, parking_lot::Mutex<SyncClapInstance>>,
    transport_snap: Option<TransportSnap>,
) -> usize {
    let is_mono = track.mono();
    let mix_frames = max_frames.min(monitor_frames);

    track_buf_l[..mix_frames].fill(0.0);
    track_buf_r[..mix_frames].fill(0.0);

    if input_channels == 0 {
        return mix_frames;
    }

    let port = (track.input_port() as usize).min(input_channels - 1);
    let right_port = if is_mono {
        port
    } else {
        (port + 1).min(input_channels - 1)
    };

    for f in 0..mix_frames {
        let base = f * input_channels;
        track_buf_l[f] = monitor_temp[base + port];
        track_buf_r[f] = monitor_temp[base + right_port];
    }

    // Process through plugin chain (skipped when FX are bypassed).
    if !track.fx_bypassed() {
        let plugins = track.plugins();
        for &plugin_id in plugins.iter() {
            if let Some(si) = plugins_guard.get(&plugin_id) {
                if let Some(mut inst) = si.try_lock() {
                    latch_transport(&mut inst, transport_snap);
                    inst.0.process(
                        &mut track_buf_l[..mix_frames],
                        &mut track_buf_r[..mix_frames],
                        mix_frames,
                    );
                }
            }
        }
    }

    mix_frames
}

/// Monitor pass-through for the count-in and stopped branches of
/// `mix_audio`: route every audible monitored track through its plugin
/// chain and sum it straight into the output with ramped gains and VU
/// peaks. Returns whether any track was mixed.
#[allow(clippy::too_many_arguments)]
pub(super) fn mix_monitor_passthrough(
    data: &mut [f32],
    channels: usize,
    tracks_guard: &IndexMap<TrackId, Track>,
    plugins_guard: &IndexMap<PluginInstanceId, parking_lot::Mutex<SyncClapInstance>>,
    monitor_temp: &[f32],
    monitor_frames: usize,
    input_channels: usize,
    track_buf_l: &mut [f32],
    track_buf_r: &mut [f32],
    transport_snap: Option<TransportSnap>,
) -> bool {
    let any_solo = any_top_level_solo(tracks_guard.values());
    let is_audible =
        |t: &&Track| -> bool { t.monitor_enabled() && !t.muted() && (!any_solo || t.soloed()) };
    let mut mixed_any = false;
    for track in tracks_guard.values().filter(|t| is_audible(t)) {
        mixed_any = true;
        let processed_frames = process_monitor_track(
            track,
            monitor_temp,
            monitor_frames,
            monitor_frames,
            input_channels,
            track_buf_l,
            track_buf_r,
            plugins_guard,
            transport_snap,
        );
        let (target_l, target_r) = track_stereo_gains(track);
        let (last_l, last_r) = track.last_gains();
        let gain_l = (last_l, target_l);
        let gain_r = (last_r, target_r);
        // Post-fader peak levels for VU meters, with the same ramp the
        // sum applies.
        let (peak_l, peak_r) =
            ramped_stereo_peaks(track_buf_l, track_buf_r, processed_frames, gain_l, gain_r);
        track.update_peak_l(peak_l);
        track.update_peak_r(peak_r);
        sum_to_output(
            data,
            channels,
            processed_frames,
            track_buf_l,
            track_buf_r,
            gain_l,
            gain_r,
        );
        track.set_last_gains(target_l, target_r);
    }
    mixed_any
}
