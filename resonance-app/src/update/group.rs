//! Track-group (folder-track) reducers (epic #36, doc #200).
//!
//! The group-header row view (todo #680) emits [`GroupMessage`]s for its
//! caret, macro `M`/`S` buttons and level trim. The behaviour behind each
//! of those controls is owned by dedicated follow-up todos so the macro
//! cascade and fold-state logic land with their tests, not buried in the
//! view todo:
//!
//! - collapse / fold state in the timeline — todo #686
//! - macro mute with cascade to members — todo #687
//! - macro solo with cascade to members — todo #688
//! - group level trim (macro scaling) — todo #689
//!
//! Until those land this handler is an inert placeholder: it routes every
//! variant to a no-op so the header is wired end-to-end (and snapshot-
//! testable) without pre-empting the reducer todos.

use iced::Task;

use crate::message::{GroupMessage, Message};
use crate::Resonance;

pub fn handle(r: &mut Resonance, m: GroupMessage) -> Task<Message> {
    match m {
        // Behaviour implemented in todos #686–#689; see module docs.
        GroupMessage::ToggleCollapse(_)
        | GroupMessage::ToggleMacroMute(_)
        | GroupMessage::ToggleMacroSolo(_)
        | GroupMessage::SetMacroLevel(_, _) => {}
        GroupMessage::CreateGroupFromSelection => create_group_from_selection(r),
    }
    Task::none()
}

/// Fold the current multi-track selection into a fresh group (the
/// "Group selected" bar / `Cmd-G`). A group of one is meaningless, so this
/// no-ops below two selected tracks — matching the bar's visibility
/// threshold. The group id is drawn from the shared track-id allocator so
/// it can never collide with a real track. The selection is cleared once
/// the group exists, so the floating bar dismisses itself.
fn create_group_from_selection(r: &mut Resonance) {
    let members = r.interaction.selected_tracks.clone();
    if members.len() < 2 {
        return;
    }
    let group_id = r.registry.allocate_sub_track_id();
    r.track_groups.create_group_from_selection(group_id, &members);
    r.interaction.select_single_track(None);
}
