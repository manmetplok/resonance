//! Update-flow coverage for the chord inspector's schema generator
//! wiring: the GENERATOR mode switch (Markov ↔ Schema), the SCHEMA /
//! ROTATION / SUBSTITUTION messages, and the live-regenerate behaviour
//! once the lane has been materialized.

use resonance_app::compose::messages::{ChordInspectorMsg, GeneratorKind};
use resonance_app::compose::ComposeMessage;
use resonance_app::message::Message;
use resonance_app::{demo, Resonance};
use resonance_music_theory::{Degree, GeneratorSpec, SchemaKind};

/// Build the demo app and return it with the first section's id.
fn build_app() -> (Resonance, u64) {
    let (mut app, _task) = Resonance::new();
    demo::seed_demo_content(&mut app);
    let def_id = app.compose_state().definitions[0].id;
    (app, def_id)
}

fn send(app: &mut Resonance, definition_id: u64, msg: ChordInspectorMsg) {
    let _ = app.update(Message::Compose(ComposeMessage::ChordInspector {
        definition_id,
        msg,
    }));
}

fn spec(app: &Resonance, definition_id: u64) -> GeneratorSpec {
    app.compose_state()
        .definitions
        .iter()
        .find(|d| d.id == definition_id)
        .expect("definition exists")
        .generator_spec
        .clone()
        .expect("generator spec set")
}

/// Switching to Schema keeps the staged chord count and defaults the
/// schema to Axis (rotation 0, no substitution); switching back to
/// Markov keeps the count again and lands on the "pop" table.
#[test]
fn generator_kind_switch_preserves_length() {
    let (mut app, def_id) = build_app();

    send(&mut app, def_id, ChordInspectorMsg::SetLength(6));
    send(
        &mut app,
        def_id,
        ChordInspectorMsg::SetGeneratorKind(GeneratorKind::Schema),
    );

    match spec(&app, def_id) {
        GeneratorSpec::Schema {
            schema,
            length,
            rotation,
            substitution,
        } => {
            assert_eq!(schema, SchemaKind::Axis);
            assert_eq!(length, 6);
            assert_eq!(rotation, 0);
            assert_eq!(substitution, 0.0);
        }
        other => panic!("expected Schema spec, got {other:?}"),
    }

    send(
        &mut app,
        def_id,
        ChordInspectorMsg::SetGeneratorKind(GeneratorKind::Markov),
    );
    match spec(&app, def_id) {
        GeneratorSpec::MarkovProgression {
            length,
            table_id,
            start,
            end,
            ..
        } => {
            assert_eq!(length, 6);
            assert_eq!(table_id, "pop");
            assert_eq!(start, None);
            assert_eq!(end, None);
        }
        other => panic!("expected Markov spec, got {other:?}"),
    }
}

/// Re-selecting the current mode must not clobber tuned fields.
#[test]
fn generator_kind_switch_is_idempotent() {
    let (mut app, def_id) = build_app();

    send(
        &mut app,
        def_id,
        ChordInspectorMsg::SetGeneratorKind(GeneratorKind::Schema),
    );
    send(
        &mut app,
        def_id,
        ChordInspectorMsg::SetSchemaKind(SchemaKind::DooWop),
    );
    send(&mut app, def_id, ChordInspectorMsg::SetSchemaRotation(2));
    send(
        &mut app,
        def_id,
        ChordInspectorMsg::SetSchemaSubstitution(0.4),
    );

    // Picking "Schema" again in the dropdown is a no-op.
    send(
        &mut app,
        def_id,
        ChordInspectorMsg::SetGeneratorKind(GeneratorKind::Schema),
    );
    match spec(&app, def_id) {
        GeneratorSpec::Schema {
            schema,
            rotation,
            substitution,
            ..
        } => {
            assert_eq!(schema, SchemaKind::DooWop);
            assert_eq!(rotation, 2);
            assert!((substitution - 0.4).abs() < 1e-6);
        }
        other => panic!("expected Schema spec, got {other:?}"),
    }
}

/// Picking a schema snaps the length to the schema's natural loop
/// length, resets the rotation, and syncs `generate_params.chord_count`.
#[test]
fn schema_pick_snaps_length_and_resets_rotation() {
    let (mut app, def_id) = build_app();

    send(
        &mut app,
        def_id,
        ChordInspectorMsg::SetGeneratorKind(GeneratorKind::Schema),
    );
    send(&mut app, def_id, ChordInspectorMsg::SetSchemaRotation(3));
    send(
        &mut app,
        def_id,
        ChordInspectorMsg::SetSchemaKind(SchemaKind::PlagalVamp),
    );

    match spec(&app, def_id) {
        GeneratorSpec::Schema {
            schema,
            length,
            rotation,
            ..
        } => {
            assert_eq!(schema, SchemaKind::PlagalVamp);
            assert_eq!(length, SchemaKind::PlagalVamp.default_length());
            assert_eq!(rotation, 0);
        }
        other => panic!("expected Schema spec, got {other:?}"),
    }
    let def = &app.compose_state().definitions[0];
    assert_eq!(
        def.generate_params.chord_count,
        SchemaKind::PlagalVamp.default_length() as u32
    );
}

/// Generate materializes the schema spec onto the lane; subsequent
/// rotation edits re-run the generator immediately (live preview), and
/// the rotated loop shows up in the generated degrees.
#[test]
fn schema_generate_and_live_rotation() {
    let (mut app, def_id) = build_app();

    send(
        &mut app,
        def_id,
        ChordInspectorMsg::SetGeneratorKind(GeneratorKind::Schema),
    );
    // Axis with the demo's staged chord count of 4 → I V vi IV.
    send(&mut app, def_id, ChordInspectorMsg::SetLength(4));
    send(&mut app, def_id, ChordInspectorMsg::Generate);

    {
        let def = &app.compose_state().definitions[0];
        assert!(app.compose_state().last_error.is_none());
        let material = def.generated_material.as_ref().expect("materialized");
        assert_eq!(material.chords.len(), 4);
        assert_eq!(material.chords[0].degree, Degree::I);
        assert_eq!(def.chords.len(), 4);
    }

    // Rotation 1 regenerates the lane without pressing Generate again:
    // the loop now opens on V.
    send(&mut app, def_id, ChordInspectorMsg::SetSchemaRotation(1));
    {
        let def = &app.compose_state().definitions[0];
        assert!(app.compose_state().last_error.is_none());
        let material = def.generated_material.as_ref().expect("still materialized");
        assert_eq!(material.chords[0].degree, Degree::V);
        assert_eq!(def.chords.len(), 4);
    }

    // Substitution edits clamp into 0..=1 and keep the lane generated.
    send(
        &mut app,
        def_id,
        ChordInspectorMsg::SetSchemaSubstitution(1.5),
    );
    match spec(&app, def_id) {
        GeneratorSpec::Schema { substitution, .. } => assert_eq!(substitution, 1.0),
        other => panic!("expected Schema spec, got {other:?}"),
    }
    assert!(app.compose_state().last_error.is_none());
    assert_eq!(app.compose_state().definitions[0].chords.len(), 4);
}
