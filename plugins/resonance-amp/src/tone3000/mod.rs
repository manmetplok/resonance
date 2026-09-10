//! Tone3000 API browser: OAuth 2.0 PKCE auth, search, download.
//!
//! Split into small submodules per the project-wide modularity preference:
//! - [`types`]: serde structs for the REST responses.
//! - [`auth`]: PKCE flow, loopback redirect, token persistence.
//! - [`client`]: blocking HTTP wrapper over `ureq` with Bearer auth.
//! - [`worker`]: background thread that owns the client and services
//!   commands from the editor, publishing results through a shared
//!   state struct the UI polls each frame.

pub mod auth;
pub mod client;
pub mod types;
pub mod worker;

/// Tone3000 publishable key (client_id). Register an application in
/// tone3000.com account settings and paste the value here before the
/// OAuth flow will work. Left empty on purpose — the editor shows a
/// "not configured" state until this is set, so an un-configured build
/// is harmless rather than broken.
pub const TONE3000_CLIENT_ID: &str = "t3k_pub_2kTrT7x4schSLLmHWtZdZmjtpBJ9G-oy";

/// Loopback port the one-shot OAuth redirect listener binds to. Must
/// match the `http://127.0.0.1:<port>` URI registered on tone3000.com.
pub const REDIRECT_PORT: u16 = 47834;

/// Root of the Tone3000 API. Every endpoint is reached by appending to
/// this.
pub const API_BASE: &str = "https://www.tone3000.com";

/// Where downloaded .nam files live. Re-exported rather than declared
/// here: `initialize()` seeds the model browser from the same directory
/// and cannot reach anything behind the `editor` feature.
pub use crate::models::{models_dir, MODEL_SUBDIR};
