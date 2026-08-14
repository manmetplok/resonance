//! Golden output pin for the Markov progression generator.
//!
//! `markov::generate` is documented as pure and deterministic, and its
//! output is persisted inside user projects: a change in sampling order
//! or in the number of RNG draws silently rewrites existing songs the
//! next time they are regenerated. Refactoring the generator must
//! therefore be output-preserving, and the plain
//! "same seed twice in one process" check in `tests/generator.rs`
//! cannot catch that — it re-runs the *new* code both times.
//!
//! This test pins concrete output for a matrix of tables, lengths,
//! orders, constraints and locks against values recorded from the
//! pre-refactor generator. If it fails, the generator's output changed:
//! that is a user-visible change to persisted material, never a test to
//! be re-blessed casually.

use resonance_music_theory::generator::degree::Degree;
use resonance_music_theory::generator::table::TableRegistry;
use resonance_music_theory::generator::{GenContext, Generator, GeneratorSpec};

/// One row of the matrix: a spec, a seed and the locked slots.
struct Case {
    name: &'static str,
    table: &'static str,
    length: u8,
    order: u8,
    start: Option<Degree>,
    end: Option<Degree>,
    seed: u64,
    locks: &'static [(usize, Degree)],
}

fn cases() -> Vec<Case> {
    let mut out = Vec::new();
    // Plain walks across every built-in table, several lengths/orders.
    for &table in &["pop", "modal", "jazz", "post-rock", "metal", "classical"] {
        for &(length, order, seed) in &[
            (1u8, 1u8, 1u64),
            (4, 1, 7),
            (8, 1, 42),
            (8, 2, 42),
            (8, 0, 42),
            (11, 3, 99),
            (16, 2, 2024),
        ] {
            out.push(Case {
                name: table,
                table,
                length,
                order,
                start: None,
                end: None,
                seed,
                locks: &[],
            });
        }
    }
    // Start / end constraints.
    out.push(Case {
        name: "start_I",
        table: "pop",
        length: 8,
        order: 1,
        start: Some(Degree::I),
        end: None,
        seed: 5,
        locks: &[],
    });
    out.push(Case {
        name: "end_I",
        table: "pop",
        length: 8,
        order: 1,
        start: None,
        end: Some(Degree::I),
        seed: 5,
        locks: &[],
    });
    out.push(Case {
        name: "start_end",
        table: "classical",
        length: 12,
        order: 2,
        start: Some(Degree::I),
        end: Some(Degree::I),
        seed: 11,
        locks: &[],
    });
    // Locks: single mid-phrase lock, lock on a split slot, dense locks.
    out.push(Case {
        name: "lock_mid",
        table: "pop",
        length: 8,
        order: 1,
        start: None,
        end: None,
        seed: 3,
        locks: &[(3, Degree::V)],
    });
    out.push(Case {
        name: "lock_split_slot",
        table: "pop",
        length: 8,
        order: 1,
        start: None,
        end: None,
        seed: 3,
        locks: &[(2, Degree::IV)],
    });
    out.push(Case {
        name: "lock_dense",
        table: "jazz",
        length: 8,
        order: 2,
        start: None,
        end: None,
        seed: 8,
        locks: &[(0, Degree::I), (2, Degree::II_MIN), (5, Degree::V)],
    });
    out.push(Case {
        name: "lock_and_end",
        table: "classical",
        length: 8,
        order: 1,
        start: Some(Degree::I),
        end: Some(Degree::I),
        seed: 21,
        locks: &[(4, Degree::IV)],
    });
    out
}

