//! Non-destructive freeze render: render one track's full
//! post-instrument / post-FX output (including SVS-rendered vocal audio
//! clips) over the project range to a 32-bit float stereo WAV at a
//! freeze-cache path.
//!
//! Unlike [`super::clip::to_audio_clip`] this MUST NOT mutate the
//! track's clips / source / notes — it only produces the cache file and
//! returns a [`FreezeCacheRef`] describing it. The frozen audio plays
//! back through master on the next playback, so (like the bounce-in-
//! place path) the render excludes master FX / master volume so those
//! processors are not applied twice.

use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use indexmap::IndexMap;
use parking_lot::RwLock;

use resonance_common::{
    compute_fingerprint, FreezeCacheRef, FreezeCacheStatus, FreezeFingerprintBuilder,
};

use crate::clap_host::PluginMap;
use crate::types::*;

use super::super::SharedState;
use super::render::{
    build_latency_comp, chunk_span, render_chunk, reset_plugins, ChunkCtx, ChunkScratch,
};

/// Bit depth of the freeze-cache WAV. Matches the project-bounce path
/// ([`super::wav::to_wav`]): 32-bit float so the cache is a bit-exact
/// capture of the rendered mix with no requantization.
const FREEZE_BIT_DEPTH: u16 = 32;

/// Error message returned when a freeze render is cancelled cooperatively.
/// [`super::freeze_terminal_event`] matches on this to emit
/// `AudioEvent::FreezeCancelled` rather than `FreezeError`.
pub const FREEZE_CANCELLED_MSG: &str = "Freeze cancelled";

