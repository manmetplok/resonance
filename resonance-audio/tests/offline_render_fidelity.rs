//! Offline-render fidelity (code review ENG-04 / ENG-06 / ENG-07 /
//! ENG-08 / ENG-13): what the export and freeze renderers write must be
//! what the project sounds like — no live-playback state leaking in, no
//! hard clip ahead of normalization, no truncated tails, automation
//! honoured, and a failed export never destroys the file it replaces.
//!
//! Drives the real renderers (`export_for_test` → `run_export`,
//! `to_freeze_cache`, `export_stems`) over engine state built by hand,
//! with a hand-rolled fake CLAP plugin (same `__instance_from_raw_for_test`
//! hook as `tests/plugin_output_scrub.rs`) that is a single-tap echo with
//! an automatable gain parameter, and whose `reset` clears the echo line.

use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use clap_sys::events::{clap_event_param_value, clap_input_events, CLAP_EVENT_PARAM_VALUE};
use clap_sys::plugin::clap_plugin;
use clap_sys::process::{clap_process, clap_process_status, CLAP_PROCESS_CONTINUE};
use indexmap::IndexMap;
use parking_lot::RwLock;

use resonance_audio::__test_support::{
    __instance_from_raw_for_test, export_for_test, AutomationSnapshot, PluginMap, PluginSlot,
    SharedState, CLIP_DECLICK_FRAMES,
};
use resonance_audio::types::*;

const SR: u32 = 48_000;
const FX_ID: PluginInstanceId = 900;
const GAIN_PARAM: u32 = 0;

// ---------------------------------------------------------------------------
// Fake plugin: echo (out = in + in[n - delay]) followed by a gain param.
// ---------------------------------------------------------------------------

struct FakeFx {
    delay: usize,
    ring: [Vec<f32>; 2],
    pos: usize,
    gain: f32,
    reset_calls: u32,
}

unsafe fn fx<'a>(plugin: *const clap_plugin) -> &'a mut FakeFx {
    unsafe { &mut *((*plugin).plugin_data as *mut FakeFx) }
}

unsafe extern "C" fn fx_init(_p: *const clap_plugin) -> bool {
    true
}
unsafe extern "C" fn fx_destroy(_p: *const clap_plugin) {}
unsafe extern "C" fn fx_activate(_p: *const clap_plugin, _sr: f64, _min: u32, _max: u32) -> bool {
    true
}
unsafe extern "C" fn fx_deactivate(_p: *const clap_plugin) {}
unsafe extern "C" fn fx_start(_p: *const clap_plugin) -> bool {
    true
}
unsafe extern "C" fn fx_stop(_p: *const clap_plugin) {}

unsafe extern "C" fn fx_reset(p: *const clap_plugin) {
    let s = unsafe { fx(p) };
    s.reset_calls += 1;
    for ch in &mut s.ring {
        ch.fill(0.0);
    }
    s.pos = 0;
}

unsafe fn apply_param_events(s: &mut FakeFx, events: *const clap_input_events) {
    if events.is_null() {
        return;
    }
    unsafe {
        let (Some(size), Some(get)) = ((*events).size, (*events).get) else {
            return;
        };
        for i in 0..size(events) {
            let h = get(events, i);
            if h.is_null() || (*h).type_ != CLAP_EVENT_PARAM_VALUE {
                continue;
            }
            let ev = &*(h as *const clap_event_param_value);
            if ev.param_id == GAIN_PARAM {
                s.gain = ev.value as f32;
            }
        }
    }
}

unsafe extern "C" fn fx_process(
    p: *const clap_plugin,
    process: *const clap_process,
) -> clap_process_status {
    unsafe {
        let s = fx(p);
        apply_param_events(s, (*process).in_events);
        let frames = (*process).frames_count as usize;
        let out = &*(*process).audio_outputs;
        let inp = &*(*process).audio_inputs;
        let bufs: Vec<(*const f32, *mut f32)> = (0..2)
            .map(|ch| (*inp.data32.add(ch) as *const f32, *out.data32.add(ch)))
            .collect();
        let start = s.pos;
        for (ch, (src, dst)) in bufs.into_iter().enumerate() {
            let mut pos = start;
            for i in 0..frames {
                // In-place host buffers: read the input before writing.
                let x = *src.add(i);
                let y = if s.delay > 0 {
                    let echo = s.ring[ch][pos];
                    s.ring[ch][pos] = x;
                    pos = (pos + 1) % s.delay;
                    x + echo
                } else {
                    x
                };
                *dst.add(i) = y * s.gain;
            }
            if ch == 1 {
                s.pos = pos;
            }
        }
    }
    CLAP_PROCESS_CONTINUE
}

/// A fake echo/gain effect. The state is leaked (the instance's `Drop`
/// still dereferences it); the raw pointer lets a test inspect it.
fn fake_fx(delay: usize) -> (PluginSlot, *mut FakeFx) {
    let mut state_ptr: *mut FakeFx = ptr::null_mut();
    let inst = __instance_from_raw_for_test(
        |_host| {
            let state = Box::into_raw(Box::new(FakeFx {
                delay,
                ring: [vec![0.0; delay.max(1)], vec![0.0; delay.max(1)]],
                pos: 0,
                gain: 1.0,
                reset_calls: 0,
            }));
            state_ptr = state;
            Box::into_raw(Box::new(clap_plugin {
                desc: ptr::null(),
                plugin_data: state as *mut c_void,
                init: Some(fx_init),
                destroy: Some(fx_destroy),
                activate: Some(fx_activate),
                deactivate: Some(fx_deactivate),
                start_processing: Some(fx_start),
                stop_processing: Some(fx_stop),
                reset: Some(fx_reset),
                process: Some(fx_process),
                get_extension: None,
                on_main_thread: None,
            })) as *const clap_plugin
        },
        SR,
    )
    .expect("fake fx instance");
    (PluginSlot::new(inst), state_ptr)
}

