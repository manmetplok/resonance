//! Markov transition tables and the registry that holds them.
//!
//! A [`MarkovTable`] maps conditioning histories (sequences of [`Degree`]s)
//! to weighted successor lists. The [`TableRegistry`] holds named tables
//! that generators look up at generation time.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::{Deserialize, Serialize};

use super::degree::Degree;

/// Harmonic function of a degree within a key: the tonic / predominant /
/// dominant roles of functional harmony (Open Music Theory naming; the
/// legacy [`crate::progression::Function`] calls the middle bucket
/// "Subdominant" and only classifies natural degree numbers).
///
/// The variant order follows the phrase arc T → PD → D, and the derived
/// `Ord` is what the phrase-model overlay in [`super::markov`] uses to
/// enforce arc monotonicity.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub enum HarmonicFunction {
    /// Stability / home: I and its substitutes (iii, vi, bIII).
    Tonic,
    /// Motion away from home, preparing the dominant: ii, IV, bVI, bII.
    Predominant,
    /// Tension demanding resolution: V, vii°, and the bVII / subtonic
    /// dominant substitutes of modal and minor-key practice.
    Dominant,
}

/// Default T/PD/D classification by scale-degree root, used for degrees
/// a table does not tag explicitly (e.g. user-registered tables).
///
/// Natural degrees: 1/3/6 = T, 2/4 = PD, 5/7 = D. Flat (borrowed)
/// degrees get the standard pop/modal readings: bVII is a dominant
/// substitute (mixolydian cadence), bIII a tonic substitute, and the
/// rest (bVI, bII Neapolitan, ...) predominants.
pub fn default_function(degree: Degree) -> HarmonicFunction {
    use HarmonicFunction::*;
    if degree.flat {
        match (degree.root.saturating_sub(1) % 7) + 1 {
            7 => Dominant,
            3 => Tonic,
            _ => Predominant,
        }
    } else {
        match (degree.root.saturating_sub(1) % 7) + 1 {
            1 | 3 | 6 => Tonic,
            2 | 4 => Predominant,
            _ => Dominant,
        }
    }
}

/// A Markov transition table over scale degrees.
///
/// Keys are conditioning histories whose length equals [`order`](Self::order);
/// values are weighted successors. Weights do not need to sum to 1.0 --
/// sampling normalizes on the fly.
///
/// `transitions` is a `BTreeMap` rather than a `HashMap` so iteration
/// order is deterministic across runs — the seeded-RNG paths in
/// `markov.rs` walk this map when no exact-length history match is
/// found and would otherwise produce different progressions across
/// builds.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarkovTable {
    /// Human-readable identifier used to look up the table in a
    /// [`TableRegistry`].
    pub id: String,
    /// Length of the conditioning history (key length). An order-1 table
    /// conditions on the single previous chord; order-2 conditions on
    /// the last two chords.
    pub order: u8,
    /// Transition map. Key = conditioning history of length `order`,
    /// value = weighted successor degrees.
    pub transitions: BTreeMap<Vec<Degree>, Vec<(Degree, f32)>>,
    /// Explicit T/PD/D tags for this table's degrees. Degrees absent
    /// from the map fall back to [`default_function`], so tables
    /// persisted before this field existed (and terse user-registered
    /// tables) keep working. Minor-mode tables (e.g. the metal builtin)
    /// need their own tags because the natural-minor VI/VII degrees
    /// function as PD/D there, not as the tonic-substitute/leading-tone
    /// the major-key heuristic assumes.
    #[serde(default)]
    pub functions: BTreeMap<Degree, HarmonicFunction>,
}

impl MarkovTable {
    /// All unique degrees that appear in this table (as history keys or
    /// as successor values), sorted for deterministic display order.
    pub fn degrees(&self) -> Vec<Degree> {
        let mut set = BTreeSet::new();
        for (key, transitions) in &self.transitions {
            for d in key {
                set.insert(*d);
            }
            for &(d, _) in transitions {
                set.insert(d);
            }
        }
        set.into_iter().collect()
    }

