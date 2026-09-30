//! Project-state round-trip for user wavetables.
//!
//! # The frames are embedded, the path rides along
//!
//! resonance-ir and resonance-amp persist only the path of the file they
//! loaded, and reload it on restore. That is right for them — an IR folder or
//! a NAM model library is a collection the user browses (Prev/Next walks the
//! directory) — but a user wavetable is a one-off, often exported from
//! another tool into a scratch folder, and a project that silently loses its
//! oscillator's sound when it moves machines or the folder is tidied is the
//! worse failure. So the state carries the imported frames themselves, as
//! little-endian `f32` in base64: at most `256 × 2048` samples (2 MiB, about
//! 2.8 MB of text), usually far less. Only the frames, not the mips — those
//! are 12× larger and rebuilt deterministically from the frames.
//!
//! The path is kept too, for the editor's display and as the fallback for a
//! state that has no frames (a hand-written preset naming a file). On restore
//! the frames win: they are exactly what was playing, whatever has since
//! happened to the file.
//!
//! ```json
//! "user_wavetables": {
//!   "osc1": { "path": "/…/growl.wav", "name": "growl",
//!             "frame_size": 2048, "frames": "<base64 f32le>" }
//! }
//! ```
//!
//! A slot with nothing loaded is omitted, and so is the key when both are
//! empty — a fresh instance saves exactly the document it always did.
//! Restoring a state without the key clears both slots, so a project opened
//! over another does not inherit its tables. A slot whose frames are unusable
//! and whose file cannot be read is cleared (the oscillator plays bundled
//! table 0) and keeps its path, with the error shown in the editor.

use base64::Engine as _;
use serde_json::{json, Map, Value};

use crate::dsp::engine::NUM_OSCS;
use crate::dsp::wavetable::WAVETABLE_SIZE;

use super::UserWavetables;

/// Top-level state key.
pub const STATE_KEY: &str = "user_wavetables";

/// Per-oscillator key inside [`STATE_KEY`].
pub fn osc_key(osc: usize) -> String {
    format!("osc{}", osc + 1)
}

/// Encode frames as base64 little-endian `f32`.
pub fn encode_frames(frames: &[f32]) -> String {
    let bytes: Vec<u8> = frames.iter().flat_map(|s| s.to_le_bytes()).collect();
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Decode [`encode_frames`]' output.
pub fn decode_frames(text: &str) -> Result<Vec<f32>, String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(text)
        .map_err(|e| format!("embedded wavetable is not valid base64: {e}"))?;
    if bytes.len() % 4 != 0 {
        return Err("embedded wavetable is not a whole number of samples".to_string());
    }
    Ok(bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect())
}

impl resonance_plugin::plugin::ExtraStateSaver for UserWavetables {
    fn save(&self) -> Map<String, Value> {
        let mut slots = Map::new();
        for osc in 0..NUM_OSCS {
            let info = self.info(osc);
            if info.path.is_empty() && info.frames.is_none() {
                continue;
            }
            let mut slot = Map::new();
            slot.insert("path".into(), json!(info.path));
            slot.insert("name".into(), json!(info.name));
            if let Some(frames) = &info.frames {
                slot.insert("frame_size".into(), json!(WAVETABLE_SIZE));
                slot.insert("frames".into(), json!(encode_frames(frames)));
            }
            slots.insert(osc_key(osc), Value::Object(slot));
        }
        let mut map = Map::new();
        if !slots.is_empty() {
            map.insert(STATE_KEY.into(), Value::Object(slots));
        }
        map
    }

    /// User wavetables are the sound, and they are embedded (frames
    /// base64), so a preset travels whole.
    fn preset_keys(&self) -> &'static [&'static str] {
        &[STATE_KEY]
    }

    fn load(&self, state: &Value) {
        let slots = state.get(STATE_KEY);
        for osc in 0..NUM_OSCS {
            let Some(slot) = slots.and_then(|s| s.get(osc_key(osc))) else {
                self.clear(osc);
                continue;
            };
            let path = slot.get("path").and_then(Value::as_str).unwrap_or("");
            let name = slot.get("name").and_then(Value::as_str).unwrap_or("");

            let embedded = slot.get("frames").and_then(Value::as_str).map(|text| {
                let size = slot.get("frame_size").and_then(Value::as_u64);
                if size != Some(WAVETABLE_SIZE as u64) {
                    return Err(format!("embedded wavetable has frame size {size:?}"));
                }
                decode_frames(text)
            });
            let restored = match embedded {
                Some(Ok(frames)) => self.restore_frames(osc, path, name, frames),
                Some(Err(e)) => Err(e),
                None => Err(String::new()),
            };
            // Unusable (or no) embedded frames: fall back to the file, which
            // clears the slot if it can't be read either.
            if restored.is_err() {
                if path.is_empty() {
                    self.clear(osc);
                } else {
                    let _ = self.restore_file(osc, path);
                }
            }
        }
    }
}
