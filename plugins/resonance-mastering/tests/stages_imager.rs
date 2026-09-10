use resonance_mastering::stages::imager::{Imager, ImagerConfig};

#[test]
fn disabled_passes_audio_unchanged() {
    let mut im = Imager::new(48_000.0);
    let mut l = vec![0.3, -0.4, 0.5, -0.6];
    let mut r = vec![0.2, -0.3, 0.4, -0.5];
    let el = l.clone();
    let er = r.clone();
    im.process_stereo(&mut l, &mut r, &ImagerConfig::default());
    assert_eq!(l, el);
    assert_eq!(r, er);
}

#[test]
fn width_one_is_identity() {
    let mut im = Imager::new(48_000.0);
    let mut l = vec![0.3_f32, -0.4, 0.5, -0.6];
    let mut r = vec![0.2_f32, -0.3, 0.4, -0.5];
    let el = l.clone();
    let er = r.clone();
    im.process_stereo(
        &mut l,
        &mut r,
        &ImagerConfig {
            enabled: true,
            width: 1.0,
            side_hpf_on: false,
            side_hpf_hz: 120.0,
        },
    );
    for (a, b) in l.iter().zip(el.iter()) {
        assert!((a - b).abs() < 1e-6);
    }
    for (a, b) in r.iter().zip(er.iter()) {
        assert!((a - b).abs() < 1e-6);
    }
}

#[test]
fn width_zero_collapses_to_mono() {
    let mut im = Imager::new(48_000.0);
    // L and R start different but should both become 0.5*(L+R).
    let mut l = vec![0.4_f32, -0.6, 0.8, 0.0];
    let mut r = vec![0.0_f32, 0.0, -0.2, 0.4];
    let expected_mono: Vec<f32> = l.iter().zip(r.iter()).map(|(a, b)| 0.5 * (a + b)).collect();
    im.process_stereo(
        &mut l,
        &mut r,
        &ImagerConfig {
            enabled: true,
            width: 0.0,
            side_hpf_on: false,
            side_hpf_hz: 120.0,
        },
    );
    for (i, (a, b)) in l.iter().zip(expected_mono.iter()).enumerate() {
        assert!((a - b).abs() < 1e-6, "left[{i}] {a} vs {b}");
    }
    for (i, (a, b)) in r.iter().zip(expected_mono.iter()).enumerate() {
        assert!((a - b).abs() < 1e-6, "right[{i}] {a} vs {b}");
    }
}

#[test]
fn side_hpf_removes_low_frequencies_from_side_channel() {
    // Build a 50 Hz anti-phase signal (pure side content).
    // After side HPF at 200 Hz the side should be heavily
    // attenuated, so L and R converge to the mono sum (= 0).
    let sr = 48_000.0_f32;
    let mut im = Imager::new(sr);
    let n = 4096;
    let mut l = vec![0.0_f32; n];
    let mut r = vec![0.0_f32; n];
    for i in 0..n {
        let s = (i as f32 / sr * 50.0 * std::f32::consts::TAU).sin() * 0.5;
        l[i] = s;
        r[i] = -s;
    }
    im.process_stereo(
        &mut l,
        &mut r,
        &ImagerConfig {
            enabled: true,
            width: 1.0,
            side_hpf_on: true,
            side_hpf_hz: 200.0,
        },
    );
    // Look at the settled tail.
    let tail = &l[n / 2..];
    let peak = tail.iter().copied().map(f32::abs).fold(0.0_f32, f32::max);
    assert!(peak < 0.05, "low-freq side peak = {peak}");
}

/// Anti-phase stereo pair — pure side content, the imager's worst case
/// for toggle/width steps because the width gain applies to the whole
/// signal.
fn side_only_input(sr: f32, freq: f32, amp: f32, n: usize) -> (Vec<f32>, Vec<f32>) {
    let mut l = vec![0.0_f32; n];
    let mut r = vec![0.0_f32; n];
    for i in 0..n {
        let s = (i as f32 / sr * freq * std::f32::consts::TAU).sin() * amp;
        l[i] = s;
        r[i] = -s;
    }
    (l, r)
}

/// Drive `im` over `input` in `block`-frame chunks, choosing the config
/// per block, and return the rendered left channel.
fn render_blocks(
    im: &mut Imager,
    input: &(Vec<f32>, Vec<f32>),
    block: usize,
    cfg_for_block: impl Fn(usize) -> ImagerConfig,
) -> Vec<f32> {
    let total = input.0.len();
    let mut l = input.0.clone();
    let mut r = input.1.clone();
    let mut start = 0;
    while start < total {
        let end = (start + block).min(total);
        let cfg = cfg_for_block(start);
        im.process_stereo(&mut l[start..end], &mut r[start..end], &cfg);
        start = end;
    }
    l
}

fn max_delta(x: &[f32], from: usize, to: usize) -> f32 {
    x[from..to]
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0_f32, f32::max)
}

