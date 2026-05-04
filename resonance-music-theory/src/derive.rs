//! Pure part generators: given a chord progression, produce MIDI notes
//! for a pad, bass line, or melody.
//!
//! The functions here do not depend on any DAW types. They take a
//! `TimedChord` list and return `GeneratedNote`s with ticks measured
//! from the start of the containing clip. The app crate is responsible
//! for converting between these and the engine's `MidiClip` / `MidiNote`.

use serde::{Deserialize, Serialize};

use crate::chord::Chord;
use crate::pitch::PitchClass;
use crate::rng::XorShift;
use crate::scale::Scale;
use crate::voicing::{close_voicing, nearest_midi_above, nearest_midi_to, voice_lead};

/// A chord positioned on the section's beat grid. Mirrors the app's
/// `ChordState` so callers don't have to take a dependency on the app
/// crate just to use these generators.
#[derive(Debug, Clone, Copy)]
pub struct TimedChord {
    pub chord: Chord,
    pub start_beat: u32,
    pub duration_beats: u32,
}

/// DAW-agnostic MIDI note. Matches `resonance_audio::types::MidiNote`
/// field-for-field; converted at the app boundary.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GeneratedNote {
    pub note: u8,
    pub velocity: f32,
    pub start_tick: u64,
    pub duration_ticks: u64,
}

// ---------- Pad ----------

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PadParams {
    /// Inclusive MIDI range the pad voicings must stay inside.
    pub register: (u8, u8),
    pub velocity: f32,
}

impl Default for PadParams {
    fn default() -> Self {
        Self {
            register: (52, 76), // E3..E5 — a safe "pad" register
            velocity: 0.7,
        }
    }
}

/// Sustained, voice-led chord voicings. The first chord is spelled as a
/// close voicing anchored to the register floor; subsequent chords are
/// voice-led from the previous voicing so common tones stay put and
/// moving voices move by the smallest interval.
pub fn derive_pad(
    chords: &[TimedChord],
    params: &PadParams,
    ticks_per_beat: u32,
) -> Vec<GeneratedNote> {
    if chords.is_empty() {
        return Vec::new();
    }
    let tpb = ticks_per_beat as u64;
    let mut out = Vec::new();

    // Seed voicing: close voicing at the register floor, then clamp any
    // voices above the register ceiling by dropping them an octave.
    let mut voicing: Vec<u8> = close_voicing(chords[0].chord, params.register.0)
        .into_iter()
        .map(|n| {
            let mut m = n;
            while m > params.register.1 && m >= 12 {
                m -= 12;
            }
            m
        })
        .collect();
    voicing.sort_unstable();

    for (i, tc) in chords.iter().enumerate() {
        if i > 0 {
            voicing = voice_lead(&voicing, &tc.chord.pitch_classes(), params.register);
        }
        let start_tick = tc.start_beat as u64 * tpb;
        let duration_ticks = tc.duration_beats as u64 * tpb;
        for &note in &voicing {
            out.push(GeneratedNote {
                note,
                velocity: params.velocity,
                start_tick,
                duration_ticks,
            });
        }
    }
    out
}

// ---------- Bass ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BassStyle {
    /// One note per chord, held for the chord's full duration.
    RootHold,
    /// Root on every beat of the chord.
    RootPulse,
    /// Root / fifth alternating on each beat.
    RootFifth,
    /// Root / octave alternating on each beat.
    Octave,
    /// Scale-stepping walking bass that approaches the next chord's root.
    /// Falls back to `RootPulse` when no scale is provided.
    Walking,
    /// Motif-based bass that consumes the section-shared motif. The exact
    /// rendering is controlled by `BassParams::motif_mode` and the per-phrase
    /// development by `BassParams::motif_phrase`.
    Motif,
}

impl BassStyle {
    pub const ALL: [BassStyle; 6] = [
        BassStyle::RootHold,
        BassStyle::RootPulse,
        BassStyle::RootFifth,
        BassStyle::Octave,
        BassStyle::Walking,
        BassStyle::Motif,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            BassStyle::RootHold => "Root hold",
            BassStyle::RootPulse => "Root pulse",
            BassStyle::RootFifth => "Root + fifth",
            BassStyle::Octave => "Octave",
            BassStyle::Walking => "Walking",
            BassStyle::Motif => "Motif",
        }
    }
}

impl std::fmt::Display for BassStyle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// How a `BassStyle::Motif` lane renders the section-shared motif.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BassMotifMode {
    /// Same intervals + rhythm, anchored to the chord's bass note in the bass register.
    SameIntervals,
    /// Same intervals + rhythm but each note's duration ratio doubled — sits under the melody.
    Augmented,
    /// Use only the motif's rhythm + accents; pitches collapse to the chord's bass note.
    RhythmOnly,
    /// Take only the motif's first note per chord, on the chord's bass note.
    FirstNoteOnly,
}

impl BassMotifMode {
    pub const ALL: [BassMotifMode; 4] = [
        BassMotifMode::SameIntervals,
        BassMotifMode::Augmented,
        BassMotifMode::RhythmOnly,
        BassMotifMode::FirstNoteOnly,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            BassMotifMode::SameIntervals => "Same intervals",
            BassMotifMode::Augmented => "Augmented",
            BassMotifMode::RhythmOnly => "Rhythm only",
            BassMotifMode::FirstNoteOnly => "First note only",
        }
    }
}

impl std::fmt::Display for BassMotifMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Default for BassMotifMode {
    fn default() -> Self {
        BassMotifMode::SameIntervals
    }
}

/// How a `BassStyle::Motif` lane chooses per-phrase transforms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BassMotifPhrase {
    /// `Identity` for every phrase — predictable foundation.
    Simple,
    /// Same `Transform` per phrase as the melody picks (shared seed → in lockstep).
    MirrorMelody,
    /// Restricted set: `Identity` or `Augment` only.
    Restricted,
}

impl BassMotifPhrase {
    pub const ALL: [BassMotifPhrase; 3] = [
        BassMotifPhrase::Simple,
        BassMotifPhrase::MirrorMelody,
        BassMotifPhrase::Restricted,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            BassMotifPhrase::Simple => "Simple",
            BassMotifPhrase::MirrorMelody => "Mirror melody",
            BassMotifPhrase::Restricted => "Restricted",
        }
    }
}

