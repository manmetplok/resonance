//! Offline (Bounce) and bench entry points into the render core.
//!
//! [`render_aux_for_test`] / [`render_aux_with_comp_for_test`] render one
//! offline block and return the master output plus every bus summing
//! buffer; [`RenderBenchHarness`] is the reusable, allocation-free
//! live-strategy render loop `benches/render_path.rs` measures. Both keep
//! the `indexmap` / scratch-buffer plumbing out of the `tests/` crate.

use indexmap::IndexMap;
use parking_lot::Mutex;

use crate::clap_host::SyncClapInstance;
use crate::engine::AutomationSnapshot;
use crate::types::*;

use super::super::midi_stash::MidiStash;
use super::super::render_core::{render_block, BlockInputs, BlockScratch, RenderStrategy};
use super::super::{transport_pos_beats, MAX_MIDI_EVENTS_PER_BUFFER, MAX_PLUGIN_OUTPUT_PORTS};

/// Test-only harness around [`render_block`]: assembles the `IndexMap` /
/// scratch-buffer plumbing from plain vecs, renders one offline (Bounce)
/// block at playhead 0, and returns the interleaved-stereo master output
/// plus the per-bus summing buffers. After the call each bus buffer still
/// holds that bus's accumulated signal *before* its own fader — for a
/// return bus that is exactly the summed aux-send contribution, so an
/// integration test can assert the tap in isolation. Keeps `indexmap` and
/// the render scaffolding out of the `tests/` crate.
#[doc(hidden)]
#[allow(clippy::type_complexity)]
pub fn render_aux_for_test(
    tracks: Vec<Track>,
    busses: Vec<Bus>,
    clips: Vec<AudioClip>,
    aux_sends: Vec<AuxSend>,
    frames: usize,
    sample_rate: u32,
) -> (Vec<f32>, Vec<(Vec<f32>, Vec<f32>)>) {
    render_aux_with_comp_for_test(
        tracks,
        busses,
        clips,
        aux_sends,
        frames,
        sample_rate,
        crate::latency::LatencyComp::empty(),
        crate::engine::AutomationSnapshot::default(),
    )
}

/// [`render_aux_for_test`] with an explicit compensation table and
/// automation snapshot, so latency tests can drive the bus-stage / dry
/// delay lines and the comp-delayed automation evaluation through the
/// real render path with synthetic delays (no live CLAP plugin needed).
#[doc(hidden)]
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn render_aux_with_comp_for_test(
    tracks: Vec<Track>,
    busses: Vec<Bus>,
    clips: Vec<AudioClip>,
    aux_sends: Vec<AuxSend>,
    frames: usize,
    sample_rate: u32,
    latency: crate::latency::LatencyComp,
    automation: crate::engine::AutomationSnapshot,
) -> (Vec<f32>, Vec<(Vec<f32>, Vec<f32>)>) {
    let tracks_guard: IndexMap<TrackId, Track> = tracks.into_iter().map(|t| (t.id, t)).collect();
    let busses_guard: IndexMap<BusId, Bus> = busses.into_iter().map(|b| (b.id, b)).collect();
    let plugins_guard: IndexMap<PluginInstanceId, Mutex<SyncClapInstance>> = IndexMap::new();
    let midi_clips: Vec<MidiClip> = Vec::new();
    let tempo_map = TempoMap::default();
    let active_busses = busses_guard.len();

    let mut data = vec![0.0f32; frames * 2];
    let mut track_buf_l = vec![0.0f32; frames];
    let mut track_buf_r = vec![0.0f32; frames];
    let mut bus_bufs: Vec<(Vec<f32>, Vec<f32>)> = (0..active_busses)
        .map(|_| (vec![0.0f32; frames], vec![0.0f32; frames]))
        .collect();
    let mut port_scratch: Vec<(Vec<f32>, Vec<f32>)> = Vec::new();
    let mut note_buf: Vec<PendingNoteEvent> = Vec::new();

    let in_filter = |_id: TrackId| true;
    let fan_out_only = |_id: TrackId| false;
    let key_only = |_id: TrackId| false;
    let key_only_bus = |_id: BusId| false;
    let mut strategy = RenderStrategy::Bounce {
        in_filter: &in_filter,
        fan_out_only: &fan_out_only,
        key_only: &key_only,
        key_only_bus: &key_only_bus,
        respect_mute_solo: false,
        freeze_raw: false,
    };

    let mut sidechain = SidechainTaps::new(frames);
    render_block(
        BlockInputs {
            channels: 2,
            tracks: &tracks_guard,
            busses: &busses_guard,
            clips: &clips,
            midi_clips: &midi_clips,
            plugins: &plugins_guard,
            tempo_map: &tempo_map,
            sample_rate,
            any_solo: false,
            active_busses,
            aux_sends: &aux_sends,
            sidechain_routes: &[],
            playhead: 0,
            frames,
            latency_comp: &latency,
            automation: &automation,
        },
        &mut BlockScratch {
            data: &mut data,
            track_buf_l: &mut track_buf_l,
            track_buf_r: &mut track_buf_r,
            bus_bufs: &mut bus_bufs,
            port_scratch: &mut port_scratch,
            note_event_buf: &mut note_buf,
            sidechain: &mut sidechain,
        },
        &mut strategy,
    );

    (data, bus_bufs)
}

