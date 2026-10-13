//! The control layer's mutation gate (todo #1161, code review
//! STATE2-05): which methods run without an open project, which are
//! refused `busy` while the app cannot take an edit, and which reads
//! stay answerable mid-drag. Split out of `mod.rs` so the dispatch
//! skeleton and the gate's allowlists can change independently.

use resonance_control::methods::control::HELLO;
use resonance_control::RpcError;

use crate::Resonance;

/// True when `method` needs no active project and so bypasses
/// [`mutation_gate_error`] (todo #1161): the read-only introspection
/// (`control.hello`, `song.*`, `job.*`) plus the project-lifecycle
/// methods (`project.*`), which are the ones that *establish* the
/// project the gate otherwise requires. Every other method mutates the
/// current project and is gated.
///
/// This is NOT "read-only": `project.*` creates and opens projects, and
/// `presets.*` / `drum_kits.*` / `amp_models.*` write the user's
/// libraries (delete, import, save). What they share is that none of
/// them edits the *open* project, so none needs the gate or an undo
/// entry. (Was `is_read_only_method`; renamed by code review ARCH2-12.
/// Its counterpart below the gate is [`reads_open_project_only`]: gated
/// methods that read the open project and so stay answerable mid-drag.)
///
/// `plugins.*` is the one addition to that list: it is read-only AND
/// project-independent (unlike `master.summary` / `edit.status`, which
/// read but describe the open project and so stay gated).
///
/// The `notes.*` create/insert/etc., `transport.*`, `track.*` /
/// `mixer.*`, `section.*`, `harmony.*`, `generate.*`, `vocal.*` and
/// `render.*` namespaces are all mutating and deliberately absent. So is
/// `meter.*` (todo #1219), which mutates nothing but measures the open
/// project — a stable `busy` with nothing open beats a measurement of a
/// project that isn't there.
pub(crate) fn bypasses_mutation_gate(method: &str) -> bool {
    use resonance_control::methods;
    method == HELLO
        || methods::song::METHODS.contains(&method)
        || methods::project::METHODS.contains(&method)
        // `plugins.catalog` reads `available_plugins`, which the
        // scanner fills at startup and which no project owns. Gating it
        // made a pure catalog query answer `busy` with nothing open, so
        // an agent could not even find out what it had to build with
        // before opening a project (todo #1236).
        || methods::plugins::METHODS.contains(&method)
        // `amp_models.*` is the user's model library, which no project
        // owns (nam-model-library.md §9.3).
        || methods::amp_models::METHODS.contains(&method)
        // `drum_kits.*` is the user's kit library, likewise.
        || methods::drum_kits::METHODS.contains(&method)
        // `presets.*` is the user's preset library (§12.2).
        || methods::presets::METHODS.contains(&method)
        // `vocal.voicebanks` lists the machine's installed voicebanks
        // (vocal-mcp-control-and-quality.md §4.5); the rest of `vocal.*`
        // edits the project and stays gated.
        || method == methods::vocal::VOICEBANKS
        // `vocal.voicebank_install` downloads into the same per-user
        // folder; no project needs to be open, nothing is undoable.
        || method == methods::vocal::VOICEBANK_INSTALL
        // `vocal.voice_presets` lists the built-in and the user's voice
        // presets: a per-user library, no project needed.
        || method == methods::vocal::VOICE_PRESETS
        || resonance_control::job::METHODS.contains(&method)
}

/// True when `method` is part of protocol v1 (i.e. listed in the
/// `control.hello` capabilities).
pub(super) fn is_protocol_method(method: &str) -> bool {
    use std::sync::OnceLock;
    static CAPABILITIES: OnceLock<Vec<&'static str>> = OnceLock::new();
    CAPABILITIES
        .get_or_init(resonance_control::methods::capabilities)
        .contains(&method)
}

