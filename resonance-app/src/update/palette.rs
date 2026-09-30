//! Command-palette reducer (command-palette.md §7.2, §7.3).

use iced::Task;

use crate::message::{Message, UiMessage};
use crate::palette::{self, PaletteItem, PaletteMode, PaletteMsg, PaletteState};
use crate::Resonance;

/// Row height in the result list, and a section header's (the list is laid
/// out at fixed heights so the reducer can keep the selection in view).
pub const ROW_HEIGHT: f32 = 50.0;
pub const HEADER_HEIGHT: f32 = 30.0;
/// The list's visible height.
pub const LIST_HEIGHT: f32 = 380.0;

/// Open the palette (or close it, when it is already open — ⌘K toggles).
/// It never opens over the recovery, startup or progress overlays, and it
/// closes any other overlay first.
pub(crate) fn open(r: &mut Resonance, mode: PaletteMode) -> Task<Message> {
    if r.ui.palette.is_some() {
        // ⌘K toggles it closed; ⌘J (another mode) switches to that mode.
        return match mode {
            PaletteMode::Commands => close(r),
            other => Task::batch([
                handle(r, PaletteMsg::Query(other.prefix().to_string())),
                iced::widget::operation::move_cursor_to_end(palette::query_input_id()),
            ]),
        };
    }
    let mut dismissed = Task::none();
    if let Some(overlay) = r.modal_overlay() {
        if !overlay.allows_palette() {
            return Task::none();
        }
        if let Some(dismiss) = overlay.dismiss_message(r) {
            dismissed = r.update(dismiss);
        }
    }
    // Closing keeps the last query, pre-selected so typing replaces it and
    // ↵ runs it again. Opening in another mode seeds that mode's prefix.
    let query = match mode {
        PaletteMode::Commands => r.ui.palette_memory.clone(),
        other => other.prefix().to_string(),
    };
    let mut state = PaletteState {
        query,
        ..PaletteState::default()
    };
    state.sections = palette::build(r, &state.query);
    r.ui.palette = Some(state);
    let id = palette::query_input_id();
    Task::batch([
        dismissed,
        iced::widget::operation::focus(id.clone()),
        iced::widget::operation::select_all(id),
    ])
}

/// Close the palette, remembering its query.
pub(crate) fn close(r: &mut Resonance) -> Task<Message> {
    if let Some(state) = r.ui.palette.take() {
        r.ui.palette_memory = state.query;
    }
    Task::none()
}

pub(crate) fn handle(r: &mut Resonance, msg: PaletteMsg) -> Task<Message> {
    let Some(state) = r.ui.palette.as_mut() else {
        return Task::none();
    };
    state.flash = None;
    match msg {
        PaletteMsg::Query(query) => {
            state.pointer_armed = false;
            state.selected = 0;
            state.scroll_y = 0.0;
            let sections = palette::build(r, &query);
            if let Some(state) = r.ui.palette.as_mut() {
                state.query = query;
                state.sections = sections;
            }
            return iced::widget::operation::scroll_to(
                palette::list_id(),
                iced::widget::scrollable::AbsoluteOffset { x: 0.0, y: 0.0 },
            );
        }
        PaletteMsg::Move(delta) => {
            let n = state.row_count();
            if n == 0 {
                return Task::none();
            }
            state.pointer_armed = false;
            state.selected = (state.selected as i64 + delta as i64).rem_euclid(n as i64) as usize;
            return follow_selection(state);
        }
        PaletteMsg::Hover(index) => {
            if state.pointer_armed && index < state.row_count() {
                state.selected = index;
            }
        }
        PaletteMsg::PointerMoved(at) => {
            // The first report is where the pointer already was when the
            // card appeared; only a later, different one is a real move.
            if state.pointer.is_some_and(|p| p != at) {
                state.pointer_armed = true;
            }
            state.pointer = Some(at);
        }
        PaletteMsg::Scrolled(y) => state.scroll_y = y,
        PaletteMsg::Submit => {
            let index = state.selected;
            return run(r, index);
        }
        PaletteMsg::Click(index) => return run(r, index),
    }
    Task::none()
}

/// The selected row's top edge in list coordinates.
fn row_top(state: &PaletteState, index: usize) -> f32 {
    let mut y = 0.0;
    let mut seen = 0;
    for section in &state.sections {
        y += HEADER_HEIGHT;
        if index < seen + section.rows.len() {
            return y + (index - seen) as f32 * ROW_HEIGHT;
        }
        seen += section.rows.len();
        y += section.rows.len() as f32 * ROW_HEIGHT;
    }
    y
}