impl std::fmt::Display for BassMotifPhrase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Default for BassMotifPhrase {
    fn default() -> Self {
        BassMotifPhrase::Simple
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BassParams {
    pub style: BassStyle,
    /// MIDI floor for the bass root. Default E1 (28).
    pub base_note: u8,
    pub velocity: f32,
    /// How the motif is rendered when `style == Motif`.
    #[serde(default)]
    pub motif_mode: BassMotifMode,
    /// How per-phrase development is handled when `style == Motif`.
    #[serde(default)]
    pub motif_phrase: BassMotifPhrase,
}

impl Default for BassParams {
    fn default() -> Self {
        Self {
            style: BassStyle::RootPulse,
            base_note: 28, // E1
            velocity: 0.85,
            motif_mode: BassMotifMode::SameIntervals,
            motif_phrase: BassMotifPhrase::Simple,
        }
    }
}

/// Section-level motif knobs shared across all motif-style lanes.
///
/// Both the melody motif renderer and the bass motif renderer consume
/// these so that, when both lanes' styles are `Motif`, they share the
/// same underlying motif identity (intervals + rhythm + accents). Only
/// register / velocity / contour / articulation / phrase length differ
/// per lane.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MotifParams {
    pub seed: u64,
    /// 0.0 = simple, 1.0 = maximum development. Drives motif length,
    /// rhythm pattern, and per-phrase transform variety.
    pub complexity: f32,
    /// Motif length override (0 = auto from complexity, else clamped to 2..=6).
    pub motif_len: u8,
    /// Probability of a leap vs step when generating motif intervals.
    pub leap_chance: f32,
}

impl Default for MotifParams {
    fn default() -> Self {
        Self {
            seed: 0,
            complexity: 0.5,
            motif_len: 0,
            leap_chance: 0.21,
        }
    }
}

pub fn derive_bass(
    chords: &[TimedChord],
    scale: Option<Scale>,
    params: &BassParams,
    ticks_per_beat: u32,
) -> Vec<GeneratedNote> {
    if chords.is_empty() {
        return Vec::new();
    }
    let tpb = ticks_per_beat as u64;
    let mut out = Vec::new();

    for (i, tc) in chords.iter().enumerate() {
        let root_pc = tc.chord.bass.unwrap_or(tc.chord.root);
        let root_midi = nearest_midi_above(root_pc, params.base_note);
        let start_tick = tc.start_beat as u64 * tpb;
        let beats = tc.duration_beats.max(1);

        match params.style {
            BassStyle::RootHold => {
                out.push(GeneratedNote {
                    note: root_midi,
                    velocity: params.velocity,
                    start_tick,
                    duration_ticks: beats as u64 * tpb,
                });
            }
            BassStyle::RootPulse => {
                for b in 0..beats {
                    out.push(GeneratedNote {
                        note: root_midi,
                        velocity: params.velocity,
                        start_tick: start_tick + b as u64 * tpb,
                        duration_ticks: tpb,
                    });
                }
            }
            BassStyle::RootFifth => {
                let fifth_pc = root_pc.transpose(7);
                let fifth_midi = nearest_midi_above(fifth_pc, root_midi);
                for b in 0..beats {
                    let note = if b % 2 == 0 { root_midi } else { fifth_midi };
                    out.push(GeneratedNote {
                        note,
                        velocity: params.velocity,
                        start_tick: start_tick + b as u64 * tpb,
                        duration_ticks: tpb,
                    });
                }
            }
            BassStyle::Octave => {
                let up = root_midi.checked_add(12).filter(|&n| n <= 127);
                for b in 0..beats {
                    let note = if b % 2 == 0 || up.is_none() {
                        root_midi
                    } else {
                        up.unwrap()
                    };
                    out.push(GeneratedNote {
                        note,
                        velocity: params.velocity,
                        start_tick: start_tick + b as u64 * tpb,
                        duration_ticks: tpb,
                    });
                }
            }
            BassStyle::Walking => {
                let next_root_midi = match (chords.get(i + 1), scale) {
                    (Some(nc), _) => {
                        let next_pc = nc.chord.bass.unwrap_or(nc.chord.root);
                        nearest_midi_to(next_pc, root_midi)
                    }
                    (None, _) => root_midi,
                };
                let line = walking_line(scale, root_midi, next_root_midi, beats as usize);
                for (b, note) in line.into_iter().enumerate() {
                    out.push(GeneratedNote {
                        note,
                        velocity: params.velocity,
                        start_tick: start_tick + b as u64 * tpb,
                        duration_ticks: tpb,
                    });
                }
            }
            BassStyle::Motif => {
                // Motif requires section-level MotifParams; without them
                // (the legacy `derive_bass` entry point), fall back to
                // RootHold so the lane still produces audible notes.
                out.push(GeneratedNote {
                    note: root_midi,
                    velocity: params.velocity,
                    start_tick,
                    duration_ticks: beats as u64 * tpb,
                });
            }
        }
    }
    out
}

/// Stepwise line from `root` toward `next_root` through scale tones.
/// When no scale is available, falls back to repeating the root.
fn walking_line(scale: Option<Scale>, root: u8, next_root: u8, beats: usize) -> Vec<u8> {
    if beats == 0 {
        return Vec::new();
    }
    let Some(scale) = scale else {
        return vec![root; beats];
    };
    if beats == 1 {
        return vec![root];
    }

    // The last beat is an approach tone — one scale step away from the
    // next chord's root, on the side we're coming from.
    let approach_dir: i32 = if next_root >= root { -1 } else { 1 };
    let approach = step_scale(&scale, next_root, approach_dir);

    if beats == 2 {
        return vec![root, approach];
    }

    // Interior beats: step from root toward the approach tone. Direction
    // is chosen by whichever end of the span the approach tone sits on.
    let up = approach > root;
    let interior_count = beats - 2;
    let mut notes = Vec::with_capacity(beats);
    notes.push(root);
    let mut cur = root;
    for _ in 0..interior_count {
        cur = step_scale(&scale, cur, if up { 1 } else { -1 });
        notes.push(cur);
    }
    notes.push(approach);
    notes
}

/// Next MIDI note in `dir` direction whose pitch class belongs to
/// `scale`. Searches up to one octave; returns `from` if no scale tone
/// is found (shouldn't happen for well-formed scales).
fn step_scale(scale: &Scale, from: u8, dir: i32) -> u8 {
    let mut n = from as i32 + dir;
    for _ in 0..12 {
        if !(0..=127).contains(&n) {
            return from;
        }
        if scale.contains(n as u8) {
            return n as u8;
        }
        n += dir;
    }
    from
}

// ---------- Melody ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MelodyStyle {
    ArpUp,
    ArpDown,
    ArpUpDown,
    /// Motif-based melodic development with phrase structure,
    /// chord-tone targeting, rhythmic variation, and contour shaping.
    #[serde(alias = "ScaleWalk")]
    Motif,
}

impl MelodyStyle {
    pub const ALL: [MelodyStyle; 4] = [
        MelodyStyle::ArpUp,
        MelodyStyle::ArpDown,
        MelodyStyle::ArpUpDown,
        MelodyStyle::Motif,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            MelodyStyle::ArpUp => "Arp up",
            MelodyStyle::ArpDown => "Arp down",
            MelodyStyle::ArpUpDown => "Arp up/down",
            MelodyStyle::Motif => "Motif",
        }
    }
}

impl std::fmt::Display for MelodyStyle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Preferred melodic contour shape for motif-based generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContourPreference {
    /// RNG picks per-phrase, weighted by research distributions.
    Auto,
    /// Rise then fall (most common in folk/pop).
    Arch,
    /// Gradual descent.
    Descending,
    /// Gradual ascent.
    Ascending,
    /// Alternating peaks and valleys.
    Wave,
}

impl ContourPreference {
    pub const ALL: [ContourPreference; 5] = [
        ContourPreference::Auto,
        ContourPreference::Arch,
        ContourPreference::Descending,
        ContourPreference::Ascending,
        ContourPreference::Wave,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ContourPreference::Auto => "Auto",
            ContourPreference::Arch => "Arch",
            ContourPreference::Descending => "Descending",
            ContourPreference::Ascending => "Ascending",
            ContourPreference::Wave => "Wave",
        }
    }
}

impl Default for ContourPreference {
    fn default() -> Self {
        Self::Auto
    }
}

impl std::fmt::Display for ContourPreference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MelodyParams {
    pub style: MelodyStyle,
    pub register: (u8, u8),
    /// Length of one melody note in ticks. 240 = 8ths at TPQN=480,
    /// 120 = 16ths, 480 = quarter notes. Used by arp styles only.
    pub note_value_ticks: u32,
    /// Probability in [0, 1] that any given slot is silent.
    pub rest_density: f32,
    pub velocity: f32,
    /// 0.0 = very simple/repetitive, 1.0 = maximum development.
    /// Controls transformation variety, motif length, harmonic tension.
    /// Only used by the Motif style.
    #[serde(default = "default_complexity")]
    pub complexity: f32,
    /// 0.0 = very legato, 1.0 = very staccato. Controls the ratio of
    /// sounding duration to rhythmic slot. Only used by the Motif style.
    #[serde(default = "default_articulation")]
    pub articulation: f32,
    /// Preferred melodic contour shape. Only used by the Motif style.
    #[serde(default)]
    pub contour: ContourPreference,
    /// Phrase length in chords (2, 4, or 8). Only used by the Motif style.
    #[serde(default = "default_phrase_len")]
    pub phrase_len: u8,
    /// Motif length override (0 = auto from complexity). Only used by
    /// the Motif style.
    #[serde(default)]
    pub motif_len: u8,
    /// Probability of a leap vs step when generating motif intervals.
    /// Only used by the Motif style.
    #[serde(default = "default_leap_chance")]
    pub leap_chance: f32,
}

fn default_complexity() -> f32 {
    0.5
}
fn default_articulation() -> f32 {
    0.3
}
fn default_phrase_len() -> u8 {
    4
}
fn default_leap_chance() -> f32 {
    0.21
}

impl Default for MelodyParams {
    fn default() -> Self {
        Self {
            style: MelodyStyle::ArpUp,
            register: (67, 88), // G4..E6
            note_value_ticks: 240,
            rest_density: 0.0,
            velocity: 0.8,
            complexity: default_complexity(),
            articulation: default_articulation(),
            contour: ContourPreference::default(),
            phrase_len: default_phrase_len(),
            motif_len: 0,
            leap_chance: default_leap_chance(),
        }
    }
}

