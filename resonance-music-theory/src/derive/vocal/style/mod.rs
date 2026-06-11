//! Vocal style profiles + the shared walker.
//!
//! Every per-style vocal generator implements [`VocalStyleProfile`]; the
//! walker (`walk_with_profile`) handles the loop structure and rng-draw
//! sequencing that all six styles share. This file also holds the
//! style-side helpers (`cap_interval`, `phrase_arch`, `beat_strength`,
//! `rhythm_trim`, `shape_velocity`, `cadence_pitch`, `phrase_role`,
//! `terminal_dur_beats`, `phrase_start_offset`, `rubato_offset`,
//! `stress_syncopation`, `chord_tone_nearest`, `snap_to_pentatonic`,
//! `is_pentatonic`, `MAX_INTERVAL`) that all six profiles draw on.

mod anthemic;
mod chant;
mod conversational;
mod folk;
mod hymnal;
mod pop_ballad;

use crate::g2p::SyllableStress;
use crate::rng::XorShift;
use crate::scale::Scale;

use super::super::cadence::{
    final_degree_fits_chord, scale_degree_of, scale_degree_pc, tendency_resolution, CadenceGoal,
};
use super::super::motif_bass::chord_tones_in_register;
use super::super::{GeneratedNote, TimedChord};
use super::melody::{chord_at_beat, snap_to_scale};
use super::params::{VocalParams, VocalStyle};
use super::VocalContext;

use anthemic::AnthemicProfile;
use chant::ChantProfile;
use conversational::ConversationalProfile;
use folk::FolkProfile;
use hymnal::HymnalProfile;
use pop_ballad::PopBalladProfile;

/// Maximum interval the SVS model can cleanly render between adjacent
/// syllables. Bigger jumps surface as glitched audio, so every style
/// caps its per-syllable interval at this value.
pub(super) const MAX_INTERVAL: i16 = 9;

/// Cap a candidate pitch so it sits within `MAX_INTERVAL` semitones of
/// the previous pitch. When clamped, snap back into the scale so we
/// stay musical.
pub(super) fn cap_interval(prev: u8, candidate: u8, lo: u8, hi: u8, scale: Option<Scale>) -> u8 {
    let delta = candidate as i16 - prev as i16;
    if delta.abs() <= MAX_INTERVAL {
        return candidate;
    }
    let dir = delta.signum();
    let capped = (prev as i16 + dir * MAX_INTERVAL).clamp(lo as i16, hi as i16) as u8;
    snap_to_scale(capped, scale, lo, hi)
}

/// Pentatonic filter: true when `note` is a "safe" pentatonic degree of
/// `scale`. Drops the 4th and 7th in major-ish modes and the 2nd and 6th
/// in minor-ish modes. Used by the Folk style.
pub(super) fn is_pentatonic(note: u8, scale: Scale) -> bool {
    use crate::scale::Mode;
    let semitone = note % 12;
    let root = scale.root.to_semitone();
    let degree = (semitone + 12 - root) % 12;
    let drop: &[u8] = match scale.mode {
        Mode::Minor | Mode::Phrygian | Mode::Locrian | Mode::HarmonicMinor => &[2, 8], // omit 2nd, b6/6
        _ => &[5, 11], // omit 4, 7 (and b7 for mixolydian close enough)
    };
    !drop.contains(&degree)
}

/// Snap to the nearest pentatonic note within range. Falls back to a
/// plain scale snap, then to the input.
pub(super) fn snap_to_pentatonic(note: u8, scale: Option<Scale>, lo: u8, hi: u8) -> u8 {
    let Some(scale) = scale else { return note };
    for d in 0..=6i16 {
        for &sign in &[1i16, -1] {
            let candidate = note as i16 + d * sign;
            if (lo as i16..=hi as i16).contains(&candidate)
                && scale.contains(candidate as u8)
                && is_pentatonic(candidate as u8, scale)
            {
                return candidate as u8;
            }
        }
    }
    snap_to_scale(note, Some(scale), lo, hi)
}

/// Pick the chord tone in `range` closest to `target`. Returns `None`
/// when the chord has no tones in the requested range.
pub(super) fn chord_tone_nearest(
    chord: crate::chord::Chord,
    range: (u8, u8),
    target: u8,
) -> Option<u8> {
    let tones = chord_tones_in_register(chord, range);
    tones
        .into_iter()
        .min_by_key(|t| (*t as i16 - target as i16).abs())
}

