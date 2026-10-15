//! A frozen track's aux sends (BUG-67, field report 2026-10-06 §3).
//!
//! The freeze cache holds the track's own post-FX signal and nothing
//! else: its sends are re-applied live from the cache on playback, so a
//! send baked into the cache — or a return bus's output summed into it —
//! would be heard twice. These tests freeze through the real renderer
//! (`to_freeze_cache`), attach the cache as a project load does, and
//! compare the master render with the unfrozen one.

use std::ffi::c_void;
use std::ptr;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use clap_sys::plugin::clap_plugin;
use clap_sys::process::{clap_process, clap_process_status, CLAP_PROCESS_CONTINUE};

use crate::multi_out_harness;

use resonance_audio::test_support::{
    __instance_from_raw_for_test, render_stem, to_freeze_cache, AutomationSnapshot, PluginSlot,
    SharedState, StemSource,
};
use resonance_audio::types::*;

const SR: u32 = 48_000;
/// The return bus every test sends to.
const RETURN: BusId = 60;
const RETURN_FX: PluginInstanceId = 700;
const TRACK: TrackId = 1;
/// Frames the master render covers (well inside the tone clip).
const FRAMES: u64 = 4_096;

// ---------------------------------------------------------------------------
// Fake return effect: out = in * gain + hiss (a constant the plugin adds on
// its own, standing in for a self-noising effect).
// ---------------------------------------------------------------------------

struct GainHiss {
    gain: f32,
    hiss: f32,
}

unsafe fn gh<'a>(plugin: *const clap_plugin) -> &'a GainHiss {
    unsafe { &*((*plugin).plugin_data as *const GainHiss) }
}

unsafe extern "C" fn gh_init(_p: *const clap_plugin) -> bool {
    true
}
unsafe extern "C" fn gh_destroy(_p: *const clap_plugin) {}
unsafe extern "C" fn gh_activate(_p: *const clap_plugin, _sr: f64, _min: u32, _max: u32) -> bool {
    true
}
unsafe extern "C" fn gh_deactivate(_p: *const clap_plugin) {}
unsafe extern "C" fn gh_start(_p: *const clap_plugin) -> bool {
    true
}
unsafe extern "C" fn gh_stop(_p: *const clap_plugin) {}
unsafe extern "C" fn gh_reset(_p: *const clap_plugin) {}

unsafe extern "C" fn gh_process(
    p: *const clap_plugin,
    process: *const clap_process,
) -> clap_process_status {
    unsafe {
        let s = gh(p);
        let frames = (*process).frames_count as usize;
        let out = &*(*process).audio_outputs;
        let inp = &*(*process).audio_inputs;
        for ch in 0..2 {
            let src = *inp.data32.add(ch) as *const f32;
            let dst = *out.data32.add(ch);
            for i in 0..frames {
                let x = *src.add(i);
                *dst.add(i) = x * s.gain + s.hiss;
            }
        }
    }
    CLAP_PROCESS_CONTINUE
}

fn gain_hiss(gain: f32, hiss: f32) -> PluginSlot {
    let inst = __instance_from_raw_for_test(
        |_host| {
            let state = Box::into_raw(Box::new(GainHiss { gain, hiss }));
            Box::into_raw(Box::new(clap_plugin {
                desc: ptr::null(),
                plugin_data: state as *mut c_void,
                init: Some(gh_init),
                destroy: Some(gh_destroy),
                activate: Some(gh_activate),
                deactivate: Some(gh_deactivate),
                start_processing: Some(gh_start),
                stop_processing: Some(gh_stop),
                reset: Some(gh_reset),
                process: Some(gh_process),
                get_extension: None,
                on_main_thread: None,
            })) as *const clap_plugin
        },
        SR,
    )
    .expect("fake return fx instance");
    PluginSlot::new(inst)
}

// ---------------------------------------------------------------------------
// Engine state
// ---------------------------------------------------------------------------

/// A return bus carrying the gain/hiss effect, at a non-unity fader.
fn add_return(shared: &SharedState, gain: f32, hiss: f32) {
    shared.edit_plugins(|p| p.insert(RETURN_FX, Arc::new(gain_hiss(gain, hiss))));
    let mut bus = Bus::new(RETURN, "Room".into());
    bus.plugin_ids.push(RETURN_FX);
    bus.set_is_return(true);
    bus.set_volume(0.8);
    shared.edit_busses(|b| {
        b.insert(RETURN, Arc::new(bus));
    });
}

