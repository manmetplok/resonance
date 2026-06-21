//! Pop schema bank generator.
//!
//! Realizes a fixed harmonic *schema* — a canonical chord loop that
//! underlies large swaths of the pop/rock/blues repertoire — instead of
//! sampling a Markov chain. Schemas give progressions a recognizable
//! identity ("this is an axis loop", "this is a 12-bar blues") that the
//! Markov sampler's bar-to-bar plausibility cannot provide.
//!
//! Variation comes from two deterministic, spec-controlled knobs:
//!
//! * **Rotation** — start the loop on a different chord of the cycle
//!   (the axis schema I–V–vi–IV is famous in all four rotations).
//! * **Function-preserving substitution** — with a per-position
//!   probability, swap a chord for another from the same mode's
//!   vocabulary that shares **at least two pitch classes** with it
//!   (I→vi, IV→ii, V→vii°, …), which preserves harmonic function while
//!   freshening the surface.
//!
//! Like the Markov generator, generation is a pure function of
//! `(spec, seed, ctx)`: identical inputs always produce identical
//! output, and locked positions from [`GenContext`] carry through
//! untouched (locked positions consume no RNG draws).

use serde::{Deserialize, Serialize};

use crate::chord::ChordQuality;
use crate::pitch::PitchClass;
use crate::rng::XorShift;
use crate::scale::{Mode, Scale};

use super::degree::Degree;
use super::{GenContext, GenerateError, GeneratedChord, GeneratedMaterial};

// ---------------------------------------------------------------------------
// Degree shorthands (Degree is a struct, so `use Degree::*` is not valid).
// ---------------------------------------------------------------------------

// Major-key diatonic
const I: Degree = Degree::I;
const II: Degree = Degree::II_MIN;
const III: Degree = Degree::III_MIN;
const IV: Degree = Degree::IV;
const V: Degree = Degree::V;
const VI: Degree = Degree::VI_MIN;
const VIID: Degree = Degree::VII_DIM;

// Borrowed / modal (major-key perspective)
const BVI: Degree = Degree::FLAT_VI;
const BVII: Degree = Degree::FLAT_VII;
const IVMN: Degree = Degree::IV_MIN;

/// II as a major triad (Lydian / secondary-dominant colour; the "II♯"
/// of the Lydian shuttle). Not on [`Degree`]'s const list because it is
/// schema-specific vocabulary.
const II_MAJ: Degree = Degree {
    root: 2,
    flat: false,
    quality: ChordQuality::Maj,
    inversion: 0,
};

// Minor-key natural degrees (see `Degree` docs: `flat: false` because a
// minor scale already places these roots correctly).
const IMIN: Degree = Degree::I_MIN;
const IIIM: Degree = Degree::III_MAJ;
const VIM: Degree = Degree::VI_MAJ;
const VIIM: Degree = Degree::VII_MAJ;

/// ii° — diminished supertonic, natural in minor keys. Used only as
/// substitution vocabulary for minor-mode schemas.
const IIDIM: Degree = Degree {
    root: 2,
    flat: false,
    quality: ChordQuality::Dim,
    inversion: 0,
};

/// v — minor dominant. Schema-specific vocabulary: the dominant root
/// recoloured minor, part of the pentatonic schema's "quality-free"
/// recolouring (see [`PENTATONIC_VOCAB`]).
const V_MIN: Degree = Degree {
    root: 5,
    flat: false,
    quality: ChordQuality::Min,
    inversion: 0,
};

// ---------------------------------------------------------------------------
// SchemaKind
// ---------------------------------------------------------------------------