/// Phrase-arch envelope: returns a 0..1 multiplier shaped like a real
/// vocal phrase — gentle build into a peak around 65 % of the line,
/// then a softer fall-off. Used by every style's velocity formula
/// to add line-shape dynamics instead of every syllable sitting at
/// the same level.
pub(super) fn phrase_arch(progress_in_line: f32) -> f32 {
    let p = progress_in_line.clamp(0.0, 1.0);
    let peak = 0.65;
    let v = if p <= peak {
        // Smooth attack: square-ease so opening syllables aren't
        // identical in level.
        (p / peak).powf(0.7)
    } else {
        // Gentler tail than attack so the line release feels natural.
        1.0 - 0.55 * ((p - peak) / (1.0 - peak)).powf(1.2)
    };
    v.clamp(0.0, 1.0)
}

/// Beat-of-bar accent strength in [0, 1]. Drives velocity accents
/// and some pitch decisions (chord-tone landing on strong beats).
///
/// Meter awareness: 4/4 puts the strongest accent on beat 1 with a
/// secondary on beat 3; 3/4 has a single strong on beat 1 and weak
/// 2 + 3; 6/8 (compound time) has primary on beat 1 and secondary
/// on beat 4 of the eighth-count, which translates to beat 0 + 1.5
/// in quarter-note time. We keep beats integers by approximating
/// 6/8 as a 6-beat cycle in eighth notes — callers that pass
/// beats_per_bar=6 get the compound feel.
pub(super) fn beat_strength(beat: u32, beats_per_bar: u32) -> f32 {
    let in_bar = beat % beats_per_bar.max(1);
    match beats_per_bar {
        // 6/8 compound: strong on 1 and 4 of the 6-eighth cycle.
        6 => match in_bar {
            0 => 1.0,
            3 => 0.70,
            _ => 0.30,
        },
        // 3/4 / waltz: strong only on 1.
        3 => match in_bar {
            0 => 1.0,
            _ => 0.30,
        },
        // 2/4 / cut time: 1 strong, 2 weak.
        2 => match in_bar {
            0 => 1.0,
            _ => 0.35,
        },
        // 4/4 default (and any other meter we treat as duple).
        _ => match in_bar {
            0 => 1.0,
            x if x == beats_per_bar / 2 => 0.65,
            _ => 0.30,
        },
    }
}

/// Per-syllable trim multiplier — controls what fraction of the
/// rigid `beat_step` slot each note actually fills. Variation comes
/// from three sources:
///   - Bar position: strong beats hold longer (long note feel),
///     weak beats are shorter (creates a gap after).
///   - Style "energy": pop ballad uses gentler variation, chant
///     uses sharper longs/shorts, conversational has irregular
///     bursts.
///   - Jitter: small per-syllable randomness so consecutive notes
///     aren't carbon copies.
///
/// `base_trim` is the style's default (e.g. 0.66 for PopBallad);
/// `range` is the half-width of the variation envelope. Returns a
/// trim in [0.30, 0.95].
pub(super) fn rhythm_trim(
    rng: &mut XorShift,
    base_trim: f32,
    beat: u32,
    beats_per_bar: u32,
    range: f32,
) -> f32 {
    let strength = beat_strength(beat, beats_per_bar); // 0..1
    // Strong beats lengthen toward base + range; weak beats shorten
    // toward base - range. Adds an audible swing without changing
    // syllable positions on the grid.
    let bias = (strength - 0.5) * 2.0 * range;
    let jitter = (rng.next_f32() - 0.5) * 0.08;
    (base_trim + bias + jitter).clamp(0.30, 0.95)
}

/// Duration of the final syllable of a line in beats. Replaces the
/// "fill the breath gap" math that used to hand the last note a
/// duration up to 4× longer than the rest of the line — that hang
/// reads as a mistake, not a held cadence note. Capped at 1.4× the
/// regular beat-step so the final note feels intentional without
/// dragging into the next phrase.
///
/// Note: an even briefer rest is still added by `enforce_no_overlap`
/// at the very end (one 64th-note gap), so the SVS pipeline always
/// sees a clean boundary into the next line.
pub(super) fn terminal_dur_beats(beat_step: f32, articulation: f32) -> f32 {
    let trim = 0.98 - 0.48 * articulation.clamp(0.0, 1.0);
    let normal = beat_step * trim;
    // Held but not absurd: 1.4x the regular note.
    let held = beat_step * 1.4;
    held.max(normal)
}

