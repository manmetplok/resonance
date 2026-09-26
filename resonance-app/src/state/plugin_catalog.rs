//! Available-plugin catalog: the CLAP scan result surfaced in Settings,
//! the mixer's Add-plugin pickers, and the `plugins.catalog` control
//! method (ARCH-06 A6-2).
//!
//! Held as a sub-struct on [`Resonance`](crate::Resonance) so handlers
//! that only care about the scan result can take `&PluginCatalog` /
//! `&mut PluginCatalog` instead of the whole app.

use resonance_audio::types::{PluginScanFailure, ScannedPlugin};

/// The CLAP plugin scan's result: what is available, what failed to
/// load, and whether a rescan is currently in flight.
#[derive(Debug, Clone, Default)]
pub struct PluginCatalog {
    /// Plugins the last scan found and could load.
    pub available_plugins: Vec<ScannedPlugin>,
    /// Bundles the last scan found but could not load (ba todo #1307).
    ///
    /// Kept because a broken `.clap` is invisible otherwise: it simply
    /// does not appear in the catalog, which reads as "not installed".
    /// Shown in Settings next to the rescan button and reported by
    /// `plugins.catalog`.
    pub plugin_scan_failures: Vec<PluginScanFailure>,
    /// A rescan has been asked for and its result has not arrived yet.
    /// Only the button's label depends on it; a scan is fast enough that
    /// nothing is blocked while it runs.
    pub plugin_scan_in_progress: bool,
}
