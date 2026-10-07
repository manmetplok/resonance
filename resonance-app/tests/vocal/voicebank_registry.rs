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
    let range = tiger.comfortable_range(None);
    assert_eq!((range.low, range.high), (48, 76));

    let unknown = BankProfile::not_installed("someone");
    assert_eq!(unknown.comfortable_range(None).source.as_str(), "default");
    assert!(unknown.speakers().is_empty());
    let missing = unknown
        .render_target(None, VoiceType::Alto, Some("/x/someone".into()))
        .unwrap_err();
    assert!(missing.to_string().contains("\"someone\" is not installed"));
}
