//! Read-deadline behaviour of `ControlClient` against an app that is
//! alive but never answers: the call must fail with
//! [`CallError::Unresponsive`] within the bound instead of parking the
//! connection mutex forever, the timed-out connection must never be
//! reused (the next call reconnects on a fresh stream), and `job.wait`
//! must ride the long deadline while ordinary calls ride the short one.

use resonance_control::methods::control::{HelloParams, HelloResult};
use resonance_control::rpc::{Request, Response};
use resonance_control::{write_message, MessageReader, PROTOCOL_VERSION};
use resonance_mcp::client::CallTimeouts;
use resonance_mcp::{CallError, ControlClient};
use serde_json::json;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

static NEXT_SOCKET: AtomicU64 = AtomicU64::new(0);

/// A fake app that accepts connections and then goes silent. Accepted
/// streams are parked, never dropped, so the client sees an app that is
/// running-but-stuck rather than a hangup (EOF).
struct SilentApp {
    path: PathBuf,
    /// Number of connections accepted so far.
    accepted: Arc<AtomicU64>,
}

impl SilentApp {
    /// `answer_hello: false` goes silent from the first byte (the
    /// handshake itself times out); `true` completes `control.hello`
    /// first, so the deadline under test is the real method's.
    fn spawn(answer_hello: bool) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "resonance-mcp-timeout-{}-{}",
            std::process::id(),
            NEXT_SOCKET.fetch_add(1, Ordering::Relaxed),
        ));
        std::fs::create_dir_all(&dir).expect("create silent-app socket dir");
        let path = dir.join("control.sock");
        let listener = UnixListener::bind(&path).expect("bind silent-app socket");
        let accepted = Arc::new(AtomicU64::new(0));
        {
            let accepted = Arc::clone(&accepted);
            std::thread::spawn(move || {
                let mut parked: Vec<UnixStream> = Vec::new();
                for stream in listener.incoming() {
                    let Ok(stream) = stream else { break };
                    accepted.fetch_add(1, Ordering::SeqCst);
                    if answer_hello {
                        answer_hello_then_hang(&stream);
                    }
                    parked.push(stream);
                }
            });
        }
        Self { path, accepted }
    }
}

impl Drop for SilentApp {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Answer the connection's `control.hello`, then never reply again.
fn answer_hello_then_hang(stream: &UnixStream) {
    let Ok(read_half) = stream.try_clone() else { return };
    let mut reader = MessageReader::from_reader(read_half);
    let Ok(Some(request)) = reader.read_message::<Request>() else {
        return;
    };
    assert_eq!(request.method, resonance_control::methods::control::HELLO);
    let _params: HelloParams = request.params().expect("well-formed hello");
    let result = HelloResult {
        app_version: "silent-app".to_owned(),
        protocol_version: PROTOCOL_VERSION,
        capabilities: Vec::new(),
    };
    let Ok(mut writer) = stream.try_clone() else { return };
    let response = Response::success(request.id.clone(), &result).expect("hello serializes");
    let _ = write_message(&mut writer, &response);
}

/// Short deadlines so an unresponsive-app scenario resolves in
/// milliseconds; the generous write deadline keeps writes out of the
/// picture (requests are tiny and always fit the socket buffer).
fn timeouts(read_ms: u64, job_wait_read_ms: u64) -> CallTimeouts {
    CallTimeouts {
        read: Duration::from_millis(read_ms),
        job_wait_read: Duration::from_millis(job_wait_read_ms),
        write: Duration::from_secs(5),
    }
}

#[test]
fn unresponsive_app_times_out_with_a_typed_error() {
    let app = SilentApp::spawn(true);
    let client = ControlClient::with_timeouts(app.path.clone(), timeouts(300, 300));

    let start = Instant::now();
    let error = client.call_blocking("song.summary", None).unwrap_err();
    let elapsed = start.elapsed();

    match &error {
        CallError::Unresponsive { method, .. } => assert_eq!(method, "song.summary"),
        other => panic!("expected Unresponsive, got {other:?}"),
    }
    assert!(
        elapsed < Duration::from_secs(10),
        "timeout took {elapsed:?}, expected well under the test bound"
    );
    // The MCP layer surfaces `actionable_message` as the tool error.
    assert!(error.actionable_message().contains("unresponsive"));
}

#[test]
fn handshake_against_a_silent_app_also_times_out() {
    let app = SilentApp::spawn(false);
    let client = ControlClient::with_timeouts(app.path.clone(), timeouts(300, 300));

    let error = client.call_blocking("song.summary", None).unwrap_err();
    assert!(
        matches!(error, CallError::Unresponsive { .. }),
        "expected Unresponsive, got {error:?}"
    );
}

#[test]
fn timed_out_connection_is_dropped_and_the_next_call_reconnects() {
    let app = SilentApp::spawn(true);
    let client = ControlClient::with_timeouts(app.path.clone(), timeouts(200, 200));

    let first = client.call_blocking("song.summary", None).unwrap_err();
    assert!(matches!(first, CallError::Unresponsive { .. }));
    assert_eq!(app.accepted.load(Ordering::SeqCst), 1);

    // The poisoned stream is never reused: the second call arrives on a
    // fresh connection (and re-handshakes on it).
    let second = client.call_blocking("song.summary", None).unwrap_err();
    assert!(matches!(second, CallError::Unresponsive { .. }));
    assert_eq!(app.accepted.load(Ordering::SeqCst), 2);
}

#[test]
fn job_wait_rides_the_long_deadline() {
    let app = SilentApp::spawn(true);
    let client = ControlClient::with_timeouts(app.path.clone(), timeouts(200, 1500));

    let start = Instant::now();
    let error = client
        .call_blocking(resonance_control::job::WAIT, Some(json!({"job_id": 1})))
        .unwrap_err();
    let elapsed = start.elapsed();

    match &error {
        CallError::Unresponsive { method, timeout } => {
            assert_eq!(method, resonance_control::job::WAIT);
            assert_eq!(*timeout, Duration::from_millis(1500));
        }
        other => panic!("expected Unresponsive, got {other:?}"),
    }
    // Waited past the ordinary 200 ms deadline: `job.wait` got its own.
    assert!(
        elapsed >= Duration::from_millis(1200),
        "job.wait timed out after only {elapsed:?}"
    );
    assert!(elapsed < Duration::from_secs(10), "took {elapsed:?}");
}
