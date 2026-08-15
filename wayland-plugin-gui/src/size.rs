//! The live window size, shared between the editor thread and the
//! public [`crate::Editor`] handle.
//!
//! The window's size is decided by the *compositor*, not by us: a
//! `xdg_toplevel.configure` arrives whenever the user drags the window
//! edge, tiles it, or the compositor otherwise re-sizes it, and the
//! editor thread simply applies what it is told. Before ba todo #1337
//! none of that came back: `Editor::get_size` returned whatever was last
//! *requested*, so a host asking the plugin how big its editor is — which
//! is how a project file records the editor size — persisted a size the
//! user had not seen since they first opened the window, and restored
//! the wrong one next session.
//!
//! [`SharedSize`] is the feedback path. The editor thread publishes
//! every applied size into it; the handle reads it. One `AtomicU64`
//! holds both dimensions (width in the high 32 bits, height in the low
//! 32) so a reader can never observe a half-updated pair — a width from
//! before the resize with a height from after it would be a size the
//! window never actually had.
//!
//! Both sides use `Relaxed` ordering deliberately: the value guards no
//! other memory, and a reader that misses the very latest resize by a
//! few microseconds simply reads the previous size and picks up the new
//! one on its next call.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// A cloneable handle to one window's current logical size.
///
/// Clones share the same cell: the editor thread holds one, the public
/// handle another.
#[derive(Debug, Clone)]
pub struct SharedSize(Arc<AtomicU64>);

impl SharedSize {
    /// A cell seeded with the window's initial size, so a reader before
    /// the first configure sees the requested size rather than zero.
    pub fn new(size: (u32, u32)) -> Self {
        Self(Arc::new(AtomicU64::new(pack(size))))
    }

    /// Publish a new size. Called by the editor thread for every size it
    /// actually applies — compositor-driven or requested — and
    /// optimistically by [`crate::Editor::set_size`].
    pub fn set(&self, size: (u32, u32)) {
        self.0.store(pack(size), Ordering::Relaxed);
    }

    /// The current size.
    pub fn get(&self) -> (u32, u32) {
        unpack(self.0.load(Ordering::Relaxed))
    }
}

/// Width into the high 32 bits, height into the low 32.
fn pack((w, h): (u32, u32)) -> u64 {
    ((w as u64) << 32) | h as u64
}

fn unpack(bits: u64) -> (u32, u32) {
    ((bits >> 32) as u32, bits as u32)
}
