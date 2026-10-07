//! Per-frame energy curve from note velocity
//! (vocal-mcp-control-and-quality.md §9): on a bank whose acoustic model
//! takes `energy` (`curve_supported(Dynamics)`), velocity drives it,
//! scaled by the lane's `velocity_dynamics`. A bank without energy gets
//! velocity through the tension curve instead ([`super::tension`]); every
//! bank also gets the post-vocoder gain ([`super::super::dynamics`]).
//!
//! Units follow the expression overlay's convention for the signed
//! per-frame inputs ([`super::overlay`]): `-1..=1`, 0 neutral. No shipped
//! bank takes energy, so this path has not been heard against a model.

use resonance_music_theory::VocalParams;
use resonance_svs::ds::SampleCurve;

use super::super::dynamics::velocity_modulation;
use super::super::paths::CurveKind;
use super::super::registry::BankProfile;
use super::f0::{F0Curve, F0_TIMESTEP};

/// The energy `SampleCurve`, or `SampleCurve::default()` (not sent) when
/// the bank has no energy input or `velocity_dynamics` is 0.
pub(super) fn build_energy_curve(
    curve: &F0Curve,
    params: &VocalParams,
    bank: &BankProfile,
) -> SampleCurve {
    let amount = params.velocity_dynamics.clamp(0.0, 1.0) as f64;
    if amount <= 0.0 || !bank.curve_supported(CurveKind::Dynamics) {
        return SampleCurve::default();
    }
    let samples = curve
        .frame_velocity
        .iter()
        .map(|&vel| {
            if vel > 0.0 {
                amount * velocity_modulation(vel) as f64
            } else {
                0.0
            }
        })
        .collect();
    SampleCurve {
        samples,
        timestep: F0_TIMESTEP,
    }
}
