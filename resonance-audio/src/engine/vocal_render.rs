//! Vocal-tuning retune for the render + bounce path (doc #160, todo #358).
//!
//! Turns a clip's non-destructive [`VocalTuning`] edits into corrected PCM
//! that the shared mixer reads. The work is split so the realtime thread
//! never pays for it:
//!
//! * [`ensure_tuning_caches`] runs **off** the realtime thread (the offline
//!   bounce / export / stem / freeze workers), in three phases:
//!   [`snapshot_tuning_jobs`] picks the tuned clips out of the published
//!   render graph (no lock: the clip `Arc`s pin their audio),
//!   [`build_tuning_caches`] runs the formant-preserving resynthesis from
//!   `resonance-dsp` on the worker, and the result is used twice. The
//!   worker renders through it at once — a [`TuningOverlay`] that patches
//!   each chunk's clip list with the fresh caches — and posts it to the
//!   engine thread ([`EngineInternal::TuningCachesBuilt`]), which attaches
//!   it to the live clips ([`apply_tuning_caches`] →
//!   [`attach_tuning_caches`]), so later playback reads it too. Until
//!   ARCH-02 B-5 the worker attached the caches itself under the clip
//!   list's write lock (and before A2-3 held that lock across the whole
//!   FFT pass); since B-5 only the engine thread publishes the graph.
//!   Untuned / identity clips have their cache cleared, restoring the
//!   zero-overhead path.
//! * The mixer hot path then reads [`AudioClip::render_frames`], which hands
//!   back the cached corrected buffer (or the untouched source).
//!
//! NOTE on live playback: the cache is only ever built by the four
//! offline paths (bounce / wav / stem / freeze) -- nothing on the engine
//! control thread calls [`ensure_tuning_caches`] -- so live playback
//! reflects a tuning edit only after an export has run. It is not the
//! "identical in playback and in bounce" guarantee this comment used to
//! claim. Today that is invisible because nothing in production can set
//! `has_edits()`: there is no tuning-edit `AudioCommand`, the app sends
//! only `AnalyzeClipPitch`, and the default correction amount is 0.0.
//! Whoever lands the tuning-edit handlers has to call
//! [`ensure_tuning_caches`] from the control thread (or rebuild per
//! clip on edit) to make the guarantee real.
//!
//! The original [`ClipSource`](crate::types::ClipSource) PCM is never
//! mutated — the cache is a separate owned buffer with the same frame
//! layout, so trim/fade/gain indexing is unchanged.
//!
//! # Pitch model
//!
//! The effective per-frame pitch ratio blends the *detected* pitch toward a
//! *target* by the per-note correction strength times the global correction
//! amount, with the target snapped to the in-key scale degree via
//! `resonance-music-theory`:
//!
//! ```text
//! eff          = note.correction_strength · global.correction_amount
//! target_grid  = snap(note.mean + note.semitone_offset)   // scale-snapped
//! centre_delta = (target_grid − note.mean) · eff           // how far the note moves
//! Δ(frame)     = centre_delta + dev(frame) · (drift − 1)    // keep/flatten vibrato
//! ratio(frame) = 2^(Δ(frame) / 12)
//! ```
//!
//! where `dev(frame)` is the detected cents-deviation from the note's mean
//! (its vibrato/drift). `drift = 1` keeps the natural deviation; `drift = 0`
//! flattens it toward a steady tone. With no correction and full drift the
//! ratio is unity, so an analysed-but-untouched clip resynthesises to within
//! numerical precision of the original.

use std::borrow::Cow;
use std::sync::Arc;

use resonance_dsp::FormantShifter;
use resonance_music_theory::pitch::PitchClass;
use resonance_music_theory::scale::Scale;

use crate::types::{AudioClip, ClipId, VocalTuning};

use super::internal::EngineInternal;
use super::thread::HandlerCtx;
use super::SharedState;

/// Per clip: the retune cache to install, or `None` to clear a stale one.
pub type TuningCaches = Vec<(ClipId, Option<Arc<[f32]>>)>;

/// Rebuild (or clear) the retune cache of every tuned clip in `shared`'s
/// published render graph, for an offline render about to start. Call
/// from a thread that may block — never from the realtime audio callback.
///
/// Clips whose tuning carries edits ([`VocalTuning::has_edits`]) get a
/// fresh corrected buffer; untuned clips still holding a cache get it
/// cleared. The result is posted to the engine thread (which attaches it
/// to the live clips, [`apply_tuning_caches`]) and returned as the
/// [`TuningOverlay`] the render itself reads through — the render cannot
/// wait for the engine thread's publish. Nothing is posted, and the
/// overlay is empty, when no clip is tuned or stale — the common case,
/// which never plans an FFT.
pub fn ensure_tuning_caches(shared: &SharedState, sample_rate: u32) -> TuningOverlay {
    let jobs = snapshot_tuning_jobs(&shared.graph.load().clips);
    if jobs.is_empty() {
        return TuningOverlay::default();
    }
    let caches = build_tuning_caches(jobs, sample_rate);
    shared
        .inbox
        .post(EngineInternal::TuningCachesBuilt(caches.clone()));
    TuningOverlay { caches }
}

