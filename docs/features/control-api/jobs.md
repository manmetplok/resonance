# Control jobs (`job.status` / `job.wait`)

> Verified against master @ `2b28debd` + vocal phase 4 on 2026-10-07.
> Spec history: ba doc #265, todo #1149 (not in the repo).

Some control calls take longer than one turn of the update loop: saving
or opening a project, rendering vocals, bouncing a mix, measuring. Those
calls answer at once with a `job_id`. The client then polls `job.status`
or blocks on `job.wait` until the job is `done` (with a result) or
`error` (with a reason). The MCP server hides most of this: its job tools
wait a bounded time and hand back the final status.

## Behaviour

### Which calls are jobs

| Method | Completes on | MCP wait budget |
|---|---|---|
| `project.new`, `project.open` | engine confirms the clear and the project replays (`engine_events::project_io::all_cleared`); fails on a load/template error | 30 s |
| `project.save`, `project.save_as` | `ProjectIoMessage::ProjectSaved` (manual saves only; autosaves never resolve a job) | 30 s |
| `vocal.render` | the SVS audio for **every** lane in the batch has landed (one job, N lanes) | 120 s |
| `vocal.voicebank_install` | the install thread finished **and** the tick rescanned the registry, so the bank is listed when the job resolves; fails `not_found` for an id the index does not list, without a kind for a failed index fetch or download, with the "upstream file changed" message for a hash mismatch ([voicebank library](../compose/voicebank-library.md)) | 120 s |
| `render.mixdown` | export completion for that output path | 300 s |
| `pool.import`, `clip.place` | the **last** asset id of the batch has reported (`clip.place` of an already-pooled file is `done` before the reply leaves) | 120 s |
| `clip.detect_tempo` | `AudioEvent::ClipTempoDetected` for that clip; fails if no tempo is found or the clip is deleted | 60 s |
| `meter.measure`, `meter.stems`, `meter.snapshot`, `meter.compare` | `AudioEvent::MixMeasured` carrying the job id as its `measure_id`; fails on `MixMeasureError` | 300 s |
| `meter.probe` | `AudioEvent::ChainProbed` carrying the job id; fails on `ChainProbeError` | 300 s |
| `master.assist` | the master measurement (and the reference's, in reference mode) | 300 s |

`external.detect_latency` and `external.bounce` are also asynchronous but
are not jobs: they acknowledge with a revision and the client reads the
outcome back with `external.status`.

### States and results

- `pending` → `running` → `done` | `error`. `progress` (0.0–1.0) is set
  where the operation reports it; `done` sets it to 1.0.
- `JobStatus { job_id, state, progress?, result?, error? }`. `result` is
  the method's payload once `done` (e.g. `MixdownResult`); its schema is
  "any JSON" on purpose. `error` is `{message, kind?}`; `kind` is set
  when the failure is classified (e.g. a mixdown's `ExportErrorKind` is
  mapped by `export_kind_to_rpc`). An old bare-string `error` still
  parses.

### `job.wait`

- Blocks until the job is terminal or `timeout_ms` passes, then returns
  the current status either way. The app caps every wait at 600 s
  (`MAX_WAIT`); an omitted timeout means the cap.
- Served on the connection's reader thread, which blocks on the
  `JobBoard` condvar. The update loop never blocks. Later requests on
  that same connection queue behind the wait; other connections are not
  affected.
- If `job.wait` reaches the update loop instead (direct dispatch in
  tests), it answers at once with the current status.

### Retention and disconnects

- Job ids are global and increase for the app's lifetime. Any connection
  may query any job.
- At most 64 entries are kept (`MAX_RETAINED_JOBS`). When over, terminal
  jobs are evicted first, already-fetched ones before unfetched ones,
  oldest first. If every job is live the table grows temporarily.
- When a connection closes, its terminal jobs are dropped and its live
  jobs are orphaned (`owner = None`), not deleted. The operation keeps
  running, and a client that reconnects after a transport hiccup can keep
  polling the same id. Orphans resolve normally and age out by the same
  64-entry bound.

### Matching completions to jobs

Each job stores a `JobToken` when it starts. Completion hooks in the
existing message handlers resolve the **oldest** live job with a matching
token (completions arrive in dispatch order), never "the newest".
Measurements and probes need no guessing: the engine echoes the job id
(todo #1243), so a live meter read and an offline render measurement can
run side by side. Only offline measurements block each other
(`has_live_offline_measure`).

### On the MCP side

`ResonanceMcp::invoke_job` starts the job, then calls
`ControlClient::wait_job` with the tool's budget. `wait_job` issues a
series of short `job.wait` calls (250 ms each, `JOB_WAIT_SLICE`) rather
than one long one, so other tool calls on the shared connection are not
stalled behind a render (CTL-07). A job still running after the budget
comes back as a normal result plus "poll job_status or block with
job_wait"; a failed job is a tool error carrying `[kind]` when present.
The `job_wait` tool's default timeout is the 10-minute cap.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Wire types | `resonance-control/src/job.rs` | `STATUS`, `WAIT`, `JobState`, `JobStarted`, `JobStatus`, `JobError`, `WaitParams` |
| Ledger | `resonance-app/src/control_jobs.rs` | `JobBoard` (`start`, `complete_token`, `fail_token`, `complete_vocal_lane`, `tick_import_asset`, `live_measure`, `wait`, `on_disconnect`), `JobToken`, `MAX_RETAINED_JOBS`, `MAX_WAIT`, `prune` |
| Starting a job | `resonance-app/src/control_jobs.rs` | `Resonance::start_control_job` |
| Blocking wait | `resonance-app/src/control_socket.rs` | `job_wait_response` |
| `job.status` + wait fallback | `resonance-app/src/update/control/job.rs` | `try_handle`, `export_kind_to_rpc` |
| Job-starting handlers | `update/control/{project,vocal,render,meter,assist,clip}.rs`, `clip/warp.rs` | calls to `start_control_job` |
| MCP waiting | `resonance-mcp/src/server.rs`, `client.rs` | `invoke_job`, `wait_for_job`, `wait_job`, `JOB_WAIT_SLICE`, `MAX_JOB_WAIT` |

Invariant: a job must resolve through its token or id from a real
completion event. Never resolve a job by guessing which one an event
belongs to.

## Surfaces

- **Control API / MCP:** `job.status` → `mcp__resonance__job_status`,
  `job.wait` → `mcp__resonance__job_wait`. The job-starting methods are
  listed above; their tools wait internally.
- **Agent skills:** the mixing and mastering skills lean on the `meter_*`
  jobs ([agent-plugin.md](agent-plugin.md)).

## How it's tested

| What | Where | Run |
|---|---|---|
| Ledger, `job.status`/`job.wait`, token completion, blocking wait, retention | `resonance-app/tests/control/control_jobs.rs` | `./scripts/run-tests.py -p resonance-app` |
| Per-method job flows | `control_project.rs`, `control_render*.rs`, `control_vocal_render_all_lanes.rs`, `control_meter*.rs`, `control_master_assist.rs`, `control_clip_place.rs` | same |
| Full flow incl. a real engine bounce job | `resonance-app/tests/e2e_compose_via_control.rs` | `cargo test -p resonance-app --test e2e_compose_via_control` |
| MCP job waiting and error mapping | `resonance-mcp/tests/translation.rs` | `./scripts/run-tests.py -p resonance-mcp` |
| `job.wait` gets the long read deadline | `resonance-mcp/tests/client_timeout.rs::job_wait_rides_the_long_deadline` | same |

Not covered: cancelling a job (there is no cancel method); a live
socket client reconnecting and resuming an orphaned job end to end
(`on_disconnect` is tested at the ledger level).

## Decisions and deviations

- Jobs are resolved from the app's existing completion messages, not a
  parallel path, so a GUI-driven save and a control-driven one share
  their completion code. A completion with no job attached is normal.
- Orphaning instead of deleting live jobs on disconnect: the MCP client
  drops its connection on any transport failure and tells the model to
  poll the same job id, so deleting them turned recovery into
  `not_found` for work that was still running.
- The MCP client slices its waits (CTL-07). One long `job.wait` used to
  hold the client's single connection for the whole render.
- `JobStatus.result` publishes a hand-written "any JSON" object schema,
  because strict MCP clients reject the boolean `true` schema schemars
  emits for `serde_json::Value`.

## Known gaps

- No `job.cancel`.
- `render.stems` would be a job but is not built:
  [render-stems-control](../../intents/control-api/render-range-and-stems.md).