/// Render the full post-instrument / post-FX output of `source_track_id`
/// (and any of its instrument sub-tracks) over the project range to a
/// 32-bit float stereo WAV at `path`, returning a [`FreezeCacheRef`] on
/// success.
///
/// The render range is `[0, project_end + tail]` where `project_end` is
/// the latest end across every audio and MIDI clip in the project and
/// `tail` the shared offline FX tail (`BOUNCE_TAIL_SECONDS`), so the
/// cache is timeline-aligned and can be played back from sample 0
/// without a stored offset. Audio clips on the track (e.g. SVS-rendered
/// vocals) and MIDI driving the track's instrument are both included.
///
/// Master FX / master volume are excluded — the cache plays back through
/// master on the next playback, so applying them here would double them.
///
/// `progress` is called with a fraction in `[0.0, 1.0]` at most once per
/// integer percent. `cancel` is this render's own cooperative cancel
/// token (created per render — see `HandlerState::freeze_cancel`):
/// flipping it to `true` aborts the render between chunks, removes the
/// partial WAV, and returns `Err`. It is never cleared here, so a
/// cancel can neither be consumed by a different renderer nor lost to a
/// later render starting.
///
/// Public so integration tests can drive the renderer directly without
/// going through the (separate) engine command path.
#[allow(clippy::too_many_arguments)]
pub fn to_freeze_cache(
    source_track_id: TrackId,
    path: String,
    shared: &Arc<SharedState>,
    cancel: &AtomicBool,
    tracks: &Arc<RwLock<IndexMap<TrackId, Track>>>,
    busses: &Arc<RwLock<IndexMap<BusId, Bus>>>,
    master: &Arc<RwLock<MasterBus>>,
    clips: &Arc<RwLock<Vec<AudioClip>>>,
    midi_clips: &Arc<RwLock<Vec<MidiClip>>>,
    plugins: &Arc<RwLock<PluginMap>>,
    tempo_map: &Arc<arc_swap::ArcSwap<TempoMap>>,
    automation: &crate::engine::AutomationSnapshot,
    sample_rate: u32,
    progress: &mut dyn FnMut(f32),
) -> Result<FreezeCacheRef, String> {
    // Same guard as the bounce paths: the offline renderer shares plugin
    // instances with the live mixer, so rendering while the transport
    // rolls would interleave process() calls (and the reset below) with
    // live playback, corrupting both outputs.
    if shared.playing.load(Ordering::Relaxed) {
        return Err("Stop transport before freezing".into());
    }

    // Refresh vocal-tuning render caches so a retuned clip on the frozen
    // track is captured corrected, identical to live playback (todo #358).
    super::super::vocal_render::ensure_tuning_caches(clips, sample_rate);

    // Resolve source + sub-tracks (multi-output instruments like
    // resonance-drums spawn sibling tracks fed by parent output ports).
    let filter_set: HashSet<TrackId> = {
        let tracks_guard = tracks.read();
        if !tracks_guard.contains_key(&source_track_id) {
            return Err(format!("Source track {source_track_id} not found"));
        }
        let mut set = HashSet::new();
        set.insert(source_track_id);
        for t in tracks_guard.values() {
            if let Some((parent, _)) = t.sub_track_of {
                if parent == source_track_id {
                    set.insert(t.id);
                }
            }
        }
        set
    };

    // The automation baked into the cache (code review ENG-08): the
    // plugin-parameter lanes of every plugin the frozen tracks run. Gain /
    // pan / mute lanes stay out — `freeze_raw` renders pre-fader, and the
    // live mixer still applies those lanes to the frozen track on playback.
    let baked = baked_automation(&filter_set, tracks, automation);

    // Compute the fingerprint of the frozen inputs before rendering so
    // the returned ref records exactly what was captured. (Engine-side
    // inputs: the filtered tracks' MIDI notes + the source track's
    // plugin chain / instrument selection + the baked plugin automation.
    // The app layer recomputes its own fingerprint to detect staleness.)
    let render_fingerprint =
        compute_track_fingerprint(&filter_set, source_track_id, tracks, midi_clips, &baked);

    // Project range: [0, latest clip/MIDI end]. Starting at 0 keeps the
    // cache timeline-aligned so it plays back from sample 0 with no
    // stored offset.
    let render_end = {
        let clips_guard = clips.read();
        let midi_guard = midi_clips.read();
        let tm = tempo_map.load();

        let audio_end = clips_guard.iter().map(|c| c.end_sample()).max();
        // Tempo-aware end to match the renderer's note scheduling under
        // tempo changes (mirrors `to_wav`).
        let midi_end = midi_guard
            .iter()
            .map(|c| tm.tick_to_abs_sample(c.start_sample, c.visible_duration_ticks(), sample_rate))
            .max();
        audio_end.into_iter().chain(midi_end).max().unwrap_or(0)
    };
    let render_start: u64 = 0;

    if render_end <= render_start {
        return Err("Nothing to freeze".into());
    }
    // The shared offline FX tail (code review ENG-07): the frozen track's
    // reverb / delay / release past the last clip end stays audible.
    let render_end = render_end + super::super::bounce_common::offline_tail_frames(sample_rate);

    let spec = hound::WavSpec {
        channels: 2,
        sample_rate,
        bits_per_sample: FREEZE_BIT_DEPTH,
        sample_format: hound::SampleFormat::Float,
    };
    // Written to a temp file and renamed into place once complete (code
    // review ENG-13): a failed or cancelled refreeze keeps the previous
    // cache intact. Every early return drops `output`, removing the temp.
    let output = super::PartialFile::new(&path);
    let mut writer = hound::WavWriter::create(output.temp(), spec)
        .map_err(|e| format!("Failed to create freeze-cache WAV: {e}"))?;

    reset_plugins(plugins);

    let bounce_tm = (**tempo_map.load()).clone();
    let master_vol = f32::from_bits(shared.master_volume_bits.load(Ordering::Relaxed));
    let latency_comp = build_latency_comp(shared, tracks, busses, plugins);
    // Render `max_latency` extra frames and drop the same number from
    // the front: plugin-delay compensation shifts every contributing
    // track by the pipeline latency, so trimming it re-aligns the cache
    // with the timeline (same reasoning as the bounce paths).
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
        automation: &baked,
        sample_rate,
        master_vol,
        latency_comp: &latency_comp,
        hard_clip: true,
    };
    let mut scratch = ChunkScratch::new();

    // 0% up front so a UI modal shows populated before the first chunk.
    progress(0.0);

    let in_filter = move |id: TrackId| filter_set.contains(&id);
    let mut pos = render_start;
    let mut last_emitted_pct: i32 = 0;
    while pos < render_stop {
        // Cooperative cancel — checked once per chunk (~tens of ms each)
        // so a UI Cancel button releases the freeze promptly. The token
        // is this render's own, so it is not cleared.
        if cancel.load(Ordering::Relaxed) {
            drop(writer);
            drop(output);
            return Err(FREEZE_CANCELLED_MSG.into());
        }

        // Tail chunks are padded up to the CLAP activation minimum and
        // only `emit` frames are consumed — see `chunk_span`.
        let (render_frames, emit) = chunk_span(render_stop - pos);
        // include_master_fx = false (cache replays through master),
        // respect_mute_solo = false (freeze the track's own output
        // regardless of its live mute/solo state), freeze_raw = true
        // (capture the raw pre-fader / pre-pan / pre-routing post-FX
        // signal so the live mixer re-applies volume / pan / routing on
        // playback and stays sample-identical to the unfrozen track).
        // `fan_out_only` is empty (ba todo #1242): freeze always
        // captures the parent plus its whole fan-out, so the parent's own
        // main output belongs in the cache.
        render_chunk(
            &ctx,
            &mut scratch,
            pos,
            render_frames,
            &in_filter,
            &|_| false,
            // No key-only tracks (ba doc #277): freeze captures the
            // track's own post-FX signal.
            &|_| false,
            &|_| false,
            false,
            false,
            true,
        );

        let drop_now = skip_frames.min(emit);
        skip_frames -= drop_now;
        for &sample in &scratch.mix_buf[drop_now * 2..emit * 2] {
            if let Err(e) = writer.write_sample(sample) {
                // Drop the partial file so a half-written cache never
                // sits next to its expected output.
                drop(writer);
                drop(output);
                return Err(format!("Freeze-cache WAV write error: {e}"));
            }
        }

        pos += emit as u64;

        // Emit progress at most once per integer percent so we don't
        // flood the caller on a long render.
        let pct = (((pos - render_start) as f32 / (render_stop - render_start) as f32) * 100.0)
            as i32;
        if pct > last_emitted_pct {
            last_emitted_pct = pct;
            progress((pct as f32 / 100.0).min(1.0));
        }
    }

    writer
        .finalize()
        .map_err(|e| format!("Freeze-cache WAV finalize error: {e}"))?;
    output.commit()?;

    progress(1.0);

    // `cache_filename` is the file name relative to the project's freeze
    // cache dir; callers pass a full path and we record just the name.
    let cache_filename = Path::new(&path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or(path);

    Ok(FreezeCacheRef::new(
        cache_filename,
        sample_rate,
        FREEZE_BIT_DEPTH,
        render_fingerprint,
        FreezeCacheStatus::Frozen,
    ))
}

