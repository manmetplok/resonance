//! Voice presets (`docs/features/compose/voice-presets.md`): the built-in
//! table seeded from the 41-voice blind listen, what a preset sets on a
//! lane, and the user preset folder.

use resonance_app::compose::vocal_svs::registry::{builtin::builtin, BUILTIN_BANKS};
use resonance_app::compose::vocal_svs::voice_presets::builtin::{
    bank_mean_score, nearest_voice_type, AUDITION_SCORES, MIN_PRESET_SCORE,
};
use resonance_app::compose::vocal_svs::voice_presets::store::machine_dir;
use resonance_app::compose::vocal_svs::voice_presets::{
    self, builtin_presets, VoicePresetStore, VoiceSettings,
};
use resonance_app::compose::vocal_svs::BankProfile;
use resonance_music_theory::{
    VocalCurvePoint, VocalParams, VocalStyle, VocalTimbre, VoiceType, VoicebankId,
};

#[test]
fn the_audition_covers_every_offered_voice_once() {
    assert_eq!(AUDITION_SCORES.len(), 41);
    for (bank, speaker, score) in AUDITION_SCORES {
        assert!((1..=5).contains(score), "{bank}/{speaker:?}: score {score}");
        let row = builtin(bank).unwrap_or_else(|| panic!("{bank} is not a built-in bank"));
        match speaker {
            Some(id) => assert!(
                row.speakers.iter().any(|s| s.id == *id),
                "{bank} has no speaker {id}"
            ),
            None => assert!(row.speakers.is_empty(), "{bank} has speakers"),
        }
    }
    let mut keys: Vec<_> = AUDITION_SCORES.iter().map(|(b, s, _)| (*b, *s)).collect();
    keys.sort();
    keys.dedup();
    assert_eq!(keys.len(), 41, "a voice is scored twice");
    // The bank means the follow-ups doc records.
    for (bank, mean) in [
        ("tiger", 4.0),
        ("katyusha", 3.0),
        ("liam_thorne", 3.0),
        ("lilia", 3.0),
        ("meiji", 2.75),
        ("allen_crow", 2.6),
        ("keiro_revenant", 2.5),
    ] {
        assert!((bank_mean_score(bank) - mean).abs() < 1e-4, "{bank}");
    }
}

#[test]
fn built_ins_are_exactly_the_voices_scored_three_or_more_best_first() {
    let presets = builtin_presets();
    let mut wanted: Vec<(String, Option<String>, u8)> = AUDITION_SCORES
        .iter()
        .filter(|(_, _, s)| *s >= MIN_PRESET_SCORE)
        .map(|(b, s, score)| (b.to_string(), s.map(str::to_owned), *score))
        .collect();
    let mut got: Vec<(String, Option<String>, u8)> = presets
        .iter()
        .map(|p| {
            (
                p.settings.voicebank.to_string(),
                p.settings.speaker.clone(),
                p.score.expect("a built-in carries its score"),
            )
        })
        .collect();
    assert_eq!(presets.len(), 31);
    wanted.sort();
    got.sort();
    assert_eq!(got, wanted);

    // Best first: score, then the bank's mean, then name.
    for pair in presets.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        let key = |p: &voice_presets::VoicePreset| {
            (
                p.score.unwrap(),
                bank_mean_score(p.settings.voicebank.as_str()),
            )
        };
        let (ka, kb) = (key(a), key(b));
        assert!(
            ka.0 > kb.0
                || (ka.0 == kb.0 && ka.1 > kb.1)
                || (ka.0 == kb.0 && ka.1 == kb.1 && a.name.to_lowercase() <= b.name.to_lowercase()),
            "{} before {}",
            a.name,
            b.name
        );
    }
    assert_eq!(presets[0].id, "tiger-mystic");
    assert_eq!(presets[0].name, "TIGER Mystic");
    assert_eq!(presets[0].score, Some(5));
    assert!(presets.iter().all(|p| p.builtin));

    let mut ids: Vec<&str> = presets.iter().map(|p| p.id.as_str()).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), presets.len(), "preset ids are unique");
    let mut names: Vec<String> = presets.iter().map(|p| p.name.to_lowercase()).collect();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), presets.len(), "preset names are unique");
}

