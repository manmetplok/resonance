//! The MCP server: shared state, invocation helpers, and the combined
//! tool router.
//!
//! The individual `#[tool]` methods live in [`crate::tools`], one module
//! per control-protocol namespace; each module contributes a named
//! router and [`ResonanceMcp::combined_router`] sums them. Everything
//! here follows ba docs #265/#266:
//!
//! - every recoverable failure (app not running, `not_found`,
//!   `needs_confirmation`, ...) is a **tool execution error**
//!   ([`CallToolResult::error`]) with actionable text the model can
//!   self-correct from — never a protocol error;
//! - read-only views return **structured content** whose schema is
//!   generated from the exact `resonance-control` wire types;
//! - long-running operations return the final [`JobStatus`] after an
//!   internal bounded `job.wait`, so the model usually gets one
//!   round-trip and can fall back to `job_status` polling.

use crate::client::ControlClient;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::model::{
    CallToolResult, ContentBlock, Implementation, InitializeResult, ServerCapabilities, ServerInfo,
};
use rmcp::{ErrorData as McpError, ServerHandler};
use resonance_control::ids::JobId;
use resonance_control::job::{JobStarted, JobState, JobStatus};
use serde::Serialize;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;

/// Cross-tool guidance sent to the client as server `instructions`.
const INSTRUCTIONS: &str = "\
Drives a RUNNING resonance DAW (the app must be open; every edit appears live in its GUI and \
lands in the app's undo history like a manual edit).\n\
\n\
Workflow: read before you write — song_summary gives the whole-song overview; song_sections, \
song_tracks, song_notes and song_vocal drill down. All ids (track_id, clip_id, section_id, \
placement_id, chord_id) come from those views and stay valid until the entity is deleted. \
Every mutating result carries a `revision` counter that the app bumps exactly once per \
mutating call, however many internal edits the call fans out into; if it jumps by more than \
your own calls, the user edited concurrently — re-read before continuing. One edit_undo takes \
back one whole call.\n\
\n\
Long-running operations (project_*, vocal_render, render_mixdown) run as jobs: \
the tool waits a bounded time and returns the final job status; if it reports still-running, \
poll job_status or block with job_wait using the returned job_id.\n\
\n\
You cannot hear anything, so verify every edit by reading it back — that is the only feedback \
loop available and it is cheap: song_notes for what a clip actually contains, song_sections for \
where sections landed, song_vocal for the singing pre-flight, track_plugin_params for a sound. \
Mutating tools that create something return its id (clip_id, track_id, section_id, chord_id), so \
use that instead of a follow-up search. Audio is NOT a measuring instrument here: render_mixdown \
always bounces the whole song, and offset 0 in the file is the earliest clip, not bar 1 — bounce \
for the user to listen to, never to work out where something sits.\n\
\n\
If a tool reports `unsupported`, or behaves like an older build, call control_hello: its \
`capabilities` list is the authoritative set of control methods the RUNNING app implements, and \
a tool whose method is missing there cannot be made to work by re-phrasing the call.\n\
\n\
Destructive operations (track_delete, section_delete, project_new/project_open with unsaved \
changes, overwriting files) are refused with a summary of what would be lost until you pass \
confirm: true (overwrite: true for render targets). Bars and beats are 1-based; clip- and \
section-relative beats are 0-based; pitch 60 = C4. A timeline {bar, beat} position — every \
reported playhead/position, and every seek, loop, clip_* and meter range you send — counts \
beat in the time signature's beat unit (an eighth in 6/8, a half in 2/2), so a reported \
position can be sent straight back; a beat past the bar's last one is refused. Clip- and \
section-relative *_beat / *_beats fields are quarter notes whatever the meter (a 6/8 bar is \
3.0 of them).\n\
\n\
Automation is a track's, bus's or master's volume/pan/mute, or a plugin parameter, changing \
over the song — one lane per target. Values are its real units unless you pass normalized. \
Read lanes with automation_lanes (each is also summarized, without points, in song_tracks and \
master_summary); write a whole lane with automation_set_lane, single points with \
automation_add_points, and fades, rides, filter sweeps or LFO-like moves with automation_shape; \
remove with automation_delete_points / automation_remove_lane, and switch a lane on or off with \
automation_set_enabled. transport_set_tempo keeps a lane at its bar; \
global_* tempo and meter events keep its sample position instead, so its bar moves. You \
cannot hear automation: verify with automation_lanes and measure with meter_measure over the \
range.";

