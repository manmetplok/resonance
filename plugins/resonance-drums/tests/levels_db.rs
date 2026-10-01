//! Levels in dB (drums-plugin-rework.md §7 E9, D6): the master, pad
//! volume and per-mic trim params read, parse and range in dB, −∞ at
//! the floor, and a v1 state's linear levels (`balance`, `oh_blend`
//! included) convert once on load, keeping the sound — under new ids
//! (`master_level`, `pad_N_level`), so a value the host re-sends by a v1
//! id after the state names no param and cannot undo the conversion.

use resonance_drums::drum_map::{self, NUM_PADS};
use resonance_drums::level::{self, db_to_gain, gain_to_db, MIN_DB};
use resonance_drums::params::{upgrade_v1_levels, DrumParams, MicSlot, MASTER_LEVEL_ID};
use resonance_drums::ResonanceDrums;
use resonance_plugin::{Param, ResonancePlugin};

#[test]
fn levels_display_and_parse_in_db() {
    let p = DrumParams::default();
    let vol = &p.pads[0].volume;
    assert_eq!(vol.default_plain(), 0.0, "pad volume defaults to 0 dB");
    assert_eq!(
        p.master_volume.default_plain(),
        0.0,
        "master defaults to 0 dB"
    );
    assert_eq!(vol.min_plain(), MIN_DB as f64);
    assert_eq!(vol.max_plain(), 6.0);
    assert_eq!(p.master_volume.max_plain(), 6.0);
    for trim in &p.pads[1].trims {
        assert_eq!(trim.default_plain(), 0.0);
        assert_eq!(trim.min_plain(), MIN_DB as f64);
        assert_eq!(trim.max_plain(), 12.0);
    }

    assert_eq!(vol.display(0.0), "0.0 dB");
    assert_eq!(vol.display(-0.04), "0.0 dB", "never -0.0");
    assert_eq!(vol.display(-6.02), "-6.0 dB");
    assert_eq!(vol.display(3.25), "+3.3 dB");
    assert_eq!(vol.display(MIN_DB as f64), "-inf dB");
    assert_eq!(p.master_volume.display(MIN_DB as f64), "-inf dB");

    assert_eq!(vol.parse("-6.0 dB"), Some(-6.0));
    assert_eq!(vol.parse("+3 dB"), Some(3.0));
    assert_eq!(vol.parse("-12"), Some(-12.0));
    assert_eq!(vol.parse("-inf dB"), Some(MIN_DB as f64));
    assert_eq!(vol.parse("loud"), None);
    for text in ["0.0 dB", "-6.0 dB", "+3.3 dB", "-inf dB"] {
        let v = vol.parse(text).unwrap();
        assert_eq!(vol.display(v), text, "{text} round-trips");
    }

    // The ids the trims answer to.
    let ids: Vec<&str> = p.pads[1].trims.iter().map(|t| t.id()).collect();
    assert_eq!(
        ids,
        [
            "pad_1_mic1_trim",
            "pad_1_mic2_trim",
            "pad_1_oh_trim",
            "pad_1_bleed_trim",
            "pad_1_room_trim",
        ]
    );
    // Pads never recorded with two close mics don't offer the second trim.
    let hat = drum_map::pad_index_for_note(drum_map::HIHAT_CLOSED).unwrap();
    assert!(p.pads[hat].trim(MicSlot::Close2).is_hidden());
    assert!(!p.pads[0].trim(MicSlot::Close2).is_hidden());
    assert!(!p.pads[hat].trim(MicSlot::Overhead).is_hidden());
}

#[test]
fn db_gain_conversion_is_exact_at_unity_and_silent_at_the_floor() {
    assert_eq!(db_to_gain(0.0), 1.0, "0 dB is exactly unity");
    assert_eq!(db_to_gain(MIN_DB), 0.0, "the floor is silence");
    assert_eq!(db_to_gain(-200.0), 0.0);
    assert!((db_to_gain(-6.0206) - 0.5).abs() < 1e-5);
    assert!((db_to_gain(6.0) - 1.9953).abs() < 1e-3);
    assert_eq!(gain_to_db(0.0), MIN_DB);
    assert_eq!(gain_to_db(1.0), 0.0);
    assert!((gain_to_db(0.8) - (-1.9382)).abs() < 1e-3);
    assert_eq!(level::db_label(gain_to_db(0.0)), "-inf dB");
}

/// A v1 project: linear volumes (0.8 default), a balance and an OH blend.
fn v1_state() -> serde_json::Value {
    let mut params = serde_json::Map::new();
    params.insert("master_volume".into(), 0.8.into());
    for i in 0..NUM_PADS {
        params.insert(format!("pad_{i}_volume"), 0.8.into());
        params.insert(format!("pad_{i}_pan"), 0.0.into());
        params.insert(format!("pad_{i}_balance"), 0.5.into());
        params.insert(format!("pad_{i}_oh_blend"), 1.0.into());
    }
    params.insert("pad_0_volume".into(), 0.5.into());
    params.insert("pad_0_balance".into(), 0.25.into());
    params.insert("pad_1_oh_blend".into(), 0.0.into());
    params.insert("pad_3_volume".into(), 0.0.into());
    serde_json::json!({ "version": 1, "params": params })
}

