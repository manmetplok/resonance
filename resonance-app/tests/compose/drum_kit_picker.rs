//! The Drum Groups Manager's kit picker reads the kit the drum track
//! really plays (drums-plugin-rework.md §8, slice K9): its name from the
//! Resonance Drums `kit_select` text, its pads from the instance's
//! `com.resonance.kit-info` report — not a hardcoded "Drummica" and the
//! General MIDI table. Without a drums instance it falls back to that
//! table.

use resonance_app::compose::drumroll::default_kit_pads;
use resonance_app::compose::messages::DrumGroupsMessage;
use resonance_app::compose::ComposeMessage;
use resonance_app::message::Message;
use resonance_app::state::ViewMode;
use resonance_app::Resonance;
use resonance_audio::types::{ChainOwner, AudioEvent, ParamInfo, ParamValueUpdate, TrackType};
use resonance_common::drum_map::GM_PADS;
use resonance_common::kit_info::{KitInfo, KitInfoPad};

const INSTANCE: u64 = 70;

fn open_manager(app: &mut Resonance) {
    let _ = app.update(Message::Compose(ComposeMessage::DrumGroups(
        DrumGroupsMessage::OpenManager,
    )));
}

fn kit_select(value: f64, text: &str) -> ParamInfo {
    ParamInfo {
        id: resonance_plugin::stable_hash("kit_select"),
        name: "Kit".to_owned(),
        min_value: -2.0,
        max_value: 999.0,
        current_value: value,
        stepped: true,
        text: text.to_owned(),
        ..Default::default()
    }
}

/// A track with Resonance Drums on it, as the engine reports it.
fn app_with_drums(kit: &str) -> Resonance {
    let (mut app, _) = Resonance::new_for_test_on(ViewMode::Compose);
    app.test_set_active_project(true);
    app.test_add_track(1, TrackType::Instrument);
    app.test_apply_engine_event(AudioEvent::PluginAdded {
        owner: ChainOwner::Track(1),
        instance_id: INSTANCE,
        plugin_name: "Resonance Drums".to_owned(),
        clap_plugin_id: "com.resonance.drums".to_owned(),
        clap_file_path: "/plugins/drums.clap".to_owned(),
        params: vec![kit_select(1.0, kit)],
        has_gui: false,
        has_sidechain_input: false,
        output_port_count: 1,
        output_port_names: vec!["Main".to_owned()],
    });
    app
}

/// A kit that names its kick "Bass Drum" and has no cymbals.
fn garage_info() -> KitInfo {
    KitInfo {
        from_kit: true,
        pads: GM_PADS
            .iter()
            .enumerate()
            .map(|(i, p)| KitInfoPad {
                note: p.note,
                name: if i == 0 {
                    "Bass Drum".into()
                } else {
                    p.name.into()
                },
                present: !p.name.contains("Crash")
                    && !p.name.contains("Ride")
                    && !p.name.contains("China"),
            })
            .collect(),
    }
}

#[test]
fn without_a_drums_instance_the_picker_shows_the_general_midi_table() {
    let (mut app, _) = Resonance::new_for_test_on(ViewMode::Compose);
    app.test_set_active_project(true);
    open_manager(&mut app);
    assert!(app.compose_state().drumroll.manager_open);
    let compose = app.compose_state();
    assert_eq!(compose.kit_name, None);
    assert_eq!(compose.kit_pads, default_kit_pads());
}

#[test]
fn the_picker_shows_the_kit_the_drums_report() {
    let mut app = app_with_drums("Garage");
    app.test_apply_engine_event(AudioEvent::PluginKitInfo {
        instance_id: INSTANCE,
        info: garage_info(),
    });
    open_manager(&mut app);
    let compose = app.compose_state();
    assert_eq!(compose.kit_name.as_deref(), Some("Garage"));
    assert_eq!(
        compose.kit_pads.len(),
        GM_PADS.len(),
        "no external-GM extras"
    );
    assert_eq!(compose.kit_pads[0].name, "Bass Drum");
    assert_eq!(compose.kit_pads[0].category, "Kick");
    let absent: Vec<&str> = compose
        .kit_pads
        .iter()
        .filter(|p| !p.present)
        .map(|p| p.name.as_str())
        .collect();
    assert_eq!(
        absent.len(),
        12,
        "the crashes, rides and chinas: {absent:?}"
    );
}