/// Why a mutating control method cannot run right now, or `None` when
/// the app can take the edit. Mirrors the pre-dispatch gates that would
/// otherwise silently swallow a synthesized domain message (startup
/// modal / offline bounce / freeze render) — a remote client must see a
/// stable `busy` error instead of a no-op that claims success.
pub(crate) fn mutation_gate_error(app: &Resonance, method: &str) -> Option<RpcError> {
    if !app.io.has_active_project {
        return Some(RpcError::busy(
            "no active project — open or create one first",
        ));
    }
    // Between `ClearAll` and `AllCleared` (a project load, a template
    // instantiation, a structural undo/redo) the replay is about to
    // rebuild everything from its snapshot: an edit taken now would be
    // acknowledged and then wiped (code review UPD-03).
    if app.io.loading || app.io.pending_load.is_some() {
        return Some(RpcError::busy(
            "a project load / undo replay is in progress; retry shortly",
        ));
    }
    // A WAV / FLAC mixdown (GUI bounce or `render.mixdown`) drives the
    // live plugin instances like a bounce in place, and the GUI gate
    // drops every synthesized domain message while it runs (code review
    // UPD-06) — so a mutation must answer `busy` here rather than no-op.
    // `meter.*` is the one namespace that stays open: its own render
    // guard refuses the offline source and keeps the live master meter
    // readable during a render, which is exactly when a client wants it.
    if app.io.bouncing && !resonance_control::methods::meter::METHODS.contains(&method) {
        return Some(RpcError::busy(
            "an offline render is in progress; retry when it finishes",
        ));
    }
    // A GUI drag is open (code review STATE2-05): its pending snapshot
    // is the pre-drag state, so an edit recorded now would be taken back
    // by the user's next undo of the drag, and even a click that moved
    // nothing would commit. Refuse edits until the gesture ends — it is a
    // pointer drag, over in moments. Calls that only read the open
    // project stay answerable, so an agent polling meters while the user
    // drags is not turned away.
    if app.session.undo.has_pending() && !reads_open_project_only(method) {
        return Some(RpcError::busy(
            "an edit gesture is still in progress in the app; retry once it finishes",
        ));
    }
    offline_render_busy_error(app)
}

/// Gated methods that need an open project but change nothing in it:
/// they stay answerable while a GUI drag is open ([`mutation_gate_error`]).
/// Anything not listed is treated as an edit, the safe default.
fn reads_open_project_only(method: &str) -> bool {
    use resonance_control::methods::{
        automation, bus, edit, external, global, master, meter, pool, track, vocal,
    };
    meter::METHODS.contains(&method)
        || [
            automation::LANES,
            bus::PLUGIN_PARAMS,
            bus::PLUGIN_PRESETS,
            edit::STATUS,
            external::DEVICES,
            external::STATUS,
            global::LIST_EVENTS,
            master::ASSIST,
            master::PLUGIN_PARAMS,
            master::PLUGIN_PRESETS,
            master::SUMMARY,
            pool::LIST,
            track::PLUGIN_PARAMS,
            track::PLUGIN_PRESETS,
            track::PRESETS,
            vocal::GET_PARAMS,
            // Reads the lane, writes the user's preset folder (no project
            // edit).
            vocal::SAVE_VOICE_PRESET,
        ]
        .contains(&method)
}

/// `busy` while an offline bounce or a track freeze holds the offline
/// renderer. Split out of [`mutation_gate_error`] because the
/// destructive project-lifecycle methods (`project.new` / `project.open`)
/// run *above* the mutation gate — they must establish a project the
/// gate otherwise requires — yet still must not swap the project out
/// from under an in-flight render. They reuse this exact check (and its
/// wording) so both surfaces refuse identically.
pub(crate) fn offline_render_busy_error(app: &Resonance) -> Option<RpcError> {
    if app.modals.bounce_in_progress.is_some() {
        return Some(RpcError::busy(
            "an offline bounce is rendering; retry when it finishes",
        ));
    }
    if app.freeze.any_in_flight() {
        return Some(RpcError::busy(
            "a track freeze is rendering; retry when it finishes",
        ));
    }
    if app.stem_export_in_progress() {
        return Some(RpcError::busy(
            "a stem export is rendering; retry when it finishes",
        ));
    }
    None
}
