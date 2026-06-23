//! Command registry — the single source of truth for every user-invokable
//! action in Resonance.
//!
//! This module is intentionally view-agnostic and (almost) iced-agnostic so it
//! can back both the keyboard-shortcut subscription and a future command
//! palette without dragging in widget code. It provides four things:
//!
//! 1. [`CommandId`] — one stable variant per action, each carrying metadata
//!    ([`CommandId::category`], [`CommandId::display_name`],
//!    [`CommandId::breadcrumb`], [`CommandId::glyph`]) and an executor
//!    ([`CommandId::to_message`]) that builds the [`Message`] to dispatch.
//! 2. [`KeyChord`] — a portable modifier+key model with parse/format helpers
//!    that render macOS-style keycap glyphs (⌘ ⌥ ⇧ ↵).
//! 3. [`BindingMap`] / [`KeymapPreset`] — the default binding table plus the
//!    Ableton Live / Logic Pro / Pro Tools / FL Studio preset maps, with
//!    lookup-by-chord and lookup-by-id.
//! 4. [`fuzzy_match`] — a dependency-free subsequence matcher returning a score
//!    and the matched character ranges (for palette highlighting).

use crate::message::*;
use crate::state::ViewMode;

// ===========================================================================
// Categories
// ===========================================================================

/// Top-level grouping for commands, used to section the command palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommandCategory {
    Transport,
    Editing,
    ViewNav,
    ComposeVocal,
    Mixer,
    Project,
}

impl CommandCategory {
    /// Human-readable section label.
    pub fn display_name(self) -> &'static str {
        match self {
            CommandCategory::Transport => "Transport",
            CommandCategory::Editing => "Editing",
            CommandCategory::ViewNav => "View & Navigation",
            CommandCategory::ComposeVocal => "Compose & Vocal",
            CommandCategory::Mixer => "Mixer",
            CommandCategory::Project => "Project",
        }
    }

    /// All categories in display order.
    pub const ALL: [CommandCategory; 6] = [
        CommandCategory::Transport,
        CommandCategory::Editing,
        CommandCategory::ViewNav,
        CommandCategory::ComposeVocal,
        CommandCategory::Mixer,
        CommandCategory::Project,
    ];
}

// ===========================================================================
// Commands
// ===========================================================================

/// A single user-invokable action. Variants are stable identifiers — bindings
/// and the palette key off them, never off display strings.
///
/// Every command maps to exactly one parameterless [`Message`] via
/// [`CommandId::to_message`]; actions that need a runtime argument (a specific
/// `TrackId`, a drag delta, …) are deliberately *not* commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommandId {
    // --- Transport ---
    TransportPlay,
    TransportStop,
    TransportPause,
    TransportRecord,
    TransportSkipBack,
    TransportSkipForward,
    TransportToggleLoop,
    TransportToggleMetronome,
    TransportCycleTimeSignature,

    // --- Editing ---
    Undo,
    Redo,
    OpenSelectedMidiClip,
    CloseMidiEditor,

    // --- View & Navigation ---
    ViewArrange,
    ViewMixer,
    ViewCompose,
    TogglePerformanceMode,
    ExitPerformanceMode,
    ZoomIn,
    ZoomOut,
    ToggleGlobalTracks,

    // --- Compose & Vocal ---
    ComposeCreateSection,
    ComposeCollapseTrack,
    ComposeClearChordSelection,

    // --- Mixer ---
    AddAudioTrack,
    AddInstrumentTrack,
    AddVocalTrack,
    AddBus,
    OpenAddTrackMenu,
    ToggleMasterFxBypass,

    // --- Project ---
    NewProject,
    OpenProject,
    SaveProject,
    SaveProjectAs,
    BounceToWav,
    ExportChordSheet,
    OpenSettings,
}

