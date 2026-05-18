//! Late-tail Feedback Delay Network: 8-channel delay bank with
//! Householder mixing matrix, per-channel one-pole damping and decay
//! gain. The recirculating feedback never passes through the diffusion
//! network — that would multiply the Hadamard's non-unit broadband gain
//! into every cycle and crush the requested RT60.

use super::CHANNELS;

/// Maximum FDN channel delay multiplier at the longest channel.
/// The classic `2^(c/(CHANNELS-1))` spread with `c = CHANNELS-1 = 7`
/// gives ~2.0 — a narrow range (1×..2×) that produces dense
/// feedback reflection pile-up instead of audibly separated echoes.
pub(super) const MAX_FDN_MULT: f32 = 2.0;

/// In-place Householder reflection: y[i] = x[i] - (2/N) * sum(x).
pub(super) fn householder_in_place(data: &mut [f32; CHANNELS]) {
    let sum: f32 = data.iter().sum();
    let factor = -2.0 / CHANNELS as f32 * sum;
    for x in data.iter_mut() {
        *x += factor;
    }
}
