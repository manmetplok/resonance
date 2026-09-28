//! Skill ↔ param-key lockstep for this plugin: every key, choice label and
//! factory preset a skill names inside a `<!-- keys: com.resonance.compressor -->` block
//! must exist here (see `resonance_dsp_test_support::skill_keys`).

resonance_dsp_test_support::skill_keys_test!(resonance_compressor::ResonanceCompressor);