impl CommandId {
    /// Every command, in registry/palette order.
    pub const ALL: [CommandId; 37] = [
        // Transport
        CommandId::TransportPlay,
        CommandId::TransportStop,
        CommandId::TransportPause,
        CommandId::TransportRecord,
        CommandId::TransportSkipBack,
        CommandId::TransportSkipForward,
        CommandId::TransportToggleLoop,
        CommandId::TransportToggleMetronome,
        CommandId::TransportCycleTimeSignature,
        // Editing
        CommandId::Undo,
        CommandId::Redo,
        CommandId::OpenSelectedMidiClip,
        CommandId::CloseMidiEditor,
        // View & Navigation
        CommandId::ViewArrange,
        CommandId::ViewMixer,
        CommandId::ViewCompose,
        CommandId::TogglePerformanceMode,
        CommandId::ExitPerformanceMode,
        CommandId::ZoomIn,
        CommandId::ZoomOut,
        CommandId::ToggleGlobalTracks,
        // Compose & Vocal
        CommandId::ComposeCreateSection,
        CommandId::ComposeCollapseTrack,
        CommandId::ComposeClearChordSelection,
        // Mixer
        CommandId::AddAudioTrack,
        CommandId::AddInstrumentTrack,
        CommandId::AddVocalTrack,
        CommandId::AddBus,
        CommandId::OpenAddTrackMenu,
        CommandId::ToggleMasterFxBypass,
        // Project
        CommandId::NewProject,
        CommandId::OpenProject,
        CommandId::SaveProject,
        CommandId::SaveProjectAs,
        CommandId::BounceToWav,
        CommandId::ExportChordSheet,
        CommandId::OpenSettings,
    ];

    /// The category this command files under.
    pub fn category(self) -> CommandCategory {
        use CommandId::*;
        match self {
            TransportPlay | TransportStop | TransportPause | TransportRecord
            | TransportSkipBack | TransportSkipForward | TransportToggleLoop
            | TransportToggleMetronome | TransportCycleTimeSignature => CommandCategory::Transport,

            Undo | Redo | OpenSelectedMidiClip | CloseMidiEditor => CommandCategory::Editing,

            ViewArrange | ViewMixer | ViewCompose | TogglePerformanceMode | ExitPerformanceMode
            | ZoomIn | ZoomOut | ToggleGlobalTracks => CommandCategory::ViewNav,

            ComposeCreateSection | ComposeCollapseTrack | ComposeClearChordSelection => {
                CommandCategory::ComposeVocal
            }

            AddAudioTrack | AddInstrumentTrack | AddVocalTrack | AddBus | OpenAddTrackMenu
            | ToggleMasterFxBypass => CommandCategory::Mixer,

            NewProject | OpenProject | SaveProject | SaveProjectAs | BounceToWav
            | ExportChordSheet | OpenSettings => CommandCategory::Project,
        }
    }