/// Decode a freeze-cache WAV at `path` into a [`FrozenSource`] ready to
/// attach to a track for playback (ba todo #577).
///
/// The cache files written by [`to_freeze_cache`] are 32-bit float stereo
/// (interleaved L/R) and timeline-aligned from sample 0, so the decoded
/// buffer plays back with no stored offset. `cache_ref` is the persisted
/// reference the buffer is being rehydrated from; it travels into the
/// returned [`FrozenSource`] unchanged so the caller keeps its
/// filename / fingerprint / status metadata.
///
/// Returns `Err` — never panics — when the file is missing, unreadable,
/// the wrong format, or truncated, so the project-load path can fall the
/// track back to a *stale* (offer-refreeze) state instead of crashing.
pub fn read_freeze_cache(path: &Path, cache_ref: FreezeCacheRef) -> Result<FrozenSource, String> {
    let mut reader = hound::WavReader::open(path)
        .map_err(|e| format!("Open freeze cache {}: {e}", path.display()))?;
    let spec = reader.spec();
    if spec.channels != 2 {
        return Err(format!(
            "Freeze cache {} is not stereo (has {} channel(s))",
            path.display(),
            spec.channels
        ));
    }

    // Caches are written as 32-bit float; decode int as a defensive
    // fallback so a hand-substituted PCM file still loads rather than
    // erroring the whole project.
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .collect::<Result<Vec<f32>, _>>()
            .map_err(|e| format!("Read freeze cache {}: {e}", path.display()))?,
        hound::SampleFormat::Int => {
            let scale = 1.0 / (1i64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.map(|v| v as f32 * scale))
                .collect::<Result<Vec<f32>, _>>()
                .map_err(|e| format!("Read freeze cache {}: {e}", path.display()))?
        }
    };

    let frame_count = (samples.len() / 2) as u64;
    Ok(FrozenSource::new(
        cache_ref,
        Arc::new(samples),
        spec.sample_rate,
        frame_count,
    ))
}

