/// Plugin parameters: master volume, the global voice/velocity/round-robin
/// controls, the kit selector and its load progress, and per pad its
/// volume, pan, mute, articulation and per-mic trims.
///
/// Every level is in dB (drums-plugin-rework.md §7 E9, D6): see
/// [`crate::level`]. A v1 state, which stored them as linear gains (and
/// a pad's two close mics as one `balance`, its overhead as `oh_blend`),
/// is converted once on load by [`upgrade_v1_levels`].
use std::sync::Arc;

use resonance_plugin::*;

use crate::articulation::{ARTICULATION_LABELS, ARTICULATION_PRIMARY};
use crate::choice::ChoiceParam;
use crate::drum_map::{NUM_PADS, PAD_MAPPINGS};
use crate::kit::OUTPUT_PORT_NAMES;
use crate::level::{self, MAX_TRIM_DB, MAX_VOLUME_DB, MIN_DB};
use crate::selection::{KitSelection, MAX_KIT_SLOT, NO_KIT};
use crate::velocity;
use crate::voice::MAX_VOICES;

/// Number of param fields per pad, used for param indexing.
pub const PARAMS_PER_PAD: usize = 13;

/// Number of global params ahead of the per-pad block, used for param
/// indexing. The flat index is an enumeration order, not an identity:
/// hosts and the control API address a param by its string id (which the
/// CLAP bridge hashes into a stable numeric id), so adding a global
/// param moves the pad block along without disturbing anything saved.
pub const GLOBAL_PARAMS: usize = 9;

/// Labels for the round-robin mode choice, indexed by parameter value.
pub const ROUND_ROBIN_LABELS: &[&str] = &["Cycle", "Random"];

/// Labels for the output mode choice, indexed by parameter value.
pub const OUTPUT_MODE_LABELS: &[&str] = &["Stereo", "Multi"];
/// `output_mode`: everything to Main (the default, D5).
pub const OUTPUT_MODE_STEREO: i32 = 0;
/// `output_mode`: per-pad ports plus the Overhead port.
pub const OUTPUT_MODE_MULTI: i32 = 1;

/// How the velocity humanize reads: `Off`, or `±5` (MIDI steps).
pub fn humanize_label(steps: f32) -> String {
    let rounded = (steps * 10.0).round() / 10.0;
    if rounded <= 0.0 {
        "Off".to_string()
    } else if rounded.fract() == 0.0 {
        format!("±{rounded:.0}")
    } else {
        format!("±{rounded:.1}")
    }
}

/// Parse [`humanize_label`] (`Off`, `±5`, `+5`, `5`).
pub fn humanize_from_label(text: &str) -> Option<f32> {
    let t = text.trim();
    if t.eq_ignore_ascii_case("off") {
        return Some(0.0);
    }
    let digits = t.trim_start_matches(['±', '+']).trim();
    digits.parse::<f32>().ok().filter(|v| v.is_finite()).map(f32::abs)
}

/// How many parameters this plugin exposes: the globals, then one block
/// of [`PARAMS_PER_PAD`] per pad.
pub const PARAM_COUNT: usize = GLOBAL_PARAMS + crate::drum_map::NUM_PADS * PARAMS_PER_PAD;

/// The master level's id. Not v1's `master_volume`: that id held a
/// linear gain, and a host that re-sends a project's saved values by id
/// after the state (the app's param overrides, an automation lane) would
/// land those linear values on the dB param, after the state's one-shot
/// conversion. Under a new id they name no param and are dropped.
pub const MASTER_LEVEL_ID: &str = "master_level";
/// v1's id for the master, as a linear gain ([`upgrade_v1_levels`]).
pub const V1_MASTER_VOLUME_ID: &str = "master_volume";
/// A pad level's id is `pad_N_<this>` — not v1's `pad_N_volume`, for the
/// reason [`MASTER_LEVEL_ID`] gives.
pub const PAD_LEVEL_FIELD: &str = "level";
/// v1's `pad_N_<this>`, a linear gain ([`upgrade_v1_levels`]).
pub const V1_PAD_VOLUME_FIELD: &str = "volume";

