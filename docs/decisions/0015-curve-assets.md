# 0015. Curve assets and the Sample Curve node

**Context.** Materials need hand-shaped 1D functions (falloffs, ramps, easing) that are awkward to build from math
nodes. UE has Curve Float assets edited in a curve editor. Requirements: a new asset file kind holding points on
the unit square, its own editor tab, a `Sample Curve` node that picks any curve of the project and samples it with
`simple` or `catmull-rom` interpolation, curve data baked into WESL (no textures, no uniforms).

Constraints found while researching:
- WGSL has no generics: an array parameter has a fixed length, so a stdlib `sample(points: array<…, N>, t)` would
  need one maximum size for all curves (or a copy of the array per call).
- WESL (wesl 0.6, used by Bevy 0.20) supports renamed imports inside collections:
  `import a::b::{sample_curve_simple as x, …};` (`wgsl-parse` grammar, `ImportItem::rename`).
- Every asset kind so far is a node graph (`Asset`); `AssetSet`, `Doc`, undo and tabs assume it.

**Decision.**
- **File** `shaders/<path>.curve.ron`: `Curve(format_version: 1, name: "...", points: [(t, value), ...])`, both
  coordinates in `0..1`, sorted by `t` (normalized on load/save: clamped, non-finite points dropped, stable sort).
  Its own model type `shader_graph_core::curve::Curve` with its own `format_version` probe (not an `AssetKind` of
  `Asset`: a curve has no nodes, and a graph-shaped file would be noise). The file name wins over `name`, as for
  graphs. `AssetType::Curve` joins `AssetId`; `AssetSet` keeps curves in a separate map (`curves`), so code that
  walks graphs (`set.assets`) is unchanged.
- **Interpolation** (`curve::Interp`): `Simple` = piecewise linear; `CatmullRom` = cubic Hermite with Catmull-Rom
  tangents for non-uniform knots, `m_i = (v_{i+1} - v_{i-1}) / (t_{i+1} - t_{i-1})`, one-sided differences at the
  ends. Outside `[t_first, t_last]` the curve holds its end values; 0 points sample to 0, one point is constant.
  Catmull-Rom may overshoot `0..1` between points (it is not clamped). `Curve::sample` is the reference
  implementation; the editor preview draws it.
- **WESL**: each curve becomes its own module `autogen/curves/<path>.wesl` (written from the saved file, like
  functions): `const POINTS: array<vec3<f32>, N>` with `(t, value, slope)` per point (slopes precomputed in Rust),
  and two functions `sample_curve_simple(t)` / `sample_curve_catmullrom(t)` that find the segment by a loop over
  the const array and call the stdlib helpers `sg_curve_linear` / `sg_curve_hermite` (`stdlib/curve.wesl`). Both
  functions are always emitted, so editing a curve rewrites only its module: dependents keep their text and Bevy's
  hot reload recompiles them through the import. A material or function imports the one it uses with an alias,
  `sg_curve_<path with / → __>_<simple|catmullrom>`, so two curves can be used in one shader.
- **Node** `stdlib::SampleCurve` (Utility): parameter `curve` (`ParamKind::Curve`, an asset path stored as
  `Value::Str`, a select of the project's curves in Properties), parameter `type` (`Enum`: `Catmull-Rom` (default) /
  `Simple`), input `t` (Float, slider 0..1), output Float. No curve selected → E004; a missing or unloadable curve
  → E006 (`Env::curves` lists the loadable ones). The node title shows the curve name, and the node body a read-only
  preview of the curve (`ui::curve_preview`: a small tiny-skia plot in the node's `type`, from the open curve tab's
  unsaved points if there is one; a vertical line at `T` while that input is unconnected; "n/a" without a loadable
  curve). It is redrawn by its own system when that data changes, not by canvas rebuilds (*update, decision 0026:*
  now a kind of the generic `ui::node_preview::NodePreview` mechanism, `CurvePreview`). The IR records used curves
  (`Ir::curves`); `DepGraph` counts them as dependencies, so saving a curve also re-checks its users.
- **Paths**: `shaders/curves/` is reserved for materials (it would clash with the generated `autogen/curves/`),
  like `stdlib/` and `functions/` (superseded by 0020: material directories are emitted as `dir_curves/`, the
  reservation is gone). Renaming a curve rewrites the `curve` parameter of every node that uses it
  (`ops::rename`, open documents too); deleting it leaves E006 at its users.
- **Editor**: `Tab::Curve(CurveDoc)` / `ViewKind::Curve` (decision 0014 lists the steps). `CurveDoc` has the
  same model-level history as graphs (`history::History<T>`, extracted from `Doc`: 200 steps, merged drags).
  The view rasterizes grid and curve with tiny-skia into an `Image` (re-rendered on revision, preview mode or size
  change); points are absolutely positioned UI nodes, so picking, hover and drags come from `bevy_picking`.
  Interactions: click selects (Shift/Ctrl toggles; since decision 0016 Shift only), drag moves the selection (one undo step, `t` and value clamped
  to `0..1`, points may pass each other and are re-sorted), double click on empty space adds a point, RMB opens a
  menu (add / delete point). Hotkeys are the graph ones: Ctrl+Z/Y, Ctrl+S, Ctrl+C/X/V/D, Delete, Ctrl+A (select
  all, also on graphs), Ctrl+W. Pasted / duplicated points land 0.05 further in `t` (a copy on top of its
  original would be invisible but change the Catmull-Rom slopes). The `Simple` / `Catmull-Rom` switch in the view header (and, before decision 0016, Properties) only changes
  the preview (`CurveDoc::preview`, default Catmull-Rom); it is not stored in the file. Properties show the selected
  point's `t` / value as number fields.

**Consequences.** A third asset kind in the browser (`+Curve`, spline icon) and in `regen`. The generated curve
module is the only place the data lives; the generated Rust crate is unaffected. Very long curves cost a linear
segment search per sample (fine for the tens of points a hand-made curve has).
