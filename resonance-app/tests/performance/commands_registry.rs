//! Unit coverage for the command registry (`resonance_app::commands`):
//! KeyChord parse/format round-tripping and glyph rendering, the fuzzy
//! subsequence matcher (score + highlight ranges), and the binding tables
//! (default + DAW presets, lookup-by-chord and lookup-by-id).
//!
//! Kept in a separate test file per the project convention of no inline
//! `#[cfg(test)]` modules.

use resonance_app::commands::{
    fuzzy_match, BindingMap, ChordKey, CommandCategory, CommandId, KeyChord, KeyGate, KeymapPreset,
    Mods, NamedKey, Scope,
};
use resonance_app::message::*;
use resonance_app::Resonance;

// ---------------------------------------------------------------------------
// Registry metadata
// ---------------------------------------------------------------------------

#[test]
fn all_commands_have_metadata_and_unique_breadcrumbs() {
    let mut seen = std::collections::HashSet::new();
    for &id in CommandId::ALL {
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
    // `ALL` is generated from the enum declaration, so it can't miss a
    // variant; it must not repeat one either.
    let unique: std::collections::HashSet<_> = CommandId::ALL.iter().collect();
    assert_eq!(unique.len(), CommandId::ALL.len());
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
    let (app, _task) = Resonance::new_for_test();
    let msg = |id: CommandId| id.to_message(&app).expect("parameterless command");
    assert!(matches!(
        msg(CommandId::SaveProject),
        Message::ProjectIo(ProjectIoMessage::SaveProject)
    ));
    assert!(matches!(
        msg(CommandId::SaveProjectAs),
        Message::ProjectIo(ProjectIoMessage::SaveProjectAs)
    ));
    assert!(matches!(
        msg(CommandId::Undo),
        Message::Undo
    ));
    assert!(matches!(
        msg(CommandId::Redo),
        Message::Redo
    ));
    assert!(matches!(
        msg(CommandId::OpenSelectedMidiClip),
        Message::MidiEditor(MidiEditorMessage::OpenSelectedMidiClip)
    ));
    assert!(matches!(
        msg(CommandId::TogglePerformanceMode),
        Message::Ui(UiMessage::TogglePerformanceMode)
    ));
    assert!(matches!(
        msg(CommandId::ExitPerformanceMode),
        Message::Ui(UiMessage::ExitPerformanceMode)
    ));
    assert!(matches!(
        msg(CommandId::TransportPlay),
        Message::Transport(TransportMessage::Play)
    ));
}

#[test]
fn every_command_builds_a_message() {
    // Exercising the executor for all commands ensures none panics and the
    // match is exhaustive at runtime as well as compile time.
    let (app, _task) = Resonance::new_for_test();
    for &id in CommandId::ALL {
        let _ = id.to_message(&app);
        let _ = id.availability(&app);
    }
}

// ---------------------------------------------------------------------------
// UX-21: selection-scoped mixer actions registered in the palette
// ---------------------------------------------------------------------------

/// code review UX-21: these actions already existed as context-menu /
/// track-header entries but had no palette command, so they were
/// unreachable by keyboard and invisible to search. Each must be
/// unavailable with no selection and available (building the same message
/// its existing GUI entry point sends) once the right thing is selected.
#[test]
fn ux21_selection_scoped_commands_gate_on_selection_and_resolve_their_target() {
    use resonance_audio::types::TrackType;

    let (mut app, _task) = Resonance::new_for_test();
    app.test_add_track(1, TrackType::Audio);
    app.test_add_bus(1, "Reverb");

    // No selection at all: every one of these is unavailable.
    for id in [
        CommandId::BounceInPlaceSelected,
        CommandId::RenameSelected,
        CommandId::DeleteSelectedBus,
        CommandId::ToggleFxBypassSelected,
        CommandId::ToggleMonoSelected,
        CommandId::SaveSelectedTrackAsPreset,
    ] {
        assert!(
            !id.availability(&app).is_yes(),
            "{id:?} should require a selection"
        );
        assert!(id.to_message(&app).is_none(), "{id:?} has no target yet");
    }

    // A track is selected: the track-scoped ones resolve against it.
    app.test_select_track(1);
    assert!(CommandId::BounceInPlaceSelected.availability(&app).is_yes());
    assert!(matches!(
        CommandId::BounceInPlaceSelected.to_message(&app),
        Some(Message::Track(TrackMessage::BounceInPlace(1)))
    ));
    assert!(CommandId::ToggleMonoSelected.availability(&app).is_yes());
    assert!(matches!(
        CommandId::ToggleMonoSelected.to_message(&app),
        Some(Message::Track(TrackMessage::ToggleTrackMono(1)))
    ));
    assert!(CommandId::SaveSelectedTrackAsPreset.availability(&app).is_yes());
    assert!(matches!(
        CommandId::SaveSelectedTrackAsPreset.to_message(&app),
        Some(Message::Track(TrackMessage::OpenSavePresetPrompt(1)))
    ));
    assert!(CommandId::RenameSelected.availability(&app).is_yes());
    assert!(matches!(
        CommandId::RenameSelected.to_message(&app),
        Some(Message::Ui(UiMessage::BeginRename(
            resonance_app::state::RenameTarget::Track(1),
            resonance_app::state::RenameSurface::Strip
        )))
    ));
    assert!(CommandId::ToggleFxBypassSelected.availability(&app).is_yes());
    assert!(matches!(
        CommandId::ToggleFxBypassSelected.to_message(&app),
        Some(Message::Track(TrackMessage::ToggleTrackFxBypass(1)))
    ));
    // The bus-only command stays unavailable while a track is selected.
    assert!(!CommandId::DeleteSelectedBus.availability(&app).is_yes());

    // A bus is selected instead: the shared commands flip to the bus
    // target, and the track-only ones (bounce, mono, save-preset) go back
    // to unavailable.
    let _ = app.update(Message::Ui(UiMessage::SelectBus(Some(1))));
    assert!(CommandId::DeleteSelectedBus.availability(&app).is_yes());
    assert!(matches!(
        CommandId::DeleteSelectedBus.to_message(&app),
        Some(Message::Bus(BusMessage::RemoveBus(1)))
    ));
    assert!(CommandId::RenameSelected.availability(&app).is_yes());
    assert!(matches!(
        CommandId::RenameSelected.to_message(&app),
        Some(Message::Ui(UiMessage::BeginRename(
            resonance_app::state::RenameTarget::Bus(1),
            resonance_app::state::RenameSurface::Strip
        )))
    ));
    assert!(CommandId::ToggleFxBypassSelected.availability(&app).is_yes());
    assert!(matches!(
        CommandId::ToggleFxBypassSelected.to_message(&app),
        Some(Message::Bus(BusMessage::ToggleBusFxBypass(1)))
    ));
    assert!(!CommandId::BounceInPlaceSelected.availability(&app).is_yes());
    assert!(!CommandId::ToggleMonoSelected.availability(&app).is_yes());
    assert!(!CommandId::SaveSelectedTrackAsPreset.availability(&app).is_yes());
}

/// `AddExternalInstrumentTrack` needs no selection — same shape as the
/// other `Add*Track` commands.
#[test]
fn ux21_add_external_instrument_track_always_available() {
    let (app, _task) = Resonance::new_for_test();
    assert!(CommandId::AddExternalInstrumentTrack.availability(&app).is_yes());
    assert!(matches!(
        CommandId::AddExternalInstrumentTrack.to_message(&app),
        Some(Message::Track(TrackMessage::AddExternalInstrumentTrack))
    ));
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
fn default_table_covers_the_live_shortcuts() {
    let map = BindingMap::resonance_default();
    // The shortcuts the pre-registry `key_press_message` hardcoded (the
    // full parity check is `shortcut_parity.rs`).
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
        assert_eq!(map.command_for(id.scope(), chord), Some(id));
        assert!(map.matches(id, chord));
        assert!(map.chords_for(id).any(|c| c == chord));
    }
    // An unbound chord resolves to nothing.
    assert_eq!(
        map.command_for(
            Scope::Global,
            KeyChord::char('q', Mods { ctrl: true, alt: true, shift: true, cmd: true })
        ),
        None
    );
}

#[test]
fn binding_table_has_no_duplicate_chords_within_a_scope() {
    let map = BindingMap::resonance_default();
    let mut seen = std::collections::HashSet::new();
    for (id, chord) in map.iter() {
        assert!(
            seen.insert((id.scope(), chord)),
            "chord {} bound twice in scope {:?}",
            chord.format_glyphs(),
            id.scope()
        );
    }
}

#[test]
fn redo_keeps_cmd_y_as_an_alternate_behind_its_primary() {
    let map = BindingMap::resonance_default();
    let chords: Vec<KeyChord> = map.chords_for(CommandId::Redo).collect();
    assert_eq!(
        chords,
        vec![KeyChord::char('z', Mods::cmd_shift()), KeyChord::char('y', Mods::cmd())]
    );
    assert_eq!(map.chord_for(CommandId::Redo), Some(KeyChord::char('z', Mods::cmd_shift())));
}

/// Gate invariant (§3.2): a default binding without ⌘/Ctrl is typing-gated,
/// so a new single-key binding cannot land ungated. (The dispatcher also
/// gates every bare chord regardless, which covers presets and rebinding.)
#[test]
fn every_bare_key_binding_is_typing_gated() {
    for (id, chord) in BindingMap::resonance_default().iter() {
        if !chord.mods.cmd && !chord.mods.ctrl {
            assert_eq!(
                id.gate(),
                KeyGate::NotWhileTyping,
                "{id:?} on bare {} must be NotWhileTyping",
                chord.format_glyphs()
            );
        }
    }
    assert_eq!(CommandId::Undo.gate(), KeyGate::NotWhileTyping);
    assert_eq!(CommandId::Redo.gate(), KeyGate::NotWhileTyping);
}

/// Toggles never re-fire on key repeat (holding F used to flap
/// Performance mode).
#[test]
fn only_seeks_and_zooms_repeat() {
    // Derived from what each command does, not from its name: a command
    // that repeats must dispatch a seek or a zoom, never a toggle.
    let (app, _task) = Resonance::new_for_test();
    let mut repeating = 0;
    for &id in CommandId::ALL {
        if !id.repeat() {
            continue;
        }
        repeating += 1;
        let message = format!("{:?}", id.to_message(&app).expect("a message"));
        assert!(
            message.contains("SeekTo(Nudge") || message.contains("Zoom"),
            "{id:?} repeats but dispatches {message}"
        );
    }
    assert_eq!(repeating, 8, "four nudges and four zooms (view + track editor)");
}

#[test]
fn set_rebinds_and_steals_chord_from_previous_owner() {
    let mut map = BindingMap::resonance_default();
    let save_chord = KeyChord::char('s', Mods::cmd());
    assert_eq!(map.command_for(Scope::Global, save_chord), Some(CommandId::SaveProject));

    // Rebind Cmd+S to Bounce; SaveProject must lose it.
    map.set(CommandId::BounceToWav, save_chord);
    assert_eq!(map.command_for(Scope::Global, save_chord), Some(CommandId::BounceToWav));
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
                map.command_for(id.scope(), chord),
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
    // Logic's Return goes to the beginning, taking Enter from Open Selected
    // MIDI Clip; `unbound()` reports exactly what a preset takes away.
    let logic = KeymapPreset::LogicPro.bindings();
    assert_eq!(
        logic.chord_for(CommandId::PlayheadToStart),
        Some(KeyChord::named(NamedKey::Enter, Mods::NONE))
    );
    assert_eq!(logic.chord_for(CommandId::TransportToggleLoop), Some(KeyChord::char('c', Mods::NONE)));
    assert_eq!(logic.chord_for(CommandId::OpenSelectedMidiClip), None);
    let unbound = KeymapPreset::LogicPro.unbound();
    assert!(unbound.contains(&CommandId::OpenSelectedMidiClip));
    assert!(unbound.contains(&CommandId::AddAudioTrack), "⌘T went to split");
    assert!(KeymapPreset::Resonance.unbound().is_empty());
}

/// No preset may be a no-op: each one changes the table.
#[test]
fn every_preset_differs_from_the_defaults() {
    let default: Vec<_> = BindingMap::resonance_default().iter().collect();
    for preset in KeymapPreset::ALL {
        if preset == KeymapPreset::Resonance {
            continue;
        }
        let map: Vec<_> = preset.bindings().iter().collect();
        assert_ne!(map, default, "{preset:?} changes nothing");
    }
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