#[test]
fn toggle_mid_signal_does_not_click() {
    // Width 2.0 on side-only material doubles the signal; toggling the
    // stage off used to halve it in a single sample. The enable
    // crossfade must keep the slope in the same class as a steady run.
    let sr = 48_000.0_f32;
    let cfg_on = ImagerConfig {
        enabled: true,
        width: 2.0,
        side_hpf_on: false,
        side_hpf_hz: 120.0,
    };
    let cfg_off = ImagerConfig {
        enabled: false,
        ..cfg_on
    };

    // 60 Hz → 800-sample period; the toggle boundary at sample 9800
    // lands exactly on a peak, the worst case for the level step.
    let block = 100;
    let toggle = 9800;
    let input = side_only_input(sr, 60.0, 0.8, 14_000);

    let steady = render_blocks(&mut Imager::new(sr), &input, block, |_| cfg_on);
    let toggled = render_blocks(&mut Imager::new(sr), &input, block, |start| {
        if start >= toggle {
            cfg_off
        } else {
            cfg_on
        }
    });

    let steady_max = max_delta(&steady, toggle - 400, toggle + 1600);
    let toggled_max = max_delta(&toggled, toggle - 400, toggle + 1600);
    // The old hard toggle stepped ~0.8 here (2.0× → 1.0× on a 0.8
    // peak); the crossfade keeps it near the sine's own slope.
    assert!(
        toggled_max < steady_max * 1.5 + 0.01,
        "toggle stepped {toggled_max} per sample vs {steady_max} steady"
    );
}

#[test]
fn width_step_in_one_block_does_not_click() {
    // Width 0 collapses side-only material to silence; stepping the
    // param 0 → 1 between two blocks used to jump the output from
    // zero to the full signal in one sample.
    let sr = 48_000.0_f32;
    let narrow = ImagerConfig {
        enabled: true,
        width: 0.0,
        side_hpf_on: false,
        side_hpf_hz: 120.0,
    };
    let wide = ImagerConfig {
        width: 1.0,
        ..narrow
    };

    let block = 100;
    let step = 9800; // On a 60 Hz peak, as above.
    let input = side_only_input(sr, 60.0, 0.8, 14_000);

    let steady = render_blocks(&mut Imager::new(sr), &input, block, |_| wide);
    let stepped = render_blocks(&mut Imager::new(sr), &input, block, |start| {
        if start >= step {
            wide
        } else {
            narrow
        }
    });

    let steady_max = max_delta(&steady, step - 400, step + 1600);
    let stepped_max = max_delta(&stepped, step - 400, step + 1600);
    // Old behavior jumped ~0.8 in one sample; the width ramp spreads
    // the step over ~10 ms, i.e. the same order as the sine's slope.
    assert!(
        stepped_max < steady_max * 2.0 + 0.01,
        "width step moved {stepped_max} per sample vs {steady_max} steady"
    );
}

#[test]
fn cutoff_moved_while_disabled_applies_cleanly_on_reenable() {
    // Era 1: enabled with a 120 Hz side HPF (coefficients cached,
    // biquad state populated). Era 2: disabled, cutoff moved to
    // 500 Hz. Era 3: re-enabled. The rendered era 3 must match an
    // imager that never saw era 1 at all — the new cutoff in force
    // immediately, and no transient replayed out of stale biquad
    // state.
    let sr = 48_000.0_f32;
    let block = 128;
    let era = 4800; // ≈ 0.1 s per era
    let total = 4 * era;
    let cfg_120 = ImagerConfig {
        enabled: true,
        width: 1.0,
        side_hpf_on: true,
        side_hpf_hz: 120.0,
    };
    let cfg_off_500 = ImagerConfig {
        enabled: false,
        side_hpf_hz: 500.0,
        ..cfg_120
    };
    let cfg_on_500 = ImagerConfig {
        enabled: true,
        ..cfg_off_500
    };

    // Side content at 100 Hz (well below the 500 Hz cutoff) plus
    // 1 kHz (above it).
    let total_input: (Vec<f32>, Vec<f32>) = {
        let (mut l, mut r) = side_only_input(sr, 100.0, 0.5, total);
        let (l2, r2) = side_only_input(sr, 1000.0, 0.3, total);
        for i in 0..total {
            l[i] += l2[i];
            r[i] += r2[i];
        }
        (l, r)
    };

    let toggled = render_blocks(&mut Imager::new(sr), &total_input, block, |start| {
        if start < era {
            cfg_120
        } else if start < 2 * era {
            cfg_off_500
        } else {
            cfg_on_500
        }
    });
    // Reference: same stream, but disabled (already at 500 Hz) for the
    // whole pre-window, so the 120 Hz era never happened. Both runs
    // re-enable at the same sample with the same fade-in.
    let fresh = render_blocks(&mut Imager::new(sr), &total_input, block, |start| {
        if start < 2 * era {
            cfg_off_500
        } else {
            cfg_on_500
        }
    });

    let mut max_diff = 0.0_f32;
    for i in 2 * era..total {
        max_diff = max_diff.max((toggled[i] - fresh[i]).abs());
    }
    assert!(
        max_diff < 1e-4,
        "re-enable after moving the cutoff while disabled diverges from a \
         clean 500 Hz render by {max_diff}"
    );

    // And the 500 Hz cutoff must actually be in force: 100 Hz side
    // content sits ~28 dB down, whereas stale 120 Hz coefficients
    // would pass it nearly unattenuated.
    let tail = total - 2 * 480; // whole 100 Hz periods, past the fade
    let (mut c_sin, mut c_cos) = (0.0_f32, 0.0_f32);
    for i in tail..total {
        let side = 0.5 * (toggled[i] - (-toggled[i])); // L = -R here pre-imager; use L directly
        let ph = i as f32 / sr * 100.0 * std::f32::consts::TAU;
        c_sin += side * ph.sin();
        c_cos += side * ph.cos();
    }
    let n = (total - tail) as f32;
    let amp_100 = 2.0 * (c_sin * c_sin + c_cos * c_cos).sqrt() / n;
    assert!(
        amp_100 < 0.1,
        "100 Hz side content renders at {amp_100} — the 500 Hz cutoff set \
         while disabled is not in force"
    );
}
