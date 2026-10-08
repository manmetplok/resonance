//! Sub-clip render units + a content-addressed render cache so a vocal
//! clip only re-renders the *segments* an edit actually touched.
//!
//! `build_segment` (see [`super::segment`]) emits a single `DsSegment`
//! covering the whole clip, and the old render path fed that one segment
//! to the acoustic + vocoder pipeline wholesale — so editing a single
//! syllable re-ran the model over every note. This module splits the
//! clip into independently-renderable [`RenderUnit`]s at the same genuine
//! silences (`SP` rests) the duration builder already inserts, renders
//! only the units whose content changed, reuses cached audio for the
//! rest, and stitches everything back onto one timeline by sample
//! offset. Because units only ever abut at silence, concatenating their
//! independently-rendered audio is seamless.
//!
//! Change tracking is *content-addressed* rather than flag-based: each
//! unit's cache key is a hash of everything that determines its rendered
//! audio (phonemes, durations, f0/gender/tension curves, and the model,
//! speaker and bank version the lane's params resolve to). An edit that changes a syllable changes that unit's
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
use resonance_svs::predict::{BankModelPaths, BankScore};

use super::dynamics::GainSpan;
use super::registry::BankProfile;
use super::rests::RestRule;
use super::segment::{build_unit_segment, unit_curve_gain_db, CURVE_FRAME_SEC};

/// One independently-renderable slice of a vocal clip: a contiguous run
/// of notes delimited by genuine silences. Carries its own `DsSegment`
/// (with leading/trailing `SP` pads, same as the whole-clip builder), the
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
    /// What the lane's render state records for this unit: [`key`] plus
    /// what the take depends on that the cache key leaves out — where the
    /// unit sits (its first note's clip tick, so moving the whole line
    /// moves the installed audio out of place) and, when
    /// `velocity_dynamics` is on, its notes' raw velocities (the post gain
    /// reads them after the cache).
    ///
    /// [`key`]: RenderUnit::key
    pub state_key: u64,
    /// The Dynamics curve's post-render gain over this unit, on a bank
    /// with no energy input (§10): one span per curve frame, in seconds
    /// from the start of the unit's audio. Applied after the cache, so
    /// [`state_key`](RenderUnit::state_key) hashes it. Empty when there is
    /// none.
    pub curve_gain: Vec<GainSpan>,
    /// The seed of every model's noise for this unit: a hash of what the
    /// models are fed (the key without the lane params that act after
    /// them), so the same input renders the same samples.
    pub seed: u64,
    /// The unit as the bank's own duration / pitch / variance models see
    /// it; what the render re-times and re-pitches the segment from when
    /// the lane uses them (`bank_timing` / `bank_pitch`).
    pub score: BankScore,
    /// The Pitch bend curve in cents per curve frame from the segment
    /// start (empty without points), laid over the bank's pitch.
    pub bend_cents: Vec<f64>,
    /// The bank models this unit renders with (none on the rule path).
    pub bank_models: BankModelPaths,
}

/// Seconds-per-tick at the given tempo. Mirrors the duration builder so
/// unit start times line up sample-for-sample with the phoneme layout.
fn seconds_per_tick(ticks_per_quarter: u32, bpm: f32) -> f64 {
    60.0 / (bpm.max(1.0) as f64 * ticks_per_quarter as f64)
}