/// Render a case's output as one stable line.
fn render(case: &Case) -> String {
    let spec = GeneratorSpec::MarkovProgression {
        length: case.length,
        table_id: case.table.to_string(),
        order: case.order,
        start: case.start,
        end: case.end,
    };
    let mut locked: Vec<Option<Degree>> = vec![None; case.length as usize];
    for &(i, d) in case.locks {
        if i < locked.len() {
            locked[i] = Some(d);
        }
    }
    let reg = TableRegistry::with_builtins();
    let ctx = GenContext {
        registry: &reg,
        locked: &locked,
    };
    let key = format!(
        "{}/{}/{}/{}/s{}",
        case.name, case.length, case.order, case.seed, case.locks.len()
    );
    match spec.generate(case.seed, &ctx) {
        Ok(m) => {
            let chords: Vec<String> = m
                .chords
                .iter()
                .map(|c| {
                    format!(
                        "{}{}{}",
                        c.degree,
                        if c.degree.inversion > 0 {
                            format!("^{}", c.degree.inversion)
                        } else {
                            String::new()
                        },
                        if c.locked { "*" } else { "" }
                    )
                })
                .collect();
            let splits: Vec<String> = m
                .splits
                .iter()
                .map(|s| {
                    format!(
                        "{}:{}{}",
                        s.slot,
                        s.degree,
                        if s.degree.inversion > 0 {
                            format!("^{}", s.degree.inversion)
                        } else {
                            String::new()
                        }
                    )
                })
                .collect();
            format!("{key} = {} | {}", chords.join(" "), splits.join(" "))
                .trim_end()
                .to_string()
        }
        Err(e) => format!("{key} = ERR {e}"),
    }
}