/// The MCP server handler: a thin, stateless translation layer over the
/// control socket. Cloneable (the client is shared via [`Arc`]).
#[derive(Clone)]
pub struct ResonanceMcp {
    pub(crate) client: Arc<ControlClient>,
}

impl ResonanceMcp {
    /// A server talking to the app through `client`.
    pub fn new(client: Arc<ControlClient>) -> Self {
        Self { client }
    }

    /// Every tool this server exposes: the per-namespace routers from
    /// [`crate::tools`], summed.
    pub fn combined_router() -> ToolRouter<Self> {
        let mut router = Self::router_control()
            + Self::router_song()
            + Self::router_project()
            + Self::router_transport()
            + Self::router_global()
            + Self::router_trackmix()
            + Self::router_external()
            + Self::router_master()
            + Self::router_bus()
            + Self::router_edit()
            + Self::router_arrange()
            + Self::router_compose()
            + Self::router_vocal()
            + Self::router_render()
            + Self::router_meter()
            + Self::router_automation()
            + Self::router_amp_models()
            + Self::router_drum_kits()
            + Self::router_presets()
            + Self::router_clip();
        normalize_schemas(&mut router);
        router
    }

    /// Call `method` and report the raw result as pretty JSON text.
    /// Failures become tool execution errors, never protocol errors.
    pub async fn invoke<P: Serialize>(
        &self,
        method: &'static str,
        params: &P,
    ) -> Result<CallToolResult, McpError> {
        match self.client.call(method, params).await {
            Ok(value) => Ok(CallToolResult::success(vec![ContentBlock::text(
                pretty(&value),
            )])),
            Err(error) => Ok(CallToolResult::error(vec![ContentBlock::text(
                error.actionable_message(),
            )])),
        }
    }

    /// Call `method` and return the result as structured content (the
    /// tool declares the matching `output_schema`).
    pub async fn invoke_structured<P: Serialize>(
        &self,
        method: &'static str,
        params: &P,
    ) -> Result<CallToolResult, McpError> {
        match self.client.call(method, params).await {
            Ok(value) => Ok(CallToolResult::structured(value)),
            Err(error) => Ok(CallToolResult::error(vec![ContentBlock::text(
                error.actionable_message(),
            )])),
        }
    }

    /// Call a job-launching `method`, then block up to `wait_ms` on
    /// `job.wait` and return the final [`JobStatus`] as structured
    /// content. A job that fails becomes a tool error; a job still
    /// running after the wait is reported as such (not an error) with
    /// polling guidance.
    pub async fn invoke_job<P: Serialize>(
        &self,
        method: &'static str,
        params: &P,
        wait_ms: u64,
    ) -> Result<CallToolResult, McpError> {
        let started: JobStarted = match self.client.call_typed(method, params).await {
            Ok(started) => started,
            Err(error) => {
                return Ok(CallToolResult::error(vec![ContentBlock::text(
                    error.actionable_message(),
                )]))
            }
        };
        match self
            .client
            .wait_job(started.job_id, Duration::from_millis(wait_ms))
            .await
        {
            Ok(status) => Ok(job_status_result(status, wait_ms)),
            // The job was started; a failed wait (e.g. dropped socket)
            // should still hand the model the job id to poll with.
            Err(error) => Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "the operation was started as job {} but waiting for it failed: {}. \
                 Poll job_status with job_id {} to track it.",
                started.job_id,
                error.actionable_message(),
                started.job_id,
            ))])),
        }
    }

    /// `job.status` / `job.wait` passthrough with terminal-state mapping.
    pub async fn job_query(
        &self,
        method: &'static str,
        job_id: JobId,
        params: &impl Serialize,
    ) -> Result<CallToolResult, McpError> {
        let status = self.client.call_typed::<_, JobStatus>(method, params).await;
        Ok(job_query_result(job_id, status))
    }

    /// Backs the `job_wait` tool: [`ControlClient::wait_job`] (sliced, so the
    /// shared connection stays usable while it blocks — CTL-07), with the
    /// same terminal-state mapping as [`Self::job_query`]. An omitted
    /// `timeout_ms` waits as long as the app would: 10 minutes.
    pub async fn wait_for_job(
        &self,
        job_id: JobId,
        timeout_ms: Option<u64>,
    ) -> Result<CallToolResult, McpError> {
        let timeout = timeout_ms.map_or(crate::client::MAX_JOB_WAIT, Duration::from_millis);
        let status = self.client.wait_job(job_id, timeout).await;
        Ok(job_query_result(job_id, status))
    }
}

