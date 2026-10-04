//! The first-party plugins' identity as the host relies on it: their CLAP
//! ids, and the param keys the app reads or writes by name (code review
//! ARCH2-03).
//!
//! The app depends on no plugin crate, so before this module it named
//! these by string literal — rename `output_mode` (renames are supported
//! via `ParamRename`) and every drums track silently spawned multi-out
//! sub-tracks with no test failing; rename a CLAP id and every template
//! track became a missing-plugin slot. Now each plugin declares its
//! `CLAP_ID` and these params *from* this module, the app imports it, and
//! each plugin's tests pin that every key below resolves to a real param.
//! `tools/arch-invariants` rejects a first-party CLAP id spelled as a
//! literal anywhere else in the plugins' or the app's sources.
//!
//! Lives in the SDK because it is the one crate every plugin and the app
//! already share.

/// `com.resonance.amp` — NAM amp/cab modeller.
pub const AMP: &str = "com.resonance.amp";
/// `com.resonance.color` — saturation / colour.
pub const COLOR: &str = "com.resonance.color";
/// `com.resonance.compressor`.
pub const COMPRESSOR: &str = "com.resonance.compressor";
/// `com.resonance.delay`.
pub const DELAY: &str = "com.resonance.delay";
/// `com.resonance.drums` — the sampled drum kit (Drummica).
pub const DRUMS: &str = "com.resonance.drums";
/// `com.resonance.eq`.
pub const EQ: &str = "com.resonance.eq";
/// `com.resonance.gate`.
pub const GATE: &str = "com.resonance.gate";
/// `com.resonance.granular-delay`.
pub const GRANULAR_DELAY: &str = "com.resonance.granular-delay";
/// `com.resonance.ir` — impulse-response loader.
pub const IR: &str = "com.resonance.ir";
/// `com.resonance.mastering` — the mastering chain `master.assist` drives.
pub const MASTERING: &str = "com.resonance.mastering";
/// `com.resonance.reverb`.
pub const REVERB: &str = "com.resonance.reverb";
/// `com.resonance.stereo`.
pub const STEREO: &str = "com.resonance.stereo";
/// `com.resonance.wavetable` — the wavetable synth.
pub const WAVETABLE: &str = "com.resonance.wavetable";

/// Every first-party CLAP id, one per crate under `plugins/`.
pub const ALL: &[&str] = &[
    AMP,
    COLOR,
    COMPRESSOR,
    DELAY,
    DRUMS,
    EQ,
    GATE,
    GRANULAR_DELAY,
    IR,
    MASTERING,
    REVERB,
    STEREO,
    WAVETABLE,
];

/// Plugins whose sidechain key port is secondary: they read a key (the
/// reverb ducks its wet return from one, the EQ's dynamic bands can
/// detect on one) but are not what an unqualified "key this track from
/// X" means. Both declare `SIDECHAIN_INPUT`, which their tests pin.
pub const SECONDARY_KEY_PLUGINS: &[&str] = &[REVERB, EQ];

/// Param keys of [`DRUMS`] the host reads by name (`drums_mirror`).
pub mod drums {
    /// The kit selector: `-2` parked (external/missing kit), `-1` the
    /// built-in kit, `0..=999` a library slot. Its text is the kit's name.
    pub const KIT_SELECT: &str = "kit_select";
    /// The selected kit's load progress, 0..1 (read-only): 1.0 once the
    /// kit plays; 0 with text "empty slot" or "failed" when nothing loads.
    pub const KIT_LOAD_PROGRESS: &str = "kit_load_progress";
    /// `0` Stereo (everything sums to the main output), `1` Multi (per-pad
    /// ports plus the Overhead port, each a sub-track).
    pub const OUTPUT_MODE: &str = "output_mode";
    /// [`OUTPUT_MODE`]'s Stereo step.
    pub const OUTPUT_MODE_STEREO: i32 = 0;
    /// [`OUTPUT_MODE`]'s Multi step.
    pub const OUTPUT_MODE_MULTI: i32 = 1;
    /// Every key above, for the plugin's lockstep test.
    pub const HOST_KEYS: &[&str] = &[KIT_SELECT, KIT_LOAD_PROGRESS, OUTPUT_MODE];
}

/// Param keys of [`AMP`] the host reads by name.
pub mod amp {
    /// The NAM model selector: a library slot whose text is the model's
    /// name; `*.set_plugin_param` resolves a model name through it.
    pub const FILE_SELECT: &str = "file_select";
    /// Every key above, for the plugin's lockstep test.
    pub const HOST_KEYS: &[&str] = &[FILE_SELECT];
}
