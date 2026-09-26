//! The reference A/B block survives save → load through the real
//! `save_project` / `load_project` / `replay_loaded_project` chain
//! (ARCH-01 A-5).
//!
//! The undo snapshot restores reference *content* — the loaded files
//! (path, name, cached loudness, markers), the active selection,
//! loudness-match and trim — from `ProjectFile::references` /
//! `reference_settings`, so those fields must round-trip through disk
//! exactly. The *monitor* state (A/B source, loop-to-mix) is not undo
//! state but is still remembered per project, so a reload brings it back
//! too. A reference whose analysis never finished carries a `-inf`
//! loudness, which JSON writes as `null`; that has to load again.

use std::collections::HashMap;
use std::path::PathBuf;

use resonance_app::message::Message;
use resonance_app::project::{load_project, save_project, LoadedProject};
use resonance_app::reference::{ReferenceMessage, ReferenceStatus};
use resonance_app::Resonance;
use resonance_audio::types::{ABSource, AudioEvent, ReferenceAnalysisStage, ReferenceId};

fn project_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "resonance-reference-disk-{tag}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create project dir");
    dir
}

fn touch(dir: &std::path::Path, name: &str) -> String {
    let path = dir.join(format!("{name}.wav"));
    std::fs::write(&path, b"").expect("write reference file");
    path.to_string_lossy().into_owned()
}

fn send(app: &mut Resonance, m: ReferenceMessage) {
    app.test_dispatch(Message::Reference(m));
}

#[test]
fn reference_content_and_monitor_state_round_trip_through_disk() {
    let dir = project_dir("round-trip");
    let refs = dir.join("refs");
    std::fs::create_dir_all(&refs).unwrap();
    let (a, b) = (touch(&refs, "a"), touch(&refs, "b"));

    let (mut authored, _task) = Resonance::new_for_test();
    authored.test_apply_engine_event(AudioEvent::ReferenceLoaded {
        id: ReferenceId(1),
        name: "kick-ref".into(),
        path: a.clone(),
        integrated_lufs: -9.5,
        waveform_peaks: vec![(-0.5, 0.5)],
        length_samples: 480_000,
    });
    // Still analysing when saved: no measured loudness yet.
    authored.test_reference_push_pending(&b);
    authored.test_apply_engine_event(AudioEvent::ReferenceAnalysisProgress {
        id: ReferenceId(2),
        stage: ReferenceAnalysisStage::Decoding,
    });
    authored.test_apply_engine_event(AudioEvent::RefMarkerAdded {
        ref_id: ReferenceId(1),
        marker_id: 3,
        position_samples: 96_000,
        label: "chorus".into(),
    });
    send(&mut authored, ReferenceMessage::SetActive(ReferenceId(2)));
    send(&mut authored, ReferenceMessage::ToggleLoudnessMatch);
    send(&mut authored, ReferenceMessage::TrimChanged(-4.5));
    send(&mut authored, ReferenceMessage::ToggleAbSource);
    send(&mut authored, ReferenceMessage::ToggleLoopToMix);

    let file = authored.test_build_project_file();
    assert_eq!(file.references.len(), 2);
    assert_eq!(file.references[1].integrated_lufs, f32::NEG_INFINITY);
    save_project(&dir, &file, &[], &[]).expect("save project");

    let loaded = load_project(&dir).expect("a project with an unanalysed reference loads");
    assert_eq!(
        loaded.file.references, file.references,
        "reference entries survive the disk"
    );
    assert_eq!(
        loaded.file.reference_settings, file.reference_settings,
        "reference settings survive the disk"
    );

    let (mut reopened, _task) = Resonance::new_for_test();
    reopened.test_replay_loaded_project_from(LoadedProject {
        file: loaded.file,
        project_dir: dir.clone(),
        midi_notes: HashMap::new(),
        plugin_states: HashMap::new(),
    });
    let st = reopened.test_reference();
    assert_eq!(st.entries.len(), 2);
    assert!(st
        .entries
        .iter()
        .all(|e| matches!(e.status, ReferenceStatus::Analyzing(_))));
    assert_eq!(st.entries[0].markers.len(), 1, "marker survives");
    assert_eq!(st.active_id, Some(st.entries[1].id), "selection survives");
    assert!(st.loudness_match, "loudness match survives");
    assert_eq!(st.trim_db, -4.5, "trim survives");
    assert_eq!(st.monitor.ab_source, ABSource::Reference, "A/B source is remembered");
    assert!(st.monitor.loop_to_mix, "loop-to-mix is remembered");
    assert_eq!(
        reopened.test_build_project_file().references,
        file.references,
        "the reopened project writes the same reference block"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
