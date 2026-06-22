//! Sub-clip render units + a content-addressed render cache so a vocal
//! clip only re-renders the *segments* an edit actually touched.
//!
//! `build_segment` (see [`super::segment`]) emits a single `DsSegment`
//! covering the whole clip, and the old render path fed that one segment
//! to the acoustic + vocoder pipeline wholesale — so editing a single
//! syllable re-ran the model over every note. This module splits the
//! clip into independently-renderable [`RenderUnit`]s at the same genuine
//! silences (`AP` rests) the duration builder already inserts, renders
//! only the units whose content changed, reuses cached audio for the
//! rest, and stitches everything back onto one timeline by sample
//! offset. Because units only ever abut at silence, concatenating their
//! independently-rendered audio is seamless.
//!
//! Change tracking is *content-addressed* rather than flag-based: each
//! unit's cache key is a hash of everything that determines its rendered
//! audio (phonemes, durations, f0/gender/tension curves, and the active
//! voicebank/singer). An edit that changes a syllable changes that unit's
//! key and nothing else, so the cache lookup alone decides what to
//! re-render — no separate per-syllable dirty bit to keep in sync. The
//! "N of M segments changed" figure the UI overlay wants (design #173
//! state 5) falls straight out of the hit/miss counts; see
//! [`RenderPlan`] and [`SvsRenderCache::last_plan`].

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use resonance_audio::types::MidiNote;
use resonance_music_theory::g2p::AssignedSyllable;
use resonance_music_theory::VocalParams;
use resonance_svs::ds::{DsSegment, SampleCurve};

use super::segment::build_segment;
use super::SILENCE_GAP_SEC;

/// One independently-renderable slice of a vocal clip: a contiguous run
/// of notes delimited by genuine silences. Carries its own `DsSegment`
/// (with leading/trailing `AP` pads, same as the whole-clip builder), the
/// note/syllable ranges it covers, the clip-relative start time its first
/// phoneme should land at, and the content [`key`](RenderUnit::key) that
/// addresses it in the cache.
#[derive(Debug, Clone)]
pub struct RenderUnit {
    /// The score for this unit. `offset` is left at `0.0`; placement onto
    /// the clip timeline is done by [`stitch`] using [`start_sec`].
    pub segment: DsSegment,
    /// Seconds from the clip's first note to this unit's first note.
    /// Drives where the unit's audio is mixed onto the final timeline.
    pub start_sec: f64,
    /// Half-open range of note indices (into the clip's note list) this
    /// unit covers.
    pub note_range: Range<usize>,
    /// Half-open range of resolved-syllable indices this unit covers.
    /// Lets the UI map a syllable edit back to the unit it dirtied.
    pub syllable_range: Range<usize>,
    /// Content hash addressing this unit's rendered audio in the cache.
    pub key: u64,
}

/// Seconds-per-tick at the given tempo. Mirrors the duration builder so
/// unit start times line up sample-for-sample with the phoneme layout.
fn seconds_per_tick(ticks_per_quarter: u32, bpm: f32) -> f64 {
    60.0 / (bpm.max(1.0) as f64 * ticks_per_quarter as f64)
}

/// Split a vocal clip into render units at genuine silences.
///
/// A boundary is placed between note `i` and `i+1` whenever the gap to
/// the next note exceeds the note's own sing duration by more than
/// [`SILENCE_GAP_SEC`] — exactly the condition under which
/// [`super::segment`]'s duration builder emits a trailing `AP` rest. Each
/// resulting unit is a self-contained `DsSegment` built from its slice of
/// notes/syllables. A clip with no internal silence yields a single unit
/// whose segment is identical to the old whole-clip build, so the
/// common continuous-phrase case is unchanged.
pub fn split_render_units(
    notes: &[MidiNote],
    params: &VocalParams,
    assigned: &[AssignedSyllable],
    ticks_per_quarter: u32,
    bpm: f32,
) -> Vec<RenderUnit> {
    if notes.is_empty() {
        return Vec::new();
    }
    let spt = seconds_per_tick(ticks_per_quarter, bpm);
    let base_tick = notes[0].start_tick;

    // Boundaries between consecutive notes: split where the duration
    // builder would insert a real rest.
    let mut starts = vec![0usize];
    for i in 0..notes.len().saturating_sub(1) {
        let slot_sec = ((notes[i + 1].start_tick.saturating_sub(notes[i].start_tick)) as f64
            * spt)
            .max(0.05);
        let sing_cap = (notes[i].duration_ticks as f64 * spt).max(0.05);
        if slot_sec > sing_cap + SILENCE_GAP_SEC {
            starts.push(i + 1);
        }
    }
    starts.push(notes.len());

    let mut units = Vec::with_capacity(starts.len() - 1);
    for w in starts.windows(2) {
        let (lo, hi) = (w[0], w[1]);
        if lo >= hi {
            continue;
        }
        let segment = build_segment(&notes[lo..hi], params, &assigned[lo..hi], ticks_per_quarter, bpm);
        let start_sec = (notes[lo].start_tick.saturating_sub(base_tick)) as f64 * spt;
        let syllable_range = assigned[lo].syllable_index..assigned[hi - 1].syllable_index + 1;
        let key = content_key(&segment, params);
        units.push(RenderUnit {
            segment,
            start_sec,
            note_range: lo..hi,
            syllable_range,
            key,
        });
    }
    units
}