    /// Short label shown in menus and the palette.
    pub fn display_name(self) -> &'static str {
        use CommandId::*;
        match self {
            TransportPlay => "Play",
            TransportStop => "Stop",
            TransportPause => "Pause",
            TransportRecord => "Record",
            TransportSkipBack => "Skip to Start",
            TransportSkipForward => "Skip to End",
            TransportToggleLoop => "Toggle Loop",
            TransportToggleMetronome => "Toggle Metronome",
            TransportCycleTimeSignature => "Cycle Time Signature",

            Undo => "Undo",
            Redo => "Redo",
            OpenSelectedMidiClip => "Open Selected MIDI Clip",
            CloseMidiEditor => "Close MIDI Editor",

            ViewArrange => "Arrange View",
            ViewMixer => "Mixer View",
            ViewCompose => "Compose View",
            TogglePerformanceMode => "Toggle Performance Mode",
            ExitPerformanceMode => "Exit Performance Mode",
            ZoomIn => "Zoom In",
            ZoomOut => "Zoom Out",
            ToggleGlobalTracks => "Toggle Global Tracks",

            ComposeCreateSection => "New Section…",
            ComposeCollapseTrack => "Collapse Track Editor",
            ComposeClearChordSelection => "Clear Chord Selection",

            AddAudioTrack => "Add Audio Track",
            AddInstrumentTrack => "Add Instrument Track",
            AddVocalTrack => "Add Vocal Track",
            AddBus => "Add Bus",
            OpenAddTrackMenu => "Add Track…",
            ToggleMasterFxBypass => "Toggle Master FX Bypass",

            NewProject => "New Project",
            OpenProject => "Open Project…",
            SaveProject => "Save",
            SaveProjectAs => "Save As…",
            BounceToWav => "Bounce to WAV…",
            ExportChordSheet => "Export Chord Sheet…",
            OpenSettings => "Settings…",
        }
    }

    /// Category-prefixed path shown as a dimmed breadcrumb in the palette,
    /// e.g. `"Project › Save"`.
    pub fn breadcrumb(self) -> String {
        format!("{} › {}", self.category().display_name(), self.display_name())
    }

    /// Optional decorative glyph for the palette row. `None` for the many
    /// commands without a distinctive icon.
    pub fn glyph(self) -> Option<char> {
        use CommandId::*;
        match self {
            TransportPlay => Some('▶'),
            TransportStop => Some('■'),
            TransportPause => Some('⏸'),
            TransportRecord => Some('●'),
            TransportSkipBack => Some('⏮'),
            TransportSkipForward => Some('⏭'),
            TransportToggleLoop => Some('🔁'),
            TransportToggleMetronome => Some('🎵'),
            SaveProject | SaveProjectAs => Some('💾'),
            OpenProject => Some('📂'),
            _ => None,
        }
    }

    /// Build the [`Message`] this command dispatches. A fresh `Message` is
    /// constructed each call (rather than cloning a stored value), so commands
    /// never need `Message: Clone` and payload-carrying variants stay out of
    /// the registry entirely.
    pub fn to_message(self) -> Message {
        use CommandId::*;
        match self {
            TransportPlay => Message::Transport(TransportMessage::Play),
            TransportStop => Message::Transport(TransportMessage::Stop),
            TransportPause => Message::Transport(TransportMessage::Pause),
            TransportRecord => Message::Transport(TransportMessage::Record),
            TransportSkipBack => Message::Transport(TransportMessage::SkipBack),
            TransportSkipForward => Message::Transport(TransportMessage::SkipForward),
            TransportToggleLoop => Message::Transport(TransportMessage::ToggleLoop),
            TransportToggleMetronome => Message::Transport(TransportMessage::ToggleMetronome),
            TransportCycleTimeSignature => {
                Message::Transport(TransportMessage::CycleTimeSignature)
            }

            Undo => Message::Undo,
            Redo => Message::Redo,
            OpenSelectedMidiClip => {
                Message::MidiEditor(MidiEditorMessage::OpenSelectedMidiClip)
            }
            CloseMidiEditor => Message::MidiEditor(MidiEditorMessage::CloseMidiEditor),

            ViewArrange => Message::Ui(UiMessage::SwitchView(ViewMode::Arrange)),
            ViewMixer => Message::Ui(UiMessage::SwitchView(ViewMode::Mixer)),
            ViewCompose => Message::Ui(UiMessage::SwitchView(ViewMode::Compose)),
            // Routed through the focus-aware request, matching the existing
            // `F` handler in update.rs::subscription.
            TogglePerformanceMode => Message::Ui(UiMessage::RequestPerformanceToggle),
            ExitPerformanceMode => Message::Ui(UiMessage::ExitPerformanceMode),
            ZoomIn => Message::Viewport(ViewportMessage::ZoomIn),
            ZoomOut => Message::Viewport(ViewportMessage::ZoomOut),
            ToggleGlobalTracks => Message::Ui(UiMessage::ToggleGlobalTracks),

            ComposeCreateSection => {
                Message::Compose(crate::compose::ComposeMessage::OpenCreateSectionDialog)
            }
            ComposeCollapseTrack => {
                Message::Compose(crate::compose::ComposeMessage::CollapseTrack)
            }
            ComposeClearChordSelection => {
                Message::Compose(crate::compose::ComposeMessage::ClearChordSelection)
            }

            AddAudioTrack => Message::Track(TrackMessage::AddTrack),
            AddInstrumentTrack => Message::Track(TrackMessage::AddInstrumentTrack),
            AddVocalTrack => Message::Track(TrackMessage::AddVocalTrack),
            AddBus => Message::Bus(BusMessage::AddBus),
            OpenAddTrackMenu => Message::Ui(UiMessage::OpenAddTrackMenu),
            ToggleMasterFxBypass => Message::Master(MasterMessage::ToggleMasterFxBypass),

            NewProject => Message::Ui(UiMessage::StartNewProject),
            OpenProject => Message::ProjectIo(ProjectIoMessage::OpenProject),
            SaveProject => Message::ProjectIo(ProjectIoMessage::SaveProject),
            SaveProjectAs => Message::ProjectIo(ProjectIoMessage::SaveProjectAs),
            BounceToWav => Message::ProjectIo(ProjectIoMessage::BounceToWav),
            ExportChordSheet => Message::ProjectIo(ProjectIoMessage::ExportChordSheet),
            OpenSettings => Message::Ui(UiMessage::OpenSettings),
        }
    }
}