pub struct DrumParams {
    /// Master level in dB, −∞ ([`MIN_DB`]) … +6, default 0 dB. Its id is
    /// [`MASTER_LEVEL_ID`] (`master_level`).
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
    /// Stereo or Multi output (E11, D5), labelled by
    /// [`OUTPUT_MODE_LABELS`]. **Stereo** (the default for a fresh
    /// instance) sums every pad and mic to Main, so a host that only
    /// reads port 0 hears the whole kit. **Multi** routes each pad's
    /// close mics to its `pad_N_output` port and every overhead take to
    /// the Overhead port. The plugin declares all seven ports either way
    /// (a port list cannot change while a host holds it); in Stereo the
    /// six beside Main are silent. Not automatable: routing, not playing.
    pub output_mode: ChoiceParam,
    /// Velocity humanize (E7): every hit's velocity moves at random by up
    /// to ± this many MIDI steps, 0 … 20, default 0 (off). Applied before
    /// the velocity curve, from a fixed-seed generator, so a render is
    /// reproducible.
    pub velocity_humanize: FloatParam,
    /// Disk streaming's preload (E14): how much of each take stays in
    /// memory — Off (every take whole), 32k, 64k or 128k frames, default
    /// 32k ([`crate::stream::DEFAULT_PRELOAD`]). Changing it reloads the
    /// kit (the instance's watcher thread calls
    /// [`crate::stream::apply_preload_param`]).
    ///
    /// Not automatable: every change is a kit reload. Not in the params
    /// state either: the state keeps carrying the preload as frames under
    /// its own key (`stream_preload`), as it did before the param existed.
    pub stream_preload: ChoiceParam,
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
            master_volume: level_param(MASTER_LEVEL_ID, "Master Volume", MAX_VOLUME_DB),
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
                    min: crate::selection::PARKED_KIT,
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
            output_mode: ChoiceParam::new(
                "output_mode",
                "Output Mode",
                OUTPUT_MODE_STEREO,
                OUTPUT_MODE_LABELS,
            )
            .not_automatable(),
            velocity_humanize: FloatParam::new(
                "velocity_humanize",
                "Velocity Humanize",
                0.0,
                FloatRange::Linear {
                    min: 0.0,
                    max: crate::dsp::sampler::MAX_HUMANIZE,
                },
            )
            .with_value_to_string(Arc::new(humanize_label))
            .with_string_to_value(Arc::new(humanize_from_label)),
            stream_preload: ChoiceParam::new(
                "stream_preload",
                "Stream Preload",
                crate::stream::preload_param_value(crate::stream::DEFAULT_PRELOAD),
                crate::stream::PRELOAD_LABELS,
            )
            .not_automatable()
            .excluded_from_state(),
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

/// The mic slots a pad's per-mic trims are indexed by: its first and
/// second close-mic bank and its overhead. E15 (bleed and room banks)
/// extends this list; a slot's trim id is `pad_N_<key>_trim` with the
/// slot's [`MIC_SLOT_KEYS`] entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MicSlot {
    /// The pad's first close-mic bank (kick In, snare Top, the tom, …).
    Close1 = 0,
    /// The pad's second close-mic bank (kick Out, snare Btm).
    Close2 = 1,
    /// The overhead bank.
    Overhead = 2,
}

/// How many [`MicSlot`]s a pad has a trim for.
pub const MIC_SLOTS: usize = 3;

/// Each [`MicSlot`]'s part of its trim's id (`pad_N_<key>_trim`).
pub const MIC_SLOT_KEYS: [&str; MIC_SLOTS] = ["mic1", "mic2", "oh"];

/// Each [`MicSlot`]'s part of its trim's name.
const MIC_SLOT_NAMES: [&str; MIC_SLOTS] = ["Close Mic 1", "Close Mic 2", "OH"];

impl MicSlot {
    /// The slot of close-mic bank `bank_index` (0 or 1).
    pub fn close(bank_index: usize) -> Self {
        if bank_index == 0 {
            Self::Close1
        } else {
            Self::Close2
        }
    }
}

