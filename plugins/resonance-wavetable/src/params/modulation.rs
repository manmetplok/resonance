//! Modulation matrix parameters.
//!
//! The matrix is stored on [`super::WavetableParams::mod_slots`] as a
//! `Vec<ModSlotParams>` of length [`crate::dsp::modulation::NUM_MOD_SLOTS`].
//! Each slot's three parameters (source, destination, amount) are defined
//! in [`super::mod_slot`] and re-exported there.

#[allow(unused_imports)]
pub use super::mod_slot::ModSlotParams;
