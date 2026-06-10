//! Shared helpers used by every per-generator lane-inspector handler.
//!
//! These centralise the "look up the definition → look up the lane → mutate
//! the generator kind" pattern so each per-parameter setter stays a one-liner.

use resonance_audio::types::TrackId;
use resonance_music_theory::VocalParams;

use crate::compose::{LaneGeneratorKind};
use crate::util::bump_seed;

/// Bump a lane's seed by `seed_mix + 1`. Centralised so the vocal regen
/// path (which dispatches through `regenerate_lane`) can reuse the same
/// seed without double-bumping in `roll_vocal_melody`.
pub(super) fn bump_lane_seed(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    seed_mix: u64,
) {
    if let Some(def) = r.compose.find_definition_mut(definition_id) {
        if let Some(cfg) = def.lane_generators.get_mut(&track_id) {
            cfg.seed = bump_seed(cfg.seed, seed_mix);
        }
    }
}

/// Mutate a lane's generator kind in-place.
pub(super) fn update_lane_gen(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    f: impl FnOnce(&mut LaneGeneratorKind),
) {
    if let Some(def) = r.compose.find_definition_mut(definition_id) {
        if let Some(cfg) = def.lane_generators.get_mut(&track_id) {
            f(&mut cfg.kind);
        }
        r.compose.last_error = None;
    }
}

/// Mutate the vocal params of a lane in-place. Skips silently when the
/// lane has a different generator kind installed.
///
/// `pub(crate)` because `update::compose::vocal_lyrics` reaches in to
/// reflect text-editor edits back into the lane's `VocalParams::draft`.
pub(crate) fn update_vocal(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    f: impl FnOnce(&mut VocalParams),
) {
    update_lane_gen(r, definition_id, track_id, |kind| {
        if let LaneGeneratorKind::Vocal(p) = kind {
            f(p);
        }
    });
}
