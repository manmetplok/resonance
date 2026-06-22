//! Pentatonic harmony generator.
//!
//! Unlike the Markov sampler (bar-to-bar plausibility) and the schema
//! bank (fixed canonical loops), this generator models the *non-functional*
//! harmony of pentatonic-rooted pop/folk/modal writing: chord **roots are
//! drawn from a five-note pentatonic scale** and chord **quality is free**
//! — decoupled from any single key's diatonic function. The result is the
//! open, parallel, drone-friendly sound of music that harmonizes a
//! pentatonic melody without committing to tertian functional progressions.
//!
//! Two pentatonic flavours are offered (see [`PentatonicFlavour`]):
//!
//! * **Major** — roots on scale degrees 1·2·3·5·6 of the major scale.
//! * **Minor** — roots on scale degrees 1·♭3·4·5·♭7 of the natural minor.
//!
//! Roots move by a **weighted random walk** around the pentatonic ring:
//! small steps (adjacent pentatonic tones) are favoured over leaps, with a
//! gentle pull back toward the tonic, so the line of roots stays smooth and
//! tonally anchored without ever resolving in the functional sense. Each
//! chord then takes either its flavour's *plain* triad (major / minor) or,
//! with probability `color`, a colour quality (sus2/sus4/add9 …) drawn
//! from the flavour's palette — these stack only pentatonic-adjacent tones
//! and keep the texture from collapsing into plain triads.
//!
//! Like every generator in this module, generation is a pure function of
//! `(spec, seed, ctx)`: identical inputs always produce identical output,
//! and locked positions from [`GenContext`] carry through untouched and
//! consume no RNG draws. A locked chord whose root coincides with a
//! pentatonic tone re-anchors the walk there; otherwise the walk's
//! position is left undisturbed.

use serde::{Deserialize, Serialize};

use crate::chord::ChordQuality;
use crate::rng::XorShift;
use crate::scale::Mode;

use super::degree::Degree;
use super::{GenContext, GenerateError, GeneratedChord, GeneratedMaterial};

// ---------------------------------------------------------------------------
// PentatonicFlavour
// ---------------------------------------------------------------------------

/// Which pentatonic scale supplies the chord roots.
///
/// The flavour fixes three things: the five root scale degrees, the home
/// [`Mode`] those degrees are expressed against (so a downstream renderer
/// projects them correctly), and the chord-quality palette (a *plain*
/// triad plus colour qualities).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PentatonicFlavour {
    /// Major pentatonic: roots on degrees 1·2·3·5·6 of the major scale.
    Major,
    /// Minor pentatonic: roots on degrees 1·♭3·4·5·♭7 of the natural
    /// minor scale (expressed as the natural minor degrees 1·3·4·5·7).
    Minor,
}

impl PentatonicFlavour {
    /// Both flavours, in display order.
    pub const ALL: [PentatonicFlavour; 2] = [PentatonicFlavour::Major, PentatonicFlavour::Minor];

    /// The five pentatonic root scale degrees (1-indexed), in ascending
    /// order around the ring. Expressed against [`PentatonicFlavour::mode`]
    /// so a renderer projecting through the matching scale lands on the
    /// pentatonic pitches.
    pub fn roots(self) -> &'static [u8; 5] {
        match self {
            // Major pentatonic = major scale minus the 4th and 7th.
            PentatonicFlavour::Major => &[1, 2, 3, 5, 6],
            // Minor pentatonic = natural minor minus the 2nd and ♭6th;
            // degrees 3 and 7 land on ♭3 and ♭7 under Mode::Minor.
            PentatonicFlavour::Minor => &[1, 3, 4, 5, 7],
        }
    }

    /// The mode the flavour's root degrees are expressed against.
    pub fn mode(self) -> Mode {
        match self {
            PentatonicFlavour::Major => Mode::Major,
            PentatonicFlavour::Minor => Mode::Minor,
        }
    }

    /// The default ("plain") triad quality for chords of this flavour.
    pub fn plain_quality(self) -> ChordQuality {
        match self {
            PentatonicFlavour::Major => ChordQuality::Maj,
            PentatonicFlavour::Minor => ChordQuality::Min,
        }
    }

    /// Colour qualities, chosen with probability `color` instead of the
    /// plain triad. Suspensions and added seconds/sevenths keep the chord
    /// tones close to the pentatonic set and reinforce the open,
    /// non-functional texture rather than implying a leading tone.
    pub fn color_qualities(self) -> &'static [ChordQuality] {
        match self {
            PentatonicFlavour::Major => {
                &[ChordQuality::Sus2, ChordQuality::Sus4, ChordQuality::Add9]
            }
            PentatonicFlavour::Minor => {
                &[ChordQuality::Sus2, ChordQuality::Sus4, ChordQuality::Min7]
            }
        }
    }

    /// Stable machine identifier (kebab-case).
    pub fn id(self) -> &'static str {
        match self {
            PentatonicFlavour::Major => "major-pentatonic",
            PentatonicFlavour::Minor => "minor-pentatonic",
        }
    }

    /// Human-readable label.
    pub fn name(self) -> &'static str {
        match self {
            PentatonicFlavour::Major => "Major Pentatonic",
            PentatonicFlavour::Minor => "Minor Pentatonic",
        }
    }
}