/// Scroll just enough to keep the selected row visible.
fn follow_selection(state: &mut PaletteState) -> Task<Message> {
    let top = row_top(state, state.selected);
    let y = if top < state.scroll_y {
        top - HEADER_HEIGHT
    } else if top + ROW_HEIGHT > state.scroll_y + LIST_HEIGHT {
        top + ROW_HEIGHT - LIST_HEIGHT
    } else {
        return Task::none();
    };
    state.scroll_y = y.max(0.0);
    iced::widget::operation::scroll_to(
        palette::list_id(),
        iced::widget::scrollable::AbsoluteOffset {
            x: 0.0,
            y: state.scroll_y,
        },
    )
}

/// Run row `index`: close the palette, then dispatch directly — bypassing
/// the typing gate, since the palette's own field had the focus and the
/// user picked the command explicitly. Every other gate still applies. An
/// unavailable row does nothing but flash its reason.
fn run(r: &mut Resonance, index: usize) -> Task<Message> {
    let Some(state) = r.ui.palette.as_mut() else {
        return Task::none();
    };
    let Some(item) = state.rows().nth(index).map(|row| row.item.clone()) else {
        return Task::none();
    };
    // Availability can have changed since the row was built (the playhead
    // moved, a rescan ran): resolve it again and show the reason.
    let unavailable = match &item {
        PaletteItem::Command(command) => match command.availability(r) {
            crate::commands::Available::Yes => None,
            crate::commands::Available::No(reason) => Some(reason),
        },
        PaletteItem::Plugin(id) if !r.plugin_catalog.available_plugins.iter().any(|p| &p.clap_plugin_id == id) => {
            Some("That plugin is no longer available")
        }
        _ => state.rows().nth(index).and_then(|row| row.unavailable),
    };
    if let Some(reason) = unavailable {
        let state = r.ui.palette.as_mut().expect("checked above");
        state.selected = index;
        state.flash = Some(reason);
        if let Some(row) = state.sections.iter_mut().flat_map(|s| s.rows.iter_mut()).nth(index) {
            row.unavailable = Some(reason);
        }
        return Task::none();
    }
    let closed = close(r);
    use crate::message::{MarkerMessage, PluginMessage, TransportMessage};
    use crate::update::transport_nav::SeekTarget;
    let message = match item {
        PaletteItem::Command(command) => {
            return Task::batch([closed, crate::update::shortcuts::execute(r, command)]);
        }
        PaletteItem::Nudge(n) => Message::Transport(TransportMessage::SeekTo(SeekTarget::NudgeBars(n))),
        PaletteItem::GoTo { bar, beat } => {
            Message::Transport(TransportMessage::SeekTo(SeekTarget::Bar { bar, beat }))
        }
        PaletteItem::Marker(id) => Message::Marker(MarkerMessage::JumpTo(id)),
        PaletteItem::Section { start } => Message::Transport(TransportMessage::SeekToSample(start)),
        PaletteItem::Track(id) => Message::Ui(UiMessage::SelectTrack(Some(id))),
        PaletteItem::Plugin(id) => {
            let plugin = r
                .plugin_catalog
                .available_plugins
                .iter()
                .find(|p| p.clap_plugin_id == id)
                .cloned();
            let (Some(track), Some(plugin)) = (r.ui.interaction.selected_track, plugin) else {
                return closed;
            };
            Message::Plugin(PluginMessage::AddPluginToTrack(track, plugin))
        }
    };
    Task::batch([closed, r.update(message)])
}

/// Keys the palette takes while it is open, before the registry sees them
/// (the global subscription delivers them; see `shortcuts::handle_key`).
/// Returns `None` when the key is not the palette's.
pub(crate) fn key(
    r: &mut Resonance,
    chord: crate::commands::KeyChord,
    captured: bool,
) -> Option<Task<Message>> {
    use crate::commands::{KeyChord, Mods, NamedKey};
    let named = |n| KeyChord::named(n, Mods::NONE);
    if chord == named(NamedKey::Escape) {
        // The query field captures Esc (it unfocuses itself), so this runs
        // whatever the capture status.
        return Some(close(r));
    }
    if chord == named(NamedKey::ArrowUp) {
        return Some(handle(r, PaletteMsg::Move(-1)));
    }
    if chord == named(NamedKey::ArrowDown) {
        return Some(handle(r, PaletteMsg::Move(1)));
    }
    if chord == named(NamedKey::Enter) && !captured {
        // The field submits ↵ itself while it has focus; this covers a
        // click that took the focus away.
        return Some(handle(r, PaletteMsg::Submit));
    }
    None
}

/// `UiMessage` helper for the view.
pub fn msg(m: PaletteMsg) -> Message {
    Message::Ui(UiMessage::Palette(m))
}
