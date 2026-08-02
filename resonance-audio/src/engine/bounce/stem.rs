//! Stem render core (ba todo #322).
//!
//! Building blocks for "export stems": render an arbitrary single
//! *source* — one track, one bus, or the whole master mix — over a
//! fixed, shared render range, reusing the same chunked render core
//! (`render::render_chunk`) that drives [`super::to_wav`] /
//! [`super::to_audio_clip`]. Every stem in an export is rendered over
//! ONE common `[render_start, render_end)` so they share a zero origin
//! and re-import sample-aligned.
//!
//! What this module provides:
//!
//! * [`StemSource`] + [`stem_filter`] — the per-source `in_filter` rules.
//!   A track includes its sub-tracks; a bus includes every track routed
//!   to it — top-level tracks (plus their sub-tracks) *and* sub-tracks
//!   routed to it on their own (ba todo #1239) — and runs that bus's FX
//!   chain; master is everything.
//! * [`render_stem`] — render one source over a shared range to an
//!   in-RAM interleaved-stereo buffer. Per-track / per-bus stems exclude
//!   master FX + master volume (like `to_audio_clip`); the master stem
//!   includes them (like `to_wav`).
//! * [`write_stem_wav`] — a WAV writer parameterised by bit depth
//!   (16-bit / 24-bit int, 32-bit float) and target sample rate
//!   (resampled only when it differs from the engine rate), generalising
//!   `wav.rs`'s hard-coded 32-float / 2-channel output.
//! * [`stem_project_range`] — the shared `[start, end)` over all clips,
//!   so an export computes the common origin once.
//!
//! Automation (epic #14) is honoured automatically: every source goes
//! through `render_chunk` → `mixer::render_block`, the same path live
//! playback uses.

use std::collections::HashSet;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use indexmap::IndexMap;
use parking_lot::{Mutex, RwLock};

use crate::clap_host::SyncClapInstance;
use crate::types::*;

use super::super::SharedState;
use super::render::{
    build_latency_comp, chunk_span, master_fx_latency, render_chunk, reset_plugins, ChunkCtx,
    ChunkScratch,
};

// `StemSource` and `StemBitDepth` are pure protocol descriptors and live
// in `crate::types` (re-exported here via `use crate::types::*`). The
// hound-specific encoding helpers below stay in the engine layer so the
// protocol layer carries no dependency on the WAV encoder.

impl StemBitDepth {
    fn bits(self) -> u16 {
        match self {
            StemBitDepth::Int16 => 16,
            StemBitDepth::Int24 => 24,
            StemBitDepth::Float32 => 32,
        }
    }

    fn format(self) -> hound::SampleFormat {
        match self {
            StemBitDepth::Int16 | StemBitDepth::Int24 => hound::SampleFormat::Int,
            StemBitDepth::Float32 => hound::SampleFormat::Float,
        }
    }
}

/// The track-id `in_filter` for a stem source, plus whether master FX +
/// master volume should be applied to the rendered mix.
///
/// `Master` keeps `set` empty and flags `all` so the closure short-cuts
/// to "every track contributes" without materialising every id.
#[derive(Debug, Clone, Default)]
pub struct StemFilter {
    /// Tracks that contribute (ignored when `all`).
    pub set: HashSet<TrackId>,
    /// `true` for the master stem: every track contributes.
    pub all: bool,
    /// Apply master FX chain + master volume + hard-clip to the result.
    pub include_master_fx: bool,
}

impl StemFilter {
    /// Does the given track contribute to this stem?
    #[inline]
    pub fn contains(&self, id: TrackId) -> bool {
        self.all || self.set.contains(&id)
    }
}

