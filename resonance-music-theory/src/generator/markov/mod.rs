//! Markov chain generator for chord progressions.
//!
//! Given a [`MarkovTable`], a seed, and optional constraints (start / end
//! degrees, locked positions), samples a chord progression of a requested
//! length. Locked chords act as fixed waypoints: the generator fills the
//! gaps between them, biasing toward degrees that can reach the next
//! waypoint as the gap narrows.
//!
//! # Pipeline
//!
//! [`generate`] is an eight-phase pipeline, one named step each:
//!
//! 1. [`place_locked_chords`] — the caller's locked waypoints;
//! 2. [`place_constraints`] — the spec's start / end degrees;
//! 3. [`precompute`] — order-1 graph, degree pool, phrase plans;
//! 4. [`fill::fill_gaps`] — sample every remaining slot;
//! 5. [`validate_end_constraint`] — did the walk honour `end`?
//! 6. [`cadence::sample_cadence_splits`] — harmonic-rhythm acceleration;
//! 7. [`assemble`] — unwrap the slots into the output list;
//! 8. [`super::inversion::decorate_inversions`] — bass-line idioms.
//!
//! # Phrase-model overlay
//!
//! On top of the raw Markov walk, a phrase-model overlay (Open Music
//! Theory's T→PD→D functional arc, see [`phrase`]) shapes the output:
//!
//! - Slots are grouped into phrases of [`phrase::PHRASE_SLOTS`] (a 4-bar
//!   group at the app's default of one chord per bar).
//! - Within each phrase the walk is masked to traverse the arc exactly
//!   once: it opens on tonic function, may prolong T or move to
//!   predominant (never regressing once PD is reached), is forced to PD
//!   on the penultimate slot, and places the cadential dominant on the
//!   phrase-final slot — so the dominant starts on the downbeat of the
//!   group's last bar and resolves onto the next hyper-downbeat (the
//!   following phrase's tonic opening, or the loop start). Premature
//!   D→T resolutions and T/PD ping-pong mid-phrase are impossible by
//!   construction.
//! - Harmonic rhythm accelerates into the cadence: the forced-PD slot
//!   of each full phrase is split in half (e.g. `| I | vi | IV ii | V |`),
//!   recorded as [`SplitChord`](super::SplitChord)s alongside the
//!   slot-aligned chords.
//! - User constraints always win: locked slots and start/end degrees are
//!   never masked, and when a phrase ends on a fixed tonic the cadence
//!   shifts left (`… PD D | T`) instead of fighting the constraint.
//!
//! # Determinism
//!
//! Generated material is persisted inside user projects, so `generate`
//! is a pure function of its inputs *and* its RNG draw sequence is part
//! of the contract: changing how many values are drawn, or in what
//! order, silently rewrites existing songs. `tests/markov_golden.rs`
//! pins concrete output for a matrix of seeds against recorded values.

mod cadence;
mod candidates;
mod fill;
pub mod phrase;

use crate::rng::XorShift;

use super::degree::Degree;
use super::table::MarkovTable;
use super::{GenContext, GenerateError, GeneratedChord, GeneratedMaterial};

use candidates::{collect_all_degrees, marginalize_to_order1, Order1Graph, SuffixCache};
use fill::{FillContext, FillState};
use phrase::build_phrase_plans;

/// When within this many transitions of a fixed successor, boost the
/// probability of degrees that can reach it.
const BIAS_WINDOW: usize = 3;

/// Multiplicative boost applied to reachable degrees inside the bias
/// window. Larger values make the generator more likely to hit the
/// target but reduce variety.
const REACHABILITY_BOOST: f32 = 5.0;

/// Sample a Markov chord progression.
///
/// This is the entry point called by `GeneratorSpec::MarkovProgression`.
/// The function is pure: identical inputs always produce identical output.
pub fn generate(
    length: u8,
    table_id: &str,
    order: u8,
    start: Option<Degree>,
    end: Option<Degree>,
    seed: u64,
    ctx: &GenContext,
) -> Result<GeneratedMaterial, GenerateError> {
    let table = ctx
        .registry
        .get(table_id)
        .ok_or_else(|| GenerateError::TableNotFound(table_id.to_string()))?;

    let len = length as usize;
    if len == 0 {
        return Ok(GeneratedMaterial {
            chords: vec![],
            splits: vec![],
        });
    }

    let mut rng = XorShift::new(seed);

    // --- 1. Place locked chords ----------------------------------------
    let mut output = place_locked_chords(len, ctx);

    // --- 2. Place start / end constraints on unlocked positions --------
    place_constraints(&mut output, start, end);

    // --- 3. Precompute helpers -----------------------------------------
    let pre = precompute(&output, table, table_id, order)?;
    let mut cache: SuffixCache = SuffixCache::new();

    // --- 4. Fill gaps --------------------------------------------------
    let fill_ctx = FillContext {
        table,
        order1: &pre.order1,
        all_degrees: &pre.all_degrees,
        effective_order: pre.effective_order,
        plans: &pre.plans,
        prefixed: &pre.prefixed,
        end,
    };
    fill::fill_gaps(
        &fill_ctx,
        &mut FillState {
            output: &mut output,
            cache: &mut cache,
            rng: &mut rng,
        },
    )?;

    // --- 5. Validate end constraint ------------------------------------
    validate_end_constraint(&output, end)?;

    // --- 6. Harmonic-rhythm acceleration into the cadence ---------------
    let mut splits = cadence::sample_cadence_splits(
        &output,
        &pre.split_slots,
        &pre.prefixed,
        table,
        &pre.order1,
        pre.effective_order,
        &mut cache,
        &mut rng,
    );

    // --- 7. Assemble output --------------------------------------------
    let mut chords = assemble(output);

    // --- 8. Inversion decorations (research §2C) -------------------------
    // Pre-dominant bass idioms (IV-precedes-ii ordering, ii6 walking the
    // bass 4→5) and the cadential 6/4 on phrase-final dominants. Sampled
    // material only — locked and constrained slots carry through
    // untouched. See `super::inversion`.
    super::inversion::decorate_inversions(
        &mut chords,
        &mut splits,
        &pre.prefixed,
        &pre.plans,
        table,
        &mut rng,
    );

    Ok(GeneratedMaterial { chords, splits })
}

