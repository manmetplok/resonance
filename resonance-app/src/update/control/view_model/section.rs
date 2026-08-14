//! Arrangement projections: the placed sections, in song order.

use crate::Resonance;
use resonance_control::methods::song::SectionPlacementView;

/// Ordered section arrangement: placements sorted by start bar, names
/// denormalized from their definitions. App bars are 0-based; the wire
/// is 1-based.
pub(in crate::update::control) fn placement_views(app: &Resonance) -> Vec<SectionPlacementView> {
    let mut placements: Vec<&crate::compose::SectionPlacementState> =
        app.compose.placements.iter().collect();
    placements.sort_by_key(|p| p.start_bar);
    placements
        .into_iter()
        .filter_map(|p| {
            let def = app.compose.definitions.iter().find(|d| d.id == p.definition_id)?;
            Some(SectionPlacementView {
                id: p.id.into(),
                definition_id: def.id.into(),
                name: def.name.clone(),
                start_bar: p.start_bar + 1,
                length_bars: def.length_bars,
            })
        })
        .collect()
}

/// Definitions in the order the arrangement first plays them (each
/// definition once), then any unplaced definitions in creation order.
pub(in crate::update::control) fn definitions_in_placement_order(
    app: &Resonance,
) -> Vec<&crate::compose::SectionDefinitionState> {
    let mut seen = std::collections::HashSet::new();
    let mut ordered = Vec::new();
    for view in placement_views(app) {
        if seen.insert(u64::from(view.definition_id)) {
            if let Some(def) = app
                .compose
                .definitions
                .iter()
                .find(|d| d.id == u64::from(view.definition_id))
            {
                ordered.push(def);
            }
        }
    }
    for def in &app.compose.definitions {
        if seen.insert(def.id) {
            ordered.push(def);
        }
    }
    ordered
}
