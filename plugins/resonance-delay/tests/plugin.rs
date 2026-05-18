use resonance_delay::params::PARAM_COUNT;
use resonance_delay::presets::PRESETS;
use resonance_delay::ResonanceDelay;
use resonance_plugin::{EventIterator, OutputBuffer, Param, ResonancePlugin, TempoInfo};

#[test]
fn param_enumeration_covers_declared_count() {
    let plugin = ResonanceDelay::new();
    assert_eq!(plugin.param_count(), PARAM_COUNT);
    let mut seen = std::collections::HashSet::new();
    for i in 0..plugin.param_count() {
        let id = plugin.param(i).id().to_string();
        assert!(seen.insert(id.clone()), "duplicate param id: {id}");
    }
}

#[test]
fn every_factory_preset_parses() {
    assert!(!PRESETS.is_empty());
    for entry in PRESETS {
        let value: serde_json::Value = serde_json::from_str(entry.json)
            .unwrap_or_else(|e| panic!("preset {:?} invalid: {e}", entry.name));
        assert!(
            value.get("params").and_then(|v| v.as_object()).is_some(),
            "preset {:?} missing `params` object",
            entry.name
        );
    }
}

#[test]
fn dsp_processes_impulse_without_nans() {
    let mut plugin = ResonanceDelay::new();
    plugin.initialize(48_000.0, 4096);

    let frames = 48_000usize;
    let mut left = vec![0.0f32; frames];
    let mut right = vec![0.0f32; frames];
    left[0] = 1.0;
    right[0] = 1.0;

    let block = 4096;
    let mut pos = 0;
    while pos < frames {
        let n = (frames - pos).min(block);
        let mut outs = [OutputBuffer {
            left: &mut left[pos..pos + n],
            right: &mut right[pos..pos + n],
        }];
        let mut ev = EventIterator::empty();
        plugin.process(&mut outs, n, &mut ev, None);
        pos += n;
    }

    for &x in left.iter().chain(right.iter()) {
        assert!(x.is_finite(), "non-finite sample: {x}");
    }
    let echo_energy: f32 = left[15000..20000].iter().map(|x| x.abs()).sum();
    assert!(
        echo_energy > 1e-4,
        "expected audible echo, got {echo_energy}"
    );
}

#[test]
fn ping_pong_crosses_channels() {
    let mut plugin = ResonanceDelay::new();
    plugin.params.routing.set_value(1); // Ping-Pong
    plugin.params.sync.set_plain(0.0); // free
    plugin.params.time_ms.set_value(100.0); // 100ms = 4800 samples
    plugin.params.feedback.set_value(0.8);
    plugin.params.mix.set_value(1.0);
    plugin.initialize(48_000.0, 4096);

    let frames = 48_000usize;
    let mut left = vec![0.0f32; frames];
    let mut right = vec![0.0f32; frames];
    // Impulse on left only.
    left[0] = 1.0;

    let block = 4096;
    let mut pos = 0;
    while pos < frames {
        let n = (frames - pos).min(block);
        let mut outs = [OutputBuffer {
            left: &mut left[pos..pos + n],
            right: &mut right[pos..pos + n],
        }];
        let mut ev = EventIterator::empty();
        plugin.process(&mut outs, n, &mut ev, None);
        pos += n;
    }

    // First echo should appear in L (the input channel) around sample 4800.
    // Second echo should appear in R around sample 9600.
    let first_echo_l: f32 = left[4500..5500].iter().map(|x| x.abs()).sum();
    let first_echo_r: f32 = right[9000..10500].iter().map(|x| x.abs()).sum();
    assert!(
        first_echo_l > 0.01,
        "expected L echo at ~4800, got energy {first_echo_l}"
    );
    assert!(
        first_echo_r > 0.01,
        "expected R echo at ~9600, got energy {first_echo_r}"
    );
}

