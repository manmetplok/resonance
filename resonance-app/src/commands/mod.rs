//! Command registry — the single source of truth for every user-invokable
//! action in Resonance (command-palette.md §3).
//!
//! The global key handler, the command palette, tooltips and menu hints
//! all read from here; none of them hardcodes a chord. The module provides:
//!
//! 1. [`CommandId`] — one stable variant per action, each carrying metadata
//!    ([`CommandId::category`], [`CommandId::display_name`],
//!    [`CommandId::breadcrumb`], [`CommandId::glyph`],
//!    [`CommandId::keywords`], [`CommandId::gate`], [`CommandId::repeat`],
//!    [`CommandId::scope`]) plus, in `resolve.rs`, the state-aware
//!    [`CommandId::availability`] and [`CommandId::to_message`].
//! 2. [`KeyChord`] (`chord.rs`) — a portable modifier+key model with
//!    parse/format helpers and the iced bridge.
//! 3. [`BindingMap`] / [`KeymapPreset`] (`bindings.rs`) — the default binding
//!    table plus the DAW preset maps, with lookup-by-chord (per [`Scope`]) and
//!    lookup-by-id.
//! 4. [`fuzzy_match`] (`fuzzy.rs`) — a dependency-free subsequence matcher
//!    returning a score and the matched character ranges.

mod bindings;
mod chord;
mod fuzzy;
mod resolve;

pub use bindings::{BindingMap, KeymapPreset};
pub use chord::{ChordKey, KeyChord, Mods, NamedKey, Platform};
pub use fuzzy::{fuzzy_match, FuzzyMatch};

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
// Gates, scopes, availability
// ===========================================================================

/// Whether a shortcut may fire while a text field holds keyboard focus
/// (command-palette.md §3.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyGate {
    /// Fires regardless of text focus (⌘/Ctrl chords a text field never
    /// consumes).
    Always,
    /// Probed through the focus probe (`UiMessage::ShortcutProbed`) and
    /// dropped while a text field is being edited: every bare key, plus Undo / Redo
    /// (iced has no text undo, UPD-11).
    NotWhileTyping,
}

/// Where a binding is live. Canvas-scoped entries document the keys a
/// canvas handles itself; their handlers look the chord up in the same
/// [`BindingMap`], so the palette can list them and a rebinding covers
/// them (command-palette.md §4.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Scope {
    Global,
    Timeline,
    MidiEditor,
    VocalRoll,
    ExpandedEditor,
}

/// Whether a command can run against the current state. The reason is shown
/// on a dimmed palette row; an unavailable command is never hidden.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Available {
    Yes,
    No(&'static str),
}

impl Available {
    pub fn is_yes(self) -> bool {
        matches!(self, Available::Yes)
    }
}

// ===========================================================================
// Commands
// ===========================================================================