pub struct PadParams {
    /// Pad level in dB, −∞ ([`MIN_DB`]) … +6, default 0 dB. Its id is
    /// `pad_N_level` ([`PAD_LEVEL_FIELD`]).
    pub volume: FloatParam,
    pub pan: FloatParam,
    pub mute: BoolParam,
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
    /// Per-mic trims in dB, −∞ … +12, default 0 dB, indexed by
    /// [`MicSlot`]: `pad_N_mic1_trim`, `pad_N_mic2_trim`, `pad_N_oh_trim`.
    /// They replace v1's `balance` (between the two close mics) and
    /// `oh_blend` (the overhead's level), which [`upgrade_v1_levels`]
    /// converts.
    ///
    /// The overhead trim scales the pad's overhead take wherever it is
    /// routed. The second close mic's trim is hidden on pads that are
    /// never recorded with two.
    pub trims: [FloatParam; MIC_SLOTS],
    /// Choke group (E12): 0 = none, 1..=[`MAX_CHOKE_GROUP`]. A hit on a
    /// pad fades out every sounding voice of the same group — the open
    /// hat cut by the closed or pedal hat. Defaults to the Drummica table
    /// ([`PAD_MAPPINGS`]): every hi-hat in group 1, nothing else choked.
    pub choke: IntParam,
    /// Which output port the pad's close mics play on in Multi output
    /// mode (E11), one of [`OUTPUT_PORT_NAMES`]. Defaults to the
    /// Drummica table (kick → Kick, …, Count Stick → Main). Its overhead
    /// take always goes to the Overhead port in Multi, and everything to
    /// Main in Stereo. Not automatable: routing, not playing.
    pub output: ChoiceParam,
    /// Pitch in semitones (E8), −24 … +24, default 0, resolved to the
    /// cent (0.01 st): one param carries both the coarse and the fine
    /// tune, so an automation lane moves pitch as one value. Played by
    /// fractional playback with 4-point Hermite interpolation; at exactly
    /// 0 the sampler stays on its integer path, bit for bit.
    pub tune: FloatParam,
    /// Hold before the decay (E8), 0 … 2000 ms, default 0. Only matters
    /// with a decay set.
    pub hold: FloatParam,
    /// Decay to silence after the hold (E8), 5 … 4000 ms, or **Off** (the
    /// top of the range, the default): the whole sample plays.
    pub decay: FloatParam,
    /// Where in the sample a hit starts (E8), 0 … 100 ms, default 0.
    pub start: FloatParam,
}

/// The tune range, either way, in semitones.
pub const MAX_TUNE_ST: f32 = 24.0;
/// The hold range's top, in ms.
pub const MAX_HOLD_MS: f32 = 2_000.0;
/// The shortest decay, in ms.
pub const MIN_DECAY_MS: f32 = 5.0;
/// The decay value that means "Off": the top of its range.
pub const DECAY_OFF_MS: f32 = 4_000.0;
/// The sample-start range's top, in ms.
pub const MAX_START_MS: f32 = 100.0;

/// How a tune reads: `0.00 st`, `+12.00 st`, `-0.50 st`.
pub fn tune_label(st: f32) -> String {
    let cents = (st * 100.0).round();
    if cents == 0.0 {
        "0.00 st".to_string()
    } else {
        format!("{:+.2} st", cents / 100.0)
    }
}

/// Parse a tune: semitones (`+12`, `-3.5 st`) or cents (`50 ct`,
/// `-25 cents`).
pub fn tune_from_label(text: &str) -> Option<f32> {
    let t = text.trim().to_ascii_lowercase();
    let (number, scale) = if let Some(n) = t
        .strip_suffix("cents")
        .or_else(|| t.strip_suffix("cent"))
        .or_else(|| t.strip_suffix("ct"))
    {
        (n, 0.01)
    } else {
        (t.strip_suffix("st").unwrap_or(&t), 1.0)
    };
    let v: f32 = number.trim().trim_start_matches('+').parse().ok()?;
    v.is_finite()
        .then(|| (v * scale).clamp(-MAX_TUNE_ST, MAX_TUNE_ST))
}

