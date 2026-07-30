//! The built-in drum groove library and its density knob (ba doc #270
//! §5, todo #1191).
//!
//! These patterns are authored, not generated: the point of asking for
//! "four-on-floor" is a kick on every beat, which a Euclidean roll at
//! some density cannot promise. So the assertions here are about the
//! actual steps, not just that something came out.

use resonance_app::compose::{
    apply_density, builtin_pattern_catalog, builtin_pattern_names, instantiate_builtin,
    is_builtin_pattern, DrumPattern,
};
use resonance_common::drum_map as gm;

/// Steps a pad fires on, by note, across the pattern's groups.
fn steps_for(pattern: &DrumPattern, note: u8) -> Vec<usize> {
    pattern
        .groups
        .iter()
        .flat_map(|g| g.pads.iter())
        .filter(|p| p.note == note)
        .flat_map(|p| {
            p.pattern
                .iter()
                .enumerate()
                .filter(|(_, v)| **v > 0)
                .map(|(i, _)| i)
        })
        .collect()
}

fn build(name: &str) -> DrumPattern {
    let mut next_id = 100;
    instantiate_builtin(name, "Test", [0, 0, 0], &mut next_id)
        .unwrap_or_else(|| panic!("built-in {name:?} exists"))
}

#[test]
fn the_documented_vocabulary_is_the_real_one() {
    // The names promised in the tool description and the not-found
    // error must all resolve, or the docs are lying to the caller.
    let expected = [
        "halftime",
        "four-on-floor",
        "industrial",
        "breakbeat",
        "blast",
        "sparse",
        "toms",
        "build",
        "fill",
    ];
    assert_eq!(builtin_pattern_names(), expected);
    for name in expected {
        assert!(is_builtin_pattern(name), "{name} resolves");
        assert!(
            !build(name).groups.is_empty(),
            "{name} installs at least one group"
        );
    }
    // Every entry carries help text for the tool surface.
    for (name, description) in builtin_pattern_catalog() {
        assert!(!description.is_empty(), "{name} has a description");
    }
}

#[test]
fn names_match_trimmed_and_case_insensitively() {
    assert!(is_builtin_pattern("  HalfTime "));
    assert!(!is_builtin_pattern("halftimes"));
    assert!(!is_builtin_pattern(""));
}

#[test]
fn four_on_floor_puts_a_kick_on_every_beat() {
    let p = build("four-on-floor");
    assert_eq!(steps_for(&p, gm::KICK), vec![0, 4, 8, 12]);
    // Backbeat on 2 and 4.
    assert_eq!(steps_for(&p, gm::SNARE), vec![4, 12]);
}

#[test]
fn halftime_puts_the_backbeat_on_three() {
    let p = build("halftime");
    // One snare, on beat 3 — that is what makes it halftime.
    assert_eq!(steps_for(&p, gm::SNARE), vec![8]);
}

#[test]
fn sparse_is_actually_sparse_and_blast_is_not() {
    let sparse = build("sparse");
    let blast = build("blast");
    let count = |p: &DrumPattern| -> usize {
        p.groups
            .iter()
            .flat_map(|g| g.pads.iter())
            .map(|pad| pad.pattern.iter().filter(|v| **v > 0).count())
            .sum()
    };
    assert_eq!(count(&sparse), 2, "kick on 1, snare on 3");
    assert!(
        count(&blast) > 8 * count(&sparse),
        "blast is dramatically busier than sparse"
    );
}

#[test]
fn fill_lands_on_a_crash() {
    let p = build("fill");
    assert_eq!(steps_for(&p, gm::CRASH_16_EDGE), vec![0]);
    assert!(!steps_for(&p, gm::TOM_LOW).is_empty(), "toms carry the fill");
}

#[test]
fn an_unknown_name_is_none() {
    let mut next_id = 1;
    assert!(instantiate_builtin("gabber", "x", [0, 0, 0], &mut next_id).is_none());
    // A failed lookup must not consume ids.
    assert_eq!(next_id, 1);
}