/// Resolve the [`StemFilter`] for `source` against the current track
/// topology. Pure (reads only the passed map) so it is unit-testable
/// without an engine.
pub fn stem_filter(source: StemSource, tracks: &IndexMap<TrackId, Track>) -> StemFilter {
    match source {
        StemSource::Master => StemFilter {
            set: HashSet::new(),
            all: true,
            include_master_fx: true,
        },
        StemSource::Track(track_id) => {
            let mut set = HashSet::new();
            set.insert(track_id);
            add_sub_tracks(track_id, tracks, &mut set);
            StemFilter {
                set,
                all: false,
                include_master_fx: false,
            }
        }
        StemSource::Bus(bus_id) => {
            let mut set = HashSet::new();
            for t in tracks.values() {
                // A SUB-TRACK carries its own routing (ba todo #1239).
                // `mixer::render_core` reads `sub_track.output()` per
                // sub-track and sums post-fader sub-track audio straight
                // into the bus — a sub-track can therefore feed a bus on
                // its own, independently of its parent, which is exactly
                // how a multi-output instrument gets a group bus (doc
                // #274 option (b): route the drum kit's six group taps
                // into one bus and insert a compressor there). Scanning
                // only top-level tracks here made such a bus resolve to
                // the EMPTY set, so its stem exported — and its
                // `meter.measure` reported — digital silence.
                if t.output() != TrackOutput::Bus(bus_id) {
                    continue;
                }
                set.insert(t.id);
                // A whole instrument stays in one stem: a bus-routed
                // parent brings its sub-tracks along, the same rule
                // `StemSource::Track` applies. The `HashSet` collapses
                // the overlap when a parent and one of its sub-tracks
                // both target this bus (each track is still rendered
                // exactly once — the mixer walks the track map once).
                if t.sub_track_of.is_none() {
                    add_sub_tracks(t.id, tracks, &mut set);
                }
            }
            add_fan_out_parents(tracks, &mut set);
            StemFilter {
                set,
                all: false,
                include_master_fx: false,
            }
        }
    }
}

/// Insert the parent of every sub-track already in `set` (ba todo #1239).
///
/// A sub-track produces no audio of its own: its signal is one output
/// port of the parent's instrument, fanned out by `mixer::render_core`
/// while the *parent* is being rendered. The `in_filter` gate skips a
/// track that is not in the set before its instrument runs, so a bus fed
/// only by sub-tracks would still render as silence unless the parent
/// comes along to drive the fan-out.
///
/// The set is an `in_filter` — "which tracks contribute to this render" —
/// not a membership list, so pulling a parent in for its fan-out is not a
/// claim that the parent feeds the bus. Its own main-output (port 0)
/// signal still follows its own routing, exactly as it does for every
/// other track in the set; on a multi-output instrument that is the port
/// almost nothing lands on (the drum kit puts one of thirty pads there —
/// ba doc #274 §1a). This does NOT walk further: a parent added here does
/// not drag in its other sub-tracks, so a tap routed to a different bus
/// stays out of this one.
fn add_fan_out_parents(tracks: &IndexMap<TrackId, Track>, set: &mut HashSet<TrackId>) {
    let parents: Vec<TrackId> = set
        .iter()
        .filter_map(|id| tracks.get(id))
        .filter_map(|t| t.sub_track_of.map(|(parent, _)| parent))
        .collect();
    set.extend(parents);
}

/// Insert every sub-track fed by `parent` into `set`.
fn add_sub_tracks(parent: TrackId, tracks: &IndexMap<TrackId, Track>, set: &mut HashSet<TrackId>) {
    for t in tracks.values() {
        if let Some((p, _)) = t.sub_track_of {
            if p == parent {
                set.insert(t.id);
            }
        }
    }
}