    /// T/PD/D function of a degree per this table's tagging, falling
    /// back to [`default_function`] for untagged degrees.
    ///
    /// Inverted degrees inherit the tag of their root position, with
    /// one classical exception: the cadential 6/4 (tonic triad over the
    /// dominant bass) *functions* as a dominant — it embellishes V and
    /// must resolve to it — so the phrase-model arc classifies it as D
    /// despite its tonic spelling.
    pub fn function_of(&self, degree: Degree) -> HarmonicFunction {
        if let Some(f) = self.functions.get(&degree) {
            return *f;
        }
        if degree.is_cadential_six_four() {
            return HarmonicFunction::Dominant;
        }
        let root_pos = degree.root_position();
        if root_pos != degree {
            if let Some(f) = self.functions.get(&root_pos) {
                return *f;
            }
        }
        default_function(root_pos)
    }
}

/// Registry of named [`MarkovTable`]s. Generators look up tables by id
/// at generation time. Call [`TableRegistry::with_builtins`] to get a
/// registry pre-populated with all built-in tables.
pub struct TableRegistry {
    tables: HashMap<String, MarkovTable>,
}

impl TableRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            tables: HashMap::new(),
        }
    }

    /// Create a registry pre-populated with all built-in tables.
    pub fn with_builtins() -> Self {
        let mut reg = Self::new();
        reg.register(builtin_pop());
        reg.register(builtin_modal());
        reg.register(builtin_jazz());
        reg.register(builtin_post_rock());
        reg.register(builtin_metal());
        reg.register(builtin_classical());
        reg
    }

    /// Register a table. Overwrites any existing table with the same id.
    pub fn register(&mut self, table: MarkovTable) {
        self.tables.insert(table.id.clone(), table);
    }

    /// Look up a table by id.
    pub fn get(&self, id: &str) -> Option<&MarkovTable> {
        self.tables.get(id)
    }
}

impl Default for TableRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Shorthand aliases for Degree constants, so the table data below stays
// readable without requiring a glob import (Degree is a struct, not an
// enum, so `use Degree::*` is not valid).
// ---------------------------------------------------------------------------

// Major-key diatonic
const I: Degree = Degree::I;
const II: Degree = Degree::II_MIN;
const III: Degree = Degree::III_MIN;
const IV: Degree = Degree::IV;
const V: Degree = Degree::V;
const VI: Degree = Degree::VI_MIN;
const VIID: Degree = Degree::VII_DIM;

// Borrowed / modal
const BVI: Degree = Degree::FLAT_VI;
const BVII: Degree = Degree::FLAT_VII;

// Minor-key natural
const IMIN: Degree = Degree::I_MIN;
const IIIM: Degree = Degree::III_MAJ;
const IVMN: Degree = Degree::IV_MIN;
const VIM: Degree = Degree::VI_MAJ;
const VIIM: Degree = Degree::VII_MAJ;

// Seventh-chord
const I7: Degree = Degree::I_MAJ7;
const II7: Degree = Degree::II_MIN7;
const III7: Degree = Degree::III_MIN7;
const IV7: Degree = Degree::IV_MAJ7;
const V7: Degree = Degree::V_DOM7;
const VI7: Degree = Degree::VI_MIN7;
const VII7: Degree = Degree::VII_HALF7;

// ---------------------------------------------------------------------------
// Helpers for building tables compactly
// ---------------------------------------------------------------------------

/// Build an order-1 entry: single conditioning degree -> weighted successors.
fn t1(from: Degree, to: &[(Degree, f32)]) -> TableRow {
    (vec![from], to.to_vec())
}

/// Build an order-2 entry: pair of conditioning degrees -> weighted successors.
fn t2(a: Degree, b: Degree, to: &[(Degree, f32)]) -> TableRow {
    (vec![a, b], to.to_vec())
}

/// A single row in a transition table: key (conditioning history) to
/// weighted successors.
type TableRow = (Vec<Degree>, Vec<(Degree, f32)>);

/// Build a function-tag map from a slice of pairs.
fn tag(pairs: &[(Degree, HarmonicFunction)]) -> BTreeMap<Degree, HarmonicFunction> {
    pairs.iter().copied().collect()
}

// Shorthand for the tag tables below.
use HarmonicFunction::{Dominant as D, Predominant as PD, Tonic as T};