/// Section-level srdc role of one lyric line (Open Music Theory v2,
/// pop phrase archetypes): lines group in fours as
/// statement–restatement–departure–conclusion (aaba / aabc). The
/// restatement re-sings the statement's contour, the departure
/// contrasts and stays open, and only the conclusion closes. Trailing
/// groups shrink from the tail: 3 lines = s r c, 2 lines = s c, and a
/// lone trailing line concludes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::derive::vocal) enum SectionLineRole {
    Statement,
    Restatement,
    Departure,
    Conclusion,
}

pub(in crate::derive::vocal) fn line_role(line_idx: usize, total_lines: usize) -> SectionLineRole {
    let group_start = (line_idx / 4) * 4;
    let group_len = total_lines.saturating_sub(group_start).min(4);
    match (group_len, line_idx - group_start) {
        (4, 0) | (3, 0) | (2, 0) => SectionLineRole::Statement,
        (4, 1) | (3, 1) => SectionLineRole::Restatement,
        (4, 2) => SectionLineRole::Departure,
        _ => SectionLineRole::Conclusion,
    }
}

/// Phrase-role classification for one line of lyrics. Antecedent
/// lines end "open" — on a scale degree that asks for more (2, 4,
/// or 7). Consequent lines end "closed" — on the tonic (1), 3rd, or
/// 5th. Drives where we land the cadence pitch. Per the srdc layout,
/// statement / restatement / departure lines stay open and only the
/// group's conclusion closes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PhraseRole {
    Antecedent,
    Consequent,
}

pub(super) fn phrase_role(line_idx: usize, total_lines: usize) -> PhraseRole {
    match line_role(line_idx, total_lines) {
        SectionLineRole::Conclusion => PhraseRole::Consequent,
        _ => PhraseRole::Antecedent,
    }
}

/// Pick a "good" cadence pitch for the final syllable of a line,
/// using the cadence formula table (`derive::cadence`) instead of the
/// old fixed degree sets:
///   - Consequent → PAC finals (the tonic), falling back through IAC
///     (3rd / 5th) to HC when the active chord contains no compatible
///     degree (closed feel).
///   - Antecedent → HC finals (7th / 2nd), falling back through IAC
///     to PAC (open feel — asks the next line to resolve).
///
/// Tendency tones resolve: when `prev_pitch` (the penult) sits on
/// degree 7, 4, or 2, the final degree that resolves it (7→1, 4→3,
/// 2→1) is preferred over a nearer landing. The ~10% deceptive swap
/// happens in the post-line formula pass
/// (`melody::apply_line_cadence_formulas`), which also rewrites the
/// penult to complete the two-note formula.
///
/// Falls back to the legacy chord-tone landing (root preference) when
/// no scale is available or nothing is reachable; the picked pitch
/// stays within an octave of `prev_pitch` so the cadence doesn't leap.
pub(super) fn cadence_pitch(
    role: PhraseRole,
    chord: Option<&TimedChord>,
    scale: Option<Scale>,
    prev_pitch: u8,
    range: (u8, u8),
) -> Option<u8> {
    let (lo, hi) = range;
    let chord = chord?;
    if let Some(scale) = scale {
        let chain: &[CadenceGoal] = match role {
            PhraseRole::Consequent => CadenceGoal::Pac.chain(),
            PhraseRole::Antecedent => CadenceGoal::Hc.chain(),
        };
        // Tier 1: formula finals compatible with the active chord.
        // Tier 2: the primary goal's finals regardless of the chord —
        // keeps antecedents "open" over chords (e.g. IV) that contain
        // none of the formula degrees, mirroring the old behavior of
        // landing on 2/4/7 colors.
        if let Some(p) = formula_final(chain, true, &scale, chord.chord, prev_pitch, lo, hi)
            .or_else(|| formula_final(&chain[..1], false, &scale, chord.chord, prev_pitch, lo, hi))
        {
            return Some(p);
        }
    }
    match role {
        PhraseRole::Consequent => {
            // Legacy closed landing: nearest chord tone with a strong
            // root preference.
            let tones = chord_tones_in_register(chord.chord, (lo, hi));
            if tones.is_empty() {
                return None;
            }
            let root_pc = chord.chord.root.to_semitone();
            let candidate = tones
                .iter()
                .filter(|t| (**t as i16 - prev_pitch as i16).abs() <= 12)
                .min_by_key(|t| {
                    let pc = (*t % 12) as i32;
                    let root_dist = ((pc - root_pc as i32).abs()).min(12 - (pc - root_pc as i32).abs());
                    let pitch_dist = (**t as i16 - prev_pitch as i16).abs() as i32;
                    // Multiply root_dist by 1000 so it dominates.
                    root_dist * 1000 + pitch_dist
                });
            candidate.copied().or_else(|| tones.iter().min_by_key(|t| (**t as i16 - prev_pitch as i16).abs()).copied())
        }
        PhraseRole::Antecedent => None,
    }
}

