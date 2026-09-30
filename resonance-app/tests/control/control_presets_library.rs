//! The plugin preset library over the control API
//! (plugin-preset-library.md §12; slice P3): filters and facets on
//! `*.plugin_presets`, ids and metadata on every entry, load by id, save
//! with metadata and a star, and the `presets.*` library methods
//! (`set_marks`, `update_meta`, `vocabulary`).
//!
//! Every app here has a private preset root and marks store (a test app
//! gets both at construction), so nothing reads the user's library.

use resonance_app::state::{PluginSlotState, ViewMode};
use resonance_app::Resonance;
use resonance_audio::types::{AudioEvent, ParamInfo, ScannedPlugin, TrackType};
use resonance_common::factory_presets::FactoryPresetEntry;
use resonance_control::ids::TrackId as ProtoTrackId;
use resonance_control::methods::plugin_preset::{
    PluginPresetSource, PluginPresetsView, PresetFilter, PresetMetaInput, SavePluginPresetResult,
};
use resonance_control::methods::{presets, track as track_proto};
use resonance_control::{ErrorKind, Request, Response};

use crate::common::roundtrip;

const TRACK: u64 = 81;
const INSTANCE: u64 = 901;
const PLUGIN_ID: &str = "com.resonance.test-synth";

fn clap_id(s: &str) -> u32 {
    resonance_plugin::stable_hash(s)
}

fn params() -> Vec<ParamInfo> {
    ["cutoff", "drive"]
        .into_iter()
        .map(|id| ParamInfo {
            id: clap_id(id),
            name: id.to_owned(),
            min_value: 0.0,
            max_value: 20_000.0,
            default_value: 1.0,
            current_value: 1.0,
            ..Default::default()
        })
        .collect()
}

