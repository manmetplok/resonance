//! CLAP `render.set` around every offline render (review finding 6): each
//! plugin is told OFFLINE before the render's first block and REALTIME
//! after its last, on every way out — completion, cancel, an export that
//! cannot write. A plugin that trades quality for time in realtime (the
//! drums stream long takes from disk and drop a late read) must not do so
//! in a bounce.

use std::ffi::{c_char, c_void, CStr};
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use clap_sys::ext::render::{
    clap_plugin_render, clap_plugin_render_mode, CLAP_EXT_RENDER, CLAP_RENDER_OFFLINE,
    CLAP_RENDER_REALTIME,
};
use clap_sys::plugin::clap_plugin;
use clap_sys::process::{clap_process, clap_process_status, CLAP_PROCESS_CONTINUE};

use resonance_audio::test_support::{
    __instance_from_raw_for_test, export_for_test, export_stems, to_freeze_cache,
    AutomationSnapshot, PluginSlot, SharedState, StemBitDepth, StemSource, StemTarget,
};
use resonance_audio::types::*;

const SR: u32 = 48_000;
const FX_ID: PluginInstanceId = 910;

#[derive(Default)]
struct ModeFx {
    /// Every `render.set`, in order: `true` = offline.
    sets: Vec<bool>,
    offline: bool,
    /// Blocks processed, and how many of them while told offline.
    blocks: u32,
    offline_blocks: u32,
}

unsafe fn fx<'a>(plugin: *const clap_plugin) -> &'a mut ModeFx {
    unsafe { &mut *((*plugin).plugin_data as *mut ModeFx) }
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
unsafe extern "C" fn fx_reset(_p: *const clap_plugin) {}
unsafe extern "C" fn fx_process(
    p: *const clap_plugin,
    _process: *const clap_process,
) -> clap_process_status {
    let s = unsafe { fx(p) };
    s.blocks += 1;
    if s.offline {
        s.offline_blocks += 1;
    }
    CLAP_PROCESS_CONTINUE
}

unsafe extern "C" fn fx_render_set(p: *const clap_plugin, mode: clap_plugin_render_mode) -> bool {
    let s = unsafe { fx(p) };
    let offline = match mode {
        CLAP_RENDER_OFFLINE => true,
        CLAP_RENDER_REALTIME => false,
        _ => return false,
    };
    s.sets.push(offline);
    s.offline = offline;
    true
}
unsafe extern "C" fn fx_hard_realtime(_p: *const clap_plugin) -> bool {
    false
}

static FX_RENDER: clap_plugin_render = clap_plugin_render {
    has_hard_realtime_requirement: Some(fx_hard_realtime),
    set: Some(fx_render_set),
};

unsafe extern "C" fn fx_get_extension(_p: *const clap_plugin, id: *const c_char) -> *const c_void {
    if unsafe { CStr::from_ptr(id) } == CLAP_EXT_RENDER {
        return &FX_RENDER as *const clap_plugin_render as *const c_void;
    }
    ptr::null()
}

fn mode_fx() -> (PluginSlot, *mut ModeFx) {
    let mut state_ptr: *mut ModeFx = ptr::null_mut();
    let inst = __instance_from_raw_for_test(
        |_host| {
            let state = Box::into_raw(Box::new(ModeFx::default()));
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
                get_extension: Some(fx_get_extension),
                on_main_thread: None,
            })) as *const clap_plugin
        },
        SR,
    )
    .expect("fake fx instance");
    (PluginSlot::new(inst), state_ptr)
}

struct Engine {
    shared: Arc<SharedState>,
    tempo_map: Arc<arc_swap::ArcSwap<TempoMap>>,
    state: *mut ModeFx,
}