/// One clip's share of a tuning-cache pass.
#[derive(Debug, Clone)]
pub struct TuningJob {
    pub clip_id: ClipId,
    /// The clip (its source PCM and its tuning), when the tuning carries
    /// edits; `None` for a clip that only needs its stale cache cleared.
    /// An `Arc` of the published clip: the audio is shared, not copied.
    pub retune: Option<Arc<AudioClip>>,
}

/// Phase 1: one [`TuningJob`] per clip whose cache state must change (a
/// tuned clip, or an untuned one still holding a cache). Empty when
/// nothing is tuned and nothing is stale, which keeps the common
/// (untuned) project off the FFT-shifter allocation entirely.
pub fn snapshot_tuning_jobs(clips: &[Arc<AudioClip>]) -> Vec<TuningJob> {
    clips
        .iter()
        .filter_map(|clip| match clip.vocal_tuning.as_ref() {
            Some(tuning) if tuning.has_edits() => Some(TuningJob {
                clip_id: clip.id,
                retune: Some(Arc::clone(clip)),
            }),
            _ if clip.tuning_render_cache.is_some() => Some(TuningJob {
                clip_id: clip.id,
                retune: None,
            }),
            _ => None,
        })
        .collect()
}

/// Phase 2: resynthesise every job that carries a retune. Pure over its
/// input. Returns `(clip id, cache-or-clear)` pairs.
pub fn build_tuning_caches(jobs: Vec<TuningJob>, sample_rate: u32) -> TuningCaches {
    // One shifter for the whole pass: it holds the FFT plans + window and is
    // re-entrant, so it can retune every clip without per-clip setup cost.
    // Built lazily so a pass that only clears stale caches never plans an
    // FFT.
    let mut shifter = None;
    jobs.into_iter()
        .map(|job| {
            let cache = job.retune.and_then(|clip| {
                let tuning = clip.vocal_tuning.as_ref()?;
                let shifter = shifter.get_or_insert_with(|| FormantShifter::new(sample_rate as f32));
                Some(Arc::from(retune_clip(clip.source.as_frames(), shifter, tuning)))
            });
            (job.clip_id, cache)
        })
        .collect()
}

/// Phase 3: install each built cache (or clear) on its clip in `clips` —
/// a render graph's list being edited (copy-on-write: only a clip whose
/// cache changes is copied). A clip removed since the snapshot is
/// skipped. Returns the number of caches (re)built.
pub fn attach_tuning_caches(clips: &mut [Arc<AudioClip>], built: &TuningCaches) -> usize {
    let mut rebuilt = 0;
    for (clip_id, cache) in built {
        let Some(clip) = clips.iter_mut().find(|c| c.id == *clip_id) else {
            continue;
        };
        if cache.is_some() {
            rebuilt += 1;
        }
        if !same_cache(&clip.tuning_render_cache, cache) {
            Arc::make_mut(clip).tuning_render_cache = cache.clone();
        }
    }
    rebuilt
}

fn same_cache(a: &Option<Arc<[f32]>>, b: &Option<Arc<[f32]>>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        _ => false,
    }
}

/// [`EngineInternal::TuningCachesBuilt`]: attach an offline render's
/// caches to the live clips. Engine thread. Publishes only when a clip's
/// cache actually changes.
pub(crate) fn apply_tuning_caches(ctx: &HandlerCtx, built: &TuningCaches) {
    let graph = ctx.shared.graph.load();
    let changes = built.iter().any(|(clip_id, cache)| {
        graph
            .clip(*clip_id)
            .is_some_and(|clip| !same_cache(&clip.tuning_render_cache, cache))
    });
    drop(graph);
    if changes {
        ctx.shared.edit_clips(|clips| attach_tuning_caches(clips, built));
    }
}

/// The retune caches an offline render reads through, until the engine
/// thread has attached them to the live clips.
#[derive(Debug, Default, Clone)]
pub struct TuningOverlay {
    caches: TuningCaches,
}

impl TuningOverlay {
    /// Nothing to patch: every render reads the graph's clips as they are.
    pub fn is_empty(&self) -> bool {
        self.caches.is_empty()
    }

    /// How many caches the pass (re)built.
    pub fn rebuilt(&self) -> usize {
        self.caches.iter().filter(|(_, c)| c.is_some()).count()
    }

