//! Per-namespace method param/result types.
//!
//! Each submodule owns one method namespace and declares:
//! - `pub const` method-name strings (`"transport.play"`, ...),
//! - a `METHODS` slice listing them,
//! - the typed params/results those methods exchange.
//!
//! Read-only vs mutating is explicit per method doc: `song.*`,
//! `job.status` and `control.hello` never mutate; every mutating result
//! carries `{revision}` (see [`crate::common::MutationAck`]).
//! Methods documented as taking no params use `()`.

pub mod arrangement;
pub mod bus;
pub mod clip;
pub mod control;
pub mod edit;
pub mod external;
pub mod generate;
pub mod global;
pub mod harmony;
pub mod master;
pub mod meter;
pub mod mixer;
pub mod notes;
pub mod plugin_preset;
pub mod plugins;
pub mod pool;
pub mod project;
pub mod render;
pub mod section;
pub mod song;
pub mod track;
pub mod transport;
pub mod vocal;

/// Method names kept reachable only for backwards compatibility, each
/// an alias for a renamed method. They ARE in [`capabilities`] — the app
/// really does answer them — but they get no MCP tool of their own, so
/// an agent sees exactly one spelling per operation.
///
/// Every entry is scheduled for removal; see the alias constant's own
/// doc comment for what it was renamed to.
#[allow(deprecated)]
pub fn deprecated_aliases() -> Vec<&'static str> {
    vec![plugins::PLUGINS_DEPRECATED_ALIAS]
}

/// Every method name in protocol v1, in stable namespace order. This is
/// what `control.hello` reports as `capabilities`.
pub fn capabilities() -> Vec<&'static str> {
    let mut methods = Vec::new();
    for namespace in [
        control::METHODS,
        song::METHODS,
        project::METHODS,
        transport::METHODS,
        global::METHODS,
        track::METHODS,
        external::METHODS,
        plugins::METHODS,
        mixer::METHODS,
        master::METHODS,
        bus::METHODS,
        edit::METHODS,
        section::METHODS,
        harmony::METHODS,
        generate::METHODS,
        notes::METHODS,
        pool::METHODS,
        clip::METHODS,
        arrangement::METHODS,
        vocal::METHODS,
        render::METHODS,
        meter::METHODS,
        crate::job::METHODS,
    ] {
        methods.extend_from_slice(namespace);
    }
    methods
}