/// Best in-range realization of a formula *final* degree from the
/// first goal in `goals` that yields one. Scans the goal's formulas in
/// table order, requires chord compatibility when `require_fit`, keeps
/// candidates within an octave of `prev_pitch`, and prefers (a) the
/// degree that resolves `prev_pitch`'s tendency tone, then (b) the
/// nearest landing.
fn formula_final(
    goals: &[CadenceGoal],
    require_fit: bool,
    scale: &Scale,
    chord: crate::chord::Chord,
    prev_pitch: u8,
    lo: u8,
    hi: u8,
) -> Option<u8> {
    let prev_resolution = scale_degree_of(scale, prev_pitch).and_then(tendency_resolution);
    for goal in goals {
        let mut best: Option<(i32, u8)> = None;
        for &(_, f_deg) in goal.formulas() {
            if require_fit && !final_degree_fits_chord(scale, f_deg, chord) {
                continue;
            }
            let pc = scale_degree_pc(scale, f_deg);
            for midi in lo..=hi {
                if midi % 12 != pc {
                    continue;
                }
                let dist = (midi as i16 - prev_pitch as i16).abs();
                if dist > 12 {
                    continue; // stay within an octave of prev
                }
                let score = if prev_resolution == Some(f_deg) { 0 } else { 1000 } + dist as i32;
                if best.is_none_or(|(b, _)| score < b) {
                    best = Some((score, midi));
                }
            }
        }
        if let Some((_, m)) = best {
            return Some(m);
        }
    }
    None
}

/// Pick a per-line phrase-start offset in beats, relative to the
/// rigid `syl_cursor * section_beats / total_syl` slot. Returns a
/// value that can be added to `line_start_beat_f` to break the
/// "every line starts on the downbeat" pattern.
///
/// Distribution (chosen to feel like written songs without sounding
/// random): 50 % downbeat (no offset), 25 % pickup (~half a bar
/// early — line starts late in the previous chord), 15 % off-beat
/// shift (+0.25 to +0.5 beats — syncopated start), 10 % anacrusis
/// (one whole beat early). Anchored by the seed so the same lyric
/// always lands on the same shape.
///
/// `line_idx` is included in the rng draw so each line picks
/// independently.
pub(super) fn phrase_start_offset(rng: &mut XorShift, beats_per_bar: u32) -> f32 {
    let bpb = beats_per_bar.max(1) as f32;
    let r = rng.next_f32();
    if r < 0.50 {
        0.0
    } else if r < 0.75 {
        // Pickup: ~half a bar early.
        -bpb * 0.5
    } else if r < 0.90 {
        // Off-beat / syncopated start: 0.25 or 0.5 beats in.
        if rng.next_f32() < 0.5 {
            0.25
        } else {
            0.5
        }
    } else {
        // Anacrusis: one whole beat early.
        -1.0
    }
}

/// Style-specific weights for the velocity-shape formula. Each profile
/// picks one fixed `VelocityShape` and the walker passes it to
/// [`shape_velocity`] alongside the per-syllable inputs.
pub(super) struct VelocityShape {
    /// Resting velocity around which the shape oscillates.
    pub(super) base: f32,
    /// Weight of the phrase-arch contribution (0 = flat,
    /// 1 = full ±0.18 envelope swing).
    pub(super) arch: f32,
    /// Weight of the beat-strength accent contribution.
    pub(super) accent: f32,
    /// Per-syllable random half-width (jitter amplitude).
    pub(super) jitter: f32,
}

