//! The depth estimate with REAL reverbs on the returns
//! (warmth-width-depth.md §7.6, W11 exit criterion): a front track with a
//! whisper of a short room, a middle one with a real send to it, and a
//! back one with a big send to a long room order front > middle > back.
//!
//! Needs the resonance-reverb binary — `target/bundled/resonance-reverb.clap`
//! (scripts/bundle.sh) or the debug cdylib `libresonance_reverb.so`
//! (`cargo build -p resonance-reverb`) — and skips without one. The
//! plugin-free twin with exact numbers is `tests/bounce/measure_depth.rs`.

use std::path::PathBuf;
use std::sync::Arc;

use resonance_audio::test_support::{
    measure_mix_detailed, AutomationSnapshot, ClapBundle, MeasureSource, PluginSlot, SharedState,
    StemSource,
};
use resonance_audio::types::*;

const SR: u32 = 48_000;
const FRAMES: usize = SR as usize * 4;

fn reverb_binary() -> Option<PathBuf> {
    [
        "target/bundled/resonance-reverb.clap",
        "../target/bundled/resonance-reverb.clap",
        "target/debug/libresonance_reverb.so",
        "../target/debug/libresonance_reverb.so",
    ]
    .into_iter()
    .map(PathBuf::from)
    .find(|p| p.exists())
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

/// A fully wet reverb instance with the given decay, as a return insert.
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

fn noise_clip(id: ClipId, track: TrackId, seed: u32) -> AudioClip {
    let mut state = seed | 1;
    let pcm: Vec<f32> = (0..FRAMES * 2)
        .map(|_| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            ((state >> 8) as f32 / 8_388_608.0 - 1.0) * 0.2
        })
        .collect();
    AudioClip {
        id,
        track_id: track,
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

#[test]
fn real_reverbs_order_three_layers_front_to_back() {
    let Some(path) = reverb_binary() else {
        eprintln!(
            "[skip] no resonance-reverb binary (scripts/bundle.sh, or cargo build -p \
             resonance-reverb)"
        );
        return;
    };
    let bundle = ClapBundle::load(&path).expect("load the reverb");
    let shared = Arc::new(SharedState::default());
    let tempo = Arc::new(arc_swap::ArcSwap::from_pointee(TempoMap::default()));

    // Short room (0.4 s) on bus 50, long room (6 s) on bus 51.
    for (bus_id, fx_id, decay) in [(50u64, 500u64, 0.4), (51, 510, 6.0)] {
        shared.edit_plugins(|p| {
            p.insert(fx_id, Arc::new(reverb(&bundle, decay)));
        });
        let mut bus = Bus::new(bus_id, format!("room {bus_id}"));
        bus.plugin_ids.push(fx_id);
        bus.set_is_return(true);
        shared.edit_busses(|b| {
            b.insert(bus_id, Arc::new(bus));
        });
    }
    for id in 1..=4u64 {
        shared.edit_tracks(|m| {
            m.insert(id, Arc::new(Track::new(id, format!("track {id}"))));
        });
        shared.edit_clips(|c| c.push(Arc::new(noise_clip(id, id, 7 * id as u32))));
    }
    let send = |id: SendId, from: TrackId, dest: BusId, level_db: f32| AuxSend {
        id,
        source: SendSource::Track(from),
        dest,
        level_db,
        pre_fader: false,
        enabled: true,
    };
    shared.aux_sends.store(Arc::new(vec![
        send(1, 1, 50, -30.0), // front: a whisper of the short room
        send(2, 2, 50, -10.0), // middle
        send(3, 3, 51, 0.0),   // back: a big send to the long room
    ]));

    let (tx, rx) = crossbeam_channel::unbounded();
    let targets = (1..=4u64).map(StemSource::Track).collect();
    let detail = DetailSet {
        depth: true,
        ..DetailSet::default()
    };
    measure_mix_detailed(
        3,
        targets,
        None,
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
    let depth = |i: usize| results[i].detail.depth.clone().expect("depth");
    let drr = |i: usize| depth(i).drr_db_estimate.expect("a track with a send");
    let (front, middle, back) = (drr(0), drr(1), drr(2));
    assert!(
        front > middle && middle > back,
        "front > middle > back: {front} {middle} {back} ({:?})",
        depth(2).sends
    );
    assert!(front - middle > 10.0, "20 dB less send is well in front: {front} {middle}");
    assert!(depth(3).dry_only);
    for i in 0..3 {
        assert!(depth(i).sends[0].return_gain_db.is_some(), "each room produced output");
    }
}