/// Build a complete order-2 table by starting with a base order-1
/// distribution for each degree, expanding it to all pairs (using the
/// last element's base distribution as the default), then applying
/// context-specific overrides for musically important pairs.
fn build_order2(
    id: &str,
    degrees: &[Degree],
    base: &[(Degree, Vec<(Degree, f32)>)],
    overrides: Vec<TableRow>,
    functions: BTreeMap<Degree, HarmonicFunction>,
) -> MarkovTable {
    let base_map: HashMap<Degree, Vec<(Degree, f32)>> = base.iter().cloned().collect();
    let mut transitions: BTreeMap<Vec<Degree>, Vec<(Degree, f32)>> = BTreeMap::new();

    // Fill every (A, B) pair with B's base distribution.
    for &a in degrees {
        for &b in degrees {
            if let Some(dist) = base_map.get(&b) {
                transitions.insert(vec![a, b], dist.clone());
            }
        }
    }

    // Apply overrides.
    for (key, dist) in overrides {
        transitions.insert(key, dist);
    }

    MarkovTable {
        id: id.to_string(),
        order: 2,
        transitions,
        functions,
    }
}

// ---------------------------------------------------------------------------
// Built-in tables
// ---------------------------------------------------------------------------

/// **Pop** (order 1) -- Biased toward the I-V-vi-IV family of circular
/// motion that dominates pop and rock from the 1950s onward. Strong tonic
/// gravity, clear dominant/subdominant roles, and a heavy vi weighting
/// ensure progressions feel "singable" and hook-friendly.
pub fn builtin_pop() -> MarkovTable {
    let transitions: BTreeMap<Vec<Degree>, Vec<(Degree, f32)>> = [
        t1(
            I,
            &[(IV, 0.30), (V, 0.25), (VI, 0.25), (II, 0.10), (III, 0.10)],
        ),
        t1(
            II,
            &[(V, 0.45), (IV, 0.20), (I, 0.15), (VI, 0.15), (III, 0.05)],
        ),
        t1(
            III,
            &[(VI, 0.35), (IV, 0.30), (II, 0.15), (I, 0.10), (V, 0.10)],
        ),
        t1(
            IV,
            &[(V, 0.35), (I, 0.30), (VI, 0.15), (II, 0.10), (III, 0.10)],
        ),
        t1(
            V,
            &[(I, 0.40), (VI, 0.30), (IV, 0.15), (II, 0.10), (III, 0.05)],
        ),
        t1(
            VI,
            &[(IV, 0.35), (II, 0.20), (V, 0.20), (I, 0.15), (III, 0.10)],
        ),
    ]
    .into_iter()
    .collect();

    MarkovTable {
        id: "pop".to_string(),
        order: 1,
        transitions,
        // Textbook major-key roles: iii and vi are tonic substitutes.
        functions: tag(&[(I, T), (II, PD), (III, T), (IV, PD), (V, D), (VI, T)]),
    }
}

