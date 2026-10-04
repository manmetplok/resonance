//! States of [`SharedState::count_in_record_arm`](super::SharedState) (code
//! review RT-08).

/// No record is waiting on a count-in.
pub const IDLE: u8 = 0;
/// A session is open; the audio thread starts it when the count-in ends.
pub const ARMED: u8 = 1;
/// The audio thread is mid-flip (a few stores); never seen for long.
pub const FIRING: u8 = 2;
/// The audio thread started the take; the engine thread finishes the
/// bookkeeping and goes back to `IDLE`.
pub const FIRED: u8 = 3;
