//! The binding table and the DAW keymap presets.
//!
//! Presets are chosen in Preferences › Keyboard, which also lists what a
//! preset leaves unbound (command-palette.md D5, §8).

use super::{CommandId, KeyChord, Mods, NamedKey, Scope};

/// A selectable keyboard layout. [`KeymapPreset::Resonance`] is the built-in
/// default; the others carry the well-known bindings of their DAW (only
/// ones we are sure of), applied on top of the defaults. A preset can take
/// a chord from a default command, which is then unbound: see
/// [`KeymapPreset::unbound`], shown by the Keyboard panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeymapPreset {
    Resonance,
    LogicPro,
    ProTools,
}

impl std::fmt::Display for KeymapPreset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.display_name())
    }
}

impl KeymapPreset {
    pub const ALL: [KeymapPreset; 3] = [
        KeymapPreset::Resonance,
        KeymapPreset::LogicPro,
        KeymapPreset::ProTools,
    ];

    /// Stable id for settings.json.
    pub fn key(self) -> &'static str {
        match self {
            KeymapPreset::Resonance => "Resonance",
            KeymapPreset::LogicPro => "LogicPro",
            KeymapPreset::ProTools => "ProTools",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            KeymapPreset::Resonance => "Resonance (default)",
            KeymapPreset::LogicPro => "Logic Pro",
            KeymapPreset::ProTools => "Pro Tools",
        }
    }

    /// Resolve this preset into a concrete [`BindingMap`]: its bindings
    /// applied on top of the Resonance defaults.
    pub fn bindings(self) -> BindingMap {
        let mut map = BindingMap::resonance_default();
        for (id, chord, alternate) in self.overrides() {
            if alternate {
                map.add_alternate(id, chord);
            } else {
                map.set(id, chord);
            }
        }
        map
    }

    /// The commands the defaults bind that this preset leaves with no
    /// chord at all, because it gave their chord to something else.
    pub fn unbound(self) -> Vec<CommandId> {
        let default = BindingMap::resonance_default();
        let preset = self.bindings();
        CommandId::ALL
            .iter()
            .copied()
            .filter(|&id| default.chord_for(id).is_some() && preset.chord_for(id).is_none())
            .collect()
    }

    /// `(command, chord, as an alternate)` — an alternate keeps the
    /// command's default chord too.
    fn overrides(self) -> Vec<(CommandId, KeyChord, bool)> {
        use CommandId::*;
        let cmd = Mods::cmd();
        let none = Mods::NONE;
        match self {
            KeymapPreset::Resonance => Vec::new(),
            // Logic Pro: C toggles Cycle, ⌘U sets the locators to the
            // selection, ⌘T splits at the playhead, Return goes to the
            // beginning. (R, K, Space, M, S already match.)
            KeymapPreset::LogicPro => vec![
                (TransportToggleLoop, KeyChord::char('c', none), false),
                (LoopSelection, KeyChord::char('u', cmd), false),
                (SplitClipAtPlayhead, KeyChord::char('t', cmd), false),
                (PlayheadToStart, KeyChord::named(NamedKey::Enter, none), false),
            ],
            // Pro Tools' numeric-keypad transport: 0 play/stop, 3 record,
            // 4 loop playback, 7 click. (⌘E separate already matches.)
            KeymapPreset::ProTools => vec![
                (TransportTogglePlay, KeyChord::char('0', none), true),
                (TransportRecord, KeyChord::char('3', none), false),
                (TransportToggleLoop, KeyChord::char('4', none), false),
                (TransportToggleMetronome, KeyChord::char('7', none), false),
            ],
        }
    }
}

/// A bidirectional binding table mapping commands to chords.
///
/// Stored as an ordered list of pairs so lookups in both directions are
/// simple linear scans (the table is tens of entries, never hot). A command
/// may own several chords: the first is its **primary** (shown in the
/// palette and tooltips), the rest are alternates. A chord never resolves to
/// two commands within one [`Scope`].
#[derive(Debug, Clone, Default)]
pub struct BindingMap {
    entries: Vec<(CommandId, KeyChord)>,
}