fn actual() -> String {
    cases()
        .iter()
        .map(render)
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn markov_output_matches_golden() {
    let actual = actual();
    if std::env::var("RESONANCE_PRINT_MARKOV_GOLDEN").is_ok() {
        println!("---8<---\n{actual}\n--->8---");
    }
    assert_eq!(
        actual.trim(),
        GOLDEN.trim(),
        "markov generator output changed — this rewrites material already \
         persisted in user projects. Only update GOLDEN together with a \
         deliberate, documented change to the generator."
    );
}

/// Recorded from the generator before the phase extraction of ba todo
/// #1249. Regenerate with
/// `RESONANCE_PRINT_MARKOV_GOLDEN=1 cargo test -p resonance-music-theory
/// --test markov_golden -- --nocapture`.
const GOLDEN: &str = r#"
pop/1/1/1/s0 = I |
pop/4/1/7/s0 = I IV IV I64^2 | 2:ii 3:V
pop/8/1/42/s0 = I IV IV I64^2 I IV IV I64^2 | 2:ii6^1 3:V 6:ii 7:V
pop/8/2/42/s0 = I IV IV I64^2 I IV IV I64^2 | 2:ii6^1 3:V 6:ii 7:V
pop/8/0/42/s0 = I IV IV I64^2 I IV IV I64^2 | 2:ii6^1 3:V 6:ii 7:V
pop/11/3/99/s0 = I iii IV V vi IV IV V I IV V | 2:ii6^1 6:ii
pop/16/2/2024/s0 = I IV IV V I vi IV V I iii IV V I IV IV I64^2 | 2:ii 6:ii 10:ii6^1 14:ii 15:V
modal/1/1/1/s0 = I |
modal/4/1/7/s0 = I IV bVI bVII | 2:IV
modal/8/1/42/s0 = I IV bVI I64^2 I IV bVI bVII | 2:IV 3:V 6:IV
modal/8/2/42/s0 = I IV bVI I64^2 I IV bVI bVII | 2:IV 3:V 6:IV
modal/8/0/42/s0 = I IV IV I64^2 I IV IV bVII | 2:bVI 3:V 6:ii
modal/11/3/99/s0 = I iii ii V vi bVI IV V I IV bVII | 2:bVI 6:ii6^1
modal/16/2/2024/s0 = I IV IV I64^2 I bVI IV bVII I iii bVI V I IV bVI V | 2:ii 3:V 6:bVI 10:ii 14:IV
jazz/1/1/1/s0 = IΔ7 |
jazz/4/1/7/s0 = IΔ7 iii7 IVΔ7 I64^2 | 2:ii7 3:V7
jazz/8/1/42/s0 = IΔ7 IVΔ7 IVΔ7 I64^2 IΔ7 iii7 IVΔ7 I64^2 | 2:ii65^1 3:V7 6:ii7 7:V7
jazz/8/2/42/s0 = IΔ7 IVΔ7 IVΔ7 I64^2 IΔ7 IVΔ7 IVΔ7 I64^2 | 2:ii65^1 3:V7 6:ii7 7:V7
jazz/8/0/42/s0 = IΔ7 IVΔ7 IVΔ7 I64^2 IΔ7 iii7 IVΔ7 I64^2 | 2:ii65^1 3:V7 6:ii7 7:V7
jazz/11/3/99/s0 = IΔ7 ii7 IVΔ7 V7 vi7 IVΔ7 IVΔ7 V7 IΔ7 ii65^1 I64^2 | 2:ii65^1 6:ii7 10:V7
jazz/16/2/2024/s0 = IΔ7 IVΔ7 IVΔ7 V7 IΔ7 vi7 IVΔ7 V7 IΔ7 ii7 IVΔ7 V7 IΔ7 IVΔ7 IVΔ7 I64^2 | 2:ii7 6:ii7 10:ii65^1 14:ii7 15:V7
post-rock/1/1/1/s0 = I |
post-rock/4/1/7/s0 = I IV IV I64^2 | 2:ii 3:V
post-rock/8/1/42/s0 = I IV IV I64^2 I IV IV I64^2 | 2:ii6^1 3:V 6:ii 7:V
post-rock/8/2/42/s0 = I IV IV I64^2 I IV IV I64^2 | 2:ii6^1 3:V 6:ii 7:V
post-rock/8/0/42/s0 = I IV IV V I IV IV bVII | 2:ii6^1 6:ii
post-rock/11/3/99/s0 = I iii IV V vi IV IV V I IV bVII | 2:ii6^1 6:ii
post-rock/16/2/2024/s0 = I IV IV V I vi IV bVII I iii IV V I IV IV V | 2:ii 6:ii 10:ii 14:ii6^1
metal/1/1/1/s0 = i |
metal/4/1/7/s0 = i iv VI VII | 2:iv
metal/8/1/42/s0 = i VI iv i64^2 i iv VI VII | 2:VI 3:V 6:iv
metal/8/2/42/s0 = i VI iv i64^2 i iv VI VII | 2:VI 3:V 6:iv
metal/8/0/42/s0 = i iv VI i64^2 i iv VI VII | 2:iv 3:V 6:iv
metal/11/3/99/s0 = i III iv V i VI iv V i iv VII | 2:VI 6:VI
metal/16/2/2024/s0 = i iv iv V i VI VI VII i III VI V i iv VI i64^2 | 2:VI 6:iv 10:iv 14:iv 15:V
classical/1/1/1/s0 = I |
classical/4/1/7/s0 = I ii IV I64^2 | 2:ii 3:V
classical/8/1/42/s0 = I IV IV I64^2 I iii IV I64^2 | 2:ii6^1 3:V 6:ii 7:V
classical/8/2/42/s0 = I IV IV I64^2 I IV IV I64^2 | 2:ii6^1 3:V 6:ii 7:V
classical/8/0/42/s0 = I IV IV I64^2 I iii IV I64^2 | 2:ii6^1 3:V 6:ii 7:V
classical/11/3/99/s0 = I ii IV V vi IV IV V I ii vii° | 2:ii6^1 6:ii
classical/16/2/2024/s0 = I IV IV V I vi IV vii° I ii IV V I IV IV V | 2:ii 6:ii 10:ii 14:ii6^1
start_I/8/1/5/s0 = I ii IV I64^2 I vi IV V | 2:ii 3:V 6:ii
end_I/8/1/5/s0 = I IV IV I64^2 vi IV V I | 2:ii 3:V 5:ii
start_end/12/2/11/s0 = I ii IV V I IV IV V I IV I64^2 I | 2:ii6^1 6:ii6^1 9:ii6^1 10:V
lock_mid/8/1/3/s1 = I iii IV V* iii ii IV I64^2 | 2:ii 6:ii6^1 7:V
lock_split_slot/8/1/3/s1 = I iii IV* V iii ii IV V | 6:ii6^1
lock_dense/8/2/8/s3 = I* IΔ7 ii* I64^2 vi7 V* vi7 I64^2 | 3:V7 6:ii65^1 7:V7
lock_and_end/8/1/21/s1 = I I IV I64^2 IV* IV V I | 2:ii 3:V 5:ii
"#;
