//! The binding table and the DAW keymap presets.
//!
//! Presets stay in the code, unexposed, until the rebinding UI can show and
//! fix what they change (command-palette.md D5).

use super::{CommandId, KeyChord, Mods, NamedKey, Scope};

/// A selectable keyboard layout. [`KeymapPreset::Resonance`] is the built-in
/// default; the others approximate the muscle memory of popular DAWs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeymapPreset {
    Resonance,
    AbletonLive,
    LogicPro,
    ProTools,
    FlStudio,
}

impl KeymapPreset {
    pub const ALL: [KeymapPreset; 5] = [
        KeymapPreset::Resonance,
        KeymapPreset::AbletonLive,
        KeymapPreset::LogicPro,
        KeymapPreset::ProTools,
        KeymapPreset::FlStudio,
    ];

    pub fn display_name(self) -> &'static str {
        match self {
            KeymapPreset::Resonance => "Resonance (default)",
            KeymapPreset::AbletonLive => "Ableton Live",
            KeymapPreset::LogicPro => "Logic Pro",
            KeymapPreset::ProTools => "Pro Tools",
            KeymapPreset::FlStudio => "FL Studio",
        }
    }

    /// Resolve this preset into a concrete [`BindingMap`]. Presets are built by
    /// applying their DAW-specific overrides on top of the Resonance defaults,
    /// so every [`CommandId`] resolves under every preset.
    pub fn bindings(self) -> BindingMap {
        let mut map = BindingMap::resonance_default();
        for (id, chord) in self.overrides() {
            map.set(id, chord);
        }
        map
    }

    /// DAW-specific deviations from the Resonance defaults.
    fn overrides(self) -> Vec<(CommandId, KeyChord)> {
        use CommandId::*;
        let cmd = Mods::cmd();
        let none = Mods::NONE;
        match self {
            KeymapPreset::Resonance => Vec::new(),
            KeymapPreset::AbletonLive => vec![
                (TransportToggleLoop, KeyChord::char('l', cmd)),
                (TransportRecord, KeyChord::named(NamedKey::Enter, none)),
                (ViewArrange, KeyChord::named(NamedKey::Tab, none)),
                (TransportToggleMetronome, KeyChord::char('m', cmd)),
            ],
            KeymapPreset::LogicPro => vec![
                (TransportRecord, KeyChord::char('r', none)),
                (TransportToggleMetronome, KeyChord::char('k', none)),
                (TransportCycleTimeSignature, KeyChord::char('t', none)),
            ],
            KeymapPreset::ProTools => vec![
                (TransportRecord, KeyChord::char('3', none)),
                (TransportPlay, KeyChord::named(NamedKey::Space, none)),
                (TransportToggleMetronome, KeyChord::char('7', none)),
                (TransportToggleLoop, KeyChord::char('4', none)),
            ],
            KeymapPreset::FlStudio => vec![
                (TransportRecord, KeyChord::char('r', none)),
                (TransportToggleLoop, KeyChord::char('l', none)),
                (SaveProjectAs, KeyChord::char('s', Mods::cmd_shift())),
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

        let entries = vec![
            // Project.
            (OpenProject, KeyChord::char('o', cmd)),
            (SaveProject, KeyChord::char('s', cmd)),
            (SaveProjectAs, KeyChord::char('s', cmd_shift)),
            // Editing. Redo keeps ⌘Y as an alternate.
            (Undo, KeyChord::char('z', cmd)),
            (Redo, KeyChord::char('z', cmd_shift)),
            (Redo, KeyChord::char('y', cmd)),
            (OpenSelectedMidiClip, KeyChord::named(NamedKey::Enter, none)),
            // Transport: marker navigation.
            (NextMarker, key('.')),
            (PrevMarker, key(',')),
            // View & Navigation.
            (TogglePerformanceMode, key('f')),
            (ExitPerformanceMode, KeyChord::named(NamedKey::Escape, none)),
            // Mixer.
            (GroupSelectedTracks, KeyChord::char('g', cmd)),
            (FreezeSelectedTracks, KeyChord::char('f', cmd)),
            (FreezeAllTracks, KeyChord::char('f', cmd_shift)),
        ];

        BindingMap { entries }
    }

    /// Bind `chord` as `id`'s only chord, dropping its previous chords and
    /// taking `chord` away from any other command in the same scope.
    pub fn set(&mut self, id: CommandId, chord: KeyChord) {
        self.entries.retain(|(other_id, other_chord)| {
            *other_id != id && !(*other_chord == chord && other_id.scope() == id.scope())
        });
        self.entries.push((id, chord));
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
