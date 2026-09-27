//! Offline-render fidelity (code review ENG-04 / ENG-06 / ENG-07 /
//! ENG-08 / ENG-13): what the export and freeze renderers write must be
//! what the project sounds like — no live-playback state leaking in, no
//! hard clip ahead of normalization, no truncated tails, automation
//! honoured, and a failed export never destroys the file it replaces.
//!
//! Drives the real renderers (`export_for_test` → `run_export`,
//! `to_freeze_cache`, `export_stems`) over engine state built by hand,
//! with a hand-rolled fake CLAP plugin (same `__instance_from_raw_for_test`
//! hook as `tests/clap_host/plugin_output_scrub.rs`) that is a single-tap echo with
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

use resonance_audio::test_support::{
    __instance_from_raw_for_test, export_for_test, export_stems, to_freeze_cache, AutomationSnapshot,
    PluginMap, PluginSlot, ResolvedParamLane, SharedState, StemBitDepth, StemSource, StemTarget,
    CLIP_DECLICK_FRAMES,
};
use resonance_audio::types::*;
use resonance_common::{AutomationLane, AutomationTarget, Breakpoint, CurveKind, FreezeCacheRef};

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
    /// When set, every `start_processing` fails (FU-M8b).
    refuse_start: bool,
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
unsafe extern "C" fn fx_start(p: *const clap_plugin) -> bool {
    !unsafe { fx(p) }.refuse_start
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
                refuse_start: false,
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
    clips: Arc<RwLock<Vec<AudioClip>>>,
    plugins: Arc<RwLock<PluginMap>>,
    tempo_map: Arc<arc_swap::ArcSwap<TempoMap>>,
}

impl Engine {
    /// One audio track (id 1) with `data` as a clip from sample 0.
    fn with_clip(data: Vec<f32>) -> Self {
        let e = Engine {
            shared: Arc::new(SharedState::default()),
            clips: Arc::new(RwLock::new(Vec::new())),
            plugins: Arc::new(RwLock::new(IndexMap::new())),
            tempo_map: Arc::new(arc_swap::ArcSwap::from_pointee(TempoMap::default())),
        };
        e.shared.edit_tracks(|m| {
            m.insert(1, std::sync::Arc::new(Track::with_type(1, "t".into(), TrackType::Audio)));
        });
        e.clips.write().push(audio_clip(data));
        e
    }

    fn add_master_fx(&self, slot: PluginSlot) {
        self.plugins.write().insert(FX_ID, slot);
        self.shared.edit_master(|master| master.plugin_ids.push(FX_ID));
    }

