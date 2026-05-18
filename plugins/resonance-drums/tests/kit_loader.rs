use std::path::PathBuf;

use resonance_drums::drum_map::{PadMapping, NUM_PADS, PAD_MAPPINGS};
use resonance_drums::kit_loader::{
    build_fallback_pad, load_kit_from_manifest, parse_vel_index, PadMicChoices,
    DEFAULT_OVERHEAD_SETUP,
};

/// Resolve the drummica manifest path for the smoke test. Honours the
/// `RESONANCE_DRUMMICA_PATH` env var so other developers and CI can
/// point at their own copy; falls back to the author's local path.
fn drummica_manifest() -> PathBuf {
    std::env::var("RESONANCE_DRUMMICA_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from("/home/jorrit/Documents/Guitar/drummica/drum_samples.json")
        })
}

fn default_choices() -> [PadMicChoices; NUM_PADS] {
    std::array::from_fn(|_| PadMicChoices::default())
}

fn default_articulations() -> [bool; NUM_PADS] {
    [false; NUM_PADS]
}

/// Smoke test: parses the drummica manifest if present, verifying that
/// the loader returns exactly NUM_PADS pads and that close-mic counts
/// match the PadMapping spec. Gated on the manifest path existing so
/// CI without the samples still passes.
#[test]
fn drummica_smoke() {
    let manifest = drummica_manifest();
    if !manifest.exists() {
        eprintln!(
            "drummica manifest not present at {}; skipping",
            manifest.display()
        );
        return;
    }
    let kit = load_kit_from_manifest(
        &manifest,
        48000.0,
        DEFAULT_OVERHEAD_SETUP,
        &default_choices(),
        &default_articulations(),
    )
    .expect("drummica kit should load cleanly");
    assert_eq!(kit.pads.len(), NUM_PADS);

    // Kick and snare each have two close mic positions (In+Out and
    // Top+Btm respectively) so they must load two banks.
    assert_eq!(
        kit.pads[0].close_mics.len(),
        2,
        "kick should load KickIn + KickOut"
    );
    assert_eq!(
        kit.pads[1].close_mics.len(),
        2,
        "snare should load SNTop + SNBtm"
    );
    assert_eq!(
        kit.pads[25].close_mics.len(),
        2,
        "rimshot should load SNTop + SNBtm"
    );

    // Tom pads each have a single position mic.
    for tom_idx in [9, 10, 11] {
        assert_eq!(
            kit.pads[tom_idx].close_mics.len(),
            1,
            "tom pad {} should have one close mic bank",
            tom_idx
        );
    }

    // Hi-hat pads have a single Hat close mic.
    for hat_idx in [2, 3, 4, 5, 6, 7, 8] {
        assert_eq!(
            kit.pads[hat_idx].close_mics.len(),
            1,
            "hi-hat pad {} should have one close mic bank",
            hat_idx
        );
    }

    // Cymbal pads (crashes, rides, chinas) have no close mics in Drummica.
    for cymbal_idx in [12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23] {
        assert_eq!(
            kit.pads[cymbal_idx].close_mics.len(),
            0,
            "cymbal pad {} has no close mic in Drummica",
            cymbal_idx
        );
    }

    // Every pad that maps to a Drummica piece has an overhead bank.
    for (i, pad) in kit.pads.iter().enumerate() {
        assert!(
            pad.overhead.is_some(),
            "pad {} ({}) should have an overhead bank",
            i,
            pad.name
        );
    }

    // Every loaded bank must have at least one velocity layer and RR.
    for pad in &kit.pads {
        for bank in pad.close_mics.iter().chain(pad.overhead.iter()) {
            assert!(
                !bank.layers.is_empty(),
                "bank {} has no layers",
                bank.setup_key
            );
            for layer in &bank.layers {
                assert!(
                    !layer.round_robins.is_empty(),
                    "bank {} layer has no round robins",
                    bank.setup_key
                );
            }
        }
    }

    // Catalog should mention every Drummica position we expect.
    for position in [
        "KickIn", "KickOut", "SNTop", "SNBtm", "Hat", "Tom01", "Tom02", "TomFloor", "OHsAB",
        "OHsXY",
    ] {
        assert!(
            kit.catalog.positions.contains_key(position),
            "catalog missing position {}",
            position
        );
    }
}

/// Smoke test for articulation: load with "ohne Teppich" for kick.
#[test]
fn drummica_articulation_smoke() {
    let manifest = drummica_manifest();
    if !manifest.exists() {
        eprintln!(
            "drummica manifest not present at {}; skipping",
            manifest.display()
        );
        return;
    }
    let mut arts = default_articulations();
    arts[0] = true; // Kick -> ohne Teppich
    let kit = load_kit_from_manifest(
        &manifest,
        48000.0,
        DEFAULT_OVERHEAD_SETUP,
        &default_choices(),
        &arts,
    )
    .expect("drummica kit with articulation toggle should load cleanly");
    assert_eq!(kit.pads.len(), NUM_PADS);
    // Kick should still have 2 close mics even with ohne Teppich.
    assert_eq!(
        kit.pads[0].close_mics.len(),
        2,
        "kick ohne Teppich should still load KickIn + KickOut"
    );
}

#[test]
fn parse_vel() {
    assert_eq!(parse_vel_index("Vel01"), Some(1));
    assert_eq!(parse_vel_index("Vel28"), Some(28));
    assert_eq!(parse_vel_index("Velocity"), None);
    assert_eq!(parse_vel_index("foo"), None);
}

/// Every embedded default sample decodes cleanly via the fallback path.
/// This runs without any external assets — it only exercises the bytes
/// baked into the binary via `include_bytes!`.
#[test]
fn fallback_pads_all_decode() {
    for mapping in &PAD_MAPPINGS {
        let pad = build_fallback_pad(mapping, 48000.0)
            .unwrap_or_else(|e| panic!("fallback for {} failed: {e}", mapping.name));
        assert_eq!(
            pad.close_mics.len(),
            1,
            "{} fallback should hold exactly one close bank",
            mapping.name
        );
        assert_eq!(pad.close_mics[0].layers.len(), 1);
        assert_eq!(pad.close_mics[0].layers[0].round_robins.len(), 1);
        assert!(
            pad.close_mics[0].layers[0].round_robins[0].frames > 0,
            "{} sample is empty",
            mapping.name
        );
        assert!(pad.overhead.is_none(), "fallback pads have no overhead");
    }
}

// Avoid unused-import on PadMapping (test below only borrows from PAD_MAPPINGS).
#[allow(dead_code)]
fn _ensure_pad_mapping_in_scope(_: &PadMapping) {}