/// A `job.status` / `job.wait` outcome as a tool result: a failed job is
/// a tool error, anything else is the structured status.
fn job_query_result(
    job_id: JobId,
    status: Result<JobStatus, crate::client::CallError>,
) -> CallToolResult {
    match status {
        Ok(status) if status.state == JobState::Error => job_failure(&status),
        Ok(status) => CallToolResult::structured(status_value(status)),
        Err(error) => CallToolResult::error(vec![ContentBlock::text(format!(
            "could not query job {job_id}: {}",
            error.actionable_message()
        ))]),
    }
}

/// Render a terminal or still-running job status for the model.
fn job_status_result(status: JobStatus, waited_ms: u64) -> CallToolResult {
    match status.state {
        JobState::Error => job_failure(&status),
        JobState::Done => CallToolResult::structured(status_value(status)),
        JobState::Pending | JobState::Running => {
            let job_id = status.job_id;
            let mut result = CallToolResult::structured(status_value(status));
            result.content.push(ContentBlock::text(format!(
                "The operation is still running after {:.1}s. Poll job_status \
                 {{\"job_id\": {job_id}}} or block with job_wait to get the final result.",
                waited_ms as f64 / 1000.0,
            )));
            result
        }
    }
}

/// A job that reached `state: error` — a tool error with the app's
/// message and, when the app classified the failure (ARCH-05 / epic C,
/// C-2), its machine-readable `kind` (e.g. `not_found`, `busy`).
fn job_failure(status: &JobStatus) -> CallToolResult {
    let text = match &status.error {
        Some(error) => match error.kind {
            Some(kind) => format!("job {} failed [{kind}]: {}", status.job_id, error.message),
            None => format!("job {} failed: {}", status.job_id, error.message),
        },
        None => format!("job {} failed: no error detail reported", status.job_id),
    };
    CallToolResult::error(vec![ContentBlock::text(text)])
}

fn status_value(status: JobStatus) -> Value {
    serde_json::to_value(status).unwrap_or(Value::Null)
}

/// Pretty-print a result value for the text channel.
pub(crate) fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

/// Rust integer/float `format` values that `schemars` emits but that are
/// not standard JSON Schema (its core defines no numeric formats). Strict
/// MCP clients — Claude Code / the Anthropic API — warn about and drop
/// them ("unknown format \"uint64\" ignored ..."). `type` + `minimum`
/// already constrain the values, so removing the format is lossless.
const NONSTANDARD_NUMERIC_FORMATS: &[&str] = &[
    "uint", "uint8", "uint16", "uint32", "uint64", "uint128", "int", "int8", "int16", "int32",
    "int64", "int128", "float", "double",
];

