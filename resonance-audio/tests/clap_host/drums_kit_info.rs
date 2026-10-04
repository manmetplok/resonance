//! `com.resonance.kit-info` end to end (drums-plugin-rework.md §8, slice
//! K9): the built Resonance Drums serves the pads of the kit it plays, and
//! the engine reports them once after creating the instance — after its
//! `PluginAdded` — then only when a rescan finds them changed.

use std::time::{Duration, Instant};

use resonance_audio::test_support::{ClapBundle, EngineHandlerHarness};
use resonance_audio::types::{AudioCommand, AudioEvent, ChainOwner};
use resonance_audio::{Track, TrackId};
use resonance_common::drum_map::GM_PADS;

use crate::plugin_binaries::plugin_binary;

const TRACK: TrackId = 1;
const INSTANCE: u64 = 9;

fn harness_with(crate_name: &str, plugin_id: &str) -> Option<EngineHandlerHarness> {
    let path = plugin_binary(crate_name)?.to_string_lossy().into_owned();
    let mut h = EngineHandlerHarness::new();
    h.push_track(Track::new(TRACK, "T1".to_string()));
    h.add_plugin(ChainOwner::Track(TRACK), path, plugin_id.to_string(), INSTANCE);
    Some(h)
}

#[test]
fn the_drums_report_their_pads_once_after_creation() {
    let Some(mut h) = harness_with("resonance-drums", "com.resonance.drums") else {
        return;
    };
    h.poll_plugin_host_requests();
    let events = h.drain_events();
    let added = events
        .iter()
        .position(|e| {
            matches!(
                e,
                AudioEvent::PluginAdded {
                    instance_id: INSTANCE,
                    ..
                }
            )
        })
        .expect("the drums load");
    let (at, info) = events
        .iter()
        .enumerate()
        .find_map(|(i, e)| match e {
            AudioEvent::PluginKitInfo {
                instance_id: INSTANCE,
                info,
            } => Some((i, info.clone())),
            _ => None,
        })
        .expect("the kit info is reported after creation");
    assert!(at > added, "after PluginAdded, so the app has the slot");
    assert_eq!(info.pads.len(), GM_PADS.len());
    for (pad, gm) in info.pads.iter().zip(GM_PADS.iter()) {
        assert_eq!(pad.note, gm.note, "slot order, on the slot's note");
        assert!(!pad.name.is_empty());
    }
    if !info.from_kit {
        // The built-in kit: every pad sounds, under its GM name.
        assert!(info.pads.iter().all(|p| p.present));
        assert_eq!(info.pads[0].name, GM_PADS[0].name);
    }

    // Nothing changed, nothing asked: no second report.
    h.poll_plugin_host_requests();
    let again = h.drain_events();
    assert!(
        !again
            .iter()
            .any(|e| matches!(e, AudioEvent::PluginKitInfo { .. })),
        "reported once: {again:?}"
    );
}

/// A kit change re-reports the pads: a state naming a kit (a checked-in
/// fixture, by path — never a real kit) loads it, and the engine reports
/// that kit's pads with `from_kit` once the plugin's rescan arrives.
#[test]
fn a_kit_change_re_reports_the_kits_pads() {
    let Some(mut h) = harness_with("resonance-drums", "com.resonance.drums") else {
        return;
    };
    h.poll_plugin_host_requests();
    let _ = h.drain_events();

    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../plugins/resonance-drums/tests/fixtures/kit_pads/it_techno/drum_samples.json");
    assert!(fixture.is_file(), "{}", fixture.display());
    let state = serde_json::json!({
        "version": 1,
        "params": {},
        "kit_path": fixture.to_string_lossy(),
    });
    h.dispatch(AudioCommand::LoadPluginState {
        instance_id: INSTANCE,
        data: serde_json::to_vec(&state).unwrap(),
    });

    let deadline = Instant::now() + Duration::from_secs(30);
    let info = loop {
        h.poll_plugin_host_requests();
        let reported = h.drain_events().into_iter().rev().find_map(|e| match e {
            AudioEvent::PluginKitInfo {
                instance_id: INSTANCE,
                info,
            } if info.from_kit => Some(info),
            _ => None,
        });
        if let Some(info) = reported {
            break info;
        }
        assert!(Instant::now() < deadline, "the kit's pads were never re-reported");
        std::thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(info.pads.len(), GM_PADS.len());
    // it_techno: "SD Count Stick" is "Perc Conga", and there are no toms.
    assert!(info.pads.iter().any(|p| p.name == "Perc Conga" && p.present));
    assert!(info.pads.iter().any(|p| !p.present), "the toms are absent");
}

/// Whether the plugin in `crate_name`'s binary exposes the extension at
/// all: the host's `get_extension` found a vtable.
fn exposes_kit_info(crate_name: &str) -> Option<bool> {
    let path = plugin_binary(crate_name)?;
    let bundle = ClapBundle::load(&path).expect("the bundle loads");
    let id = bundle.descriptors().first().map(|d| d.id.clone())?;
    let instance = bundle.create_instance(&id, 48_000).expect("create_instance");
    Some(instance.has_kit_info())
}

/// Only a plugin with a kit to report exposes `com.resonance.kit-info`:
/// the bridge registers it for a plugin with a `kit_info_source`, so the
/// gate — like any third-party plugin — has no vtable for the host to
/// call, and the engine never reports kit info for it.
#[test]
fn a_plugin_without_the_extension_reports_no_kit_info() {
    if let Some(drums) = exposes_kit_info("resonance-drums") {
        assert!(drums, "the drums expose their pads");
    }
    let Some(gate) = exposes_kit_info("resonance-gate") else {
        return;
    };
    assert!(!gate, "a plugin without a kit_info_source has no extension");

    let Some(mut h) = harness_with("resonance-gate", "com.resonance.gate") else {
        return;
    };
    h.poll_plugin_host_requests();
    assert!(!h
        .drain_events()
        .iter()
        .any(|e| matches!(e, AudioEvent::PluginKitInfo { .. })));
}