/// Combined velocity formula: base + phrase-arch contribution +
/// beat-of-bar accent + per-syllable jitter, clamped to [0.4, 1.0].
///
/// `shape` carries the style-level weights; the remaining arguments are
/// the per-syllable inputs the walker computes once per step.
pub(super) fn shape_velocity(
    rng: &mut XorShift,
    shape: &VelocityShape,
    progress_in_line: f32,
    beat: u32,
    beats_per_bar: u32,
) -> f32 {
    let arch = phrase_arch(progress_in_line) - 0.5; // -0.5..+0.5
    let accent = beat_strength(beat, beats_per_bar) - 0.5; // -0.5..+0.5
    let noise = (rng.next_f32() - 0.5) * 2.0 * shape.jitter;
    (shape.base + shape.arch * 0.36 * arch + shape.accent * 0.20 * accent + noise).clamp(0.4, 1.0)
}

/// Subtle per-syllable timing wobble — micro-rubato, ±`max_beats`
/// around the rigid grid position. Returns a beats-offset (positive
/// = ahead, negative = lag). Real singers don't sit exactly on the
/// click; tiny variation kills the "sequenced" feel.
pub(super) fn rubato_offset(rng: &mut XorShift, max_beats: f32) -> f32 {
    (rng.next_f32() - 0.5) * 2.0 * max_beats
}

/// Division-level syncopation offset for a primary-stress syllable
/// (Open Music Theory, rhythm in pop music): with probability `chance`
/// the onset anticipates its grid slot by a *quantized* division —
/// half the slot (eighth level for quarter-note slots, 70 %) or a
/// quarter of it (sixteenth level, 30 %) — instead of the continuous
/// micro-rubato unstressed syllables get. Returns a beats-offset
/// (negative = earlier); `0.0` keeps the syllable on the grid.
pub(super) fn stress_syncopation(rng: &mut XorShift, chance: f32, slot: f32) -> f32 {
    if rng.next_f32() >= chance {
        return 0.0;
    }
    if rng.next_f32() < 0.7 {
        -slot * 0.5
    } else {
        -slot * 0.25
    }
}

// ===========================================================================
// Style-profile trait + shared walker
// ===========================================================================
//
// Every per-style vocal generator (`derive_pop_ballad`, `derive_folk`, …)
// used to be a free function with a near-identical outer skeleton:
// destructure `VocalContext`, loop lines, loop syllables, push notes.
// The musical decisions inside that skeleton — pitch picking, slot
// width, rubato, duration, velocity, cadence — were the only parts
// that actually varied between styles. The trait below carves out
// exactly those decision points so the loop itself only has to be
// written once, in `walk_with_profile`.
//
// Profile authors do **not** override the walker structure. They
// implement methods on a unit struct (or, for Folk, a struct that
// carries the cross-line echo memory) and the walker calls them in
// the same order every style used in the legacy code. Preserving
// that order matters: the rng draws inside `pick_pitch` /
// `dur_beats` / `velocity` must happen in the same sequence the
// pre-refactor function used or the deterministic output drifts.

/// Per-line shared state computed once by the walker, then passed
/// (immutably) into every profile method for the line's syllables.
///
/// `extras` is the profile's private blob — Folk stores its echo
/// source and long/short ratios there, Anthemic its climax index,
/// Chant its triplet-feel flag, etc. The walker treats it as opaque.
pub(super) struct LineState<E> {
    pub(super) line_idx: usize,
    pub(super) line_syl: u32,
    /// Prefix-sum of `line_syllables[0..line_idx]` — used by
    /// PopBallad to compute the contour curve's *global* (across-
    /// section) progress for the syllable, which differs from the
    /// per-line progress every other style uses.
    pub(super) syl_cursor: u32,
    /// `(line_end_beat_f - line_start_beat_f) * (1 - breath_frac)` —
    /// Folk and Chant size their per-syllable slots from this.
    pub(super) sing_span: f32,
    /// `sing_span / line_syl`. Profiles that don't reshape rhythm
    /// (PopBallad, Conversational, Hymnal, Anthemic) use this as
    /// every syllable's slot directly. Folk + Chant override `slot`
    /// to return per-syllable values that fluctuate around this.
    pub(super) beat_step: f32,
    /// Prev-pitch *at the start of this line*. Folk reads this so
    /// echo lines can build their phrase offsets relative to the
    /// first pitch of the line they're echoing.
    pub(super) line_first_pitch: u8,
    /// Effective register for this line. Hymnal narrows to a 9-st
    /// band on top of `ctx.lo`; Chant narrows around a centre. Others
    /// use `(ctx.lo, ctx.hi)`.
    pub(super) band_lo: u8,
    pub(super) band_hi: u8,
    pub(super) articulation: f32,
    pub(super) extras: E,
}

