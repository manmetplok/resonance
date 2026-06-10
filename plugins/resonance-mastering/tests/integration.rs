//! End-to-end integration tests for the mastering plugin. Exercises the
//! full CLAP lifecycle: construct, initialize, run audio through the
//! chain, verify the stage behaves as advertised. Stage-level unit tests
//! live next to each DSP module in `src/stages/*`.

use resonance_mastering::assistant::analyze::AnalysisResult;
use resonance_mastering::assistant::{Genre, ReferenceTrack, Target};
use resonance_mastering::ResonanceMastering;
use resonance_plugin::{EventIterator, OutputBuffer, ResonancePlugin};

#[test]
fn param_enumeration_covers_declared_count() {
    let plugin = ResonanceMastering::new();
    assert_eq!(plugin.param_count(), resonance_mastering::PARAM_COUNT);
    let mut seen = std::collections::HashSet::new();
    for i in 0..plugin.param_count() {
        let id = plugin.param(i).id().to_string();
        assert!(seen.insert(id.clone()), "duplicate param id: {id}");
    }
}

#[test]
fn state_round_trips_through_save_load() {
    let plugin = ResonanceMastering::new();
    plugin.params().target_lufs.set_value(-11.5);
    plugin.params().input_trim_db.set_value(3.0);
    plugin.params().bypass.set_value(true);
    plugin.params().corrective_eq.bands[1].on.set_value(true);
    plugin.params().corrective_eq.bands[1].gain.set_value(-4.5);
    plugin.params().tonal_eq.bands[2].on.set_value(true);
    plugin.params().tonal_eq.bands[2].freq.set_value(3200.0);

    let saved = plugin.save_state();
    let mut other = ResonanceMastering::new();
    assert!(other.load_state(&saved));
    assert!(
        (plugin.params().target_lufs.value() - other.params().target_lufs.value()).abs() < 1e-3
    );
    assert!(
        (plugin.params().input_trim_db.value() - other.params().input_trim_db.value()).abs() < 1e-3
    );
    assert_eq!(
        plugin.params().bypass.value(),
        other.params().bypass.value()
    );
    assert_eq!(
        plugin.params().corrective_eq.bands[1].on.value(),
        other.params().corrective_eq.bands[1].on.value()
    );
    assert!(
        (plugin.params().corrective_eq.bands[1].gain.value()
            - other.params().corrective_eq.bands[1].gain.value())
        .abs()
            < 1e-3
    );
    assert_eq!(
        plugin.params().tonal_eq.bands[2].on.value(),
        other.params().tonal_eq.bands[2].on.value()
    );
    assert!(
        (plugin.params().tonal_eq.bands[2].freq.value()
            - other.params().tonal_eq.bands[2].freq.value())
        .abs()
            < 1e-3
    );
}

/// Stream a stereo sine through the plugin across many blocks and
/// return the concatenated output.
fn stream_sine(
    plugin: &mut ResonanceMastering,
    freq_hz: f32,
    amp: f32,
    total_samples: usize,
    block: usize,
) -> (Vec<f32>, Vec<f32>) {
    let sr = 48_000.0_f32;
    let step = freq_hz * std::f32::consts::TAU / sr;
    let mut phase = 0.0_f32;
    let mut out_l = Vec::with_capacity(total_samples);
    let mut out_r = Vec::with_capacity(total_samples);
    let mut done = 0;
    while done < total_samples {
        let n = (total_samples - done).min(block);
        let mut l = vec![0.0_f32; n];
        let mut r = vec![0.0_f32; n];
        for i in 0..n {
            let s = phase.sin() * amp;
            l[i] = s;
            r[i] = s;
            phase += step;
        }
        let mut outs = [OutputBuffer {
            left: &mut l,
            right: &mut r,
        }];
        let mut ev = EventIterator::empty();
        plugin.process(&mut outs, n, &mut ev, None);
        out_l.extend_from_slice(&l);
        out_r.extend_from_slice(&r);
        done += n;
    }
    (out_l, out_r)
}