// ===========================================================================
// Key chords
// ===========================================================================

/// Modifier-key set for a [`KeyChord`]. Order-independent; `ctrl` and `cmd`
/// are kept distinct so the model is faithful on every platform even though
/// the default presets are authored macOS-first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub cmd: bool,
}

impl Mods {
    pub const NONE: Mods = Mods {
        ctrl: false,
        alt: false,
        shift: false,
        cmd: false,
    };

    pub const fn cmd() -> Mods {
        Mods {
            cmd: true,
            ..Mods::NONE
        }
    }
    pub const fn cmd_shift() -> Mods {
        Mods {
            cmd: true,
            shift: true,
            ..Mods::NONE
        }
    }
}

/// A named (non-character) key usable in a chord. Kept deliberately small —
/// just the keys the registry actually binds — and decoupled from iced so
/// parse/format are unit-testable without a windowing backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NamedKey {
    Enter,
    Escape,
    Space,
    Tab,
    Backspace,
    Delete,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    Plus,
    Minus,
    Comma,
}

impl NamedKey {
    /// Canonical token used by [`KeyChord`] parse/format round-tripping.
    fn token(self) -> &'static str {
        match self {
            NamedKey::Enter => "Enter",
            NamedKey::Escape => "Escape",
            NamedKey::Space => "Space",
            NamedKey::Tab => "Tab",
            NamedKey::Backspace => "Backspace",
            NamedKey::Delete => "Delete",
            NamedKey::ArrowUp => "ArrowUp",
            NamedKey::ArrowDown => "ArrowDown",
            NamedKey::ArrowLeft => "ArrowLeft",
            NamedKey::ArrowRight => "ArrowRight",
            NamedKey::Plus => "Plus",
            NamedKey::Minus => "Minus",
            NamedKey::Comma => "Comma",
        }
    }

    /// Keycap glyph used when formatting a chord for display.
    fn glyph(self) -> &'static str {
        match self {
            NamedKey::Enter => "↵",
            NamedKey::Escape => "Esc",
            NamedKey::Space => "Space",
            NamedKey::Tab => "⇥",
            NamedKey::Backspace => "⌫",
            NamedKey::Delete => "⌦",
            NamedKey::ArrowUp => "↑",
            NamedKey::ArrowDown => "↓",
            NamedKey::ArrowLeft => "←",
            NamedKey::ArrowRight => "→",
            NamedKey::Plus => "+",
            NamedKey::Minus => "−",
            NamedKey::Comma => ",",
        }
    }

    fn from_token(s: &str) -> Option<NamedKey> {
        let k = match s.to_ascii_lowercase().as_str() {
            "enter" | "return" | "↵" => NamedKey::Enter,
            "escape" | "esc" | "⎋" => NamedKey::Escape,
            "space" | "␣" => NamedKey::Space,
            "tab" | "⇥" => NamedKey::Tab,
            "backspace" | "⌫" => NamedKey::Backspace,
            "delete" | "del" | "⌦" => NamedKey::Delete,
            "arrowup" | "up" | "↑" => NamedKey::ArrowUp,
            "arrowdown" | "down" | "↓" => NamedKey::ArrowDown,
            "arrowleft" | "left" | "←" => NamedKey::ArrowLeft,
            "arrowright" | "right" | "→" => NamedKey::ArrowRight,
            "plus" => NamedKey::Plus,
            "minus" => NamedKey::Minus,
            "comma" => NamedKey::Comma,
            _ => return None,
        };
        Some(k)
    }
}

/// The non-modifier portion of a chord: either a printable character (stored
/// lowercased so `Shift` is the single source of truth for case) or a named
/// key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChordKey {
    Char(char),
    Named(NamedKey),
}

/// A keyboard shortcut: a set of modifiers plus one key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyChord {
    pub mods: Mods,
    pub key: ChordKey,
}

impl KeyChord {
    /// Construct a chord from a single character (lowercased) and modifiers.
    pub fn char(c: char, mods: Mods) -> KeyChord {
        KeyChord {
            mods,
            key: ChordKey::Char(c.to_ascii_lowercase()),
        }
    }

    /// Construct a chord from a named key and modifiers.
    pub fn named(key: NamedKey, mods: Mods) -> KeyChord {
        KeyChord {
            mods,
            key: ChordKey::Named(key),
        }
    }

