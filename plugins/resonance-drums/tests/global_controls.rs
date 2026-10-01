//! The GLOBAL card's three controls do what they claim (ba todo #1326).
//!
//! POLYPHONY, VELOCITY CURVE and ROUND ROBIN were drawn from constants
//! and threw their interaction away: `MAX_VOICES` was a hard constant
//! with no parameter, and neither a velocity curve nor a random
//! round-robin existed anywhere in the DSP. Each is a parameter now, so
//! it is reachable from the editor and from `set_plugin_param` alike —
//! these tests drive the sampler through the parameters, which is the
//! path both surfaces take.

use resonance_drums::drum_map::{self, PAD_MAPPINGS};
use resonance_drums::dsp::voice_pick::{
    pick_rr, pick_rr_random, pick_velocity_layer, RoundRobinMode, NO_LAST_TAKE,
};
use resonance_drums::dsp::DrumSampler;
use resonance_drums::kit::{LoadedMicBank, LoadedPad, LoadedSample, VelocityLayer};
use resonance_drums::params::{DrumParams, ROUND_ROBIN_LABELS};
use resonance_drums::velocity;
use resonance_drums::voice::MAX_VOICES;
use resonance_drums::ResonanceDrums;
use resonance_plugin::param::Param;
use resonance_plugin::ResonancePlugin;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn make_sampler() -> DrumSampler {
    let (_tx, rx) = crossbeam_channel::unbounded::<Vec<LoadedPad>>();
    DrumSampler::new(rx)
}

fn layer(takes: usize) -> VelocityLayer {
    VelocityLayer::new((0..takes)
            // Long enough that a hit is still sounding when the next
            // one lands, so the voice count is what is under test.
            .map(|_| LoadedSample::from_data(vec![0.5; 8192]))
            .collect())
}

/// A pad with `layers` velocity layers of `takes` takes each, on a
/// single close mic — one voice per hit, which keeps the voice
/// arithmetic in these tests obvious.
fn pad_with(index: usize, layers: usize, takes: usize) -> LoadedPad {
    let m = &PAD_MAPPINGS[index];
    LoadedPad {
        name: m.name.to_string(),
        choke_group: m.choke_group,
        output_group: m.output_group,
        close_mics: vec![LoadedMicBank {
            position: "close".to_string(),
            setup_key: String::new(),
            layers: (0..layers).map(|_| layer(takes)).collect(),
        }],
        extra_banks: Vec::new(),
        overhead: None,
    }
}

fn sampler_with_pads(layers: usize, takes: usize) -> DrumSampler {
    let mut sampler = make_sampler();
    sampler.pads = (0..PAD_MAPPINGS.len())
        .map(|i| pad_with(i, layers, takes))
        .collect();
    sampler
}

fn active_voices(sampler: &DrumSampler) -> usize {
    sampler.voices.iter().filter(|v| v.active).count()
}

/// Pads with no choke group, so hits accumulate voices instead of
/// silencing each other. (Hi-hats all share one choke group.)
fn unchoked_notes() -> Vec<u8> {
    PAD_MAPPINGS
        .iter()
        .filter(|m| m.choke_group.is_none())
        .map(|m| m.note)
        .collect()
}

// ---------------------------------------------------------------------------
// Polyphony
// ---------------------------------------------------------------------------

/// The default is the ceiling the plugin has always had, so an existing
/// project is untouched until someone turns it down.
#[test]
fn polyphony_defaults_to_the_hard_voice_limit() {
    let params = DrumParams::default();
    assert_eq!(params.polyphony.value(), MAX_VOICES as i32);
    assert_eq!(params.polyphony.max_plain(), MAX_VOICES as f64);
    assert_eq!(params.polyphony.min_plain(), 1.0);
}