#[test]
fn no_built_in_is_an_effect_mode() {
    for p in builtin_presets() {
        let bank = builtin(p.settings.voicebank.as_str()).expect("a built-in bank");
        if let Some(id) = &p.settings.speaker {
            let speaker = bank
                .speakers
                .iter()
                .find(|s| s.id == id)
                .expect("a known speaker");
            assert!(!speaker.effect, "{} is an effect mode", p.name);
        }
    }
    // And every effect mode scored below the cut.
    for bank in BUILTIN_BANKS {
        for s in bank.speakers.iter().filter(|s| s.effect) {
            assert!(
                !builtin_presets()
                    .iter()
                    .any(|p| p.settings.voicebank == bank.id
                        && p.settings.speaker.as_deref() == Some(s.id)),
                "{}/{} became a preset",
                bank.id,
                s.id
            );
        }
    }
}

#[test]
fn built_ins_sing_their_comfortable_range_with_the_nearest_voice_type() {
    for p in builtin_presets() {
        let profile = BankProfile::not_installed(p.settings.voicebank.as_str());
        let comfortable = profile.comfortable_range(p.settings.speaker.as_deref());
        let (lo, hi) = p.settings.range;
        assert!(
            lo >= comfortable.low && hi <= comfortable.high && lo < hi,
            "{}: {lo}..{hi} outside {}..{}",
            p.name,
            comfortable.low,
            comfortable.high
        );
        assert_eq!((lo, hi), (comfortable.low, comfortable.high), "{}", p.name);
        assert_eq!(p.settings.voice, nearest_voice_type((lo, hi)), "{}", p.name);
        p.settings
            .validate()
            .unwrap_or_else(|e| panic!("{}: {e}", p.name));
        assert!(
            p.tags
                .iter()
                .any(|t| ["low", "mid", "high"].contains(&t.as_str())),
            "{}",
            p.name
        );
    }
    let by_id = |id: &str| builtin_presets().iter().find(|p| p.id == id).unwrap();
    assert_eq!(by_id("tiger-mystic").settings.voice, VoiceType::Tenor);
    assert_eq!(
        by_id("liam_thorne-deep").settings.voice,
        VoiceType::Baritone
    );
    assert_eq!(by_id("katyusha-standard").settings.voice, VoiceType::Alto);
    // Allen Crow's open mode has its own, higher range.
    assert_eq!(by_id("allen_crow-open").settings.range, (55, 74));
    assert!(by_id("liam_thorne-rock").tags.contains(&"low".to_owned()));
    assert!(by_id("katyusha-strong").tags.contains(&"high".to_owned()));
}