/// Per-syllable computed inputs the walker hands to the profile.
/// Built once per `s` and shared between `pick_pitch`, `dur_beats`,
/// and `velocity` so each method sees a consistent view of the
/// syllable's grid position + active chord.
pub(super) struct StepInputs<'a> {
    pub(super) s: u32,
    pub(super) is_final: bool,
    pub(super) progress_in_line: f32,
    pub(super) beat_round: u32,
    pub(super) chord: Option<&'a TimedChord>,
    pub(super) prev_pitch: u8,
    /// This syllable's slot width in beats (= `line.beat_step` for
    /// most styles; overridden by Folk's long-short ratio and Chant's
    /// triplet-feel).
    pub(super) slot: f32,
}

/// The contract each `VocalStyle` variant implements. The methods
/// expose every decision point that varied across the six legacy
/// per-style functions; everything else (line span math, beat-of-bar
/// chord lookup, `cap_interval`, the `enforce_no_overlap` pass) is
/// shared in `walk_with_profile`.
///
/// **rng-draw order:** the walker calls `slot`, then `rubato_max`,
/// then (if non-zero) draws one `rubato_offset`, then `pick_pitch`,
/// `finalize_pitch`, `dur_beats`, `velocity`. The stress-syncopation
/// overlay draws from a per-syllable *derived* stream, so it never
/// perturbs this sequence. Implementors must
/// preserve their original draw order *within* each method —
/// reordering even a single `rng.next_f32()` changes the
/// deterministic output for that style.
pub(super) trait VocalStyleProfile {
    /// Style-private per-line scratchpad. Set in [`begin_line`],
    /// read in the rest of the methods.
    type LineExtras: Default;

    /// Effective MIDI range for the style. Most styles use the
    /// user-set `(ctx.lo, ctx.hi)`; Hymnal caps to a 9-semitone band
    /// at the bottom, Chant narrows to a ~5-semitone band around the
    /// speaking pitch.
    fn band(&self, ctx: &VocalContext) -> (u8, u8) {
        (ctx.lo, ctx.hi)
    }

    /// Pitch the walker uses as the "previous pitch" for syllable 0
    /// of the first line. Each style anchors at a different register:
    /// PopBallad / Anthemic mid, Conversational a hair below mid,
    /// Folk near the top (descending phrases), Chant / Hymnal at
    /// the band centre.
    fn init_prev_pitch(&self, ctx: &VocalContext, band: (u8, u8)) -> u8;

    /// Breath-gap fraction of each line that's silent. Some styles
    /// clamp the user-set `params.breath` (Folk ≥0.20, Chant ≥0.18,
    /// Anthemic ×0.6); Hymnal returns 0 (no breath, strict timing).
    fn breath_frac(&self, params: &VocalParams) -> f32 {
        params.breath.clamp(0.0, 0.9)
    }

    /// Whether to apply the random per-line phrase-start offset
    /// (pickup / anacrusis / off-beat). Hymnal returns `false` —
    /// strict timing is core to the style.
    fn use_phrase_start_offset(&self) -> bool {
        true
    }

    /// Build the per-line scratchpad. Called once before the
    /// syllable loop, after `LineState`'s shared fields are filled
    /// in. Free to consume rng draws — Folk uses one for its
    /// long-short ratio jitter, Chant one for its triplet-feel
    /// coin flip.
    fn begin_line(
        &mut self,
        rng: &mut XorShift,
        ctx: &VocalContext,
        line: &LineState<Self::LineExtras>,
    ) -> Self::LineExtras;

    /// Width of syllable `s`'s grid slot in beats. Default = the
    /// line's uniform `beat_step`. Folk overrides this for its
    /// long-short pairs; Chant overrides it for triplet-feel.
    fn slot(&self, line: &LineState<Self::LineExtras>, s: u32) -> f32 {
        let _ = s;
        line.beat_step
    }

    /// Maximum rubato half-width in beats for syllable `s`. The
    /// walker draws a rubato offset in `[-max, +max]` only when this
    /// is non-zero **and** `s` is neither the first nor the last
    /// syllable of the line (line edges stay on the grid so phrases
    /// still resolve to the chord). Default = 0 (strict grid).
    fn rubato_max(
        &self,
        line: &LineState<Self::LineExtras>,
        s: u32,
        slot: f32,
    ) -> f32 {
        let _ = (line, s, slot);
        0.0
    }

