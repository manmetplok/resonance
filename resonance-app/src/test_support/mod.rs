//! Test-support accessors for [`Resonance`].
//!
//! Public read-only accessors / mutators for integration tests. These
//! exist so `tests/*.rs` files (which are external compile units and
//! see only public API) can verify reducer-driven state changes
//! without poking at private fields. They're `#[doc(hidden)]` because
//! they aren't part of the library's user-facing surface —
//! application code inside the crate still goes through the
//! `pub(crate)` fields directly.
//!
//! Kept out of `lib.rs` purely for separation of concerns; this is a
//! plain `impl Resonance` continuation, not behind any feature gate
//! (integration tests can't see `#[cfg(test)]` items, and a required
//! cargo feature would complicate `cargo test`).

//! Split per domain (ba todo #1134); each submodule is a plain
//! `impl Resonance` continuation, so the split is invisible to
//! callers — tests keep calling `app.test_*()` methods unchanged.

mod mixer_plugins;
pub use mixer_plugins::SendSlotAffordances;
mod pool_media;
mod project;
mod timeline;
mod tracks;
mod transport;
mod vocal_compose;
