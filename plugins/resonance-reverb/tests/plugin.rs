use resonance_plugin::{EventIterator, OutputBuffer, ResonancePlugin};
use resonance_reverb::params::PARAM_COUNT;
use resonance_reverb::presets::PRESETS;
use resonance_reverb::ResonanceReverb;

#[test]
fn param_enumeration_covers_declared_count() {
    let plugin = ResonanceReverb::new();
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
    let mut plugin = ResonanceReverb::new();
    plugin.initialize(48_000.0, 4096);

    let frames = 4096usize;
    let mut left = vec![0.0_f32; frames];
    let mut right = vec![0.0_f32; frames];
    left[0] = 1.0;
    right[0] = 1.0;

    let mut outs = [OutputBuffer {
        left: &mut left,
        right: &mut right,
    }];
    let mut ev = EventIterator::empty();
    plugin.process(&mut outs, frames, &mut ev, None);

    for &x in left.iter().chain(right.iter()) {
        assert!(x.is_finite(), "non-finite sample: {x}");
    }
    let tail_energy: f32 = left[200..].iter().map(|x| x.abs()).sum();
    assert!(
        tail_energy > 1e-4,
        "expected audible tail, got {tail_energy}"
    );
}

/// Diagnostic: dumps the impulse response envelope so it's visible
/// in `cargo test -- --nocapture`. Not an assertion — just a window
/// into what the reverb actually produces for a unit impulse.
#[test]
fn debug_impulse_response_envelope() {
    let mut plugin = ResonanceReverb::new();
    plugin.initialize(48_000.0, 4096);
    plugin.params.mix.set_value(1.0); // 100% wet
    plugin.params.er_level.set_value(0.0); // isolate FDN path
    plugin.params.predelay.set_value(0.0);
    plugin.params.size.set_value(0.5);
    plugin.params.decay.set_value(2.0);

    let frames = 48_000usize; // 1 second
    let mut left = vec![0.0_f32; frames];
    let mut right = vec![0.0_f32; frames];
    left[0] = 1.0;
    right[0] = 1.0;

    // Process in blocks of 4096 so smoothers settle naturally.
    let block = 4096usize;
    let mut pos = 0;
    while pos < frames {
        let n = (frames - pos).min(block);
        let (l_slice, _) = left[pos..pos + n].split_at_mut(n);
        let (r_slice, _) = right[pos..pos + n].split_at_mut(n);
        let mut outs = [OutputBuffer {
            left: l_slice,
            right: r_slice,
        }];
        let mut ev = EventIterator::empty();
        plugin.process(&mut outs, n, &mut ev, None);
        pos += n;
    }

    // Print energy in 10 ms windows.
    println!("\nImpulse response envelope (10 ms bins, L channel, size=0.5, decay=2s):");
    let win = 480; // 10 ms @ 48k
    for bin in 0..100 {
        let start = bin * win;
        let end = (start + win).min(frames);
        let peak = left[start..end]
            .iter()
            .map(|x| x.abs())
            .fold(0.0_f32, f32::max);
        let db = if peak > 1e-9 {
            20.0 * peak.log10()
        } else {
            -120.0
        };
        let bars = ((db + 60.0).max(0.0) / 2.0) as usize;
        println!(
            "  t={:4} ms  peak={:8.5}  {:6.1} dB  {}",
            bin * 10,
            peak,
            db,
            "#".repeat(bars)
        );
    }
}

/// Regression test for the "reverb onset too late" bug: with default
/// size (0.5, log-mapped to ~90 ms) and predelay 0, the first wet
/// energy must arrive within ~50 ms (2400 samples @ 48 kHz). This
/// pins the FDN channel-0 delay + diffusion latency to something
/// room-like, not cathedral-like.
#[test]
fn wet_onset_is_room_like_not_cathedral() {
    let mut plugin = ResonanceReverb::new();
    plugin.initialize(48_000.0, 4096);
    // Crank mix to 1.0 so the signal we measure is purely wet, and
    // drop ER to 0 to isolate the FDN/diffusion path from the ER path.
    plugin.params.mix.set_value(1.0);
    plugin.params.er_level.set_value(0.0);
    plugin.params.predelay.set_value(0.0);
    plugin.params.size.set_value(0.5);

    let frames = 4096usize;
    let mut left = vec![0.0_f32; frames];
    let mut right = vec![0.0_f32; frames];
    left[0] = 1.0;
    right[0] = 1.0;

    let mut outs = [OutputBuffer {
        left: &mut left,
        right: &mut right,
    }];
    let mut ev = EventIterator::empty();
    plugin.process(&mut outs, frames, &mut ev, None);

    // Find first sample after t=1 where wet magnitude exceeds a small
    // threshold. Skip index 0 because it contains the dry leak from
    // the one-sample pre-delay tap-before-push ordering.
    let threshold = 0.001f32;
    let first_nonzero = (1..frames)
        .find(|&i| left[i].abs() > threshold)
        .unwrap_or(frames);
    assert!(
        first_nonzero < 2400,
        "wet onset landed at sample {first_nonzero} (> 50 ms @ 48k) — reverb is too late"
    );
}