/// **Modal** (order 1) -- Weights up bVII, bVI, and plagal (IV->I) motion
/// for a Mixolydian / Lydian / Aeolian flavour. Less emphasis on V->I
/// resolution; more lateral, "colour" movement. Good for ambient, folk,
/// and film-score writing where you want harmonic interest without
/// functional gravity.
pub fn builtin_modal() -> MarkovTable {
    let transitions: BTreeMap<Vec<Degree>, Vec<(Degree, f32)>> = [
        t1(
            I,
            &[
                (IV, 0.20),
                (BVII, 0.25),
                (V, 0.15),
                (VI, 0.10),
                (BVI, 0.10),
                (II, 0.10),
                (III, 0.10),
            ],
        ),
        t1(
            II,
            &[
                (V, 0.25),
                (IV, 0.20),
                (BVII, 0.20),
                (I, 0.15),
                (VI, 0.10),
                (BVI, 0.10),
            ],
        ),
        t1(
            III,
            &[
                (IV, 0.25),
                (VI, 0.20),
                (BVII, 0.15),
                (I, 0.15),
                (II, 0.15),
                (BVI, 0.10),
            ],
        ),
        t1(
            IV,
            &[
                (I, 0.25),
                (BVII, 0.20),
                (V, 0.15),
                (BVI, 0.15),
                (VI, 0.10),
                (II, 0.10),
                (III, 0.05),
            ],
        ),
        t1(
            V,
            &[
                (I, 0.30),
                (VI, 0.15),
                (IV, 0.15),
                (BVII, 0.15),
                (BVI, 0.10),
                (II, 0.10),
                (III, 0.05),
            ],
        ),
        t1(
            VI,
            &[
                (IV, 0.20),
                (BVII, 0.20),
                (V, 0.15),
                (I, 0.15),
                (BVI, 0.15),
                (II, 0.10),
                (III, 0.05),
            ],
        ),
        t1(
            BVI,
            &[
                (BVII, 0.40),
                (IV, 0.20),
                (I, 0.15),
                (V, 0.10),
                (VI, 0.10),
                (II, 0.05),
            ],
        ),
        t1(
            BVII,
            &[
                (I, 0.30),
                (IV, 0.25),
                (BVI, 0.15),
                (V, 0.10),
                (VI, 0.10),
                (II, 0.05),
                (III, 0.05),
            ],
        ),
    ]
    .into_iter()
    .collect();

    MarkovTable {
        id: "modal".to_string(),
        order: 1,
        transitions,
        // bVII is tagged Dominant (the mixolydian dominant substitute);
        // bVI Predominant (it prepares bVII or V in the aeolian cadence
        // bVI–bVII–I). The diatonic degrees keep their textbook roles.
        functions: tag(&[
            (I, T),
            (II, PD),
            (III, T),
            (IV, PD),
            (V, D),
            (VI, T),
            (BVI, PD),
            (BVII, D),
        ]),
    }
}

/// **Post-rock** (order 1) -- Sustained, cyclical movement with heavy
/// I<->IV and I<->vi orbits. Reduced dominant gravity (V is less
/// "resolving" here), bVII for modal colour, and iii for an ethereal,
/// dreamy quality. Think Explosions in the Sky or Mogwai: long builds
/// over slowly shifting harmony.
pub fn builtin_post_rock() -> MarkovTable {
    let transitions: BTreeMap<Vec<Degree>, Vec<(Degree, f32)>> = [
        t1(
            I,
            &[
                (IV, 0.30),
                (VI, 0.25),
                (BVII, 0.15),
                (III, 0.10),
                (V, 0.10),
                (II, 0.10),
            ],
        ),
        t1(
            II,
            &[
                (IV, 0.25),
                (V, 0.20),
                (I, 0.20),
                (VI, 0.15),
                (III, 0.10),
                (BVII, 0.10),
            ],
        ),
        t1(
            III,
            &[
                (IV, 0.25),
                (VI, 0.20),
                (I, 0.20),
                (BVII, 0.15),
                (V, 0.10),
                (II, 0.10),
            ],
        ),
        t1(
            IV,
            &[
                (I, 0.30),
                (VI, 0.20),
                (V, 0.15),
                (BVII, 0.15),
                (II, 0.10),
                (III, 0.10),
            ],
        ),
        t1(
            V,
            &[
                (IV, 0.25),
                (I, 0.25),
                (VI, 0.20),
                (BVII, 0.10),
                (II, 0.10),
                (III, 0.10),
            ],
        ),
        t1(
            VI,
            &[
                (IV, 0.30),
                (I, 0.25),
                (BVII, 0.15),
                (III, 0.10),
                (V, 0.10),
                (II, 0.10),
            ],
        ),
        t1(
            BVII,
            &[
                (I, 0.30),
                (IV, 0.25),
                (VI, 0.15),
                (V, 0.10),
                (III, 0.10),
                (II, 0.10),
            ],
        ),
    ]
    .into_iter()
    .collect();

    MarkovTable {
        id: "post-rock".to_string(),
        order: 1,
        transitions,
        // Same calls as the modal table: bVII acts as the dominant
        // substitute in this style's reduced-V harmonic language.
        functions: tag(&[
            (I, T),
            (II, PD),
            (III, T),
            (IV, PD),
            (V, D),
            (VI, T),
            (BVII, D),
        ]),
    }
}