#[test]
fn default_chain_is_pure_delay() {
    // With all EQ bands off the chain is a pure delay. Feed a sine,
    // compare output[latency..] to input[..−latency].
    let mut plugin = ResonanceMastering::new();
    plugin.initialize(48_000.0, 4096);
    let latency = plugin.latency_samples() as usize;
    let total = latency + 4096;

    let (out_l, out_r) = stream_sine(&mut plugin, 440.0, 0.5, total, 512);

    let sr = 48_000.0_f32;
    let step = 440.0 * std::f32::consts::TAU / sr;
    let input: Vec<f32> = (0..total).map(|i| (i as f32 * step).sin() * 0.5).collect();

    let mut max_err = 0.0_f32;
    for i in latency..total {
        max_err = max_err.max((out_l[i] - input[i - latency]).abs());
        max_err = max_err.max((out_r[i] - input[i - latency]).abs());
    }
    assert!(max_err < 5e-3, "pure-delay error = {max_err}");
}

#[test]
fn bypass_output_is_input_delayed_by_latency() {
    // With the plugin bypassed the output must still be delayed by
    // exactly `latency_samples()` so host delay compensation and A/B
    // comparisons stay aligned with the processed path.
    let mut plugin = ResonanceMastering::new();
    plugin.initialize(48_000.0, 4096);
    plugin.params().bypass.set_value(true);
    let latency = plugin.latency_samples() as usize;
    assert!(latency > 0, "chain should report nonzero latency");
    let total = latency + 4096;

    let (out_l, out_r) = stream_sine(&mut plugin, 440.0, 0.5, total, 512);

    // Regenerate the input exactly as `stream_sine` does (accumulated
    // phase) so the delayed copy can be compared bit-exactly.
    let sr = 48_000.0_f32;
    let step = 440.0 * std::f32::consts::TAU / sr;
    let mut phase = 0.0_f32;
    let input: Vec<f32> = (0..total)
        .map(|_| {
            let s = phase.sin() * 0.5;
            phase += step;
            s
        })
        .collect();

    // Pre-latency output must be the delay line's zero fill.
    for i in 0..latency {
        assert_eq!(out_l[i], 0.0, "expected silence at sample {i}");
        assert_eq!(out_r[i], 0.0, "expected silence at sample {i}");
    }
    // After that, an exact delayed copy of the input.
    let mut max_err = 0.0_f32;
    for i in latency..total {
        max_err = max_err.max((out_l[i] - input[i - latency]).abs());
        max_err = max_err.max((out_r[i] - input[i - latency]).abs());
    }
    assert_eq!(max_err, 0.0, "bypass should be a bit-exact delayed copy");
}

#[test]
fn bell_cut_propagates_through_chain() {
    // Enable a -12 dB bell band on the corrective EQ at 1 kHz.
    // A 1 kHz sine through the plugin should emerge ~4× quieter.
    let mut plugin = ResonanceMastering::new();
    plugin.initialize(48_000.0, 4096);
    plugin.params().corrective_eq.bands[1].on.set_value(true);
    plugin.params().corrective_eq.bands[1]
        .band_type
        .set_value(0);
    plugin.params().corrective_eq.bands[1]
        .freq
        .set_value(1000.0);
    plugin.params().corrective_eq.bands[1].q.set_value(1.0);
    plugin.params().corrective_eq.bands[1].gain.set_value(-12.0);

    let latency = plugin.latency_samples() as usize;
    let total = latency + 8192;
    let (out_l, _out_r) = stream_sine(&mut plugin, 1000.0, 0.5, total, 512);

    let tail_start = latency + 2048;
    let n = (total - tail_start) as f64;
    let sum_sq: f64 = out_l[tail_start..]
        .iter()
        .map(|&x| (x as f64).powi(2))
        .sum();
    let out_rms = (sum_sq / n).sqrt() as f32;
    let in_rms = 0.5_f32 / 2.0_f32.sqrt();
    let gain_db = 20.0 * (out_rms / in_rms).log10();
    assert!(
        (gain_db - -12.0).abs() < 2.0,
        "chain bell cut at 1 kHz = {gain_db} dB"
    );
}

