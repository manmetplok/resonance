/// Plugin parameters: master volume, the global voice/velocity/round-robin
/// controls, the kit selector and its load progress, and per-pad volume,
/// pan, mute, OH blend, balance and articulation choice.
use std::sync::Arc;

use resonance_plugin::*;

use crate::articulation::{ARTICULATION_LABELS, ARTICULATION_PRIMARY};
use crate::choice::ChoiceParam;
use crate::drum_map::{NUM_PADS, PAD_MAPPINGS};
use crate::selection::{KitSelection, MAX_KIT_SLOT, NO_KIT};
use crate::velocity;
use crate::voice::MAX_VOICES;

/// Number of param fields per pad, used for param indexing.
pub const PARAMS_PER_PAD: usize = 6;

/// Number of global params ahead of the per-pad block, used for param
/// indexing. The flat index is an enumeration order, not an identity:
/// hosts and the control API address a param by its string id (which the
/// CLAP bridge hashes into a stable numeric id), so adding a global
/// param moves the pad block along without disturbing anything saved.
pub const GLOBAL_PARAMS: usize = 6;

/// Labels for the round-robin mode choice, indexed by parameter value.
pub const ROUND_ROBIN_LABELS: &[&str] = &["Cycle", "Random"];

/// How many parameters this plugin exposes: the globals, then one block
/// of [`PARAMS_PER_PAD`] per pad.
pub const PARAM_COUNT: usize = GLOBAL_PARAMS + crate::drum_map::NUM_PADS * PARAMS_PER_PAD;

pub struct DrumParams {
    pub master_volume: FloatParam,
    /// Ceiling on simultaneously sounding voices. A hit uses one voice
    /// per loaded mic bank (a kick with in/out mics plus overheads uses
    /// three), matching how the sampler counts them. Defaults to
    /// [`MAX_VOICES`], the hard cap the plugin has always had, so the
    /// parameter changes nothing until it is turned down.
    pub polyphony: IntParam,
    /// Global velocity curve, -1 (hard) … 0 (linear) … +1 (soft). See
    /// [`crate::velocity`]; 0 is an exact identity.
    pub velocity_curve: FloatParam,
    /// How the sampler walks a layer's recorded takes — see
    /// [`crate::dsp::voice_pick::RoundRobinMode`]. Defaults to Cycle,
    /// which is what the sampler always did.
    pub round_robin_mode: ChoiceParam,
    /// The kit: a stable slot in the shared kit library, or
    /// [`NO_KIT`] for the built-in kit (drums-plugin-rework.md §5.1, D4).
    /// Its text is the kit's name — `"<name> (missing)"` when the state
    /// named a kit that is not on this machine — and a name parses back to
    /// the slot, so the control API picks a kit by name. Setting it loads
    /// the kit ([`crate::selection::apply_pending`]).
    ///
    /// Not automatable: every change is a multi-gigabyte decode. Not in
    /// the state either: a slot is this machine's library layout, so the
    /// state carries the kit as a `kit_ref` and the slot is derived.
    pub kit_select: IntParam,
    /// Read-only 0..1: how far the kit `kit_select` names is from being in
    /// place on the audio thread — 1.0 only once the audio thread took it
    /// (§5.4). Written by the plugin every block; hosts and agents poll it.
    pub kit_load_progress: FloatParam,
    /// What `kit_select` means beyond a slot (a missing kit, a kit with no
    /// slot), and the library handle its text and the loader resolve
    /// against. Shared with the bridge, the saver and the editor.
    pub selection: Arc<KitSelection>,
    pub pads: [PadParams; NUM_PADS],
}