// ---------------------------------------------------------------------------
// Engine state
// ---------------------------------------------------------------------------

struct Engine {
    shared: Arc<SharedState>,
    tracks: Arc<RwLock<IndexMap<TrackId, Track>>>,
    busses: Arc<RwLock<IndexMap<BusId, Bus>>>,
    master: Arc<RwLock<MasterBus>>,
    clips: Arc<RwLock<Vec<AudioClip>>>,
    midi_clips: Arc<RwLock<Vec<MidiClip>>>,
    plugins: Arc<RwLock<PluginMap>>,
    tempo_map: Arc<arc_swap::ArcSwap<TempoMap>>,
}

impl Engine {
    /// One audio track (id 1) with `data` as a clip from sample 0.
    fn with_clip(data: Vec<f32>) -> Self {
        let e = Engine {
            shared: Arc::new(SharedState::default()),
            tracks: Arc::new(RwLock::new(IndexMap::new())),
            busses: Arc::new(RwLock::new(IndexMap::new())),
            master: Arc::new(RwLock::new(MasterBus::new())),
            clips: Arc::new(RwLock::new(Vec::new())),
            midi_clips: Arc::new(RwLock::new(Vec::new())),
            plugins: Arc::new(RwLock::new(IndexMap::new())),
            tempo_map: Arc::new(arc_swap::ArcSwap::from_pointee(TempoMap::default())),
        };
        e.tracks
            .write()
            .insert(1, Track::with_type(1, "t".into(), TrackType::Audio));
        e.clips.write().push(audio_clip(data));
        e
    }

    fn add_master_fx(&self, slot: PluginSlot) {
        self.plugins.write().insert(FX_ID, slot);
        self.master.write().plugin_ids.push(FX_ID);
    }

    #[allow(dead_code)]
    fn add_track_fx(&self, slot: PluginSlot) {
        self.plugins.write().insert(FX_ID, slot);
        let _ = self.tracks.read()[&1].push_plugin(FX_ID);
    }

    fn export(&self, path: &Path, settings: &ExportSettings, cancel: bool) -> Vec<AudioEvent> {
        self.export_with(path, settings, cancel, &AutomationSnapshot::default())
    }

    fn export_with(
        &self,
        path: &Path,
        settings: &ExportSettings,
        cancel: bool,
        automation: &AutomationSnapshot,
    ) -> Vec<AudioEvent> {
        export_for_test(
            path.to_string_lossy().into_owned(),
            settings,
            &AtomicBool::new(cancel),
            &self.shared,
            &self.tracks,
            &self.busses,
            &self.master,
            &self.clips,
            &self.midi_clips,
            &self.plugins,
            &self.tempo_map,
            automation,
            SR,
        )
    }
}

fn audio_clip(data: Vec<f32>) -> AudioClip {
    AudioClip {
        id: 1,
        track_id: 1,
        start_sample: 0,
        source: ClipSource::Memory(data),
        name: "src".into(),
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

/// Stereo-interleaved 220 Hz sine, `frames` long at `amp`.
fn tone(frames: usize, amp: f32) -> Vec<f32> {
    (0..frames)
        .flat_map(|i| {
            let s = (i as f32 * 220.0 * std::f32::consts::TAU / SR as f32).sin() * amp;
            [s, s]
        })
        .collect()
}

fn read_f32_wav(path: &Path) -> Vec<f32> {
    hound::WavReader::open(path)
        .expect("wav opens")
        .into_samples::<f32>()
        .collect::<Result<Vec<_>, _>>()
        .expect("wav decodes")
}

fn tmp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "resonance-offline-fidelity-{}-{name}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("out.wav")
}

fn peak(s: &[f32]) -> f32 {
    s.iter().fold(0.0f32, |m, x| m.max(x.abs()))
}

fn assert_completed(events: &[AudioEvent]) {
    assert!(
        events.iter().any(|e| matches!(e, AudioEvent::ExportComplete { .. })),
        "export must complete: {events:?}"
    );
}

// ---------------------------------------------------------------------------
// ENG-04 — offline renders start from a CLAP-reset plugin state
// ---------------------------------------------------------------------------

#[test]
fn export_does_not_inherit_live_playback_tails() {
    const DELAY: usize = 4_800; // 100 ms echo
    let src = tone(SR as usize / 2, 0.25);
    let e = Engine::with_clip(src.clone());
    let (slot, state) = fake_fx(DELAY);
    // "Live playback" before the export: a loud burst through the master
    // echo leaves its tail sitting in the delay line.
    {
        let mut inst = slot.lock();
        let mut l = vec![0.9f32; 1024];
        let mut r = vec![0.9f32; 1024];
        inst.0.process(&mut l, &mut r, 1024);
    }
    e.add_master_fx(slot);

    let path = tmp("eng04");
    assert_completed(&e.export(&path, &ExportSettings::default_wav(), false));
    let out = read_f32_wav(&path);

    // Within the first echo period the file is exactly the dry clip: no
    // echo of the pre-export burst.
    let first = &out[..DELAY * 2];
    assert!(peak(first) > 0.1, "export must not be silent");
    // (The clip's own start declick is skipped: it is not the echo.)
    let skip = CLIP_DECLICK_FRAMES as usize * 2;
    let leak = first[skip..]
        .iter()
        .zip(&src[skip..])
        .fold(0.0f32, |m, (a, b)| m.max((a - b).abs()));
    assert!(leak < 1e-6, "live-playback echo leaked into the export (max diff {leak})");
    assert!(unsafe { (*state).reset_calls } >= 1, "export must CLAP-reset every plugin");
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}