fn factory(id: &str, name: &str, cutoff: f64, meta: serde_json::Value) -> FactoryPresetEntry {
    FactoryPresetEntry {
        id: id.to_owned(),
        name: name.to_owned(),
        json: format!(r#"{{"version":1,"params":{{"cutoff":{cutoff},"drive":0.1}}}}"#),
        meta: Some(meta.to_string()),
    }
}

fn scanned() -> ScannedPlugin {
    ScannedPlugin {
        clap_file_path: "/nonexistent/test-synth.clap".to_owned(),
        clap_plugin_id: PLUGIN_ID.to_owned(),
        name: "Test Synth".to_owned(),
        vendor: "Resonance".to_owned(),
        is_instrument: true,
        factory_presets: vec![
            factory(
                "bass-reese",
                "Bass — Reese",
                400.0,
                serde_json::json!({"category": "Bass", "genres": ["drum-and-bass"],
                    "character": ["dark", "wide"], "instrument": ["synth-bass"]}),
            ),
            factory(
                "pad-glass",
                "Pad — Glass",
                9000.0,
                serde_json::json!({"category": "Pad", "genres": ["ambient"],
                    "character": ["bright", "airy"], "tags": ["shimmer"]}),
            ),
        ],
    }
}

/// A fresh app with the synth on a track.
fn app() -> Resonance {
    let (app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    with_plugin(app)
}

/// `app` (plain or capturing) with the synth scanned and on a track.
fn with_plugin(mut app: Resonance) -> Resonance {
    app.test_set_active_project(true);
    app.test_apply_engine_event(AudioEvent::PluginsScanned {
        plugins: vec![scanned()],
    });
    app.test_add_track(TRACK, TrackType::Instrument);
    app.test_push_track_plugin(
        TRACK,
        PluginSlotState::new(
            INSTANCE,
            "Test Synth".to_owned(),
            PLUGIN_ID.to_owned(),
            "/nonexistent/test-synth.clap".to_owned(),
            params(),
            false,
        ),
    );
    app
}

fn call<T: serde::Serialize>(app: &mut Resonance, method: &str, params: &T) -> Response {
    roundtrip(app, Request::new(1, method, params).expect("params serialize"))
}

fn list(app: &mut Resonance, filter: PresetFilter) -> PluginPresetsView {
    let response = call(
        app,
        track_proto::PLUGIN_PRESETS,
        &track_proto::PluginPresetsParams {
            track_id: ProtoTrackId(TRACK),
            plugin_id: Some(PLUGIN_ID.to_owned()),
            occurrence: None,
            filter,
        },
    );
    serde_json::from_value(response.result.expect("plugin_presets should succeed")).unwrap()
}

fn names(view: &PluginPresetsView) -> Vec<&str> {
    view.presets.iter().map(|p| p.name.as_str()).collect()
}

fn save(app: &mut Resonance, name: &str, meta: Option<PresetMetaInput>, favorite: bool) -> String {
    let response = call(
        app,
        track_proto::SAVE_PLUGIN_PRESET,
        &track_proto::SavePluginPresetParams {
            track_id: ProtoTrackId(TRACK),
            plugin_id: Some(PLUGIN_ID.to_owned()),
            occurrence: None,
            name: name.to_owned(),
            overwrite: false,
            meta,
            favorite: favorite.then_some(true),
            overwrite_id: None,
        },
    );
    let result: SavePluginPresetResult =
        serde_json::from_value(response.result.expect("save should succeed")).unwrap();
    app.test_apply_engine_event(AudioEvent::PluginPresetStateSaved {
        instance_id: INSTANCE,
        data: br#"{"version":1,"params":{"cutoff":777.0,"drive":0.5}}"#.to_vec(),
        preset_form: true,
        first_party: true,
    });
    result.id
}

#[test]
fn entries_carry_ids_and_metadata_and_filters_and_facets_apply() {
    let mut app = app();
    let all = list(&mut app, PresetFilter::default());
    assert_eq!(all.total, 2);
    let reese = &all.presets[0];
    assert_eq!(reese.id, "bass-reese");
    assert_eq!(reese.category.as_deref(), Some("Bass"));
    assert_eq!(reese.genres, vec!["drum-and-bass"]);
    assert_eq!(reese.instrument, vec!["synth-bass"]);

    let dark = list(
        &mut app,
        PresetFilter {
            character: vec!["dark".into()],
            ..Default::default()
        },
    );
    assert_eq!(names(&dark), vec!["Bass — Reese"]);
    // The character facet counts ignore the character selection itself.
    let bright = dark.facets.character.iter().find(|c| c.value == "bright");
    assert_eq!(bright.map(|c| c.count), Some(1));

    let q = list(
        &mut app,
        PresetFilter {
            query: Some("tag:shimmer".into()),
            ..Default::default()
        },
    );
    assert_eq!(names(&q), vec!["Pad — Glass"]);

    let paged = list(
        &mut app,
        PresetFilter {
            limit: Some(1),
            offset: Some(1),
            ..Default::default()
        },
    );
    assert_eq!(paged.total, 2);
    assert_eq!(names(&paged), vec!["Pad — Glass"]);
}

#[test]
fn a_load_by_id_wins_over_the_name_and_extra_false_recalls_params_only() {
    let (app, _task, rx) = Resonance::new_for_test_with_capture();
    let mut app = with_plugin(app);
    while rx.try_recv().is_ok() {}
    let response = call(
        &mut app,
        track_proto::LOAD_PLUGIN_PRESET,
        &track_proto::LoadPluginPresetParams {
            track_id: ProtoTrackId(TRACK),
            plugin_id: Some(PLUGIN_ID.to_owned()),
            occurrence: None,
            preset: "this name is ignored".to_owned(),
            source: None,
            preset_id: Some("pad-glass".to_owned()),
            extra: Some(false),
        },
    );
    assert!(response.error.is_none(), "{:?}", response.error);
    assert_eq!(app.test_plugin_param(INSTANCE, clap_id("cutoff")), Some(9000.0));
    let commands: Vec<_> = rx.try_iter().collect();
    let preset_state = commands.iter().any(|c| {
        matches!(c, resonance_audio::types::AudioCommand::LoadPluginPresetState { .. })
    });
    assert!(!preset_state, "extra: false recalls the params only");
    // A control-API load is a user pick: it lands in the recents.
    let recent = list(
        &mut app,
        PresetFilter {
            query: Some("is:recent".into()),
            ..Default::default()
        },
    );
    assert_eq!(names(&recent), vec!["Pad — Glass"]);
    assert!(recent.presets[0].last_used.is_some());
}

#[test]
fn a_save_returns_its_id_and_carries_meta_and_a_star() {
    let mut app = app();
    let id = save(
        &mut app,
        "Night Lead",
        Some(PresetMetaInput {
            category: Some("lead".into()),
            genres: Some(vec!["Synthwave".into()]),
            tags: Some(vec!["ferrous".into()]),
            ..Default::default()
        }),
        true,
    );
    let view = list(
        &mut app,
        PresetFilter {
            source: Some(PluginPresetSource::User),
            ..Default::default()
        },
    );
    let entry = &view.presets[0];
    assert_eq!(entry.id, id, "the id minted up front is the preset's");
    assert_eq!(entry.category.as_deref(), Some("Lead"));
    assert_eq!(entry.genres, vec!["synthwave"]);
    assert_eq!(entry.tags, vec!["ferrous"]);
    assert!(entry.favorite);
}

#[test]
fn set_marks_stars_a_factory_preset_without_touching_the_project() {
    let mut app = app();
    let revision = app.revision();
    let response = call(
        &mut app,
        presets::SET_MARKS,
        &presets::SetMarksParams {
            plugin_id: PLUGIN_ID.to_owned(),
            preset_id: "bass-reese".to_owned(),
            favorite: Some(true),
            tags: Some(vec!["Mine".into()]),
        },
    );
    let result: presets::EntryResult = serde_json::from_value(response.result.unwrap()).unwrap();
    assert!(result.entry.favorite);
    assert_eq!(result.entry.personal_tags, vec!["mine"]);
    assert_eq!(app.revision(), revision, "library state, not a project edit");

    let favs = list(
        &mut app,
        PresetFilter {
            favorites_only: true,
            ..Default::default()
        },
    );
    assert_eq!(names(&favs), vec!["Bass — Reese"]);

    let none = call(
        &mut app,
        presets::SET_MARKS,
        &presets::SetMarksParams {
            plugin_id: PLUGIN_ID.to_owned(),
            preset_id: "bass-reese".to_owned(),
            ..Default::default()
        },
    );
    assert_eq!(none.error.unwrap().kind(), ErrorKind::InvalidParams);
    let unknown = call(
        &mut app,
        presets::SET_MARKS,
        &presets::SetMarksParams {
            plugin_id: PLUGIN_ID.to_owned(),
            preset_id: "no-such".to_owned(),
            favorite: Some(true),
            ..Default::default()
        },
    );
    assert_eq!(unknown.error.unwrap().kind(), ErrorKind::NotFound);
}

#[test]
fn update_meta_edits_a_user_preset_and_refuses_a_factory_one() {
    let mut app = app();
    let id = save(&mut app, "Mine", None, false);
    let response = call(
        &mut app,
        presets::UPDATE_META,
        &presets::UpdateMetaParams {
            plugin_id: PLUGIN_ID.to_owned(),
            preset_id: id.clone(),
            set: Some(PresetMetaInput {
                description: Some("Late-night lead.".into()),
                character: Some(vec!["warm".into()]),
                ..Default::default()
            }),
            add_tags: vec!["demo".into()],
            remove_tags: vec![],
        },
    );
    let result: presets::EntryResult = serde_json::from_value(response.result.unwrap()).unwrap();
    assert_eq!(result.entry.description.as_deref(), Some("Late-night lead."));
    assert_eq!(result.entry.character, vec!["warm"]);
    assert_eq!(result.entry.tags, vec!["demo"]);
    assert_eq!(result.entry.name, "Mine", "the name is not changed here");

    let refused = call(
        &mut app,
        presets::UPDATE_META,
        &presets::UpdateMetaParams {
            plugin_id: PLUGIN_ID.to_owned(),
            preset_id: "bass-reese".to_owned(),
            add_tags: vec!["x".into()],
            ..Default::default()
        },
    );
    let err = refused.error.expect("a factory preset is read-only");
    assert!(err.message.contains("set_marks"), "{}", err.message);
}

#[test]
fn the_vocabulary_lists_seeded_values_then_values_in_use() {
    let mut app = app();
    save(
        &mut app,
        "Odd",
        Some(PresetMetaInput {
            genres: Some(vec!["shoegaze".into()]),
            ..Default::default()
        }),
        false,
    );
    let response = call(&mut app, presets::VOCABULARY, &presets::VocabularyParams {});
    let vocab: presets::Vocabulary = serde_json::from_value(response.result.unwrap()).unwrap();
    assert_eq!(vocab.genres.first().map(String::as_str), Some("ambient"));
    assert!(vocab.genres.iter().any(|g| g == "shoegaze"), "{:?}", vocab.genres);
    assert!(vocab.categories_effect.iter().any(|c| c == "Bus"));
    assert!(vocab.tags.iter().any(|t| t == "shimmer"));
}

// ---------------------------------------------------------------------------
// presets.search / presets.rename / presets.delete (slice P4)
// ---------------------------------------------------------------------------

#[test]
fn search_finds_presets_across_plugins_with_facets() {
    let mut app = app();
    let response = call(
        &mut app,
        presets::SEARCH,
        &presets::SearchParams {
            plugin_id: None,
            filter: PresetFilter {
                query: Some("char:dark".into()),
                ..Default::default()
            },
        },
    );
    let result: presets::SearchResult = serde_json::from_value(response.result.unwrap()).unwrap();
    assert_eq!(result.total, 1);
    assert_eq!(result.hits[0].plugin_id, PLUGIN_ID);
    assert_eq!(result.hits[0].entry.id, "bass-reese");
    assert!(result.facets.category.iter().any(|c| c.value == "Bass"));
    // No project is needed: library state.
    let (mut bare, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    let r = call(&mut bare, presets::SEARCH, &presets::SearchParams::default());
    assert!(r.error.is_none(), "{:?}", r.error);
}

#[test]
fn rename_keeps_the_id_and_delete_needs_confirm_and_goes_to_the_trash() {
    let mut app = app();
    let id = save(&mut app, "Before", None, false);
    let response = call(
        &mut app,
        presets::RENAME,
        &presets::RenameParams {
            plugin_id: PLUGIN_ID.to_owned(),
            preset_id: id.clone(),
            name: "After".to_owned(),
        },
    );
    let result: presets::EntryResult = serde_json::from_value(response.result.unwrap()).unwrap();
    assert_eq!(result.entry.name, "After");
    assert_eq!(result.entry.id, id);

    let refused = call(
        &mut app,
        presets::RENAME,
        &presets::RenameParams {
            plugin_id: PLUGIN_ID.to_owned(),
            preset_id: "bass-reese".to_owned(),
            name: "Mine".to_owned(),
        },
    );
    assert_eq!(refused.error.unwrap().kind(), ErrorKind::InvalidParams);

    let unconfirmed = call(
        &mut app,
        presets::DELETE,
        &presets::DeleteParams {
            plugin_id: PLUGIN_ID.to_owned(),
            preset_id: id.clone(),
            confirm: false,
        },
    );
    let err = unconfirmed.error.expect("refused without confirm");
    assert!(err.message.contains("After"), "{}", err.message);
    assert_eq!(names(&list(&mut app, PresetFilter::default())).len(), 3);

    let deleted = call(
        &mut app,
        presets::DELETE,
        &presets::DeleteParams {
            plugin_id: PLUGIN_ID.to_owned(),
            preset_id: id,
            confirm: true,
        },
    );
    let result: presets::DeleteResult = serde_json::from_value(deleted.result.unwrap()).unwrap();
    assert!(std::path::Path::new(&result.trashed_path).is_file());
    assert!(result.trashed_path.contains(".trash"), "{}", result.trashed_path);
    assert_eq!(names(&list(&mut app, PresetFilter::default())).len(), 2);
}

// ---------------------------------------------------------------------------
// The loaded preset's identity (slice P5)
// ---------------------------------------------------------------------------

fn load_by_id(app: &mut Resonance, id: &str) {
    let response = call(
        app,
        track_proto::LOAD_PLUGIN_PRESET,
        &track_proto::LoadPluginPresetParams {
            track_id: ProtoTrackId(TRACK),
            plugin_id: Some(PLUGIN_ID.to_owned()),
            occurrence: None,
            preset: String::new(),
            source: None,
            preset_id: Some(id.to_owned()),
            extra: None,
        },
    );
    assert!(response.error.is_none(), "{:?}", response.error);
}

/// A host load names the preset straight away; a host param edit marks a
/// plugin that does not report as modified. Once the plugin reports, its
/// report is the truth: identity, modified, and `modified_known`.
#[test]
fn the_view_reports_the_loaded_preset_and_whether_it_was_edited() {
    use resonance_app::message::{Message, PluginMessage};
    let mut app = app();
    assert!(list(&mut app, PresetFilter::default()).current.is_none());

    load_by_id(&mut app, "pad-glass");
    let view = list(&mut app, PresetFilter::default());
    let current = view.current.as_ref().expect("the host load names the preset");
    assert_eq!((current.id.as_str(), current.name.as_str()), ("pad-glass", "Pad — Glass"));
    assert_eq!(current.category.as_deref(), Some("Pad"), "the full entry");
    assert!(!view.modified);
    assert!(!view.modified_known);

    let _ = app.update(Message::Plugin(PluginMessage::SetPluginParam(
        INSTANCE,
        clap_id("drive"),
        0.7,
    )));
    assert!(list(&mut app, PresetFilter::default()).modified, "a host edit");

    // The plugin reports (its own browser picked another preset, edited).
    app.test_apply_engine_event(AudioEvent::PluginPresetIdentity {
        instance_id: INSTANCE,
        identity: Some(resonance_common::preset_session::IdentityReport {
            source: "factory".into(),
            id: "bass-reese".into(),
            name: "Bass — Reese".into(),
            modified: false,
        }),
    });
    let view = list(&mut app, PresetFilter::default());
    assert_eq!(view.current.as_ref().map(|c| c.id.as_str()), Some("bass-reese"));
    assert!(!view.modified);
    assert!(view.modified_known);
    // A reporting plugin compares for itself: a host edit is not assumed.
    let _ = app.update(Message::Plugin(PluginMessage::SetPluginParam(
        INSTANCE,
        clap_id("drive"),
        0.2,
    )));
    assert!(!list(&mut app, PresetFilter::default()).modified);

    app.test_apply_engine_event(AudioEvent::PluginPresetIdentity {
        instance_id: INSTANCE,
        identity: None,
    });
    assert!(list(&mut app, PresetFilter::default()).current.is_none());
}

/// A third-party plugin's `loaded()` is its identity when it reports
/// nothing else: a factory preset by load key.
#[test]
fn a_plugin_that_only_says_loaded_is_named_by_its_load_key() {
    let mut app = app();
    app.test_apply_engine_event(AudioEvent::PluginPresetLoaded {
        instance_id: INSTANCE,
        location: resonance_audio::types::PluginPresetLocation::Plugin,
        load_key: Some("pad-glass".into()),
    });
    let view = list(&mut app, PresetFilter::default());
    assert_eq!(view.current.as_ref().map(|c| c.name.as_str()), Some("Pad — Glass"));
    assert!(!view.modified_known);
}

/// The params under an enabled automation lane are sent to the plugin,
/// which leaves them out of its modified comparison (D8).
#[test]
fn automated_params_are_sent_to_the_plugin_as_ignored() {
    use resonance_audio::types::AudioCommand;
    use resonance_common::{AutomationLane, AutomationTarget, Breakpoint, CurveKind};
    let (app, _task, rx) = Resonance::new_for_test_with_capture();
    let mut app = with_plugin(app);
    while rx.try_recv().is_ok() {}
    let target = AutomationTarget::PluginParam {
        instance: INSTANCE,
        param_id: clap_id("cutoff"),
    };
    let lane = AutomationLane::new(
        7,
        target.clone(),
        vec![Breakpoint::new(0, 0.5, CurveKind::Linear)],
    );
    app.test_apply_engine_event(AudioEvent::AutomationLaneChanged { lane });
    let sent: Vec<_> = rx
        .try_iter()
        .filter_map(|c| match c {
            AudioCommand::SetPluginPresetIgnoredParams {
                instance_id,
                clap_ids,
            } => Some((instance_id, clap_ids)),
            _ => None,
        })
        .collect();
    assert_eq!(sent, vec![(INSTANCE, vec![clap_id("cutoff")])]);

    app.test_apply_engine_event(AudioEvent::AutomationLaneCleared { target });
    let sent: Vec<_> = rx
        .try_iter()
        .filter_map(|c| match c {
            AudioCommand::SetPluginPresetIgnoredParams { clap_ids, .. } => Some(clap_ids),
            _ => None,
        })
        .collect();
    assert_eq!(sent, vec![Vec::<u32>::new()], "the last lane gone: nothing ignored");
}

// ---------------------------------------------------------------------------
// A preset on add (slice P6)
// ---------------------------------------------------------------------------

const EMPTY_TRACK: u64 = 82;

fn echo_added(app: &mut Resonance, instance_id: u64) {
    app.test_apply_engine_event(AudioEvent::PluginAdded {
        track_id: EMPTY_TRACK,
        instance_id,
        plugin_name: "Test Synth".to_owned(),
        clap_plugin_id: PLUGIN_ID.to_owned(),
        clap_file_path: "/nonexistent/test-synth.clap".to_owned(),
        params: params(),
        has_gui: false,
        has_sidechain_input: false,
        output_port_count: 1,
        output_port_names: Vec::new(),
    });
}

/// `track.add_instrument {preset}` loads the preset once the plugin's
/// params arrive with the engine's echo, and names it; an unknown preset
/// is refused before anything is added.
#[test]
fn an_instrument_added_with_a_preset_comes_up_with_that_sound() {
    let mut app = app();
    app.test_add_track(EMPTY_TRACK, TrackType::Instrument);
    let add = |app: &mut Resonance, preset: &str| {
        call(
            app,
            track_proto::ADD_INSTRUMENT,
            &track_proto::AddPluginParams {
                track_id: ProtoTrackId(EMPTY_TRACK),
                plugin_id: PLUGIN_ID.to_owned(),
                preset: Some(preset.to_owned()),
            },
        )
    };

    let refused = add(&mut app, "No Such Preset");
    assert_eq!(refused.error.map(|e| e.kind()), Some(ErrorKind::NotFound));
    let next = app.test_next_plugin_id();

    let response = add(&mut app, "bass-reese");
    assert!(response.error.is_none(), "{:?}", response.error);
    assert_eq!(app.test_next_plugin_id(), next + 1, "nothing was added by the refusal");
    echo_added(&mut app, next);
    assert_eq!(app.test_plugin_param(next, clap_id("cutoff")), Some(400.0));

    let view: PluginPresetsView = serde_json::from_value(
        call(
            &mut app,
            track_proto::PLUGIN_PRESETS,
            &track_proto::PluginPresetsParams {
                track_id: ProtoTrackId(EMPTY_TRACK),
                plugin_id: Some(PLUGIN_ID.to_owned()),
                occurrence: None,
                filter: PresetFilter::default(),
            },
        )
        .result
        .expect("plugin_presets"),
    )
    .unwrap();
    assert_eq!(view.current.map(|c| c.id), Some("bass-reese".to_owned()));
}

// ---------------------------------------------------------------------------
// Third-party (opaque) presets — slice P7
// ---------------------------------------------------------------------------

const VENDOR_TRACK: u64 = 83;
const VENDOR_INSTANCE: u64 = 902;
const VENDOR_ID: &str = "com.vendor.synth";
const OPAQUE: &[u8] = &[0x00, 0xff, 0xfe, 0x80, b'V', b'S', b'T', 0x01, 0xc3, 0x28];

/// A third-party plugin's state is not a Resonance document: the save
/// stores it as a `clap-state` blob, the load hands the same bytes back
/// through `LoadPluginPresetState` (one undo entry), and the mirror takes
/// the values the plugin reports afterwards.
#[test]
fn a_third_party_plugins_state_saves_and_loads_as_an_opaque_preset() {
    use resonance_audio::types::AudioCommand;
    let (app, _task, rx) = Resonance::new_for_test_with_capture();
    let mut app = with_plugin(app);
    app.test_add_track(VENDOR_TRACK, TrackType::Instrument);
    app.test_push_track_plugin(
        VENDOR_TRACK,
        PluginSlotState::new(
            VENDOR_INSTANCE,
            "Vendor Synth".to_owned(),
            VENDOR_ID.to_owned(),
            "/nonexistent/vendor.clap".to_owned(),
            params(),
            false,
        ),
    );
    while rx.try_recv().is_ok() {}
    let saved = call(
        &mut app,
        track_proto::SAVE_PLUGIN_PRESET,
        &track_proto::SavePluginPresetParams {
            track_id: ProtoTrackId(VENDOR_TRACK),
            plugin_id: Some(VENDOR_ID.to_owned()),
            occurrence: None,
            name: "Glass".to_owned(),
            overwrite: false,
            meta: None,
            favorite: None,
            overwrite_id: None,
        },
    );
    assert!(saved.error.is_none(), "{:?}", saved.error);
    assert!(rx.try_iter().any(|c| matches!(
        c,
        AudioCommand::SavePluginPresetState { instance_id: VENDOR_INSTANCE }
    )));
    app.test_apply_engine_event(AudioEvent::PluginPresetStateSaved {
        instance_id: VENDOR_INSTANCE,
        data: OPAQUE.to_vec(),
        preset_form: false,
        first_party: false,
    });

    let listed: PluginPresetsView = serde_json::from_value(
        call(
            &mut app,
            track_proto::PLUGIN_PRESETS,
            &track_proto::PluginPresetsParams {
                track_id: ProtoTrackId(VENDOR_TRACK),
                plugin_id: Some(VENDOR_ID.to_owned()),
                occurrence: None,
                filter: PresetFilter::default(),
            },
        )
        .result
        .expect("plugin_presets"),
    )
    .unwrap();
    assert_eq!(names(&listed), vec!["Glass"]);

    let loaded = call(
        &mut app,
        track_proto::LOAD_PLUGIN_PRESET,
        &track_proto::LoadPluginPresetParams {
            track_id: ProtoTrackId(VENDOR_TRACK),
            plugin_id: Some(VENDOR_ID.to_owned()),
            occurrence: None,
            preset: "Glass".to_owned(),
            source: None,
            preset_id: None,
            extra: Some(false),
        },
    );
    assert!(loaded.error.is_none(), "{:?}", loaded.error);
    let sent = rx.try_iter().find_map(|c| match c {
        AudioCommand::LoadPluginPresetState { instance_id: VENDOR_INSTANCE, data, .. } => Some(data),
        _ => None,
    });
    assert_eq!(sent.as_deref(), Some(OPAQUE), "the bytes go back untouched, extra or not");

    let mut refreshed = params();
    refreshed[0].current_value = 1234.0;
    app.test_apply_engine_event(AudioEvent::PluginParamsRefreshed {
        instance_id: VENDOR_INSTANCE,
        params: refreshed,
    });
    assert_eq!(app.test_plugin_param(VENDOR_INSTANCE, clap_id("cutoff")), Some(1234.0));
}

/// Provenance, not content (review M3): a third-party plugin whose state
/// happens to be JSON with a `params` object (nih-plug style) is still
/// saved as an opaque blob, and loads back whole — a param-by-param recall
/// of it could never load.
#[test]
fn a_third_party_json_state_is_kept_opaque() {
    use resonance_audio::types::AudioCommand;
    let nih_style = br#"{"params":{"cutoff":"0.25","gain":"-3 dB"},"fields":{}}"#;
    let (app, _task, rx) = Resonance::new_for_test_with_capture();
    let mut app = with_plugin(app);
    app.test_add_track(VENDOR_TRACK, TrackType::Instrument);
    app.test_push_track_plugin(
        VENDOR_TRACK,
        PluginSlotState::new(
            VENDOR_INSTANCE,
            "Vendor Synth".to_owned(),
            VENDOR_ID.to_owned(),
            "/nonexistent/vendor.clap".to_owned(),
            params(),
            false,
        ),
    );
    let saved = call(
        &mut app,
        track_proto::SAVE_PLUGIN_PRESET,
        &track_proto::SavePluginPresetParams {
            track_id: ProtoTrackId(VENDOR_TRACK),
            plugin_id: Some(VENDOR_ID.to_owned()),
            occurrence: None,
            name: "Nih".to_owned(),
            overwrite: false,
            meta: None,
            favorite: None,
            overwrite_id: None,
        },
    );
    assert!(saved.error.is_none(), "{:?}", saved.error);
    app.test_apply_engine_event(AudioEvent::PluginPresetStateSaved {
        instance_id: VENDOR_INSTANCE,
        data: nih_style.to_vec(),
        preset_form: false,
        first_party: false,
    });
    while rx.try_recv().is_ok() {}
    let loaded = call(
        &mut app,
        track_proto::LOAD_PLUGIN_PRESET,
        &track_proto::LoadPluginPresetParams {
            track_id: ProtoTrackId(VENDOR_TRACK),
            plugin_id: Some(VENDOR_ID.to_owned()),
            occurrence: None,
            preset: "Nih".to_owned(),
            source: None,
            preset_id: None,
            extra: None,
        },
    );
    assert!(loaded.error.is_none(), "it loads: {:?}", loaded.error);
    let cmds: Vec<_> = rx.try_iter().collect();
    let sent = cmds.iter().find_map(|c| match c {
        AudioCommand::LoadPluginPresetState { instance_id: VENDOR_INSTANCE, data, .. } => {
            Some(data.clone())
        }
        _ => None,
    });
    assert_eq!(sent.as_deref(), Some(&nih_style[..]), "whole and untouched");
    assert!(
        !cmds.iter().any(|c| matches!(c, AudioCommand::SetPluginParam { instance_id: VENDOR_INSTANCE, .. })),
        "no param-by-param recall of an opaque state"
    );
}