    /// Parse a chord from a `"+"`-separated spec such as `"Cmd+Shift+S"`,
    /// `"Ctrl+Alt+Enter"`, `"F"`, or `"Escape"`. Modifier and key tokens are
    /// case-insensitive; glyphs (`⌘ ⌥ ⇧`) are accepted too. Returns `None` for
    /// an empty spec, an unknown token, or a missing/duplicate key.
    pub fn parse(spec: &str) -> Option<KeyChord> {
        let mut mods = Mods::NONE;
        let mut key: Option<ChordKey> = None;
        for raw in spec.split('+') {
            let token = raw.trim();
            if token.is_empty() {
                continue;
            }
            if let Some(()) = apply_modifier(&mut mods, token) {
                continue;
            }
            // Not a modifier — must be the (single) key.
            if key.is_some() {
                return None;
            }
            if let Some(named) = NamedKey::from_token(token) {
                key = Some(ChordKey::Named(named));
            } else {
                let mut chars = token.chars();
                let c = chars.next()?;
                if chars.next().is_some() {
                    // Multi-character token that isn't a known named key.
                    return None;
                }
                key = Some(ChordKey::Char(c.to_ascii_lowercase()));
            }
        }
        Some(KeyChord { mods, key: key? })
    }

    /// Render the chord as macOS keycap glyphs in canonical order
    /// (⌃⌥⇧⌘ then the key), e.g. `KeyChord::char('s', Mods::cmd_shift())`
    /// → `"⇧⌘S"`.
    pub fn format_glyphs(self) -> String {
        let mut out = String::new();
        if self.mods.ctrl {
            out.push('⌃');
        }
        if self.mods.alt {
            out.push('⌥');
        }
        if self.mods.shift {
            out.push('⇧');
        }
        if self.mods.cmd {
            out.push('⌘');
        }
        match self.key {
            ChordKey::Char(c) => out.extend(c.to_uppercase()),
            ChordKey::Named(n) => out.push_str(n.glyph()),
        }
        out
    }

    /// Render the chord with `"+"`-separated word tokens — the inverse of
    /// [`KeyChord::parse`] (round-trips for every chord this module builds).
    pub fn format_tokens(self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if self.mods.ctrl {
            parts.push("Ctrl".to_string());
        }
        if self.mods.alt {
            parts.push("Alt".to_string());
        }
        if self.mods.shift {
            parts.push("Shift".to_string());
        }
        if self.mods.cmd {
            parts.push("Cmd".to_string());
        }
        match self.key {
            ChordKey::Char(c) => parts.push(c.to_ascii_uppercase().to_string()),
            ChordKey::Named(n) => parts.push(n.token().to_string()),
        }
        parts.join("+")
    }
}

fn apply_modifier(mods: &mut Mods, token: &str) -> Option<()> {
    match token.to_ascii_lowercase().as_str() {
        "cmd" | "command" | "super" | "win" | "meta" | "⌘" => mods.cmd = true,
        "ctrl" | "control" | "⌃" => mods.ctrl = true,
        "alt" | "opt" | "option" | "⌥" => mods.alt = true,
        "shift" | "⇧" => mods.shift = true,
        _ => return None,
    }
    Some(())
}

impl std::fmt::Display for KeyChord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.format_glyphs())
    }
}

// ===========================================================================
// iced bridge
// ===========================================================================

impl KeyChord {
    /// Build a chord from a live iced key event, or `None` for keys the
    /// registry never binds (modifier-only presses, exotic named keys). The
    /// `cmd` modifier follows iced's [`Modifiers::command`], which already maps
    /// to ⌘ on macOS and Ctrl elsewhere.
    pub fn from_iced(key: &iced::keyboard::Key, modifiers: iced::keyboard::Modifiers) -> Option<KeyChord> {
        use iced::keyboard::key::Named as N;
        use iced::keyboard::Key;

        let mods = Mods {
            // `command()` is the platform-correct accelerator modifier.
            cmd: modifiers.command(),
            // Only surface a raw Ctrl when it isn't already standing in for
            // the command modifier (avoids double-counting on Windows/Linux).
            ctrl: modifiers.control() && !modifiers.command(),
            alt: modifiers.alt(),
            shift: modifiers.shift(),
        };

        let chord_key = match key {
            Key::Character(c) => {
                let ch = c.chars().next()?;
                if ch.is_whitespace() {
                    return None;
                }
                ChordKey::Char(ch.to_ascii_lowercase())
            }
            Key::Named(named) => ChordKey::Named(match named {
                N::Enter => NamedKey::Enter,
                N::Escape => NamedKey::Escape,
                N::Space => NamedKey::Space,
                N::Tab => NamedKey::Tab,
                N::Backspace => NamedKey::Backspace,
                N::Delete => NamedKey::Delete,
                N::ArrowUp => NamedKey::ArrowUp,
                N::ArrowDown => NamedKey::ArrowDown,
                N::ArrowLeft => NamedKey::ArrowLeft,
                N::ArrowRight => NamedKey::ArrowRight,
                _ => return None,
            }),
            _ => return None,
        };

        Some(KeyChord {
            mods,
            key: chord_key,
        })
    }
}