/// FNV-1a hash of everything that determines a unit's rendered audio:
/// the phoneme/note/duration arrays, every per-frame curve, and the
/// active voicebank + singer (which select the acoustic/vocoder model and
/// speaker embedding — not captured by the segment arrays themselves).
/// Two units with the same key render bit-identical audio, so a cache hit
/// is always safe to reuse.
fn content_key(seg: &DsSegment, params: &VocalParams) -> u64 {
    let mut h = Fnv::new();
    // Model + speaker identity. The segment arrays already fold in the
    // voicebank's phoneme substitutions, but not which model renders them.
    h.str(&format!(
        "{:?}|{:?}|{:?}",
        params.voicebank, params.singer, params.singer_meiji
    ));
    for s in &seg.ph_seq {
        h.str(s);
    }
    h.f64s(&seg.ph_dur);
    h.i32s(&seg.note_seq_midi);
    h.f64s(&seg.note_dur);
    h.curve(&seg.f0);
    h.curve(&seg.gender);
    h.curve(&seg.velocity);
    h.curve(&seg.energy);
    h.curve(&seg.breathiness);
    h.curve(&seg.voicing);
    h.curve(&seg.tension);
    for &l in &seg.languages {
        h.bytes(&l.to_le_bytes());
    }
    h.finish()
}

/// Tally of what one render pass did: how many of the clip's `total`
/// units changed (re-rendered) versus were reused from cache. This is the
/// "N of M segments changed" figure design #173 state 5 surfaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RenderPlan {
    pub total: usize,
    pub changed: usize,
    pub reused: usize,
}

/// Output of [`render_units_cached`]: the stitched mono waveform plus the
/// reuse tally for the pass.
#[derive(Debug, Clone)]
pub struct StitchedRender {
    pub mono: Vec<f32>,
    pub sample_rate: u32,
    pub plan: RenderPlan,
}

/// Content-addressed cache of per-unit rendered audio. Lives across
/// renders of one vocal clip (keyed `(definition, track)` by the caller)
/// so an edit only pays for the units it touched. Stale entries — units
/// that no longer exist after an edit — are evicted on every pass, so the
/// cache stays bounded to the clip's current segment count.
#[derive(Debug, Default)]
pub struct SvsRenderCache {
    /// Unit content key → rendered mono audio at [`model_sr`].
    entries: HashMap<u64, Vec<f32>>,
    /// Sample rate of every cached buffer. Set on first render; all units
    /// of a clip share one voicebank and therefore one rate.
    model_sr: Option<u32>,
    /// Tally from the most recent [`render_units_cached`] pass, for the
    /// UI overlay to read. `None` until the clip has been rendered once.
    last_plan: Option<RenderPlan>,
}

impl SvsRenderCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// The "N of M segments changed" tally from the last render, or `None`
    /// if this clip hasn't been rendered yet.
    pub fn last_plan(&self) -> Option<RenderPlan> {
        self.last_plan
    }

    /// Number of cached unit buffers currently held.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Render a clip's units, reusing cached audio for unchanged units and
