//! The passes that run once over the whole callback buffer after the
//! arrangement has been rendered: master FX, metronome, master volume and
//! the A/B mix meter.

use std::sync::atomic::Ordering;

use indexmap::IndexMap;
use parking_lot::{Mutex, RwLockReadGuard};

use crate::clap_host::SyncClapInstance;
use crate::engine::AutomationSnapshot;
use crate::mixer::automation_apply::auto_master_volume;
use crate::mixer::click::render_metronome_clicks;
use crate::mixer::master::{apply_master_fx_chain, apply_master_volume_and_peaks};
use crate::types::*;

use super::context::{BlockTiming, CallbackInputs, CallbackScratch};
use super::seam::Seam;

/// What the master passes need from the block that just rendered.
pub(super) struct MasterTail<'a> {
    /// Taken by value so the read lock is released at exactly the point
    /// the FX chain no longer needs it — before the click and volume
    /// passes, which is a meaningful window on the realtime thread.
    pub(super) plugins_guard:
        RwLockReadGuard<'a, IndexMap<PluginInstanceId, Mutex<SyncClapInstance>>>,
    pub(super) sidechain_routes: &'a [SidechainRoute],
    pub(super) automation: &'a AutomationSnapshot,
    /// The block's total compensation latency, for the comp-delayed
    /// master-gain evaluation.
    pub(super) max_latency: u64,
    pub(super) playhead: u64,
    pub(super) frames: usize,
    pub(super) seam: Option<Seam>,
}

pub(super) fn run_master_passes(
    inputs: &CallbackInputs<'_>,
    scratch: &mut CallbackScratch<'_>,
    timing: &BlockTiming<'_>,
    tail: MasterTail<'_>,
) {
    let shared = inputs.shared;
    let channels = inputs.channels;

    // Master FX chain: run over the full callback buffer post-bus-sum,
    // before the metronome click is layered in and before the master volume
    // pass. Skipped when globally bypassed.
    if !shared.master_fx_bypassed.load(Ordering::Relaxed) {
        apply_master_fx_chain(
            scratch.data,
            channels,
            inputs.master,
            &tail.plugins_guard,
            scratch.track_buf_l,
            scratch.track_buf_r,
            timing.transport,
            tail.sidechain_routes,
            scratch.sidechain,
        );
    }

    drop(tail.plugins_guard);

    // Metronome click synthesis. When a loop seam split the callback, the
    // mapping from output frame index to timeline frame changes at the
    // seam: frames before `head_frames` play from `playhead`, frames after
    // play from `loop_in`.
    if timing.metronome {
        render_metronome_clicks(
            scratch.data,
            channels,
            inputs.sample_rate,
            timing.map,
            timing.bpm,
            timing.num,
            tail.frames,
            tail.playhead,
            tail.seam.map(Seam::as_ranges),
        );
    }

    // Apply master volume, hard clip, and compute master peak levels.
    // A master-gain automation lane (evaluated at the buffer's end frame)
    // overrides the static fader; the pass ramps from the previous block's
    // value so the sweep stays click-free. Evaluated at the comp-delayed
    // position: the mix reaching master is max_latency() behind the raw
    // playhead, so a drawn master move lands on the audio it was drawn
    // against (doc #260 finding #9).
    let master_eval = (tail.playhead + tail.frames as u64).saturating_sub(tail.max_latency);
    let auto_master = auto_master_volume(tail.automation, master_eval);
    apply_master_volume_and_peaks(scratch.data, channels, shared, auto_master);

    // Meter the processed mix (post master FX + volume) for the A/B panel.
    let metered = &scratch.data[..tail.frames * channels];
    scratch
        .ab_meters
        .mix
        .feed_interleaved(metered, channels, tail.frames);
    shared.mix_meter.store(&scratch.ab_meters.mix.snapshot());
}
