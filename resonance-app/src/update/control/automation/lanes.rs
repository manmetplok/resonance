//! `automation.lanes` — every lane, or a filtered subset. Read-only.

use super::edit::resolve_range;
use super::target::spec_owner;
use super::view::{lane_view, sorted_lanes, LaneContext, LaneInfo};
use crate::update::control::reply::{failure, success};
use crate::Resonance;
use resonance_control::methods::automation::{LanesParams, LanesResult};
use resonance_control::{Request, Response, RpcError};

/// List lanes in (owner, kind, parameter) order, orphans last. Every
/// filter field narrows; an orphan (no owner) only shows up unfiltered
/// by owner.
pub(super) fn lanes(app: &Resonance, request: &Request) -> Response {
    let params: LanesParams = match crate::update::control::optional_params(request) {
        Ok(p) => p,
        Err(e) => return failure(request, e),
    };
    match build(app, &params) {
        Ok(result) => success(request, &result),
        Err(e) => failure(request, e),
    }
}

fn build(app: &Resonance, params: &LanesParams) -> Result<LanesResult, RpcError> {
    let owner = spec_owner(app, &params.target)?;
    let window = params
        .range
        .as_ref()
        .map(|range| resolve_range(app, range))
        .transpose()?;
    let ctx = LaneContext::new(app);
    let lanes = sorted_lanes(app, &ctx)
        .into_iter()
        .filter(|(info, _)| owner.is_none_or(|owner| info.owner == Some(owner)))
        .filter(|(info, _)| matches_filter(info, params))
        .map(|(info, lane)| lane_view(app, &info, lane, window))
        .collect();
    Ok(LanesResult {
        revision: app.revision(),
        lanes,
    })
}

/// The non-owner filter fields: `control`, `plugin_id` / `occurrence`,
/// and `param` — matched the way `find_param` resolves one (name
/// case-insensitively, numeric id, first-party string key), plus a
/// device lane's own param id.
fn matches_filter(info: &LaneInfo, params: &LanesParams) -> bool {
    let spec = &params.target;
    let lane = &info.target;
    if spec.control.is_some() && lane.spec.control != spec.control {
        return false;
    }
    if spec.plugin_id.is_some() && lane.spec.plugin_id != spec.plugin_id {
        return false;
    }
    if spec.occurrence.is_some() && lane.spec.occurrence != spec.occurrence {
        return false;
    }
    if let Some(wanted) = spec.param.as_deref().map(str::trim) {
        let by_name = lane
            .param_name
            .as_deref()
            .or(lane.spec.param.as_deref())
            .is_some_and(|name| name.eq_ignore_ascii_case(wanted));
        let by_id = lane.param_id.is_some_and(|id| {
            wanted.parse::<u32>() == Ok(id) || resonance_plugin::stable_hash(wanted) == id
        });
        if !by_name && !by_id {
            return false;
        }
    }
    true
}