    /// Probability that a primary-stress syllable replaces its timing
    /// jitter with a division-level syncopation (a quantized half- or
    /// quarter-slot anticipation, see [`stress_syncopation`]). Like
    /// rubato, never applied to a line's first or last syllable.
    /// Hymnal and Chant return 0: strict grid is core to Hymnal, and
    /// Chant's recitation has its own triplet-feel rhythm engine.
    fn stress_syncopation_chance(&self) -> f32 {
        0.55
    }

    /// Pick the syllable's pitch — handles cadence override on the
    /// final syllable, chord-tone anchoring on strong beats, and any
    /// style-specific snapping (e.g. `snap_to_pentatonic` for Folk).
    /// The walker then applies `cap_interval` against the line's
    /// effective band so the final pitch is at most `MAX_INTERVAL`
    /// semitones from the previous one.
    fn pick_pitch(
        &self,
        ctx: &VocalContext,
        line: &LineState<Self::LineExtras>,
        inp: &StepInputs<'_>,
        rng: &mut XorShift,
    ) -> u8;

    /// Syllable duration in beats. Profiles return whatever
    /// combination of `slot`, `beat_step`, articulation trim, and
    /// `rhythm_trim` rng jitter they used in the legacy code.
    fn dur_beats(
        &self,
        line: &LineState<Self::LineExtras>,
        inp: &StepInputs<'_>,
        rng: &mut XorShift,
        beats_per_bar: u32,
    ) -> f32;

    /// Minimum duration in ticks (post-multiplication by `tpb`).
    /// Chant overrides this to `tpb / 8` so its tight sixteenth-feel
    /// notes aren't forced up to a quarter-note floor.
    fn min_dur_ticks(&self, tpb: u64) -> u64 {
        (tpb / 4).max(1)
    }

    /// MIDI velocity for the syllable. Always called — profiles
    /// build it from [`shape_velocity`] plus optional bumps
    /// (Anthemic's climax punch, Chant's spit lift).
    fn velocity(
        &self,
        line: &LineState<Self::LineExtras>,
        inp: &StepInputs<'_>,
        rng: &mut XorShift,
        beats_per_bar: u32,
    ) -> f32;

    /// Called once per line after the syllable loop. Folk uses this
    /// to stash the line's pitch offsets in its echo memory so the
    /// next pair of lines can echo this contour. Default does
    /// nothing.
    fn end_line(&mut self, line_idx: usize, line_offsets: Vec<i16>) {
        let _ = (line_idx, line_offsets);
    }
}

