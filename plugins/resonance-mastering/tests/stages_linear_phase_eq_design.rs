use resonance_mastering::stages::linear_phase_eq::band::{BandConfig, BandType};
use resonance_mastering::stages::linear_phase_eq::design::FirDesigner;
use resonance_mastering::stages::linear_phase_eq::FIR_LENGTH;

#[test]
fn no_enabled_bands_produces_impulse_like_fir() {
    // When no bands are enabled, the chain is flat (magnitude = 1)
    // and the IFFT should give a delta function at the FIR centre.
    let mut d = FirDesigner::new();
    let bands: [BandConfig; 0] = [];
    let h = d.design(&bands, 48_000.0);
    let centre = FIR_LENGTH / 2;
    assert!((h[centre] - 1.0).abs() < 1e-2, "h[center] = {}", h[centre]);
    for (i, &v) in h.iter().enumerate() {
        if i != centre {
            assert!(v.abs() < 0.05, "h[{i}] = {v}");
        }
    }
}

#[test]
fn bell_boost_is_symmetric() {
    // A bell band produces a symmetric FIR (linear-phase property).
    // h[centre + k] ≈ h[centre - k] for all k.
    let mut d = FirDesigner::new();
    let bands = [BandConfig {
        enabled: true,
        band_type: BandType::Bell,
        freq_hz: 1000.0,
        q: 1.0,
        gain_db: 6.0,
    }];
    let h = d.design(&bands, 48_000.0);
    let centre = FIR_LENGTH / 2;
    for k in 1..200 {
        let err = (h[centre + k] - h[centre - k]).abs();
        assert!(err < 1e-5, "asymmetry at ±{k}: {err}");
    }
}