// ===========================================================================
// Keymap presets & binding table
// ===========================================================================

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

/// A bidirectional binding table mapping commands to chords. Stored as an
/// ordered list of pairs so lookups in both directions are simple linear scans
/// (the table is tens of entries, never hot).
#[derive(Debug, Clone, Default)]
pub struct BindingMap {
    entries: Vec<(CommandId, KeyChord)>,
}

impl BindingMap {
    /// The canonical Resonance default bindings. Includes — and extends — every
    /// shortcut currently hardcoded in `update.rs::subscription`.
    pub fn resonance_default() -> BindingMap {
        use CommandId::*;
        let cmd = Mods::cmd();
        let cmd_shift = Mods::cmd_shift();
        let none = Mods::NONE;

        let entries = vec![
            // Project (mirrors the existing Cmd+S / Cmd+Shift+S / Cmd+O).
            (NewProject, KeyChord::char('n', cmd)),
            (OpenProject, KeyChord::char('o', cmd)),
            (SaveProject, KeyChord::char('s', cmd)),
            (SaveProjectAs, KeyChord::char('s', cmd_shift)),
            (BounceToWav, KeyChord::char('b', cmd)),
            (ExportChordSheet, KeyChord::char('e', cmd)),
            (OpenSettings, KeyChord::char(',', cmd)),
            // Editing (mirrors the existing Cmd+Z / Cmd+Shift+Z / Cmd+Y, Enter).
            (Undo, KeyChord::char('z', cmd)),
            (Redo, KeyChord::char('z', cmd_shift)),
            (OpenSelectedMidiClip, KeyChord::named(NamedKey::Enter, none)),
            (CloseMidiEditor, KeyChord::named(NamedKey::Escape, cmd)),
            // Transport.
            (TransportPlay, KeyChord::named(NamedKey::Space, none)),
            (TransportStop, KeyChord::char('s', none)),
            (TransportRecord, KeyChord::char('r', none)),
            (TransportSkipBack, KeyChord::named(NamedKey::ArrowLeft, cmd)),
            (TransportSkipForward, KeyChord::named(NamedKey::ArrowRight, cmd)),
            (TransportToggleLoop, KeyChord::char('l', none)),
            (TransportToggleMetronome, KeyChord::char('m', none)),
            (TransportCycleTimeSignature, KeyChord::char('t', none)),
            // View & Navigation (mirrors the existing `F` / `Esc`).
            (ViewArrange, KeyChord::char('1', cmd)),
            (ViewMixer, KeyChord::char('2', cmd)),
            (ViewCompose, KeyChord::char('3', cmd)),
            (TogglePerformanceMode, KeyChord::char('f', none)),
            (ExitPerformanceMode, KeyChord::named(NamedKey::Escape, none)),
            (ZoomIn, KeyChord::char('=', cmd)),
            (ZoomOut, KeyChord::char('-', cmd)),
            (ToggleGlobalTracks, KeyChord::char('g', cmd)),
            // Compose & Vocal.
            (ComposeCreateSection, KeyChord::char('n', cmd_shift)),
            (ComposeCollapseTrack, KeyChord::named(NamedKey::Escape, Mods { shift: true, ..Mods::NONE })),
            // Mixer.
            (AddAudioTrack, KeyChord::char('t', cmd)),
            (AddInstrumentTrack, KeyChord::char('t', cmd_shift)),
            (OpenAddTrackMenu, KeyChord::char('t', Mods { cmd: true, alt: true, ..Mods::NONE })),
        ];

        BindingMap { entries }
    }

