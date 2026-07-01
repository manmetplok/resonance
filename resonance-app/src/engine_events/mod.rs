//! Engine → GUI event dispatch.
//!
//! Each `AudioEvent` variant is routed to a per-domain handler module
//! by the free `handle_engine_event` function in `dispatch.rs`. The
//! dispatch itself stays thin so it's easy to find which file owns a
//! given event.

mod aux_sends;
mod clips;
mod dispatch;
mod midi;
mod midi_map;
pub mod performance;
mod plugins;
mod pool;
mod presets;
mod project_io;
mod reference;
mod tracks;
mod transport;

pub(crate) use dispatch::handle_engine_event;
