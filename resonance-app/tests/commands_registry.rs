//! Unit coverage for the command registry (`resonance_app::commands`):
//! KeyChord parse/format round-tripping and glyph rendering, the fuzzy
//! subsequence matcher (score + highlight ranges), and the binding tables
//! (default + DAW presets, lookup-by-chord and lookup-by-id).
//!
//! Kept in a separate test file per the project convention of no inline
//! `#[cfg(test)]` modules.

use resonance_app::commands::{
    fuzzy_match, BindingMap, ChordKey, CommandCategory, CommandId, KeyChord, KeymapPreset, Mods,
    NamedKey,
};
use resonance_app::message::*;

// ---------------------------------------------------------------------------
// Registry metadata
// ---------------------------------------------------------------------------

#[test]
fn all_commands_have_metadata_and_unique_breadcrumbs() {
    let mut seen = std::collections::HashSet::new();
    for &id in &CommandId::ALL {
        assert!(
            !id.display_name().is_empty(),
            "{id:?} has an empty display name"
        );
        let bc = id.breadcrumb();
        assert!(
            bc.starts_with(id.category().display_name()),
            "{id:?} breadcrumb {bc:?} should start with its category"
        );
        assert!(
            seen.insert(id.display_name()),
            "duplicate display name: {:?}",
            id.display_name()
        );
    }
    assert_eq!(CommandId::ALL.len(), 37);
}

#[test]
fn every_category_has_at_least_one_command() {
    for cat in CommandCategory::ALL {
        assert!(
            CommandId::ALL.iter().any(|c| c.category() == cat),
            "category {cat:?} has no commands"
        );
    }
}

#[test]
fn to_message_builds_the_expected_variant() {
    // Spot-check that the executor wires representative commands to the
    // correct Message (Message isn't PartialEq, so we match structurally).
    assert!(matches!(
        CommandId::SaveProject.to_message(),
        Message::ProjectIo(ProjectIoMessage::SaveProject)
    ));
    assert!(matches!(
        CommandId::SaveProjectAs.to_message(),
        Message::ProjectIo(ProjectIoMessage::SaveProjectAs)
    ));
    assert!(matches!(
        CommandId::Undo.to_message(),
        Message::Undo
    ));
    assert!(matches!(
        CommandId::Redo.to_message(),
        Message::Redo
    ));
    assert!(matches!(
        CommandId::OpenSelectedMidiClip.to_message(),
        Message::MidiEditor(MidiEditorMessage::OpenSelectedMidiClip)
    ));
    assert!(matches!(
        CommandId::TogglePerformanceMode.to_message(),
        Message::Ui(UiMessage::RequestPerformanceToggle)
    ));
    assert!(matches!(
        CommandId::ExitPerformanceMode.to_message(),
        Message::Ui(UiMessage::ExitPerformanceMode)
    ));
    assert!(matches!(
        CommandId::TransportPlay.to_message(),
        Message::Transport(TransportMessage::Play)
    ));
}

#[test]
fn every_command_builds_a_message() {
    // Exercising the executor for all commands ensures none panics and the
    // match is exhaustive at runtime as well as compile time.
    for &id in &CommandId::ALL {
        let _ = id.to_message();
    }
}

// ---------------------------------------------------------------------------
// KeyChord parse / format
// ---------------------------------------------------------------------------

#[test]
fn parse_basic_chord_with_modifiers() {
    let chord = KeyChord::parse("Cmd+Shift+S").unwrap();
    assert_eq!(
        chord,
        KeyChord {
            mods: Mods::cmd_shift(),
            key: ChordKey::Char('s'),
        }
    );
}

#[test]
fn parse_is_case_insensitive_and_order_independent() {
    let a = KeyChord::parse("cmd+shift+s").unwrap();
    let b = KeyChord::parse("SHIFT+CMD+S").unwrap();
    assert_eq!(a, b);
    // Character is normalised to lowercase regardless of input case.
    assert_eq!(KeyChord::parse("Cmd+S").unwrap().key, ChordKey::Char('s'));
}

#[test]
fn parse_accepts_glyph_modifiers() {
    assert_eq!(
        KeyChord::parse("⌘+⇧+S").unwrap(),
        KeyChord::char('s', Mods::cmd_shift())
    );
    assert_eq!(
        KeyChord::parse("⌘+S").unwrap(),
        KeyChord::char('s', Mods::cmd())
    );
}