/// **Metal** (order 1) -- Minor-key power-chord motion built around the
/// i-bVI-bVII-i spine that defines metal from Black Sabbath onward.
/// Heavy tonic return, prominent bVI and bVII, and a major V for the
/// harmonic-minor "evil" sound. Use with a minor scale (natural minor,
/// Phrygian, harmonic minor) for best results.
///
/// Degrees use minor-key conventions: i (`I_MIN`), III (`III_MAJ`),
/// iv (`IV_MIN`), V (`V`), VI (`VI_MAJ`), VII (`VII_MAJ`). These are
/// the natural degrees of a minor scale without the `flat` modifier.
pub fn builtin_metal() -> MarkovTable {
    let transitions: BTreeMap<Vec<Degree>, Vec<(Degree, f32)>> = [
        t1(
            IMIN,
            &[
                (VIM, 0.25),
                (VIIM, 0.20),
                (IVMN, 0.20),
                (V, 0.15),
                (IIIM, 0.10),
                (IMIN, 0.10),
            ],
        ),
        t1(
            IIIM,
            &[
                (IVMN, 0.20),
                (VIM, 0.20),
                (VIIM, 0.20),
                (IMIN, 0.20),
                (V, 0.15),
                (IIIM, 0.05),
            ],
        ),
        t1(
            IVMN,
            &[
                (IMIN, 0.25),
                (V, 0.20),
                (VIIM, 0.20),
                (VIM, 0.15),
                (IIIM, 0.10),
                (IVMN, 0.10),
            ],
        ),
        t1(
            V,
            &[
                (IMIN, 0.40),
                (VIM, 0.15),
                (IVMN, 0.15),
                (VIIM, 0.15),
                (IIIM, 0.10),
                (V, 0.05),
            ],
        ),
        t1(
            VIM,
            &[
                (VIIM, 0.35),
                (IVMN, 0.15),
                (V, 0.15),
                (IMIN, 0.15),
                (IIIM, 0.10),
                (VIM, 0.10),
            ],
        ),
        t1(
            VIIM,
            &[
                (IMIN, 0.35),
                (VIM, 0.15),
                (IVMN, 0.15),
                (V, 0.10),
                (IIIM, 0.15),
                (VIIM, 0.10),
            ],
        ),
    ]
    .into_iter()
    .collect();

    MarkovTable {
        id: "metal".to_string(),
        order: 1,
        transitions,
        // Minor-mode mapping. Idiosyncratic calls: VI is tagged
        // Predominant (in the i–VI–VII–i spine it prepares the
        // subtonic, unlike the major-key vi tonic substitute) and VII
        // Dominant (subtonic dominant substitute). III is the
        // relative-major tonic substitute; V keeps its harmonic-minor
        // dominant role.
        functions: tag(&[
            (IMIN, T),
            (IIIM, T),
            (IVMN, PD),
            (V, D),
            (VIM, PD),
            (VIIM, D),
        ]),
    }
}

