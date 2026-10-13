//! `VoicebankRegistry` scans (vocal-mcp-control-and-quality.md §4.2–4.3)
//! over temp-dir fixture banks: configs and dictionaries only, no ONNX.

use std::path::{Path, PathBuf};

use resonance_app::compose::vocal_svs::{BankProfile, VoicebankRegistry, VoicebankRoots};
use resonance_app::{write_voicebank_fixture, VoicebankFixture};
use resonance_music_theory::{VoiceType, VoicebankId};

fn fresh_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join(format!(
            "resonance-voicebank-registry-{}",
            std::process::id()
        ))
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(dir: &Path, rel: &str, body: &str) {
    let path = dir.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

fn registry(banks: &Path, vocoders: Option<&Path>) -> VoicebankRegistry {
    VoicebankRegistry::scan(VoicebankRoots {
        banks: Some(banks.to_path_buf()),
        vocoders: vocoders.map(Path::to_path_buf),
        dev_models: Vec::new(),
    })
}

#[test]
fn scans_user_banks_by_folder_slug_and_skips_non_banks() {
    let root = fresh_dir("slugs");
    write_voicebank_fixture(&root.join("My Bank_v2"), &VoicebankFixture::default());
    std::fs::create_dir_all(root.join("not-a-bank/docs")).unwrap();
    write(&root, "stray.txt", "x");

    let reg = registry(&root, None);
    let ids: Vec<&str> = reg.installed().map(|b| b.id.as_str()).collect();
    assert_eq!(ids, ["my-bank-v2"]);
    let bank = reg.get("my-bank-v2").unwrap();
    assert!(bank.user);
    assert!(bank.vocoder_config.is_some());
}

#[test]
fn finds_an_acoustic_config_two_levels_down() {
    // Meiji's layout: configs/configs/dsconfig.yaml with the files a
    // level up, a languages.json named by the config, en/ phonemes.
    let root = fresh_dir("nested");
    let bank = root.join("lunai");
    write(
        &bank,
        "configs/configs/dsconfig.yaml",
        "acoustic: ../../files/acoustic.onnx\nphonemes: ../../files/phonemes.txt\n\
         languages: ../../files/languages.json\nvocoder: dsvocoder\nuse_lang_id: true\n\
         use_tension_embed: true\nspeakers:\n- embeds/standard\n- embeds/whisper\n",
    );
    write(
        &bank,
        "configs/configs/dsdur/dsconfig.yaml",
        "dur: dur.onnx\n",
    );
    let mut phonemes = vec!["AP".to_string(), "SP".into(), "hh".into()];
    for ph in resonance_music_theory::g2p::ARPABET_PHONEMES {
        if *ph != "hh" {
            phonemes.push(format!("en/{ph}"));
            phonemes.push(format!("ja/{ph}"));
        }
    }
    write(&bank, "files/phonemes.txt", &phonemes.join("\n"));
    write(&bank, "files/languages.json", r#"{"ja": 1, "en": 4}"#);
    write(
        &bank,
        "configs/configs/dsvocoder/vocoder.yaml",
        "name: v\nmodel: v.onnx\n",
    );
    write(&bank, "configs/configs/dsvocoder/v.onnx", "");

    let reg = registry(&root, None);
    let profile = reg.profile(&VoicebankId::new("lunai"));
    let installed = profile.installed().expect("found two levels down");
    assert!(installed
        .acoustic_config
        .ends_with("configs/configs/dsconfig.yaml"));
    assert!(profile.english());
    assert!(profile.missing_phonemes().is_empty());
    assert_eq!(profile.phoneme_name("ah"), "en/ah");
    assert_eq!(profile.phoneme_name("hh"), "hh");
    assert_eq!(profile.language_id("ah"), Some(4), "the bank's id for en");
    assert_eq!(profile.language_id("hh"), Some(0), "the universal bucket");
    let speakers: Vec<String> = profile.speakers().into_iter().map(|s| s.id).collect();
    assert_eq!(speakers, ["standard", "whisper"]);
    let target = profile
        .render_target(Some("whisper"), VoiceType::Alto, None)
        .unwrap();
    assert_eq!(target.speaker.as_deref(), Some("embeds/whisper"));
}

#[test]
fn a_bank_naming_a_shared_vocoder_finds_it_in_the_vocoder_folder() {
    let root = fresh_dir("shared-vocoder");
    let banks = root.join("voicebanks");
    let vocoders = root.join("vocoders");
    write(
        &banks.join("solo"),
        "dsconfig.yaml",
        "phonemes: phonemes.txt\nacoustic: acoustic.onnx\nvocoder: pc_nsf_hifigan_2025\n",
    );
    write(
        &banks.join("solo"),
        "phonemes.txt",
        &resonance_music_theory::g2p::ARPABET_PHONEMES.join("\n"),
    );

    // Not there yet: listed, but its render is `voicebank_missing`, naming
    // the vocoder.
    let reg = registry(&banks, Some(&vocoders));
    let profile = reg.profile(&VoicebankId::new("solo"));
    assert!(profile.installed().unwrap().vocoder_config.is_none());
    let missing = profile
        .render_target(None, VoiceType::Alto, None)
        .unwrap_err();
    assert_eq!(missing.vocoder.as_deref(), Some("pc_nsf_hifigan_2025"));

    // Installed under a folder of another name: found by its `name`.
    write(
        &vocoders,
        "pc-nsf/vocoder.yaml",
        "name: pc_nsf_hifigan_2025\nmodel: m.onnx\n",
    );
    write(&vocoders, "pc-nsf/m.onnx", "");
    let reg = registry(&banks, Some(&vocoders));
    let profile = reg.profile(&VoicebankId::new("solo"));
    assert!(profile.render_target(None, VoiceType::Alto, None).is_ok());
}

#[test]
fn a_user_bank_wins_over_a_development_one_with_the_same_id() {
    let root = fresh_dir("dev-vs-user");
    let models = root.join("models");
    write_voicebank_fixture(
        &models.join("voicebanks/lilia"),
        &VoicebankFixture::shipped("lilia"),
    );
    write_voicebank_fixture(&root.join("user/lilia"), &VoicebankFixture::default());

    let dev_only = VoicebankRegistry::scan(VoicebankRoots {
        dev_models: vec![models.clone()],
        ..VoicebankRoots::default()
    });
    let lilia = dev_only.get("lilia").expect("the dev layout's fixed id");
    assert!(!lilia.user);
    assert_eq!(lilia.name, "LIEE Lilia", "the built-in display name");

    let both = VoicebankRegistry::scan(VoicebankRoots {
        banks: Some(root.join("user")),
        dev_models: vec![models],
        ..VoicebankRoots::default()
    });
    assert!(both.get("lilia").unwrap().user);
}

#[test]
fn rescan_picks_up_installs_and_deletions() {
    let root = fresh_dir("rescan");
    let mut reg = registry(&root, None);
    assert_eq!(reg.installed().count(), 0);
    write_voicebank_fixture(&root.join("fresh"), &VoicebankFixture::default());
    reg.rescan();
    assert!(reg.get("fresh").is_some());
    std::fs::remove_dir_all(root.join("fresh")).unwrap();
    reg.rescan();
    assert!(reg.get("fresh").is_none());
}

#[test]
fn a_bank_that_is_not_installed_answers_from_the_built_in_table() {
    let tiger = BankProfile::not_installed("tiger");
    assert!(!tiger.is_installed());
    assert_eq!(
        tiger.default_speaker(VoiceType::Soprano).as_deref(),
        Some("glam")
    );
    assert_eq!(
        tiger.default_speaker(VoiceType::Bass).as_deref(),
        Some("mystic")
    );
    // TIGER is a male voice: a pop line sits C3..A4.
    let range = tiger.comfortable_range(None);
    assert_eq!((range.low, range.high), (48, 69));
    assert_eq!(tiger.comfortable_range(Some("glam")), range);
    // Lilia is an alto whose words blur as she climbs; Meiji sits low.
    let lilia = BankProfile::not_installed("lilia").comfortable_range(None);
    assert_eq!((lilia.low, lilia.high), (50, 70));
    let meiji = BankProfile::not_installed("meiji").comfortable_range(Some("phantom"));
    assert_eq!((meiji.low, meiji.high), (47, 66));

    let unknown = BankProfile::not_installed("someone");
    assert_eq!(unknown.comfortable_range(None).source.as_str(), "default");
    assert!(unknown.speakers().is_empty());
    let missing = unknown
        .render_target(None, VoiceType::Alto, Some("/x/someone".into()))
        .unwrap_err();
    assert!(missing.to_string().contains("\"someone\" is not installed"));
}

/// A LUNAI v170 bank as its zip unpacks (Katyusha, Keiro Revenant, Liam
/// Thorne, Allen Crow): the acoustic config in `configs/`, the models and
/// lists in `files/`, the bank's own dur / pitch / variance models (the
/// duration model with only the `standard` speaker), the English
/// dictionary in `configs/dsdur/`, the vocoder in `tgm_hifigan/`.
fn write_lunai_v170_bank(bank: &Path, speakers: &[&str]) {
    let speaker_list: String = speakers.iter().map(|s| format!("- embeds/{s}\n")).collect();
    write(
        bank,
        "configs/dsconfig.yaml",
        &format!(
            "acoustic: ../files/acoustic.onnx\nphonemes: ../files/phonemes.json\n\
             languages: ../files/languages.json\nvocoder: dsvocoder\nhidden_size: 384\n\
             use_lang_id: true\nuse_energy_embed: false\nuse_breathiness_embed: false\n\
             use_tension_embed: true\nspeakers:\n{speaker_list}"
        ),
    );
    for (folder, model) in [("dsdur", "dur"), ("dspitch", "pitch"), ("dsvariance", "variance")] {
        let model_speakers = if folder == "dsdur" {
            "- embeds/standard\n".to_owned()
        } else {
            speaker_list.clone()
        };
        write(
            bank,
            &format!("configs/{folder}/dsconfig.yaml"),
            &format!(
                "{model}: ../../files/{model}.onnx\nlinguistic: ../../files/linguistic-{model}.onnx\n\
                 phonemes: ../../files/phonemes-variance.json\n\
                 languages: ../../files/languages.json\nhidden_size: 384\nuse_lang_id: true\n\
                 speakers:\n{model_speakers}"
            ),
        );
        write(bank, &format!("files/{model}.onnx"), "");
        write(bank, &format!("files/linguistic-{model}.onnx"), "");
    }
    let mut phonemes = vec![r#""AP": 1"#.to_string(), r#""SP": 2"#.into(), r#""hh": 3"#.into()];
    let mut id = 4;
    for ph in resonance_music_theory::g2p::ARPABET_PHONEMES {
        if *ph != "hh" {
            phonemes.push(format!(r#""en/{ph}": {id}"#));
            phonemes.push(format!(r#""ru/{ph}": {}"#, id + 1));
            id += 2;
        }
    }
    let phonemes = format!("{{{}}}", phonemes.join(", "));
    write(bank, "files/phonemes.json", &phonemes);
    write(bank, "files/phonemes-variance.json", &phonemes);
    write(
        bank,
        "files/languages.json",
        r#"{"ar": 1, "de": 2, "en": 3, "ja": 7, "ru": 11, "zh": 14}"#,
    );
    write(bank, "files/acoustic.onnx", "");
    write(
        bank,
        "configs/dsdur/dsdict-en.yaml",
        "symbols:\n- {symbol: en/ah, type: vowel}\nentries:\n- {grapheme: walk, phonemes: [w, ao, k]}\n\
         replacements:\n- {from: ah, to: en/ah}\n- {from: hh, to: hh}\n- {from: q, to: en/cl}\n",
    );
    write(
        bank,
        "configs/dsvocoder/vocoder.yaml",
        "name: tgm_hifigan_v110\nmodel: ../../tgm_hifigan/tgm_hifigan_v110.onnx\n",
    );
    write(bank, "tgm_hifigan/tgm_hifigan_v110.onnx", "");
    write(bank, "configs/character.yaml", "name: Some Singer v170\n");
    for s in speakers {
        write(bank, &format!("configs/embeds/{s}.emb"), "");
    }
}

/// The four LUNAI English singers register under their folder names —
/// the ids their plok.org entries install into (`keiro_revenant`, not the
/// slug `keiro-revenant`) — with the built-in table's names, mode names,
/// ranges and terms over what their files say.
#[test]
fn lunai_v170_banks_register_under_their_folder_ids_with_the_builtin_facts() {
    let root = fresh_dir("lunai-v170");
    write_lunai_v170_bank(&root.join("katyusha"), &["standard", "strong", "delicate", "lullaby"]);
    write_lunai_v170_bank(
        &root.join("keiro_revenant"),
        &[
            "standard", "native", "cold", "fragile", "mellow", "aggressive", "solid", "vf",
            "whisper", "growl",
        ],
    );
    write_lunai_v170_bank(
        &root.join("liam_thorne"),
        &[
            "standard", "gentle", "tired", "deep", "powerful", "aggressive", "rock", "crispy", "vf",
            "growl",
        ],
    );
    write_lunai_v170_bank(
        &root.join("allen_crow"),
        &["standard", "heavy", "open", "gentle", "whisper"],
    );

    let reg = registry(&root, None);
    let ids: Vec<&str> = reg.installed().map(|b| b.id.as_str()).collect();
    assert_eq!(ids, ["allen_crow", "katyusha", "keiro_revenant", "liam_thorne"]);
    for (id, name, speakers) in [
        ("katyusha", "Katyusha", 4),
        ("keiro_revenant", "Keiro Revenant", 10),
        ("liam_thorne", "Liam Thorne", 10),
        ("allen_crow", "Allen Crow", 5),
    ] {
        let profile = reg.profile(&VoicebankId::new(id));
        let bank = profile.installed().expect(id);
        assert_eq!(profile.name(), name);
        assert!(profile.english(), "{id} sings English");
        assert!(profile.missing_phonemes().is_empty(), "{id}");
        assert_eq!(profile.phoneme_name("ah"), "en/ah", "{id}");
        assert_eq!(profile.phoneme_name("hh"), "hh", "{id}: the universal bucket");
        assert_eq!(profile.language_id("ah"), Some(3), "{id}: en is 3 in languages.json");
        assert_eq!(profile.language_id("AP"), Some(0), "{id}");
        assert!(profile.dictionary().is_some(), "{id}: dsdur/dsdict-en.yaml");
        let curves = profile.curve_support();
        assert!(curves.tension && !curves.energy && !curves.breathiness, "{id}");
        let models = profile.bank_models();
        assert!(
            models.duration.is_some() && models.pitch.is_some() && models.variance.is_some(),
            "{id}: all three of the bank's own models"
        );
        assert_eq!(bank.vocoder_name.as_deref(), Some("tgm_hifigan_v110"), "{id}");
        assert!(bank.vocoder_config.is_some(), "{id}: its bundled vocoder");
        assert_eq!(profile.speakers().len(), speakers, "{id}");
        assert_eq!(profile.default_speaker(VoiceType::Alto).as_deref(), Some("standard"));

        let builtin = profile.builtin().expect("a built-in row");
        let range = profile.comfortable_range(None);
        assert_eq!(range.source.as_str(), "builtin", "{id}");
        assert_eq!((range.low, range.high), builtin.range, "{id}");
        for (speaker, own) in builtin.speaker_ranges {
            let r = profile.comfortable_range(Some(speaker));
            assert_eq!((r.low, r.high), *own, "{id}/{speaker}");
        }

        let terms = reg.terms(id);
        assert_eq!(terms.credit, Some(format!("{name} from LUNAI Project")));
        assert_eq!(
            terms.terms_url.as_deref(),
            Some("https://lunaiproject.github.io/termsofuse/")
        );
        assert_eq!(terms.commercial.as_deref(), Some("permission"));
    }
    let keiro = reg.profile(&VoicebankId::new("keiro_revenant"));
    let fry = keiro.speakers().into_iter().find(|s| s.id == "vf").unwrap();
    assert_eq!(fry.name, "Vocal fry");
    assert!(fry.effect, "vocal fry is an effect mode");
    let target = keiro
        .render_target(Some("growl"), VoiceType::Tenor, None)
        .unwrap();
    assert_eq!(target.speaker.as_deref(), Some("embeds/growl"));

    // Not installed, a LUNAI id still validates like the bank it will be.
    let missing = BankProfile::not_installed("liam_thorne");
    assert_eq!(missing.name(), "Liam Thorne");
    assert_eq!(missing.phoneme_name("ah"), "en/ah");
    assert_eq!(missing.language_id("ah"), Some(3));
    assert!(missing.curve_support().tension);
}

/// The built-in table marks exactly the growl, vocal fry and whisper modes
/// as effect modes (Meiji's `phantom`, whispery but a lead, is not one),
/// and no bank's default speaker — for any voice type — is an effect mode.
#[test]
fn effect_modes_are_marked_and_never_a_banks_default() {
    use resonance_app::compose::vocal_svs::registry::BUILTIN_BANKS;

    let mut effects: Vec<(&str, &str)> = Vec::new();
    for bank in BUILTIN_BANKS {
        for s in bank.speakers.iter().filter(|s| s.effect) {
            effects.push((bank.id, s.id));
        }
        for voice in VoiceType::ALL.iter().copied() {
            let Some(default) = (bank.default_speaker)(voice) else {
                continue;
            };
            let speaker = bank
                .speakers
                .iter()
                .find(|s| s.id == default)
                .unwrap_or_else(|| panic!("{}: default {default:?} is not a speaker", bank.id));
            assert!(!speaker.effect, "{}: default {default:?} is an effect mode", bank.id);
        }
        // The not-installed profile (what a lane on a missing bank reports)
        // agrees with the table.
        let profile = BankProfile::not_installed(bank.id);
        if let Some(default) = profile.default_speaker(VoiceType::Alto) {
            let info = profile.speakers().into_iter().find(|s| s.id == default).unwrap();
            assert!(!info.effect, "{}: {default:?}", bank.id);
        }
    }
    assert_eq!(
        effects,
        [
            ("keiro_revenant", "vf"),
            ("keiro_revenant", "whisper"),
            ("keiro_revenant", "growl"),
            ("liam_thorne", "vf"),
            ("liam_thorne", "growl"),
            ("allen_crow", "whisper"),
        ]
    );
}

/// The Speaker picker lists a bank's lead modes in the bank's order, then
/// its effect modes under an `Effects ‹` prefix — here for Keiro Revenant
/// not installed (the picker's not-installed entry) and installed.
#[test]
fn the_speaker_picker_lists_effect_modes_last_under_effects() {
    let (mut app, _task) = resonance_app::Resonance::new_for_test();
    let expected: Vec<(String, String)> = [
        ("standard", "Standard"),
        ("native", "Native"),
        ("cold", "Cold"),
        ("fragile", "Fragile"),
        ("mellow", "Mellow"),
        ("aggressive", "Aggressive"),
        ("solid", "Solid"),
        ("vf", "Effects \u{2039} Vocal fry"),
        ("whisper", "Effects \u{2039} Whisper"),
        ("growl", "Effects \u{2039} Growl"),
    ]
    .iter()
    .map(|(id, label)| (id.to_string(), label.to_string()))
    .collect();
    assert_eq!(
        app.test_speaker_choices("keiro_revenant").as_ref(),
        Some(&expected),
        "not installed"
    );

    let speakers = expected.iter().rev().map(|(id, _)| format!("embeds/{id}")).collect();
    app.test_install_voicebank(
        "keiro_revenant",
        &VoicebankFixture {
            speakers,
            ..VoicebankFixture::default()
        },
    );
    assert_eq!(
        app.test_speaker_choices("keiro_revenant").as_ref(),
        Some(&expected),
        "installed: the table's order, whatever the model's"
    );
    // A bank without effect modes keeps its plain names.
    let tiger = app.test_speaker_choices("tiger").expect("tiger (not installed)");
    assert!(tiger.iter().all(|(_, label)| !label.contains("Effects")), "{tiger:?}");
}