/// A canonical pop/rock/blues chord schema.
///
/// Each kind owns a base degree loop (see [`SchemaKind::base_degrees`])
/// and a home mode (see [`SchemaKind::mode`]) that determines how its
/// degrees are projected for tone-overlap computations and which
/// substitution vocabulary applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SchemaKind {
    /// 12-bar blues: I I I I | IV IV I I | V IV I I.
    TwelveBarBlues,
    /// Doo-wop changes: I–vi–IV–V ("Stand by Me").
    DooWop,
    /// Axis progression: I–V–vi–IV. Famous in all four rotations —
    /// use the spec's `rotation` field.
    Axis,
    /// Hopscotch: IV–V–vi–I (off-tonic loop that never opens on I).
    Hopscotch,
    /// Lament: i–♭VII–♭VI–V (Andalusian / descending-tetrachord bass).
    /// Minor-mode: degrees use minor-key conventions (i, VII, VI, V).
    Lament,
    /// Plagal vamp: I–IV oscillation.
    PlagalVamp,
    /// Double-plagal cascade: ♭VII–IV–I ("Hey Jude" outro).
    DoublePlagal,
    /// Plagal sigh: IV–iv–I (major subdominant melting into minor).
    PlagalSigh,
    /// Mixolydian shuttle: I–♭VII.
    MixolydianShuttle,
    /// Dorian shuttle: i–IV (minor tonic with major subdominant).
    /// Minor-mode.
    DorianShuttle,
    /// Lydian shuttle: I–II (major supertonic, the "II♯" colour).
    LydianShuttle,
    /// Diatonic circle of fifths: I–IV–vii°–iii–vi–ii–V–I.
    CircleOfFifths,
    /// Puff opener: I–iii–IV–I ("Puff, the Magic Dragon").
    Puff,
    /// Pentatonic walk: I–ii–iii–V–vi. Every chord is rooted on a
    /// major-pentatonic scale degree (1 2 3 5 6 — no 4 or 7), so the
    /// progression has no leading-tone pull and a rootsy, modal-folk
    /// identity. Unlike the diatonic schemas the substitution vocabulary
    /// is **quality-free**: every position can be recoloured major↔minor
    /// on the same pentatonic root (I↔i, ii↔II, V↔v, …), and most can
    /// additionally swap for a function-sharing pentatonic neighbour
    /// (the supertonic, which shares only its root and fifth with the
    /// other pentatonic triads, is limited to its same-root flip).
    Pentatonic,
}

impl SchemaKind {
    /// All schema kinds, in display order.
    pub const ALL: [SchemaKind; 14] = [
        SchemaKind::TwelveBarBlues,
        SchemaKind::DooWop,
        SchemaKind::Axis,
        SchemaKind::Hopscotch,
        SchemaKind::Lament,
        SchemaKind::PlagalVamp,
        SchemaKind::DoublePlagal,
        SchemaKind::PlagalSigh,
        SchemaKind::MixolydianShuttle,
        SchemaKind::DorianShuttle,
        SchemaKind::LydianShuttle,
        SchemaKind::CircleOfFifths,
        SchemaKind::Puff,
        SchemaKind::Pentatonic,
    ];

