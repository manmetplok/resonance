//! `com.resonance.kit-info` end to end (drums-plugin-rework.md §8, slice
//! K9): the built Resonance Drums serves the pads of the kit it plays, and
//! the engine reports them once after creating the instance — after its
//! `PluginAdded` — then only when a rescan finds them changed.

use resonance_audio::test_support::EngineHandlerHarness;
use resonance_audio::types::AudioEvent;
use resonance_audio::{Track, TrackId};
use resonance_common::drum_map::GM_PADS;

use crate::plugin_binaries::plugin_binary;

const TRACK: TrackId = 1;
const INSTANCE: u64 = 9;

fn harness_with(crate_name: &str, plugin_id: &str) -> Option<EngineHandlerHarness> {
    let path = plugin_binary(crate_name)?.to_string_lossy().into_owned();
    let mut h = EngineHandlerHarness::new();
    h.push_track(Track::new(TRACK, "T1".to_string()));
    h.add_plugin(TRACK, path, plugin_id.to_string(), INSTANCE);
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

#[test]
fn a_plugin_without_the_extension_reports_no_kit_info() {
    let Some(mut h) = harness_with("resonance-gate", "com.resonance.gate") else {
        return;
    };
    h.poll_plugin_host_requests();
    assert!(!h
        .drain_events()
        .iter()
        .any(|e| matches!(e, AudioEvent::PluginKitInfo { .. })));
}
