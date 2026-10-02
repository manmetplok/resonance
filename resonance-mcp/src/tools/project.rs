//! `project_*` — project lifecycle with explicit paths (never file
//! dialogs). All four run as jobs in the app; the tools wait a bounded
//! time and return the final job status.

use crate::server::ResonanceMcp;
use rmcp::handler::server::tool::schema_for_output;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::ErrorData as McpError;
use rmcp::{tool, tool_router};
use resonance_control::job::JobStatus;
use resonance_control::methods::project;

/// How long project open/save jobs are awaited before handing the model
/// the job id to poll.
const PROJECT_WAIT_MS: u64 = 30_000;

#[tool_router(router = router_project, vis = "pub(crate)")]
impl ResonanceMcp {
    #[tool(
        description = "Start a fresh project (optional template name). If the current project \
                       has unsaved changes this is refused with a summary — pass confirm: true \
                       to discard them. Save first with project_save if in doubt. The new \
                       project is untitled; its edits are undoable (edit_undo) before it is \
                       ever saved.",
        annotations(destructive_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<JobStatus>()
    )]
    async fn project_new(
        &self,
        Parameters(params): Parameters<project::NewParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_job(project::NEW, &params, PROJECT_WAIT_MS).await
    }

    #[tool(
        description = "Open a project from an absolute path. Refused with a summary when the \
                       current project has unsaved changes — pass confirm: true to discard them. \
                       Opens the last saved version; autosave_available in the result means a \
                       crash left newer work behind — reopen with recover_autosave: true to load it.",
        annotations(destructive_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<JobStatus>()
    )]
    async fn project_open(
        &self,
        Parameters(params): Parameters<project::OpenParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_job(project::OPEN, &params, PROJECT_WAIT_MS).await
    }

    #[tool(
        description = "Save the project in place; pass path (absolute) only on first save. \
                       Overwriting an existing file at a NEW path needs confirm: true.",
        annotations(destructive_hint = false, idempotent_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<JobStatus>()
    )]
    async fn project_save(
        &self,
        Parameters(params): Parameters<project::SaveParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_job(project::SAVE, &params, PROJECT_WAIT_MS).await
    }

    #[tool(
        description = "Save the project to a new absolute path. Overwriting an existing file \
                       there needs confirm: true.",
        annotations(destructive_hint = true, open_world_hint = false),
        output_schema = schema_for_output::<JobStatus>()
    )]
    async fn project_save_as(
        &self,
        Parameters(params): Parameters<project::SaveAsParams>,
    ) -> Result<CallToolResult, McpError> {
        self.invoke_job(project::SAVE_AS, &params, PROJECT_WAIT_MS)
            .await
    }
}
