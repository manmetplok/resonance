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
//!   chain; master is everything. Either kind of filter also pulls in the
//!   PARENT of any sub-tracks it contains, because a sub-track's audio is
//!   one output port of the parent's instrument and only exists while the
//!   parent renders; a parent pulled in for that reason alone is flagged
//!   `fan_out_only` so its own main output stays out of the stem (ba todo
//!   #1242).
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
use parking_lot::RwLock;
use thiserror::Error;

use crate::clap_host::PluginMap;
use crate::types::*;

use super::super::SharedState;
use super::render::{
    build_latency_comp, chunk_span, master_fx_latency, render_chunk, reset_plugins, ChunkCtx,
    ChunkScratch,
};
use super::PartialFileError;

/// Failure rendering or writing a stem ([`render_stem`] /
/// [`write_stem_wav`]) — the two share one error type because
/// `stem_export.rs`'s `export_stems` chains them into a single
/// `Result<(), StemError>` per target. Message text matches the
/// historical `format!()` / literal strings.
#[derive(Debug, Error)]
pub enum StemError {
    #[error("Stop transport before rendering stems")]
    TransportRunning,
    #[error("Empty render range")]
    EmptyRange,
    /// [`frozen_fan_out_refusal`]'s message: a tap whose parent is
    /// frozen has no separable signal to render.
    #[error("{0}")]
    FrozenFanOut(String),
    #[error("Failed to create WAV file: {0}")]
    Create(#[source] hound::Error),
    #[error("WAV write error: {0}")]
    Write(#[source] hound::Error),
    #[error("WAV finalize error: {0}")]
    Finalize(#[source] hound::Error),
    #[error(transparent)]
    Commit(#[from] PartialFileError),
}

impl From<StemError> for EngineError {
    fn from(e: StemError) -> Self {
        let kind = match &e {
            StemError::TransportRunning => EngineErrorKind::Busy,
            StemError::EmptyRange | StemError::FrozenFanOut(_) => EngineErrorKind::Unsupported,
            StemError::Create(_) | StemError::Write(_) | StemError::Finalize(_) => {
                EngineErrorKind::Io
            }
            StemError::Commit(_) => EngineErrorKind::Io,
        };
        EngineError::new(kind, e.to_string())
    }
}

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
    /// Tracks in `set` that are present **only** to drive a sub-track
    /// fan-out (ba todo #1242). Their instrument must run — that is the
    /// only way their sub-tracks get any audio at all — but their own
    /// main output (port 0) belongs to a different stem, so its chain,
    /// fader, aux sends and routing contribute nothing here. Never
    /// populated for `Master`.
    pub fan_out_only: HashSet<TrackId>,
    /// Tracks in `set` that are present **only** as a sidechain KEY
    /// SOURCE for a plugin inside this stem (ba doc #277). They render —
    /// that is the only way their audio exists to be captured — and are
    /// then dropped before the fader, so they key without joining the
    /// mix.
    ///
    /// Without this a stem was rendered without its key sources, every
    /// key resolved to silence, and each keyed plugin fell back to its
    /// own input. `meter.measure` on the ducked track then reported the
    /// same figure to two decimals no matter what was routed in — which
    /// is exactly how the feature is verified, so it read as "sidechain
    /// does nothing" even after the delivery path was fixed.
    pub key_only: HashSet<TrackId>,
    /// Busses that are present only as a key source, for the same
    /// reason. Their chain runs and is captured; their output never
    /// reaches master.
    pub key_only_busses: HashSet<BusId>,
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

    /// Is this track in the filter *only* to drive its sub-tracks'
    /// fan-out, so its own main output must be discarded?
    #[inline]
    pub fn is_fan_out_only(&self, id: TrackId) -> bool {
        !self.all && self.fan_out_only.contains(&id)
    }

    /// Is this track in the filter *only* to be captured as a key, so its
    /// audio must not reach the stem's mix?
    #[inline]
    pub fn is_key_only(&self, id: TrackId) -> bool {
        !self.all && self.key_only.contains(&id)
    }

    /// The bus twin of [`StemFilter::is_key_only`].
    #[inline]
    pub fn is_key_only_bus(&self, id: BusId) -> bool {
        !self.all && self.key_only_busses.contains(&id)
    }
}

/// Resolve the [`StemFilter`] for `source` against the current track
/// topology. Pure (reads only the passed map) so it is unit-testable
/// without an engine.
///
/// Sidechain-free shorthand for [`stem_filter_with_keys`]; a slice whose
/// plugins are keyed from outside it needs that one instead.
pub fn stem_filter(source: StemSource, tracks: &IndexMap<TrackId, Track>) -> StemFilter {
    stem_filter_with_keys(source, tracks, &IndexMap::new(), &[])
}

/// [`stem_filter`], plus the sidechain KEY SOURCES the filtered slice
/// needs in order to sound like itself (ba doc #277).
///
/// A stem renders part of the graph. A plugin inside that part may be
/// keyed from a track or bus OUTSIDE it — which is the normal case, since
/// the whole point of a key is that it comes from somewhere else — and a
/// source that never renders is never captured, so the key resolves to
/// silence and the plugin falls back to its own input. The stem then
/// measures a graph that does not exist: exactly the figure the mix would
/// produce with nothing routed at all.
///
/// Such sources are pulled in as `key_only`: rendered so they can be
/// captured, dropped before the fader so they never join the mix.
pub fn stem_filter_with_keys(
    source: StemSource,
    tracks: &IndexMap<TrackId, Track>,
    busses: &IndexMap<BusId, Bus>,
    routes: &[SidechainRoute],
) -> StemFilter {
    match source {
        StemSource::Master => StemFilter {
            set: HashSet::new(),
            fan_out_only: HashSet::new(),
            key_only: HashSet::new(),
            key_only_busses: HashSet::new(),
            all: true,
            include_master_fx: true,
        },
        StemSource::Track(track_id) => {
            let mut set = HashSet::new();
            set.insert(track_id);
            add_sub_tracks(track_id, tracks, &mut set);
            // When the target IS a sub-track it produces no audio of its
            // own — its signal is one output port of the PARENT's
            // instrument (ba todo #1242, the `StemSource::Track` sibling
            // of #1239's bus fix). Without the parent to drive the
            // fan-out the stem is digital silence, which `render.stems`,
            // a single-sub-track export and `meter.measure` all reported
            // as a real measurement.
            //
            // A parent pulled in this way is a fan-out DRIVER, not a
            // member: "Drums -> Hats" must be hats. The sibling taps are
            // held out by `sub_track_disposition`'s own `in_filter` gate,
            // and the parent's port-0 chain by `fan_out_only`.
            let (key_only, key_only_busses) =
                add_key_sources(tracks, busses, routes, None, &mut set);
            let fan_out_only = add_fan_out_parents(tracks, &mut set);
            StemFilter {
                set,
                fan_out_only,
                key_only,
                key_only_busses,
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
            let (key_only, key_only_busses) =
                add_key_sources(tracks, busses, routes, Some(bus_id), &mut set);
            let fan_out_only = add_fan_out_parents(tracks, &mut set);
            StemFilter {
                set,
                fan_out_only,
                key_only,
                key_only_busses,
                all: false,
                include_master_fx: false,
            }
        }
    }
}

/// Pull every sidechain key source feeding a plugin inside `set` into the
/// render, returning the tracks and busses that are there ONLY for that.
///
/// `own_bus` is the bus being stemmed, if any — its own insert chain is
/// part of the slice, so a key routed into it counts too.
///
/// A key source that is already a member of the stem is left alone: it is
/// audible here anyway, and marking it key-only would drop it from the
/// mix it belongs to.
fn add_key_sources(
    tracks: &IndexMap<TrackId, Track>,
    busses: &IndexMap<BusId, Bus>,
    routes: &[SidechainRoute],
    own_bus: Option<BusId>,
    set: &mut HashSet<TrackId>,
) -> (HashSet<TrackId>, HashSet<BusId>) {
    let mut key_tracks = HashSet::new();
    let mut key_busses = HashSet::new();
    if routes.is_empty() {
        return (key_tracks, key_busses);
    }

    // Every plugin instance inside the slice: the chains of its tracks,
    // plus the stemmed bus's own chain.
    let mut inside: Vec<PluginInstanceId> = Vec::new();
    for id in set.iter() {
        if let Some(track) = tracks.get(id) {
            inside.extend(track.plugins().iter().copied());
        }
    }
    if let Some(bus_id) = own_bus {
        if let Some(bus) = busses.get(&bus_id) {
            inside.extend(bus.plugin_ids.iter().copied());
        }
    }

    for plugin in inside {
        let Some(source) = crate::types::sidechain::route_source(routes, plugin) else {
            continue;
        };
        match source {
            SendSource::Track(id) => {
                if set.insert(id) {
                    key_tracks.insert(id);
                }
            }
            SendSource::Bus(id) => {
                if own_bus == Some(id) {
                    continue;
                }
                // A bus carries nothing of its own: it has to be fed by
                // the tracks routed into it, so they render too — and
                // they are key-only for exactly the same reason.
                for t in tracks.values() {
                    if t.output() == TrackOutput::Bus(id) && set.insert(t.id) {
                        key_tracks.insert(t.id);
                    }
                }
                key_busses.insert(id);
            }
        }
    }
    (key_tracks, key_busses)
}

/// Insert the parent of every sub-track already in `set` (ba todo #1239),
/// returning the parents that were **added by this walk** — i.e. those
/// that are in the filter only to drive a fan-out (ba todo #1242).
///
/// A sub-track produces no audio of its own: its signal is one output
/// port of the parent's instrument, fanned out by `mixer::render_core`
/// while the *parent* is being rendered. The `in_filter` gate skips a
/// track that is not in the set before its instrument runs, so a stem fed
/// only by sub-tracks would still render as silence unless the parent
/// comes along to drive the fan-out.
///
/// The set is an `in_filter` — "which tracks contribute to this render" —
/// not a membership list, so pulling a parent in for its fan-out is not a
/// claim that the parent belongs to this stem. That is exactly what the
/// returned set records: such a parent runs its instrument (the fan-out
/// needs it) but its own main-output (port 0) chain is discarded, so a
/// sub-track stem carries that tap and nothing else. On the real drum kit
/// port 0 is one of thirty pads — the count stick, ba doc #274 §1a — so
/// the leak is small but it is audible whenever that pad is played, and
/// "hats" must mean hats.
///
/// A parent that is already in the set for its OWN sake (it is the stem's
/// target, or it is itself routed to the bus being stemmed) is not
/// returned, so its main output still renders normally.
///
/// This does NOT walk further: a parent added here does not drag in its
/// other sub-tracks, so a tap routed to a different bus stays out of this
/// one.
fn add_fan_out_parents(
    tracks: &IndexMap<TrackId, Track>,
    set: &mut HashSet<TrackId>,
) -> HashSet<TrackId> {
    let parents: Vec<TrackId> = set
        .iter()
        .filter_map(|id| tracks.get(id))
        .filter_map(|t| t.sub_track_of.map(|(parent, _)| parent))
        .collect();
    // `HashSet::insert` reports whether the id is new, which is precisely
    // "pulled in for the fan-out and for nothing else".
    parents
        .into_iter()
        .filter(|&parent| set.insert(parent))
        .collect()
}

/// Refuse a stem whose sub-track audio would have to come out of a
/// FROZEN parent's cache (ba todo #1248).
///
/// Freezing a multi-output instrument bakes its whole fan-out into ONE
/// cache file: `freeze_raw` forces every sub-track into master at unity
/// (`render_core`'s `force_master_route`) so the parent's cache carries
/// the summed kit, and on playback the frozen branch fills the parent's
/// buffer from that cache and never runs the instrument — so
/// `extra_ports_filled` stays 0 and the sub-tracks produce nothing at
/// all. While a parent is frozen its taps have no independent signal;
/// that is a property of the freeze design (doc #187), not an oversight
/// here.
///
/// So a stem or measurement of one such tap has no honest answer.
/// Before ba todo #1242 it returned silence; #1242's fan-out fix made it
/// return the frozen parent's cache, i.e. THE WHOLE KIT under the name
/// of one tap — plausible, confident and wrong, which this arc has
/// repeatedly established is the worse failure. `discard_own_output`
/// cannot help: it lives in the instrument arm of the per-track loop,
/// which a frozen track never reaches, and zeroing the frozen buffer
/// instead would delete the tap's own signal along with its siblings'.
///
/// Of the three options ba todo #1248 lists this is (b), and it is
/// enforced HERE rather than at the control layer so that stem export
/// and `render.stems` are covered by the same rule as `meter.measure` —
/// the defect is in the render path, and every caller of that path
/// deserves the answer. It is also the only option that leaves doc
/// #187's parity invariant untouched: nothing about how a frozen track
/// PLAYS changes, an offline request simply gets an error instead of a
/// wrong buffer. Option (a) (fall back to unfrozen rendering) would
/// measure audio the user is not hearing whenever the cache is stale,
/// and option (c) (unfreeze on demand) would mutate the project to
/// answer a read-only question.
///
/// Only fan-out parents are checked: `StemSource::Track(frozen_parent)`
/// is still perfectly renderable — the cache IS that instrument's whole
/// output — and is deliberately left alone.
fn frozen_fan_out_refusal(
    filter: &StemFilter,
    tracks: &IndexMap<TrackId, Track>,
) -> Option<String> {
    // Walk `tracks` rather than the `HashSet` so the message is
    // deterministic when more than one frozen parent is involved.
    let (parent_id, parent) = tracks
        .iter()
        .find(|(id, t)| {
            filter.fan_out_only.contains(id) && t.frozen_source.load_full().is_some()
        })?;
    let taps: Vec<&str> = tracks
        .values()
        .filter(|t| {
            t.sub_track_of.map(|(p, _)| p) == Some(*parent_id) && filter.contains(t.id)
        })
        .map(|t| t.name.as_str())
        .collect();
    let which = match taps.as_slice() {
        [] => String::new(),
        [one] => format!(" (\"{one}\")"),
        many => format!(" ({})", many.join(", ")),
    };
    Some(format!(
        "Track {parent_id} (\"{}\") is frozen, and its freeze cache holds the \
         whole instrument as one signal — the sub-track{which} cannot be \
         separated out of it. Unfreeze track {parent_id} to render or measure \
         its individual outputs.",
        parent.name
    ))
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
    midi_clips: &[Arc<MidiClip>],
    tempo_map: &Arc<arc_swap::ArcSwap<TempoMap>>,
    sample_rate: u32,
) -> Option<(SamplePos, SamplePos)> {
    let clips_guard = clips.read();
    let midi_guard = midi_clips;
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
    plugins: &Arc<RwLock<PluginMap>>,
    tempo_map: &Arc<arc_swap::ArcSwap<TempoMap>>,
    sample_rate: u32,
) -> Result<Vec<f32>, StemError> {
    // Same guard as the other offline renderers: rendering while the
    // transport rolls would interleave shared plugin process()/reset
    // calls with live playback and corrupt both outputs.
    if shared.playing.load(Ordering::Relaxed) {
        return Err(StemError::TransportRunning);
    }
    if render_end <= render_start {
        return Err(StemError::EmptyRange);
    }

    // Refresh vocal-tuning render caches so each stem mixes corrected audio
    // for any retuned clip, identical to live playback (todo #358).
    super::super::vocal_render::ensure_tuning_caches(clips, sample_rate);

    let filter = {
        let tracks_guard = tracks.read();
        // Key sources are part of the render even when they are not part
        // of the stem (ba doc #277) — without them every keyed plugin in
        // the slice silently falls back to its own input.
        let routes = shared.sidechain_routes.load();
        let filter =
            stem_filter_with_keys(source, &tracks_guard, &busses.read(), &routes);
        // A tap whose parent is FROZEN has no separable signal at all
        // (ba todo #1248) — refuse rather than hand back the whole kit.
        if let Some(message) = frozen_fan_out_refusal(&filter, &tracks_guard) {
            return Err(StemError::FrozenFanOut(message));
        }
        filter
    };
    // The master stem honours mute/solo (it is the real mix); isolated
    // track/bus stems render their source regardless of mute/solo.
    let respect_mute_solo = filter.include_master_fx;

    reset_plugins(plugins, shared);

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
        plugins,
        tempo_map: &bounce_tm,
        automation: &automation,
        sample_rate,
        master_vol,
        latency_comp: &latency_comp,
        hard_clip: true,
    };
    let mut scratch = ChunkScratch::new();

    let total_frames = (render_end - render_start) as usize;
    let mut output = vec![0.0f32; total_frames * 2];

    let in_filter = |id: TrackId| filter.contains(id);
    let fan_out_only = |id: TrackId| filter.is_fan_out_only(id);
    // Key sources render so they can be captured, and are dropped before
    // the fader so they key this stem without joining it (ba doc #277).
    let key_only = |id: TrackId| filter.is_key_only(id);
    let key_only_bus = |id: BusId| filter.is_key_only_bus(id);
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
            &fan_out_only,
            &key_only,
            &key_only_bus,
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
) -> Result<(), StemError> {
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
    // Temp file + rename on success (code review ENG-13): a failed write
    // leaves any previous file at `path` intact and no partial behind.
    let output = super::PartialFile::new(path);
    let mut writer = hound::WavWriter::create(output.temp(), spec).map_err(StemError::Create)?;

    match bit_depth {
        StemBitDepth::Float32 => {
            for &s in pcm {
                writer.write_sample(s).map_err(StemError::Write)?;
            }
        }
        StemBitDepth::Int16 => {
            for &s in pcm {
                let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16;
                writer.write_sample(v).map_err(StemError::Write)?;
            }
        }
        StemBitDepth::Int24 => {
            const MAX_24: f32 = 8_388_607.0; // 2^23 - 1
            for &s in pcm {
                let v = (s.clamp(-1.0, 1.0) * MAX_24).round() as i32;
                writer.write_sample(v).map_err(StemError::Write)?;
            }
        }
    }

    writer.finalize().map_err(StemError::Finalize)?;
    output.commit()?;
    Ok(())
}
