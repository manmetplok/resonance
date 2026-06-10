//! Monitor input handling: de-interleave the cpal input stream, route
//! each track's chosen channel(s) into its stereo L/R pair, and run
//! the track's plugin chain on the result.
//!
//! Two callers:
//! - the timeline render loop (`render_timeline_block`) when a track
//!   has monitoring enabled and the engine is playing back;
//! - the no-playback / count-in branches in `mix_audio`, which keep
//!   every audible monitored track flowing through to the master so
//!   the performer can hear themselves.

pub(crate) use crate::limits::MAX_PLUGIN_OUTPUT_PORTS;

use indexmap::IndexMap;

use crate::clap_host::SyncClapInstance;
use crate::types::*;

use super::common::latch_transport;

/// De-interleave monitor input into track buffers and process through plugins.
/// Returns the number of frames written. `monitor_temp` is interleaved
/// multi-channel input audio (the raw stream straight from the device);
/// `input_channels` tells us how many channels are in each frame, and the
/// track's own `input_port` picks which channel(s) to route into its
/// stereo L/R pair.
#[allow(clippy::too_many_arguments)]
pub(super) fn process_monitor_track(
    track: &Track,
    monitor_temp: &[f32],
    monitor_frames: usize,
    max_frames: usize,
    input_channels: usize,
    track_buf_l: &mut [f32],
    track_buf_r: &mut [f32],
    plugins_guard: &IndexMap<PluginInstanceId, parking_lot::Mutex<SyncClapInstance>>,
    transport_snap: Option<(f64, u16, u16, bool, f64)>,
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
