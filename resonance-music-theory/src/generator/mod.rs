//! Generative material system for sections.
//!
//! A section can optionally carry a [`GeneratorSpec`] describing how its
//! content should be produced. Generation is a **pure, deterministic
//! function** of (spec, seed, context): the same spec with the same seed
//! and the same locked chords always produces identical output.
//!
//! Both the spec and the materialized output are persisted. The spec is
//! provenance ("how was this made?"); the output is what downstream code
//! (sequencer, UI) actually reads. Re-generating bumps the seed (or
//! accepts an explicit one) and re-runs the generator; locked elements
//! carry through unchanged.
//!
//! # Locking
//!
//! Every element in [`GeneratedMaterial`] carries a `locked` flag.
//! Locked elements are fixed waypoints that regeneration must preserve.
//! Locks are passed *into* the generator via [`GenContext`], not applied
//! after the fact. For the Markov generator, locked chords partition the
//! output into gaps that are filled independently, conditioned on the
//! locked neighbours.
//!
//! # Adding a new generator
//!
//! 1. Add a variant to [`GeneratorSpec`] (tagged enum -- old JSON stays valid).
//! 2. Implement the sampling logic as a function in a new submodule.
//! 3. Wire the variant into [`Generator`] for [`GeneratorSpec`].

pub mod degree;
pub mod markov;
pub mod schema;
pub mod table;

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub use degree::Degree;
pub use schema::SchemaKind;
pub use table::{HarmonicFunction, MarkovTable, TableRegistry};

/// Describes how to generate material for a section. Serialized with an
/// internally-tagged `"type"` discriminator so new variants extend the
/// JSON schema without breaking existing project files.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum GeneratorSpec {
    /// Sample a chord progression from a Markov chain over scale degrees.
    MarkovProgression {
        /// Number of chords to produce.
        length: u8,
        /// Id of the [`MarkovTable`] to sample from (looked up in
        /// [`TableRegistry`] at generation time).
        table_id: String,
        /// Conditioning history length. If shorter than the table's order,
        /// the generator backs off automatically; if longer, extra history
        /// is simply ignored.
        order: u8,
        /// Force the first chord to this degree (unless position 0 is locked).
        #[serde(default)]
        start: Option<Degree>,
        /// Force the last chord to this degree (unless the last position
        /// is locked). Returns [`GenerateError::EndUnreachable`] if the
        /// table cannot produce a path that ends on this degree.
        #[serde(default)]
        end: Option<Degree>,
    },
    /// Realize a canonical pop/rock/blues schema (12-bar blues, axis,
    /// doo-wop, ...) with optional rotation and function-preserving
    /// substitution. See [`schema::SchemaKind`].
    Schema {
        /// Which schema to realize.
        schema: SchemaKind,
        /// Number of chords to produce. The schema loop is tiled
        /// cyclically to fill this length (use
        /// [`SchemaKind::default_length`] for one pass).
        length: u8,
        /// Rotate the base loop by this many positions before tiling
        /// (e.g. axis rotation 1 = V-vi-IV-I). Wraps modulo the loop
        /// length.
        #[serde(default)]
        rotation: u8,
        /// Per-position probability (0..=1) of swapping the canonical
        /// chord for a substitute sharing >= 2 pitch classes with it.
        #[serde(default)]
        substitution: f32,
    },
}

/// Pure generation interface. Implementations must be deterministic:
/// the same `(seed, ctx)` must always produce the same output.
pub trait Generator {
    /// Generate material from a seed and context.
    fn generate(&self, seed: u64, ctx: &GenContext) -> Result<GeneratedMaterial, GenerateError>;
}

impl Generator for GeneratorSpec {
    fn generate(&self, seed: u64, ctx: &GenContext) -> Result<GeneratedMaterial, GenerateError> {
        match self {
            GeneratorSpec::MarkovProgression {
                length,
                table_id,
                order,
                start,
                end,
            } => markov::generate(*length, table_id, *order, *start, *end, seed, ctx),
            GeneratorSpec::Schema {
                schema,
                length,
                rotation,
                substitution,
            } => schema::generate(*schema, *length, *rotation, *substitution, seed, ctx),
        }
    }
}

/// Context passed to generators at generation time.
pub struct GenContext<'a> {
    /// Table registry for Markov generators.
    pub registry: &'a TableRegistry,
    /// Locked positions. Length must equal the spec's requested output
    /// length. `Some(degree)` means that position is fixed and must not
    /// change; `None` means the generator is free to fill it.
    pub locked: &'a [Option<Degree>],
}

/// A single generated chord with its lock state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeneratedChord {
    /// The scale degree produced (or preserved) at this position.
    pub degree: Degree,
    /// If true, this chord was carried through from `GenContext::locked`
    /// and was not sampled.
    pub locked: bool,
}

/// A harmonic-rhythm split produced by the phrase-model overlay: the
/// grid slot `slot` is divided in half, with the slot's main chord
/// (`GeneratedMaterial::chords[slot]`) taking the first half and
/// `degree` the second. Used to accelerate harmonic rhythm into the
/// cadence (e.g. `| I | vi | IV ii | V |`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SplitChord {
    /// Index into `GeneratedMaterial::chords` of the slot being split.
    pub slot: u8,
    /// The chord occupying the second half of the slot.
    pub degree: Degree,
}

/// The materialized output of a generator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeneratedMaterial {
    /// One chord per requested position (grid slot). Locking and
    /// regeneration are slot-positional over this list.
    pub chords: Vec<GeneratedChord>,
    /// Harmonic-rhythm splits layered on top of `chords` by the
    /// phrase-model overlay. Kept out of `chords` so slot-positional
    /// semantics (and material persisted before this field existed)
    /// stay intact.
    #[serde(default)]
    pub splits: Vec<SplitChord>,
}

/// Errors that can occur during generation.
#[derive(Debug, Error)]
pub enum GenerateError {
    /// The requested table id was not found in the registry.
    #[error("table '{0}' not found in registry")]
    TableNotFound(String),
    /// The end-degree constraint cannot be reached from the current state
    /// within the remaining number of transitions.
    #[error("end degree unreachable within {steps} step(s)")]
    EndUnreachable {
        /// How many transitions remained when the generator gave up.
        steps: usize,
    },
    /// The end-degree constraint conflicts with a locked chord at the
    /// last position.
    #[error("end degree conflicts with locked chord at last position")]
    EndConflictsWithLock,
    /// The requested Markov table has no transitions at all (or only
    /// transitions whose targets aren't reachable from the current
    /// history with any back-off). A user-registered empty table would
    /// otherwise panic in `weighted_sample`.
    #[error("Markov table '{0}' has no usable transitions")]
    EmptyTable(String),
}