#[test]
fn a_v1_state_converts_linear_levels_to_db_once() {
    let mut drums = ResonanceDrums::new();
    assert!(drums.load_state(&serde_json::to_vec(&v1_state()).unwrap()));
    let p = &drums.bridge.params;
    let near = |a: f32, b: f32| (a - b).abs() < 1e-4;

    // Every level plays the gain it played in v1.
    assert!(near(db_to_gain(p.master_volume.value()), 0.8));
    assert!(near(db_to_gain(p.pads[0].volume.value()), 0.5));
    assert!(near(db_to_gain(p.pads[5].volume.value()), 0.8));
    assert_eq!(
        p.pads[3].volume.value(),
        MIN_DB,
        "a silent pad stays silent (-inf)"
    );
    // The kick's balance of 0.25: In at 0.75, Out at 0.25.
    assert!(near(
        db_to_gain(p.pads[0].trim(MicSlot::Close1).value()),
        0.75
    ));
    assert!(near(
        db_to_gain(p.pads[0].trim(MicSlot::Close2).value()),
        0.25
    ));
    // The snare's default 0.5 balance: both mics at half, as in v1.
    assert!(near(
        db_to_gain(p.pads[1].trim(MicSlot::Close1).value()),
        0.5
    ));
    assert!(near(
        db_to_gain(p.pads[1].trim(MicSlot::Close2).value()),
        0.5
    ));
    // OH blend 0 -> the OH trim at -inf; 1 -> 0 dB.
    assert_eq!(p.pads[1].trim(MicSlot::Overhead).value(), MIN_DB);
    assert_eq!(p.pads[0].trim(MicSlot::Overhead).value(), 0.0);
    // A one-close-mic pad ignored the balance in v1: its trim stays 0 dB.
    let hat = drum_map::pad_index_for_note(drum_map::HIHAT_CLOSED).unwrap();
    assert_eq!(p.pads[hat].trim(MicSlot::Close1).value(), 0.0);

    // Saved again, it is v2 (no balance, no oh_blend, no v1 volume ids)
    // and loads unchanged.
    let saved: serde_json::Value = serde_json::from_slice(&drums.save_state()).unwrap();
    let saved_params = saved["params"].as_object().unwrap();
    assert!(saved_params.keys().all(|k| !k.ends_with("_balance")
        && !k.ends_with("_oh_blend")
        && !k.ends_with("_volume")));
    assert!(saved_params.contains_key(MASTER_LEVEL_ID));
    assert!(saved_params.contains_key("pad_0_level"));
    let mut again = saved.clone();
    assert!(
        !upgrade_v1_levels(&mut again),
        "a v2 state is not converted again"
    );
    assert_eq!(again, saved);
    let mut reloaded = ResonanceDrums::new();
    assert!(reloaded.load_state(&serde_json::to_vec(&saved).unwrap()));
    assert_eq!(
        reloaded.bridge.params.pads[0].volume.value(),
        p.pads[0].volume.value()
    );
    assert_eq!(
        reloaded.bridge.params.master_volume.value(),
        p.master_volume.value()
    );
}

#[test]
fn a_typographic_minus_parses() {
    assert_eq!(level::db_from_label("\u{2212}6 dB"), Some(-6.0));
    assert_eq!(level::db_from_label(" \u{2212}12.5dB "), Some(-12.5));
    assert_eq!(level::db_from_label("\u{2212}inf dB"), Some(MIN_DB));
    assert_eq!(level::db_from_label("-3"), Some(-3.0));
}

#[test]
fn a_state_without_params_is_left_alone() {
    let mut no_params = serde_json::json!({ "kit_ref": null });
    assert!(!upgrade_v1_levels(&mut no_params));
    let mut v2 = serde_json::json!({ "params": { "master_level": -3.0, "pad_0_level": -1.0 } });
    let before = v2.clone();
    assert!(!upgrade_v1_levels(&mut v2));
    assert_eq!(v2, before);
}

/// A state from a build that had dB levels under the v1 ids (K7
/// development builds: no `balance`, so not v1) moves its values to the
/// new ids as they are — they are dB already.
#[test]
fn a_db_state_under_the_old_ids_moves_without_converting() {
    let mut k7 = serde_json::json!({ "params": {
        "master_volume": -3.0, "pad_2_volume": -12.0, "pad_2_pan": 0.5,
    } });
    assert!(upgrade_v1_levels(&mut k7));
    assert_eq!(
        k7,
        serde_json::json!({ "params": {
            "master_level": -3.0, "pad_2_level": -12.0, "pad_2_pan": 0.5,
        } })
    );
}

/// What reopening a v1 project does (ferrous.rproj, post-metal-1.rproj):
/// the app loads the plugin state, then re-sends the project's saved
/// param values by id (`apply_pending_param_overrides`: a value whose id
/// the plugin no longer declares is skipped). Those values are v1's
/// linear gains under `master_volume` / `pad_N_volume`; they used to land
/// on the dB params after the conversion — 0.8 "dB" for a 0.8 gain.
#[test]
fn a_v1_projects_stale_overrides_cannot_undo_the_conversion() {
    let mut drums = ResonanceDrums::new();
    assert!(drums.load_state(&serde_json::to_vec(&v1_state()).unwrap()));
    // The app's override pass: by id, skipping what the plugin lacks.
    let overrides = [("master_volume", 0.8), ("pad_0_volume", 0.5), ("pad_5_volume", 0.8)];
    let mut applied = 0;
    for (id, value) in overrides {
        if let Some(param) = (0..drums.param_count())
            .map(|i| drums.param(i))
            .find(|p| p.id() == id)
        {
            param.set_plain(value);
            applied += 1;
        }
    }
    assert_eq!(applied, 0, "no v1 level id names a param any more");
    let p = &drums.bridge.params;
    let near = |a: f32, b: f32| (a - b).abs() < 1e-4;
    assert!(near(db_to_gain(p.master_volume.value()), 0.8));
    assert!(near(db_to_gain(p.pads[0].volume.value()), 0.5));
    assert!(near(db_to_gain(p.pads[5].volume.value()), 0.8));
}
