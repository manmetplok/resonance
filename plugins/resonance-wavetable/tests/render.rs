use resonance_plugin::{EventIterator, NoteEvent};
use resonance_wavetable::dsp::engine::SynthEngine;
use resonance_wavetable::params::WavetableParams;
use resonance_wavetable::viz::WavetableVizState;

const SR: f32 = 48_000.0;
const BLOCK: usize = 512;

fn render(engine: &mut SynthEngine, params: &WavetableParams, events: &[NoteEvent]) -> Vec<f32> {
    let mut left = vec![0.0f32; BLOCK];
    let mut right = vec![0.0f32; BLOCK];
    let mut iter = EventIterator::new(events);
    engine.render_block(&mut left, &mut right, BLOCK, params, &mut iter, None);
    left.extend_from_slice(&right);
    left
}

fn active_voices(engine: &mut SynthEngine, params: &WavetableParams) -> u32 {
    let viz = WavetableVizState::new();
    engine.publish_viz(params, &viz);
    viz.read_snapshot().active_voice_count
}

/// Sanity for the oscillator-mixing skip: with an oscillator enabled the
/// active-voice path still produces real, finite audio.
#[test]
fn enabled_oscillator_produces_audio() {
    let params = WavetableParams::new();
    let mut engine = SynthEngine::new();
    engine.initialize(SR);

    let out = render(
        &mut engine,
        &params,
        &[NoteEvent::NoteOn {
            note: 60,
            velocity: 1.0,
            timing: 0,
        }],
    );

    assert!(out.iter().all(|s| s.is_finite()));
    let peak = out.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(peak > 1e-3, "enabled oscillator rendered silence");
}

/// With both oscillators disabled the block must be exactly silent, and —
/// crucially — voice lifecycle must still advance: envelopes keep running,
/// so a released note drains to Idle instead of being stuck forever by the
/// mixing skip.
#[test]
fn disabled_oscillators_are_silent_and_voices_drain() {
    let params = WavetableParams::new();
    params.osc1.enabled.set_value(false);
    params.osc2.enabled.set_value(false);
    // Keep the lifecycle check fast.
    params.amp_env.release.set_value(0.05);

    let mut engine = SynthEngine::new();
    engine.initialize(SR);

    let out = render(
        &mut engine,
        &params,
        &[NoteEvent::NoteOn {
            note: 60,
            velocity: 1.0,
            timing: 0,
        }],
    );
    assert!(
        out.iter().all(|s| *s == 0.0),
        "disabled oscillators leaked audio"
    );
    assert_eq!(active_voices(&mut engine, &params), 1);

    // Release the note and render well past the release time.
    render(&mut engine, &params, &[NoteEvent::NoteOff { note: 60, timing: 0 }]);
    for _ in 0..((SR as usize) / BLOCK) {
        let out = render(&mut engine, &params, &[]);
        assert!(out.iter().all(|s| *s == 0.0));
    }

    assert_eq!(
        active_voices(&mut engine, &params),
        0,
        "voice stuck non-idle: envelope stopped advancing under the osc skip"
    );
}

/// Re-enabling the oscillators mid-voice resumes audio: the skip must not
/// have frozen any state a live voice depends on.
#[test]
fn reenabling_oscillators_resumes_audio() {
    let params = WavetableParams::new();
    params.osc1.enabled.set_value(false);
    params.osc2.enabled.set_value(false);

    let mut engine = SynthEngine::new();
    engine.initialize(SR);

    let out = render(
        &mut engine,
        &params,
        &[NoteEvent::NoteOn {
            note: 60,
            velocity: 1.0,
            timing: 0,
        }],
    );
    assert!(out.iter().all(|s| *s == 0.0));

    params.osc1.enabled.set_value(true);
    let out = render(&mut engine, &params, &[]);
    let peak = out.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(peak > 1e-3, "re-enabled oscillator stayed silent");
}

/// DSP2-16: the control-rate grid (filter/mod refresh every 16 samples,
/// drift every 64) runs across blocks, so with static parameters the
/// output does not depend on the block size. It used to restart at each
/// block's sample 0, so a bounce and a live render in different block
/// sizes stepped a fast LFO on cutoff at different samples.
#[test]
fn output_does_not_depend_on_block_size() {
    let params = WavetableParams::new();
    params.filter.enabled.set_value(true);
    params.filter.cutoff.set_value(1200.0);
    params.lfo1.rate.set_value(9.0);
    params.lfo1.depth.set_value(0.8);
    params.mod_slots[0].source.set_value(1); // LFO1
    params.mod_slots[0].destination.set_value(5); // filter cutoff
    params.mod_slots[0].amount.set_value(0.6);
    params.analog.drift.set_value(0.5);
    let render_in = |block: usize| {
        let mut engine = SynthEngine::new();
        engine.initialize(SR);
        let total = 24_000usize;
        let mut out = Vec::with_capacity(total);
        let mut start = 0;
        while start < total {
            let n = block.min(total - start);
            let events = if start == 0 {
                vec![NoteEvent::NoteOn {
                    note: 48,
                    velocity: 1.0,
                    timing: 0,
                }]
            } else {
                Vec::new()
            };
            let mut left = vec![0.0f32; n];
            let mut right = vec![0.0f32; n];
            let mut iter = EventIterator::new(&events);
            engine.render_block(&mut left, &mut right, n, &params, &mut iter, None);
            out.extend_from_slice(&left);
            start += n;
        }
        out
    };
    let a = render_in(128);
    let b = render_in(37);
    assert!(a.iter().fold(0.0f32, |m, s| m.max(s.abs())) > 1e-3, "rendered silence");
    let diff = a.iter().zip(&b).fold(0.0f32, |m, (x, y)| m.max((x - y).abs()));
    assert!(diff < 1e-5, "block 128 vs block 37 differ by {diff:.2e}");
}
