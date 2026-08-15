//! `plugins.rescan` — pick up plugins installed since the app started
//! (ba todo #1307, finding X10).
//!
//! The catalog was filled by exactly one scan, at startup, so installing
//! a plugin meant restarting Resonance — for the GUI's add-plugin menus
//! and for `plugins.catalog` alike. This is the other half of the
//! Settings button: the same message, so an agent and a human reach the
//! same scan.
//!
//! Deliberately NOT project-gated (it sits in `plugins::METHODS`, which
//! `is_read_only_method` allowlists): the plugin catalog is a fact about
//! the machine, not about the open project, and a client should be able
//! to install a plugin and find it before it creates anything.
//!
//! The refreshed catalog does not come back in this reply — the scan
//! runs on the engine thread — so the ack means "asked", and
//! `plugins.catalog` is where the result (and any load failure) shows
//! up, exactly as `external.detect_latency` works.

use super::reply::{ack, reject};
use crate::message::{Message, PluginMessage};
use crate::update::control::run_via_update;
use crate::Resonance;
use iced::Task;
use resonance_control::methods::plugins;
use resonance_control::{Request, Response};

/// Handle a mutating `plugins.*` request, or `None` when `method`
/// belongs to another namespace. (`plugins.catalog` is read-only and is
/// answered by `song::try_handle`, above the mutation gate.)
pub(super) fn try_handle(
    app: &mut Resonance,
    request: &Request,
) -> Option<(Response, Task<Message>)> {
    match request.method.as_str() {
        plugins::RESCAN => Some(rescan(app, request)),
        _ => None,
    }
}

/// `plugins.rescan` — look for newly installed plugins.
///
/// No confirm flag: the scan is additive by construction. Bundles
/// already loaded stay loaded at the address their live instances came
/// from, so nothing that is playing, open or referenced can be disturbed
/// — there is no destruction to confirm. (The startup scan, which does
/// drop every instance, is not reachable from the control API at all.)
fn rescan(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    // Params are accepted and empty; parse them anyway so a client that
    // sends something unexpected learns it here rather than being
    // silently ignored.
    if let Err(e) = request.params::<plugins::RescanParams>() {
        return reject(request, e);
    }
    let task = run_via_update(app, Message::Plugin(PluginMessage::RescanPlugins));
    (ack(app, request), task)
}