/// The single per-syllable walker every `VocalStyleProfile`
/// implementation flows through. Replaces the six legacy
/// `derive_*` functions — each style is now just a profile.
fn walk_with_profile<P: VocalStyleProfile>(
    ctx: &VocalContext<'_>,
    mut profile: P,
) -> Vec<GeneratedNote> {
    let VocalContext {
        chords,
        params,
        tpb,
        section_beats,
        beats_per_bar,
        ref line_syllables,
        total_syl,
        seed,
        ..
    } = *ctx;

    let mut rng = XorShift::new(seed.max(1));
    let mut out = Vec::with_capacity(total_syl as usize);

    let band = profile.band(ctx);
    let mut prev_pitch = profile.init_prev_pitch(ctx, band);

    let breath_frac = profile.breath_frac(params);
    let articulation = params.articulation.clamp(0.0, 1.0);
    let min_dur = profile.min_dur_ticks(tpb);

    let mut syl_cursor: u32 = 0;
    for (line_idx, &line_syl) in line_syllables.iter().enumerate() {
        if line_syl == 0 {
            continue;
        }
        let raw_line_start =
            syl_cursor as f32 * section_beats as f32 / total_syl as f32;
        let line_offset = if profile.use_phrase_start_offset() {
            phrase_start_offset(&mut rng, beats_per_bar)
        } else {
            0.0
        };
        let line_start_beat_f = (raw_line_start + line_offset).max(0.0);
        let line_end_beat_f =
            (syl_cursor + line_syl) as f32 * section_beats as f32 / total_syl as f32;
        let line_beat_span = (line_end_beat_f - line_start_beat_f).max(0.001);
        let sing_span = line_beat_span * (1.0 - breath_frac);
        let beat_step = sing_span / line_syl as f32;
        let line_first_pitch = prev_pitch;
        let mut line = LineState {
            line_idx,
            line_syl,
            syl_cursor,
            sing_span,
            beat_step,
            line_first_pitch,
            band_lo: band.0,
            band_hi: band.1,
            articulation,
            extras: <P::LineExtras as Default>::default(),
        };
        line.extras = profile.begin_line(&mut rng, ctx, &line);

        let mut line_offsets: Vec<i16> = Vec::with_capacity(line_syl as usize);
        let mut beat_cursor = 0.0_f32;
        for s in 0..line_syl {
            let progress_in_line = s as f32 / line_syl.max(1) as f32;
            let slot = profile.slot(&line, s);
            let rubato_max = profile.rubato_max(&line, s, slot);
            // Stressed syllables trade the continuous micro-jitter for
            // division-level syncopation: a quantized anticipation at
            // the eighth/sixteenth division of the slot. Unstressed
            // syllables (and stressed ones that fail the coin) keep
            // the rubato wobble. The syncopation draws come from a
            // *derived* per-syllable stream — the shared `rng` keeps
            // its legacy draw sequence, so the pitch / duration /
            // velocity decisions are untouched by the timing overlay.
            let stress = ctx
                .line_stresses
                .get(line_idx)
                .and_then(|l| l.get(s as usize))
                .copied()
                .unwrap_or_default();
            let sync_chance = profile.stress_syncopation_chance();
            let rubato = if s == 0 || s + 1 == line_syl {
                0.0
            } else {
                let wobble = if rubato_max == 0.0 {
                    0.0
                } else {
                    rubato_offset(&mut rng, rubato_max)
                };
                if stress == SyllableStress::Primary && sync_chance > 0.0 {
                    let mut sync_rng = XorShift::new(
                        seed.max(1)
                            ^ (line_idx as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15)
                            ^ (u64::from(s) + 1).wrapping_mul(0xC2B2_AE3D_27D4_EB4F),
                    );
                    let sync = stress_syncopation(&mut sync_rng, sync_chance, slot);
                    if sync != 0.0 { sync } else { wobble }
                } else {
                    wobble
                }
            };
            let beat_f = line_start_beat_f + beat_cursor + rubato;
            // The musical decisions (chord lookup, beat-strength
            // accents, strong-beat anchoring) read the syllable's
            // *grid* beat: rubato and stress syncopation are
            // performance-timing displacements — an anticipated onset
            // still belongs to the beat it anticipates, so only the
            // rendered `start_tick` moves.
            let grid_beat_f = line_start_beat_f + beat_cursor;
            beat_cursor += slot;
            let beat_round =
                grid_beat_f.floor().clamp(0.0, (section_beats - 1) as f32) as u32;
            let chord = chord_at_beat(chords, beat_round);
            let is_final = s + 1 == line_syl;
            let inp = StepInputs {
                s,
                is_final,
                progress_in_line,
                beat_round,
                chord,
                prev_pitch,
                slot,
            };
            let raw = profile.pick_pitch(ctx, &line, &inp, &mut rng);
            let pitch = cap_interval(prev_pitch, raw, line.band_lo, line.band_hi, ctx.scale);
            let dur_beats = profile.dur_beats(&line, &inp, &mut rng, beats_per_bar);
            let velocity = profile.velocity(&line, &inp, &mut rng, beats_per_bar);
            let start_tick = (beat_f as f64 * tpb as f64) as u64;
            let dur_ticks = ((dur_beats as f64 * tpb as f64) as u64).max(min_dur);
            out.push(GeneratedNote {
                note: pitch,
                velocity,
                start_tick,
                duration_ticks: dur_ticks,
            });
            line_offsets.push(pitch as i16 - line_first_pitch as i16);
            prev_pitch = pitch;
        }
        profile.end_line(line_idx, line_offsets);
        syl_cursor += line_syl;
    }
    out
}

/// Dispatch table — picks the concrete profile for a `VocalStyle`
/// variant and runs the shared walker.
pub(super) fn derive_with_profile(ctx: &VocalContext<'_>) -> Vec<GeneratedNote> {
    match ctx.params.style {
        VocalStyle::PopBallad => walk_with_profile(ctx, PopBalladProfile),
        VocalStyle::Conversational => walk_with_profile(ctx, ConversationalProfile),
        VocalStyle::Hymnal => walk_with_profile(ctx, HymnalProfile),
        VocalStyle::Folk => walk_with_profile(ctx, FolkProfile::default()),
        VocalStyle::Anthemic => walk_with_profile(ctx, AnthemicProfile),
        VocalStyle::Chant => walk_with_profile(ctx, ChantProfile),
    }
}