fn send(from: TrackId, level_db: f32, pre_fader: bool) -> AuxSend {
    AuxSend {
        id: 1_000 + from as SendId,
        source: SendSource::Track(from),
        dest: RETURN,
        level_db,
        pre_fader,
        enabled: true,
    }
}

fn tone(frames: usize) -> Vec<f32> {
    (0..frames)
        .flat_map(|i| {
            let s = (i as f32 * 220.0 * std::f32::consts::TAU / SR as f32).sin() * 0.5;
            [s, s * 0.7]
        })
        .collect()
}

fn audio_clip(id: ClipId, track_id: TrackId, data: Vec<f32>) -> AudioClip {
    AudioClip {
        id,
        track_id,
        start_sample: 0,
        source: ClipSource::memory(data),
        name: "tone".into(),
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
        warp_algorithm: Default::default(),
        warp_markers: Vec::new(),
        tuning_render_cache: None,
    }
}

/// One audio track playing a tone, at a non-unity fader and off-centre
/// pan, so a send tapped at the wrong gain shows.
fn tone_track_state() -> (Arc<SharedState>, Arc<arc_swap::ArcSwap<TempoMap>>) {
    let shared = Arc::new(SharedState::default());
    let tempo_map = Arc::new(arc_swap::ArcSwap::from_pointee(TempoMap::default()));
    let track = Track::with_type(TRACK, "track".into(), TrackType::Audio);
    track.set_volume(0.6);
    track.set_pan(0.3);
    shared.edit_tracks(|m| {
        m.insert(TRACK, Arc::new(track));
    });
    shared.edit_clips(|c| c.push(Arc::new(audio_clip(1, TRACK, tone(SR as usize)))));
    (shared, tempo_map)
}

fn master(shared: &Arc<SharedState>, tempo_map: &Arc<arc_swap::ArcSwap<TempoMap>>) -> Vec<f32> {
    render_stem(
        StemSource::Master,
        0,
        FRAMES,
        shared,
        tempo_map,
        &AutomationSnapshot::default(),
        SR,
    )
    .expect("master render succeeds")
}

