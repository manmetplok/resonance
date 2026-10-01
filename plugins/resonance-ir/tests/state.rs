use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;
use resonance_ir::state::IrExtraState;
use resonance_plugin::plugin::ExtraStateSaver;

/// Direct round-trip through the `ExtraStateSaver` interface. This
/// simulates the audio-processor code path in the CLAP bridge: the
/// owned plugin has been moved into `ClapAudioProcessor`, so
/// `save()` on the bridge side talks to the cached saver Arc
/// instead of calling the plugin's trait default.
#[test]
fn extra_saver_roundtrip_active_path() {
    let src_path = Arc::new(Mutex::new(
        "/definitely/not/real/active_cab.wav".to_string(),
    ));
    let saver = IrExtraState {
        ir_path: src_path.clone(),
        file_list: Arc::new(Mutex::new(Vec::new())),
        load_request: Arc::new(AtomicI32::new(-1)),
        adopt: Arc::new(AtomicI32::new(-1)),
    };

    let mut json = serde_json::json!({ "params": {} });
    for (k, v) in saver.save() {
        json.as_object_mut().unwrap().insert(k, v);
    }

    let dst_path = Arc::new(Mutex::new(String::new()));
    let restored_saver = IrExtraState {
        ir_path: dst_path.clone(),
        file_list: Arc::new(Mutex::new(Vec::new())),
        load_request: Arc::new(AtomicI32::new(-1)),
        adopt: Arc::new(AtomicI32::new(-1)),
    };
    restored_saver.load(&json);

    assert_eq!(
        dst_path.lock().clone(),
        "/definitely/not/real/active_cab.wav",
        "ir_path should round-trip through the ExtraStateSaver"
    );
}

/// Restoring a project file that references an IR must leave the
/// plugin in a state where the persistent loader thread will
/// actually rebuild the convolver. The saver must populate all
/// three load-side side-effects: `ir_path`, `file_list`, and
/// `load_request`.
#[test]
fn extra_saver_load_populates_file_list_and_queues_loader() {
    let dir = std::env::temp_dir().join(format!(
        "resonance-ir-saver-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
    ));
    std::fs::create_dir_all(&dir).unwrap();

    let wav_a = dir.join("aaa_first.wav");
    let wav_b = dir.join("bbb_second.wav");
    let wav_c = dir.join("ccc_third.wav");
    for p in [&wav_a, &wav_b, &wav_c] {
        std::fs::write(p, b"").unwrap();
    }
    let target = wav_b.to_string_lossy().into_owned();

    let ir_path = Arc::new(Mutex::new(String::new()));
    let file_list = Arc::new(Mutex::new(Vec::<String>::new()));
    let load_request = Arc::new(AtomicI32::new(-1));

    let saver = IrExtraState {
        ir_path: ir_path.clone(),
        file_list: file_list.clone(),
        load_request: load_request.clone(),
        adopt: Arc::new(AtomicI32::new(-1)),
    };

    let state = serde_json::json!({
        "params": {},
        "ir_path": target,
    });
    saver.load(&state);

    assert_eq!(ir_path.lock().clone(), target);
    let files = file_list.lock().clone();
    assert_eq!(files.len(), 3, "all three .wav files should be listed");
    assert!(files.iter().any(|f| f == &target));
    let expected_idx = files.iter().position(|f| f == &target).unwrap() as i32;
    assert_eq!(load_request.load(Ordering::Acquire), expected_idx);

    let _ = std::fs::remove_dir_all(&dir);
}

/// A preset with `ir_path: ""` means "no IR": the saver asks the audio
/// thread to fade the current one out (it used to keep playing). A new
/// path's directory index is handed over too, so the excluded
/// `file_select` walks the new folder rather than keeping the old index.
#[test]
fn an_empty_ir_path_clears_and_a_new_one_moves_the_index() {
    use resonance_ir::state::{ADOPT_CLEAR, ADOPT_NONE};
    let dir = std::env::temp_dir().join(format!("resonance-ir-adopt-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for name in ["a.wav", "b.wav", "c.wav"] {
        std::fs::write(dir.join(name), b"").unwrap();
    }
    let adopt = Arc::new(AtomicI32::new(ADOPT_NONE));
    let saver = IrExtraState {
        ir_path: Arc::new(Mutex::new("/old/cab.wav".to_string())),
        file_list: Arc::new(Mutex::new(Vec::new())),
        load_request: Arc::new(AtomicI32::new(-1)),
        adopt: adopt.clone(),
    };
    let c = dir.join("c.wav").to_string_lossy().into_owned();
    saver.load(&serde_json::json!({"params": {}, "ir_path": c}));
    assert_eq!(adopt.load(Ordering::Acquire), 2, "c.wav is the third file");

    saver.load(&serde_json::json!({"params": {}, "ir_path": ""}));
    assert_eq!(adopt.load(Ordering::Acquire), ADOPT_CLEAR);
    assert!(saver.file_list.lock().is_empty());

    adopt.store(ADOPT_NONE, Ordering::Release);
    saver.load(&serde_json::json!({"params": {}}));
    assert_eq!(adopt.load(Ordering::Acquire), ADOPT_NONE, "no key: keep");
    let _ = std::fs::remove_dir_all(&dir);
}