/// Two drum tracks: the picker describes the one the details panel
/// shows, else the first in track order — and a report from the other
/// instance does not move it.
#[test]
fn with_two_drum_tracks_the_picker_follows_the_selected_one() {
    use resonance_app::compose::SelectedLane;
    const OTHER: u64 = 71;
    let mut app = app_with_drums("Garage");
    app.test_add_track(2, TrackType::Instrument);
    app.test_apply_engine_event(AudioEvent::PluginAdded {
        owner: ChainOwner::Track(2),
        instance_id: OTHER,
        plugin_name: "Resonance Drums".to_owned(),
        clap_plugin_id: "com.resonance.drums".to_owned(),
        clap_file_path: "/plugins/drums.clap".to_owned(),
        params: vec![kit_select(0.0, "Drummica")],
        has_gui: false,
        has_sidechain_input: false,
        output_port_count: 1,
        output_port_names: vec!["Main".to_owned()],
    });
    app.test_apply_engine_event(AudioEvent::PluginKitInfo {
        instance_id: INSTANCE,
        info: garage_info(),
    });
    let mut drummica = garage_info();
    drummica.pads[0].name = "Kick Teppich".into();
    app.test_apply_engine_event(AudioEvent::PluginKitInfo {
        instance_id: OTHER,
        info: drummica,
    });

    // Nothing selected: the first drum track in track order.
    open_manager(&mut app);
    assert_eq!(app.compose_state().kit_name.as_deref(), Some("Garage"));
    assert_eq!(app.compose_state().kit_pads[0].name, "Bass Drum");

    // The second drum track selected: its kit.
    let _ = app.update(Message::Compose(ComposeMessage::SelectLane(
        SelectedLane::Drums(2),
    )));
    open_manager(&mut app);
    assert_eq!(app.compose_state().kit_name.as_deref(), Some("Drummica"));
    assert_eq!(app.compose_state().kit_pads[0].name, "Kick Teppich");

    // A new report from the first track's drums leaves it alone.
    let mut changed = garage_info();
    changed.pads[0].name = "Other Kick".into();
    app.test_apply_engine_event(AudioEvent::PluginKitInfo {
        instance_id: INSTANCE,
        info: changed,
    });
    assert_eq!(app.compose_state().kit_name.as_deref(), Some("Drummica"));
    assert_eq!(app.compose_state().kit_pads[0].name, "Kick Teppich");
}

#[test]
fn a_new_kit_on_the_drums_updates_the_picker_without_reopening_it() {
    let mut app = app_with_drums("Garage");
    open_manager(&mut app);
    assert_eq!(app.compose_state().kit_name.as_deref(), Some("Garage"));
    assert_eq!(
        app.compose_state().kit_pads,
        default_kit_pads(),
        "no pads reported yet: the table"
    );

    // The user picks another kit: the drums' text rescan names it, and the
    // kit-info report follows with its pads.
    app.test_apply_engine_event(AudioEvent::PluginParamValuesChanged {
        instance_id: INSTANCE,
        values: vec![ParamValueUpdate {
            id: resonance_plugin::stable_hash("kit_select"),
            value: 0.0,
            text: "Drummica".to_owned(),
        }],
    });
    app.test_apply_engine_event(AudioEvent::PluginKitInfo {
        instance_id: INSTANCE,
        info: garage_info(),
    });
    let compose = app.compose_state();
    assert_eq!(compose.kit_name.as_deref(), Some("Drummica"));
    assert_eq!(compose.kit_pads[0].name, "Bass Drum");

    // Assigning a pad takes the kit's name for it.
    let group_id = compose.drum_patterns[0].groups[0].id;
    let _ = app.update(Message::Compose(ComposeMessage::DrumGroups(
        DrumGroupsMessage::ManagerSelectGroup { group_id },
    )));
    let kick = GM_PADS[0].note;
    let groups = &app.compose_state().drum_patterns[0].groups;
    let already = groups.iter().any(|g| g.pads.iter().any(|p| p.note == kick));
    let toggle = |app: &mut Resonance| {
        let _ = app.update(Message::Compose(ComposeMessage::DrumGroups(
            DrumGroupsMessage::TogglePadAssignment {
                group_id,
                note: kick,
            },
        )));
    };
    toggle(&mut app);
    if already
        && !app.compose_state().drum_patterns[0].groups[0]
            .pads
            .iter()
            .any(|p| p.note == kick)
    {
        // It was in this group and the toggle took it out: put it back.
        toggle(&mut app);
    }
    let pad = app.compose_state().drum_patterns[0].groups[0]
        .pads
        .iter()
        .find(|p| p.note == kick)
        .cloned()
        .expect("the kick is in the group");
    assert_eq!(pad.name, "Bass Drum");
}