/// Normalize every tool's input and output schema so the published
/// surface is standard-compliant and warning-free in strict clients:
/// strip the non-standard numeric `format` annotations schemars adds.
/// The `ToolRouter`/`Tool` fields are public, so we rewrite in place.
fn normalize_schemas(router: &mut ToolRouter<ResonanceMcp>) {
    for route in router.map.values_mut() {
        let tool = &mut route.attr;
        let mut input = (*tool.input_schema).clone();
        inline_defs(&mut input);
        strip_nonstandard_formats(&mut input);
        // Say what `check_arguments` enforces: the typed params already
        // publish `additionalProperties: false`; a tool without params
        // gets it here.
        if input.contains_key("properties") && !input.contains_key("additionalProperties") {
            input.insert("additionalProperties".into(), Value::Bool(false));
        }
        tool.input_schema = Arc::new(input);
        if let Some(output) = tool.output_schema.take() {
            let mut out = (*output).clone();
            inline_defs(&mut out);
            strip_nonstandard_formats(&mut out);
            tool.output_schema = Some(Arc::new(out));
        }
    }
}

/// Replace every local `{"$ref": "#/$defs/X"}` with the body of `X` and
/// drop `$defs`, so each tool schema is self-contained.
///
/// Some MCP clients (the Claude desktop bridge among them) forward a
/// tool's schema without its `$defs`. A parameter typed only by a
/// dangling `$ref` then has no known type, and the client sends its
/// value as a JSON string — `"42"` instead of `42` — which serde
/// rejects for every id newtype ("invalid type: string, expected u64").
/// Inlining removes the dependency on the client resolving refs.
/// Sibling keys next to a `$ref` (e.g. `description`) are kept. A ref
/// that would recurse into itself is left as-is.
fn inline_defs(map: &mut serde_json::Map<String, Value>) {
    let Some(Value::Object(defs)) = map.remove("$defs") else {
        return;
    };
    let mut stack = Vec::new();
    for value in map.values_mut() {
        inline_refs_in_value(value, &defs, &mut stack);
    }
    if contains_ref(&Value::Object(map.clone())) {
        // A recursive type kept a `$ref`: restore `$defs` so it resolves.
        map.insert("$defs".into(), Value::Object(defs));
    }
}

fn inline_refs_in_value(
    value: &mut Value,
    defs: &serde_json::Map<String, Value>,
    stack: &mut Vec<String>,
) {
    match value {
        Value::Object(obj) => {
            let target = obj
                .get("$ref")
                .and_then(Value::as_str)
                .and_then(|r| r.strip_prefix("#/$defs/"))
                .map(str::to_owned);
            if let Some(name) = target {
                if let (Some(Value::Object(def)), false) = (defs.get(&name), stack.contains(&name)) {
                    obj.remove("$ref");
                    let mut body = def.clone();
                    // Keys written next to the `$ref` win over the def's own.
                    for (k, v) in std::mem::take(obj) {
                        body.insert(k, v);
                    }
                    *obj = body;
                    stack.push(name);
                    for child in obj.values_mut() {
                        inline_refs_in_value(child, defs, stack);
                    }
                    stack.pop();
                    return;
                }
            }
            for child in obj.values_mut() {
                inline_refs_in_value(child, defs, stack);
            }
        }
        Value::Array(items) => items
            .iter_mut()
            .for_each(|v| inline_refs_in_value(v, defs, stack)),
        _ => {}
    }
}

fn contains_ref(value: &Value) -> bool {
    match value {
        Value::Object(obj) => obj.contains_key("$ref") || obj.values().any(contains_ref),
        Value::Array(items) => items.iter().any(contains_ref),
        _ => false,
    }
}

/// Recursively drop `format` keys carrying a [`NONSTANDARD_NUMERIC_FORMATS`]
/// value, everywhere in a JSON Schema object (`properties`, `items`,
/// `$defs`, nested schemas, ...).
fn strip_nonstandard_formats(map: &mut serde_json::Map<String, Value>) {
    if let Some(Value::String(fmt)) = map.get("format") {
        if NONSTANDARD_NUMERIC_FORMATS.contains(&fmt.as_str()) {
            map.remove("format");
        }
    }
    for value in map.values_mut() {
        strip_formats_in_value(value);
    }
}