#[test]
fn deliveries_are_defaults_with_a_light_character_fit() {
    let defaults = VoiceSettings::from_params(&VocalParams::default());
    let by_id = |id: &str| builtin_presets().iter().find(|p| p.id == id).unwrap();
    // A plain mode sings exactly as auditioned: a new lane's delivery.
    for id in [
        "tiger-mystic",
        "lilia",
        "katyusha-standard",
        "keiro_revenant-native",
    ] {
        let s = &by_id(id).settings;
        assert_eq!(s.tension, defaults.tension, "{id}");
        assert_eq!(s.articulation, defaults.articulation, "{id}");
        assert_eq!(s.vibrato, defaults.vibrato, "{id}");
        assert_eq!(s.velocity_dynamics, defaults.velocity_dynamics, "{id}");
    }
    for p in builtin_presets() {
        let s = &p.settings;
        assert_eq!(s.timbre, VocalTimbre::Warm, "{}", p.name);
        assert_eq!(s.vibrato_rate, defaults.vibrato_rate, "{}", p.name);
        assert_eq!(s.portamento_ms, defaults.portamento_ms, "{}", p.name);
        assert_eq!(
            s.rest_threshold_beats, defaults.rest_threshold_beats,
            "{}",
            p.name
        );
        assert!(s.bank_timing && s.bank_pitch, "{}", p.name);
        assert!(
            (0.4..=0.6).contains(&s.consonant_emphasis),
            "{}: intelligible band",
            p.name
        );
        // Modest: no slider moves more than 0.4 from the default.
        for (field, v, d) in [
            ("tension", s.tension, defaults.tension),
            ("vibrato", s.vibrato, defaults.vibrato),
            ("articulation", s.articulation, defaults.articulation),
            (
                "velocity_dynamics",
                s.velocity_dynamics,
                defaults.velocity_dynamics,
            ),
        ] {
            assert!(
                (v - d).abs() <= 0.4 + 1e-6,
                "{}: {field} {v} vs {d}",
                p.name
            );
        }
    }
    let gentle = &by_id("liam_thorne-gentle").settings;
    let rock = &by_id("liam_thorne-rock").settings;
    assert!(gentle.tension < defaults.tension && rock.tension > defaults.tension);
    assert!(gentle.velocity_dynamics < rock.velocity_dynamics);
    assert!(by_id("meiji-phantom").settings.vibrato < defaults.vibrato);
    assert!(by_id("liam_thorne-gentle")
        .tags
        .contains(&"gentle".to_owned()));
}

/// A lane with everything a preset must leave alone set away from the
/// defaults.
fn finished_part() -> VocalParams {
    let mut p = VocalParams::default();
    p.style = VocalStyle::Folk;
    p.theme = "a finished part".into();
    p.leap_range = 0.4;
    p.lines = 6;
    p.draft[0].text = "my own words".into();
    p.curves.dynamics.push(VocalCurvePoint {
        beat: 1.0,
        value: 0.25,
        stepped: false,
    });
    p.tension = -0.3;
    p.timbre = VocalTimbre::Bright;
    p
}

#[test]
fn applying_sets_exactly_the_voice_and_delivery() {
    let preset = builtin_presets()
        .iter()
        .find(|p| p.id == "liam_thorne-rock")
        .unwrap();
    let before = finished_part();
    let mut after = before.clone();
    preset.settings.apply_to(&mut after);

    assert!(preset.settings.matches(&after));
    assert_eq!(VoiceSettings::from_params(&after), preset.settings);
    assert_eq!(after.voicebank, VoicebankId::new("liam_thorne"));
    assert_eq!(after.speaker.as_deref(), Some("rock"));
    // Everything else is the lane's own.
    let mut expected = before.clone();
    preset.settings.apply_to(&mut expected);
    assert_eq!(after.draft, before.draft);
    assert_eq!(after.curves, before.curves);
    assert_eq!(after.style, before.style);
    assert_eq!(after.theme, before.theme);
    assert_eq!(after.leap_range, before.leap_range);
    assert_eq!(after.lines, before.lines);
    assert!(!preset.settings.matches(&before));
    // `matches` sees one slider moved.
    let mut nudged = after.clone();
    nudged.vibrato += 0.01;
    assert!(!preset.settings.matches(&nudged));
}