/// Reusable benchmark harness over the **live** render path
/// ([`RenderStrategy::Live`]) — the exact code the audio callback runs,
/// minus the CLAP plugin calls (a harness project carries no plugin
/// instances, so per-block cost here is pure engine overhead).
///
/// State is built once and reused across renders so the measured loop is
/// allocation-free, matching the realtime callback. Used by
/// `benches/render_path.rs`; not part of the public API.
#[doc(hidden)]
pub struct RenderBenchHarness {
    tracks: IndexMap<TrackId, Track>,
    busses: IndexMap<BusId, Bus>,
    clips: Vec<AudioClip>,
    midi_clips: Vec<MidiClip>,
    plugins: IndexMap<PluginInstanceId, Mutex<SyncClapInstance>>,
    tempo_map: TempoMap,
    aux_sends: Vec<AuxSend>,
    sidechain: SidechainTaps,
    latency: crate::latency::LatencyComp,
    automation: AutomationSnapshot,
    data: Vec<f32>,
    track_buf_l: Vec<f32>,
    track_buf_r: Vec<f32>,
    bus_bufs: Vec<(Vec<f32>, Vec<f32>)>,
    port_scratch: Vec<(Vec<f32>, Vec<f32>)>,
    note_buf: Vec<PendingNoteEvent>,
    midi_stash: MidiStash,
    frames: usize,
    sample_rate: u32,
}

#[doc(hidden)]
impl RenderBenchHarness {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        tracks: Vec<Track>,
        busses: Vec<Bus>,
        clips: Vec<AudioClip>,
        midi_clips: Vec<MidiClip>,
        aux_sends: Vec<AuxSend>,
        tempo_map: TempoMap,
        frames: usize,
        sample_rate: u32,
    ) -> Self {
        let tracks: IndexMap<TrackId, Track> = tracks.into_iter().map(|t| (t.id, t)).collect();
        let busses: IndexMap<BusId, Bus> = busses.into_iter().map(|b| (b.id, b)).collect();
        let bus_count = busses.len();
        Self {
            tracks,
            busses,
            clips,
            midi_clips,
            plugins: IndexMap::new(),
            tempo_map,
            aux_sends,
            sidechain: SidechainTaps::new(frames),
            latency: crate::latency::LatencyComp::empty(),
            automation: AutomationSnapshot::default(),
            data: vec![0.0; frames * 2],
            track_buf_l: vec![0.0; frames],
            track_buf_r: vec![0.0; frames],
            bus_bufs: (0..bus_count)
                .map(|_| (vec![0.0; frames], vec![0.0; frames]))
                .collect(),
            port_scratch: (0..MAX_PLUGIN_OUTPUT_PORTS)
                .map(|_| (vec![0.0; frames], vec![0.0; frames]))
                .collect(),
            note_buf: Vec::with_capacity(MAX_MIDI_EVENTS_PER_BUFFER),
            midi_stash: MidiStash::new(),
            frames,
            sample_rate,
        }
    }

    /// Render one live block at `playhead`. Returns the interleaved
    /// output so the caller can black-box it.
    pub fn render(&mut self, playhead: u64) -> &[f32] {
        let frames = self.frames;
        self.data.fill(0.0);
        let active_busses = self.busses.len();
        let transport_snap = Some(crate::mixer::common::TransportSnap {
            bpm: self.tempo_map.bpm as f64,
            num: self.tempo_map.numerator as u16,
            den: self.tempo_map.denominator as u16,
            playing: true,
            pos_beats: transport_pos_beats(&self.tempo_map, playhead, self.sample_rate),
        });
        let mut strategy = RenderStrategy::Live {
            midi_stash: &mut self.midi_stash,
            transport_snap,
            monitor_temp: &[],
            monitor_frames: 0,
            input_channels: 0,
        };
        render_block(
            BlockInputs {
                channels: 2,
                tracks: &self.tracks,
                busses: &self.busses,
                clips: &self.clips,
                midi_clips: &self.midi_clips,
                plugins: &self.plugins,
                tempo_map: &self.tempo_map,
                sample_rate: self.sample_rate,
                any_solo: false,
                active_busses,
                aux_sends: &self.aux_sends,
                sidechain_routes: &[],
                playhead,
                frames,
                latency_comp: &self.latency,
                automation: &self.automation,
            },
            &mut BlockScratch {
                data: &mut self.data[..frames * 2],
                track_buf_l: &mut self.track_buf_l,
                track_buf_r: &mut self.track_buf_r,
                bus_bufs: &mut self.bus_bufs,
                port_scratch: &mut self.port_scratch,
                note_event_buf: &mut self.note_buf,
                sidechain: &mut self.sidechain,
            },
            &mut strategy,
        );
        &self.data
    }
}

