//! The `decay` detail (reverb-algorithms.md R9) on a REAL reverb: a track
//! plays two seconds of noise, sending into a fully wet resonance-reverb on
//! a return, and stops; the decay read off the TRACK's stem (which carries
//! its send's return back, while a bus target only hears tracks routed
//! into the bus) is the reverb's decay knob.
//!
//! Needs the resonance-reverb binary (`plugin_binaries`: the debug cdylib
//! `./scripts/run-tests.py` builds, or the bundle) and fails without one
//! unless `RESONANCE_ALLOW_MISSING_PLUGIN_BINARIES` is set. The synthetic
//! half (known T60s, overlapped and cut-off decays) is
//! `resonance-metering/tests/decay.rs`.

use std::path::PathBuf;
use std::sync::Arc;

use resonance_audio::test_support::{
    measure_mix_detailed, AutomationSnapshot, ClapBundle, MeasureSource, PluginSlot, SharedState,
    StemSource,
};
use resonance_audio::types::*;
use resonance_metering::decay::DecayEnd;

use crate::plugin_binaries::plugin_binary;

const SR: u32 = 48_000;
/// The noise plays for this long, from 0.
const PLAY: usize = SR as usize * 2;
const BUS: u64 = 50;
const FX: u64 = 500;
const TRACK: u64 = 1;

fn reverb_binary() -> Option<PathBuf> {
    plugin_binary("resonance-reverb")
}

/// Set `key` to `to` wherever it appears in a JSON state.
fn set_key(value: &mut serde_json::Value, key: &str, to: f64) -> bool {
    match value {
        serde_json::Value::Object(map) => {
            let mut found = false;
            for (k, v) in map.iter_mut() {
                if k == key {
                    *v = serde_json::json!(to);
                    found = true;
                } else {
                    found |= set_key(v, key, to);
                }
            }
            found
        }
        _ => false,
    }
}

/// A fully wet reverb with the given decay knob, damping open.
fn reverb(bundle: &ClapBundle, decay_s: f64) -> PluginSlot {
    let id = bundle.descriptors()[0].id.clone();
    let mut inst = bundle.create_instance(&id, SR).expect("reverb instance");
    let saved = inst.save_state().expect("reverb saves state");
    let mut json: serde_json::Value = serde_json::from_slice(&saved).expect("JSON state");
    assert!(set_key(&mut json, "mix", 1.0), "no `mix`: {json}");
    assert!(set_key(&mut json, "decay", decay_s), "no `decay`: {json}");
    assert!(inst.reload_with_state(&serde_json::to_vec(&json).unwrap()));
    PluginSlot::new(inst)
}

/// Two seconds of low-passed noise (most energy below 2 kHz, where a
/// reverb's decay knob is defined), then nothing.
fn noise_clip() -> AudioClip {
    let mut state = 12_345u32;
    let mut lp = [0.0f32; 2];
    let pcm: Vec<f32> = (0..PLAY * 2)
        .map(|i| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let x = ((state >> 8) as f32 / 8_388_608.0 - 1.0) * 0.5;
            let ch = &mut lp[i % 2];
            *ch += 0.2 * (x - *ch);
            *ch
        })
        .collect();
    AudioClip {
        id: 1,
        track_id: TRACK,
        start_sample: 0,
        source: ClipSource::memory(pcm),
        name: "noise".into(),
        trim_start_frames: 0,
        trim_end_frames: 0,
        fade_in_frames: 0,
        fade_in_curve: FadeCurve::default(),
        fade_out_frames: 0,
        fade_out_curve: FadeCurve::default(),
        gain_db: 0.0,
        vocal_tuning: None,
        warp_enabled: false,
        original_bpm: None,
        transpose_semitones: 0.0,
        warp_algorithm: WarpAlgorithm::default(),
        warp_markers: Vec::new(),
        tuning_render_cache: None,
    }
}