// ---------------------------------------------------------------------------
// Phase 1 / 2: fixed slots
// ---------------------------------------------------------------------------

/// Phase 1: seed the slot list with the caller's locked chords.
fn place_locked_chords(len: usize, ctx: &GenContext) -> Vec<Option<GeneratedChord>> {
    let mut output: Vec<Option<GeneratedChord>> = vec![None; len];
    for (i, slot) in ctx.locked.iter().enumerate().take(len) {
        if let Some(degree) = slot {
            output[i] = Some(GeneratedChord {
                degree: *degree,
                locked: true,
            });
        }
    }
    output
}

/// Phase 2: apply the spec's start / end degrees, which yield to locks.
fn place_constraints(
    output: &mut [Option<GeneratedChord>],
    start: Option<Degree>,
    end: Option<Degree>,
) {
    let len = output.len();
    if let Some(start_deg) = start {
        if output[0].is_none() {
            output[0] = Some(GeneratedChord {
                degree: start_deg,
                locked: false,
            });
        }
    }
    if let Some(end_deg) = end {
        if output[len - 1].is_none() {
            output[len - 1] = Some(GeneratedChord {
                degree: end_deg,
                locked: false,
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Phase 3: precomputation shared by the sampling phases
// ---------------------------------------------------------------------------

/// Everything phases 4, 6 and 8 need that depends only on the table and
/// the fixed slots — computed once per `generate` call.
struct Precomputed {
    /// Order-1 view of the table, for reachability queries.
    order1: Order1Graph,
    /// Every degree the table knows, sorted; the empty-row fallback pool.
    all_degrees: Vec<Degree>,
    /// Snapshot of the degrees fixed before sampling (locks + start/end).
    prefixed: Vec<Option<Degree>>,
    /// Per-slot `(min, max)` harmonic-function windows.
    plans: Vec<(u8, u8)>,
    /// Slots eligible for harmonic-rhythm splitting.
    split_slots: Vec<usize>,
    /// Conditioning length actually used when looking up transitions.
    effective_order: usize,
}

/// Phase 3: build the order-1 graph, the degree pool and the phrase plans.
fn precompute(
    output: &[Option<GeneratedChord>],
    table: &MarkovTable,
    table_id: &str,
    order: u8,
) -> Result<Precomputed, GenerateError> {
    let order1 = marginalize_to_order1(table);
    let all_degrees = collect_all_degrees(table);

    // Snapshot of pre-placed degrees (locks + start/end constraints).
    // The phrase plans are built against these, fixed slots are never
    // masked, and a fixed slot resets the arc level (a user lock
    // legitimately restarts the phrase arc).
    let prefixed: Vec<Option<Degree>> = output
        .iter()
        .map(|slot| slot.as_ref().map(|c| c.degree))
        .collect();
    let (plans, split_slots) = build_phrase_plans(output.len(), &prefixed, table);

    // A user-registered table with no transitions would otherwise drive
    // `weighted_sample` past its assertions and panic in release. The
    // registry is open (third parties register via `register`), so this
    // is reachable through the public API.
    if all_degrees.is_empty() {
        return Err(GenerateError::EmptyTable(table_id.to_string()));
    }

    Ok(Precomputed {
        order1,
        all_degrees,
        prefixed,
        plans,
        split_slots,
        // Use the spec's order as the effective conditioning length,
        // which may be shorter than the table's order (forcing back-off)
        // or longer (extra history is simply ignored).
        effective_order: order as usize,
    })
}

// ---------------------------------------------------------------------------
// Phase 5 / 7: validation and assembly
// ---------------------------------------------------------------------------

/// Phase 5: the walk may have failed to land on the requested `end`.
fn validate_end_constraint(
    output: &[Option<GeneratedChord>],
    end: Option<Degree>,
) -> Result<(), GenerateError> {
    let Some(end_deg) = end else {
        return Ok(());
    };
    let last = output[output.len() - 1]
        .as_ref()
        .expect("all positions should be filled");
    if last.degree == end_deg {
        return Ok(());
    }
    if last.locked {
        Err(GenerateError::EndConflictsWithLock)
    } else {
        Err(GenerateError::EndUnreachable { steps: 0 })
    }
}

/// Phase 7: every slot is filled by now, so drop the `Option` layer.
fn assemble(output: Vec<Option<GeneratedChord>>) -> Vec<GeneratedChord> {
    output
        .into_iter()
        .map(|o| o.expect("all positions should be filled"))
        .collect()
}
