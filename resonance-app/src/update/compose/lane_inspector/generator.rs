//! `LaneInspectorMsg::SetGenerator`: switch a lane's generator kind.

use resonance_audio::types::TrackId;
use resonance_music_theory::{BassParams, MelodyParams, PadParams, VocalParams};

use crate::compose::{LaneGeneratorConfig, LaneGeneratorKind, LaneGeneratorKindTag};
use crate::util::seed_from_id;

/// Switch the generator type for a lane. Manual removes the entry; the
/// other tags install a default-params generator with a deterministic
/// per-tag seed derived from the section's definition id.
pub(super) fn set_generator(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    tag: LaneGeneratorKindTag,
) {
    if let Some(def) = r.compose.find_definition_mut(definition_id) {
        match tag {
            LaneGeneratorKindTag::Manual => {
                def.lane_generators.remove(&track_id);
            }
            LaneGeneratorKindTag::Bass => {
                def.lane_generators.insert(
                    track_id,
                    LaneGeneratorConfig {
                        kind: LaneGeneratorKind::Bass(BassParams::default()),
                        seed: seed_from_id(definition_id),
                    },
                );
            }
            LaneGeneratorKindTag::Melody => {
                def.lane_generators.insert(
                    track_id,
                    LaneGeneratorConfig {
                        kind: LaneGeneratorKind::Melody(MelodyParams::default()),
                        seed: definition_id.wrapping_mul(0x517CC1B727220A95),
                    },
                );
            }
            LaneGeneratorKindTag::Pad => {
                def.lane_generators.insert(
                    track_id,
                    LaneGeneratorConfig {
                        kind: LaneGeneratorKind::Pad(PadParams::default()),
                        seed: definition_id.wrapping_mul(0x6C62272E07BB0142),
                    },
                );
            }
            LaneGeneratorKindTag::Vocal => {
                def.lane_generators.insert(
                    track_id,
                    LaneGeneratorConfig {
                        kind: LaneGeneratorKind::Vocal(VocalParams::default()),
                        seed: definition_id.wrapping_mul(0xBF58476D1CE4E5B9),
                    },
                );
            }
        }
        r.compose.last_error = None;
    }
}
