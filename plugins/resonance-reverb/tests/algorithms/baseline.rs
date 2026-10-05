//! R0 baseline: the `resonance_metering::decay` harness run on Classic
//! (reverb-algorithms.md §5.1), the numbers the new engines must beat on
//! L1–L4.
//!
//! Each scenario renders the plugin's impulse response through
//! `ResonancePlugin::process` (the path `tests/dsp_golden.rs` uses): a
//! fresh instance, every parameter that matters pinned, 100 % wet, no
//! pre-delay, and a unit impulse on both channels at sample 0 (a mono
//! send). The assertions are only the facts any working reverb must hold:
//! finite, not silent, decaying. The figures themselves are printed, not
//! asserted — Classic is the baseline, not a target:
//!
//!     cargo test -p resonance-reverb --test algorithms baseline -- --nocapture

use resonance_metering::decay::ImpulseReport;
use resonance_plugin::{EventIterator, OutputBuffer, ResonancePlugin};
use resonance_reverb::params::ReverbParams;
use resonance_reverb::ResonanceReverb;

use crate::common::{energy_db, BLOCK, SR};

#[derive(Clone, Copy, Debug)]
struct Setting {
    size: f32,
    decay: f32,
    damping: f32,
}

/// Decay 0.5 / 2 / 8 s at sizes 0.2 / 0.5 / 0.9 with the default 8 kHz
/// damping, then the three decays at size 0.5 with damping wide open
/// (20 kHz), which leaves L1 (the loop-gain approximation) as the only
/// source of decay error.
fn settings() -> Vec<Setting> {
    let mut v = Vec::new();
    for decay in [0.5, 2.0, 8.0] {
        for size in [0.2, 0.5, 0.9] {
            v.push(Setting {
                size,
                decay,
                damping: 8_000.0,
            });
        }
    }
    for decay in [0.5, 2.0, 8.0] {
        v.push(Setting {
            size: 0.5,
            decay,
            damping: 20_000.0,
        });
    }
    v
}

fn pin(p: &ReverbParams, s: Setting) {
    p.size.set_value(s.size);
    p.decay.set_value(s.decay);
    p.damping.set_value(s.damping);
    p.predelay.set_value(0.0);
    p.mix.set_value(1.0);
    p.width.set_value(1.0);
    p.diffusion.set_value(0.8);
    p.er_level.set_value(0.4);
    p.er_time.set_value(0.5);
    p.mod_rate.set_value(1.0);
    p.mod_depth.set_value(0.3);
    p.freeze.set_value(false);
}

/// Long enough for a T30 fit even if Classic runs ~50 % long, and for the
/// late-tail metrics on the shortest decay.
fn render_seconds(s: Setting) -> f32 {
    (1.6 * s.decay + 0.5).max(2.5)
}

fn render(s: Setting) -> (Vec<f32>, Vec<f32>) {
    let mut plugin = ResonanceReverb::new();
    pin(&plugin.params, s);
    plugin.initialize(SR, BLOCK as u32);

    let total = (render_seconds(s) * SR) as usize;
    let (mut out_l, mut out_r) = (Vec::with_capacity(total), Vec::with_capacity(total));
    let mut left = vec![0.0f32; BLOCK];
    let mut right = vec![0.0f32; BLOCK];
    let mut n = 0;
    while n < total {
        let frames = BLOCK.min(total - n);
        left.fill(0.0);
        right.fill(0.0);
        if n == 0 {
            left[0] = 1.0;
            right[0] = 1.0;
        }
        {
            let mut outs = [OutputBuffer {
                left: &mut left[..frames],
                right: &mut right[..frames],
            }];
            plugin.process(&mut outs, frames, &mut EventIterator::empty(), None);
        }
        out_l.extend_from_slice(&left[..frames]);
        out_r.extend_from_slice(&right[..frames]);
        n += frames;
    }
    (out_l, out_r)
}

#[test]
fn classic_impulse_metrics_baseline() {
    println!(
        "\nClassic baseline (48 kHz, impulse, 100 % wet, default diffusion/ER/mod)\n\
         size  decay  damp   midT30  err%   {}",
        ImpulseReport::table_header()
    );
    for s in settings() {
        let (l, r) = render(s);
        let rep = ImpulseReport::analyze(&l, &r, SR);
        let mid = rep.mid_t30();
        let err = mid.map(|t| 100.0 * (t - s.decay) / s.decay);
        println!(
            "{:>4.1} {:>5.1}s {:>5.0} {:>7}s {:>6}  {rep}",
            s.size,
            s.decay,
            s.damping,
            mid.map_or("-".into(), |t| format!("{t:.3}")),
            err.map_or("-".into(), |e| format!("{e:+.1}")),
        );

        // The facts, not the targets.
        assert!(rep.finite, "{s:?}: non-finite output");
        assert!(rep.peak > 1e-3, "{s:?}: silent (peak {:.2e})", rep.peak);
        assert!(
            rep.rms_2s_dbfs > -60.0,
            "{s:?}: first 2 s at {:.1} dBFS (silence guard)",
            rep.rms_2s_dbfs
        );
        let tenth = l.len() / 10;
        let head = energy_db(&l[..tenth], &r[..tenth]);
        let tail = energy_db(&l[l.len() - tenth..], &r[r.len() - tenth..]);
        assert!(
            head - tail > 20.0,
            "{s:?}: does not decay (first tenth {head:.1} dB, last {tail:.1} dB)"
        );
        assert!(rep.broadband.t30.is_some(), "{s:?}: no broadband T30");
        assert!(rep.late_iacc.is_some() && rep.mono_fold_db.is_some());
    }
}