impl Engine {
    /// One audio track (id 1) with a short clip, through the fake FX.
    fn new() -> Self {
        let shared = Arc::new(SharedState::default());
        shared.edit_tracks(|m| {
            m.insert(
                1,
                Arc::new(Track::with_type(1, "t".into(), TrackType::Audio)),
            );
        });
        let data: Vec<f32> = (0..SR as usize / 10).flat_map(|_| [0.1, 0.1]).collect();
        shared.edit_clips(|c| c.push(Arc::new(audio_clip(data))));
        let (slot, state) = mode_fx();
        shared.edit_plugins(|p| p.insert(FX_ID, Arc::new(slot)));
        let _ = shared.tracks()[&1].push_plugin(FX_ID);
        Self {
            shared,
            tempo_map: Arc::new(arc_swap::ArcSwap::from_pointee(TempoMap::default())),
            state,
        }
    }

    fn fx(&self) -> &ModeFx {
        unsafe { &*self.state }
    }

    fn export(&self, path: &Path, cancel: bool) -> Vec<AudioEvent> {
        export_for_test(
            path.to_string_lossy().into_owned(),
            &ExportSettings::default_wav(),
            &AtomicBool::new(cancel),
            &self.shared,
            &self.tempo_map,
            &AutomationSnapshot::default(),
            SR,
        )
    }
}

fn audio_clip(data: Vec<f32>) -> AudioClip {
    AudioClip {
        id: 1,
        track_id: 1,
        start_sample: 0,
        source: ClipSource::memory(data),
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

fn tmp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "resonance-offline-render-mode-{}-{name}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("out.wav")
}

#[test]
fn an_export_renders_every_block_offline_and_ends_realtime() {
    let e = Engine::new();
    let path = tmp("export");
    let events = e.export(&path, false);
    assert!(
        events
            .iter()
            .any(|ev| matches!(ev, AudioEvent::ExportComplete { .. })),
        "{events:?}"
    );
    assert_eq!(
        e.fx().sets,
        vec![true, false],
        "offline for the render, then realtime"
    );
    assert!(e.fx().blocks > 0);
    assert_eq!(
        e.fx().offline_blocks,
        e.fx().blocks,
        "every block rendered offline"
    );
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn a_cancelled_export_still_ends_realtime() {
    let e = Engine::new();
    let path = tmp("cancel");
    let events = e.export(&path, true);
    assert!(
        events.iter().any(|ev| matches!(
            ev,
            AudioEvent::ExportError {
                kind: ExportErrorKind::Cancelled,
                ..
            }
        )),
        "{events:?}"
    );
    assert_eq!(
        e.fx().sets.last(),
        Some(&false),
        "back to realtime: {:?}",
        e.fx().sets
    );
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn an_export_that_cannot_write_leaves_the_plugin_realtime() {
    let e = Engine::new();
    let path = std::env::temp_dir()
        .join(format!("resonance-no-such-dir-{}", std::process::id()))
        .join("deeper")
        .join("out.wav");
    let events = e.export(&path, false);
    assert!(
        events
            .iter()
            .any(|ev| matches!(ev, AudioEvent::ExportError { .. })),
        "{events:?}"
    );
    assert!(!e.fx().offline, "never left offline: {:?}", e.fx().sets);
}

#[test]
fn a_freeze_and_a_stem_render_are_offline_too() {
    let e = Engine::new();
    let path = tmp("freeze");
    to_freeze_cache(
        1,
        path.to_string_lossy().into_owned(),
        &e.shared,
        &AtomicBool::new(false),
        &e.tempo_map,
        &AutomationSnapshot::default(),
        SR,
        &mut |_| {},
    )
    .expect("freeze must succeed");
    assert_eq!(e.fx().sets, vec![true, false]);
    assert_eq!(e.fx().offline_blocks, e.fx().blocks);

    let stem_path = path.with_file_name("stem.wav");
    let (tx, _rx) = crossbeam_channel::unbounded();
    export_stems(
        vec![StemTarget {
            source: StemSource::Track(1),
            path: stem_path.to_string_lossy().into_owned(),
        }],
        None,
        SR,
        StemBitDepth::Float32,
        true,
        &e.shared,
        &AtomicBool::new(false),
        &e.tempo_map,
        &AutomationSnapshot::default(),
        SR,
        &tx,
    );
    assert_eq!(e.fx().sets, vec![true, false, true, false]);
    assert_eq!(e.fx().offline_blocks, e.fx().blocks);
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}