    /// Insert or replace the chord bound to `id`. Also removes any *other*
    /// command currently bound to `chord`, so a binding table never resolves
    /// one chord to two commands.
    pub fn set(&mut self, id: CommandId, chord: KeyChord) {
        self.entries.retain(|(other_id, other_chord)| *other_id != id && *other_chord != chord);
        self.entries.push((id, chord));
    }

    /// Remove any binding for `id`.
    pub fn clear(&mut self, id: CommandId) {
        self.entries.retain(|(other_id, _)| *other_id != id);
    }

    /// The command bound to `chord`, if any (lookup-by-chord).
    pub fn command_for(&self, chord: KeyChord) -> Option<CommandId> {
        self.entries
            .iter()
            .find(|(_, c)| *c == chord)
            .map(|(id, _)| *id)
    }

    /// The chord bound to `id`, if any (lookup-by-id).
    pub fn chord_for(&self, id: CommandId) -> Option<KeyChord> {
        self.entries
            .iter()
            .find(|(other, _)| *other == id)
            .map(|(_, c)| *c)
    }

    /// All `(command, chord)` pairs in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = (CommandId, KeyChord)> + '_ {
        self.entries.iter().copied()
    }

    /// Number of bound commands.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

// ===========================================================================
// Fuzzy matcher
// ===========================================================================

/// A successful fuzzy match: a relevance `score` (higher is better) and the
/// matched character ranges in the haystack as `[start, end)` half-open spans
/// over `char` indices, merged so adjacent matches form one highlight run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuzzyMatch {
    pub score: i32,
    pub ranges: Vec<(usize, usize)>,
}

/// Case-insensitive subsequence fuzzy match of `needle` against `haystack`.
///
/// Returns `None` unless every character of `needle` appears in `haystack` in
/// order. An empty needle matches everything with score `0` and no ranges.
/// Scoring rewards consecutive runs and matches at word boundaries (start of
/// string, or following a separator / case transition) and lightly penalises
/// leading and intermediate gaps, so `"opmc"` ranks "Open MIDI Clip" above a
/// scattered coincidental hit.
pub fn fuzzy_match(needle: &str, haystack: &str) -> Option<FuzzyMatch> {
    let needle: Vec<char> = needle.chars().filter(|c| !c.is_whitespace()).collect();
    if needle.is_empty() {
        return Some(FuzzyMatch {
            score: 0,
            ranges: Vec::new(),
        });
    }
    let hay: Vec<char> = haystack.chars().collect();

    let mut score: i32 = 0;
    let mut matched: Vec<usize> = Vec::with_capacity(needle.len());
    let mut hi = 0usize; // index into hay
    let mut prev_match: Option<usize> = None;

    for &nc in &needle {
        let target = nc.to_ascii_lowercase();
        let mut found = None;
        while hi < hay.len() {
            if hay[hi].to_ascii_lowercase() == target {
                found = Some(hi);
                break;
            }
            hi += 1;
        }
        let idx = found?;

        // Base reward for the match.
        score += 1;
        match prev_match {
            Some(p) if p + 1 == idx => {
                // Consecutive run — strongly preferred.
                score += 5;
            }
            Some(_) => {
                // A gap between matched chars; small penalty.
                score -= 1;
            }
            None => {
                // Leading gap before the first match; penalise distance.
                score -= idx as i32;
            }
        }
        if is_word_boundary(&hay, idx) {
            score += 3;
        }

        matched.push(idx);
        prev_match = Some(idx);
        hi = idx + 1;
    }

    Some(FuzzyMatch {
        score,
        ranges: merge_ranges(&matched),
    })
}

/// Whether `hay[idx]` begins a "word" — index 0, or preceded by a separator,
/// or a lower→upper case transition (camelCase boundary).
fn is_word_boundary(hay: &[char], idx: usize) -> bool {
    if idx == 0 {
        return true;
    }
    let prev = hay[idx - 1];
    if prev == ' ' || prev == '-' || prev == '_' || prev == '/' || prev == '›' || prev == '.' {
        return true;
    }
    prev.is_lowercase() && hay[idx].is_uppercase()
}

/// Collapse a sorted list of matched indices into half-open `[start, end)`
/// ranges, merging adjacent indices.
fn merge_ranges(indices: &[usize]) -> Vec<(usize, usize)> {
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for &i in indices {
        match ranges.last_mut() {
            Some(last) if last.1 == i => last.1 = i + 1,
            _ => ranges.push((i, i + 1)),
        }
    }
    ranges
}