/// The slice of `automation` a freeze of `filter_set` bakes in: the
/// plugin-parameter lanes of every plugin on those tracks' chains (code
/// review ENG-08). Mix lanes are left out — see the call site.
fn baked_automation(
    filter_set: &HashSet<TrackId>,
    tracks: &Arc<RwLock<IndexMap<TrackId, Track>>>,
    automation: &crate::engine::AutomationSnapshot,
) -> crate::engine::AutomationSnapshot {
    let mut baked = crate::engine::AutomationSnapshot::default();
    let tracks_guard = tracks.read();
    for track in filter_set.iter().filter_map(|id| tracks_guard.get(id)) {
        for id in track.plugins().iter() {
            if let Some(lanes) = automation.plugin_params.get(id) {
                baked.plugin_params.insert(*id, lanes.clone());
            }
        }
    }
    baked
}

/// Compute a stable fingerprint over the engine-visible freeze inputs:
/// the filtered tracks' MIDI notes plus the source track's plugin chain
/// (instrument = first slot) and the baked plugin-parameter automation.
/// Re-rendering with identical inputs yields the same hash; editing any
/// of them changes it, which the app layer uses to mark a frozen track
/// stale.
fn compute_track_fingerprint(
    filter_set: &HashSet<TrackId>,
    source_track_id: TrackId,
    tracks: &Arc<RwLock<IndexMap<TrackId, Track>>>,
    midi_clips: &Arc<RwLock<Vec<MidiClip>>>,
    baked: &crate::engine::AutomationSnapshot,
) -> u64 {
    let mut notes = Vec::new();
    {
        let midi_guard = midi_clips.read();
        // Deterministic order: clips sorted by (track, start, id) so the
        // hash is independent of storage order.
        let mut relevant: Vec<&MidiClip> = midi_guard
            .iter()
            .filter(|c| filter_set.contains(&c.track_id))
            .collect();
        relevant.sort_by_key(|c| (c.track_id, c.start_sample, c.id));
        for clip in relevant {
            notes.extend_from_slice(&clip.track_id.to_le_bytes());
            notes.extend_from_slice(&clip.start_sample.to_le_bytes());
            notes.extend_from_slice(&clip.trim_start_ticks.to_le_bytes());
            notes.extend_from_slice(&clip.trim_end_ticks.to_le_bytes());
            for n in &clip.notes {
                notes.push(n.note);
                notes.extend_from_slice(&n.velocity.to_le_bytes());
                notes.extend_from_slice(&n.start_tick.to_le_bytes());
                notes.extend_from_slice(&n.duration_ticks.to_le_bytes());
            }
        }
    }

    let (instrument_id, plugin_params) = {
        let tracks_guard = tracks.read();
        match tracks_guard.get(&source_track_id) {
            Some(track) => {
                let chain = track.plugins();
                let instrument = chain.first().map(|id| id.to_string()).unwrap_or_default();
                let mut params = Vec::with_capacity(chain.len() * 8);
                for id in chain.iter() {
                    params.extend_from_slice(&id.to_le_bytes());
                }
                (instrument, params)
            }
            None => (String::new(), Vec::new()),
        }
    };
    let mut plugin_params = plugin_params;
    // Baked automation, in a fixed (instance, param) order so map order
    // never perturbs the hash (code review ENG-08).
    let mut lanes: Vec<(PluginInstanceId, &crate::engine::ResolvedParamLane)> = baked
        .plugin_params
        .iter()
        .flat_map(|(id, lanes)| lanes.iter().map(move |l| (*id, l)))
        .collect();
    lanes.sort_by_key(|(id, l)| (*id, l.param_id));
    for (id, l) in lanes {
        plugin_params.extend_from_slice(b"auto");
        plugin_params.extend_from_slice(&id.to_le_bytes());
        plugin_params.extend_from_slice(&l.param_id.to_le_bytes());
        plugin_params.extend_from_slice(&l.min.to_bits().to_le_bytes());
        plugin_params.extend_from_slice(&l.max.to_bits().to_le_bytes());
        for p in &l.lane.points {
            plugin_params.extend_from_slice(&p.time_frames.to_le_bytes());
            plugin_params.extend_from_slice(&p.value.to_bits().to_le_bytes());
            plugin_params.push(p.curve as u8);
        }
    }

    let inputs = FreezeFingerprintBuilder::new()
        .with_notes(notes)
        .with_plugin_params(plugin_params)
        .with_instrument_id(instrument_id)
        .build();
    compute_fingerprint(&inputs)
}
