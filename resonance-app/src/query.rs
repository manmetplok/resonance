//! Pure, headless queries over app state that the view (and the control
//! layer) ask — derived values, not state and not event handling (code
//! review ARCH2-05: the view used to reach into `engine_events` for them).

pub mod performance;
