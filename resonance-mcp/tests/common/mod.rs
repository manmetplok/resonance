//! A fake resonance app: a unix-socket JSON-RPC server speaking the
//! `resonance-control` protocol, for exercising the MCP translation
//! layer without any GUI.

use resonance_control::methods::control::{HelloParams, HelloResult};
use resonance_control::rpc::{Request, Response};
use resonance_control::{write_message, FramingError, MessageReader, RpcError};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

static NEXT_SOCKET: AtomicU64 = AtomicU64::new(0);

/// What the fake should do with one non-handshake request.
/// `None` closes the connection (simulates an app crash/restart).
pub type Handler = dyn Fn(&Request) -> Option<Response> + Send + Sync + 'static;

/// Handle to the fake app; the socket lives in a per-test temp path.
pub struct FakeApp {
    path: PathBuf,
    /// Every non-handshake request seen, in order, across connections.
    pub requests: Arc<Mutex<Vec<Request>>>,
    /// Number of completed `control.hello` handshakes (connections).
    pub handshakes: Arc<AtomicU64>,
}

impl FakeApp {
    /// Spawn a fake app answering `control.hello` with `protocol_version`
    /// and everything else through `handler`.
    pub fn spawn(
        protocol_version: u32,
        handler: impl Fn(&Request) -> Option<Response> + Send + Sync + 'static,
    ) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "resonance-mcp-test-{}-{}",
            std::process::id(),
            NEXT_SOCKET.fetch_add(1, Ordering::Relaxed),
        ));
        std::fs::create_dir_all(&dir).expect("create fake-app socket dir");
        let path = dir.join("control.sock");
        let listener = UnixListener::bind(&path).expect("bind fake-app socket");
        let requests: Arc<Mutex<Vec<Request>>> = Arc::default();
        let handshakes = Arc::new(AtomicU64::new(0));
        let handler: Arc<Handler> = Arc::new(handler);
        {
            let requests = Arc::clone(&requests);
            let handshakes = Arc::clone(&handshakes);
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(stream) = stream else { break };
                    // Tests drive a single client; serving connections
                    // serially keeps the fake simple and deterministic.
                    serve(
                        stream,
                        protocol_version,
                        &handler,
                        &requests,
                        &handshakes,
                    );
                }
            });
        }
        Self {
            path,
            requests,
            handshakes,
        }
    }

    /// The socket path to hand to `ControlClient::new`.
    pub fn path(&self) -> PathBuf {
        self.path.clone()
    }

    /// The methods of every non-handshake request seen so far.
    pub fn seen_methods(&self) -> Vec<String> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .map(|r| r.method.clone())
            .collect()
    }
}

impl Drop for FakeApp {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn serve(
    stream: UnixStream,
    protocol_version: u32,
    handler: &Arc<Handler>,
    requests: &Arc<Mutex<Vec<Request>>>,
    handshakes: &Arc<AtomicU64>,
) {
    let mut writer = match stream.try_clone() {
        Ok(w) => w,
        Err(_) => return,
    };
    let mut reader = MessageReader::from_reader(stream);
    loop {
        let request: Request = match reader.read_message() {
            Ok(Some(request)) => request,
            Ok(None) => return,
            Err(FramingError::Invalid { source, .. }) => {
                let _ = write_message(
                    &mut writer,
                    &Response::failure(None, RpcError::parse_error(source.to_string())),
                );
                continue;
            }
            Err(_) => return,
        };
        let response = if request.method == resonance_control::methods::control::HELLO {
            let _params: HelloParams = request.params().expect("well-formed hello");
            handshakes.fetch_add(1, Ordering::Relaxed);
            let result = HelloResult {
                app_version: "fake-app".to_owned(),
                protocol_version,
                capabilities: resonance_control::methods::capabilities()
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
            };
            Response::success(request.id.clone(), &result).expect("hello result serializes")
        } else {
            requests.lock().unwrap().push(request.clone());
            match handler(&request) {
                Some(response) => response,
                // Simulated crash: close this connection.
                None => return,
            }
        };
        if write_message(&mut writer, &response).is_err() {
            return;
        }
    }
}

/// Shorthand: a success response echoing `result` for `request`.
pub fn ok<T: serde::Serialize>(request: &Request, result: &T) -> Option<Response> {
    Some(Response::success(request.id.clone(), result).expect("result serializes"))
}

/// Shorthand: an error response for `request`.
pub fn fail(request: &Request, error: RpcError) -> Option<Response> {
    Some(Response::failure(Some(request.id.clone()), error))
}