/// Turning polyphony down caps how many voices can sound at once.
#[test]
fn polyphony_caps_active_voices() {
    let notes = unchoked_notes();
    assert!(notes.len() >= 8, "need enough pads to exceed the cap");

    for limit in [1usize, 2, 5] {
        let mut sampler = sampler_with_pads(1, 1);
        let params = DrumParams::default();
        params.polyphony.set_value(limit as i32);
        sampler.update_global_settings(&params);

        for note in &notes {
            sampler.note_on(*note, 0.9);
            assert!(
                active_voices(&sampler) <= limit,
                "polyphony {limit} exceeded: {} voices active",
                active_voices(&sampler)
            );
        }
        assert_eq!(
            active_voices(&sampler),
            limit,
            "polyphony {limit} should fill up to the cap"
        );
    }
}

/// At the default, the sampler still uses every slot it has — the cap
/// adds no new stealing.
#[test]
fn default_polyphony_still_reaches_every_voice() {
    let mut sampler = sampler_with_pads(1, 1);
    sampler.update_global_settings(&DrumParams::default());

    let notes = unchoked_notes();
    // More hits than voices, whatever the cap.
    for _ in 0..MAX_VOICES.div_ceil(notes.len()) + 1 {
        for note in &notes {
            sampler.note_on(*note, 0.9);
        }
    }
    assert_eq!(active_voices(&sampler), MAX_VOICES);
}

/// All three controls are real, host-visible parameters, so the editor
/// is not the only way to reach them.
#[test]
fn the_global_controls_are_exposed_as_parameters() {
    let plugin = ResonanceDrums::new();
    let ids: Vec<&str> = (0..plugin.param_count())
        .map(|i| plugin.param(i))
        .filter(|p| !p.is_hidden())
        .map(|p| p.id())
        .collect();
    for id in ["polyphony", "velocity_curve", "round_robin_mode"] {
        assert!(ids.contains(&id), "{id} is not exposed to the host");
    }
}

/// A write through the `Param` trait — what `set_plugin_param` and a
/// host automation lane both do — is what the sampler picks up at the
/// top of the next block.
#[test]
fn a_param_write_caps_the_sampler() {
    let params = DrumParams::default();
    let polyphony: &dyn Param = &params.polyphony;
    polyphony.set_plain(8.0);

    let mut sampler = sampler_with_pads(1, 1);
    sampler.update_global_settings(&params);
    assert_eq!(sampler.global_settings().max_voices, 8);

    for note in unchoked_notes() {
        sampler.note_on(note, 0.9);
    }
    assert_eq!(active_voices(&sampler), 8);
}

/// A parameter cannot ask for more voices than the sampler has slots,
/// or for none at all.
#[test]
fn polyphony_is_clamped_to_the_sampler() {
    let params = DrumParams::default();
    let polyphony: &dyn Param = &params.polyphony;
    let mut sampler = make_sampler();

    polyphony.set_plain(1000.0);
    sampler.update_global_settings(&params);
    assert_eq!(sampler.global_settings().max_voices, MAX_VOICES);

    polyphony.set_plain(0.0);
    sampler.update_global_settings(&params);
    assert_eq!(sampler.global_settings().max_voices, 1);
}

// ---------------------------------------------------------------------------
// Velocity curve
// ---------------------------------------------------------------------------

/// Linear is an exact identity: an untouched project cannot change.
#[test]
fn the_default_velocity_curve_is_an_exact_identity() {
    assert_eq!(DrumParams::default().velocity_curve.value(), 0.0);
    for step in 0..=100 {
        let v = step as f32 / 100.0;
        assert_eq!(velocity::shape(v, 0.0).to_bits(), v.to_bits());
    }
}

