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
//!   A track includes its sub-tracks; a bus includes every top-level
//!   track routed to it (plus their sub-tracks) and runs that bus's FX
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
    build_latency_comp, render_chunk, reset_plugins, ChunkCtx, ChunkScratch, BOUNCE_CHUNK,
};

/// Which slice of the mix a stem captures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StemSource {
    /// A single track plus its sub-tracks (multi-output instruments fan
    /// out to sibling sub-tracks). Excludes master FX + master volume,
    /// like a bounce-in-place clip, so the stem re-imports at unity.
    Track(TrackId),
    /// A return / aux / group bus: every top-level track routed to the
    /// bus (plus their sub-tracks). The bus's own FX chain runs (it is
    /// fed only by the in-filter tracks), but master FX + master volume
    /// are excluded.
    Bus(BusId),
    /// The full mix — every track, with master FX, master volume and the
    /// final hard-clip applied, identical to [`super::to_wav`].
    Master,
}

/// PCM encoding for a written stem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StemBitDepth {
    /// 16-bit signed integer.
    Int16,
    /// 24-bit signed integer (packed 3 bytes/sample).
    Int24,
    /// 32-bit IEEE float (the engine's native format — lossless).
    Float32,
}

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
                // Only top-level tracks carry an output routing; their
                // sub-tracks ride along whichever way the parent goes.
                if t.sub_track_of.is_none() && t.output() == TrackOutput::Bus(bus_id) {
                    set.insert(t.id);
                    add_sub_tracks(t.id, tracks, &mut set);
                }
            }
            StemFilter {
                set,
                all: false,
                include_master_fx: false,
            }
        }
    }
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
    let latency_comp = build_latency_comp(tracks, busses, plugins);
    let comp_latency = latency_comp.max_latency();
    let render_stop = render_end + comp_latency;
    let mut skip_frames = comp_latency as usize;

    let ctx = ChunkCtx {
        shared,
        tracks,
        busses,
        master,
        clips,
        midi_clips,
        plugins,
        tempo_map: &bounce_tm,
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
        let frames = ((render_stop - pos) as usize).min(BOUNCE_CHUNK);
        render_chunk(
            &ctx,
            &mut scratch,
            pos,
            frames,
            &in_filter,
            filter.include_master_fx,
            respect_mute_solo,
        );
        // Drop the leading plugin-latency frames so the stem aligns with
        // the timeline (and with every other stem over this range).
        let drop_now = skip_frames.min(frames);
        skip_frames -= drop_now;
        let copy = (frames - drop_now).min(total_frames - written);
        output[written * 2..(written + copy) * 2]
            .copy_from_slice(&scratch.mix_buf[drop_now * 2..(drop_now + copy) * 2]);
        written += copy;
        pos += frames as u64;
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