/// How a time in ms reads: `0 ms`, `250 ms`, `1.20 s`.
pub fn ms_label(ms: f32) -> String {
    if ms >= 1_000.0 {
        format!("{:.2} s", ms / 1_000.0)
    } else if ms >= 10.0 {
        format!("{ms:.0} ms")
    } else {
        format!("{ms:.1} ms")
    }
}

/// Parse a time: `250 ms`, `1.2 s`, or a bare number of ms.
pub fn ms_from_label(text: &str) -> Option<f32> {
    let t = text.trim().to_ascii_lowercase();
    let (number, scale) = if let Some(n) = t.strip_suffix("ms") {
        (n, 1.0)
    } else if let Some(n) = t.strip_suffix('s') {
        (n, 1_000.0)
    } else {
        (t.as_str(), 1.0)
    };
    let v: f32 = number.trim().parse().ok()?;
    v.is_finite().then_some(v * scale)
}

/// How a decay reads: `Off` at the top of the range, else its time.
pub fn decay_label(ms: f32) -> String {
    if ms >= DECAY_OFF_MS {
        "Off".to_string()
    } else {
        ms_label(ms)
    }
}

/// Parse a decay: `Off` or a time.
pub fn decay_from_label(text: &str) -> Option<f32> {
    if text.trim().eq_ignore_ascii_case("off") {
        Some(DECAY_OFF_MS)
    } else {
        ms_from_label(text)
    }
}

/// A time param in ms over `min..max` (its text carries the unit, `ms` or
/// `s`, so no fixed unit is declared), skewed so the short end — where
/// drum envelopes live — gets most of the travel.
fn ms_param(
    id: &'static str,
    name: &'static str,
    default: f32,
    min: f32,
    max: f32,
) -> FloatParam {
    FloatParam::new(
        id,
        name,
        default,
        FloatRange::Skewed {
            min,
            max,
            factor: -1.5,
        },
    )
    .with_value_to_string(Arc::new(ms_label))
    .with_string_to_value(Arc::new(ms_from_label))
}

/// The highest choke group a pad can be put in.
pub const MAX_CHOKE_GROUP: i32 = 8;

/// How a choke group reads: `None`, `Group 1` … `Group 8`.
pub fn choke_label(group: i32) -> String {
    if group <= 0 {
        "None".to_string()
    } else {
        format!("Group {group}")
    }
}

/// Parse [`choke_label`] (or a bare number) back to a group.
pub fn choke_from_label(text: &str) -> Option<i32> {
    let t = text.trim();
    if t.eq_ignore_ascii_case("none") || t.eq_ignore_ascii_case("off") {
        return Some(0);
    }
    let digits = if t.len() >= 5 && t[..5].eq_ignore_ascii_case("group") {
        t[5..].trim()
    } else {
        t
    };
    digits
        .parse::<i32>()
        .ok()
        .map(|g| g.clamp(0, MAX_CHOKE_GROUP))
}

/// A static id or name for a per-pad parameter.
fn leak(text: String) -> &'static str {
    Box::leak(text.into_boxed_str())
}

/// A level in dB from −∞ ([`MIN_DB`]) up to `max_db`, default 0 dB. The
/// travel is skewed toward the top, where levels are set: 0 dB sits at
/// about four fifths of a fader.
fn level_param(id: &'static str, name: &'static str, max_db: f32) -> FloatParam {
    FloatParam::new(
        id,
        name,
        0.0,
        FloatRange::Skewed {
            min: MIN_DB,
            max: max_db,
            factor: 1.0,
        },
    )
    .with_unit("dB")
    .with_value_to_string(Arc::new(level::db_label))
    .with_string_to_value(Arc::new(level::db_from_label))
}