/// Soft lifts a hit, hard pushes it down, and both keep the ends fixed
/// and the mapping monotonic.
#[test]
fn the_velocity_curve_maps_as_it_claims() {
    for curve in [0.25_f32, 0.5, 1.0] {
        assert_eq!(velocity::shape(0.0, curve), 0.0);
        assert_eq!(velocity::shape(1.0, curve), 1.0);
        assert_eq!(velocity::shape(0.0, -curve), 0.0);
        assert_eq!(velocity::shape(1.0, -curve), 1.0);

        let mut previous = -1.0;
        for step in 0..=100 {
            let v = step as f32 / 100.0;
            let soft = velocity::shape(v, curve);
            let hard = velocity::shape(v, -curve);
            assert!(soft >= previous, "soft curve must stay monotonic");
            previous = soft;
            if v > 0.0 && v < 1.0 {
                assert!(soft > v, "soft should lift {v}, got {soft}");
                assert!(hard < v, "hard should lower {v}, got {hard}");
            }
        }
    }
}

/// Out-of-range input is clamped rather than producing NaN.
#[test]
fn the_velocity_curve_clamps_its_inputs() {
    assert_eq!(velocity::shape(-1.0, 0.5), 0.0);
    assert_eq!(velocity::shape(2.0, 0.5), 1.0);
    assert!(velocity::shape(0.5, 9.0).is_finite());
    assert!(velocity::shape(0.5, -9.0).is_finite());
}

/// What the curve is for: the same MIDI velocity reaches a different
/// recorded layer. Driven through the real trigger path.
#[test]
fn the_velocity_curve_moves_which_layer_fires() {
    let velocity_in = 0.4_f32;
    let layers = 4;

    let linear_layer = {
        let mut sampler = sampler_with_pads(layers, 1);
        sampler.update_global_settings(&DrumParams::default());
        sampler.note_on(drum_map::KICK, velocity_in);
        sampler
            .voices
            .iter()
            .find(|v| v.active)
            .expect("a voice should have fired")
            .layer_index
    };

    let soft_layer = {
        let mut sampler = sampler_with_pads(layers, 1);
        let params = DrumParams::default();
        params.velocity_curve.set_value(1.0);
        sampler.update_global_settings(&params);
        sampler.note_on(drum_map::KICK, velocity_in);
        sampler
            .voices
            .iter()
            .find(|v| v.active)
            .expect("a voice should have fired")
            .layer_index
    };

    assert_eq!(
        linear_layer,
        pick_velocity_layer(velocity_in, layers),
        "linear must pick the layer the raw velocity maps to"
    );
    assert!(
        soft_layer > linear_layer,
        "a soft curve should reach a louder layer: {soft_layer} vs {linear_layer}"
    );
}

/// A single-layer pad bakes dynamics into the trigger gain, so the
/// curve has to move that too.
#[test]
fn the_velocity_curve_moves_the_trigger_gain() {
    let mut sampler = sampler_with_pads(1, 1);
    let params = DrumParams::default();
    params.velocity_curve.set_value(1.0);
    sampler.update_global_settings(&params);
    sampler.note_on(drum_map::KICK, 0.5);

    let gain = sampler
        .voices
        .iter()
        .find(|v| v.active)
        .expect("a voice should have fired")
        .base_gain;
    assert!(
        (gain - velocity::shape(0.5, 1.0)).abs() < 1e-6,
        "trigger gain should follow the curve, got {gain}"
    );
}

/// The curve reads as a name everywhere a parameter is rendered, and
/// what it displays it parses back.
#[test]
fn the_velocity_curve_is_labelled() {
    let params = DrumParams::default();
    let param: &dyn Param = &params.velocity_curve;
    assert_eq!(param.display(0.0), "Linear");
    assert_eq!(param.display(1.0), "Soft 100%");
    assert_eq!(param.display(-0.5), "Hard 50%");

    for value in [-1.0_f64, -0.5, 0.0, 0.25, 1.0] {
        let text = param.display(value);
        let parsed = param.parse(&text).expect("labels must parse back");
        assert!(
            (parsed - value).abs() < 0.005,
            "round trip lost {value}: '{text}' -> {parsed}"
        );
    }
    assert_eq!(param.parse("linear"), Some(0.0));
    assert!(param.parse("banana").is_none());
}

