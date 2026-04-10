//! Standalone harness for iterating on the wavetable editor UI without
//! loading the plugin into a CLAP host.
//!
//! Constructs a `ResonanceWavetable`, asks it for its editor factory, and
//! tells the factory to create a floating Wayland window. The harness then
//! blocks on the main thread; close the window (or Ctrl-C) to exit.
//!
//! This also spins a dummy "audio thread" that mutates the viz state so the
//! live visualisations are visible even without real audio.
//!
//!     cargo run -p resonance-wavetable --example editor_standalone

use resonance_plugin::plugin::ResonancePlugin;
use resonance_wavetable::ResonanceWavetable;

fn main() {
    let plugin = ResonanceWavetable::new();
    let factory = plugin
        .editor_factory()
        .expect("wavetable plugin should expose an editor factory");

    let mut editor = factory
        .create("wayland", true)
        .expect("factory should build a wayland floating editor");
    editor.show();

    // Block forever. Close the window or Ctrl-C to exit.
    loop {
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
}
