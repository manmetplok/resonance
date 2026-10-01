//! Pure helpers for picking velocity layers and round-robin takes.
//!
//! Kept out of `DrumSampler` so they're unit-testable without
//! constructing a full sampler (which spawns a janitor thread).

/// Maximum velocity layer count we track per pad in the round-robin counter
/// array. Drummica's deepest pad has 28 layers, so 32 is a comfortable cap.
pub const MAX_LAYERS: usize = 32;

/// Map a MIDI velocity in [0, 1] onto a layer index in [0, n_layers).
///
/// Uses equal-width buckets. Callers must guarantee `n_layers >= 1`; with
/// `n_layers == 1` the result is always 0.
pub fn pick_velocity_layer(velocity: f32, n_layers: usize) -> usize {
    debug_assert!(n_layers >= 1, "n_layers must be at least 1");
    if n_layers <= 1 {
        return 0;
    }
    ((velocity.clamp(0.0, 1.0) * n_layers as f32) as usize).min(n_layers - 1)
}

/// Levels closer together than this (dB, loudest minus softest usable
/// layer) say nothing about loudness: the layers are picked by
/// [`pick_velocity_layer`]'s equal buckets at unity, as before E7.
pub const MIN_LAYER_SPREAD_DB: f32 = 1.0;

/// A layer measured quieter than this (dB) is no strike — a silent or
/// broken take — and is never picked by loudness.
pub const UNUSABLE_LAYER_DB: f32 = -100.0;

/// Pick a velocity layer by **measured loudness** (E7), and the gain
/// that puts the hit exactly where the velocity asks for.
///
/// `level(i)` is layer `i`'s measured level in dB (see
/// `VelocityLayer::level_db`); `n_layers >= 1`. The velocity (0..1,
/// after the global curve) asks for a target level on a straight dB line
/// from the softest usable layer (velocity 0) to the loudest (velocity
/// 1). The layer whose level is nearest the target plays, at the gain
/// that makes up the difference — so the output level follows the
/// velocity continuously: crossing from one layer to the next swaps the
/// recording, not the loudness. The softest and loudest hits play their
/// layers at unity, and no layer is ever moved by more than half the gap
/// to its neighbour. Layers out of order (a soft layer recorded louder
/// than the next) are simply picked where their level says.
///
/// When the levels are too close to tell apart ([`MIN_LAYER_SPREAD_DB`],
/// or no layer is usable) the layers are picked as before E7: equal
/// velocity buckets, at unity.
///
/// No allocation, no lock: runs in `note_on` on the audio thread.
pub fn pick_layer_by_level(
    velocity: f32,
    n_layers: usize,
    level: impl Fn(usize) -> f32,
) -> (usize, f32) {
    let mut lo = f32::INFINITY;
    let mut hi = f32::NEG_INFINITY;
    for i in 0..n_layers {
        let l = level(i);
        if l > UNUSABLE_LAYER_DB {
            lo = lo.min(l);
            hi = hi.max(l);
        }
    }
    let spread = hi - lo;
    if !spread.is_finite() || spread < MIN_LAYER_SPREAD_DB {
        return (pick_velocity_layer(velocity, n_layers), 1.0);
    }
    let target = lo + (hi - lo) * velocity.clamp(0.0, 1.0);
    let mut best = 0;
    let mut best_dist = f32::INFINITY;
    for i in 0..n_layers {
        let l = level(i);
        if l <= UNUSABLE_LAYER_DB {
            continue;
        }
        let dist = (l - target).abs();
        if dist < best_dist {
            best = i;
            best_dist = dist;
        }
    }
    let gain_db = target - level(best);
    (best, (gain_db * (std::f32::consts::LN_10 / 20.0)).exp())
}

/// Map index `index` of `n_from` onto `n_to` by relative position: the
/// result is the one of `n_to` equal buckets that holds the centre of
/// bucket `index` of `n_from`. With equal counts it is the identity, so banks that
/// share a shape play exactly the cell the reference bank picked; a bank
/// with fewer layers (or takes) plays its nearest one instead of none
/// (drums-plugin-rework.md §7 E7). Integer arithmetic only — this runs
/// in `note_on` on the audio thread. `n_to == 0` gives 0.
pub fn map_relative(index: usize, n_from: usize, n_to: usize) -> usize {
    if n_to == n_from || n_from == 0 {
        return index.min(n_to.saturating_sub(1));
    }
    ((2 * index + 1) * n_to / (2 * n_from)).min(n_to.saturating_sub(1))
}

/// Advance a round-robin counter and return the RR index for this trigger.
/// Wraps the counter at `u32::MAX` so it can run indefinitely.
pub fn pick_rr(counter: &mut u32, n_rrs: usize) -> usize {
    debug_assert!(n_rrs >= 1, "n_rrs must be at least 1");
    let idx = (*counter as usize) % n_rrs;
    *counter = counter.wrapping_add(1);
    idx
}

/// How the sampler picks which recorded take of a layer fires.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RoundRobinMode {
    /// Walk the takes in order. The default, and what the plugin always
    /// did before the control was wired up (ba todo #1326).
    #[default]
    Cycle,
    /// Pick at random, never the take that just played.
    Random,
}

impl RoundRobinMode {
    /// Read the mode off its parameter value.
    pub fn from_param(value: i32) -> Self {
        if value == 1 {
            Self::Random
        } else {
            Self::Cycle
        }
    }
}

/// Marker for "this layer has not fired yet", stored in the per-layer
/// last-take table.
pub const NO_LAST_TAKE: u16 = u16::MAX;

/// Advance a 32-bit xorshift state and return the next value. Chosen
/// over a real RNG because it is three shifts and an xor: the audio
/// thread runs this inside `note_on`, and it must not allocate, lock or
/// call into the OS.
pub fn next_random(state: &mut u32) -> u32 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *state = x;
    x
}

/// Pick a random take, excluding `last` when there is more than one to
/// choose from.
///
/// Uniform sampling would repeat the previous take about 1/n of the
/// time, and an audible repeat is the one thing round robin exists to
/// prevent — so the draw is uniform over the *other* takes. With two
/// takes that degenerates to alternating, which is the best "random"
/// two takes can do.
pub fn pick_rr_random(state: &mut u32, last: u16, n_rrs: usize) -> usize {
    debug_assert!(n_rrs >= 1, "n_rrs must be at least 1");
    if n_rrs <= 1 {
        return 0;
    }
    let last = last as usize;
    if last < n_rrs {
        let pick = (next_random(state) as usize) % (n_rrs - 1);
        if pick >= last {
            pick + 1
        } else {
            pick
        }
    } else {
        (next_random(state) as usize) % n_rrs
    }
}
