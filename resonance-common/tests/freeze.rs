use resonance_common::freeze::{
    compute_fingerprint, FreezeCacheRef, FreezeCacheStatus, FreezeFingerprintBuilder,
    FreezeFingerprintInputs, TrackFreezeState,
};

// --- FreezeCacheRef tests ---

#[test]
fn freeze_cache_ref_serialize_deserialize() {
    let r#ref = FreezeCacheRef::new(
        "track_0_freeze.wav".to_string(),
        44100,
        32,
        123456789,
        FreezeCacheStatus::Frozen,
    );
    let json = serde_json::to_string(&r#ref).expect("serialize");
    let back: FreezeCacheRef = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(r#ref, back);
}

#[test]
fn freeze_cache_ref_status_variants() {
    for status in [
        FreezeCacheStatus::Frozen,
        FreezeCacheStatus::Stale,
        FreezeCacheStatus::Failed,
    ] {
        let r#ref = FreezeCacheRef::new(
            "test.wav".to_string(),
            48000,
            24,
            0,
            status,
        );
        assert_eq!(r#ref.status, status);
        match status {
            FreezeCacheStatus::Frozen => {
                assert!(r#ref.is_valid());
                assert!(!r#ref.is_stale());
                assert!(!r#ref.is_failed());
            }
            FreezeCacheStatus::Stale => {
                assert!(!r#ref.is_valid());
                assert!(r#ref.is_stale());
                assert!(!r#ref.is_failed());
            }
            FreezeCacheStatus::Failed => {
                assert!(!r#ref.is_valid());
                assert!(!r#ref.is_stale());
                assert!(r#ref.is_failed());
            }
        }
    }
}

#[test]
fn freeze_cache_status_serialize_deserialize() {
    for status in [
        FreezeCacheStatus::Frozen,
        FreezeCacheStatus::Stale,
        FreezeCacheStatus::Failed,
    ] {
        let json = serde_json::to_string(&status).expect("serialize");
        let back: FreezeCacheStatus = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(status, back);
    }
}

// --- TrackFreezeState tests ---

#[test]
fn track_freeze_state_unfrozen() {
    let state = TrackFreezeState::unfrozen();
    assert!(!state.is_frozen);
    assert!(state.cache_ref.is_none());
    assert!(!state.is_validly_frozen());
    assert!(!state.is_stale());
}

#[test]
fn track_freeze_state_frozen_valid() {
    let cache_ref = FreezeCacheRef::new(
        "test.wav".to_string(),
        44100,
        32,
        123,
        FreezeCacheStatus::Frozen,
    );
    let state = TrackFreezeState::frozen(cache_ref.clone());
    assert!(state.is_frozen);
    assert!(state.cache_ref.is_some());
    assert!(state.is_validly_frozen());
    assert!(!state.is_stale());
    assert_eq!(state.as_ref(), Some(&cache_ref));
}

#[test]
fn track_freeze_state_frozen_stale() {
    let cache_ref = FreezeCacheRef::new(
        "test.wav".to_string(),
        44100,
        32,
        123,
        FreezeCacheStatus::Stale,
    );
    let state = TrackFreezeState::frozen(cache_ref);
    assert!(state.is_frozen);
    assert!(state.is_stale());
    assert!(!state.is_validly_frozen());
}

#[test]
fn track_freeze_state_serialize_deserialize() {
    let cache_ref = FreezeCacheRef::new(
        "test.wav".to_string(),
        44100,
        32,
        123,
        FreezeCacheStatus::Frozen,
    );
    let state = TrackFreezeState::frozen(cache_ref.clone());
    let json = serde_json::to_string(&state).expect("serialize");
    let back: TrackFreezeState = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(state, back);
}

#[test]
fn track_freeze_state_default() {
    let state: TrackFreezeState = Default::default();
    assert!(!state.is_frozen);
    assert!(state.cache_ref.is_none());
}

// --- Fingerprint tests ---

#[test]
fn fingerprint_same_inputs_same_hash() {
    let inputs = FreezeFingerprintInputs {
        notes: b"note_data".to_vec(),
        lyrics: b"lyric_data".to_vec(),
        plugin_params: b"param_data".to_vec(),
        instrument_id: "piano".to_string(),
    };
    let hash1 = compute_fingerprint(&inputs);
    let hash2 = compute_fingerprint(&inputs);
    assert_eq!(hash1, hash2, "Same inputs should produce same fingerprint");
}

#[test]
fn fingerprint_different_inputs_different_hash() {
    let inputs1 = FreezeFingerprintInputs {
        notes: b"note_data".to_vec(),
        lyrics: b"lyric_data".to_vec(),
        plugin_params: b"param_data".to_vec(),
        instrument_id: "piano".to_string(),
    };
    let inputs2 = FreezeFingerprintInputs {
        notes: b"different_note_data".to_vec(),
        lyrics: b"lyric_data".to_vec(),
        plugin_params: b"param_data".to_vec(),
        instrument_id: "piano".to_string(),
    };
    let hash1 = compute_fingerprint(&inputs1);
    let hash2 = compute_fingerprint(&inputs2);
    assert_ne!(
        hash1, hash2,
        "Different inputs should produce different fingerprints"
    );
}

#[test]
fn fingerprint_changed_lyrics_different_hash() {
    let inputs1 = FreezeFingerprintInputs {
        notes: b"note_data".to_vec(),
        lyrics: b"lyric_data".to_vec(),
        plugin_params: b"param_data".to_vec(),
        instrument_id: "piano".to_string(),
    };
    let inputs2 = FreezeFingerprintInputs {
        notes: b"note_data".to_vec(),
        lyrics: b"different_lyrics".to_vec(),
        plugin_params: b"param_data".to_vec(),
        instrument_id: "piano".to_string(),
    };
    let hash1 = compute_fingerprint(&inputs1);
    let hash2 = compute_fingerprint(&inputs2);
    assert_ne!(
        hash1, hash2,
        "Changed lyrics should produce different fingerprint"
    );
}

#[test]
fn fingerprint_changed_plugin_params_different_hash() {
    let inputs1 = FreezeFingerprintInputs {
        notes: b"note_data".to_vec(),
        lyrics: b"lyric_data".to_vec(),
        plugin_params: b"param_data".to_vec(),
        instrument_id: "piano".to_string(),
    };
    let inputs2 = FreezeFingerprintInputs {
        notes: b"note_data".to_vec(),
        lyrics: b"lyric_data".to_vec(),
        plugin_params: b"different_params".to_vec(),
        instrument_id: "piano".to_string(),
    };
    let hash1 = compute_fingerprint(&inputs1);
    let hash2 = compute_fingerprint(&inputs2);
    assert_ne!(
        hash1, hash2,
        "Changed plugin params should produce different fingerprint"
    );
}

#[test]
fn fingerprint_changed_instrument_different_hash() {
    let inputs1 = FreezeFingerprintInputs {
        notes: b"note_data".to_vec(),
        lyrics: b"lyric_data".to_vec(),
        plugin_params: b"param_data".to_vec(),
        instrument_id: "piano".to_string(),
    };
    let inputs2 = FreezeFingerprintInputs {
        notes: b"note_data".to_vec(),
        lyrics: b"lyric_data".to_vec(),
        plugin_params: b"param_data".to_vec(),
        instrument_id: "guitar".to_string(),
    };
    let hash1 = compute_fingerprint(&inputs1);
    let hash2 = compute_fingerprint(&inputs2);
    assert_ne!(
        hash1, hash2,
        "Changed instrument should produce different fingerprint"
    );
}

#[test]
fn fingerprint_builder() {
    let inputs = FreezeFingerprintBuilder::new()
        .with_notes(b"notes".to_vec())
        .with_lyrics(b"lyrics".to_vec())
        .with_plugin_params(b"params".to_vec())
        .with_instrument_id("synth")
        .build();

    assert_eq!(inputs.notes, b"notes");
    assert_eq!(inputs.lyrics, b"lyrics");
    assert_eq!(inputs.plugin_params, b"params");
    assert_eq!(inputs.instrument_id, "synth");
}

#[test]
fn fingerprint_inputs_serialize_deserialize() {
    let inputs = FreezeFingerprintInputs {
        notes: b"note_data".to_vec(),
        lyrics: b"lyric_data".to_vec(),
        plugin_params: b"param_data".to_vec(),
        instrument_id: "piano".to_string(),
    };
    let json = serde_json::to_string(&inputs).expect("serialize");
    let back: FreezeFingerprintInputs = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(inputs, back);
}
