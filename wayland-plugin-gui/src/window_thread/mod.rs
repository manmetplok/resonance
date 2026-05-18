//! The editor thread.
//!
//! Owns a Wayland connection, an xdg_toplevel window, an EGL context and an
//! `egui_glow::Painter`. Runs an SCTK calloop event loop, consumes commands
//! from the public [`Editor`] handle, and repaints on demand.
//!
//! This module is split into:
//! - [`state`] — the [`State`] struct mutated by SCTK dispatch handlers.
//! - [`delegates`] — SCTK delegate trait impls (compositor, output, seat,
//!   keyboard, pointer, xdg window) and the `delegate_*!` macro wiring.
//! - [`event_loop`] — the main editor thread entry point and event loop.
//! - [`paint`] — per-frame egui paint + EGL surface management.
//! - [`debug`] — opt-in framebuffer dump for development debugging.

mod debug;
mod delegates;
mod event_loop;
mod paint;
mod state;

pub use event_loop::{Command, EditorThread};