    /// The canonical degree loop in root rotation.
    pub fn base_degrees(self) -> &'static [Degree] {
        match self {
            SchemaKind::TwelveBarBlues => &[I, I, I, I, IV, IV, I, I, V, IV, I, I],
            SchemaKind::DooWop => &[I, VI, IV, V],
            SchemaKind::Axis => &[I, V, VI, IV],
            SchemaKind::Hopscotch => &[IV, V, VI, I],
            SchemaKind::Lament => &[IMIN, VIIM, VIM, V],
            SchemaKind::PlagalVamp => &[I, IV],
            SchemaKind::DoublePlagal => &[BVII, IV, I],
            SchemaKind::PlagalSigh => &[IV, IVMN, I],
            SchemaKind::MixolydianShuttle => &[I, BVII],
            SchemaKind::DorianShuttle => &[IMIN, IV],
            SchemaKind::LydianShuttle => &[I, II_MAJ],
            SchemaKind::CircleOfFifths => &[I, IV, VIID, III, VI, II, V, I],
            SchemaKind::Puff => &[I, III, IV, I],
            // Walks the five major-pentatonic triad roots in order.
            SchemaKind::Pentatonic => &[I, II, III, V, VI],
        }
    }

    /// Natural output length: one pass through the loop.
    pub fn default_length(self) -> u8 {
        self.base_degrees().len() as u8
    }

    /// The mode the schema's degree conventions assume. Minor-mode
    /// schemas (lament, Dorian shuttle) use minor-key degree constants
    /// and the minor substitution vocabulary; everything else is
    /// expressed from the major-key perspective.
    pub fn mode(self) -> Mode {
        match self {
            SchemaKind::Lament | SchemaKind::DorianShuttle => Mode::Minor,
            _ => Mode::Major,
        }
    }

    /// Stable machine identifier (kebab-case).
    pub fn id(self) -> &'static str {
        match self {
            SchemaKind::TwelveBarBlues => "twelve-bar-blues",
            SchemaKind::DooWop => "doo-wop",
            SchemaKind::Axis => "axis",
            SchemaKind::Hopscotch => "hopscotch",
            SchemaKind::Lament => "lament",
            SchemaKind::PlagalVamp => "plagal-vamp",
            SchemaKind::DoublePlagal => "double-plagal",
            SchemaKind::PlagalSigh => "plagal-sigh",
            SchemaKind::MixolydianShuttle => "mixolydian-shuttle",
            SchemaKind::DorianShuttle => "dorian-shuttle",
            SchemaKind::LydianShuttle => "lydian-shuttle",
            SchemaKind::CircleOfFifths => "circle-of-fifths",
            SchemaKind::Puff => "puff",
            SchemaKind::Pentatonic => "pentatonic",
        }
    }

    /// Substitution vocabulary for this schema's function-preserving
    /// swaps. Most schemas draw from the shared major/minor vocabulary
    /// (see [`vocabulary`]); the pentatonic schema overrides it with a
    /// quality-free, pentatonic-rooted set (see [`PENTATONIC_VOCAB`]).
    fn substitution_vocab(self) -> &'static [Degree] {
        match self {
            SchemaKind::Pentatonic => PENTATONIC_VOCAB,
            _ => vocabulary(self.mode()),
        }
    }

    /// Human-readable label.
    pub fn name(self) -> &'static str {
        match self {
            SchemaKind::TwelveBarBlues => "12-Bar Blues",
            SchemaKind::DooWop => "Doo-Wop",
            SchemaKind::Axis => "Axis",
            SchemaKind::Hopscotch => "Hopscotch",
            SchemaKind::Lament => "Lament",
            SchemaKind::PlagalVamp => "Plagal Vamp",
            SchemaKind::DoublePlagal => "Double Plagal",
            SchemaKind::PlagalSigh => "Plagal Sigh",
            SchemaKind::MixolydianShuttle => "Mixolydian Shuttle",
            SchemaKind::DorianShuttle => "Dorian Shuttle",
            SchemaKind::LydianShuttle => "Lydian Shuttle",
            SchemaKind::CircleOfFifths => "Circle of Fifths",
            SchemaKind::Puff => "Puff",
            SchemaKind::Pentatonic => "Pentatonic",
        }
    }
}

impl std::fmt::Display for SchemaKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

// ---------------------------------------------------------------------------
// Generation
// ---------------------------------------------------------------------------