/// The shared render window `[start, end)` covering every audio + MIDI
/// clip in the project, matching [`super::to_wav`]'s range computation.
/// An export renders every stem over this one range so they line up.
///
/// Returns `None` when there is nothing to render.
pub fn stem_project_range(
    clips: &Arc<RwLock<Vec<AudioClip>>>,
    midi_clips: &Arc<RwLock<Vec<MidiClip>>>,
    tempo_map: &Arc<arc_swap::ArcSwap<TempoMap>>,
    sample_rate: u32,
) -> Option<(SamplePos, SamplePos)> {
    let clips_guard = clips.read();
    let midi_guard = midi_clips.read();
    let tm = tempo_map.load();

    if clips_guard.is_empty() && midi_guard.is_empty() {
        return None;
    }
    let audio_start = clips_guard.iter().map(|c| c.start_sample).min();
    let audio_end = clips_guard.iter().map(|c| c.end_sample()).max();
    let midi_start = midi_guard.iter().map(|c| c.start_sample).min();
    let midi_end = midi_guard
        .iter()
        .map(|c| tm.tick_to_abs_sample(c.start_sample, c.visible_duration_ticks(), sample_rate))
        .max();

    let start = audio_start.into_iter().chain(midi_start).min().unwrap_or(0);
    let end = audio_end.into_iter().chain(midi_end).max().unwrap_or(0);
    if end <= start {
        None
    } else {
        Some((start, end))
    }
}

/// Render one `source` over the shared `[render_start, render_end)` to an
/// in-RAM interleaved-stereo buffer (`(render_end - render_start) * 2`
/// samples at the engine `sample_rate`).
///
/// All stems passed the same range produce equal-length buffers that
/// share a common zero origin (`render_start`), so they re-import
/// sample-aligned. Plugin-delay compensation is applied and the leading
/// `max_latency` frames trimmed — identical to the other bounce paths —
/// so the stem lands on the timeline with zero net shift.
///
/// Per-track / per-bus stems ignore mute/solo (you want each source's
/// audio regardless of how it sits in the mix); the master stem honours
/// them so it matches live playback exactly.
///
/// Returns `Err` if the transport is rolling (the offline renderer
/// shares plugin instances with the live mixer) or the range is empty.
#[allow(clippy::too_many_arguments)]
pub fn render_stem(
    source: StemSource,
    render_start: SamplePos,
    render_end: SamplePos,
    shared: &Arc<SharedState>,
    tracks: &Arc<RwLock<IndexMap<TrackId, Track>>>,
    busses: &Arc<RwLock<IndexMap<BusId, Bus>>>,
    master: &Arc<RwLock<MasterBus>>,
    clips: &Arc<RwLock<Vec<AudioClip>>>,
    midi_clips: &Arc<RwLock<Vec<MidiClip>>>,
    plugins: &Arc<RwLock<IndexMap<PluginInstanceId, Mutex<SyncClapInstance>>>>,
    tempo_map: &Arc<arc_swap::ArcSwap<TempoMap>>,
    sample_rate: u32,
) -> Result<Vec<f32>, String> {
    // Same guard as the other offline renderers: rendering while the
    // transport rolls would interleave shared plugin process()/reset
    // calls with live playback and corrupt both outputs.
    if shared.playing.load(Ordering::Relaxed) {
        return Err("Stop transport before rendering stems".into());
    }
    if render_end <= render_start {
        return Err("Empty render range".into());
    }

    let filter = stem_filter(source, &tracks.read());
    // The master stem honours mute/solo (it is the real mix); isolated
    // track/bus stems render their source regardless of mute/solo.
    let respect_mute_solo = filter.include_master_fx;

    reset_plugins(plugins);

    let bounce_tm = (**tempo_map.load()).clone();
    let master_vol = f32::from_bits(shared.master_volume_bits.load(Ordering::Relaxed));
    let latency_comp = build_latency_comp(shared, tracks, busses, plugins);
    // Stems that include the master FX chain (the master stem) are
    // shifted by its latency on top of the track/bus comp; pre-rolling
    // and trimming both keeps every stem mutually sample-aligned and
    // preserves the master stem's tail (doc #260 finding #8).
    let master_latency = if filter.include_master_fx {
        master_fx_latency(shared, master, plugins)
    } else {
        0
    };
    let comp_latency = latency_comp.max_latency() + master_latency;
    let render_stop = render_end + comp_latency;
    let mut skip_frames = comp_latency as usize;

    // Stem export predates parameter automation (epic #40) and threads no
    // lane snapshot through its command path, so render with an empty one —
    // matching the pre-merge stem behaviour. Automated lanes still apply on
    // the live/bounce/export paths.
    let automation = crate::engine::AutomationSnapshot::default();
    let ctx = ChunkCtx {
        shared,
        tracks,
        busses,
        master,
        clips,
        midi_clips,
        plugins,
        tempo_map: &bounce_tm,
        automation: &automation,
        sample_rate,
        master_vol,
        latency_comp: &latency_comp,
    };
    let mut scratch = ChunkScratch::new();

    let total_frames = (render_end - render_start) as usize;
    let mut output = vec![0.0f32; total_frames * 2];

    let in_filter = |id: TrackId| filter.contains(id);
    let mut pos = render_start;
    let mut written: usize = 0;
    while pos < render_stop {
        // Tail chunks are padded up to the CLAP activation minimum and
        // only `emit` frames are consumed — see `chunk_span`.
        let (render_frames, emit) = chunk_span(render_stop - pos);
        render_chunk(
            &ctx,
            &mut scratch,
            pos,
            render_frames,
            &in_filter,
            filter.include_master_fx,
            respect_mute_solo,
            false,
        );
        // Drop the leading plugin-latency frames so the stem aligns with
        // the timeline (and with every other stem over this range).
        let drop_now = skip_frames.min(emit);
        skip_frames -= drop_now;
        let copy = (emit - drop_now).min(total_frames - written);
        output[written * 2..(written + copy) * 2]
            .copy_from_slice(&scratch.mix_buf[drop_now * 2..(drop_now + copy) * 2]);
        written += copy;
        pos += emit as u64;
    }

    Ok(output)
}

