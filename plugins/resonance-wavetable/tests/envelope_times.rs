//! DSP2-12: envelope times mean what their labels say.
//!
//! - Attack: time from 0 to the peak.
//! - Decay: time from the peak to the sustain level.
//! - Release: time from full scale to −60 dB.
//!
//! The curve knob changes the *shape* of each stage, not its length.
//! Before the fix the times were one-pole time constants: "Attack 100 ms"
//! peaked at 147 ms and "Release 10 s" held a voice for ~68 s. Sustain is
//! also smoothed, so automating it no longer steps the level.

use resonance_wavetable::dsp::envelope::{AdsrEnvelope, EnvCoeffs, EnvStage};

const SR: f32 = 48_000.0;

fn coeffs(a: f32, d: f32, s: f32, r: f32, curve: f32) -> EnvCoeffs {
    EnvCoeffs::for_params(a, d, s, r, curve, SR)
}

fn env() -> AdsrEnvelope {
    let mut e = AdsrEnvelope::new();
    e.set_sample_rate(SR);
    e
}

/// Seconds until `done` first holds, ticking `env` with `c`.
fn time_until(env: &mut AdsrEnvelope, c: &EnvCoeffs, done: impl Fn(&AdsrEnvelope) -> bool) -> f32 {
    for n in 0..(SR as usize * 60) {
        if done(env) {
            return n as f32 / SR;
        }
        env.next(c);
    }
    f32::INFINITY
}

fn assert_close(what: &str, got: f32, want: f32) {
    assert!(
        (got - want).abs() <= want * 0.02 + 2.0 / SR,
        "{what}: {:.1} ms, label {:.1} ms",
        got * 1000.0,
        want * 1000.0
    );
}

#[test]
fn attack_reaches_the_peak_at_the_labelled_time() {
    for curve in [-1.0, -0.5, 0.0, 0.5, 1.0] {
        for t in [0.01, 0.1, 1.0] {
            let c = coeffs(t, 0.3, 0.5, 0.3, curve);
            let mut e = env();
            e.trigger();
            let got = time_until(&mut e, &c, |e| e.stage != EnvStage::Attack);
            assert_close(&format!("attack {t} s, curve {curve}"), got, t);
        }
    }
}

#[test]
fn decay_reaches_sustain_at_the_labelled_time() {
    for curve in [-1.0, 0.0, 1.0] {
        for sustain in [0.0, 0.5, 0.8] {
            let t = 0.4;
            let c = coeffs(0.001, t, sustain, 0.3, curve);
            let mut e = env();
            e.trigger();
            time_until(&mut e, &c, |e| e.stage == EnvStage::Decay);
            let got = time_until(&mut e, &c, |e| e.stage == EnvStage::Sustain);
            assert_close(&format!("decay to {sustain}, curve {curve}"), got, t);
        }
    }
}

#[test]
fn release_falls_60_db_at_the_labelled_time_and_frees_the_voice() {
    for curve in [-1.0, 0.0, 1.0] {
        for t in [0.3, 10.0] {
            let c = coeffs(0.001, 0.001, 1.0, t, curve);
            let mut e = env();
            e.trigger();
            time_until(&mut e, &c, |e| e.stage == EnvStage::Sustain);
            e.release();
            let got = time_until(&mut e, &c, |e| e.level <= 0.001);
            assert_close(&format!("release {t} s, curve {curve}"), got, t);
            let idle = got + time_until(&mut e, &c, |e| e.is_idle());
            assert!(
                idle < 1.3 * t,
                "release {t} s, curve {curve}: voice held for {idle:.2} s"
            );
        }
    }
}

#[test]
fn sustain_changes_glide_instead_of_stepping() {
    let mut e = env();
    let c = coeffs(0.001, 0.01, 0.8, 0.3, 0.0);
    e.trigger();
    time_until(&mut e, &c, |e| e.stage == EnvStage::Sustain);
    let low = coeffs(0.001, 0.01, 0.2, 0.3, 0.0);
    let mut prev = e.level;
    let mut max_step = 0.0f32;
    for _ in 0..(SR as usize / 10) {
        let v = e.next(&low);
        max_step = max_step.max((v - prev).abs());
        prev = v;
    }
    assert!((prev - 0.2).abs() < 1e-3, "sustain did not arrive: {prev}");
    assert!(max_step < 0.01, "sustain 0.8 -> 0.2 stepped {max_step:.3} in one sample");
}