/// Declares [`CommandId`] and derives [`CommandId::ALL`] from the same list,
/// so the palette order can never miss a variant.
macro_rules! command_ids {
    ($( $(#[$meta:meta])* $variant:ident ),* $(,)?) => {
        /// A single user-invokable action. Variants are stable identifiers —
        /// bindings and the palette key off them, never off display strings.
        ///
        /// Actions that need a free-form runtime argument (a drag delta, a
        /// typed name) are deliberately *not* commands; actions on the
        /// current selection resolve their target in
        /// [`CommandId::to_message`].
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum CommandId {
            $( $(#[$meta])* $variant ),*
        }

        impl CommandId {
            /// Every command, in registry/palette order (declaration order).
            pub const ALL: &'static [CommandId] = &[ $( CommandId::$variant ),* ];

            /// Stable string id, used wherever a command is persisted (the
            /// palette's recents, keymap overrides), so saved state
            /// survives enum reordering. It is the variant name: renaming
            /// a variant is a format change.
            pub fn key(self) -> &'static str {
                match self {
                    $( CommandId::$variant => stringify!($variant) ),*
                }
            }

            /// The command whose [`key`](Self::key) is `key`.
            pub fn from_key(key: &str) -> Option<CommandId> {
                CommandId::ALL.iter().copied().find(|c| c.key() == key)
            }
        }
    };
}

command_ids! {
    // --- Transport ---
    TransportTogglePlay,
    TransportPlayPause,
    TransportPlay,
    TransportStop,
    TransportRecord,
    TransportToggleLoop,
    TransportToggleMetronome,
    TransportCycleTimeSignature,
    // Playhead and loop control.
    PlayheadToStart,
    PlayheadToEnd,
    PlayheadToLoopStart,
    PlayheadToLoopEnd,
    SetLoopStartAtPlayhead,
    SetLoopEndAtPlayhead,
    TransportPlayFromLoopStart,
    LoopSectionAtPlayhead,
    NudgeBackBar,
    NudgeForwardBar,
    NudgeBackBeat,
    NudgeForwardBeat,
    PrevMarker,
    NextMarker,
    PrevSectionStart,
    NextSectionStart,
    GoToBar,
    AddMarkerAtPlayhead,
    LoopSelection,
    TransportSkipBack,
    TransportSkipForward,
    ToggleFollowPlayhead,

    // --- Editing ---
    Undo,
    Redo,
    SplitClipAtPlayhead,
    DuplicateSelection,
    QuantizeSelectedNotes,
    OpenSelectedMidiClip,
    CloseMidiEditor,
    // Canvas-scoped: handled by the canvas that owns the keys, runnable
    // from the palette against the open surface's selection.
    TimelineDeleteSelection,
    DeleteSelectedNotes,
    SelectAllNotes,
    SelectNotesInView,
    VocalDeleteNote,
    VocalToggleSlur,
    ExpandedZoomIn,
    ExpandedZoomOut,

    // --- View & Navigation ---
    ViewArrange,
    ViewMixer,
    ViewCompose,
    TogglePerformanceMode,
    ExitPerformanceMode,
    ZoomIn,
    ZoomOut,
    ToggleGlobalTracks,
    ToggleBrowser,
    ToggleReferencePanel,
    ToggleMarkersOverview,

    // --- Compose & Vocal ---
    ComposeCreateSection,
    ComposeCollapseTrack,
    ComposeClearChordSelection,
    AddChordAtPlayhead,
    DeleteChordAtPlayhead,
    ToggleChordPinAtPlayhead,

    // --- Mixer ---
    AddAudioTrack,
    AddInstrumentTrack,
    AddVocalTrack,
    AddDrumTrack,
    AddBus,
    OpenAddTrackMenu,
    ToggleMasterFxBypass,
    GroupSelectedTracks,
    FreezeSelectedTracks,
    FreezeAllTracks,
    ToggleMuteSelected,
    ToggleSoloSelected,
    ToggleArmSelected,
    DeleteSelectedTrack,
    RescanPlugins,
    ShowMissingPlugins,
    // The selected plugin's presets (plugin-preset-library.md §6.6).
    PreviousPluginPreset,
    NextPluginPreset,
    BrowsePluginPresets,

    // --- Project ---
    CommandPalette,
    NewProject,
    OpenProject,
    SaveProject,
    SaveProjectAs,
    BounceToWav,
    ExportStemsMidi,
    ImportMidi,
    ImportAudio,
    SaveAsTemplate,
    RelinkMissingMedia,
    ExportChordSheet,
    OpenSettings,
}

impl CommandId {
    /// The category this command files under.
    pub fn category(self) -> CommandCategory {
        use CommandId::*;
        match self {
            TransportTogglePlay | TransportPlayPause | TransportPlayFromLoopStart
            | TransportPlay | TransportStop | TransportRecord | TransportSkipBack
            | TransportSkipForward | TransportToggleLoop | TransportToggleMetronome
            | TransportCycleTimeSignature | PlayheadToStart | PlayheadToEnd
            | PlayheadToLoopStart | PlayheadToLoopEnd | SetLoopStartAtPlayhead
            | SetLoopEndAtPlayhead | LoopSectionAtPlayhead | NudgeBackBar | NudgeForwardBar
            | NudgeBackBeat | NudgeForwardBeat | PrevMarker | NextMarker | PrevSectionStart
            | NextSectionStart | AddMarkerAtPlayhead | ToggleFollowPlayhead | LoopSelection
            | GoToBar => {
                CommandCategory::Transport
            }

            Undo | Redo | SplitClipAtPlayhead | DuplicateSelection | QuantizeSelectedNotes
            | OpenSelectedMidiClip | CloseMidiEditor | TimelineDeleteSelection
            | DeleteSelectedNotes | SelectAllNotes | SelectNotesInView | VocalDeleteNote
            | VocalToggleSlur | ExpandedZoomIn | ExpandedZoomOut => CommandCategory::Editing,

            ViewArrange | ViewMixer | ViewCompose | TogglePerformanceMode | ExitPerformanceMode
            | ZoomIn | ZoomOut | ToggleGlobalTracks | ToggleBrowser | ToggleReferencePanel
            | ToggleMarkersOverview => CommandCategory::ViewNav,

            ComposeCreateSection | ComposeCollapseTrack | ComposeClearChordSelection
            | AddChordAtPlayhead | DeleteChordAtPlayhead | ToggleChordPinAtPlayhead => {
                CommandCategory::ComposeVocal
            }

            AddAudioTrack | AddInstrumentTrack | AddVocalTrack | AddBus | OpenAddTrackMenu
            | ToggleMasterFxBypass | GroupSelectedTracks | FreezeSelectedTracks
            | FreezeAllTracks | AddDrumTrack | ToggleMuteSelected | ToggleSoloSelected
            | ToggleArmSelected | DeleteSelectedTrack | RescanPlugins | ShowMissingPlugins
            | PreviousPluginPreset | NextPluginPreset | BrowsePluginPresets => {
                CommandCategory::Mixer
            }

            CommandPalette | NewProject | OpenProject | SaveProject | SaveProjectAs
            | BounceToWav | ExportStemsMidi | ImportMidi | ImportAudio | SaveAsTemplate
            | RelinkMissingMedia | ExportChordSheet | OpenSettings => CommandCategory::Project,
        }
    }

    /// Short label shown in menus and the palette.
    pub fn display_name(self) -> &'static str {
        use CommandId::*;
        match self {
            TransportTogglePlay => "Play / Stop",
            TransportPlayPause => "Play / Pause",
            TransportPlayFromLoopStart => "Play from Loop Start",
            TransportPlay => "Play",
            TransportStop => "Stop and Return to Zero",
            TransportRecord => "Record",
            TransportToggleLoop => "Toggle Loop",
            TransportToggleMetronome => "Toggle Metronome",
            TransportCycleTimeSignature => "Cycle Time Signature",
            PlayheadToStart => "Playhead to Project Start",
            PlayheadToEnd => "Playhead to Project End",
            PlayheadToLoopStart => "Playhead to Loop Start",
            PlayheadToLoopEnd => "Playhead to Loop End",
            SetLoopStartAtPlayhead => "Set Loop Start at Playhead",
            SetLoopEndAtPlayhead => "Set Loop End at Playhead",
            LoopSectionAtPlayhead => "Loop Section at Playhead",
            NudgeBackBar => "Nudge Playhead Back 1 Bar",
            NudgeForwardBar => "Nudge Playhead Forward 1 Bar",
            NudgeBackBeat => "Nudge Playhead Back 1 Beat",
            NudgeForwardBeat => "Nudge Playhead Forward 1 Beat",
            PrevMarker => "Previous Marker",
            NextMarker => "Next Marker",
            PrevSectionStart => "Previous Section Start",
            NextSectionStart => "Next Section Start",
            AddMarkerAtPlayhead => "Add Marker at Playhead",
            TransportSkipBack => "Rewind 5 s",
            TransportSkipForward => "Fast-forward 5 s",
            ToggleFollowPlayhead => "Toggle Follow Playhead",
            LoopSelection => "Loop Selection",
            GoToBar => "Go to Bar…",

            Undo => "Undo",
            Redo => "Redo",
            SplitClipAtPlayhead => "Split Clip at Playhead",
            DuplicateSelection => "Duplicate Selection",
            QuantizeSelectedNotes => "Quantize Selected Notes",
            OpenSelectedMidiClip => "Open Selected MIDI Clip",
            CloseMidiEditor => "Close MIDI Editor",
            TimelineDeleteSelection => "Delete Selection",
            DeleteSelectedNotes => "Delete Selected Notes",
            SelectAllNotes => "Select All Notes",
            SelectNotesInView => "Select Notes in View",
            VocalDeleteNote => "Delete Selected Vocal Note",
            VocalToggleSlur => "Toggle Slur",
            ExpandedZoomIn => "Zoom Track Editor In",
            ExpandedZoomOut => "Zoom Track Editor Out",

            ViewArrange => "Arrange View",
            ViewMixer => "Mixer View",
            ViewCompose => "Compose View",
            TogglePerformanceMode => "Toggle Performance Mode",
            ExitPerformanceMode => "Exit Performance Mode",
            ZoomIn => "Zoom In",
            ZoomOut => "Zoom Out",
            ToggleGlobalTracks => "Toggle Global Tracks",
            ToggleBrowser => "Toggle Browser",
            ToggleReferencePanel => "Toggle Reference Panel",
            ToggleMarkersOverview => "Toggle Markers Overview",

            ComposeCreateSection => "New Section…",
            ComposeCollapseTrack => "Collapse Track Editor",
            ComposeClearChordSelection => "Clear Chord Selection",
            AddChordAtPlayhead => "Add Chord at Playhead",
            DeleteChordAtPlayhead => "Delete Chord at Playhead",
            ToggleChordPinAtPlayhead => "Pin / Unpin Chord at Playhead",

            AddAudioTrack => "Add Audio Track",
            AddInstrumentTrack => "Add Instrument Track",
            AddVocalTrack => "Add Vocal Track",
            AddDrumTrack => "Add Drum Track",
            AddBus => "Add Bus",
            OpenAddTrackMenu => "Add Track…",
            ToggleMasterFxBypass => "Toggle Master FX Bypass",
            GroupSelectedTracks => "Group Selected Tracks",
            FreezeSelectedTracks => "Freeze Selected Tracks",
            FreezeAllTracks => "Freeze All Tracks",
            ToggleMuteSelected => "Mute Selected Tracks",
            ToggleSoloSelected => "Solo Selected Tracks",
            ToggleArmSelected => "Arm Selected Tracks",
            DeleteSelectedTrack => "Delete Selected Track…",
            RescanPlugins => "Rescan Plugins",
            ShowMissingPlugins => "Show Missing Plugins",
            PreviousPluginPreset => "Previous Preset",
            NextPluginPreset => "Next Preset",
            BrowsePluginPresets => "Browse Presets…",

            CommandPalette => "Command Palette",
            NewProject => "New Project",
            OpenProject => "Open Project…",
            SaveProject => "Save",
            SaveProjectAs => "Save As…",
            BounceToWav => "Bounce to WAV…",
            ExportStemsMidi => "Export Stems / MIDI…",
            ImportMidi => "Import MIDI…",
            ImportAudio => "Import Audio to Pool…",
            SaveAsTemplate => "Save as Template",
            RelinkMissingMedia => "Relink Missing Media…",
            ExportChordSheet => "Export Chord Sheet…",
            OpenSettings => "Settings…",
        }
    }

    /// Category-prefixed path shown as a dimmed breadcrumb in the palette,
    /// e.g. `"Project › Save"`.
    pub fn breadcrumb(self) -> String {
        format!("{} › {}", self.category().display_name(), self.display_name())
    }

    /// The palette row's glyph: a Font Awesome codepoint (`theme::ICON_FONT`).
    /// Commands without a distinctive icon use their category's.
    pub fn glyph(self) -> Option<char> {
        use crate::theme::fa;
        use CommandId::*;
        let icon = match self {
            TransportTogglePlay | TransportPlay | TransportPlayFromLoopStart => fa::PLAY,
            TransportStop => fa::STOP,
            TransportPlayPause => fa::PAUSE,
            TransportRecord => fa::CIRCLE,
            TransportSkipBack => fa::BACKWARD_FAST,
            TransportSkipForward => fa::FORWARD_FAST,
            PlayheadToStart | NudgeBackBar | NudgeBackBeat | PrevSectionStart => {
                fa::BACKWARD_STEP
            }
            PlayheadToEnd | NudgeForwardBar | NudgeForwardBeat | NextSectionStart => {
                fa::FORWARD_STEP
            }
            TransportToggleMetronome => fa::METRONOME,
            TransportToggleLoop | PlayheadToLoopStart | PlayheadToLoopEnd
            | SetLoopStartAtPlayhead | SetLoopEndAtPlayhead | LoopSectionAtPlayhead => {
                fa::ARROW_ROTATE_LEFT
            }
            PrevMarker | NextMarker | AddMarkerAtPlayhead => fa::FLAG,
            PreviousPluginPreset => fa::BACKWARD_STEP,
            NextPluginPreset => fa::FORWARD_STEP,
            Undo | Redo => fa::ARROW_ROTATE_LEFT,
            ZoomIn => fa::MAGNIFYING_GLASS_PLUS,
            ZoomOut => fa::MAGNIFYING_GLASS_MINUS,
            TogglePerformanceMode | ExitPerformanceMode => fa::GUITAR,
            FreezeSelectedTracks | FreezeAllTracks => fa::SNOWFLAKE,
            AddVocalTrack => fa::MICROPHONE,
            AddInstrumentTrack => fa::MUSIC,
            AddAudioTrack => fa::WAVE_SQUARE,
            OpenProject => fa::FOLDER_OPEN,
            SaveProject | SaveProjectAs => fa::FLOPPY_DISK,
            BounceToWav => fa::COMPACT_DISC,
            CommandPalette => fa::MAGNIFYING_GLASS,
            OpenSettings => fa::SLIDERS,
            _ => match self.category() {
                CommandCategory::Transport => fa::CLOCK,
                CommandCategory::Editing => fa::BARS,
                CommandCategory::ViewNav => fa::EYE,
                CommandCategory::ComposeVocal => fa::MUSIC,
                CommandCategory::Mixer => fa::SLIDERS,
                CommandCategory::Project => fa::FOLDER,
            },
        };
        Some(icon)
    }

    /// Search aliases. [`fuzzy_match`] runs over the display name first and
    /// over these at a lower weight.
    pub fn keywords(self) -> &'static [&'static str] {
        use CommandId::*;
        match self {
            TransportTogglePlay => &["start", "transport", "spacebar"],
            TransportPlayPause => &["stop in place", "hold", "transport"],
            TransportPlayFromLoopStart => &["cycle", "transport"],
            TransportPlay => &["start", "transport"],
            TransportStop => &["transport", "halt", "zero", "rewind"],
            PlayheadToStart => &["home", "beginning", "rewind", "zero"],
            PlayheadToEnd => &["end", "last"],
            PlayheadToLoopStart | PlayheadToLoopEnd => &["cycle", "locate", "jump"],
            SetLoopStartAtPlayhead | SetLoopEndAtPlayhead => &["cycle", "in point", "out point"],
            LoopSectionAtPlayhead => &["cycle", "part", "verse", "chorus"],
            NudgeBackBar | NudgeForwardBar | NudgeBackBeat | NudgeForwardBeat => {
                &["move", "step", "playhead"]
            }
            PrevSectionStart | NextSectionStart => &["part", "jump", "verse", "chorus"],
            AddMarkerAtPlayhead => &["locator", "flag", "cue"],
            TransportSkipBack | TransportSkipForward => &["seconds", "skip", "scrub"],
            ToggleFollowPlayhead => &["scroll", "track playhead"],
            LoopSelection => &["cycle", "clip", "region"],
            GoToBar => &["jump", "locate", "position", "measure"],
            SplitClipAtPlayhead => &["cut", "razor", "slice"],
            DuplicateSelection => &["copy", "repeat", "clone"],
            QuantizeSelectedNotes => &["grid", "snap", "timing"],
            TimelineDeleteSelection | DeleteSelectedNotes | VocalDeleteNote => {
                &["remove", "erase", "backspace"]
            }
            SelectAllNotes | SelectNotesInView => &["piano roll", "notes"],
            VocalToggleSlur => &["legato", "melisma", "lyric"],
            ExpandedZoomIn | ExpandedZoomOut => &["vertical", "height"],
            ToggleBrowser => &["media", "files", "pool", "samples"],
            ToggleReferencePanel => &["a/b", "compare", "reference track"],
            ToggleMarkersOverview => &["locators", "list"],
            AddChordAtPlayhead | DeleteChordAtPlayhead | ToggleChordPinAtPlayhead => {
                &["chord track", "harmony", "region"]
            }
            AddDrumTrack => &["new track", "beat", "kit"],
            ToggleMuteSelected => &["silence", "mute"],
            ToggleSoloSelected => &["isolate", "solo"],
            ToggleArmSelected => &["record arm", "enable recording"],
            DeleteSelectedTrack => &["remove", "trash"],
            RescanPlugins => &["clap", "scan", "refresh"],
            ShowMissingPlugins => &["clap", "unavailable"],
            PreviousPluginPreset | NextPluginPreset => &["plugin", "patch", "program", "step"],
            BrowsePluginPresets => &["plugin", "patch", "library", "favorites"],
            ExportStemsMidi => &["render", "stems", "midi file"],
            ImportMidi => &["midi file", "load"],
            ImportAudio => &["wav", "samples", "load"],
            SaveAsTemplate => &["preset", "starter"],
            RelinkMissingMedia => &["missing files", "locate"],
            TransportRecord => &["arm", "capture", "take"],
            TransportToggleLoop => &["cycle", "repeat"],
            TransportToggleMetronome => &["click", "count"],
            TransportCycleTimeSignature => &["meter", "time signature"],
            NextMarker | PrevMarker => &["marker", "locator", "jump"],
            Undo => &["revert", "back"],
            Redo => &["again", "forward"],
            OpenSelectedMidiClip => &["piano roll", "edit notes"],
            CloseMidiEditor => &["piano roll"],
            ViewArrange => &["timeline", "tracks", "tab"],
            ViewMixer => &["mix", "faders", "tab"],
            ViewCompose => &["sections", "chords", "tab"],
            TogglePerformanceMode | ExitPerformanceMode => &["stage", "live", "fullscreen"],
            ZoomIn | ZoomOut => &["magnify", "scale"],
            ToggleGlobalTracks => &["tempo", "signature", "shelf"],
            ComposeCreateSection => &["verse", "chorus", "part"],
            AddAudioTrack | AddInstrumentTrack | AddVocalTrack => &["new track", "create"],
            AddBus => &["group", "return", "aux"],
            OpenAddTrackMenu => &["new track", "create"],
            ToggleMasterFxBypass => &["master", "effects", "mastering"],
            GroupSelectedTracks => &["folder", "link"],
            FreezeSelectedTracks | FreezeAllTracks => &["render", "bounce", "cpu"],
            CommandPalette => &["search", "find", "actions"],
            NewProject => &["create", "file"],
            OpenProject => &["load", "file"],
            SaveProject | SaveProjectAs => &["write", "file"],
            BounceToWav => &["export", "render", "mixdown"],
            ExportChordSheet => &["pdf", "lead sheet"],
            OpenSettings => &["preferences", "options", "config"],
            _ => &[],
        }
    }

    /// Whether this command may fire while a text field holds focus. Every
    /// default binding without ⌘/Ctrl must be [`KeyGate::NotWhileTyping`]
    /// (enforced by a registry test).
    pub fn gate(self) -> KeyGate {
        use CommandId::*;
        match self {
            // ⌘/Ctrl-only chords a text field never consumes.
            GoToBar | CommandPalette | SaveProject | SaveProjectAs | OpenProject | NewProject | BounceToWav
            | ExportChordSheet | OpenSettings | GroupSelectedTracks | FreezeSelectedTracks
            | FreezeAllTracks | ViewArrange | ViewMixer | ViewCompose | ZoomIn | ZoomOut
            | ToggleGlobalTracks | AddAudioTrack | AddInstrumentTrack | OpenAddTrackMenu
            | CloseMidiEditor | SplitClipAtPlayhead | DuplicateSelection | LoopSelection
            | ToggleBrowser | DeleteSelectedTrack | ExportStemsMidi | ImportMidi
            | ImportAudio => KeyGate::Always,
            // Undo / Redo are ⌘ chords but also what a user presses inside a
            // text field (UPD-11); everything else is (or may become) a bare
            // key.
            _ => KeyGate::NotWhileTyping,
        }
    }

    /// Whether running this command puts it in the palette's *Recent*:
    /// not for the commands that only open the palette.
    pub fn records_recent(self) -> bool {
        !matches!(self, CommandId::CommandPalette | CommandId::GoToBar)
    }

    /// Whether key repeat re-fires this command. True only for nudges and
    /// zoom — never for a toggle.
    pub fn repeat(self) -> bool {
        use CommandId::*;
        matches!(
            self,
            ZoomIn | ZoomOut | NudgeBackBar | NudgeForwardBar | NudgeBackBeat | NudgeForwardBeat
                | ExpandedZoomIn | ExpandedZoomOut
        )
    }

    /// Where the command's binding is live.
    pub fn scope(self) -> Scope {
        use CommandId::*;
        match self {
            TimelineDeleteSelection => Scope::Timeline,
            DeleteSelectedNotes | SelectAllNotes | SelectNotesInView => Scope::MidiEditor,
            VocalDeleteNote | VocalToggleSlur => Scope::VocalRoll,
            ExpandedZoomIn | ExpandedZoomOut | ComposeCollapseTrack => Scope::ExpandedEditor,
            _ => Scope::Global,
        }
    }
}