impl PadParams {
    fn new(index: usize) -> Self {
        let id = |field: &str| leak(format!("pad_{index}_{field}"));
        let name = |field: &str| leak(format!("Pad {index} {field}"));
        let mapping = &PAD_MAPPINGS[index];
        let trims = std::array::from_fn(|slot| {
            let param = level_param(
                id(&format!("{}_trim", MIC_SLOT_KEYS[slot])),
                name(&format!("{} Trim", MIC_SLOT_NAMES[slot])),
                MAX_TRIM_DB,
            );
            if slot == MicSlot::Close2 as usize && mapping.close_mic_positions.len() < 2 {
                param.hidden()
            } else {
                param
            }
        });

        Self {
            volume: level_param(id(PAD_LEVEL_FIELD), name("Volume"), MAX_VOLUME_DB),
            pan: FloatParam::new(
                id("pan"),
                name("Pan"),
                0.0,
                FloatRange::Linear {
                    min: -1.0,
                    max: 1.0,
                },
            )
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            mute: BoolParam::new(id("mute"), name("Mute"), false),
            articulation: {
                let param = ChoiceParam::new(
                    id("articulation"),
                    name("Articulation"),
                    ARTICULATION_PRIMARY,
                    ARTICULATION_LABELS,
                );
                if mapping.has_articulation {
                    param
                } else {
                    param.hidden()
                }
            },
            trims,
            choke: IntParam::new(
                id("choke"),
                name("Choke Group"),
                mapping.choke_group.map_or(0, i32::from),
                IntRange::Linear {
                    min: 0,
                    max: MAX_CHOKE_GROUP,
                },
            )
            .with_value_to_string(Arc::new(choke_label))
            .with_string_to_value(Arc::new(choke_from_label)),
            output: ChoiceParam::new(
                id("output"),
                name("Output"),
                mapping.output_group.index() as i32,
                &OUTPUT_PORT_NAMES,
            )
            .not_automatable(),
            tune: FloatParam::new(
                id("tune"),
                name("Tune"),
                0.0,
                FloatRange::Linear {
                    min: -MAX_TUNE_ST,
                    max: MAX_TUNE_ST,
                },
            )
            .with_unit("st")
            .with_value_to_string(Arc::new(tune_label))
            .with_string_to_value(Arc::new(tune_from_label)),
            hold: ms_param(id("hold"), name("Hold"), 0.0, 0.0, MAX_HOLD_MS),
            decay: ms_param(
                id("decay"),
                name("Decay"),
                DECAY_OFF_MS,
                MIN_DECAY_MS,
                DECAY_OFF_MS,
            )
            .with_value_to_string(Arc::new(decay_label))
            .with_string_to_value(Arc::new(decay_from_label)),
            start: ms_param(id("start"), name("Sample Start"), 0.0, 0.0, MAX_START_MS),
        }
    }

    /// The trim of `slot`.
    pub fn trim(&self, slot: MicSlot) -> &FloatParam {
        &self.trims[slot as usize]
    }
}

impl Default for PadParams {
    fn default() -> Self {
        Self::new(0)
    }
}

impl DrumParams {
    /// The exposed parameters in host order: the globals, then each
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
            6 => return &self.output_mode,
            7 => return &self.velocity_humanize,
            8 => return &self.stream_preload,
            _ => {}
        }
        let pad_idx = (index - GLOBAL_PARAMS) / PARAMS_PER_PAD;
        let field = (index - GLOBAL_PARAMS) % PARAMS_PER_PAD;
        let pad = &self.pads[pad_idx];
        match field {
            0 => &pad.volume,
            1 => &pad.pan,
            2 => &pad.mute,
            3 => &pad.articulation,
            4 => &pad.trims[0],
            5 => &pad.trims[1],
            6 => &pad.trims[2],
            7 => &pad.choke,
            8 => &pad.output,
            9 => &pad.tune,
            10 => &pad.hold,
            11 => &pad.decay,
            12 => &pad.start,
            _ => &pad.volume,
        }
    }
}

