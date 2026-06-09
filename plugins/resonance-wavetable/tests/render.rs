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
    engine.render_block(&mut left, &mut right, BLOCK, params, &mut iter);
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
