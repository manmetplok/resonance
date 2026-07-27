//! Export job-model tests (ba todo #649): `ExportSettings` serde
//! round-trips and the `BounceToWav` → `ExportAudio` default-WAV mapping.

use resonance_audio::types::{
    BitDepth, ExportFormat, ExportMetadata, ExportSettings, FlacLevel, Mp3Rate, NormalizeMode,
    NormalizeSpec, OpusOptimize,
};

#[test]
fn export_settings_serde_round_trip() {
    let settings = ExportSettings {
        format: ExportFormat::Flac {
            bit_depth: BitDepth::I24,
            sample_rate: Some(44_100),
            compression: FlacLevel::Max,
        },
        normalize: NormalizeSpec {
            enabled: true,
            mode: NormalizeMode::IntegratedLufs,
            target_db: -14.0,
            ceiling_dbtp: -1.0,
        },
        metadata: ExportMetadata {
            title: Some("Take 3".into()),
            artist: Some("Resonance".into()),
            album: None,
            year: Some(2026),
        },
    };

    let json = serde_json::to_string(&settings).expect("serialize");
    let back: ExportSettings = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(settings, back);
}

#[test]
fn every_export_format_round_trips() {
    let formats = [
        ExportFormat::Wav {
            bit_depth: BitDepth::I16,
            sample_rate: None,
        },
        ExportFormat::Wav {
            bit_depth: BitDepth::F32,
            sample_rate: Some(48_000),
        },
        ExportFormat::Flac {
            bit_depth: BitDepth::I24,
            sample_rate: None,
            compression: FlacLevel::Fast,
        },
        ExportFormat::Mp3 {
            mode: Mp3Rate::Vbr,
            bitrate_kbps: 256,
        },
        ExportFormat::Mp3 {
            mode: Mp3Rate::Cbr,
            bitrate_kbps: 320,
        },
        ExportFormat::Opus {
            bitrate_kbps: 160,
            optimize: OpusOptimize::Voice,
        },
    ];

    for fmt in formats {
        let json = serde_json::to_string(&fmt).expect("serialize");
        let back: ExportFormat = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(fmt, back, "round-trip mismatch for {fmt:?}");
    }
}

#[test]
fn normalize_spec_default_is_disabled() {
    let n = NormalizeSpec::default();
    assert!(!n.enabled);
    assert_eq!(n.mode, NormalizeMode::IntegratedLufs);
    assert_eq!(n.ceiling_dbtp, -1.0);
}

/// The `BounceToWav` shim maps onto `ExportAudio` with
/// [`ExportSettings::default_wav`]; assert that target is byte-identical
/// to the legacy bounce: 32-bit-float WAV at the engine rate, no
/// normalization, no metadata.
#[test]
fn bounce_to_wav_maps_to_default_wav_settings() {
    let mapped = ExportSettings::default_wav();

    assert_eq!(
        mapped.format,
        ExportFormat::Wav {
            bit_depth: BitDepth::F32,
            sample_rate: None,
        },
        "default WAV must be 32-bit float at the engine rate"
    );
    assert!(
        !mapped.normalize.enabled,
        "legacy bounce applied no loudness normalization"
    );
    assert_eq!(
        mapped.metadata,
        ExportMetadata::default(),
        "legacy bounce embedded no metadata"
    );

    // `ExportSettings::default()` is the same default-WAV spec.
    assert_eq!(mapped, ExportSettings::default());
}