/// Realize a schema into a progression.
///
/// This is the entry point called by `GeneratorSpec::Schema`. The base
/// loop is rotated by `rotation` positions, tiled cyclically to
/// `length` chords, then each unlocked position is independently
/// substituted with probability `substitution` (clamped to `0..=1`) by
/// a vocabulary chord sharing at least [`MIN_SHARED_TONES`] pitch
/// classes with the canonical chord. Locked positions from `ctx` are
/// preserved verbatim and consume no RNG draws.
///
/// The function is pure: identical inputs always produce identical
/// output.
pub fn generate(
    kind: SchemaKind,
    length: u8,
    rotation: u8,
    substitution: f32,
    seed: u64,
    ctx: &GenContext,
) -> Result<GeneratedMaterial, GenerateError> {
    let len = length as usize;
    if len == 0 {
        return Ok(GeneratedMaterial {
            chords: vec![],
            splits: vec![],
        });
    }

    let base = kind.base_degrees();
    let rot = rotation as usize % base.len();
    let sub_p = if substitution.is_finite() {
        substitution.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let mode = kind.mode();
    let vocab = kind.substitution_vocab();

    let mut rng = XorShift::new(seed);
    let mut chords = Vec::with_capacity(len);

    for i in 0..len {
        // Locked positions carry through unchanged and consume no
        // randomness, so re-rolling the seed never disturbs them.
        if let Some(Some(degree)) = ctx.locked.get(i) {
            chords.push(GeneratedChord {
                degree: *degree,
                locked: true,
            });
            continue;
        }

        let canonical = base[(i + rot) % base.len()];
        let mut degree = canonical;

        if sub_p > 0.0 && rng.next_f32() < sub_p {
            let candidates = substitutes(canonical, vocab, mode);
            if !candidates.is_empty() {
                degree = candidates[rng.next_range(candidates.len())];
            }
        }

        chords.push(GeneratedChord {
            degree,
            locked: false,
        });
    }

    Ok(GeneratedMaterial {
        chords,
        splits: vec![],
    })
}

// ---------------------------------------------------------------------------
// Function-preserving substitution
// ---------------------------------------------------------------------------

/// Minimum number of shared pitch classes for a substitution to count
/// as function-preserving (e.g. I↔vi share two tones, I↔vi↔iii chain
/// the tonic prolongation family; IV↔ii the predominant family).
pub const MIN_SHARED_TONES: u32 = 2;

/// Substitution vocabulary for a mode. Degrees are expressed with the
/// same conventions the schemas use (major-key constants for major,
/// minor-key naturals for minor).
fn vocabulary(mode: Mode) -> &'static [Degree] {
    match mode {
        Mode::Minor => &[IMIN, IIDIM, IIIM, Degree::IV_MIN, IV, V, VIM, VIIM],
        _ => &[I, II, III, IV, V, VI, VIID, BVI, BVII, IVMN, II_MAJ],
    }
}

/// Substitution vocabulary for [`SchemaKind::Pentatonic`]: every triad
/// rooted on a major-pentatonic scale degree (1 2 3 5 6), in both major
/// and minor qualities. Because a major and minor triad on the same root
/// share two pitch classes (root + fifth), the [`MIN_SHARED_TONES`]
/// filter admits same-root quality flips, giving the schema its
/// "quality-free" recolouring while still excluding the avoid-tone roots
/// 4 and 7.
const PENTATONIC_VOCAB: &[Degree] = &[I, IMIN, II, II_MAJ, III, IIIM, V, V_MIN, VI, VIM];

/// All vocabulary degrees that share at least [`MIN_SHARED_TONES`]
/// pitch classes with `original` (projected through `mode`), excluding
/// `original` itself. Order follows the vocabulary table, so sampling
/// stays deterministic.
fn substitutes(original: Degree, vocab: &'static [Degree], mode: Mode) -> Vec<Degree> {
    let original_pcs = pitch_class_mask(original, mode);
    vocab
        .iter()
        .copied()
        .filter(|&candidate| {
            candidate != original
                && shared_tone_count(original_pcs, pitch_class_mask(candidate, mode))
                    >= MIN_SHARED_TONES
        })
        .collect()
}

/// Bitmask (bit n = pitch class n) of a degree's chord tones, projected
/// through a reference scale on C in the given mode. Key choice is
/// irrelevant for overlap counting — transposition preserves shared
/// tone counts.
fn pitch_class_mask(degree: Degree, mode: Mode) -> u16 {
    let scale = Scale::new(PitchClass::C, mode);
    let chord = degree.to_chord(scale);
    let mut mask = 0u16;
    for pc in chord.pitch_classes() {
        mask |= 1 << pc.to_semitone();
    }
    mask
}

/// Number of pitch classes two chord-tone masks have in common.
fn shared_tone_count(a: u16, b: u16) -> u32 {
    (a & b).count_ones()
}