#[test]
fn glue_compressor_reduces_loud_signal_through_chain() {
    let mut plugin = ResonanceMastering::new();
    plugin.initialize(48_000.0, 4096);
    plugin.params().glue_compressor.on.set_value(true);
    plugin.params().glue_compressor.threshold.set_value(-30.0);
    plugin.params().glue_compressor.ratio.set_value(8.0);
    plugin.params().glue_compressor.attack.set_value(1.0);
    plugin.params().glue_compressor.release.set_value(50.0);
    plugin.params().glue_compressor.knee.set_value(0.0);
    plugin.params().glue_compressor.makeup.set_value(0.0);
    plugin.params().glue_compressor.mix.set_value(1.0);

    let latency = plugin.latency_samples() as usize;
    let total = latency + 8192;
    let (out_l, _out_r) = stream_sine(&mut plugin, 1000.0, 0.8, total, 512);

    let tail_start = latency + 4096;
    let peak = out_l[tail_start..]
        .iter()
        .copied()
        .map(f32::abs)
        .fold(0.0_f32, f32::max);
    assert!(
        peak < 0.4,
        "compressed peak = {peak} (expected significantly below input 0.8)"
    );
}

#[test]
fn saturator_adds_harmonic_content_through_chain() {
    let mut plugin = ResonanceMastering::new();
    plugin.initialize(48_000.0, 4096);
    plugin.params().saturator.on.set_value(true);
    plugin.params().saturator.drive.set_value(12.0);
    plugin.params().saturator.character.set_value(0.0);
    plugin.params().saturator.mix.set_value(1.0);

    let sr = 48_000.0_f32;
    let f0 = 1000.0_f32;
    let latency = plugin.latency_samples() as usize;
    let total = latency + 4096;
    let (out_l, _out_r) = stream_sine(&mut plugin, f0, 0.7, total, 512);

    let tail_start = latency + 1024;
    let tail_len = total - tail_start;
    let mut h3 = 0.0_f32;
    for i in 0..tail_len {
        let t = i as f32 / sr;
        let basis = (std::f32::consts::TAU * 3.0 * f0 * t).sin();
        h3 += out_l[tail_start + i] * basis;
    }
    h3 = h3.abs() / tail_len as f32;
    assert!(h3 > 0.01, "third-harmonic energy = {h3}");
}

#[test]
fn multiband_low_band_compresses_only_low_content() {
    for (freq, expect_quiet) in [(50.0_f32, true), (5000.0_f32, false)] {
        let mut plugin = ResonanceMastering::new();
        plugin.initialize(48_000.0, 4096);
        plugin.params().multiband.on.set_value(true);
        plugin.params().multiband.bands[0].on.set_value(true);
        plugin.params().multiband.bands[0]
            .threshold
            .set_value(-30.0);
        plugin.params().multiband.bands[0].ratio.set_value(8.0);

        let latency = plugin.latency_samples() as usize;
        let total = latency + 8192;
        let (out_l, _out_r) = stream_sine(&mut plugin, freq, 0.5, total, 512);

        let tail_start = latency + 2048;
        let peak = out_l[tail_start..]
            .iter()
            .copied()
            .map(f32::abs)
            .fold(0.0_f32, f32::max);

        if expect_quiet {
            assert!(peak < 0.35, "band0 compressed 50 Hz peak = {peak}");
        } else {
            assert!(
                peak > 0.35,
                "5 kHz signal should not be compressed by band0: peak = {peak}"
            );
        }
    }
}