impl BindingMap {
    /// The canonical Resonance default bindings (command-palette.md §5).
    pub fn resonance_default() -> BindingMap {
        use CommandId::*;
        let cmd = Mods::cmd();
        let cmd_shift = Mods::cmd_shift();
        let none = Mods::NONE;
        let key = |c: char| KeyChord::char(c, none);
        let shift = Mods {
            shift: true,
            ..Mods::NONE
        };
        let alt = Mods {
            alt: true,
            ..Mods::NONE
        };
        let named = KeyChord::named;
        let cmd_alt = Mods {
            cmd: true,
            alt: true,
            ..Mods::NONE
        };

        let entries = vec![
            // Project (§5.6).
            (CommandPalette, KeyChord::char('k', cmd)),
            (CommandPalette, KeyChord::char('p', cmd_shift)),
            (NewProject, KeyChord::char('n', cmd)),
            (OpenProject, KeyChord::char('o', cmd)),
            (SaveProject, KeyChord::char('s', cmd)),
            (SaveProjectAs, KeyChord::char('s', cmd_shift)),
            (BounceToWav, KeyChord::char('b', cmd)),
            (ExportStemsMidi, KeyChord::char('e', cmd_shift)),
            (ImportMidi, KeyChord::char('i', cmd)),
            (ImportAudio, KeyChord::char('i', cmd_shift)),
            (OpenSettings, KeyChord::char(',', cmd)),
            // Editing. Redo keeps ⌘Y as an alternate.
            (Undo, KeyChord::char('z', cmd)),
            (Redo, KeyChord::char('z', cmd_shift)),
            (Redo, KeyChord::char('y', cmd)),
            (OpenSelectedMidiClip, KeyChord::named(NamedKey::Enter, none)),
            (CloseMidiEditor, KeyChord::named(NamedKey::Escape, cmd)),
            (SplitClipAtPlayhead, KeyChord::char('e', cmd)),
            (DuplicateSelection, KeyChord::char('d', cmd)),
            (QuantizeSelectedNotes, key('q')),
            (LoopSelection, KeyChord::char('l', cmd)),
            // Transport (§5.1).
            (TransportTogglePlay, named(NamedKey::Space, none)),
            (TransportPlayPause, named(NamedKey::Space, shift)),
            (TransportPlayFromLoopStart, named(NamedKey::Space, alt)),
            (TransportRecord, key('r')),
            (TransportToggleLoop, key('l')),
            (TransportToggleMetronome, key('k')),
            // Playhead and loop control (§5.2).
            (PlayheadToStart, named(NamedKey::Home, none)),
            (PlayheadToStart, named(NamedKey::ArrowLeft, cmd)),
            (PlayheadToEnd, named(NamedKey::End, none)),
            (PlayheadToEnd, named(NamedKey::ArrowRight, cmd)),
            (PlayheadToLoopStart, key('[')),
            (PlayheadToLoopEnd, key(']')),
            (SetLoopStartAtPlayhead, key('i')),
            (SetLoopEndAtPlayhead, key('o')),
            (LoopSectionAtPlayhead, KeyChord::char('l', shift)),
            (NudgeBackBar, named(NamedKey::ArrowLeft, none)),
            (NudgeForwardBar, named(NamedKey::ArrowRight, none)),
            (NudgeBackBeat, named(NamedKey::ArrowLeft, alt)),
            (NudgeForwardBeat, named(NamedKey::ArrowRight, alt)),
            (NextMarker, key('.')),
            (PrevMarker, key(',')),
            (NextSectionStart, KeyChord::char('.', shift)),
            (PrevSectionStart, KeyChord::char(',', shift)),
            (AddMarkerAtPlayhead, KeyChord::char('m', shift)),
            (GoToBar, KeyChord::char('j', cmd)),
            // View & Navigation.
            (ViewArrange, KeyChord::char('1', cmd)),
            (ViewMixer, KeyChord::char('2', cmd)),
            (ViewCompose, KeyChord::char('3', cmd)),
            (TogglePerformanceMode, key('f')),
            (ExitPerformanceMode, KeyChord::named(NamedKey::Escape, none)),
            (ZoomIn, KeyChord::char('=', cmd)),
            (ZoomOut, KeyChord::char('-', cmd)),
            (ToggleBrowser, KeyChord::char('b', cmd_alt)),
            (ToggleGlobalTracks, KeyChord::char('g', cmd_alt)),
            // Mixer.
            (GroupSelectedTracks, KeyChord::char('g', cmd)),
            (FreezeSelectedTracks, KeyChord::char('f', cmd)),
            (FreezeAllTracks, KeyChord::char('f', cmd_shift)),
            (AddAudioTrack, KeyChord::char('t', cmd)),
            (AddInstrumentTrack, KeyChord::char('t', cmd_shift)),
            (OpenAddTrackMenu, KeyChord::char('t', cmd_alt)),
            (ToggleMuteSelected, key('m')),
            (ToggleSoloSelected, key('s')),
            (ToggleArmSelected, KeyChord::char('r', shift)),
            (DeleteSelectedTrack, named(NamedKey::Backspace, cmd)),
            // Canvas-local keys (§4.3): live only while that canvas owns
            // the keyboard, where they may shadow a global chord.
            // Delete / Backspace with any of ⇧ ⌘ ⌥ too, as the canvases
            // always accepted: ⌘⌫ over a canvas selection deletes the
            // selection and never reaches Delete Selected Track.
            (TimelineDeleteSelection, named(NamedKey::Delete, none)),
            (TimelineDeleteSelection, named(NamedKey::Backspace, none)),
            (TimelineDeleteSelection, named(NamedKey::Delete, shift)),
            (TimelineDeleteSelection, named(NamedKey::Backspace, shift)),
            (TimelineDeleteSelection, named(NamedKey::Delete, cmd)),
            (TimelineDeleteSelection, named(NamedKey::Backspace, cmd)),
            (DeleteSelectedNotes, named(NamedKey::Delete, none)),
            (DeleteSelectedNotes, named(NamedKey::Backspace, none)),
            (DeleteSelectedNotes, named(NamedKey::Delete, shift)),
            (DeleteSelectedNotes, named(NamedKey::Backspace, shift)),
            (DeleteSelectedNotes, named(NamedKey::Delete, cmd)),
            (DeleteSelectedNotes, named(NamedKey::Backspace, cmd)),
            (SelectAllNotes, KeyChord::char('a', cmd)),
            (SelectNotesInView, KeyChord::char('a', cmd_shift)),
            (VocalDeleteNote, named(NamedKey::Delete, none)),
            (VocalDeleteNote, named(NamedKey::Backspace, none)),
            (VocalDeleteNote, named(NamedKey::Delete, shift)),
            (VocalDeleteNote, named(NamedKey::Backspace, shift)),
            (VocalDeleteNote, named(NamedKey::Delete, cmd)),
            (VocalDeleteNote, named(NamedKey::Backspace, cmd)),
            (VocalToggleSlur, key('s')),
            (VocalToggleSlur, KeyChord::char('s', shift)),
            (VocalToggleSlur, KeyChord::char('=', shift)),
            (VocalToggleSlur, named(NamedKey::Plus, none)),
            (ExpandedZoomIn, key('=')),
            (ExpandedZoomIn, KeyChord::char('=', shift)),
            (ExpandedZoomIn, named(NamedKey::Plus, none)),
            (ExpandedZoomOut, key('-')),
            (ComposeCollapseTrack, KeyChord::named(NamedKey::Escape, none)),
        ];

        BindingMap { entries }
    }