fn strip_formats_in_value(value: &mut Value) {
    match value {
        Value::Object(child) => strip_nonstandard_formats(child),
        Value::Array(items) => items.iter_mut().for_each(strip_formats_in_value),
        _ => {}
    }
}

/// Refuse arguments `tool` does not declare (code review ARCH2-06).
///
/// A tool's published input schema lists every key it reads under
/// `properties`; the typed params behind it are
/// `#[serde(deny_unknown_fields)]`, but a tool without params never
/// parses its arguments at all, and rmcp reports a params parse failure
/// as a protocol error the model may never see. So the check runs here,
/// before dispatch, for every tool: a key not in `properties` (unless the
/// schema opens `additionalProperties`) is a tool error naming the key
/// and the accepted ones, never a success that ignored it.
pub fn check_arguments(
    tool: &rmcp::model::Tool,
    arguments: Option<&serde_json::Map<String, Value>>,
) -> Result<(), CallToolResult> {
    let Some(arguments) = arguments else {
        return Ok(());
    };
    let schema = &tool.input_schema;
    let Some(Value::Object(properties)) = schema.get("properties") else {
        return Ok(());
    };
    if schema
        .get("additionalProperties")
        .is_some_and(|open| open != &Value::Bool(false))
    {
        return Ok(());
    }
    let unknown: Vec<&str> = arguments
        .keys()
        .filter(|k| !properties.contains_key(k.as_str()))
        .map(String::as_str)
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    let quoted = |keys: &mut dyn Iterator<Item = &str>| {
        keys.map(|k| format!("`{k}`")).collect::<Vec<_>>().join(", ")
    };
    let accepted = if properties.is_empty() {
        "it takes no arguments".to_string()
    } else {
        format!("it accepts {}", quoted(&mut properties.keys().map(String::as_str)))
    };
    let noun = if unknown.len() == 1 { "argument" } else { "arguments" };
    Err(CallToolResult::error(vec![ContentBlock::text(format!(
        "{} does not take the {noun} {}; {accepted}. Nothing was done: fix the call and retry.",
        tool.name,
        quoted(&mut unknown.into_iter()),
    ))]))
}

#[rmcp::tool_handler(router = Self::combined_router())]
impl ServerHandler for ResonanceMcp {
    /// Dispatch a tool call after [`check_arguments`]. A params parse
    /// failure (a wrong type, an unknown field nested in an object) comes
    /// back from rmcp as an `invalid_params` protocol error, which clients
    /// tend to render opaquely; it is reported as a tool error instead so
    /// the model reads serde's message and can correct the call.
    async fn call_tool(
        &self,
        request: rmcp::model::CallToolRequestParams,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<rmcp::model::CallToolResponse, McpError> {
        let router = Self::combined_router();
        if let Some(tool) = router.get(&request.name) {
            if let Err(refused) = check_arguments(tool, request.arguments.as_ref()) {
                return Ok(rmcp::model::CallToolResponse::Complete(refused));
            }
        }
        let name = request.name.clone();
        let tcc = rmcp::handler::server::tool::ToolCallContext::new(self, request, context);
        match router.call(tcc).await {
            Err(e) if e.code == rmcp::model::ErrorCode::INVALID_PARAMS => {
                Ok(rmcp::model::CallToolResponse::Complete(CallToolResult::error(vec![
                    ContentBlock::text(format!(
                        "{name}: {}. Nothing was done: fix the call and retry.",
                        e.message
                    )),
                ])))
            }
            other => other,
        }
    }

    fn get_info(&self) -> ServerInfo {
        // Report OUR crate name/version, not rmcp's: `from_build_env`
        // reads env! at the rmcp crate's compile site.
        InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new(env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"))
                    .with_title("resonance"),
            )
            .with_instructions(INSTRUCTIONS)
    }
}
