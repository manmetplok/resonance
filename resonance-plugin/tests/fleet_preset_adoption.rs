//! Every plugin in the fleet reaches the shared preset surface (ba todo
//! #1358, audit findings X1 and X2).
//!
//! `resonance-plugin` shipping `PresetBank` / `PresetSession` /
//! `preset_bar` does nothing on its own — the finding was "0 of 11 plugins
//! can save a preset", and it stays true until each editor adopts them. So
//! this asserts the adoption itself rather than the machinery, which is
//! covered in `tests/presets.rs`.
//!
//! The sources are `include_str!`d rather than read at runtime, so the
//! guard is compiled from the same tree it checks and cannot drift from
//! what actually builds. It is a source-text check, which is a blunt
//! instrument — but the alternative is a dependency from this crate onto
//! all eleven plugins that depend on it, which is a cycle.
//!
//! **Adding a plugin crate means adding it to [`FLEET`].** A new plugin
//! that skips the preset surface is exactly the regression this exists to
//! catch, and it can only catch what it is told about.

/// Every plugin crate, with its `lib.rs` and the editor source that draws
/// its chrome.
const FLEET: &[(&str, &str, &str)] = &[
    (
        "resonance-amp",
        include_str!("../../plugins/resonance-amp/src/lib.rs"),
        include_str!("../../plugins/resonance-amp/src/editor/header.rs"),
    ),
    (
        "resonance-compressor",
        include_str!("../../plugins/resonance-compressor/src/lib.rs"),
        include_str!("../../plugins/resonance-compressor/src/editor/app.rs"),
    ),
    (
        "resonance-delay",
        include_str!("../../plugins/resonance-delay/src/lib.rs"),
        include_str!("../../plugins/resonance-delay/src/editor/app.rs"),
    ),
    (
        "resonance-drums",
        include_str!("../../plugins/resonance-drums/src/lib.rs"),
        include_str!("../../plugins/resonance-drums/src/editor/chrome.rs"),
    ),
    (
        "resonance-eq",
        include_str!("../../plugins/resonance-eq/src/lib.rs"),
        include_str!("../../plugins/resonance-eq/src/editor/app.rs"),
    ),
    (
        "resonance-gate",
        include_str!("../../plugins/resonance-gate/src/lib.rs"),
        include_str!("../../plugins/resonance-gate/src/editor/mod.rs"),
    ),
    (
        "resonance-granular-delay",
        include_str!("../../plugins/resonance-granular-delay/src/lib.rs"),
        include_str!("../../plugins/resonance-granular-delay/src/editor/app.rs"),
    ),
    (
        "resonance-ir",
        include_str!("../../plugins/resonance-ir/src/lib.rs"),
        include_str!("../../plugins/resonance-ir/src/editor/header.rs"),
    ),
    (
        "resonance-mastering",
        include_str!("../../plugins/resonance-mastering/src/lib.rs"),
        include_str!("../../plugins/resonance-mastering/src/editor/header.rs"),
    ),
    (
        "resonance-reverb",
        include_str!("../../plugins/resonance-reverb/src/lib.rs"),
        include_str!("../../plugins/resonance-reverb/src/editor/mod.rs"),
    ),
    (
        "resonance-wavetable",
        include_str!("../../plugins/resonance-wavetable/src/lib.rs"),
        include_str!("../../plugins/resonance-wavetable/src/editor/chrome.rs"),
    ),
];

/// The audit counted eleven plugins. If that number moves, the list above
/// has to move with it — otherwise this file silently stops covering the
/// newcomer.
#[test]
fn the_fleet_is_the_size_the_audit_counted() {
    assert_eq!(
        FLEET.len(),
        11,
        "the audit's X1 finding is about 11 plugins; add the new crate to FLEET"
    );
}