#[test]
fn limiter_enforces_ceiling_end_to_end() {
    let mut plugin = ResonanceMastering::new();
    plugin.initialize(48_000.0, 4096);
    plugin.params().limiter.on.set_value(true);
    plugin.params().limiter.ceiling.set_value(-6.0);
    plugin.params().limiter.release.set_value(50.0);

    let latency = plugin.latency_samples() as usize;
    let total = latency + 8192;
    let (out_l, out_r) = stream_sine(&mut plugin, 1000.0, 0.9, total, 512);

    let tail = latency + 2048;
    let ceiling_lin = 10.0_f32.powf(-6.0 / 20.0);
    let peak_l = out_l[tail..]
        .iter()
        .copied()
        .map(f32::abs)
        .fold(0.0_f32, f32::max);
    let peak_r = out_r[tail..]
        .iter()
        .copied()
        .map(f32::abs)
        .fold(0.0_f32, f32::max);
    let peak = peak_l.max(peak_r);
    assert!(
        peak <= ceiling_lin * 1.02,
        "end-to-end peak {peak} exceeds -6 dBTP ceiling {ceiling_lin}"
    );
}

#[test]
fn assistant_reference_target_drives_suggestions_label() {
    let mut plugin = ResonanceMastering::new();
    plugin.initialize(48_000.0, 4096);

    let amp = 10.0_f32.powf(-14.0 / 20.0);
    let _ = stream_sine(&mut plugin, 1000.0, amp, 3 * 48_000, 1024);

    let spectrum = vec![-12.0_f32; resonance_metering::NUM_OCTAVE_BINS];
    let fake_ref = ReferenceTrack {
        display_name: "mock_reference.wav".to_string(),
        sample_rate: 48_000.0,
        analysis: AnalysisResult {
            sample_rate: 48_000.0,
            duration_s: 10.0,
            integrated_lufs: -12.5,
            short_term_lufs: -12.5,
            true_peak_dbtp: -0.5,
            crest_db: 14.0,
            correlation: 0.9,
            spectrum_db: spectrum,
        },
    };
    plugin.viz().assistant.set_reference_for_testing(fake_ref);

    let suggestions = plugin
        .viz()
        .assistant
        .analyze(Target::Reference(
            plugin.viz().assistant.reference().unwrap(),
        ))
        .expect("reference-based analysis should succeed");

    assert_eq!(suggestions.target_label, "mock_reference.wav");
    assert!((suggestions.target_lufs - -12.5).abs() < 1e-3);
}

#[test]
fn assistant_analyze_and_apply_writes_params() {
    let mut plugin = ResonanceMastering::new();
    plugin.initialize(48_000.0, 4096);

    let amp = 10.0_f32.powf(-14.0 / 20.0);
    let total = 3 * 48_000_usize;
    let _ = stream_sine(&mut plugin, 1000.0, amp, total, 1024);

    let suggestions = plugin
        .viz()
        .assistant
        .analyze(Target::Genre(Genre::Rock))
        .expect("assistant should return suggestions with enough audio");

    suggestions.apply_to(plugin.params());
    assert_eq!(
        plugin.params().target_lufs.value(),
        Genre::Rock.target_lufs()
    );
    assert!(plugin.params().limiter.on.value());
    assert!(plugin.params().limiter.ceiling.value() <= 0.0);
}

#[test]
fn process_updates_viz_snapshot() {
    let mut plugin = ResonanceMastering::new();
    plugin.initialize(48_000.0, 4096);

    let amp = 10.0_f32.powf(-23.0 / 20.0);
    let total = (8.0 * 48_000.0) as usize;
    let _ = stream_sine(&mut plugin, 1000.0, amp, total, 1024);

    let snap = plugin.viz().load_snapshot();
    assert!(
        (snap.integrated_lufs - -23.0).abs() < 0.3,
        "integrated {} LUFS",
        snap.integrated_lufs
    );
}
