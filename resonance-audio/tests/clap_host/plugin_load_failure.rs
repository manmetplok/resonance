//! An add that produces no instance has to be **addressable** (ba doc
//! #275 P5, todo #1309).
//!
//! Every `AddPlugin` / `AddPluginToBus` / `AddPluginToMaster` gets
//! exactly one of two answers: `PluginAdded` or `PluginLoadFailed`. The
//! second one is new — before it, a `.clap` the engine could not load
//! reached the app as a bare `AudioEvent::Error` string. That string
//! named the *problem* but never the *instance*, and the caller of a
//! project-load replay knows its instances by id, so the app had nothing
//! to attach the failure to: the placeholder slot it had already created
//! stayed indistinguishable from a working plugin.
//!
//! What is pinned here is the part that makes the event useful — the id
//! survives — plus the bundle lookup that produces most of its reasons,
//! which used to send its own generic error and now hands the reason
//! back to the caller instead.

use resonance_audio::test_support::{ensure_bundle, plugin_load_failed_event};
use resonance_audio::types::AudioEvent;

/// The whole point: the failure names the instance the command asked
/// for, so the app can mark THAT slot rather than guess.
#[test]
fn the_failure_event_carries_the_instance_id_the_add_asked_for() {
    let event = plugin_load_failed_event(
        Some(4242),
        "com.thirdparty.mega-verb",
        "/opt/clap/MegaVerb.clap",
        "Failed to load plugin: No such file or directory".to_owned(),
    );

    match event {
        AudioEvent::PluginLoadFailed {
            instance_id,
            clap_plugin_id,
            clap_file_path,
            reason,
        } => {
            assert_eq!(
                instance_id,
                Some(4242),
                "without the id this is the un-addressable error it replaced"
            );
            assert_eq!(clap_plugin_id, "com.thirdparty.mega-verb");
            assert_eq!(clap_file_path, "/opt/clap/MegaVerb.clap");
            assert_eq!(
                reason, "Failed to load plugin: No such file or directory",
                "the loader's own words, not a house string"
            );
        }
        other => panic!("expected PluginLoadFailed, got {other:?}"),
    }
}

/// An add that let the ENGINE allocate has no id to report, and that is
/// a different situation rather than a broken one: nothing was added, so
/// there is no slot to mark and the failure is just a message.
#[test]
fn an_engine_allocated_add_reports_no_instance() {
    let event = plugin_load_failed_event(None, "com.x.y", "/x/y.clap", "nope".to_owned());
    assert!(matches!(
        event,
        AudioEvent::PluginLoadFailed {
            instance_id: None,
            ..
        }
    ));
}

/// A bundle that isn't on disk — the machine-without-the-plugin case —
/// hands the reason back to the caller. It used to send its own
/// `AudioEvent::Error` and return `None`, which is why the caller had no
/// reason to attach to the slot.
#[test]
fn a_bundle_that_is_not_there_yields_a_reason_rather_than_a_silent_none() {
    let mut bundles = Vec::new();
    let missing = std::path::Path::new("/definitely/not/here/MegaVerb.clap");

    let reason = ensure_bundle(&mut bundles, missing, "com.thirdparty.mega-verb")
        .expect_err("a nonexistent bundle cannot load")
        .to_string();
    assert!(
        reason.starts_with("Failed to load plugin:"),
        "the reason has to say what failed: {reason}"
    );
    assert!(
        bundles.is_empty(),
        "and nothing was pushed onto the bundle list"
    );
}