/// Measure the sending track's `decay` detail with the reverb's knob at
/// `decay_s`, over `0..range_s`.
fn measure_track(bundle: &ClapBundle, decay_s: f64, range_s: f64) -> MixMeasurement {
    let shared = Arc::new(SharedState::default());
    let tempo = Arc::new(arc_swap::ArcSwap::from_pointee(TempoMap::default()));
    shared.edit_plugins(|p| {
        p.insert(FX, Arc::new(reverb(bundle, decay_s)));
    });
    let mut bus = Bus::new(BUS, "room".to_string());
    bus.plugin_ids.push(FX);
    bus.set_is_return(true);
    shared.edit_busses(|b| {
        b.insert(BUS, Arc::new(bus));
    });
    shared.edit_tracks(|m| {
        m.insert(TRACK, Arc::new(Track::new(TRACK, "source".to_string())));
    });
    shared.edit_clips(|c| c.push(Arc::new(noise_clip())));
    shared.aux_sends.store(Arc::new(vec![AuxSend {
        id: 1,
        source: SendSource::Track(TRACK),
        dest: BUS,
        level_db: 0.0,
        pre_fader: false,
        enabled: true,
    }]));

    let (tx, rx) = crossbeam_channel::unbounded();
    let detail = DetailSet {
        decay: true,
        ..DetailSet::default()
    };
    measure_mix_detailed(
        9,
        vec![StemSource::Track(TRACK)],
        Some((0, (range_s * f64::from(SR)) as u64)),
        MeasureSource::Render,
        detail,
        &shared,
        &tempo,
        &AutomationSnapshot::default(),
        SR,
        &tx,
    );
    let events: Vec<AudioEvent> = rx.try_iter().collect();
    let [AudioEvent::MixMeasured { results, .. }] = events.as_slice() else {
        panic!("expected one MixMeasured, got {events:?}");
    };
    results[0].clone()
}

#[test]
fn the_sending_track_decay_reads_the_reverb_knob() {
    let Some(path) = reverb_binary() else {
        return;
    };
    let bundle = ClapBundle::load(&path).expect("load the reverb");
    for knob in [0.8f64, 2.0] {
        let m = measure_track(&bundle, knob, 2.0 + 2.0 * knob + 0.5);
        assert!(m.lufs_integrated > -60.0, "the track is not silent: {}", m.lufs_integrated);
        let d = m.detail.decay.expect("decay asked for");
        println!(
            "knob {knob}: start {:.3} s, {:?}, {:.1} dB, EDT {:?} T20 {:?} T30 {:?}, \
             20 dB after {:?}, bands {:?}",
            d.start_s(),
            d.ends,
            d.dynamic_range_db,
            d.times.edt,
            d.times.t20,
            d.times.t30,
            d.tail_20db_s,
            d.bands.iter().map(|b| b.times.t30).collect::<Vec<_>>(),
        );
        assert!(d.found && d.clean, "knob {knob}: {d:?}");
        assert!(
            (d.start_s() - 2.0).abs() < 0.1,
            "knob {knob}: the decay starts at the clip's end, not {} s",
            d.start_s()
        );
        assert_eq!(d.ends, Some(DecayEnd::Floor), "knob {knob}");
        let t30 = d.times.t30.expect("clean means T30") as f64;
        assert!(
            (t30 - knob).abs() / knob < 0.25,
            "knob {knob}: broadband T30 {t30:.3} s"
        );
        let t = |hz: f32| d.bands.iter().find(|b| b.center_hz == hz).unwrap().times.t30;
        let mid = 0.5 * (t(500.0).expect("500 Hz") + t(1_000.0).expect("1 kHz")) as f64;
        assert!(
            (mid - knob).abs() / knob < 0.2,
            "knob {knob}: mid T30 {mid:.3} s"
        );
        assert!(d.times.edt.is_some() && d.times.t20.is_some());
        assert!(d.note(0.0).is_none());
    }
}

#[test]
fn a_range_that_cuts_the_tail_reports_no_t30() {
    let Some(path) = reverb_binary() else {
        return;
    };
    let bundle = ClapBundle::load(&path).expect("load the reverb");
    // A 4 s tail, cut 0.4 s after the stop.
    let d = measure_track(&bundle, 4.0, 2.4).detail.decay.expect("decay asked for");
    assert!(d.found && !d.clean, "{d:?}");
    assert_eq!(d.ends, Some(DecayEnd::RangeEnd));
    assert_eq!((d.times.t20, d.times.t30), (None, None));
    assert!(d.note(0.0).expect("a note").contains("range ends"));
}