impl Default for DrumParams {
    fn default() -> Self {
        let selection = Arc::new(KitSelection::new());
        let text_sel = selection.clone();
        let parse_sel = selection.clone();
        Self {
            master_volume: FloatParam::new(
                "master_volume",
                "Master Volume",
                0.8,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            polyphony: IntParam::new(
                "polyphony",
                "Polyphony",
                MAX_VOICES as i32,
                IntRange::Linear {
                    min: 1,
                    max: MAX_VOICES as i32,
                },
            ),
            velocity_curve: FloatParam::new(
                "velocity_curve",
                "Velocity Curve",
                0.0,
                FloatRange::Linear {
                    min: -1.0,
                    max: 1.0,
                },
            )
            .with_value_to_string(Arc::new(velocity::curve_label))
            .with_string_to_value(Arc::new(velocity::curve_from_label)),
            round_robin_mode: ChoiceParam::new(
                "round_robin_mode",
                "Round Robin",
                0,
                ROUND_ROBIN_LABELS,
            ),
            // 1001 steps — past the engine's choice-label walk, so a
            // parameter query never enumerates the library.
            kit_select: IntParam::new(
                "kit_select",
                "Kit",
                NO_KIT,
                IntRange::Linear {
                    min: NO_KIT,
                    max: MAX_KIT_SLOT,
                },
            )
            .with_value_to_string(Arc::new(move |v| text_sel.text(v)))
            .with_string_to_value(Arc::new(move |t| parse_sel.parse(t)))
            .not_automatable()
            .excluded_from_state(),
            kit_load_progress: FloatParam::new(
                "kit_load_progress",
                "Kit Load Progress",
                1.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_value_to_string(Arc::new(|v| format!("{:.0}%", v * 100.0)))
            .read_only(),
            selection,
            pads: std::array::from_fn(PadParams::new),
        }
    }
}

impl DrumParams {
    /// The articulation of every pad, in the shape the kit loader takes:
    /// false = primary piece, true = the alternate one.
    ///
    /// Derived — the parameters are the source of truth. Anything that
    /// needs the articulation set (the loader, the editor, the watcher)
    /// reads it from here rather than keeping its own copy.
    pub fn articulations(&self) -> [bool; NUM_PADS] {
        std::array::from_fn(|i| self.pads[i].articulation.value() != ARTICULATION_PRIMARY)
    }
}

pub struct PadParams {
    pub volume: FloatParam,
    pub pan: FloatParam,
    pub mute: BoolParam,
    /// Blend amount (0..1) for this pad's overhead contribution when
    /// summed into the Overhead output port. 1.0 = full level, 0.0 =
    /// completely muted from the overhead bus. Defaults to 1.0 so the
    /// plugin sounds the same on first instantiation as it did before
    /// the multi-output rewrite.
    ///
    /// For pads the library records with overheads only (all cymbals in
    /// Drummica) the overhead take is routed to the pad's own group port
    /// instead — see `VoiceDestination::Overhead` — and this param scales
    /// it there, so turning it down still silences the pad.
    pub oh_blend: FloatParam,
    /// Balance (0..1) between the pad's two close-mic banks. 0.5 is
    /// equal — used as the default so the pre-existing single-bank
    /// sound is preserved. 0.0 favours the "left" side (kick In or
    /// snare Top), 1.0 favours the "right" side (kick Out or snare
    /// Btm). Ignored for pads with fewer than two close-mic banks.
    pub balance: FloatParam,
    /// Articulation choice: which recorded variant of the piece this pad
    /// plays, labelled by [`ARTICULATION_LABELS`] (0 = "mit Teppich",
    /// 1 = "ohne Teppich").
    ///
    /// **This parameter is the source of truth.** The kit loader builds
    /// the pad from the piece it selects, so writing it — from the
    /// inspector's chips, a host automation lane, or `set_plugin_param`
    /// over the control API — reloads the pad's samples through the same
    /// path (see [`crate::articulation`]).
    ///
    /// Hidden for pads the kit has no alternate recording of: they would
    /// be a control that cannot move anything, so the host is not offered
    /// one. The id still exists and still persists, so nothing that was
    /// saved against it breaks.
    pub articulation: ChoiceParam,
}

impl PadParams {
    fn new(index: usize) -> Self {
        // Use leaked strings for unique static IDs per pad
        let vol_id: &'static str = Box::leak(format!("pad_{}_volume", index).into_boxed_str());
        let vol_name: &'static str = Box::leak(format!("Pad {} Volume", index).into_boxed_str());
        let pan_id: &'static str = Box::leak(format!("pad_{}_pan", index).into_boxed_str());
        let pan_name: &'static str = Box::leak(format!("Pad {} Pan", index).into_boxed_str());
        let mute_id: &'static str = Box::leak(format!("pad_{}_mute", index).into_boxed_str());
        let mute_name: &'static str = Box::leak(format!("Pad {} Mute", index).into_boxed_str());
        let oh_id: &'static str = Box::leak(format!("pad_{}_oh_blend", index).into_boxed_str());
        let oh_name: &'static str = Box::leak(format!("Pad {} OH Blend", index).into_boxed_str());
        let bal_id: &'static str = Box::leak(format!("pad_{}_balance", index).into_boxed_str());
        let bal_name: &'static str = Box::leak(format!("Pad {} Balance", index).into_boxed_str());
        let art_id: &'static str =
            Box::leak(format!("pad_{}_articulation", index).into_boxed_str());
        let art_name: &'static str =
            Box::leak(format!("Pad {} Articulation", index).into_boxed_str());

        Self {
            volume: FloatParam::new(
                vol_id,
                vol_name,
                0.8,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            pan: FloatParam::new(
                pan_id,
                pan_name,
                0.0,
                FloatRange::Linear {
                    min: -1.0,
                    max: 1.0,
                },
            )
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            mute: BoolParam::new(mute_id, mute_name, false),
            oh_blend: FloatParam::new(
                oh_id,
                oh_name,
                1.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            balance: FloatParam::new(
                bal_id,
                bal_name,
                0.5,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            articulation: {
                let param =
                    ChoiceParam::new(art_id, art_name, ARTICULATION_PRIMARY, ARTICULATION_LABELS);
                if PAD_MAPPINGS[index].has_articulation {
                    param
                } else {
                    param.hidden()
                }
            },
        }
    }
}

impl Default for PadParams {
    fn default() -> Self {
        Self::new(0)
    }
}

impl DrumParams {
    /// The exposed parameters in host order: the six globals, then each
    /// pad's block of [`PARAMS_PER_PAD`].
    ///
    /// One ordered list, read by both `ResonancePlugin::param` and the
    /// editor's preset bar, rather than the same indexing arithmetic
    /// restated per call site (ba todo #1358).
    pub fn param_at(&self, index: usize) -> &dyn Param {
        match index {
            0 => return &self.master_volume,
            1 => return &self.polyphony,
            2 => return &self.velocity_curve,
            3 => return &self.round_robin_mode,
            4 => return &self.kit_select,
            5 => return &self.kit_load_progress,
            _ => {}
        }
        let pad_idx = (index - GLOBAL_PARAMS) / PARAMS_PER_PAD;
        let field = (index - GLOBAL_PARAMS) % PARAMS_PER_PAD;
        let pad = &self.pads[pad_idx];
        match field {
            0 => &pad.volume,
            1 => &pad.pan,
            2 => &pad.mute,
            3 => &pad.oh_blend,
            4 => &pad.balance,
            5 => &pad.articulation,
            _ => &pad.volume,
        }
    }
}