/// invoking `render_fn` only for the ones whose content key isn't in the
/// cache, then stitch every unit's audio onto one mono timeline by its
/// `start_sec` offset.
///
/// `render_fn` produces the mono waveform (and its sample rate) for a
/// single unit's segment — in production it runs the acoustic + vocoder
/// pipeline; tests pass a stub. After the pass the cache holds exactly the
/// current units' audio (stale entries evicted) and [`SvsRenderCache::last_plan`]
/// reflects the hit/miss tally.
pub fn render_units_cached(
    units: &[RenderUnit],
    cache: &mut SvsRenderCache,
    mut render_fn: impl FnMut(&DsSegment) -> Result<(Vec<f32>, u32), String>,
) -> Result<StitchedRender, String> {
    if units.is_empty() {
        return Err("no render units".to_string());
    }

    let mut plan = RenderPlan {
        total: units.len(),
        ..RenderPlan::default()
    };
    let mut call_sr: Option<u32> = None;
    let mut per_unit: Vec<Vec<f32>> = Vec::with_capacity(units.len());

    for unit in units {
        if let Some(buf) = cache.entries.get(&unit.key) {
            plan.reused += 1;
            per_unit.push(buf.clone());
        } else {
            let (mono, sr) = render_fn(&unit.segment)?;
            // Every unit of a clip shares one voicebank, hence one rate;
            // the first render this pass fixes it.
            call_sr.get_or_insert(sr);
            cache.entries.insert(unit.key, mono.clone());
            plan.changed += 1;
            per_unit.push(mono);
        }
    }

    // Rate comes from this pass's renders, or — if every unit was a cache
    // hit — from the stored rate of the buffers we just reused.
    let sr = call_sr
        .or(cache.model_sr)
        .ok_or("render cache has no sample rate")?;

    let mono = stitch(units, &per_unit, sr);

    // Evict units that no longer exist so the cache tracks the clip's
    // current segmentation rather than growing without bound.
    let live: HashSet<u64> = units.iter().map(|u| u.key).collect();
    cache.entries.retain(|k, _| live.contains(k));
    cache.model_sr = Some(sr);
    cache.last_plan = Some(plan);

    Ok(StitchedRender {
        mono,
        sample_rate: sr,
        plan,
    })
}

/// Mix each unit's audio onto a single mono timeline at the sample offset
/// implied by its `start_sec`. Units only overlap inside silence pads
/// (boundaries are genuine rests), so summing is seamless; the resulting
/// layout matches what a single whole-clip render would have produced.
fn stitch(units: &[RenderUnit], per_unit: &[Vec<f32>], sr: u32) -> Vec<f32> {
    let mut placements: Vec<(usize, &Vec<f32>)> = Vec::with_capacity(units.len());
    let mut total_len = 0usize;
    for (unit, buf) in units.iter().zip(per_unit) {
        let offset = (unit.start_sec * sr as f64).round().max(0.0) as usize;
        total_len = total_len.max(offset + buf.len());
        placements.push((offset, buf));
    }
    let mut out = vec![0.0f32; total_len];
    for (offset, buf) in placements {
        for (i, &s) in buf.iter().enumerate() {
            out[offset + i] += s;
        }
    }
    out
}

/// Minimal 64-bit FNV-1a accumulator. A fixed, well-distributed hash with
/// no per-process seed, so keys are stable across the clip's render
/// passes (unlike `DefaultHasher`'s randomised SipHash) — required for the
/// cache to recognise an unchanged unit between renders.
struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Fnv(0xcbf29ce484222325)
    }
    fn bytes(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= b as u64;
            self.0 = self.0.wrapping_mul(0x100000001b3);
        }
    }
    fn str(&mut self, s: &str) {
        self.bytes(s.as_bytes());
        self.bytes(&[0xff]); // separator so "ab"+"c" != "a"+"bc"
    }
    fn f64s(&mut self, xs: &[f64]) {
        for &x in xs {
            self.bytes(&x.to_bits().to_le_bytes());
        }
        self.bytes(&[0xfe]);
    }
    fn i32s(&mut self, xs: &[i32]) {
        for &x in xs {
            self.bytes(&x.to_le_bytes());
        }
        self.bytes(&[0xfd]);
    }
    fn curve(&mut self, c: &SampleCurve) {
        self.f64s(&c.samples);
        self.bytes(&c.timestep.to_bits().to_le_bytes());
    }
    fn finish(&self) -> u64 {
        self.0
    }
}
