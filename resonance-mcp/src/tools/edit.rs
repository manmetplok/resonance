//! `edit_*` — undo / redo against the app's own history.

use crate::server::ResonanceMcp;
use rmcp::handler::server::tool::schema_for_output;
use rmcp::model::CallToolResult;
use rmcp::ErrorData as McpError;
use rmcp::{tool, tool_router};
use resonance_control::methods::edit;

#[tool_router(router = router_edit, vis = "pub(crate)")]
impl ResonanceMcp {
    #[tool(
        description = "What undo and redo would do right now, WITHOUT doing it: undo_label / \
                       redo_label name the edit at each end of the history (\"track volume\", \
                       \"add plugin\", \"note edit\"), plus can_undo / can_redo. \
                       \
                       Read this before edit_undo. The undo stack is SHARED with the user's \
                       own edits in the app's window — the top entry may be something THEY did \
                       seconds ago, and undoing it would silently revert their work. If the \
                       label does not match what you just did, do not undo. Read-only.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<edit::EditStatus>()
    )]
    async fn edit_status(&self) -> Result<CallToolResult, McpError> {
        self.invoke_structured(edit::STATUS, &()).await
    }

    #[tool(
        description = "Undo the most recent edit — the app's own history, the same one Ctrl+Z \
                       drives. Returns the label of what was undone plus the history's new \
                       state, so you can confirm you backed out what you meant to. An empty \
                       history is a clean no-op (undone: null), not an error. \
                       \
                       THE STACK IS SHARED with the user's GUI edits: call edit_status first \
                       and check that undo_label matches your own last edit, or you may revert \
                       their work instead of yours. The revision counter is how you detect \
                       that: if it advanced by more than your own calls, the user has been \
                       editing too. Undo is one step at a time, and a project load clears the \
                       history entirely.",
        annotations(destructive_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<edit::UndoResult>()
    )]
    async fn edit_undo(&self) -> Result<CallToolResult, McpError> {
        self.invoke_structured(edit::UNDO, &()).await
    }

    #[tool(
        description = "Re-apply the most recently undone edit. Returns the label of what came \
                       back plus the history's new state; nothing to redo is a clean no-op \
                       (redone: null), not an error. Note that making ANY new edit clears the \
                       redo stack, so redo is only available immediately after an undo. Like \
                       edit_undo this drives the app's shared history — check edit_status \
                       first.",
        annotations(destructive_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<edit::RedoResult>()
    )]
    async fn edit_redo(&self) -> Result<CallToolResult, McpError> {
        self.invoke_structured(edit::REDO, &()).await
    }
}