#[test]
fn parse_named_keys() {
    assert_eq!(
        KeyChord::parse("Enter").unwrap(),
        KeyChord::named(NamedKey::Enter, Mods::NONE)
    );
    assert_eq!(
        KeyChord::parse("Cmd+Escape").unwrap(),
        KeyChord::named(NamedKey::Escape, Mods::cmd())
    );
    assert_eq!(
        KeyChord::parse("esc").unwrap().key,
        ChordKey::Named(NamedKey::Escape)
    );
}

#[test]
fn parse_rejects_malformed_specs() {
    assert!(KeyChord::parse("").is_none(), "empty spec");
    assert!(KeyChord::parse("Cmd").is_none(), "modifiers only, no key");
    assert!(KeyChord::parse("S+S").is_none(), "two keys");
    assert!(KeyChord::parse("Cmd+Frobnicate").is_none(), "unknown key");
}

#[test]
fn format_glyphs_uses_macos_keycaps_in_canonical_order() {
    assert_eq!(KeyChord::char('s', Mods::cmd()).format_glyphs(), "⌘S");
    assert_eq!(KeyChord::char('s', Mods::cmd_shift()).format_glyphs(), "⇧⌘S");
    let all = Mods {
        ctrl: true,
        alt: true,
        shift: true,
        cmd: true,
    };
    assert_eq!(KeyChord::char('k', all).format_glyphs(), "⌃⌥⇧⌘K");
    assert_eq!(
        KeyChord::named(NamedKey::Enter, Mods::NONE).format_glyphs(),
        "↵"
    );
}

#[test]
fn format_tokens_round_trips_through_parse() {
    // Every chord the registry knows must survive a format→parse cycle so
    // bindings can be persisted as text.
    for preset in KeymapPreset::ALL {
        for (_id, chord) in preset.bindings().iter() {
            let text = chord.format_tokens();
            let reparsed = KeyChord::parse(&text)
                .unwrap_or_else(|| panic!("could not reparse {text:?}"));
            assert_eq!(reparsed, chord, "round trip failed for {text:?}");
        }
    }
}

// ---------------------------------------------------------------------------
// Binding tables
// ---------------------------------------------------------------------------

#[test]
fn default_table_covers_the_hardcoded_subscription_shortcuts() {
    let map = BindingMap::resonance_default();
    // The shortcuts currently hardcoded in update.rs::subscription.
    assert_eq!(
        map.chord_for(CommandId::SaveProject),
        Some(KeyChord::char('s', Mods::cmd()))
    );
    assert_eq!(
        map.chord_for(CommandId::SaveProjectAs),
        Some(KeyChord::char('s', Mods::cmd_shift()))
    );
    assert_eq!(
        map.chord_for(CommandId::OpenProject),
        Some(KeyChord::char('o', Mods::cmd()))
    );
    assert_eq!(
        map.chord_for(CommandId::Undo),
        Some(KeyChord::char('z', Mods::cmd()))
    );
    assert_eq!(
        map.chord_for(CommandId::Redo),
        Some(KeyChord::char('z', Mods::cmd_shift()))
    );
    assert_eq!(
        map.chord_for(CommandId::OpenSelectedMidiClip),
        Some(KeyChord::named(NamedKey::Enter, Mods::NONE))
    );
    assert_eq!(
        map.chord_for(CommandId::TogglePerformanceMode),
        Some(KeyChord::char('f', Mods::NONE))
    );
    assert_eq!(
        map.chord_for(CommandId::ExitPerformanceMode),
        Some(KeyChord::named(NamedKey::Escape, Mods::NONE))
    );
}

#[test]
fn lookup_by_chord_and_by_id_are_consistent() {
    let map = BindingMap::resonance_default();
    for (id, chord) in map.iter() {
        assert_eq!(map.command_for(chord), Some(id));
        assert_eq!(map.chord_for(id), Some(chord));
    }
    // An unbound chord resolves to nothing.
    assert_eq!(
        map.command_for(KeyChord::char('q', Mods { ctrl: true, alt: true, shift: true, cmd: true })),
        None
    );
}

#[test]
fn binding_table_has_no_duplicate_chords() {
    let map = BindingMap::resonance_default();
    let mut seen = std::collections::HashSet::new();
    for (_id, chord) in map.iter() {
        assert!(
            seen.insert(chord),
            "chord {} bound twice in default table",
            chord.format_glyphs()
        );
    }
}