/// Freeze `track` through the real renderer and attach the cache to it,
/// as `SetTrackFrozenSource` does after a successful freeze.
fn freeze_and_attach(
    shared: &Arc<SharedState>,
    tempo_map: &Arc<arc_swap::ArcSwap<TempoMap>>,
    track: TrackId,
    name: &str,
) {
    let path = std::env::temp_dir().join(format!(
        "resonance_freeze_aux_{}_{name}.wav",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    let cache_ref = to_freeze_cache(
        track,
        path.to_string_lossy().into_owned(),
        shared,
        &AtomicBool::new(false),
        tempo_map,
        &AutomationSnapshot::default(),
        SR,
        &mut |_| {},
    )
    .expect("freeze must succeed");
    let reader = hound::WavReader::open(&path).expect("cache opens");
    let rate = reader.spec().sample_rate;
    let samples: Vec<f32> = reader
        .into_samples::<f32>()
        .collect::<Result<_, _>>()
        .expect("cache decodes");
    let _ = std::fs::remove_file(&path);
    let frames = (samples.len() / 2) as u64;
    let source = FrozenSource::new(cache_ref, Arc::new(samples), rate, frames);
    shared
        .tracks()
        .get(&track)
        .expect("frozen track")
        .frozen_source
        .store(Some(Arc::new(source)));
}

fn peak(s: &[f32]) -> f32 {
    s.iter().fold(0.0f32, |m, x| m.max(x.abs()))
}

/// Bit-for-bit equality, reporting the first difference instead of
/// dumping both renders.
fn assert_identical(a: &[f32], b: &[f32], what: &str) {
    assert_eq!(a.len(), b.len(), "{what}: lengths differ");
    if let Some(i) = a.iter().zip(b).position(|(x, y)| x.to_bits() != y.to_bits()) {
        panic!("{what}: first difference at sample {i}: {} vs {}", a[i], b[i]);
    }
}

fn assert_frozen_matches_unfrozen(pre_fader: bool) {
    let (shared, tempo_map) = tone_track_state();
    add_return(&shared, 0.5, 0.0);
    shared.aux_sends.store(Arc::new(vec![send(TRACK, -6.0, pre_fader)]));

    let unfrozen = master(&shared, &tempo_map);
    assert!(peak(&unfrozen) > 0.1, "reference render must be audible");
    let tag = if pre_fader { "pre" } else { "post" };
    freeze_and_attach(&shared, &tempo_map, TRACK, tag);
    let frozen = master(&shared, &tempo_map);

    // Bit-identical: the cache is the track's post-FX signal exactly as
    // the unfrozen track computes it, and the fader, pan and send then
    // run the same arithmetic on it.
    assert_identical(&unfrozen, &frozen, &format!("{tag}-fader send: frozen vs unfrozen"));
}

/// A post-fader send to a return carrying an effect: the frozen track
/// sounds exactly like the unfrozen one — the send is tapped once, at
/// the fader, not baked at unity into the cache and tapped again.
#[test]
fn a_frozen_tracks_post_fader_send_is_applied_once() {
    assert_frozen_matches_unfrozen(false);
}

/// The same for a pre-fader send.
#[test]
fn a_frozen_tracks_pre_fader_send_is_applied_once() {
    assert_frozen_matches_unfrozen(true);
}

/// A return whose effect makes sound on its own (a noise floor, a tail
/// from earlier input) must not land in a freeze cache: the live return
/// already plays it, so a cache carrying it plays it twice.
#[test]
fn a_self_noising_return_stays_out_of_the_cache() {
    let (shared, tempo_map) = tone_track_state();
    add_return(&shared, 0.5, 0.01);
    shared.aux_sends.store(Arc::new(vec![send(TRACK, -6.0, false)]));

    let unfrozen = master(&shared, &tempo_map);
    freeze_and_attach(&shared, &tempo_map, TRACK, "hiss");
    let frozen = master(&shared, &tempo_map);
    assert_identical(&unfrozen, &frozen, "the return's own output was baked into the cache");
}

// ---------------------------------------------------------------------------
// A multi-output kit
// ---------------------------------------------------------------------------

/// The multi-output harness kit, given a MIDI clip so the freeze has a
/// range to render (the fake instrument ignores the notes), plus the
/// self-noising return.
fn kit_state() -> multi_out_harness::EngineState {
    let state = multi_out_harness::EngineState::new();
    let clip = MidiClip {
        id: 1,
        track_id: multi_out_harness::PARENT,
        start_sample: 0,
        duration_ticks: 1_920 * 4,
        notes: Vec::new(),
        name: "kit".into(),
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    };
    state.shared.edit_midi_clips(|clips| clips.push(Arc::new(clip)));
    add_return(&state.shared, 0.5, 0.01);
    state
}

/// A frozen kit with a return in the project: the cache holds the kit's
/// taps and nothing of the return, so frozen and unfrozen are the same.
#[test]
fn a_frozen_kit_bakes_no_return_output() {
    let state = kit_state();
    let unfrozen = master(&state.shared, &state.tempo_map);
    assert!(peak(&unfrozen) > 0.1, "the kit must be audible");
    freeze_and_attach(&state.shared, &state.tempo_map, multi_out_harness::PARENT, "kit");
    let frozen = master(&state.shared, &state.tempo_map);
    assert_identical(&unfrozen, &frozen, "a frozen kit must not carry the return's output");
}

/// A kit tap's send cannot be replayed from a frozen kit: the cache is
/// the kit's summed taps, and no tap signal is left to send. The cache
/// carries no trace of the send either — not the dry signal it tapped,
/// not the return it fed — so the frozen kit sounds like the unfrozen
/// kit with that send switched off, never like the send applied twice.
#[test]
fn a_frozen_kits_tap_send_is_neither_baked_nor_doubled() {
    let state = kit_state();
    let tap_send = send(multi_out_harness::TAP_A, 0.0, false);

    let without_send = master(&state.shared, &state.tempo_map);
    state.shared.aux_sends.store(Arc::new(vec![tap_send]));
    let with_send = master(&state.shared, &state.tempo_map);
    assert!(
        peak(&with_send) > peak(&without_send) + 0.05,
        "the tap's send must be audible unfrozen"
    );

    freeze_and_attach(&state.shared, &state.tempo_map, multi_out_harness::PARENT, "kit_send");
    let frozen = master(&state.shared, &state.tempo_map);
    assert_identical(&frozen, &without_send, "a frozen kit plays its taps without the tap's send");
}
