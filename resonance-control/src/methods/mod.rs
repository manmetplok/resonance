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

pub mod control;
pub mod generate;
pub mod harmony;
pub mod master;
pub mod mixer;
pub mod notes;
pub mod project;
pub mod render;
pub mod section;
pub mod song;
pub mod track;
pub mod transport;
pub mod vocal;

/// Every method name in protocol v1, in stable namespace order. This is
/// what `control.hello` reports as `capabilities`.
pub fn capabilities() -> Vec<&'static str> {
    let mut methods = Vec::new();
    for namespace in [
        control::METHODS,
        song::METHODS,
        project::METHODS,
        transport::METHODS,
        track::METHODS,
        mixer::METHODS,
        master::METHODS,
        section::METHODS,
        harmony::METHODS,
        generate::METHODS,
        notes::METHODS,
        vocal::METHODS,
        render::METHODS,
        crate::job::METHODS,
    ] {
        methods.extend_from_slice(namespace);
    }
    methods
}