    /// A shared copy of [`resonance_default`](Self::resonance_default), for
    /// callers that need a `&'static` map (canvas tests).
    pub fn default_ref() -> &'static BindingMap {
        static DEFAULT: std::sync::OnceLock<BindingMap> = std::sync::OnceLock::new();
        DEFAULT.get_or_init(BindingMap::resonance_default)
    }

    /// Bind `chord` as `id`'s only chord, dropping its previous chords and
    /// taking `chord` away from any other command in the same scope.
    pub fn set(&mut self, id: CommandId, chord: KeyChord) {
        self.entries.retain(|(other_id, other_chord)| {
            *other_id != id && !(*other_chord == chord && other_id.scope() == id.scope())
        });
        self.entries.push((id, chord));
    }

    /// Make `chord` `id`'s primary chord, keeping its alternates, and take
    /// it from any other command in the same scope.
    pub fn set_primary(&mut self, id: CommandId, chord: KeyChord) {
        self.entries.retain(|(other_id, other_chord)| {
            !(*other_chord == chord && other_id.scope() == id.scope())
        });
        match self.entries.iter_mut().find(|(other, _)| *other == id) {
            Some(entry) => entry.1 = chord,
            None => self.entries.push((id, chord)),
        }
    }

    /// Add `chord` as an alternate for `id`, taking it away from any other
    /// command in the same scope.
    pub fn add_alternate(&mut self, id: CommandId, chord: KeyChord) {
        self.entries.retain(|(other_id, other_chord)| {
            !(*other_chord == chord && other_id.scope() == id.scope())
        });
        self.entries.push((id, chord));
    }

    /// Remove every binding for `id`.
    pub fn clear(&mut self, id: CommandId) {
        self.entries.retain(|(other_id, _)| *other_id != id);
    }

    /// The command bound to `chord` in `scope`, if any.
    pub fn command_for(&self, scope: Scope, chord: KeyChord) -> Option<CommandId> {
        self.entries
            .iter()
            .find(|(id, c)| *c == chord && id.scope() == scope)
            .map(|(id, _)| *id)
    }

    /// Whether `chord` triggers `id` (primary or alternate).
    pub fn matches(&self, id: CommandId, chord: KeyChord) -> bool {
        self.entries.iter().any(|(other, c)| *other == id && *c == chord)
    }

    /// The primary chord bound to `id`, if any.
    pub fn chord_for(&self, id: CommandId) -> Option<KeyChord> {
        self.chords_for(id).next()
    }

    /// Every chord bound to `id`, primary first.
    pub fn chords_for(&self, id: CommandId) -> impl Iterator<Item = KeyChord> + '_ {
        self.entries
            .iter()
            .filter(move |(other, _)| *other == id)
            .map(|(_, c)| *c)
    }

    /// All `(command, chord)` pairs in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = (CommandId, KeyChord)> + '_ {
        self.entries.iter().copied()
    }

    /// Number of bindings (alternates included).
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}