/// Split a vocal clip into render units at genuine silences.
///
/// A boundary is placed between note `i` and `i+1` whenever the gap to
/// the next note is at least the lane's rest threshold
/// ([`RestRule`], `rest_threshold_beats`) — exactly the condition under
/// which [`super::segment`]'s duration builder emits a trailing rest —
/// unless the next note's syllable asks for a breath, which then fills
/// the end of that rest inside the same unit. Each
/// resulting unit is a self-contained `DsSegment` built from its slice of
/// notes/syllables. A clip with no internal silence yields a single unit
/// whose segment is identical to the old whole-clip build, so the
/// common continuous-phrase case is unchanged.
#[allow(clippy::too_many_arguments)]
pub fn split_render_units(
    notes: &[MidiNote],
    params: &VocalParams,
    assigned: &[AssignedSyllable],
    ticks_per_quarter: u32,
    bpm: f32,
    bank: &BankProfile,
) -> Vec<RenderUnit> {
    if notes.is_empty() {
        return Vec::new();
    }
    let spt = seconds_per_tick(ticks_per_quarter, bpm);
    let base_tick = notes[0].start_tick;
    let model = model_identity(params, bank);
    let lane = lane_identity(params);
    let active = bank.active_bank_models(params.bank_timing, params.bank_pitch);
    let rest_rule = RestRule::new(params, bpm);

    // Boundaries between consecutive notes: split where the duration
    // builder would insert a real rest.
    let mut starts = vec![0usize];
    for i in 0..notes.len().saturating_sub(1) {
        // Unfloored, like the duration builder (VIEW-34); a slot under
        // the floor can never hold a rest anyway.
        let slot_sec =
            (notes[i + 1].start_tick.saturating_sub(notes[i].start_tick)) as f64 * spt;
        let sing_cap = (notes[i].duration_ticks as f64 * spt).max(0.05);
        // A rest ending in a written breath stays inside one unit, so the
        // breath can take the end of the gap (up to 0.35 s) rather than
        // only the next unit's leading pad.
        let breath_next = assigned.get(i + 1).is_some_and(|a| a.breath_before);
        if rest_rule.is_rest(slot_sec, sing_cap) && !breath_next {
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
        // Each unit's segment samples the expression curves from its own
        // first note (section-relative ticks), so a curve point lands on
        // the same beat whichever unit sings it.
        let built = build_unit_segment(
            &notes[lo..hi],
            params,
            &assigned[lo..hi],
            ticks_per_quarter,
            bpm,
            bank,
        );
        let segment = built.segment;
        let gain_db = unit_curve_gain_db(
            &notes[lo..hi],
            params,
            ticks_per_quarter,
            bpm,
            bank,
            segment.f0.samples.len(),
        );
        let start_sec = (notes[lo].start_tick.saturating_sub(base_tick)) as f64 * spt;
        let syllable_range = assigned[lo].syllable_index..assigned[hi - 1].syllable_index + 1;
        let key = content_key(&segment, &model, &lane, &active, &built.score, &built.bend_cents);
        // The models' noise is seeded from what they are fed, not from the
        // lane params that only shape the take after them
        // (`velocity_dynamics` on a bank without energy): changing one of
        // those re-renders the same audio.
        let seed = content_key(&segment, &model, "", &active, &built.score, &built.bend_cents);
        let state_key = state_key(key, &notes[lo..hi], params.velocity_dynamics, &gain_db);
        let curve_gain = gain_db
            .iter()
            .enumerate()
            .map(|(i, &gain_db)| GainSpan {
                start_sec: i as f64 * CURVE_FRAME_SEC,
                end_sec: (i + 1) as f64 * CURVE_FRAME_SEC,
                gain_db,
            })
            .collect();
        units.push(RenderUnit {
            segment,
            start_sec,
            note_range: lo..hi,
            syllable_range,
            key,
            state_key,
            curve_gain,
            seed,
            score: built.score,
            bend_cents: built.bend_cents,
            bank_models: active.clone(),
        });
    }
    units
}

/// What renders a unit, beyond the segment itself: the files the render
/// loads (acoustic and vocoder config), the speaker key the lane's speaker
/// resolves to, and the bank's version — so a rescan that resolves the
/// same params to another model, speaker or release re-renders instead of
/// reusing audio from the old one. A bank that is not installed cannot
/// render; its id and speaker stand in (the render fails before the cache
/// is consulted).
pub(crate) fn model_identity(params: &VocalParams, bank: &BankProfile) -> String {
    match bank.render_target(params.speaker.as_deref(), params.voice, None) {
        Ok(target) => {
            let active = bank.active_bank_models(params.bank_timing, params.bank_pitch);
            let path = |p: &Option<std::path::PathBuf>| {
                p.as_ref().map_or("-".to_owned(), |p| p.display().to_string())
            };
            format!(
                "{}|{}|{:?}|{:?}|dur={}|pitch={}|var={}",
                target.acoustic_config.display(),
                target.vocoder_config.display(),
                target.speaker,
                bank.installed().and_then(|b| b.version.as_deref()),
                path(&active.duration),
                path(&active.pitch),
                path(&active.variance),
            )
        }
        Err(_) => format!("{}|{:?}", params.voicebank, params.speaker),
    }
}

/// The lane params a unit's audio depends on beyond what its segment
/// arrays show: `rest_threshold_beats` (where the unit's rests fall) and
/// `velocity_dynamics` (the post-vocoder gain and, on a bank with energy,
/// the energy curve). Hashed into every unit's key, so changing either
/// re-renders the lane and its plan hash moves.
fn lane_identity(params: &VocalParams) -> String {
    format!(
        "rest={}|vel_dyn={}",
        params.rest_threshold_beats.to_bits(),
        params.velocity_dynamics.to_bits()
    )
}

/// FNV-1a hash of everything that determines a unit's rendered audio:
/// the phoneme/note/duration arrays, every per-frame curve, and the
/// [`model_identity`] that renders them (the acoustic/vocoder model, the
/// speaker embedding, the bank version and the bank's own models the lane
/// uses — not captured by the segment arrays themselves). Two units with
/// the same key render bit-identical audio (the models' noise is seeded
/// from the key), so a cache hit is always safe to reuse.
///
/// The key never needs a model run: the bank models' output is a pure
/// function of the [`BankScore`] (hashed here whenever one of them is in
/// use), the speaker, the models and the seed. When the bank's pitch model
/// replaces the synthesized f0, the f0 curve (portamento, vibrato) does
/// not reach the take, so the key hashes the pitch bend that rides on the
/// bank's pitch instead.
fn content_key(
    seg: &DsSegment,
    model: &str,
    lane: &str,
    active: &BankModelPaths,
    score: &BankScore,
    bend_cents: &[f64],
) -> u64 {
    let mut h = Fnv::new();
    // The segment arrays already fold in the voicebank's phoneme
    // substitutions, but not which model renders them.
    h.str(model);
    h.str(lane);
    for s in &seg.ph_seq {
        h.str(s);
    }
    h.f64s(&seg.ph_dur);
    h.i32s(&seg.note_seq_midi);
    h.f64s(&seg.note_dur);
    if active.pitch.is_some() {
        h.bytes(&[0xb1]);
        h.f64s(bend_cents);
    } else {
        h.curve(&seg.f0);
    }
    if active.duration.is_some() || active.pitch.is_some() {
        h.bytes(&[0xb0]);
        h.score(score);
    }
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

/// The render-state key of a unit whose cache key is `key` and whose
/// notes are `notes`: the cache key is position-free (a unit moved in time
/// reuses its audio), but the installed take is not — its units are
/// stitched at their offsets and the take is placed by the lane's first
/// note — so the state key also hashes the unit's first clip tick, which
/// carries both the unit's offset from the first note and the lead-in.
/// With `velocity_dynamics` above 0 it hashes the notes' velocities too:
/// the post-vocoder gain reads them after the cache, so on a bank whose
/// segment ignores velocity a velocity edit changes no cache key. Same for
/// the Dynamics curve's post-render gain (`curve_gain_db`, empty when it
/// has none): a curve edit on a bank without energy moves no cache key.
fn state_key(key: u64, notes: &[MidiNote], velocity_dynamics: f32, curve_gain_db: &[f32]) -> u64 {
    let mut h = Fnv::new();
    h.bytes(&key.to_le_bytes());
    let first_tick = notes.first().map_or(0, |n| n.start_tick);
    h.bytes(&first_tick.to_le_bytes());
    if velocity_dynamics > 0.0 {
        for n in notes {
            h.bytes(&n.velocity.to_bits().to_le_bytes());
        }
    }
    if !curve_gain_db.is_empty() {
        h.bytes(&[0xfc]);
        for g in curve_gain_db {
            h.bytes(&g.to_bits().to_le_bytes());
        }
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
    /// The first unit's [`UnitAudio::lead_sec`]: what is left of the
    /// take's leading pad before it starts singing.
    pub first_lead_sec: f64,
}

/// Content-addressed cache of per-unit rendered audio. Lives across
/// renders of one vocal clip (keyed `(definition, track)` by the caller)
/// so an edit only pays for the units it touched. Stale entries — units
/// that no longer exist after an edit — are evicted on every pass, so the
/// cache stays bounded to the clip's current segment count.
#[derive(Debug, Default)]
pub struct SvsRenderCache {
    /// Unit content key → rendered mono audio at [`model_sr`] and the
    /// silence its leading pad kept before the first phoneme (seconds).
    entries: HashMap<u64, (Vec<f32>, f64)>,
    /// Sample rate of every cached buffer. Set on first render; all units
    /// of a clip share one voicebank and therefore one rate.
    model_sr: Option<u32>,
    /// Tally from the most recent [`render_units_cached`] pass, for the
    /// UI overlay to read. `None` until the clip has been rendered once.
    last_plan: Option<RenderPlan>,
    /// The unit state keys ([`RenderUnit::state_key`]) of the most recent
    /// pass, in unit order: the take it stitched, by content and
    /// position. Recorded with the installed audio so the lane's render
    /// state can tell whether that take is still current.
    last_keys: Vec<u64>,
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

    /// The unit state keys of the take the last pass stitched, in unit
    /// order.
    pub fn last_keys(&self) -> &[u64] {
        &self.last_keys
    }

    /// Number of cached unit buffers currently held.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// One unit's rendered audio.
#[derive(Debug, Clone)]
pub struct UnitAudio {
    pub mono: Vec<f32>,
    pub sample_rate: u32,
    /// Seconds of silence the unit's leading pad kept before its first
    /// phoneme — the rest of the pad is pre-roll the first unit sings
    /// before its first note. The rule layout's leading `SP`, or what the
    /// bank's duration model left of it.
    pub lead_sec: f64,
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
/// reflects the hit/miss tally. A unit's lead is its segment's leading
/// `SP`; [`render_units_cached_with`] lets the renderer report its own.
pub fn render_units_cached(
    units: &[RenderUnit],
    cache: &mut SvsRenderCache,
    mut render_fn: impl FnMut(&DsSegment) -> Result<(Vec<f32>, u32), String>,
) -> Result<StitchedRender, String> {
    render_units_cached_with(units, cache, |unit| {
        let (mono, sample_rate) = render_fn(&unit.segment)?;
        Ok(UnitAudio {
            mono,
            sample_rate,
            lead_sec: unit.segment.ph_dur.first().copied().unwrap_or(0.0),
        })
    })
}

/// [`render_units_cached`] with a renderer that takes the whole unit and
/// reports where its first phoneme starts ([`UnitAudio::lead_sec`]).
pub fn render_units_cached_with(
    units: &[RenderUnit],
    cache: &mut SvsRenderCache,
    mut render_fn: impl FnMut(&RenderUnit) -> Result<UnitAudio, String>,
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
    let mut first_lead_sec = 0.0;

    for (i, unit) in units.iter().enumerate() {
        let lead = if let Some((buf, lead)) = cache.entries.get(&unit.key) {
            plan.reused += 1;
            per_unit.push(buf.clone());
            *lead
        } else {
            let audio = render_fn(unit)?;
            // Every unit of a clip shares one voicebank, hence one rate;
            // the first render this pass fixes it.
            call_sr.get_or_insert(audio.sample_rate);
            cache
                .entries
                .insert(unit.key, (audio.mono.clone(), audio.lead_sec));
            plan.changed += 1;
            per_unit.push(audio.mono);
            audio.lead_sec
        };
        if i == 0 {
            first_lead_sec = lead;
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
    cache.last_keys = units.iter().map(|u| u.state_key).collect();

    Ok(StitchedRender {
        mono,
        sample_rate: sr,
        plan,
        first_lead_sec,
    })
}

/// The unit state keys a render of this clip would record, in unit order
/// — [`split_render_units`]'s [`RenderUnit::state_key`]s without keeping
/// the segments. What the lane's render state compares with the installed
/// take's [`SvsRenderCache::last_keys`].
#[allow(clippy::too_many_arguments)]
pub fn plan_unit_keys(
    notes: &[MidiNote],
    params: &VocalParams,
    assigned: &[AssignedSyllable],
    ticks_per_quarter: u32,
    bpm: f32,
    bank: &BankProfile,
) -> Vec<u64> {
    split_render_units(notes, params, assigned, ticks_per_quarter, bpm, bank)
        .into_iter()
        .map(|u| u.state_key)
        .collect()
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
    fn score(&mut self, score: &BankScore) {
        for p in &score.phones {
            self.str(&p.symbol);
            self.bytes(&p.language.to_le_bytes());
            self.bytes(&(p.group as u64).to_le_bytes());
            self.bytes(&p.rule_sec.to_bits().to_le_bytes());
        }
        self.bytes(&[0xfa]);
        for g in &score.groups {
            self.bytes(&g.start_sec.to_bits().to_le_bytes());
            self.bytes(&g.midi.to_le_bytes());
        }
        self.bytes(&[0xf9]);
        for n in &score.notes {
            self.bytes(&n.start_sec.to_bits().to_le_bytes());
            self.bytes(&n.end_sec.to_bits().to_le_bytes());
            self.bytes(&n.midi.to_bits().to_le_bytes());
            self.bytes(&[n.slur as u8, n.voiced as u8]);
        }
        self.bytes(&score.total_sec.to_bits().to_le_bytes());
        self.bytes(&score.lead_max_sec.to_bits().to_le_bytes());
    }
    fn curve(&mut self, c: &SampleCurve) {
        self.f64s(&c.samples);
        self.bytes(&c.timestep.to_bits().to_le_bytes());
    }
    fn finish(&self) -> u64 {
        self.0
    }
}
