//! Pins the vocal parameter enums' variant counts and user-visible
//! `as_str()` labels. The labels show up verbatim in the UI chips and
//! must never drift when the enums are reshaped (e.g. the strum
//! `VariantArray` / `IntoStaticStr` derives replacing hand-rolled
//! arrays and match arms).

use resonance_music_theory::{
    SyllableMode, VocalContour, VocalMood, VocalPov, VocalRhymeScheme, VocalSinger,
    VocalSingerMeiji, VocalStyle, VocalTimbre, VocalVoicebank, VoiceType,
};

#[test]
fn variant_counts_are_stable() {
    assert_eq!(VocalMood::ALL.len(), 6);
    assert_eq!(VocalPov::ALL.len(), 5);
    assert_eq!(VocalRhymeScheme::ALL.len(), 5);
    assert_eq!(VoiceType::ALL.len(), 6);
    assert_eq!(SyllableMode::ALL.len(), 3);
    assert_eq!(VocalContour::ALL.len(), 5);
    assert_eq!(VocalStyle::ALL.len(), 6);
    assert_eq!(VocalVoicebank::ALL.len(), 3);
    assert_eq!(VocalSinger::ALL.len(), 7);
    assert_eq!(VocalSingerMeiji::ALL.len(), 4);
    assert_eq!(VocalTimbre::ALL.len(), 4);
}

#[test]
fn mood_labels_match_variant_names() {
    let labels: Vec<&str> = VocalMood::ALL.iter().map(|m| m.as_str()).collect();
    assert_eq!(
        labels,
        ["Yearning", "Defiant", "Hopeful", "Reflective", "Joyful", "Melancholy"]
    );
}

#[test]
fn pov_labels_use_ordinal_spellings() {
    let labels: Vec<&str> = VocalPov::ALL.iter().map(|p| p.as_str()).collect();
    assert_eq!(
        labels,
        ["1st singular", "1st plural", "2nd person", "3rd person", "Narrator"]
    );
}

#[test]
fn rhyme_scheme_labels_are_uppercase() {
    let labels: Vec<&str> = VocalRhymeScheme::ALL.iter().map(|r| r.as_str()).collect();
    assert_eq!(labels, ["AABB", "ABAB", "ABCB", "ABBA", "Free"]);
}

#[test]
fn voice_type_labels_shorten_mezzo() {
    let labels: Vec<&str> = VoiceType::ALL.iter().map(|v| v.as_str()).collect();
    assert_eq!(
        labels,
        ["Soprano", "Mezzo", "Alto", "Tenor", "Baritone", "Bass"]
    );
}

#[test]
fn syllable_mode_labels_match_variant_names() {
    let labels: Vec<&str> = SyllableMode::ALL.iter().map(|m| m.as_str()).collect();
    assert_eq!(labels, ["Syllabic", "Mixed", "Melismatic"]);
}

#[test]
fn contour_labels_match_variant_names() {
    let labels: Vec<&str> = VocalContour::ALL.iter().map(|c| c.as_str()).collect();
    assert_eq!(labels, ["Arch", "Rise", "Fall", "Wave", "Flat"]);
}

#[test]
fn style_labels_spell_pop_ballad_with_a_space() {
    let labels: Vec<&str> = VocalStyle::ALL.iter().map(|s| s.as_str()).collect();
    assert_eq!(
        labels,
        ["Pop ballad", "Conversational", "Hymnal", "Folk", "Anthemic", "Chant"]
    );
}

#[test]
fn voicebank_labels_uppercase_tiger() {
    let labels: Vec<&str> = VocalVoicebank::ALL.iter().map(|v| v.as_str()).collect();
    assert_eq!(labels, ["TIGER", "Lilia", "Meiji"]);
}

#[test]
fn singer_labels_match_variant_names() {
    let labels: Vec<&str> = VocalSinger::ALL.iter().map(|s| s.as_str()).collect();
    assert_eq!(
        labels,
        ["Glam", "Fresh", "Disco", "Royal", "Electric", "Mystic", "Vinyl"]
    );
}

#[test]
fn meiji_singer_labels_match_variant_names() {
    let labels: Vec<&str> = VocalSingerMeiji::ALL.iter().map(|s| s.as_str()).collect();
    assert_eq!(labels, ["Standard", "Hunter", "Lilith", "Phantom"]);
}

#[test]
fn timbre_labels_match_variant_names() {
    let labels: Vec<&str> = VocalTimbre::ALL.iter().map(|t| t.as_str()).collect();
    assert_eq!(labels, ["Airy", "Warm", "Edged", "Bright"]);
}

#[test]
fn serde_uses_variant_idents_not_strum_labels() {
    // Project files persist these enums through serde, which serializes
    // the Rust variant ident — strum's `serialize = "..."` display
    // labels must not leak into the JSON.
    let json = serde_json::to_string(&VocalStyle::PopBallad).expect("serialize");
    assert_eq!(json, "\"PopBallad\"");
    let json = serde_json::to_string(&VocalVoicebank::Tiger).expect("serialize");
    assert_eq!(json, "\"Tiger\"");
    let json = serde_json::to_string(&VocalPov::FirstSingular).expect("serialize");
    assert_eq!(json, "\"FirstSingular\"");
    let json = serde_json::to_string(&VoiceType::MezzoSoprano).expect("serialize");
    assert_eq!(json, "\"MezzoSoprano\"");
}