#[test]
fn instantiating_allocates_unique_ids_and_advances_the_counter() {
    let mut next_id = 500;
    let a = instantiate_builtin("breakbeat", "A", [1, 2, 3], &mut next_id).unwrap();
    let b = instantiate_builtin("breakbeat", "B", [4, 5, 6], &mut next_id).unwrap();

    assert_ne!(a.id, b.id, "each instance is its own pattern");
    let a_groups: Vec<u64> = a.groups.iter().map(|g| g.id).collect();
    let b_groups: Vec<u64> = b.groups.iter().map(|g| g.id).collect();
    for id in &a_groups {
        assert!(!b_groups.contains(id), "group ids do not overlap");
    }
    assert!(next_id > 500);
    assert_eq!(a.name, "A");
    assert_eq!(a.color, [1, 2, 3]);
}

// ---------------- density ----------------

fn hit_count(p: &DrumPattern) -> usize {
    p.groups
        .iter()
        .flat_map(|g| g.pads.iter())
        .map(|pad| pad.pattern.iter().filter(|v| **v > 0).count())
        .sum()
}

#[test]
fn density_one_leaves_the_groove_exactly_as_authored() {
    let authored = build("breakbeat");
    let mut scaled = build("breakbeat");
    apply_density(&mut scaled, 1.0);
    for (a, b) in authored.groups.iter().zip(scaled.groups.iter()) {
        for (pa, pb) in a.pads.iter().zip(b.pads.iter()) {
            assert_eq!(pa.pattern, pb.pattern);
        }
    }
}

#[test]
fn lower_density_monotonically_thins_the_groove() {
    let counts: Vec<usize> = [0.2f32, 0.55, 0.8, 1.0]
        .into_iter()
        .map(|d| {
            let mut p = build("industrial");
            apply_density(&mut p, d);
            hit_count(&p)
        })
        .collect();
    for pair in counts.windows(2) {
        assert!(
            pair[0] <= pair[1],
            "density must not make a groove busier as it drops: {counts:?}"
        );
    }
    assert!(
        counts[0] < counts[3],
        "the lowest density is thinner than the authored groove: {counts:?}"
    );
}

/// The property that makes density usable for a build: thinning must
/// never silence a voice outright, or a quiet verse loses its kick
/// rather than getting a quieter one.
#[test]
fn thinning_never_silences_a_voice() {
    for name in builtin_pattern_names() {
        let authored = build(name);
        for d in [0.0f32, 0.1, 0.3, 0.5, 0.9] {
            let mut scaled = build(name);
            apply_density(&mut scaled, d);
            for (ga, gs) in authored.groups.iter().zip(scaled.groups.iter()) {
                for (pa, ps) in ga.pads.iter().zip(gs.pads.iter()) {
                    let had = pa.pattern.iter().any(|v| *v > 0);
                    let has = ps.pattern.iter().any(|v| *v > 0);
                    assert!(
                        !had || has,
                        "{name} at density {d}: pad {:?} lost every hit",
                        pa.name
                    );
                }
            }
        }
    }
}

/// Density keeps a proportion of each voice's hits, strongest beat
/// first — so a four-on-the-floor kick walks 1 → 2 → 3 → 4 hits across a
/// build rather than jumping between coarse tiers, and whatever survives
/// always includes the downbeat.
#[test]
fn density_scales_each_voice_proportionally_strongest_first() {
    let kicks_at = |d: f32| {
        let mut p = build("four-on-floor");
        apply_density(&mut p, d);
        steps_for(&p, gm::KICK)
    };

    assert_eq!(kicks_at(0.25), vec![0]);
    assert_eq!(kicks_at(0.5), vec![0, 8], "the half-bar beat outranks 2 and 4");
    assert_eq!(kicks_at(0.75), vec![0, 4, 8]);
    assert_eq!(kicks_at(1.0), vec![0, 4, 8, 12]);

    // The downbeat survives every level.
    for d in [0.05f32, 0.25, 0.5, 0.75] {
        assert!(kicks_at(d).contains(&0), "downbeat kept at density {d}");
    }
}

#[test]
fn density_is_clamped_rather_than_panicking() {
    for d in [-5.0f32, 0.0, 2.0, f32::INFINITY] {
        let mut p = build("toms");
        apply_density(&mut p, d);
        assert!(hit_count(&p) > 0, "density {d} leaves a playable pattern");
    }
}
