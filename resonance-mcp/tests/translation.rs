//! Translation-layer tests: drive `ResonanceMcp` tools against a fake
//! control-socket server (no GUI). Cover the request mapping, structured
//! results, job waiting, error mapping, handshake version check, and
//! reconnect-after-drop behaviour.

mod common;

use common::{fail, ok, FakeApp};
use resonance_control::ids::JobId;
use resonance_control::job::{JobError, JobState, JobStatus};
use resonance_control::{ErrorKind, RpcError, PROTOCOL_VERSION};
use resonance_mcp::{ControlClient, ResonanceMcp};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Build an MCP server pointed at `app`.
fn server(app: &FakeApp) -> ResonanceMcp {
    ResonanceMcp::new(ControlClient::new(app.path()))
}

/// The structured JSON a `CallToolResult` carries (panics if unstructured).
fn structured(result: &rmcp::model::CallToolResult) -> Value {
    result
        .structured_content
        .clone()
        .expect("expected structured content")
}

fn is_error(result: &rmcp::model::CallToolResult) -> bool {
    result.is_error == Some(true)
}

/// Concatenated text of a result's content blocks.
fn text(result: &rmcp::model::CallToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test]
async fn song_summary_returns_structured_result() {
    let payload = json!({
        "tempo_bpm": 128.0,
        "time_signature": {"numerator": 4, "denominator": 4},
        "sample_rate": 48000,
        "length_bars": 16.0,
        "length_samples": 1_000_000,
        "transport": "stopped",
        "playhead": {"bar": 1, "beat": 1.0, "sample": 0},
        "sections": [],
        "tracks": [],
        "revision": 3
    });
    let expected = payload.clone();
    let app = FakeApp::spawn(PROTOCOL_VERSION, move |req| ok(req, &expected));
    let mcp = server(&app);

    let result = mcp.invoke_structured("song.summary", &()).await.unwrap();
    assert!(!is_error(&result));
    assert_eq!(structured(&result), payload);
    assert_eq!(app.seen_methods(), vec!["song.summary"]);
    // One handshake for the connection, before the real call.
    assert_eq!(app.handshakes.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn transport_seek_forwards_params() {
    let seen: Arc<std::sync::Mutex<Option<Value>>> = Arc::default();
    let captured = Arc::clone(&seen);
    let app = FakeApp::spawn(PROTOCOL_VERSION, move |req| {
        *captured.lock().unwrap() = req.params.clone();
        ok(
            req,
            &json!({
                "state": "playing",
                "playhead": {"bar": 17, "beat": 1.0, "sample": 480000},
                "looping": false,
                "revision": 4
            }),
        )
    });
    let mcp = server(&app);

    let params = json!({"bar": 17, "beat": 1.0});
    let result = mcp.invoke_structured("transport.seek", &params).await.unwrap();
    assert!(!is_error(&result));
    assert_eq!(
        seen.lock().unwrap().clone(),
        Some(json!({"bar": 17, "beat": 1.0}))
    );
}

#[tokio::test]
async fn not_found_becomes_tool_error_with_hint() {
    let app = FakeApp::spawn(PROTOCOL_VERSION, |req| {
        fail(req, RpcError::not_found("No track with id 7"))
    });
    let mcp = server(&app);

    let result = mcp.invoke("track.rename", &json!({"track_id": 7, "name": "x"}))
        .await
        .unwrap();
    assert!(is_error(&result));
    let message = text(&result);
    assert!(message.contains("No track with id 7"), "{message}");
    // The self-correction hint points at the song_* views.
    assert!(message.contains("song_"), "{message}");
}

#[tokio::test]
async fn needs_confirmation_error_suggests_confirm_flag() {
    let app = FakeApp::spawn(PROTOCOL_VERSION, |req| {
        fail(
            req,
            RpcError::needs_confirmation("Deleting track 2 removes 3 clips"),
        )
    });
    let mcp = server(&app);

    let result = mcp.invoke("track.delete", &json!({"track_id": 2}))
        .await
        .unwrap();
    assert!(is_error(&result));
    let message = text(&result);
    assert!(message.contains("confirm"), "{message}");
    assert_eq!(RpcError::needs_confirmation("x").kind(), ErrorKind::NeedsConfirmation);
}

#[tokio::test]
async fn unsupported_method_passes_through_as_tool_error() {
    // Simulates the app's clean `unsupported` for namespaces not yet
    // landed (#1150-#1157): the MCP layer must surface, not crash.
    let app = FakeApp::spawn(PROTOCOL_VERSION, |req| {
        fail(req, RpcError::unsupported("generate.part not implemented"))
    });
    let mcp = server(&app);

    let result = mcp
        .invoke_structured(
            "generate.part",
            &json!({"section_id": 1, "track_id": 1, "role": "lead"}),
        )
        .await
        .unwrap();
    assert!(is_error(&result));
    assert!(text(&result).contains("not implemented"), "{}", text(&result));
}

#[tokio::test]
async fn app_not_running_is_actionable_tool_error() {
    // No fake app: connect must fail with a "start resonance" message,
    // and the server must NOT crash.
    let missing = std::env::temp_dir().join("resonance-mcp-test-nonexistent/control.sock");
    let mcp = ResonanceMcp::new(ControlClient::new(missing));

    let result = mcp.invoke("transport.play", &()).await.unwrap();
    assert!(is_error(&result));
    let message = text(&result);
    assert!(message.contains("not running"), "{message}");
    assert!(message.contains("Start the resonance app"), "{message}");
}

#[tokio::test]
async fn incompatible_protocol_version_is_handshake_error() {
    let app = FakeApp::spawn(PROTOCOL_VERSION + 1, |req| ok(req, &json!({"revision": 1})));
    let mcp = server(&app);

    let result = mcp.invoke("transport.play", &()).await.unwrap();
    assert!(is_error(&result));
    let message = text(&result);
    assert!(message.contains("protocol"), "{message}");
    // The real request never ran — only the handshake was attempted.
    assert!(app.seen_methods().is_empty(), "{:?}", app.seen_methods());
}

#[tokio::test]
async fn job_tool_waits_and_returns_terminal_status() {
    let job_id = JobId(9);
    let app = FakeApp::spawn(PROTOCOL_VERSION, move |req| match req.method.as_str() {
        "render.mixdown" => ok(req, &json!({"job_id": 9})),
        "job.wait" => {
            let status = JobStatus {
                job_id,
                state: JobState::Done,
                progress: Some(1.0),
                result: Some(json!({"path": "/tmp/out.wav", "duration_s": 12.5})),
                error: None,
            };
            ok(req, &status)
        }
        other => panic!("unexpected method {other}"),
    });
    let mcp = server(&app);

    let result = mcp
        .invoke_job("render.mixdown", &json!({"path": "/tmp/out.wav"}), 1000)
        .await
        .unwrap();
    assert!(!is_error(&result));
    let status = structured(&result);
    assert_eq!(status["state"], "done");
    assert_eq!(status["result"]["path"], "/tmp/out.wav");
    assert_eq!(app.seen_methods(), vec!["render.mixdown", "job.wait"]);
}

#[tokio::test]
async fn job_tool_maps_failed_job_to_tool_error() {
    let app = FakeApp::spawn(PROTOCOL_VERSION, move |req| match req.method.as_str() {
        "vocal.render" => ok(req, &json!({"job_id": 3})),
        "job.wait" => ok(
            req,
            &JobStatus {
                job_id: JobId(3),
                state: JobState::Error,
                progress: None,
                result: None,
                // `kind` (ARCH-05 / epic C, C-2) surfaces in the tool's
                // text alongside the message.
                error: Some(JobError::new("voicebank not found", Some(ErrorKind::NotFound))),
            },
        ),
        other => panic!("unexpected method {other}"),
    });
    let mcp = server(&app);

    let result = mcp.invoke_job("vocal.render", &json!({}), 1000).await.unwrap();
    assert!(is_error(&result));
    let text = text(&result);
    assert!(text.contains("voicebank not found"), "{text}");
    assert!(text.contains("not_found"), "{text}");
}

#[tokio::test]
async fn reconnects_after_dropped_socket() {
    // First call sees a connection that drops (handler returns None
    // after recording the request); the second call must reconnect
    // (new handshake) and succeed.
    let calls = Arc::new(AtomicU64::new(0));
    let seen = Arc::clone(&calls);
    let app = FakeApp::spawn(PROTOCOL_VERSION, move |req| {
        let n = seen.fetch_add(1, Ordering::Relaxed);
        if n == 0 {
            None // drop the connection mid-call
        } else {
            ok(req, &json!({"revision": 2}))
        }
    });
    let mcp = server(&app);

    let first = mcp.invoke("transport.play", &()).await.unwrap();
    assert!(is_error(&first), "first call should surface the drop");

    let second = mcp.invoke("transport.play", &()).await.unwrap();
    assert!(!is_error(&second), "second call should reconnect: {}", text(&second));
    // Two handshakes = the client reconnected for the retry.
    assert_eq!(app.handshakes.load(Ordering::Relaxed), 2);
}

#[tokio::test]
async fn combined_router_exposes_every_control_method() {
    // One MCP tool per control method — control.hello and job.* included
    // — so the published surface can't silently drop a protocol method.
    // The exception is a deprecated alias: the app still answers it, so
    // it is in `capabilities`, but it deliberately gets no tool, because
    // an agent must see exactly one spelling per operation.
    let router = ResonanceMcp::combined_router();
    let tools: std::collections::BTreeSet<String> = router
        .list_all()
        .into_iter()
        .map(|t| t.name.to_string())
        .collect();
    let aliases = resonance_control::methods::deprecated_aliases();
    // `a.b` -> `a_b` is the whole naming rule.
    let expected: std::collections::BTreeSet<String> = resonance_control::methods::capabilities()
        .into_iter()
        .filter(|m| !aliases.contains(m))
        .map(|m| m.replacen('.', "_", 1))
        .collect();
    for alias in &aliases {
        assert!(
            !tools.contains(&alias.replacen('.', "_", 1)),
            "{alias} is deprecated and must not be published as an MCP tool"
        );
    }
    assert_eq!(
        tools, expected,
        "published tool names diverged from the control method list"
    );
}

/// CTL-07: a long job wait must not hold the one shared connection for
/// its whole duration. The fake blocks each `job.wait` for its requested
/// `timeout_ms` (as the app does) and the job never finishes; a
/// concurrent `song.summary` must still come back promptly instead of
/// queueing behind the wait.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_long_job_wait_does_not_stall_concurrent_calls() {
    let app = FakeApp::spawn(PROTOCOL_VERSION, move |req| match req.method.as_str() {
        "render.mixdown" => ok(req, &json!({"job_id": 5})),
        "job.wait" => {
            let ms = req
                .params
                .as_ref()
                .and_then(|p| p.get("timeout_ms"))
                .and_then(Value::as_u64)
                .unwrap_or(600_000)
                .min(10_000);
            std::thread::sleep(std::time::Duration::from_millis(ms));
            ok(
                req,
                &JobStatus {
                    job_id: JobId(5),
                    state: JobState::Running,
                    progress: Some(0.5),
                    result: None,
                    error: None,
                },
            )
        }
        "song.summary" => ok(req, &json!({"revision": 1})),
        other => panic!("unexpected method {other}"),
    });
    let mcp = server(&app);

    let waiter = {
        let mcp = mcp.clone();
        tokio::spawn(async move {
            mcp.invoke_job("render.mixdown", &json!({"path": "/tmp/out.wav"}), 4_000)
                .await
                .unwrap()
        })
    };
    // Let the wait get onto the connection first.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    let start = std::time::Instant::now();
    let summary = mcp.invoke("song.summary", &json!({})).await.unwrap();
    let elapsed = start.elapsed();
    assert!(!is_error(&summary), "{}", text(&summary));
    assert!(
        elapsed < std::time::Duration::from_millis(1_500),
        "song.summary queued behind the job wait for {elapsed:?}"
    );

    // The wait itself still honours its full bound and reports the job
    // as still running.
    let waited = waiter.await.unwrap();
    assert!(!is_error(&waited), "{}", text(&waited));
    assert_eq!(structured(&waited)["state"], "running");
}

/// The meter tools publish the opt-in `detail` option (warmth-width-depth.md
/// §7.1) in their input schema, with every detail name, and say what it
/// adds in their description.
#[test]
fn meter_tools_publish_the_detail_option() {
    for name in ["meter_measure", "meter_stems"] {
        let tool = ResonanceMcp::combined_router()
            .list_all()
            .into_iter()
            .find(|t| t.name == name)
            .unwrap_or_else(|| panic!("{name} is published"));
        let schema = serde_json::to_string(&tool.input_schema).unwrap();
        assert!(schema.contains("\"detail\""), "{name} schema lacks detail: {schema}");
        let description = tool.description.as_deref().unwrap_or_default();
        assert!(description.contains("detail"), "{name} description never names detail");
        for detail in ["spectrum", "stereo", "dynamics"] {
            assert!(
                schema.contains(&format!("\"{detail}\"")),
                "{name} schema lacks {detail}: {schema}"
            );
        }
        assert!(description.contains("spectrum"), "{name} description never names spectrum");
    }
}

/// `master_assist` publishes both target modes, the five genres and the
/// pool-asset reference in its input schema, and says in its description
/// that nothing is applied.
#[test]
fn master_assist_publishes_both_modes() {
    let tool = ResonanceMcp::combined_router()
        .list_all()
        .into_iter()
        .find(|t| t.name == "master_assist")
        .expect("master_assist is published");
    let schema = serde_json::to_string(&tool.input_schema).unwrap();
    for word in ["\"mode\"", "\"genre\"", "\"reference\"", "\"pool_asset_id\"", "\"range\""] {
        assert!(schema.contains(word), "master_assist schema lacks {word}: {schema}");
    }
    for genre in ["rock", "indie", "acoustic", "jazz", "pop"] {
        assert!(schema.contains(&format!("\"{genre}\"")), "schema lacks {genre}");
    }
    let description = tool.description.as_deref().unwrap_or_default();
    assert!(description.contains("NOTHING IS APPLIED"), "{description}");
    assert!(description.contains("master_set_plugin_param"), "{description}");
}

/// The assistant's suggestion round-trips: typed params go out as the
/// wire shape, and the job's result comes back as an `AssistResult` whose
/// param writes an agent can replay through `master.set_plugin_param`.
#[tokio::test]
async fn master_assist_round_trips_params_and_result() {
    use resonance_control::methods::master::{AssistGenre, AssistMode, AssistParams, AssistResult};
    let seen: Arc<std::sync::Mutex<Option<Value>>> = Arc::default();
    let captured = Arc::clone(&seen);
    let result_payload = json!({
        "target": {"mode": "genre", "genre": "rock", "label": "Rock", "target_lufs": -11.0},
        "plugin_id": "com.resonance.mastering",
        "master_slot": 0,
        "measured": {
            "lufs_integrated": -20.5, "true_peak_db": -3.0, "crest_db": 14.2,
            "correlation": 0.81, "measured_seconds": 30.0
        },
        "suggestions": [
            {"stage": "tonal_low_shelf", "rationale": ["Low shelf: -2.0 dB"],
             "params": [{"key": "tone_b0_on", "value": 1.0}, {"key": "tone_b0_gain", "value": -2.0}]},
            {"stage": "diagnostic", "rationale": ["Input integrated loudness: -20.5 LUFS"],
             "params": []}
        ],
        "deviations": [
            {"hz": 50.0, "lo_db": 10.0, "hi_db": 19.0, "measured_db": 21.0, "deviation_db": 2.0}
        ]
    });
    let payload = result_payload.clone();
    let app = FakeApp::spawn(PROTOCOL_VERSION, move |req| match req.method.as_str() {
        "master.assist" => {
            *captured.lock().unwrap() = req.params.clone();
            ok(req, &json!({"job_id": 11}))
        }
        "job.wait" => ok(
            req,
            &JobStatus {
                job_id: JobId(11),
                state: JobState::Done,
                progress: Some(1.0),
                result: Some(payload.clone()),
                error: None,
            },
        ),
        other => panic!("unexpected method {other}"),
    });
    let mcp = server(&app);
    let params = AssistParams {
        mode: AssistMode::Genre,
        genre: Some(AssistGenre::Rock),
        pool_asset_id: None,
        range: None,
    };
    let result = mcp.invoke_job("master.assist", &params, 1000).await.unwrap();
    assert!(!is_error(&result), "{}", text(&result));
    assert_eq!(
        seen.lock().unwrap().clone(),
        Some(json!({"mode": "genre", "genre": "rock"}))
    );
    let status = structured(&result);
    let assist: AssistResult =
        serde_json::from_value(status["result"].clone()).expect("an AssistResult");
    assert_eq!(assist.master_slot, Some(0));
    let writes: Vec<(&str, f64)> = assist
        .suggestions
        .iter()
        .flat_map(|s| s.params.iter().map(|p| (p.key.as_str(), p.value)))
        .collect();
    assert_eq!(writes, vec![("tone_b0_on", 1.0), ("tone_b0_gain", -2.0)]);
    assert_eq!(serde_json::to_value(&assist).unwrap(), result_payload);
}
