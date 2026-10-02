//! Transient view-layer/session UI state (ARCH-06 second tier, A-12e):
//! pure presentation and session-interaction bookkeeping that is never
//! persisted and never rides the undo snapshot.
//!
//! Grouped together per the A-12 survey
//! (`docs/design/A-12-resonance-fields.md`): the current tab, the tab
//! Performance mode was entered from, cached pick-list options, the
//! transport-bar label cache, mixer-panel UI state, and timeline/clip
//! interaction state, plus the report of the last arrangement bar-shift.
//! `Resonance::update_depth` — a re-entrancy guard around the `update()`
//! dispatch loop itself, not view-presentation state — is deliberately
//! left out; see its doc comment on `Resonance`.

use crate::state::ClipInteractionState;
use crate::state::MixerUiState;
use crate::state::ViewMode;

/// Transient view-layer/session UI state — never persisted, never in the
/// undo snapshot.
///
/// No `Clone` / `Default`: `ClipInteractionState` and `MixerUiState` are
/// `Debug, Default` only (no `Clone`), and `ViewMode` has no `Default` (it
/// is always given explicitly — see [`Resonance::assemble`]), matching
/// `Resonance` itself, which was never `Clone` or `Default` either.
#[derive(Debug)]
pub struct UiTransientState {
    /// Cached pick-list option lists for the view layer. Rebuilt only
    /// when source data changes (devices, busses, plugin scan) so a
    /// continuous resize doesn't reallocate option vecs every frame.
    /// See `view::ui_caches` for the cache and rebuild API.
    ///
    /// `pub(crate)`, not `pub` like this struct's other fields: `UiViewCaches`
    /// itself is `pub(crate)` (a view-layer-only cache type), so this field
    /// keeps that visibility rather than widening it.
    pub(crate) view_caches: crate::view::ui_caches::UiViewCaches,
    /// Lazy-memoised label strings for the transport bar's stat blocks
    /// (position, time, sig, key, loop). Re-formatted only when the
    /// underlying inputs change. Refreshed by `refresh_transport_labels`
    /// after every `update()` dispatch (plus at construction and after
    /// demo seeding) so `view()` only ever reads it — the view layer
    /// never mutates state. See `view::transport_labels`.
    ///
    /// `pub(crate)` for the same reason as `view_caches` above.
    pub(crate) transport_labels: crate::view::transport_labels::TransportLabels,
    /// The active tab (Arrange / Mixer / Compose / Performance).
    pub view_mode: ViewMode,
    /// The view that was active when Performance mode was entered, so
    /// exiting (`F` toggle / `Esc` / the Exit button) returns the user to
    /// where they were rather than always to Arrange. `None` whenever the
    /// current `view_mode` is not `Performance`.
    pub pre_performance_view: Option<ViewMode>,
    /// What the last `arrangement.insert_bars` / `remove_bars` moved.
    ///
    /// A bar shift touches five collections at once, so its report can
    /// only be assembled while it runs — the control handler dispatches
    /// the edit through `update()` (which is what makes it one undo
    /// entry) and reads the tally back from here. Overwritten by each
    /// shift and never persisted.
    pub last_arrangement_shift: Option<crate::update::arrangement::ShiftOutcome>,
    /// Timeline/clip selection and interaction state (drag, trim, MIDI
    /// editor, expanded-lane tracking, ...).
    pub interaction: ClipInteractionState,
    /// Mixer-panel UI state (selection, collapsed inspector groups, open
    /// menus, ...).
    pub mixer: MixerUiState,
    /// The active keymap: what every global shortcut and canvas-local key
    /// resolves through (command-palette.md §4): the preset and overrides
    /// in `settings.keymap`, resolved by `update::keymap::resolve`.
    pub keymap: crate::commands::BindingMap,
    /// How the typing gate asks whether a text field holds focus; always
    /// `Live` outside tests.
    pub typing_probe: crate::update::shortcuts::TypingProbe,
    /// The open command palette, if any (command-palette.md §7).
    pub palette: Option<crate::palette::PaletteState>,
    /// The query the palette closed with, restored (pre-selected) on the
    /// next open so "run it again" is ⌘K ↵.
    pub palette_memory: String,
    /// Set when *Recent* changed and `settings.json` has not been written
    /// since; the tick writes it after a quiet spell.
    pub recent_dirty_since: Option<std::time::Instant>,
    /// The Settings overlay's tab and the Keyboard panel's state.
    pub keymap_editor: crate::update::keymap::KeymapEditorState,
    /// The app window's last-known inner size, from the window's
    /// `Opened` / `Resized` events. Starts at [`DEFAULT_WINDOW_SIZE`] (the
    /// window's own minimum, so never larger than the real window). The
    /// floating generic plugin window clamps its position to it so its
    /// title bar stays reachable.
    pub window_size: iced::Size,
}

/// The app window's opening size, which is also its minimum
/// (`main.rs`).
pub const DEFAULT_WINDOW_SIZE: iced::Size = iced::Size::new(1440.0, 900.0);

impl UiTransientState {
    /// Select a single track (or clear the selection with `None`) — the
    /// one entry point every track selection goes through. Beyond
    /// [`ClipInteractionState::select_single_track`] it takes the mixer's
    /// bus / master selection off when a track is selected: the inspector
    /// shows a selected bus first, then the master, then a track, so a
    /// track selected from anywhere (a clip drag or trim, the Arrange
    /// context menu) would otherwise leave the inspector describing a
    /// stale bus or the master.
    pub fn select_track(&mut self, id: Option<resonance_audio::types::TrackId>) {
        if id.is_some() {
            self.clear_channel_selection();
        }
        self.interaction.select_single_track(id);
    }

    /// Toggle a track in the multi-selection (an additive click), with
    /// the same bus / master exclusivity as [`Self::select_track`].
    pub fn toggle_track_selection(&mut self, id: resonance_audio::types::TrackId) {
        self.clear_channel_selection();
        self.interaction.toggle_track_selection(id);
    }

    /// Drop the mixer's bus and master selection.
    pub fn clear_channel_selection(&mut self) {
        self.mixer.selected_bus = None;
        self.mixer.selected_master = false;
    }
}
