//! **Chamber** (R4, reverb-algorithms.md §4.4): a Room voicing — the
//! 1960s echo chamber, warm and dense, kind to vocals.
//!
//! Same engine as Room ([`super::room::RoomCore`]); the structural
//! differences are fixed here, the warmth (more bass decay, a lower
//! treble crossover, no modulation) is carried by the presets, because
//! parameter defaults are global:
//!
//! - **Smaller, harder box**: 4 × 5.5 × 3 m up to 9 × 13 × 5.5 m, walls
//!   absorbing only 15 %, so the second-order images arrive nearly as
//!   strong as the first; they are weighted up a further 1.6× (denser ER),
//!   and scattered harder (1–4 ms allpasses at 0.7 against Room's 0.65).
//! - **Slightly longer lines**: 10–90 ms at size scale 1, scaled
//!   0.6×–1.3× with `size` (6–54 ms up to 13–117 ms).
//! - **Little modulation**: at most 3 samples at `mod_depth` 1 (Room: 8).
//!   The factory chamber (`Vocal Chamber`) runs none.
//! - **Denser diffuser**: six 0.4–3.3 ms allpasses, coefficient up to
//!   0.75.
//! - **Levels**: ER 0.28, tail 0.7 (wet energy at the defaults matches
//!   Classic's).

use super::room::{room_family_engine, EarlyVoicing, Voicing};

/// Chamber's voicing (see the module docs).
pub(in crate::dsp::algo) const CHAMBER: Voicing = Voicing {
    early: EarlyVoicing {
        small: [4.0, 5.5, 3.0],
        large: [9.0, 13.0, 5.5],
        source: [0.36, 0.66, 0.47],
        listener: [0.58, 0.29, 0.43],
        absorption: 0.15,
        second_order_gain: 1.6,
        scatter_ms: [[1.3, 2.7, 4.1], [1.1, 2.9, 4.3]],
        scatter_gain: 0.7,
        level: 0.28,
        envelope_scale: 1.0,
    },
    line_ms: (10.0, 90.0),
    size_scale: (0.6, 1.3),
    diffuser_ms: [
        [0.37, 0.61, 0.97, 1.31, 2.17, 3.29],
        [0.41, 0.67, 0.89, 1.23, 2.39, 3.07],
    ],
    diffuser_gain: 0.75,
    diffused_early: false,
    mod_depth_max: 3.0,
    decay_range: (0.1, 30.0),
    direct_diffuse: 0.3,
    late_level: 0.7,
    late_decay_norm: 0.0,
    seed: 0x0043_4841_4D42_4552,
};

room_family_engine!(ChamberEngine, 16, CHAMBER);