#[test]
fn find_takes_an_id_or_a_name_and_matching_picks_the_first() {
    let user = vec![voice_presets::capture(
        "My Lead",
        String::new(),
        Vec::new(),
        &finished_part(),
    )];
    assert_eq!(
        voice_presets::find(&user, "tiger-mystic").unwrap().name,
        "TIGER Mystic"
    );
    assert_eq!(
        voice_presets::find(&user, "tiger mystic").unwrap().id,
        "tiger-mystic"
    );
    assert_eq!(
        voice_presets::find(&user, "user:My Lead").unwrap().name,
        "My Lead"
    );
    assert_eq!(
        voice_presets::find(&user, "my lead").unwrap().id,
        "user:My Lead"
    );
    assert!(voice_presets::find(&user, "nobody").is_none());
    assert_eq!(
        voice_presets::matching(&user, &finished_part()).map(|p| p.id.as_str()),
        Some("user:My Lead")
    );
    let mut mystic = VocalParams::default();
    builtin_presets()[0].settings.apply_to(&mut mystic);
    assert_eq!(
        voice_presets::matching(&user, &mystic).map(|p| p.id.as_str()),
        Some("tiger-mystic")
    );
}

#[test]
fn user_presets_save_list_rename_and_delete() {
    let dir = tempfile::tempdir().unwrap();
    let store = VoicePresetStore::new(Some(dir.path().join("voice-presets")));
    assert!(store.load().is_empty());

    let preset = voice_presets::capture(
        "Folk / lead",
        "for the verses".into(),
        vec!["folk".into()],
        &finished_part(),
    );
    store.save(&preset).unwrap();
    assert!(store.exists("Folk / lead"));
    let loaded = store.load();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0], preset, "a saved preset reads back as saved");
    assert_eq!(loaded[0].id, "user:Folk / lead");
    assert_eq!(loaded[0].score, None);
    assert!(!loaded[0].builtin);

    // Overwrite in place.
    let mut changed = preset.clone();
    changed.settings.vibrato = 0.6;
    store.save(&changed).unwrap();
    assert_eq!(store.load(), vec![changed.clone()]);

    store
        .save(&voice_presets::capture(
            "B",
            String::new(),
            Vec::new(),
            &VocalParams::default(),
        ))
        .unwrap();
    assert!(
        store.rename("Folk / lead", "B").is_err(),
        "a taken name is refused"
    );
    store.rename("Folk / lead", "A").unwrap();
    let names: Vec<String> = store.load().into_iter().map(|p| p.name).collect();
    assert_eq!(names, ["A", "B"]);
    assert_eq!(
        store.load()[0].settings,
        changed.settings,
        "a rename keeps the settings"
    );

    store.delete("A").unwrap();
    assert!(store.delete("A").is_err());
    let names: Vec<String> = store.load().into_iter().map(|p| p.name).collect();
    assert_eq!(names, ["B"]);
    assert!(store
        .save(&voice_presets::capture(
            "  ",
            String::new(),
            Vec::new(),
            &VocalParams::default()
        ))
        .is_err());
}

#[test]
fn a_corrupt_user_preset_is_quarantined_not_listed() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("broken.json"), "{ not json").unwrap();
    std::fs::write(
        dir.path().join("bad range.json"),
        r#"{"name": "bad range", "settings": {"range": [70, 60]}}"#,
    )
    .unwrap();
    // A file with only a name reads as a new lane's voice.
    std::fs::write(dir.path().join("bare.json"), r#"{"name": "bare"}"#).unwrap();
    let store = VoicePresetStore::new(Some(dir.path().to_path_buf()));
    let loaded = store.load();
    assert_eq!(loaded.len(), 1);
    assert_eq!(
        loaded[0].settings,
        VoiceSettings::from_params(&VocalParams::default())
    );
    assert!(!dir.path().join("broken.json").exists());
}

#[test]
fn the_machine_folder_honours_the_override() {
    use std::path::PathBuf;
    assert_eq!(
        machine_dir(
            Some(PathBuf::from("/x/presets")),
            Some(PathBuf::from("/data"))
        ),
        Some(PathBuf::from("/x/presets"))
    );
    assert_eq!(
        machine_dir(None, Some(PathBuf::from("/data"))),
        Some(PathBuf::from("/data/resonance/voice-presets"))
    );
    assert_eq!(machine_dir(None, None), None);
}