#[test]
fn set_rebinds_and_steals_chord_from_previous_owner() {
    let mut map = BindingMap::resonance_default();
    let save_chord = KeyChord::char('s', Mods::cmd());
    assert_eq!(map.command_for(save_chord), Some(CommandId::SaveProject));

    // Rebind Cmd+S to Bounce; SaveProject must lose it.
    map.set(CommandId::BounceToWav, save_chord);
    assert_eq!(map.command_for(save_chord), Some(CommandId::BounceToWav));
    assert_ne!(map.chord_for(CommandId::SaveProject), Some(save_chord));
}

#[test]
fn all_presets_resolve_every_command() {
    // Presets are built atop the defaults, so every command resolves under
    // every preset and the bidirectional lookups stay consistent.
    for preset in KeymapPreset::ALL {
        let map = preset.bindings();
        assert!(!map.is_empty());
        for (id, chord) in map.iter() {
            assert_eq!(
                map.command_for(chord),
                Some(id),
                "{:?}: chord {} did not resolve back to {id:?}",
                preset,
                chord.format_glyphs()
            );
        }
        // Core defaults survive into every preset.
        assert!(map.chord_for(CommandId::SaveProject).is_some());
        assert!(map.chord_for(CommandId::Undo).is_some());
    }
}

#[test]
fn preset_overrides_take_effect() {
    // Ableton remaps Record to Enter; Resonance default keeps it on `R`.
    let resonance = KeymapPreset::Resonance.bindings();
    let ableton = KeymapPreset::AbletonLive.bindings();
    assert_eq!(
        resonance.chord_for(CommandId::TransportRecord),
        Some(KeyChord::char('r', Mods::NONE))
    );
    assert_eq!(
        ableton.chord_for(CommandId::TransportRecord),
        Some(KeyChord::named(NamedKey::Enter, Mods::NONE))
    );
}

// ---------------------------------------------------------------------------
// Fuzzy matcher
// ---------------------------------------------------------------------------

#[test]
fn fuzzy_empty_needle_matches_everything() {
    let m = fuzzy_match("", "Open Project").unwrap();
    assert_eq!(m.score, 0);
    assert!(m.ranges.is_empty());
}

#[test]
fn fuzzy_non_subsequence_does_not_match() {
    assert!(fuzzy_match("xyz", "Open Project").is_none());
    // `j` only appears in "Project"; no `p` follows it, so "jp" is not a
    // subsequence even though both letters are present.
    assert!(fuzzy_match("jp", "Open Project").is_none());
}

#[test]
fn fuzzy_is_case_insensitive() {
    assert!(fuzzy_match("OPEN", "open project").is_some());
    assert!(fuzzy_match("op", "Open Project").is_some());
}

#[test]
fn fuzzy_ranges_point_at_matched_chars() {
    // Contiguous prefix match yields a single merged range.
    let m = fuzzy_match("open", "Open Project").unwrap();
    assert_eq!(m.ranges, vec![(0, 4)]);

    // Acronym-style match across word boundaries yields separate ranges.
    let hay = "Save Project As";
    let m = fuzzy_match("spa", hay).unwrap();
    let chars: Vec<char> = hay.chars().collect();
    for (s, e) in &m.ranges {
        for c in &chars[*s..*e] {
            assert!(!c.is_whitespace());
        }
    }
    // S(0), P(5), A(13) → three single-char ranges.
    assert_eq!(m.ranges, vec![(0, 1), (5, 6), (13, 14)]);
}

#[test]
fn fuzzy_prefers_contiguous_and_word_boundary_matches() {
    // Contiguous run scores higher than the same chars scattered.
    let contiguous = fuzzy_match("save", "Save Project").unwrap();
    let scattered = fuzzy_match("save", "Show advanced view effects").unwrap();
    assert!(
        contiguous.score > scattered.score,
        "contiguous {} should beat scattered {}",
        contiguous.score,
        scattered.score
    );

    // A word-boundary match beats a mid-word coincidence for the same needle.
    let boundary = fuzzy_match("p", "Open Project").unwrap();
    let midword = fuzzy_match("p", "Tempo").unwrap();
    assert!(boundary.score >= midword.score);
}
