//! **Ambience** (spec phase R6, built with Room): a dense cluster of
//! early reflections and a very short 8-line FDN — "space without
//! reverb", to sit at a few percent on a mix bus (reverb-algorithms.md
//! §4.4).
//!
//! Same engine as Room ([`super::room::RoomCore`]) with 8 lines
//! (Householder feedback). Its EDT, not its T60, carries the space: the
//! reflections are loud and the tail is quiet, so the first 10 dB fall
//! inside the cluster (EDT 0.10–0.15 s against a T30 of 0.35–0.8 s over
//! decay 0.5–1 s) and what is left decays below hearing in a mix.
//!
//! - **Small box**: 2.2 × 2.8 × 2.2 m up to 6 × 8 × 3.5 m, so the
//!   first- and second-order images land in 0–40 ms; second order
//!   weighted 1.3×.
//! - **A dense cluster**: the reflections are fed from the *diffused*
//!   input, so every image is a short dense burst rather than a click
//!   (echo density 0.9 by 12–15 ms), and their envelope decays at
//!   0.12 × `decay`: the cluster is over well before the tail, whatever
//!   the knob.
//! - **A quiet tail**: lines 3–25 ms (scaled 0.6×–1.4× with `size`), the
//!   tail at 0.11 with the diffused input added straight in at 2× that
//!   (the onset under the cluster), and its level normalised by
//!   `(1 s / T60)^0.25`, so a longer decay lengthens the tail more than
//!   it raises it.
//! - **No modulation** (`mod_rate`/`mod_depth` are accepted and do
//!   nothing).
//! - **Decay clamped to 0.1–1.0 s.**

use super::room::{room_family_engine, EarlyVoicing, Voicing};

/// Ambience's voicing (see the module docs).
pub(in crate::dsp::algo) const AMBIENCE: Voicing = Voicing {
    early: EarlyVoicing {
        small: [2.2, 2.8, 2.2],
        large: [6.0, 8.0, 3.5],
        source: [0.37, 0.65, 0.45],
        listener: [0.59, 0.32, 0.42],
        absorption: 0.35,
        second_order_gain: 1.3,
        scatter_ms: [[0.23, 0.59, 1.07], [0.29, 0.53, 1.19]],
        scatter_gain: 0.4,
        level: 0.55,
        envelope_scale: 0.12,
    },
    line_ms: (3.0, 25.0),
    size_scale: (0.6, 1.4),
    diffuser_ms: [
        [0.23, 0.43, 0.71, 0.97, 1.61, 2.27],
        [0.27, 0.47, 0.67, 0.89, 1.73, 2.13],
    ],
    diffuser_gain: 0.55,
    diffused_early: true,
    mod_depth_max: 0.0,
    decay_range: (0.1, 1.0),
    direct_diffuse: 2.0,
    late_level: 0.11,
    late_decay_norm: 0.25,
    seed: 0x414D_4249_454E_4345,
};

room_family_engine!(AmbienceEngine, 8, AMBIENCE);
