---
name: graphics
description: "Use for 2D/3D graphics, rendering, geometry, and procedural/geo modeling."
---

# Graphics & geometry

## Math & geometry
- Be explicit about coordinate systems, units, winding order, and handedness; document them.
- Watch numerical precision (prefer `f64` for geo/world-space; normalize vectors; guard against NaN).
- Build meshes with correct normals and indexed geometry; keep transforms composable (TRS).

## Rendering
- Separate scene/data from rendering; make geometry generation deterministic and testable.
- Mind performance: batch draw calls, reuse buffers, level-of-detail for large scenes.
- For geo/CAD work, track CRS/projection explicitly and cite data sources.