pub fn derive_melody(
    chords: &[TimedChord],
    scale: Option<Scale>,
    params: &MelodyParams,
    ticks_per_beat: u32,
    seed: u64,
) -> Vec<GeneratedNote> {
    if chords.is_empty() {
        return Vec::new();
    }

    if params.style == MelodyStyle::Motif {
        return derive_motif_melody(chords, scale, params, ticks_per_beat, seed);
    }

    let tpb = ticks_per_beat as u64;
    let slot_ticks = params.note_value_ticks.max(1) as u64;
    let mut out = Vec::new();
    let mut rng = XorShift::new(seed);

    for tc in chords {
        let chord_start = tc.start_beat as u64 * tpb;
        let chord_len = (tc.duration_beats as u64).max(1) * tpb;
        let tones = chord_tones_in_register(tc.chord, params.register);
        if tones.is_empty() {
            continue;
        }

        let slots = (chord_len / slot_ticks).max(1) as usize;
        for slot in 0..slots {
            let rest_roll = rng.next_f32();
            if params.rest_density > 0.0 && rest_roll < params.rest_density {
                continue;
            }

            let note = match params.style {
                MelodyStyle::ArpUp => tones[slot % tones.len()],
                MelodyStyle::ArpDown => tones[tones.len() - 1 - (slot % tones.len())],
                MelodyStyle::ArpUpDown => {
                    let n = tones.len();
                    if n < 2 {
                        tones[0]
                    } else {
                        let cycle = 2 * n - 2;
                        let idx = slot % cycle;
                        if idx < n {
                            tones[idx]
                        } else {
                            tones[cycle - idx]
                        }
                    }
                }
                MelodyStyle::Motif => unreachable!(),
            };

            out.push(GeneratedNote {
                note,
                velocity: params.velocity,
                start_tick: chord_start + slot as u64 * slot_ticks,
                duration_ticks: slot_ticks,
            });
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Motif-based melody engine
// ---------------------------------------------------------------------------

/// A single note in a motif, stored as a relative interval from an anchor
/// pitch so that transposition and inversion are simple arithmetic.
#[derive(Debug, Clone)]
struct MotifNote {
    /// Signed interval in semitones from the motif's anchor pitch.
    interval: i8,
    /// Duration as a multiple of a base rhythmic unit.
    duration_ratio: u8,
    /// Slight velocity emphasis on this note.
    accent: bool,
}

/// Transformation to apply to a motif when developing it across phrases.
#[derive(Debug, Clone, Copy)]
enum Transform {
    Identity,
    TransposeUp(i8),
    TransposeDown(i8),
    Invert,
    Retrograde,
    Augment,
    Diminish,
    Fragment(usize),
}

/// Internal contour shape for a phrase.
#[derive(Debug, Clone, Copy)]
enum Contour {
    Arch,
    Descending,
    Ascending,
    Wave,
}

/// Plan for a single melodic phrase.
struct PhrasePlan {
    chord_range: (usize, usize),
    contour: Contour,
    is_consequent: bool,
}

/// Rhythm pattern library: each pattern is a list of duration ratios.
/// The ratios are scaled to fill the available time. Higher indices are
/// more rhythmically complex.
const RHYTHM_PATTERNS: &[&[u8]] = &[
    &[1, 1, 1, 1],       // steady
    &[2, 1, 1],           // long-short-short
    &[1, 1, 2],           // short-short-long
    &[1, 2, 1],           // short-long-short
    &[3, 1, 2, 2],        // dotted feel
    &[1, 1, 1, 1, 2],     // four eighths + quarter
    &[2, 1, 1, 2, 2],     // varied
    &[1, 1, 2, 1, 1],     // syncopated center
];

/// Build a motif: a short melodic cell of 2-6 notes with relative intervals
/// and a rhythmic pattern. Intervals are unbounded by lane register — each
/// lane clamps to its own register at render time, so two lanes built from
/// the same `MotifParams` and chord get identical interval shapes.
fn build_motif(
    rng: &mut XorShift,
    chord: Chord,
    scale: Option<Scale>,
    motif: &MotifParams,
) -> Vec<MotifNote> {
    let len = if motif.motif_len > 0 {
        (motif.motif_len as usize).clamp(2, 6)
    } else {
        (2.0 + motif.complexity * 4.0).round() as usize
    };

    // Pick a rhythm pattern. Higher complexity biases toward later
    // (more complex) patterns.
    let max_pattern = (motif.complexity * (RHYTHM_PATTERNS.len() - 1) as f32).ceil() as usize;
    let pattern_idx = rng.next_range(max_pattern.max(1) + 1).min(RHYTHM_PATTERNS.len() - 1);
    let rhythm = RHYTHM_PATTERNS[pattern_idx];

    // Build interval contour.
    let chord_intervals = chord_tone_intervals(&chord);
    let has_scale = scale.is_some();
    let mut notes = Vec::with_capacity(len);
    let mut current_interval: i8 = 0;

    for i in 0..len {
        let duration_ratio = rhythm[i % rhythm.len()];
        let accent = i == 0 || duration_ratio >= 2;

        if i == 0 {
            notes.push(MotifNote {
                interval: 0,
                duration_ratio,
                accent,
            });
            continue;
        }

        // Choose: step, leap, or repeat.
        let roll = rng.next_f32();
        let repeat_chance = 0.11;
        let step_chance = 1.0 - motif.leap_chance - repeat_chance;

        let new_interval = if roll < repeat_chance {
            current_interval
        } else if roll < repeat_chance + step_chance {
            let step_size = if rng.next_f32() < 0.6 { 1 } else { 2 };
            let dir: i8 = if rng.next_f32() < 0.5 { 1 } else { -1 };
            let candidate = current_interval + dir * step_size;
            if has_scale {
                candidate
            } else {
                snap_to_chord_interval(candidate, &chord_intervals)
            }
        } else {
            let leap_size = 3 + (rng.next_f32() * 4.0) as i8;
            let dir: i8 = if rng.next_f32() < 0.5 { 1 } else { -1 };
            let candidate = current_interval + dir * leap_size;
            if has_scale {
                candidate
            } else {
                snap_to_chord_interval(candidate, &chord_intervals)
            }
        };

        current_interval = new_interval.clamp(-10, 10);

        notes.push(MotifNote {
            interval: current_interval,
            duration_ratio,
            accent,
        });
    }

    if let Some(last) = notes.last_mut() {
        last.interval = snap_to_chord_interval(last.interval, &chord_intervals);
    }

    notes
}

/// Pre-compute the per-phrase Transform sequence for a motif plan. Uses a
/// fresh RNG seeded only from `motif.seed` so two callers with the same
/// `MotifParams` always agree on the sequence — this is what makes
/// `BassMotifPhrase::MirrorMelody` lock to the melody.
fn plan_motif_transforms(
    num_phrases: usize,
    motif_len: usize,
    complexity: f32,
    seed: u64,
) -> Vec<Transform> {
    let mut rng = XorShift::new(seed.wrapping_add(0xA1B2C3D4E5F60718));
    (0..num_phrases)
        .map(|i| pick_transform(motif_len, i, complexity, &mut rng))
        .collect()
}


/// Get the semitone intervals of a chord's pitch classes relative to
/// the root (e.g. major = [0, 4, 7]).
fn chord_tone_intervals(chord: &Chord) -> Vec<i8> {
    let root = chord.root.to_semitone() as i8;
    chord
        .pitch_classes()
        .iter()
        .map(|pc| {
            let diff = pc.to_semitone() as i8 - root;
            if diff < 0 { diff + 12 } else { diff }
        })
        .collect()
}

/// Snap an interval to the nearest chord-tone interval (mod 12).
fn snap_to_chord_interval(interval: i8, chord_intervals: &[i8]) -> i8 {
    if chord_intervals.is_empty() {
        return interval;
    }
    let norm = ((interval % 12) + 12) % 12;
    let octave = interval - norm;
    let mut best = chord_intervals[0];
    let mut best_dist = 12i8;
    for &ci in chord_intervals {
        let dist = ((norm - ci).abs()).min((norm - ci + 12).abs()).min((norm - ci - 12).abs());
        if dist < best_dist {
            best_dist = dist;
            best = ci;
        }
    }
    octave + best
}

/// Apply a transformation to a motif, returning a new motif.
fn transform_motif(motif: &[MotifNote], transform: Transform) -> Vec<MotifNote> {
    match transform {
        Transform::Identity => motif.to_vec(),
        Transform::TransposeUp(n) => motif
            .iter()
            .map(|note| MotifNote {
                interval: note.interval + n,
                ..*note
            })
            .collect(),
        Transform::TransposeDown(n) => motif
            .iter()
            .map(|note| MotifNote {
                interval: note.interval - n,
                ..*note
            })
            .collect(),
        Transform::Invert => motif
            .iter()
            .map(|note| MotifNote {
                interval: -note.interval,
                ..*note
            })
            .collect(),
        Transform::Retrograde => {
            let mut reversed = motif.to_vec();
            reversed.reverse();
            reversed
        }
        Transform::Augment => motif
            .iter()
            .map(|note| MotifNote {
                duration_ratio: note.duration_ratio.saturating_mul(2).max(1),
                ..*note
            })
            .collect(),
        Transform::Diminish => motif
            .iter()
            .map(|note| MotifNote {
                duration_ratio: (note.duration_ratio / 2).max(1),
                ..*note
            })
            .collect(),
        Transform::Fragment(n) => motif[..n.min(motif.len())].to_vec(),
    }
}

/// Pick a contour for a phrase from the preference or RNG.
fn pick_contour(pref: ContourPreference, is_consequent: bool, rng: &mut XorShift) -> Contour {
    match pref {
        ContourPreference::Arch => Contour::Arch,
        ContourPreference::Descending => Contour::Descending,
        ContourPreference::Ascending => Contour::Ascending,
        ContourPreference::Wave => Contour::Wave,
        ContourPreference::Auto => {
            // Research-weighted: arch 29%, desc 27%, asc 22%, wave 22%.
            // Consequent phrases bias toward descending (resolution).
            let roll = rng.next_f32();
            if is_consequent {
                if roll < 0.40 {
                    Contour::Descending
                } else if roll < 0.75 {
                    Contour::Arch
                } else {
                    Contour::Ascending
                }
            } else if roll < 0.29 {
                Contour::Arch
            } else if roll < 0.56 {
                Contour::Descending
            } else if roll < 0.78 {
                Contour::Ascending
            } else {
                Contour::Wave
            }
        }
    }
}

/// Divide chords into phrases and assign contours.
fn plan_phrases(
    chords: &[TimedChord],
    contour_pref: ContourPreference,
    phrase_len: u8,
    rng: &mut XorShift,
) -> Vec<PhrasePlan> {
    let plen = (phrase_len as usize).max(1);
    let mut plans = Vec::new();
    let mut i = 0;
    let mut phrase_index = 0;

    while i < chords.len() {
        let end = (i + plen).min(chords.len());
        let is_consequent = phrase_index % 2 == 1;
        let contour = pick_contour(contour_pref, is_consequent, rng);
        plans.push(PhrasePlan {
            chord_range: (i, end),
            contour,
            is_consequent,
        });
        i = end;
        phrase_index += 1;
    }
    plans
}

/// Pick a transformation based on complexity and phrase position.
fn pick_transform(
    motif_len: usize,
    phrase_idx: usize,
    complexity: f32,
    rng: &mut XorShift,
) -> Transform {
    if phrase_idx == 0 {
        return Transform::Identity;
    }

    // Low complexity: mainly identity and transpose.
    // High complexity: full repertoire.
    let roll = rng.next_f32();
    let transpose_amount = 1 + rng.next_range(5) as i8;

    if complexity < 0.3 {
        // Simple: 40% identity, 30% transpose up, 30% transpose down
        if roll < 0.40 {
            Transform::Identity
        } else if roll < 0.70 {
            Transform::TransposeUp(transpose_amount)
        } else {
            Transform::TransposeDown(transpose_amount)
        }
    } else if complexity < 0.7 {
        // Moderate: add inversion and fragmentation
        if roll < 0.20 {
            Transform::Identity
        } else if roll < 0.40 {
            Transform::TransposeUp(transpose_amount)
        } else if roll < 0.60 {
            Transform::TransposeDown(transpose_amount)
        } else if roll < 0.75 {
            Transform::Invert
        } else {
            let frag_len = 2.max(motif_len / 2);
            Transform::Fragment(frag_len)
        }
    } else {
        // Complex: full repertoire
        if roll < 0.10 {
            Transform::Identity
        } else if roll < 0.25 {
            Transform::TransposeUp(transpose_amount)
        } else if roll < 0.40 {
            Transform::TransposeDown(transpose_amount)
        } else if roll < 0.55 {
            Transform::Invert
        } else if roll < 0.65 {
            Transform::Retrograde
        } else if roll < 0.75 {
            Transform::Augment
        } else if roll < 0.85 {
            Transform::Diminish
        } else {
            let frag_len = 2.max(motif_len / 2);
            Transform::Fragment(frag_len)
        }
    }
}

/// Compute a contour-based anchor offset in semitones for a given
/// position within a phrase.
fn contour_offset(contour: Contour, position: f32, register_span: u8) -> i8 {
    let half_span = (register_span / 4) as f32;
    let offset = match contour {
        Contour::Arch => {
            // Parabola peaking at position 0.5.
            let x = position - 0.5;
            half_span * (1.0 - 4.0 * x * x)
        }
        Contour::Descending => half_span * (1.0 - position),
        Contour::Ascending => half_span * position,
        Contour::Wave => {
            // One full sine cycle.
            (half_span * 0.7) * (position * std::f32::consts::TAU).sin()
        }
    };
    offset as i8
}

/// Align a MIDI note to the current harmony based on beat strength.
fn align_to_harmony(
    raw_midi: u8,
    beat_position: u64,
    tpb: u64,
    chord: Chord,
    scale: Option<Scale>,
    register: (u8, u8),
) -> u8 {
    let chord_tones = chord_tones_in_register(chord, register);
    if chord_tones.is_empty() {
        return raw_midi.clamp(register.0, register.1);
    }

    // Strong beat: position is a multiple of 2 beats.
    let is_strong = tpb > 0 && beat_position % (2 * tpb) == 0;

    if is_strong {
        // Must be a chord tone.
        if chord_tones.contains(&raw_midi) {
            return raw_midi;
        }
        return nearest_in_set(raw_midi, &chord_tones);
    }

    // Weak beat: allow scale tones.
    if let Some(scale) = scale {
        if scale.contains(raw_midi) {
            return raw_midi;
        }
        // Snap to nearest scale tone in register.
        let up = step_scale(&scale, raw_midi, 1);
        let down = step_scale(&scale, raw_midi, -1);
        let d_up = (up as i16 - raw_midi as i16).unsigned_abs() as u8;
        let d_down = (down as i16 - raw_midi as i16).unsigned_abs() as u8;
        let snapped = if d_up <= d_down { up } else { down };
        return snapped.clamp(register.0, register.1);
    }

    // No scale: snap to chord tone.
    nearest_in_set(raw_midi, &chord_tones)
}

/// Find the nearest value in a sorted set to the target.
fn nearest_in_set(target: u8, set: &[u8]) -> u8 {
    let mut best = set[0];
    let mut best_dist = (target as i16 - best as i16).unsigned_abs();
    for &v in &set[1..] {
        let dist = (target as i16 - v as i16).unsigned_abs();
        if dist < best_dist {
            best = v;
            best_dist = dist;
        }
    }
    best
}

/// Post-processing: resolve large leaps (>5 semitones) with stepwise
/// fill notes in the opposite direction.
fn apply_gap_fill(notes: &mut Vec<GeneratedNote>, scale: &Scale, register: (u8, u8)) {
    let mut i = 0;
    while i + 1 < notes.len() {
        let leap = notes[i + 1].note as i16 - notes[i].note as i16;
        if leap.unsigned_abs() > 5 {
            let fill_dir: i32 = if leap > 0 { -1 } else { 1 };
            // Check if next notes already resolve the leap.
            let already_filled = (i + 2 < notes.len()) && {
                let next_step = notes[i + 2].note as i16 - notes[i + 1].note as i16;
                (fill_dir > 0 && next_step > 0) || (fill_dir < 0 && next_step < 0)
            };
            if !already_filled {
                // Insert 1-2 fill notes by splitting the post-leap note's duration.
                let fill_count = if leap.unsigned_abs() > 7 { 2 } else { 1 };
                let post = &notes[i + 1];
                if post.duration_ticks > fill_count as u64 * 60 {
                    let fill_dur = post.duration_ticks / (fill_count as u64 + 1);
                    let mut fill_notes = Vec::new();
                    let mut cur = post.note;
                    let orig_start = post.start_tick;
                    for f in 0..fill_count {
                        cur = step_scale(scale, cur, fill_dir);
                        cur = cur.clamp(register.0, register.1);
                        fill_notes.push(GeneratedNote {
                            note: cur,
                            velocity: post.velocity * 0.9,
                            start_tick: orig_start + post.duration_ticks - (fill_count - f) as u64 * fill_dur,
                            duration_ticks: fill_dur,
                        });
                    }
                    // Shorten the post-leap note.
                    notes[i + 1].duration_ticks -= fill_count as u64 * fill_dur;
                    let insert_pos = i + 2;
                    for (j, note) in fill_notes.into_iter().enumerate() {
                        notes.insert(insert_pos + j, note);
                    }
                    i += 1 + fill_count; // skip past inserted notes
                    continue;
                }
            }
        }
        i += 1;
    }
}

/// Realize a single phrase from the motif and its transformation,
/// anchored to the chords and shaped by contour. The Transform is supplied
/// externally so that lanes which want to share transform plans (bass
/// `MirrorMelody` mode) can compute them up-front from a fresh RNG.
fn realize_phrase(
    motif: &[MotifNote],
    transform: Transform,
    phrase: &PhrasePlan,
    chords: &[TimedChord],
    scale: Option<Scale>,
    register: (u8, u8),
    articulation: f32,
    velocity_base: f32,
    tpb: u64,
) -> Vec<GeneratedNote> {
    let transformed = transform_motif(motif, transform);
    if transformed.is_empty() {
        return Vec::new();
    }

    let phrase_chords = &chords[phrase.chord_range.0..phrase.chord_range.1];
    let register_span = register.1.saturating_sub(register.0);
    let register_mid = (register.0 as u16 + register.1 as u16) / 2;

    let mut out = Vec::new();
    let sounding_ratio = 1.0 - articulation * 0.55;
    let min_duration = (tpb / 8).max(1);

    for (ci, tc) in phrase_chords.iter().enumerate() {
        let chord_start = tc.start_beat as u64 * tpb;
        let chord_ticks = tc.duration_beats as u64 * tpb;
        if chord_ticks == 0 {
            continue;
        }

        // Position within phrase for contour shaping (0.0 to 1.0).
        let phrase_position = if phrase_chords.len() > 1 {
            ci as f32 / (phrase_chords.len() - 1) as f32
        } else {
            0.5
        };
        let c_offset = contour_offset(phrase.contour, phrase_position, register_span);

        // Choose anchor: a chord tone near the contour target.
        let tones = chord_tones_in_register(tc.chord, register);
        if tones.is_empty() {
            continue;
        }
        let target = (register_mid as i16 + c_offset as i16).clamp(register.0 as i16, register.1 as i16) as u8;
        let anchor = nearest_in_set(target, &tones);

        // Scale the motif's duration ratios to fill this chord's time.
        let total_ratio: u64 = transformed.iter().map(|n| n.duration_ratio as u64).sum();
        if total_ratio == 0 {
            continue;
        }

        // Tile the motif to fill the chord duration. If the motif is
        // shorter than the chord, repeat it; if longer, truncate.
        let mut tick_cursor = chord_start;
        let chord_end = chord_start + chord_ticks;
        let mut motif_idx = 0;

        while tick_cursor < chord_end {
            let mn = &transformed[motif_idx % transformed.len()];
            let note_ticks = (chord_ticks * mn.duration_ratio as u64 / total_ratio).max(1);
            let remaining = chord_end - tick_cursor;
            let actual_ticks = note_ticks.min(remaining);

            if actual_ticks < min_duration {
                break;
            }

            let raw_midi = (anchor as i16 + mn.interval as i16).clamp(0, 127) as u8;
            let raw_clamped = raw_midi.clamp(register.0, register.1);

            let beat_pos = tick_cursor - chord_start;
            let aligned = align_to_harmony(raw_clamped, beat_pos, tpb, tc.chord, scale, register);

            let sounding = ((actual_ticks as f64 * sounding_ratio as f64) as u64).max(min_duration);
            let vel = if mn.accent {
                (velocity_base + 0.05).min(1.0)
            } else {
                velocity_base
            };

            out.push(GeneratedNote {
                note: aligned,
                velocity: vel,
                start_tick: tick_cursor,
                duration_ticks: sounding,
            });

            tick_cursor += actual_ticks;
            motif_idx += 1;
        }
    }

    // Consequent phrases resolve: snap the last note to the chord root.
    if phrase.is_consequent && !out.is_empty() {
        let last_chord = phrase_chords.last().unwrap();
        let root_tones = chord_tones_in_register(last_chord.chord, register);
        if let Some(root) = root_tones.first() {
            // Find the chord root (lowest chord tone = root in close position).
            let last = out.last_mut().unwrap();
            last.note = nearest_in_set(last.note, &[*root]);
        }
    }

    out
}

/// Top-level motif-based melody generator.
///
/// Back-compat shim: pulls motif knobs from `MelodyParams`. Direct callers
/// (and the inline tests) keep working unchanged. The app routes through
/// [`derive_motif_melody_with_section`] instead so the section's
/// `MotifParams` wins.
fn derive_motif_melody(
    chords: &[TimedChord],
    scale: Option<Scale>,
    params: &MelodyParams,
    ticks_per_beat: u32,
    seed: u64,
) -> Vec<GeneratedNote> {
    let motif = MotifParams {
        seed,
        complexity: params.complexity,
        motif_len: params.motif_len,
        leap_chance: params.leap_chance,
    };
    derive_motif_melody_with_section(chords, scale, params, &motif, seed, ticks_per_beat)
}

/// Section-aware motif-based melody generator.
///
/// `motif_params.seed` drives the shared motif (intervals + rhythm + accents)
/// and the per-phrase Transform sequence — both shared across all Motif
/// lanes in a section. `lane_seed` drives lane-local randomness only:
/// phrase contour selection (when `params.contour == Auto`) and rest-density
/// hole placement. Pressing Regenerate on a single lane should bump
/// `lane_seed` so the motif identity stays put while the lane gets a fresh
/// surface variation.
pub fn derive_motif_melody_with_section(
    chords: &[TimedChord],
    scale: Option<Scale>,
    params: &MelodyParams,
    motif_params: &MotifParams,
    lane_seed: u64,
    ticks_per_beat: u32,
) -> Vec<GeneratedNote> {
    if chords.is_empty() {
        return Vec::new();
    }
    let tpb = ticks_per_beat as u64;

    let mut motif_rng = XorShift::new(motif_params.seed);
    let motif = build_motif(&mut motif_rng, chords[0].chord, scale, motif_params);

    let mut lane_rng = XorShift::new(lane_seed);
    let phrases = plan_phrases(chords, params.contour, params.phrase_len, &mut lane_rng);
    let transforms = plan_motif_transforms(
        phrases.len(),
        motif.len(),
        motif_params.complexity,
        motif_params.seed,
    );

    // Per-phrase octave displacement keeps the motif identity intact
    // (same intervals + rhythm) while giving each Regenerate press an
    // audible shift. Without this, lane_seed only nudges contour and
    // rest-density randomization — invisible when the user pinned a
    // specific ContourPreference and rest_density sits at its default 0.
    let phrase_octave_offsets: Vec<i8> = (0..phrases.len())
        .map(|i| {
            if i == 0 {
                return 0;
            }
            let roll = lane_rng.next_f32();
            if roll < 0.55 {
                0
            } else if roll < 0.85 {
                12
            } else {
                -12
            }
        })
        .collect();

    let mut all_notes = Vec::new();
    let rest_gap = (tpb as f64 * (0.5 + params.rest_density as f64)) as u64;

    for (pi, phrase) in phrases.iter().enumerate() {
        let mut phrase_notes = realize_phrase(
            &motif,
            transforms[pi],
            phrase,
            chords,
            scale,
            params.register,
            params.articulation,
            params.velocity,
            tpb,
        );

        if let Some(scale) = scale {
            apply_gap_fill(&mut phrase_notes, &scale, params.register);
        }

        if pi > 0 && rest_gap > 0 {
            if let Some(last) = all_notes.last_mut() {
                let last_note: &mut GeneratedNote = last;
                if last_note.duration_ticks > rest_gap {
                    last_note.duration_ticks -= rest_gap;
                }
            }
        }

        let octave_shift = phrase_octave_offsets[pi];
        if octave_shift != 0 {
            for n in phrase_notes.iter_mut() {
                let candidate = (n.note as i16 + octave_shift as i16).clamp(0, 127) as u8;
                if candidate >= params.register.0 && candidate <= params.register.1 {
                    n.note = candidate;
                }
            }
        }

        all_notes.extend(phrase_notes);
    }

    if params.rest_density > 0.0 {
        let mut filtered = Vec::with_capacity(all_notes.len());
        for note in all_notes {
            if lane_rng.next_f32() >= params.rest_density {
                filtered.push(note);
            }
        }
        all_notes = filtered;
    }

    all_notes
}

// ---------------------------------------------------------------------------
// Bass motif renderer
// ---------------------------------------------------------------------------

/// Section-aware motif-based bass generator. Builds the shared motif from
/// `motif` + the first chord, then renders it across the chord progression
/// according to `bass.motif_mode` (what part of the motif to use) and
/// `bass.motif_phrase` (how to develop it across phrases).
///
/// `motif_params.seed` drives the shared motif and Transform plan (so a
/// melody Motif lane in the same section produces matching interval shapes
/// and, in `MirrorMelody` mode, matching transforms). `lane_seed` drives
/// only this lane's phrase-contour selection — pressing Regenerate on the
/// bass lane bumps `lane_seed` so the bass surface varies while the shared
/// motif stays put.
pub fn derive_bass_motif(
    chords: &[TimedChord],
    scale: Option<Scale>,
    bass: &BassParams,
    motif_params: &MotifParams,
    lane_seed: u64,
    ticks_per_beat: u32,
) -> Vec<GeneratedNote> {
    if chords.is_empty() {
        return Vec::new();
    }
    let tpb = ticks_per_beat as u64;

    let mut motif_rng = XorShift::new(motif_params.seed);
    let motif = build_motif(&mut motif_rng, chords[0].chord, scale, motif_params);
    if motif.is_empty() {
        return Vec::new();
    }

    let mut lane_rng = XorShift::new(lane_seed);
    let phrases = plan_phrases(chords, ContourPreference::Auto, 4, &mut lane_rng);

    // Restricted mode picks transforms from the lane RNG so pressing
    // Regenerate produces a fresh per-phrase plan without disturbing the
    // section's shared motif. Simple stays Identity, MirrorMelody stays
    // locked to the melody.
    let transforms: Vec<Transform> = match bass.motif_phrase {
        BassMotifPhrase::Simple => vec![Transform::Identity; phrases.len()],
        BassMotifPhrase::MirrorMelody => plan_motif_transforms(
            phrases.len(),
            motif.len(),
            motif_params.complexity,
            motif_params.seed,
        ),
        BassMotifPhrase::Restricted => {
            let mut restricted_rng = XorShift::new(lane_seed.wrapping_add(0xB1A2_5E55_C0FF_EE01));
            (0..phrases.len())
                .map(|i| {
                    if i == 0 || restricted_rng.next_f32() < 0.5 {
                        Transform::Identity
                    } else {
                        Transform::Augment
                    }
                })
                .collect()
        }
    };

    // Per-phrase octave displacement keeps the bass motivically identical
    // (same intervals and rhythm) while giving each Regenerate press an
    // audible shift — phrases occasionally jump up an octave or drop down,
    // staying inside the bass register at render time. This is the main
    // source of lane-local variation for Simple and MirrorMelody modes,
    // which otherwise have no per-lane randomness.
    let phrase_octave_offsets: Vec<i8> = (0..phrases.len())
        .map(|i| {
            // First phrase always at the anchor octave so the section
            // opens on the expected pitch.
            if i == 0 {
                return 0;
            }
            let roll = lane_rng.next_f32();
            if roll < 0.55 {
                0
            } else if roll < 0.85 {
                12
            } else {
                -12
            }
        })
        .collect();

    let mut out = Vec::new();
    for (pi, phrase) in phrases.iter().enumerate() {
        let transformed = transform_motif(&motif, transforms[pi]);
        if transformed.is_empty() {
            continue;
        }
        let octave_shift = phrase_octave_offsets[pi];
        let phrase_chords = &chords[phrase.chord_range.0..phrase.chord_range.1];

        for tc in phrase_chords {
            let chord_start = tc.start_beat as u64 * tpb;
            let chord_ticks = tc.duration_beats as u64 * tpb;
            if chord_ticks == 0 {
                continue;
            }
            let bass_pc = tc.chord.bass.unwrap_or(tc.chord.root);
            let base_anchor = nearest_midi_above(bass_pc, bass.base_note);
            let anchor = shifted_anchor(base_anchor, octave_shift, bass);

            render_bass_motif_chord(
                &transformed,
                tc,
                anchor,
                bass,
                scale,
                chord_start,
                chord_ticks,
                tpb,
                &mut out,
            );
        }
    }

    out
}

/// Apply an octave shift to a bass anchor while keeping it inside the
/// configured bass window (`base_note ..= base_note + 24`). If the shift
/// would take the anchor outside that window, fall back to the unshifted
/// anchor — the variation is musical, not a license to leave the register.
fn shifted_anchor(base_anchor: u8, octave_shift: i8, bass: &BassParams) -> u8 {
    if octave_shift == 0 {
        return base_anchor;
    }
    let lo = bass.base_note;
    let hi = bass.base_note.saturating_add(24);
    let candidate = (base_anchor as i16 + octave_shift as i16).clamp(0, 127) as u8;
    if candidate >= lo && candidate <= hi {
        candidate
    } else {
        base_anchor
    }
}

fn render_bass_motif_chord(
    motif: &[MotifNote],
    tc: &TimedChord,
    anchor: u8,
    bass: &BassParams,
    scale: Option<Scale>,
    chord_start: u64,
    chord_ticks: u64,
    tpb: u64,
    out: &mut Vec<GeneratedNote>,
) {
    let min_duration = (tpb / 8).max(1);
    let bass_register = (bass.base_note, bass.base_note.saturating_add(24));

    match bass.motif_mode {
        BassMotifMode::FirstNoteOnly => {
            let first = &motif[0];
            let vel = if first.accent {
                (bass.velocity + 0.05).min(1.0)
            } else {
                bass.velocity
            };
            out.push(GeneratedNote {
                note: anchor,
                velocity: vel,
                start_tick: chord_start,
                duration_ticks: chord_ticks,
            });
        }
        BassMotifMode::RhythmOnly => {
            let total_ratio: u64 = motif.iter().map(|n| n.duration_ratio as u64).sum();
            if total_ratio == 0 {
                return;
            }
            let mut tick_cursor = chord_start;
            let chord_end = chord_start + chord_ticks;
            let mut idx = 0;
            while tick_cursor < chord_end {
                let mn = &motif[idx % motif.len()];
                let note_ticks = (chord_ticks * mn.duration_ratio as u64 / total_ratio).max(1);
                let remaining = chord_end - tick_cursor;
                let actual = note_ticks.min(remaining);
                if actual < min_duration {
                    break;
                }
                let vel = if mn.accent {
                    (bass.velocity + 0.05).min(1.0)
                } else {
                    bass.velocity
                };
                out.push(GeneratedNote {
                    note: anchor,
                    velocity: vel,
                    start_tick: tick_cursor,
                    duration_ticks: actual,
                });
                tick_cursor += actual;
                idx += 1;
            }
        }
        BassMotifMode::SameIntervals | BassMotifMode::Augmented => {
            let augment = bass.motif_mode == BassMotifMode::Augmented;
            let total_ratio: u64 = motif
                .iter()
                .map(|n| n.duration_ratio as u64 * if augment { 2 } else { 1 })
                .sum();
            if total_ratio == 0 {
                return;
            }
            let mut tick_cursor = chord_start;
            let chord_end = chord_start + chord_ticks;
            let mut idx = 0;
            while tick_cursor < chord_end {
                let mn = &motif[idx % motif.len()];
                let dr = mn.duration_ratio as u64 * if augment { 2 } else { 1 };
                let note_ticks = (chord_ticks * dr / total_ratio).max(1);
                let remaining = chord_end - tick_cursor;
                let actual = note_ticks.min(remaining);
                if actual < min_duration {
                    break;
                }
                let raw = (anchor as i16 + mn.interval as i16).clamp(0, 127) as u8;
                let clamped = raw.clamp(bass_register.0, bass_register.1);
                let beat_pos = tick_cursor - chord_start;
                let aligned =
                    align_to_harmony(clamped, beat_pos, tpb, tc.chord, scale, bass_register);
                let vel = if mn.accent {
                    (bass.velocity + 0.05).min(1.0)
                } else {
                    bass.velocity
                };
                out.push(GeneratedNote {
                    note: aligned,
                    velocity: vel,
                    start_tick: tick_cursor,
                    duration_ticks: actual,
                });
                tick_cursor += actual;
                idx += 1;
            }
        }
    }
}

/// Every MIDI note inside `register` whose pitch class appears in
/// `chord`, sorted ascending and deduplicated.
fn chord_tones_in_register(chord: Chord, register: (u8, u8)) -> Vec<u8> {
    let pcs: Vec<PitchClass> = chord.pitch_classes();
    let (lo, hi) = register;
    let mut notes = Vec::new();
    for midi in lo..=hi {
        let pc = PitchClass::from_semitone(midi % 12);
        if pcs.contains(&pc) {
            notes.push(midi);
        }
    }
    notes.sort_unstable();
    notes.dedup();
    notes
}

// ---------------------------------------------------------------------------
// Drum motif rhythm
// ---------------------------------------------------------------------------

/// One onset in a motif-derived rhythm. Pitch is the caller's
/// responsibility — drum lanes substitute their own pad note. Accent
/// flags propagate from the motif so a downstream renderer can lift
/// accented hits' velocity without re-deriving the motif.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RhythmHit {
    pub start_tick: u64,
    pub duration_ticks: u64,
    pub accent: bool,
}

/// Project the section-shared motif onto the chord progression as a
/// pure rhythm: tile the motif's duration ratios across each chord
/// (same logic as the melody / bass renderers) and emit one
/// [`RhythmHit`] per onset.
///
/// The output rhythm is identical across chords in the progression
/// because we always use the `Identity` transform. That keeps the
/// generated drum part rhythmically locked to whatever the user is
/// playing on the melody / bass lanes — pressing a key that bumps the
/// motif seed re-derives every motif-driven lane the same way.
///
/// This function does no pitch work, so it doesn't need the section's
/// scale. Drum lanes route the hits through their pad note + velocity
/// at clip-build time.
pub fn derive_motif_rhythm(
    chords: &[TimedChord],
    motif_params: &MotifParams,
    ticks_per_beat: u32,
) -> Vec<RhythmHit> {
    if chords.is_empty() {
        return Vec::new();
    }
    let tpb = ticks_per_beat as u64;

    let mut motif_rng = XorShift::new(motif_params.seed);
    let motif = build_motif(&mut motif_rng, chords[0].chord, None, motif_params);
    if motif.is_empty() {
        return Vec::new();
    }

    let total_ratio: u64 = motif.iter().map(|n| n.duration_ratio as u64).sum();
    if total_ratio == 0 {
        return Vec::new();
    }
    // Drop notes shorter than a 32nd of a quarter — anything finer is
    // rhythmically unintelligible and tends to create accidental
    // double-hits when the math rounds.
    let min_duration = (tpb / 8).max(1);

    let mut out = Vec::new();
    for tc in chords {
        let chord_start = tc.start_beat as u64 * tpb;
        let chord_ticks = tc.duration_beats as u64 * tpb;
        if chord_ticks == 0 {
            continue;
        }

        let mut tick_cursor = chord_start;
        let chord_end = chord_start + chord_ticks;
        let mut motif_idx = 0;

        while tick_cursor < chord_end {
            let mn = &motif[motif_idx % motif.len()];
            let note_ticks = (chord_ticks * mn.duration_ratio as u64 / total_ratio).max(1);
            let remaining = chord_end - tick_cursor;
            let actual = note_ticks.min(remaining);
            if actual < min_duration {
                break;
            }
            out.push(RhythmHit {
                start_tick: tick_cursor,
                duration_ticks: actual,
                accent: mn.accent,
            });
            tick_cursor += actual;
            motif_idx += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chord::ChordQuality;
    use crate::pitch::PitchClass::*;
    use crate::scale::Mode;

    fn tc(chord: Chord, start_beat: u32, duration_beats: u32) -> TimedChord {
        TimedChord {
            chord,
            start_beat,
            duration_beats,
        }
    }

    // ---------- Pad ----------

    #[test]
    fn pad_empty_in_empty_out() {
        assert!(derive_pad(&[], &PadParams::default(), 480).is_empty());
    }

    #[test]
    fn pad_produces_one_note_per_voice_per_chord() {
        let chords = vec![
            tc(Chord::new(C, ChordQuality::Maj), 0, 4),
            tc(Chord::new(F, ChordQuality::Maj), 4, 4),
        ];
        let p = PadParams::default();
        let notes = derive_pad(&chords, &p, 480);
        assert_eq!(notes.len(), 6); // 3 voices × 2 chords
    }

    #[test]
    fn pad_voices_stay_in_register() {
        let chords = vec![
            tc(Chord::new(C, ChordQuality::Maj), 0, 4),
            tc(Chord::new(F, ChordQuality::Maj7), 4, 4),
            tc(Chord::new(G, ChordQuality::Dom7), 8, 4),
        ];
        let p = PadParams {
            register: (48, 72),
            velocity: 0.7,
        };
        for n in derive_pad(&chords, &p, 480) {
            assert!(n.note >= 48 && n.note <= 72, "{} out of register", n.note);
        }
    }

    #[test]
    fn pad_start_ticks_match_beats() {
        let chords = vec![
            tc(Chord::new(C, ChordQuality::Maj), 0, 4),
            tc(Chord::new(G, ChordQuality::Maj), 4, 4),
        ];
        let notes = derive_pad(&chords, &PadParams::default(), 480);
        // First chord at beat 0 → start_tick 0; second at beat 4 → 1920.
        let c_start: Vec<u64> = notes
            .iter()
            .filter(|n| n.start_tick == 0)
            .map(|n| n.start_tick)
            .collect();
        let g_start: Vec<u64> = notes
            .iter()
            .filter(|n| n.start_tick == 1920)
            .map(|n| n.start_tick)
            .collect();
        assert_eq!(c_start.len(), 3);
        assert_eq!(g_start.len(), 3);
    }

    // ---------- Bass ----------

    #[test]
    fn bass_empty_in_empty_out() {
        assert!(derive_bass(&[], None, &BassParams::default(), 480).is_empty());
    }

    #[test]
    fn bass_root_hold_one_note_per_chord() {
        let chords = vec![
            tc(Chord::new(C, ChordQuality::Maj), 0, 4),
            tc(Chord::new(G, ChordQuality::Maj), 4, 4),
            tc(Chord::new(A, ChordQuality::Min), 8, 4),
        ];
        let p = BassParams {
            style: BassStyle::RootHold,
            ..BassParams::default()
        };
        let notes = derive_bass(&chords, None, &p, 480);
        assert_eq!(notes.len(), 3);
        assert_eq!(notes[0].duration_ticks, 4 * 480);
    }

    #[test]
    fn bass_root_pulse_has_one_note_per_beat() {
        let chords = vec![tc(Chord::new(C, ChordQuality::Maj), 0, 4)];
        let p = BassParams {
            style: BassStyle::RootPulse,
            ..BassParams::default()
        };
        let notes = derive_bass(&chords, None, &p, 480);
        assert_eq!(notes.len(), 4);
        assert!(notes.iter().all(|n| n.note == notes[0].note));
    }

    #[test]
    fn bass_slash_chord_uses_bass_pitch_class() {
        // Am/G: root should be G, not A.
        let chord = Chord::new(A, ChordQuality::Min).with_bass(G);
        let chords = vec![tc(chord, 0, 4)];
        let p = BassParams {
            style: BassStyle::RootHold,
            ..BassParams::default()
        };
        let notes = derive_bass(&chords, None, &p, 480);
        assert_eq!(notes.len(), 1);
        // Expect G at or above base_note 28 (E1) — the nearest G ≥ 28 is G1 = 31.
        assert_eq!(notes[0].note % 12, G.to_semitone());
    }

    #[test]
    fn bass_walking_falls_back_without_scale() {
        let chords = vec![
            tc(Chord::new(C, ChordQuality::Maj), 0, 4),
            tc(Chord::new(G, ChordQuality::Maj), 4, 4),
        ];
        let p = BassParams {
            style: BassStyle::Walking,
            ..BassParams::default()
        };
        let notes = derive_bass(&chords, None, &p, 480);
        assert_eq!(notes.len(), 8);
    }

    #[test]
    fn bass_walking_uses_scale_tones() {
        let chords = vec![
            tc(Chord::new(C, ChordQuality::Maj), 0, 4),
            tc(Chord::new(G, ChordQuality::Maj), 4, 4),
        ];
        let scale = Scale::new(C, Mode::Major);
        let p = BassParams {
            style: BassStyle::Walking,
            ..BassParams::default()
        };
        let notes = derive_bass(&chords, Some(scale), &p, 480);
        // Every note must belong to the scale.
        for n in &notes {
            assert!(
                scale.contains(n.note),
                "walking bass note {} not in C major",
                n.note
            );
        }
        // Should produce one note per beat across both chords.
        assert_eq!(notes.len(), 8);
    }

    // ---------- Melody ----------

    #[test]
    fn melody_empty_in_empty_out() {
        assert!(derive_melody(&[], None, &MelodyParams::default(), 480, 0).is_empty());
    }

    #[test]
    fn melody_arp_up_stays_in_register() {
        let chords = vec![
            tc(Chord::new(C, ChordQuality::Maj), 0, 4),
            tc(Chord::new(F, ChordQuality::Maj), 4, 4),
        ];
        let p = MelodyParams::default();
        let notes = derive_melody(&chords, None, &p, 480, 1);
        assert!(!notes.is_empty());
        for n in &notes {
            assert!(n.note >= p.register.0 && n.note <= p.register.1);
        }
    }

    #[test]
    fn melody_arp_uses_chord_tones_only() {
        let chord = Chord::new(C, ChordQuality::Maj); // [C, E, G]
        let chords = vec![tc(chord, 0, 4)];
        let p = MelodyParams {
            style: MelodyStyle::ArpUp,
            ..MelodyParams::default()
        };
        let notes = derive_melody(&chords, None, &p, 480, 1);
        for n in &notes {
            let pc = n.note % 12;
            assert!(pc == 0 || pc == 4 || pc == 7, "non-chord note {}", n.note);
        }
    }

    #[test]
    fn melody_rest_density_one_produces_no_notes() {
        let chords = vec![tc(Chord::new(C, ChordQuality::Maj), 0, 4)];
        let p = MelodyParams {
            rest_density: 1.0,
            ..MelodyParams::default()
        };
        let notes = derive_melody(&chords, None, &p, 480, 3);
        assert!(notes.len() <= 1);
    }

    #[test]
    fn melody_seed_reproducible() {
        let chords = vec![
            tc(Chord::new(C, ChordQuality::Maj), 0, 4),
            tc(Chord::new(G, ChordQuality::Maj), 4, 4),
        ];
        let scale = Scale::new(C, Mode::Major);
        let p = MelodyParams {
            style: MelodyStyle::Motif,
            ..MelodyParams::default()
        };
        let a = derive_melody(&chords, Some(scale), &p, 480, 42);
        let b = derive_melody(&chords, Some(scale), &p, 480, 42);
        assert_eq!(a, b);
    }

    // ---------- Motif ----------

    fn motif_params() -> MelodyParams {
        MelodyParams {
            style: MelodyStyle::Motif,
            ..MelodyParams::default()
        }
    }

    fn standard_chords() -> Vec<TimedChord> {
        vec![
            tc(Chord::new(C, ChordQuality::Maj), 0, 4),
            tc(Chord::new(F, ChordQuality::Maj), 4, 4),
            tc(Chord::new(G, ChordQuality::Maj), 8, 4),
            tc(Chord::new(C, ChordQuality::Maj), 12, 4),
        ]
    }

    #[test]
    fn motif_empty_in_empty_out() {
        assert!(derive_melody(&[], None, &motif_params(), 480, 0).is_empty());
    }

    #[test]
    fn motif_stays_in_register() {
        let chords = standard_chords();
        let p = motif_params();
        let notes = derive_melody(&chords, Some(Scale::new(C, Mode::Major)), &p, 480, 42);
        assert!(!notes.is_empty());
        for n in &notes {
            assert!(
                n.note >= p.register.0 && n.note <= p.register.1,
                "note {} out of register ({}, {})",
                n.note,
                p.register.0,
                p.register.1
            );
        }
    }

    #[test]
    fn motif_strong_beats_are_chord_tones() {
        let chords = standard_chords();
        let scale = Scale::new(C, Mode::Major);
        let p = motif_params();
        let notes = derive_melody(&chords, Some(scale), &p, 480, 42);
        let tpb = 480u64;
        for n in &notes {
            let beat_in_chord = n.start_tick % (4 * tpb);
            let is_strong = beat_in_chord % (2 * tpb) == 0;
            if is_strong {
                // Find which chord this note belongs to.
                let chord_idx = chords
                    .iter()
                    .rposition(|tc| (tc.start_beat as u64 * tpb) <= n.start_tick)
                    .unwrap_or(0);
                let chord = chords[chord_idx].chord;
                let pcs = chord.pitch_classes();
                let note_pc = PitchClass::from_semitone(n.note % 12);
                assert!(
                    pcs.contains(&note_pc),
                    "strong-beat note {} (pc {:?}) not a chord tone of {:?}",
                    n.note,
                    note_pc,
                    chord
                );
            }
        }
    }

    #[test]
    fn motif_seed_deterministic() {
        let chords = standard_chords();
        let scale = Scale::new(C, Mode::Major);
        let p = motif_params();
        let a = derive_melody(&chords, Some(scale), &p, 480, 123);
        let b = derive_melody(&chords, Some(scale), &p, 480, 123);
        assert_eq!(a, b);
    }

    #[test]
    fn motif_respects_scale() {
        let chords = standard_chords();
        let scale = Scale::new(C, Mode::Major);
        let p = MelodyParams {
            style: MelodyStyle::Motif,
            complexity: 0.3, // keep it simple to avoid chromatic passing tones
            ..MelodyParams::default()
        };
        let notes = derive_melody(&chords, Some(scale), &p, 480, 7);
        for n in &notes {
            assert!(
                scale.contains(n.note),
                "motif note {} not in C major",
                n.note
            );
        }
    }

    #[test]
    fn motif_has_varied_durations() {
        let chords = standard_chords();
        let p = MelodyParams {
            style: MelodyStyle::Motif,
            complexity: 0.7,
            ..MelodyParams::default()
        };
        // Try several seeds — at least one should produce varied durations.
        let mut found_varied = false;
        for seed in 0..20u64 {
            let notes = derive_melody(&chords, Some(Scale::new(C, Mode::Major)), &p, 480, seed);
            let unique_durations: std::collections::HashSet<u64> =
                notes.iter().map(|n| n.duration_ticks).collect();
            if unique_durations.len() >= 2 {
                found_varied = true;
                break;
            }
        }
        assert!(found_varied, "motif should produce varied note durations");
    }

    #[test]
    fn motif_no_scale_falls_back_to_chord_tones() {
        let chords = standard_chords();
        let p = motif_params();
        let notes = derive_melody(&chords, None, &p, 480, 42);
        assert!(!notes.is_empty());
        for n in &notes {
            // Without a scale, every note should be a chord tone of
            // some chord in the progression.
            let chord_idx = chords
                .iter()
                .rposition(|tc| (tc.start_beat as u64 * 480) <= n.start_tick)
                .unwrap_or(0);
            let pcs = chords[chord_idx].chord.pitch_classes();
            let note_pc = PitchClass::from_semitone(n.note % 12);
            assert!(
                pcs.contains(&note_pc),
                "no-scale note {} (pc {:?}) not a chord tone",
                n.note,
                note_pc
            );
        }
    }

    #[test]
    fn motif_contour_arch_peaks_in_middle() {
        let chords = vec![
            tc(Chord::new(C, ChordQuality::Maj), 0, 4),
            tc(Chord::new(F, ChordQuality::Maj), 4, 4),
            tc(Chord::new(G, ChordQuality::Maj), 8, 4),
            tc(Chord::new(C, ChordQuality::Maj), 12, 4),
            tc(Chord::new(F, ChordQuality::Maj), 16, 4),
            tc(Chord::new(G, ChordQuality::Maj), 20, 4),
            tc(Chord::new(C, ChordQuality::Maj), 24, 4),
            tc(Chord::new(C, ChordQuality::Maj), 28, 4),
        ];
        let scale = Scale::new(C, Mode::Major);
        let p = MelodyParams {
            style: MelodyStyle::Motif,
            contour: ContourPreference::Arch,
            phrase_len: 8,
            ..MelodyParams::default()
        };
        // Over several seeds, the peak note should tend toward the middle.
        let mut peak_in_middle = 0;
        for seed in 0..20u64 {
            let notes = derive_melody(&chords, Some(scale), &p, 480, seed);
            if notes.is_empty() {
                continue;
            }
            let peak_idx = notes
                .iter()
                .enumerate()
                .max_by_key(|(_, n)| n.note)
                .map(|(i, _)| i)
                .unwrap();
            let ratio = peak_idx as f32 / notes.len() as f32;
            if (0.2..=0.8).contains(&ratio) {
                peak_in_middle += 1;
            }
        }
        // At least half the seeds should peak in the middle 60%.
        assert!(
            peak_in_middle >= 10,
            "arch contour should peak in middle, but only {peak_in_middle}/20 did"
        );
    }

    #[test]
    fn motif_serde_alias_scale_walk() {
        // Old project files with "ScaleWalk" should deserialize to Motif.
        let json = r#""ScaleWalk""#;
        let style: MelodyStyle = serde_json::from_str(json).unwrap();
        assert_eq!(style, MelodyStyle::Motif);
    }
}