    /// `clips` with this overlay's caches installed. Borrowed as-is when
    /// the overlay is empty (the common case: no allocation per chunk);
    /// otherwise a copy of the list in which each overlaid clip whose cache
    /// differs is a patched copy (sharing its audio).
    pub fn apply<'a>(&self, clips: &'a [Arc<AudioClip>]) -> Cow<'a, [Arc<AudioClip>]> {
        if self.caches.is_empty() {
            return Cow::Borrowed(clips);
        }
        let mut patched = clips.to_vec();
        attach_tuning_caches(&mut patched, &self.caches);
        Cow::Owned(patched)
    }
}

/// Resynthesise `source` (interleaved stereo `[l, r, …]`) into a corrected
/// interleaved-stereo buffer of the **same length**, applying `tuning`'s
/// per-frame pitch ratio with the formant-preserving `shifter`.
///
/// Mono or empty input is returned unchanged. The two channels are shifted
/// independently (each keeps its own phase coherence) by one shared ratio
/// curve, matching the single monophonic f0 contour the analysis produced.
pub fn retune_clip(source: &[f32], shifter: &FormantShifter, tuning: &VocalTuning) -> Vec<f32> {
    let total_frames = source.len() / 2;
    if total_frames == 0 {
        return source.to_vec();
    }

    // De-interleave into per-channel buffers for the shifter.
    let mut left = Vec::with_capacity(total_frames);
    let mut right = Vec::with_capacity(total_frames);
    for f in source.chunks_exact(2) {
        left.push(f[0]);
        right.push(f[1]);
    }

    let ratio_curve = pitch_ratio_curve(tuning, total_frames);
    let (l_out, r_out) = shifter.process_stereo(&left, &right, &ratio_curve);

    // Re-interleave; guard the lengths in case the shifter ever returns a
    // shorter buffer so indexing stays in bounds.
    let mut out = vec![0.0f32; total_frames * 2];
    for i in 0..total_frames {
        if let Some(&l) = l_out.get(i) {
            out[i * 2] = l;
        }
        if let Some(&r) = r_out.get(i) {
            out[i * 2 + 1] = r;
        }
    }
    out
}

/// Build the per-source-frame pitch-ratio curve (one frequency multiplier
/// per stereo frame) the [`FormantShifter`] sweeps across the clip.
///
/// Frames outside every detected note — and notes whose effective correction
/// and drift leave the pitch unchanged — get a unity ratio (`1.0`), so the
/// shifter passes those frames through untouched. Exposed (not just used
/// internally) so the model can be unit-tested without running the FFT.
pub fn pitch_ratio_curve(tuning: &VocalTuning, total_frames: usize) -> Vec<f32> {
    let mut curve = vec![1.0f32; total_frames];
    if total_frames == 0 {
        return curve;
    }

    let scale = Scale::new(
        PitchClass::from_semitone(tuning.global.key),
        tuning.global.scale.to_mode(),
    );
    let global_amount = tuning.global.correction_amount.clamp(0.0, 1.0);

    for note in &tuning.notes {
        let start = (note.start_frame as usize).min(total_frames);
        let end = (note.end_frame as usize).min(total_frames);
        if end <= start {
            continue;
        }

        let edit = note.edit;
        let eff = edit.correction_strength.clamp(0.0, 1.0) * global_amount;
        let drift = edit.drift.clamp(0.0, 1.0);

        // Scale-snapped target, then blend the note centre from detected mean
        // toward it by the effective strength.
        let target = note.mean_pitch_midi + edit.semitone_offset;
        let target_grid = scale.snap_pitch(target, 1.0);
        let centre_delta = (target_grid - note.mean_pitch_midi) * eff;

        // When the note neither moves its centre nor flattens its vibrato the
        // ratio is unity for the whole span — leave the curve untouched so the
        // shifter passes the frames through (cheaper, exact reconstruction).
        if centre_delta == 0.0 && (drift - 1.0).abs() <= f32::EPSILON {
            continue;
        }

        let span = (end - start) as f32;
        for (i, f) in (start..end).enumerate() {
            let pos = if span > 1.0 { i as f32 / (span - 1.0) } else { 0.0 };
            let dev_semitones = sample_cents(&note.cents_contour, pos) / 100.0;
            let delta = centre_delta + dev_semitones * (drift - 1.0);
            curve[f] = 2.0f32.powf(delta / 12.0);
        }
    }

    curve
}

/// Sample a note's per-analysis-frame cents-deviation contour at normalised
/// position `pos ∈ [0, 1]`, linearly interpolating between points. An empty
/// contour (deviation unknown / flat) gives `0.0` cents.
fn sample_cents(contour: &[f32], pos: f32) -> f32 {
    match contour {
        [] => 0.0,
        [only] => *only,
        _ => {
            let p = pos.clamp(0.0, 1.0) * (contour.len() - 1) as f32;
            let i = p.floor() as usize;
            if i >= contour.len() - 1 {
                contour[contour.len() - 1]
            } else {
                let frac = p - i as f32;
                contour[i] + (contour[i + 1] - contour[i]) * frac
            }
        }
    }
}