#[test]
fn sync_division_matches_bpm() {
    let mut plugin = ResonanceDelay::new();
    plugin.initialize(48_000.0, 4096);
    plugin.params.sync.set_plain(1.0); // sync on
    plugin.params.division.set_value(4); // 1/4 note
    plugin.params.feedback.set_value(0.5);
    plugin.params.mix.set_value(1.0);
    plugin.params.mod_depth.set_value(0.0);

    let tempo = Some(TempoInfo {
        bpm: 120.0,
        time_sig_num: 4,
        time_sig_den: 4,
        playing: true,
        song_pos_beats: 0.0,
    });

    // At 120 BPM, 1/4 note = 0.5s = 24000 samples.
    let frames = 48_000usize;
    let mut left = vec![0.0f32; frames];
    let mut right = vec![0.0f32; frames];
    left[0] = 1.0;
    right[0] = 1.0;

    let block = 4096;
    let mut pos = 0;
    while pos < frames {
        let n = (frames - pos).min(block);
        let mut outs = [OutputBuffer {
            left: &mut left[pos..pos + n],
            right: &mut right[pos..pos + n],
        }];
        let mut ev = EventIterator::empty();
        plugin.process(&mut outs, n, &mut ev, tempo);
        pos += n;
    }

    // Find the first significant echo.
    let threshold = 0.01;
    let first_echo = (1..frames)
        .find(|&i| left[i].abs() > threshold)
        .unwrap_or(frames);
    // Should be within 200 samples of 24000.
    assert!(
        (first_echo as i64 - 24000).unsigned_abs() < 200,
        "expected echo at ~24000, got {first_echo}"
    );
}

#[test]
fn freeze_sustains_signal() {
    let mut plugin = ResonanceDelay::new();
    plugin.initialize(48_000.0, 4096);
    plugin.params.sync.set_plain(0.0);
    plugin.params.time_ms.set_value(100.0);
    plugin.params.feedback.set_value(0.8);
    plugin.params.mix.set_value(1.0);

    // Feed a tone for 0.5s.
    let sr = 48_000usize;
    let tone_frames = sr / 2;
    let total_frames = sr * 2;
    let mut left = vec![0.0f32; total_frames];
    let mut right = vec![0.0f32; total_frames];
    for i in 0..tone_frames {
        let t = i as f32 / sr as f32;
        let s = (440.0 * t * std::f32::consts::TAU).sin() * 0.5;
        left[i] = s;
        right[i] = s;
    }

    // Process first half (with signal).
    let block = 4096;
    let mut pos = 0;
    while pos < tone_frames {
        let n = (tone_frames - pos).min(block);
        let mut outs = [OutputBuffer {
            left: &mut left[pos..pos + n],
            right: &mut right[pos..pos + n],
        }];
        let mut ev = EventIterator::empty();
        plugin.process(&mut outs, n, &mut ev, None);
        pos += n;
    }

    // Enable freeze.
    plugin.params.freeze.set_plain(1.0);

    // Measure RMS right after freeze.
    let pre_freeze_start = tone_frames;
    let pre_freeze_end = (tone_frames + sr / 10).min(total_frames);
    while pos < pre_freeze_end {
        let n = (pre_freeze_end - pos).min(block);
        let mut outs = [OutputBuffer {
            left: &mut left[pos..pos + n],
            right: &mut right[pos..pos + n],
        }];
        let mut ev = EventIterator::empty();
        plugin.process(&mut outs, n, &mut ev, None);
        pos += n;
    }
    let rms_early: f32 = left[pre_freeze_start..pre_freeze_end]
        .iter()
        .map(|x| x * x)
        .sum::<f32>()
        / (pre_freeze_end - pre_freeze_start) as f32;

    // Continue for another second.
    while pos < total_frames {
        let n = (total_frames - pos).min(block);
        let mut outs = [OutputBuffer {
            left: &mut left[pos..pos + n],
            right: &mut right[pos..pos + n],
        }];
        let mut ev = EventIterator::empty();
        plugin.process(&mut outs, n, &mut ev, None);
        pos += n;
    }

    let late_start = total_frames - sr / 4;
    let rms_late: f32 = left[late_start..total_frames]
        .iter()
        .map(|x| x * x)
        .sum::<f32>()
        / (total_frames - late_start) as f32;

    // Frozen signal should sustain within 6 dB of the early level.
    let ratio_db = 10.0 * (rms_late / rms_early.max(1e-12)).log10();
    assert!(
        ratio_db > -6.0,
        "freeze signal decayed too much: {ratio_db:.1} dB"
    );
}