/// **Jazz** (order 2) -- Seventh-chord vocabulary with strong ii-V-I
/// cadential gravity and circle-of-fifths motion. The second-order
/// conditioning captures the crucial context that makes jazz progressions
/// idiomatic: after ii-V the pull to I is overwhelming, but after V-I
/// the field opens up for the next phrase. Covers the full diatonic
/// seventh-chord set.
pub fn builtin_jazz() -> MarkovTable {
    let degrees = [I7, II7, III7, IV7, V7, VI7, VII7];

    let base: Vec<(Degree, Vec<(Degree, f32)>)> = vec![
        (
            I7,
            vec![
                (II7, 0.25),
                (VI7, 0.25),
                (IV7, 0.20),
                (V7, 0.15),
                (III7, 0.10),
                (VII7, 0.05),
            ],
        ),
        (
            II7,
            vec![
                (V7, 0.50),
                (III7, 0.15),
                (IV7, 0.15),
                (VII7, 0.10),
                (I7, 0.05),
                (VI7, 0.05),
            ],
        ),
        (
            III7,
            vec![
                (VI7, 0.40),
                (IV7, 0.20),
                (II7, 0.15),
                (VII7, 0.10),
                (I7, 0.10),
                (V7, 0.05),
            ],
        ),
        (
            IV7,
            vec![
                (V7, 0.30),
                (II7, 0.20),
                (III7, 0.15),
                (VI7, 0.15),
                (I7, 0.10),
                (VII7, 0.10),
            ],
        ),
        (
            V7,
            vec![
                (I7, 0.45),
                (VI7, 0.25),
                (IV7, 0.10),
                (III7, 0.10),
                (II7, 0.05),
                (VII7, 0.05),
            ],
        ),
        (
            VI7,
            vec![
                (II7, 0.40),
                (V7, 0.20),
                (IV7, 0.15),
                (III7, 0.10),
                (I7, 0.10),
                (VII7, 0.05),
            ],
        ),
        (
            VII7,
            vec![
                (III7, 0.35),
                (I7, 0.20),
                (VI7, 0.15),
                (V7, 0.15),
                (II7, 0.10),
                (IV7, 0.05),
            ],
        ),
    ];

    let overrides = vec![
        // ii-V -> I (the defining jazz cadence; overwhelming tonic pull)
        t2(
            II7,
            V7,
            &[
                (I7, 0.60),
                (VI7, 0.20),
                (IV7, 0.10),
                (III7, 0.05),
                (II7, 0.03),
                (VII7, 0.02),
            ],
        ),
        // V-I -> open field (new phrase; spread across cycle starters)
        t2(
            V7,
            I7,
            &[
                (II7, 0.25),
                (VI7, 0.25),
                (IV7, 0.25),
                (V7, 0.10),
                (III7, 0.10),
                (VII7, 0.05),
            ],
        ),
        // I-vi -> ii (turnaround start)
        t2(
            I7,
            VI7,
            &[
                (II7, 0.50),
                (V7, 0.15),
                (IV7, 0.15),
                (III7, 0.10),
                (I7, 0.05),
                (VII7, 0.05),
            ],
        ),
        // vi-ii -> V (turnaround continuation)
        t2(
            VI7,
            II7,
            &[
                (V7, 0.60),
                (IV7, 0.15),
                (III7, 0.10),
                (I7, 0.05),
                (VII7, 0.05),
                (VI7, 0.05),
            ],
        ),
        // iii-vi -> ii (circle of fifths)
        t2(
            III7,
            VI7,
            &[
                (II7, 0.50),
                (V7, 0.15),
                (IV7, 0.15),
                (I7, 0.10),
                (III7, 0.05),
                (VII7, 0.05),
            ],
        ),
        // V-vi -> ii (deceptive cadence recovery)
        t2(
            V7,
            VI7,
            &[
                (II7, 0.45),
                (IV7, 0.20),
                (V7, 0.10),
                (III7, 0.10),
                (I7, 0.10),
                (VII7, 0.05),
            ],
        ),
        // I-ii -> V (starting a ii-V from tonic)
        t2(
            I7,
            II7,
            &[
                (V7, 0.55),
                (IV7, 0.15),
                (III7, 0.10),
                (VII7, 0.10),
                (I7, 0.05),
                (VI7, 0.05),
            ],
        ),
        // vii-iii -> vi (circle of fifths)
        t2(
            VII7,
            III7,
            &[
                (VI7, 0.50),
                (II7, 0.15),
                (IV7, 0.15),
                (I7, 0.10),
                (V7, 0.05),
                (VII7, 0.05),
            ],
        ),
    ];

    build_order2(
        "jazz",
        &degrees,
        &base,
        overrides,
        // Seventh-chord analogues of the textbook roles; viiø7 is a
        // rootless V9 in practice, hence Dominant.
        tag(&[
            (I7, T),
            (II7, PD),
            (III7, T),
            (IV7, PD),
            (V7, D),
            (VI7, T),
            (VII7, D),
        ]),
    )
}