/// Write an interleaved-stereo `[-1.0, 1.0]` buffer to a WAV file at the
/// requested `bit_depth`. When `target_rate` differs from `engine_rate`
/// the buffer is linearly resampled first; otherwise it is written
/// through untouched.
///
/// Generalises `wav.rs`, which is fixed to 32-bit-float / 2-channel.
/// `samples` are stereo-interleaved (length must be even).
pub fn write_stem_wav(
    path: &str,
    samples: &[f32],
    engine_rate: u32,
    target_rate: u32,
    bit_depth: StemBitDepth,
) -> Result<(), String> {
    // Resample to the requested rate only when it actually differs —
    // a matching rate is a straight passthrough (no quality loss).
    let resampled;
    let pcm: &[f32] = if target_rate != engine_rate {
        resampled = crate::decode::linear_resample(samples, engine_rate, target_rate);
        &resampled
    } else {
        samples
    };

    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: target_rate,
        bits_per_sample: bit_depth.bits(),
        sample_format: bit_depth.format(),
    };
    let mut writer = hound::WavWriter::create(path, spec)
        .map_err(|e| format!("Failed to create WAV file: {e}"))?;

    match bit_depth {
        StemBitDepth::Float32 => {
            for &s in pcm {
                writer
                    .write_sample(s)
                    .map_err(|e| format!("WAV write error: {e}"))?;
            }
        }
        StemBitDepth::Int16 => {
            for &s in pcm {
                let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16;
                writer
                    .write_sample(v)
                    .map_err(|e| format!("WAV write error: {e}"))?;
            }
        }
        StemBitDepth::Int24 => {
            const MAX_24: f32 = 8_388_607.0; // 2^23 - 1
            for &s in pcm {
                let v = (s.clamp(-1.0, 1.0) * MAX_24).round() as i32;
                writer
                    .write_sample(v)
                    .map_err(|e| format!("WAV write error: {e}"))?;
            }
        }
    }

    writer
        .finalize()
        .map_err(|e| format!("WAV finalize error: {e}"))
}
