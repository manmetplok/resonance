//! The arrange-row layout model lives in `state::arrange_layout` (ARCH2-05:
//! `state::arrange` reads it, and state imports nothing from the view);
//! this is the view's import path for it.

pub use crate::state::arrange_layout::*;