/// **Classical** (order 2) -- Functional harmony with strong authentic
/// cadences (V-I), well-defined pre-dominant function (ii, IV -> V),
/// and circle-of-fifths root motion. The second-order conditioning
/// captures cadential preparation: after IV-V or ii-V the resolution
/// to I is nearly certain. Deceptive cadences (V-vi) are present but
/// rare, and the recovery path is idiomatic (vi -> IV or ii).
pub fn builtin_classical() -> MarkovTable {
    let degrees = [I, II, III, IV, V, VI, VIID];

    let base: Vec<(Degree, Vec<(Degree, f32)>)> = vec![
        (
            I,
            vec![
                (V, 0.25),
                (IV, 0.25),
                (VI, 0.15),
                (II, 0.15),
                (III, 0.10),
                (VIID, 0.10),
            ],
        ),
        (
            II,
            vec![
                (V, 0.45),
                (VIID, 0.15),
                (IV, 0.15),
                (I, 0.10),
                (III, 0.10),
                (VI, 0.05),
            ],
        ),
        (
            III,
            vec![
                (VI, 0.35),
                (IV, 0.25),
                (II, 0.15),
                (I, 0.10),
                (V, 0.10),
                (VIID, 0.05),
            ],
        ),
        (
            IV,
            vec![
                (V, 0.35),
                (I, 0.20),
                (II, 0.15),
                (VI, 0.10),
                (VIID, 0.10),
                (III, 0.10),
            ],
        ),
        (
            V,
            vec![
                (I, 0.50),
                (VI, 0.20),
                (IV, 0.10),
                (II, 0.10),
                (III, 0.05),
                (VIID, 0.05),
            ],
        ),
        (
            VI,
            vec![
                (II, 0.30),
                (IV, 0.25),
                (V, 0.15),
                (I, 0.10),
                (III, 0.10),
                (VIID, 0.10),
            ],
        ),
        (
            VIID,
            vec![
                (I, 0.55),
                (VI, 0.15),
                (III, 0.10),
                (IV, 0.10),
                (V, 0.05),
                (II, 0.05),
            ],
        ),
    ];

    let overrides = vec![
        // IV-V -> I (authentic cadence via predominant)
        t2(
            IV,
            V,
            &[
                (I, 0.55),
                (VI, 0.20),
                (III, 0.05),
                (II, 0.05),
                (IV, 0.05),
                (VIID, 0.10),
            ],
        ),
        // ii-V -> I (authentic cadence via supertonic)
        t2(
            II,
            V,
            &[
                (I, 0.60),
                (VI, 0.20),
                (III, 0.05),
                (IV, 0.05),
                (II, 0.05),
                (VIID, 0.05),
            ],
        ),
        // V-I -> subdominant region (fresh start after cadence)
        t2(
            V,
            I,
            &[
                (IV, 0.25),
                (VI, 0.20),
                (II, 0.20),
                (V, 0.15),
                (III, 0.10),
                (VIID, 0.10),
            ],
        ),
        // V-vi -> IV (deceptive cadence recovery)
        t2(
            V,
            VI,
            &[
                (IV, 0.30),
                (II, 0.30),
                (V, 0.15),
                (I, 0.10),
                (III, 0.10),
                (VIID, 0.05),
            ],
        ),
        // vi-ii -> V (circle of fifths continuation)
        t2(
            VI,
            II,
            &[
                (V, 0.55),
                (IV, 0.15),
                (VIID, 0.10),
                (I, 0.10),
                (III, 0.05),
                (VI, 0.05),
            ],
        ),
        // iii-vi -> ii (circle of fifths)
        t2(
            III,
            VI,
            &[
                (II, 0.40),
                (IV, 0.25),
                (V, 0.15),
                (I, 0.10),
                (VIID, 0.05),
                (III, 0.05),
            ],
        ),
        // vii-I -> subdominant (after leading-tone resolution)
        t2(
            VIID,
            I,
            &[
                (IV, 0.30),
                (V, 0.20),
                (VI, 0.15),
                (II, 0.15),
                (III, 0.10),
                (VIID, 0.10),
            ],
        ),
        // I-V -> I or vi (tonic-dominant elaboration)
        t2(
            I,
            V,
            &[
                (I, 0.45),
                (VI, 0.25),
                (IV, 0.10),
                (III, 0.10),
                (II, 0.05),
                (VIID, 0.05),
            ],
        ),
    ];

    build_order2(
        "classical",
        &degrees,
        &base,
        overrides,
        tag(&[
            (I, T),
            (II, PD),
            (III, T),
            (IV, PD),
            (V, D),
            (VI, T),
            (VIID, D),
        ]),
    )
}