/// Bring a v1 state's levels up to v2, in place: returns whether it
/// changed anything.
///
/// v1 stored `master_volume` and `pad_N_volume` as linear gains (0..1,
/// default 0.8), a pad's two close mics as one `pad_N_balance` (0..1:
/// the first mic at `1 − b`, the second at `b`, and only on pads that
/// had two), and its overhead level as `pad_N_oh_blend` (0..1). v2 has
/// them all in dB under **new ids** — `master_level`, `pad_N_level` and
/// the per-mic trims (E9) — so nothing that addresses a param by its v1
/// id (a project's re-sent param overrides, an automation lane) can put
/// a linear value on a dB param after this conversion: it names no param
/// and is dropped. A state is v1 when its params carry any `balance` or
/// `oh_blend` — every v1 save wrote every one, and v2 writes none.
///
/// The conversion keeps the sound: each gain becomes the dB that plays
/// it ([`level::gain_to_db`], so a silent 0 becomes −∞), and a pad's two
/// mics keep the gains the balance gave them. A pad with one close mic
/// ignored the balance in v1, so its trim stays at 0 dB.
///
/// The balance is converted against the **static** pad table
/// ([`PAD_MAPPINGS`]`[i].close_mic_positions`): whether pad `i` has two
/// close mics is what the Drummica table says, not what the kit the
/// state names loads — a state is upgraded before (and without) any kit
/// being loaded, so there is nothing else to go by. A kit whose pad has
/// two close mics where the table says one (or the other way round)
/// therefore gets 0 dB trims where v1 split the balance; its levels are
/// still the v1 ones, only that pad's mic balance is reset.
///
/// A state saved by a build between the dB switch and the id change (K7
/// development builds: dB values under the old ids, no `balance`) has
/// its values moved to the new ids as they are, not converted again.
/// Either way a value already under a new id is never overwritten, so
/// the upgrade is idempotent.
///
/// Run on every load path before the params are read, as part of the
/// plugin's state upgrade ([`crate::upgrade_state`]).
pub fn upgrade_v1_levels(state: &mut serde_json::Value) -> bool {
    let Some(params) = state.get_mut("params").and_then(|p| p.as_object_mut()) else {
        return false;
    };
    let is_v1 = is_v1_params(params);
    let to_db = |gain: f64| serde_json::Value::from(level::gain_to_db(gain as f32) as f64);
    let mut changed = false;
    // Move `from` to `to`, converting a v1 gain; a value already under
    // `to` wins.
    let mut carry = |params: &mut serde_json::Map<String, serde_json::Value>,
                     from: &str,
                     to: &str| {
        let Some(old) = params.remove(from) else {
            return;
        };
        changed = true;
        if params.contains_key(to) {
            return;
        }
        let value = match old.as_f64() {
            Some(v) if is_v1 => to_db(v),
            _ => old,
        };
        params.insert(to.to_string(), value);
    };
    carry(params, V1_MASTER_VOLUME_ID, MASTER_LEVEL_ID);
    for i in 0..NUM_PADS {
        carry(
            params,
            &format!("pad_{i}_{V1_PAD_VOLUME_FIELD}"),
            &format!("pad_{i}_{PAD_LEVEL_FIELD}"),
        );
    }
    if !is_v1 {
        return changed;
    }
    for (i, mapping) in PAD_MAPPINGS.iter().enumerate() {
        let key = |field: &str| format!("pad_{i}_{field}");
        if let Some(b) = params.remove(&key("balance")).and_then(|v| v.as_f64()) {
            if mapping.close_mic_positions.len() == 2 {
                params.entry(key("mic1_trim")).or_insert(to_db(1.0 - b));
                params.entry(key("mic2_trim")).or_insert(to_db(b));
            }
        }
        if let Some(o) = params.remove(&key("oh_blend")).and_then(|v| v.as_f64()) {
            params.entry(key("oh_trim")).or_insert(to_db(o));
        }
    }
    true
}

/// Whether a state's params are v1's: any `balance` or `oh_blend`.
pub fn is_v1_params(params: &serde_json::Map<String, serde_json::Value>) -> bool {
    (0..NUM_PADS).any(|i| {
        params.contains_key(&format!("pad_{i}_balance"))
            || params.contains_key(&format!("pad_{i}_oh_blend"))
    })
}