/// Every plugin hands its `PresetSession` to the bridge, which is what
/// makes the loaded-preset identity survive closing the window (finding
/// X2, the persistence half).
#[test]
fn every_plugin_publishes_its_preset_session_as_extra_state() {
    for (crate_name, lib_rs, _editor) in FLEET {
        assert!(
            lib_rs.contains("fn extra_state_saver"),
            "{crate_name}: no extra_state_saver, so the loaded preset cannot \
             survive the window closing"
        );
        assert!(
            lib_rs.contains("Some(self.presets.clone())"),
            "{crate_name}: extra_state_saver must return the shared \
             PresetSession. A plugin with a saver of its own chains it with \
             PresetSession::with_extra rather than choosing between the two"
        );
    }
}

/// Every editor draws the shared bar, rather than a private preset combo.
/// This is the half a user can see: save, rename, delete, and what is
/// loaded right now.
#[test]
fn every_editor_draws_the_shared_preset_bar() {
    for (crate_name, _lib_rs, editor) in FLEET {
        assert!(
            editor.contains("preset_bar("),
            "{crate_name}: its editor chrome does not draw preset_bar, so \
             this plugin still cannot save a preset"
        );
    }
}

/// No editor keeps its own idea of which preset is loaded.
///
/// Gate, granular and wavetable each grew a private index into their
/// factory list. All three were display-only, none could represent a user
/// preset, and none survived the window closing — `PresetSession` is the
/// one place that answer lives now (ba todo #1280).
#[test]
fn no_editor_tracks_the_loaded_preset_itself() {
    for (crate_name, _lib_rs, editor) in FLEET {
        for line in editor.lines() {
            // Skip prose: the doc comments here explain what these fields
            // *used* to be, which is worth keeping and is not a field.
            if line.trim_start().starts_with("//") {
                continue;
            }
            for stale in ["selected_preset", "preset_idx"] {
                assert!(
                    !line.contains(stale),
                    "{crate_name}: `{stale}` is a private copy of what \
                     PresetSession already tracks — found in `{}`",
                    line.trim()
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Factory preset files (plugin-preset-library.md P1)
// ---------------------------------------------------------------------------

use resonance_plugin::library_marks::vocab;
use resonance_plugin::presets::{FactoryEntry, PresetFile, PresetLibrary, PresetMeta, Query};

/// Every plugin crate (the audit's eleven plus color and stereo), with its
/// `lib.rs` for the CLAP id.
const ALL_PLUGINS: &[(&str, &str)] = &[
    ("resonance-amp", include_str!("../../plugins/resonance-amp/src/lib.rs")),
    ("resonance-color", include_str!("../../plugins/resonance-color/src/lib.rs")),
    ("resonance-compressor", include_str!("../../plugins/resonance-compressor/src/lib.rs")),
    ("resonance-delay", include_str!("../../plugins/resonance-delay/src/lib.rs")),
    ("resonance-drums", include_str!("../../plugins/resonance-drums/src/lib.rs")),
    ("resonance-eq", include_str!("../../plugins/resonance-eq/src/lib.rs")),
    ("resonance-gate", include_str!("../../plugins/resonance-gate/src/lib.rs")),
    (
        "resonance-granular-delay",
        include_str!("../../plugins/resonance-granular-delay/src/lib.rs"),
    ),
    ("resonance-ir", include_str!("../../plugins/resonance-ir/src/lib.rs")),
    ("resonance-mastering", include_str!("../../plugins/resonance-mastering/src/lib.rs")),
    ("resonance-reverb", include_str!("../../plugins/resonance-reverb/src/lib.rs")),
    ("resonance-stereo", include_str!("../../plugins/resonance-stereo/src/lib.rs")),
    ("resonance-wavetable", include_str!("../../plugins/resonance-wavetable/src/lib.rs")),
];

/// The plugins whose factory bank is for an instrument, so its presets use
/// the instrument categories (§4.4). Every other bank is an effect's.
const INSTRUMENT_PLUGINS: &[&str] = &["resonance-wavetable", "resonance-drums"];

/// Every factory preset id, per plugin, in bank order. A factory id is
/// never reused for a different sound and never silently dropped: a
/// favourite, a tag or a project's loaded identity may point at it. Add
/// new ids here; renaming or removing one is a decision, not a cleanup.
const PINNED_IDS: &[(&str, &[&str])] = &[
    ("resonance-color", &[
        "bus-warm-glue", "bass-iron", "vocal-tube-air", "drums-tape-15", "master-subtle-tape",
    ]),
    ("resonance-compressor", &[
        "kick-punch", "snare-slam", "bass-glue", "vocal-lead", "guitar-control", "drum-bus",
        "mix-bus", "master-glue", "parallel-smash", "transparent", "bus-auto-glue",
    ]),
    ("resonance-delay", &[
        "quarter-note", "dotted-eighth", "slapback", "dub", "ping-pong-eighth", "lo-fi-tape",
    ]),
    ("resonance-eq", &[
        "kick-punch", "kick-sub", "snare-crack", "snare-body", "bass-tight", "bass-warm",
        "guitar-body", "guitar-air", "vocal-clarity", "synth-wide", "master-polish",
    ]),
    ("resonance-gate", &[
        "init-default", "vocal-noise-gate", "drums-snare-gate", "drums-tom-gate",
        "guitar-noise-floor", "gentle-expander", "dialogue-room-tone", "keyed-open-on-kick",
        "keyed-trance-gate",
    ]),
    ("resonance-granular-delay", &[
        "init-per-grain-cloud", "eighth-triplet-echo-tempo-locked-grains",
        "d-minor-shimmer-scale-quantize-diffusion", "vocal-doubler-psola-hq",
        "reverse-haze-ping-pong-hp-damp", "frozen-drone-lo-fi-freeze",
        "tape-warble-repitch-clean-repeats", "aligned-cloud-wsola-onsets",
    ]),
    ("resonance-reverb", &[
        "tight-room", "vocal-plate", "warm-hall", "cathedral", "ambient-bloom", "shimmer-drone",
        "snare-plate", "snare-tight", "snare-ambient", "snare-gated",
    ]),
    ("resonance-stereo", &[
        "init-transparent", "mono-bass-below-120", "widen-mono-source", "master-gentle-width",
        "vocal-micro-shift-double", "pad-diffuse-wide", "haas-safe",
    ]),
    ("resonance-wavetable", &[
        "init", "lead-supersaw", "lead-analog-square", "lead-sync-screamer",
        "lead-hard-sync-sweep", "bass-reese", "bass-sub-round", "bass-acid-squelch",
        "bass-ladder-fm-growl", "bass-wobble", "pad-warm-analog", "pad-juno-chorus",
        "pad-glass-shimmer", "pad-evolving-choir", "pluck-digital-bell", "pluck-nylon-harp",
        "pluck-stack", "keys-electric-piano", "keys-driven-chords", "keys-cathedral-organ",
        "keys-vintage-poly", "arp-formant-talker", "arp-metallic-sequence", "fx-risers",
        "fx-noise-sweep", "fx-drone-texture", "brass-stab", "strings-ensemble",
    ]),
];

/// `(id, name, file)` for every entry of a plugin's factory table, read
/// out of its `presets.rs` source (the same source the MCP lockstep test
/// reads names from).
fn factory_entries(crate_name: &str) -> Vec<(String, String, String)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../plugins");
    let Ok(src) = std::fs::read_to_string(root.join(crate_name).join("src/presets.rs")) else {
        return Vec::new();
    };
    let literal = |line: &str, key: &str| -> Option<String> {
        let rest = line.trim().strip_prefix(key)?.trim_start();
        let rest = rest.strip_prefix('"')?;
        Some(rest[..rest.find('"')?].to_string())
    };
    let mut out = Vec::new();
    let (mut id, mut name) = (None, None);
    for line in src.lines() {
        if let Some(v) = literal(line, "id:") {
            id = Some(v);
        } else if let Some(v) = literal(line, "name:") {
            name = Some(v);
        } else if let Some(v) = literal(line, "json: include_str!(") {
            let file = v.trim_start_matches("../presets/").to_string();
            out.push((
                id.take()
                    .unwrap_or_else(|| panic!("{crate_name}: {file} has no id")),
                name.take()
                    .unwrap_or_else(|| panic!("{crate_name}: {file} has no name")),
                file,
            ));
        }
    }
    out
}

fn clap_id_of(lib_rs: &str) -> &str {
    let key = "const CLAP_ID: &'static str = \"";
    let at = lib_rs.find(key).expect("every plugin declares CLAP_ID");
    let rest = &lib_rs[at + key.len()..];
    &rest[..rest.find('"').unwrap()]
}

/// Every factory entry in the fleet is a format-1 file whose id and name
/// match its table entry, with a category from its plugin class's
/// vocabulary, a description and seeded facet values (§4.4, §5, D7).
#[test]
fn every_factory_preset_is_a_complete_format_1_file() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../plugins");
    let mut total = 0;
    for (crate_name, lib_rs) in ALL_PLUGINS {
        let clap_id = clap_id_of(lib_rs);
        let entries = factory_entries(crate_name);
        let categories = if INSTRUMENT_PLUGINS.contains(crate_name) {
            vocab::CATEGORIES_INSTRUMENT
        } else {
            vocab::CATEGORIES_EFFECT
        };
        let mut seen = std::collections::HashSet::new();
        for (id, name, file_name) in &entries {
            let at = format!("{crate_name}: {file_name}");
            let slug = id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
            assert!(!id.is_empty() && slug, "{at}: id {id:?} is not a slug");
            assert!(seen.insert(id.clone()), "{at}: duplicate id {id:?}");

            let path = root.join(crate_name).join("presets").join(file_name);
            let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{at}: {e}"));
            let raw: serde_json::Value = serde_json::from_str(&text).unwrap();
            let file = PresetFile::parse(&text).unwrap_or_else(|e| panic!("{at}: {e}"));
            assert_eq!(&file.id, id, "{at}: the file's id differs from the table's");
            assert_eq!(
                &file.meta.name, name,
                "{at}: meta.name must equal the Rust literal (D7)"
            );
            assert_eq!(file.plugin.id, clap_id, "{at}: plugin.id");
            let stored: PresetMeta = serde_json::from_value(raw["meta"].clone()).unwrap();
            assert_eq!(
                stored.clone().normalized(),
                stored,
                "{at}: meta is not in its stored spelling"
            );

            let meta = &file.meta;
            let category = meta
                .category
                .as_deref()
                .unwrap_or_else(|| panic!("{at}: no category"));
            assert!(
                categories.contains(&category),
                "{at}: category {category:?} is not one of {categories:?}"
            );
            // One convention for every bank's reset preset.
            assert_eq!(
                name.starts_with("Init"),
                category == "Init",
                "{at}: an Init preset has the Init category, and only it does"
            );
            assert!(meta.description.is_some(), "{at}: no description");
            assert!(!meta.character.is_empty(), "{at}: no character");
            for c in &meta.character {
                assert!(
                    vocab::CHARACTER.contains(&c.as_str()),
                    "{at}: character {c:?} is not in the seeded vocabulary"
                );
            }
            for i in &meta.instrument {
                assert!(
                    vocab::INSTRUMENT.contains(&i.as_str()),
                    "{at}: instrument {i:?} is not in the seeded vocabulary"
                );
            }
            let params = file
                .state
                .doc
                .as_ref()
                .and_then(|doc| doc.get("params"))
                .and_then(|p| p.as_object());
            assert!(params.is_some(), "{at}: no state.doc.params");
        }

        // No stray file that no table entry ships.
        if let Ok(dir) = std::fs::read_dir(root.join(crate_name).join("presets")) {
            for f in dir.flatten() {
                let n = f.file_name().to_string_lossy().into_owned();
                assert!(
                    entries.iter().any(|(_, _, file)| *file == n),
                    "{crate_name}: presets/{n} is not in the factory table"
                );
            }
        }

        let pinned: Vec<&str> = PINNED_IDS
            .iter()
            .find(|(c, _)| c == crate_name)
            .map(|(_, ids)| ids.to_vec())
            .unwrap_or_default();
        let ids: Vec<&str> = entries.iter().map(|(id, _, _)| id.as_str()).collect();
        assert_eq!(
            ids, pinned,
            "{crate_name}: the factory id set moved; see PINNED_IDS"
        );
        total += entries.len();
    }
    assert_eq!(total, 95, "the audit counted 95 factory presets in 9 plugins");
}

/// Convergence (10)/(11): the preset library now searches with the shared
/// engine (`library_view::BrowserModel`) and slugs with the shared
/// `library_marks` rules. Every factory preset of the fleet must still be
/// found by each of its own metadata values, through the facet filters
/// and through the text syntax an agent or a browser types.
#[test]
fn every_factory_preset_matches_its_own_filters_after_the_swap() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../plugins");
    let library = PresetLibrary::new();
    let read = |crate_name: &str, file: &str| {
        std::fs::read_to_string(root.join(crate_name).join("presets").join(file)).unwrap()
    };
    let mut checked = 0;
    for (crate_name, lib_rs) in ALL_PLUGINS {
        let clap_id = clap_id_of(lib_rs);
        let entries = factory_entries(crate_name);
        library.register_factory_entries(
            clap_id,
            entries.iter().map(|(id, name, file)| FactoryEntry {
                id: id.clone(),
                name: name.clone(),
                json: read(crate_name, file),
            }),
        );
        for (id, _, file) in &entries {
            let at = format!("{crate_name}: {file}");
            let meta = PresetFile::parse(&read(crate_name, file)).unwrap().meta;
            let found = |q: Query| -> bool {
                library
                    .query(&q)
                    .hits
                    .iter()
                    .any(|h| h.record.preset.id == *id)
            };
            let base = || Query::plugin(clap_id);
            let text = |t: String| Query {
                text: t,
                ..base()
            };
            let category = meta.category.clone().unwrap();
            let q = Query {
                category: vec![category.to_lowercase()],
                ..base()
            };
            assert!(found(q), "{at}: category");
            assert!(found(text(format!("cat:{}", category.to_lowercase()))), "{at}: cat:");
            // Scoped tokens match the value or a slug prefix, not any
            // substring, and fold case.
            assert!(found(text(format!("cat:{}", category.to_uppercase()))), "{at}: CAT:");
            let inner: String = category.to_lowercase().chars().skip(1).collect();
            if inner.len() >= 2 {
                assert!(!found(text(format!("cat:{inner}"))), "{at}: cat:{inner} is no prefix");
            }
            for g in &meta.genres {
                let q = Query {
                    genres: vec![g.clone()],
                    ..base()
                };
                assert!(found(q), "{at}: genre {g}");
                assert!(found(text(format!("genre:{g}"))), "{at}: genre:{g}");
                // `genre:rock` must not find `post-rock`.
                if let Some((_, tail)) = g.rsplit_once('-') {
                    let other = meta.genres.iter().any(|o| o.starts_with(tail));
                    if !other {
                        assert!(!found(text(format!("genre:{tail}"))), "{at}: genre:{tail}");
                    }
                }
            }
            for c in &meta.character {
                let q = Query {
                    character: vec![c.clone()],
                    ..base()
                };
                assert!(found(q), "{at}: character {c}");
                assert!(found(text(format!("char:{c}"))), "{at}: char:{c}");
            }
            for i in &meta.instrument {
                let q = Query {
                    instrument: vec![i.clone()],
                    ..base()
                };
                assert!(found(q), "{at}: instrument {i}");
                assert!(found(text(format!("for:{i}"))), "{at}: for:{i}");
            }
            for t in &meta.tags {
                let q = Query {
                    tags: vec![t.clone()],
                    ..base()
                };
                assert!(found(q), "{at}: tag {t}");
                assert!(found(text(format!("tag:{t}"))), "{at}: tag:{t}");
            }
            assert!(found(text(meta.name.clone())), "{at}: by name");
            assert!(found(text("is:factory".into())), "{at}: is:factory");
            checked += 1;
        }
    }
    assert_eq!(checked, 95);
}