impl std::fmt::Display for PentatonicFlavour {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

// ---------------------------------------------------------------------------
// Generation
// ---------------------------------------------------------------------------

/// Realize a pentatonic-harmony progression.
///
/// This is the entry point called by `GeneratorSpec::Pentatonic`. The walk
/// starts on the tonic, then steps around the pentatonic ring with the
/// weighting in [`step_weights`]; each unlocked position takes the plain
/// triad, or — with probability `color` (clamped to `0..=1`) — a colour
/// quality from the flavour's palette. Locked positions from `ctx` are
/// preserved verbatim, consume no RNG draws, and re-anchor the walk when
/// their root is a pentatonic tone.
///
/// The function is pure: identical inputs always produce identical output.
pub fn generate(
    flavour: PentatonicFlavour,
    length: u8,
    color: f32,
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

    let roots = flavour.roots();
    let plain = flavour.plain_quality();
    let colors = flavour.color_qualities();
    let color_p = if color.is_finite() {
        color.clamp(0.0, 1.0)
    } else {
        0.0
    };

    let mut rng = XorShift::new(seed);
    let mut chords = Vec::with_capacity(len);
    // Index into `roots` of the walk's current position. Starts on the
    // tonic (position 0).
    let mut pos = 0usize;

    for i in 0..len {
        // Locked positions carry through unchanged and consume no
        // randomness, so re-rolling the seed never disturbs them. A locked
        // chord rooted on a pentatonic tone re-anchors the walk there.
        if let Some(Some(degree)) = ctx.locked.get(i) {
            chords.push(GeneratedChord {
                degree: *degree,
                locked: true,
            });
            if !degree.flat {
                if let Some(p) = roots.iter().position(|&r| r == degree.root) {
                    pos = p;
                }
            }
            continue;
        }

        // Position 0 opens on the tonic; later positions walk the ring.
        if i != 0 {
            pos = next_position(pos, &mut rng);
        }

        let quality = if color_p > 0.0 && rng.next_f32() < color_p {
            colors[rng.next_range(colors.len())]
        } else {
            plain
        };

        chords.push(GeneratedChord {
            degree: Degree {
                root: roots[pos],
                flat: false,
                quality,
                inversion: 0,
            },
            locked: false,
        });
    }

    Ok(GeneratedMaterial {
        chords,
        splits: vec![],
    })
}

// ---------------------------------------------------------------------------
// Weighted random walk around the pentatonic ring
// ---------------------------------------------------------------------------

/// Number of roots in a pentatonic scale.
const RING: usize = 5;

/// Sample the next ring position from `cur` using [`step_weights`].
fn next_position(cur: usize, rng: &mut XorShift) -> usize {
    let weights = step_weights(cur);
    let total: u32 = weights.iter().sum();
    // `total` is a fixed positive constant, but guard anyway.
    if total == 0 {
        return cur;
    }
    let mut pick = rng.next_range(total as usize) as u32;
    for (j, &w) in weights.iter().enumerate() {
        if pick < w {
            return j;
        }
        pick -= w;
    }
    RING - 1
}

/// Transition weights from ring position `cur` to each of the five
/// positions. Adjacent pentatonic steps (circular distance 1) are weighted
/// highest, distance-2 steps lower; staying put is allowed but rare, and
/// the tonic (position 0) gets a small extra pull so the harmony keeps
/// gravitating home without ever cadencing functionally.
fn step_weights(cur: usize) -> [u32; RING] {
    let mut w = [0u32; RING];
    for (j, slot) in w.iter_mut().enumerate() {
        let base = if j == cur {
            1 // stay: discouraged but not forbidden
        } else {
            match circular_distance(cur, j) {
                1 => 4,
                _ => 2, // distance 2 (the only other option on a 5-ring)
            }
        };
        // Tonic pull (does not apply to staying on the tonic).
        *slot = if j == 0 && j != cur { base + 1 } else { base };
    }
    w
}

/// Shortest distance between two positions around a ring of [`RING`].
fn circular_distance(a: usize, b: usize) -> usize {
    let d = a.abs_diff(b);
    d.min(RING - d)
}
