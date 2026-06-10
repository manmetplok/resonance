//! Per-block timeline rendering for the live audio callback: a thin
//! wrapper over [`render_core::render_block`] with the
//! [`RenderStrategy::Live`] policy (non-blocking plugin locks with the
//! MIDI stash fallback, transport latching, monitor-input mixing,
//! gain ramps, and VU peak metering).
//!
//! Called once per buffer in the no-seam path, twice (head + tail)
//! when a buffer crosses a loop boundary. Allocation-free.

use indexmap::IndexMap;

use crate::clap_host::SyncClapInstance;
use crate::latency::LatencyComp;
use crate::types::*;

use super::common::TransportSnap;
use super::midi_stash::MidiStash;
use super::render_core::{render_block, RenderStrategy};

/// Render one contiguous timeline sub-block into a slice of the output.
/// Separated from `mix_audio` so that a buffer which crosses the loop seam
/// can be rendered as two sub-blocks (pre-wrap and post-wrap) with different
/// `playhead` values, giving sample-accurate cycle playback.
///
/// The caller is responsible for:
/// - Passing `data` sliced to exactly `frames * channels` samples.
/// - Passing `monitor_temp` sliced to the corresponding portion of this
///   callback's live input (monitor is timeline-independent — it streams
///   linearly across the full callback, not per sub-block's playhead).
/// - Clearing the output buffer before the first call.
/// - Running the metronome and master-volume passes once over the full
///   callback buffer afterwards.
#[allow(clippy::too_many_arguments)]
pub(super) fn render_timeline_block(
    data: &mut [f32],
    channels: usize,
    tracks_guard: &IndexMap<TrackId, Track>,
    busses_guard: &IndexMap<BusId, Bus>,
    clips_guard: &[AudioClip],
    midi_clips_guard: &[MidiClip],
    plugins_guard: &IndexMap<PluginInstanceId, parking_lot::Mutex<SyncClapInstance>>,
    tempo_map: &TempoMap,
    sample_rate: u32,
    any_solo: bool,
    active_busses: usize,
    playhead: u64,
    frames: usize,
    track_buf_l: &mut [f32],
    track_buf_r: &mut [f32],
    bus_bufs: &mut [(Vec<f32>, Vec<f32>)],
    port_scratch: &mut [(Vec<f32>, Vec<f32>)],
    note_event_buf: &mut Vec<PendingNoteEvent>,
    midi_stash: &mut MidiStash,
    monitor_temp: &[f32],
    monitor_frames: usize,
    input_channels: usize,
    transport_snap: Option<TransportSnap>,
    latency_comp: &LatencyComp,
) {
    let mut strategy = RenderStrategy::Live {
        midi_stash,
        transport_snap,
        monitor_temp,
        monitor_frames,
        input_channels,
    };
    render_block(
        data,
        channels,
        tracks_guard,
        busses_guard,
        clips_guard,
        midi_clips_guard,
        plugins_guard,
        tempo_map,
        sample_rate,
        any_solo,
        active_busses,
        playhead,
        frames,
        track_buf_l,
        track_buf_r,
        bus_bufs,
        port_scratch,
        note_event_buf,
        latency_comp,
        &mut strategy,
    );
}