// ---------------------------------------------------------------------------
// Round robin
// ---------------------------------------------------------------------------

#[test]
fn round_robin_defaults_to_cycle_and_is_labelled() {
    let params = DrumParams::default();
    assert_eq!(
        RoundRobinMode::from_param(params.round_robin_mode.value()),
        RoundRobinMode::Cycle
    );
    assert_eq!(params.round_robin_mode.labels(), ROUND_ROBIN_LABELS);
    let param: &dyn Param = &params.round_robin_mode;
    assert_eq!(param.display(0.0), "Cycle");
    assert_eq!(param.display(1.0), "Random");
    assert_eq!(param.parse("random"), Some(1.0));
}

/// Cycle is unchanged: the takes walk in order.
#[test]
fn cycle_mode_walks_the_takes_in_order() {
    let mut counter = 0;
    let picked: Vec<usize> = (0..6).map(|_| pick_rr(&mut counter, 3)).collect();
    assert_eq!(picked, vec![0, 1, 2, 0, 1, 2]);
}

/// Random never plays the same take twice running — the one thing round
/// robin exists to prevent.
#[test]
fn random_mode_never_repeats_the_previous_take() {
    let mut state = 0x1234_5678;
    let mut last = NO_LAST_TAKE;
    for _ in 0..2000 {
        let picked = pick_rr_random(&mut state, last, 4);
        assert!(picked < 4);
        if last != NO_LAST_TAKE {
            assert_ne!(picked, last as usize, "random repeated a take");
        }
        last = picked as u16;
    }
}

/// …and it still reaches every take.
#[test]
fn random_mode_covers_every_take() {
    let mut state = 0x1234_5678;
    let mut last = NO_LAST_TAKE;
    let mut seen = [false; 5];
    for _ in 0..2000 {
        let picked = pick_rr_random(&mut state, last, 5);
        seen[picked] = true;
        last = picked as u16;
    }
    assert!(seen.iter().all(|s| *s), "some take never fired: {seen:?}");
}

/// A single take can only ever pick itself, in either mode.
#[test]
fn random_mode_handles_a_single_take() {
    let mut state = 1;
    assert_eq!(pick_rr_random(&mut state, NO_LAST_TAKE, 1), 0);
    assert_eq!(pick_rr_random(&mut state, 0, 1), 0);
}

/// Through the sampler: with the parameter set to Random, consecutive
/// hits on one pad stop marching 0, 1, 2, 0 …
#[test]
fn random_mode_changes_what_the_sampler_picks() {
    let takes = 4;

    let cycle: Vec<usize> = {
        let mut sampler = sampler_with_pads(1, takes);
        sampler.update_global_settings(&DrumParams::default());
        (0..8)
            .map(|_| {
                sampler.note_on(drum_map::KICK, 0.9);
                sampler
                    .voices
                    .iter()
                    .filter(|v| v.active)
                    .max_by_key(|v| v.age)
                    .expect("a voice should have fired")
                    .rr_index
            })
            .collect()
    };
    assert_eq!(cycle, vec![0, 1, 2, 3, 0, 1, 2, 3]);

    let random: Vec<usize> = {
        let mut sampler = sampler_with_pads(1, takes);
        let params = DrumParams::default();
        params.round_robin_mode.set_value(1);
        sampler.update_global_settings(&params);
        (0..8)
            .map(|_| {
                sampler.note_on(drum_map::KICK, 0.9);
                sampler
                    .voices
                    .iter()
                    .filter(|v| v.active)
                    .max_by_key(|v| v.age)
                    .expect("a voice should have fired")
                    .rr_index
            })
            .collect()
    };
    assert_ne!(random, cycle, "Random should not walk in order");
    for pair in random.windows(2) {
        assert_ne!(pair[0], pair[1], "Random repeated a take back to back");
    }
}
