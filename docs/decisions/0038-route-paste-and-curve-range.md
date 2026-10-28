# 0038. Pasting a Named Route declaration; curve output range

## Pasting a Named Route declaration

**Context.** Decision 0011 pasted a copied `stdlib::NamedRoute` declaration as a duplicate, like any node. A second
declaration of the same name in the same graph is never what the user wants: a declaration is the one place a value
enters the route, and the reason to copy it is to read that value somewhere else. UE5 users expect the copy to turn
into a usage.

**Decision.** `ClipboardData::source` records the graph (`AssetId`) the nodes were copied from. `Doc::paste`
(Ctrl+V, and Ctrl+D, which is copy + paste) pastes a copied declaration as a `stdlib::NamedRouteUsage` when:
- the target graph is the source graph, and
- the node with the original declaration's id still exists there and is still a route declaration.

The usage takes the declaration's pasted position (the usual paste offset and grid snap) and `route` = the original
declaration's id. Copied wires leaving the declaration's `out` leave the usage's `out` (both nodes are pass-throughs
with the same output pin); wires into the declaration are dropped, as a usage has no input. Usages copied together
with such a declaration keep pointing at the original. The paste stays one undo step and the pasted nodes are
selected.

**Fallback.** Into another graph, or after the original declaration was deleted (or its id now belongs to another
kind of node), a usage would dangle (E004). There the declaration is duplicated as before and usages copied with it
follow the copy (decision 0011).

**Consequences.** No format change. The usage kind is found by its role (`NodeRole::RouteUsage`), as the palette's
route entries do. `Doc::paste` takes the registry to read node roles.

## Curve output range

**Context.** Curve points live on the unit square (decision 0015), so a curve meant to output, say, `-2..10` had to be
followed by a remap in every graph that samples it, and its points were awkward to place in the editor. The user
asked for a min / max on the curve asset.

**Decision.** `Curve` gains an **output range** `min` / `max` (`f32`, defaults `0` / `1`). The points stay normalized
(`0..1`, clamped by `normalize`, which leaves the range alone); a point value `v` outputs `min + v·(max − min)`.
- **File.** Both fields are serde-defaulted and skipped at their default, so existing files load unchanged and re-save
  byte-identically; they follow `name` when set. No `format_version` bump (controller ruling): the fields are additive
  and opt-in, and an older editor ignores unknown fields (serde's default, as the `format_version` probe relies on)
  — it would sample such a curve in `0..1`.
  A non-finite bound or `min >= max` is a load error (`LoadError::Parse`, `curve::range_error`) → E009.
- **Reference math.** `Curve::sample` returns real units (0 points → `min`); `sample_normalized` (and
  `sample_normalized_with`) give the unscaled shape. Hermite and linear interpolation are linear in the values and the
  slopes, so scaling after interpolation equals interpolating scaled points; Catmull-Rom overshoot scales with the
  range.
- **Codegen.** The range is baked into the curve module's constants: values `min + v·(max − min)`, slopes
  `·(max − min)`, the 0- / 1-point constants likewise. At the default range these are the same `f32`s, so the
  generated WESL is byte-identical to before (no snapshot or sample-game file changed); dependents never see the
  range, and editing it rewrites only the curve's own module.
- **Editor.** The curve tab's right sidebar shows Min / Max number fields (`PropTarget::CurveRange`, a `Float2`; one
  undo step per edit, merged while scrubbing). An edit making `min >= max` is refused with a warning and changes
  nothing. The plot's value-axis labels and the selected point's Value field are in real units (the field converts to
  and from normalized; an unchanged value keeps the stored one exactly); drags, arrow nudges and the Shift grid snap stay
  in normalized space, and the in-node Sample Curve preview keeps drawing the normalized shape. The sidebar no longer
  repeats the point count, which the editor statusbar shows.