    fn add_track_fx(&self, slot: PluginSlot) {
        self.plugins.write().insert(FX_ID, slot);
        let _ = self.shared.tracks()[&1].push_plugin(FX_ID);
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
            &self.clips,
            &self.plugins,
            &self.tempo_map,
            automation,
            SR,
        )
    }
    fn freeze(&self, path: &Path, automation: &AutomationSnapshot) -> FreezeCacheRef {
        to_freeze_cache(
            1,
            path.to_string_lossy().into_owned(),
            &self.shared,
            &AtomicBool::new(false),
            &self.clips,
            &self.plugins,
            &self.tempo_map,
            automation,
            SR,
            &mut |_| {},
        )
        .expect("freeze must succeed")
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

// ---------------------------------------------------------------------------
// ENG-06 — normalized export keeps float headroom until the limiter
// ---------------------------------------------------------------------------

#[test]
fn normalized_export_does_not_hard_clip_overs_before_gain() {
    // A +6 dBFS sine (peak 2.0) with no master FX: normalizing it to
    // -14 LUFS needs ~-17 dB of gain, so the file has headroom to spare
    // and must be a clean, scaled sine — not a flat-topped one.
    let e = Engine::with_clip(tone(SR as usize, 2.0));
    let settings = ExportSettings {
        normalize: NormalizeSpec {
            enabled: true,
            mode: NormalizeMode::IntegratedLufs,
            target_db: -14.0,
            ceiling_dbtp: -1.0,
        },
        ..ExportSettings::default_wav()
    };
    let path = tmp("eng06");
    assert_completed(&e.export(&path, &settings, false));
    let out = read_f32_wav(&path);

    // Middle 0.5 s, away from the clip's start/end declicks.
    let mid = &out[(SR as usize / 4) * 2..(3 * SR as usize / 4) * 2];
    let pk = peak(mid);
    assert!(pk > 0.05, "normalized export must not be silent (peak {pk})");
    assert!(pk <= 10f32.powf(-1.0 / 20.0) + 1e-3, "peak {pk} above the ceiling");
    let rms = (mid.iter().map(|s| s * s).sum::<f32>() / mid.len() as f32).sqrt();
    let crest = pk / rms;
    assert!(
        (crest - std::f32::consts::SQRT_2).abs() < 0.02,
        "crest {crest}: a sine's is sqrt(2); lower means the overs were hard-clipped"
    );
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

// ---------------------------------------------------------------------------
// ENG-07 — master export renders the FX tail, same length as the stems
// ---------------------------------------------------------------------------

#[test]
fn master_export_keeps_fx_tail_and_matches_stem_length() {
    // A 250 ms clip into a 500 ms master echo: the echo sounds entirely
    // after the last clip ends, so a render that stops at the clip end
    // loses it.
    let clip_frames = SR as usize / 4;
    let delay = SR as usize / 2;
    let e = Engine::with_clip(tone(clip_frames, 0.25));
    let (slot, _state) = fake_fx(delay);
    e.add_master_fx(slot);

    let path = tmp("eng07");
    assert_completed(&e.export(&path, &ExportSettings::default_wav(), false));
    let out = read_f32_wav(&path);
    let frames = out.len() / 2;
    assert!(
        frames >= clip_frames + 2 * SR as usize,
        "export is {frames} frames: the tail past the last clip end was cut"
    );
    let echo = &out[delay * 2..(delay + clip_frames) * 2];
    assert!(peak(echo) > 0.1, "the echo after the last clip end must be in the file");

    // The master stem over the default range (with its FX tail) is the
    // same length as the master export.
    let stem_path = path.with_file_name("master_stem.wav");
    let (tx, rx) = crossbeam_channel::unbounded();
    export_stems(
        vec![StemTarget {
            source: StemSource::Master,
            path: stem_path.to_string_lossy().into_owned(),
        }],
        None,
        SR,
        StemBitDepth::Float32,
        true,
        &e.shared,
        &AtomicBool::new(false),
        &e.clips,
        &e.plugins,
        &e.tempo_map,
        SR,
        &tx,
    );
    drop(tx);
    let events: Vec<AudioEvent> = rx.try_iter().collect();
    assert!(
        events.iter().any(|e| matches!(e, AudioEvent::StemExportComplete { .. })),
        "stem export must complete: {events:?}"
    );
    assert_eq!(read_f32_wav(&stem_path).len(), out.len(), "stem and master lengths differ");
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

// ---------------------------------------------------------------------------
// ENG-08 — freeze bakes the track's plugin-parameter automation
// ---------------------------------------------------------------------------

/// A snapshot holding one plugin-param lane on the fake FX's gain,
/// ramping `from` → `to` across `frames`, mapped onto a `0..=1` range.
fn gain_ramp(from: f32, to: f32, frames: u64) -> AutomationSnapshot {
    let target = AutomationTarget::PluginParam {
        instance: FX_ID,
        param_id: GAIN_PARAM,
    };
    let lane = AutomationLane::new(
        1,
        target,
        vec![
            Breakpoint::new(0, from, CurveKind::Linear),
            Breakpoint::new(frames, to, CurveKind::Linear),
        ],
    );
    let mut snap = AutomationSnapshot::default();
    snap.plugin_params.insert(
        FX_ID,
        vec![ResolvedParamLane {
            param_id: GAIN_PARAM,
            lane,
            min: 0.0,
            max: 1.0,
        }],
    );
    snap
}

/// Mean absolute level of interleaved frames `[from, to)`.
fn level(s: &[f32], from: usize, to: usize) -> f32 {
    let w = &s[from * 2..to * 2];
    w.iter().map(|x| x.abs()).sum::<f32>() / w.len() as f32
}

#[test]
fn freeze_renders_the_tracks_plugin_automation() {
    let frames = SR as usize;
    let e = Engine::with_clip(tone(frames, 0.5));
    let (slot, _state) = fake_fx(0);
    e.add_track_fx(slot);

    let path = tmp("eng08");
    let flat = e.freeze(&path, &AutomationSnapshot::default());
    let ramp = gain_ramp(0.0, 1.0, frames as u64);
    let swept = e.freeze(&path, &ramp);
    let out = read_f32_wav(&path);

    // The cache follows the 0 → 1 gain sweep: near-silent at the start,
    // loud at the end.
    let head = level(&out, 0, frames / 10);
    let tail = level(&out, frames * 9 / 10, frames);
    assert!(tail > 0.2, "frozen audio must not be silent (end level {tail})");
    assert!(
        head < tail * 0.2,
        "the cache must carry the automated gain sweep (start {head}, end {tail})"
    );

    // The automation is part of what the cache was rendered from.
    assert_ne!(
        flat.render_fingerprint, swept.render_fingerprint,
        "an automation-lane change must change the freeze fingerprint"
    );
    let reswept = e.freeze(&path, &gain_ramp(0.0, 0.5, frames as u64));
    assert_ne!(swept.render_fingerprint, reswept.render_fingerprint);
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

// ---------------------------------------------------------------------------
// ENG-13 — a failed export never destroys the file it would replace
// ---------------------------------------------------------------------------

/// Every `*.partial` file left in `dir`.
fn partials(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "partial"))
        .collect()
}

#[test]
fn a_failed_export_leaves_the_previous_file_untouched() {
    let e = Engine::with_clip(tone(SR as usize / 4, 0.25));
    let path = tmp("eng13");
    let dir = path.parent().unwrap().to_path_buf();
    std::fs::write(&path, b"previous good export").unwrap();

    // A render that stops before finishing (the cancel path stands in for
    // any mid-render failure: every one of them used to have truncated
    // the target already, when the sink was built).
    let events = e.export(&path, &ExportSettings::default_wav(), true);
    assert!(
        events.iter().any(|e| matches!(
            e,
            AudioEvent::ExportError {
                kind: ExportErrorKind::Cancelled,
                ..
            }
        )),
        "export must report the cancel: {events:?}"
    );
    assert_eq!(
        std::fs::read(&path).expect("the previous file must still exist"),
        b"previous good export",
        "a failed export must not touch the previous file"
    );
    assert!(partials(&dir).is_empty(), "no temp file may be left behind");

    // A successful export replaces it, and leaves no temp file either.
    assert_completed(&e.export(&path, &ExportSettings::default_wav(), false));
    assert!(peak(&read_f32_wav(&path)) > 0.1, "the new export is in place");
    assert!(partials(&dir).is_empty(), "no temp file may be left behind");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// FU-M8b — a plugin an export's reset leaves dead is reported, once
// ---------------------------------------------------------------------------

#[test]
fn a_plugin_that_will_not_restart_after_the_export_reset_is_reported_once() {
    let e = Engine::with_clip(tone(SR as usize / 10, 0.25));
    let (slot, state) = fake_fx(0);
    e.add_master_fx(slot);
    unsafe { (*state).refuse_start = true };

    let path = tmp("fu-m8b");
    assert_completed(&e.export(&path, &ExportSettings::default_wav(), false));
    assert_eq!(
        std::mem::take(&mut *e.shared.plugins_dead_after_reset.lock()),
        vec![FX_ID],
        "the dead plugin is queued for the engine loop's error report"
    );

    // Still dead at the next export: not reported again.
    assert_completed(&e.export(&path, &ExportSettings::default_wav(), false));
    assert!(e.shared.plugins_dead_after_reset.lock().is_empty());
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}
