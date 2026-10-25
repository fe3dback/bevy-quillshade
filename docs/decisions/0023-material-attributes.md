# 0023. Material Attributes (PBR), `Wrapped` output mode, WorldAlignedBlend, node double-click

**Context.** QA follow-up (`docs/superpowers/specs/2026-10-26-material-attributes-design.md`): a landscape material
blends several layers (grass, rock, …), each with its own albedo / roughness / normal / … chain. Without a struct
pin every blend needs one `Lerp` per channel and five or more parallel wires. UE solves this with Material
Attributes; the user wants the same, plus UE's `WorldAlignedBlend` to pick layers by slope, and a double-click on a
node that refers to another file to open it.

**Decision.**
- **Type.** `PinType::MaterialAttributesPbr`, UI label `MaterialAttributes (PBR)`, a purple pin color of its own. It
  maps to the WESL struct `MaterialAttributesPbr` in the stdlib module `material` (`crates/shader_stdlib/wesl/
  material.wesl`, always written to `autogen/stdlib/material.wesl`). Field metadata (id, label, type, default, which
  fields are active for given `PbrSettings`) lives in `shader_graph_core::attributes`, shared by the stdlib nodes,
  the output node and `build_ir`. Fields, in order: `base_color` (1,1,1), `metallic` 0, `specular` 0.5, `roughness`
  0.5, `emissive` (0,0,0), `opacity` 1, `opacity_mask` 1, `normal` (0,0,1) (tangent space), `ambient_occlusion` 1. It
  is not numeric: conversions are identity only, so every polymorphic math node refuses it (E003), and so do `If` /
  `BoolSelect` (WGSL `select` cannot take a struct).
- **World Position Offset is not a field.** UE compiles every attribute separately and traces only WPO for the
  vertex shader. Our functions are separate WESL modules (decision 0002): computing the struct inside a function in
  the vertex stage would run the whole function there, including fragment-only `textureSample`. WPO stays a separate
  input of the output node; for a landscape it is per material, not per layer.
- **No literal of its own.** `Value::zero` of the type is `Str("")`; `wesl_literal` of any value as this type is the
  constructor call with every default (`attributes::pbr_default_literal`). There is no inline editor, no "Input
  defaults" row and no struct type for Custom nodes, user nodes (`shaders/.nodes/*.ron`, rejected with a message
  naming the type) or Parameter nodes. `FunctionInput` / `FunctionOutput` list the type; an attributes function input
  is optional and defaults to the constructor call.
- **Imports.** A module that names the struct must import it. The import (`attributes::import()`, i.e.
  `("material", "MaterialAttributesPbr")` in `EmitSink::stdlib`) is registered at exactly three points: in
  `EmitCtx::define_typed` when the `let` type is the struct (Make, Lerp, a function call returning attributes); in
  `build_ir` step 6 when an unconnected input of the struct type gets the default constructor (Break, Lerp, a function
  call argument); and in `build_ir` step 7 for a function whose own signature has an attributes input or output.
- **Nodes.** Category `Material Attributes`: `stdlib::MakeMaterialAttributesPbr` (one input per field, the fixed
  defaults above, output `attributes`) and `stdlib::BreakMaterialAttributesPbr` (input `attributes`, one output per
  field, `{v}.{field}`). Make's defaults are fixed values, **not** the game's `StandardMaterial` values: a layer is
  fully defined (UE semantics). They work in every material kind and in functions.
- **Lerp.** `stdlib::Lerp` resolves to the struct when `a` or `b` is fed by it: `a`, `b`, `out` become
  `MaterialAttributesPbr`, `alpha` is a scalar `Float` (a vector alpha truncates with W104), and the emit calls
  `sg_lerp_attributes_pbr` (per-field `mix`; the normal is not renormalized, the material output normalizes it). The
  other side must be attributes too or unconnected (= defaults); a vector there is E003. No new keyword or alias. In
  the editor, `PinDef::attributes` (`accepts_attributes()`) marks the polymorphic inputs that also take the struct;
  `doc::connection_error` looks at the node's other such inputs (the target pin itself is excluded, because a new edge
  replaces it): a struct source is refused when another one carries a number, a numeric source when another one
  carries attributes. `search::compatible_pin` offers Lerp `a` for a dropped attributes wire.
- **`Wrapped` output mode (Pbr3d only).** `PbrSettings::output: PbrOutputMode { Default, Wrapped }`, serialized as
  `output: Wrapped` and skipped when `Default`, so existing files stay byte-identical and `FORMAT_VERSION` stays 1
  (same precedent as `Asset::preview`, decision 0022). In `Wrapped`, `output::Pbr3d` has two inputs: `attributes`
  ("Material Attributes") and World Position Offset; pin ids of the material output node therefore depend on
  material settings, not only on params (`NodeDef::signature` doc). The emit expands a connected struct into the same
  per-field output keys as `Default` (`base_color`, `metallic`, …), only for fields active under the current Shading
  and Blend (`attributes::pbr_field_active`), so the WESL material templates and the preview are unchanged. An
  unconnected `attributes` writes nothing, like `Default` mode with nothing connected (`StandardMaterial` values
  stay). With `attributes` connected every active field is written, so `StandardMaterial`'s base color, metallic and
  roughness are overridden. The normal is always written (the default (0,0,1) is the identity under the TBN) under
  the existing `VERTEX_TANGENTS` guard, so **W103 "Normal requires mesh tangents" is not reported in `Wrapped`** —
  it would always fire. In `Default` the pins are as before; their inactive flags now come from
  `pbr_field_active` too.
- **Switching modes in the editor.** `PropTarget::OutputMode`, a select `Default` / `Wrapped` labelled "Output" after
  Shading. `Editor::set_pbr_output_mode` (like `convert_nodes`) recomputes the signature of every material output
  node and drops edges and inline values of pins that no longer exist, all in one `Doc::edit`: one undo step brings
  them back. No auto-rewiring (e.g. inserting a Make node) is done.
- **Preview** (extends decision 0022). `ShaderOutputDto::slots` maps a slot to `PreviewSource { pin, field }`. A node
  whose first output is the struct (Make, a Lerp of attributes, a function call returning attributes, reroutes, …)
  maps every slot to its field (`PreviewSlot::attributes_field`): Albedo → `base_color`, …; `build_ir` then reads
  `{expr}.{field}` with the field's type. The modes keep their meaning: `unlit` / `lit` show `base_color` as albedo,
  `full` maps every slot except Opacity. A root in `Wrapped` mode has no per-field pins; there `full` writes the
  fields that `pbr_field_active` allows.
- **`stdlib::WorldAlignedBlend`** (category Utility, keywords slope / angle / world aligned / terrain). Inputs Blend
  Sharpness (10), Blend Bias (-0.7), Normal (implicit: `Builtin::WorldNormal`, the Fresnel pattern), Up ((0,1,0),
  Bevy is Y-up). With `d = dot(normalize(N), normalize(Up))`: `Mask = saturate((d + bias) * sharpness + 1)` — 1
  where `d ≥ -bias`, falling to 0 over `1 / sharpness`; with the defaults slopes up to ~45° are 1 and beyond ~53°
  are 0 — and `Angle = degrees(acos(clamp(d, -1, 1)))` (0 = facing up, 90 = wall), via `sg_world_aligned_blend` /
  `sg_slope_angle` in `shading.wesl`. UE's exact formula is not public; this one is **an approximation** consistent
  with how UE's Sharpness and Bias are reported to behave (bias 0 → every upward surface, bias -2 → none). UE's "w/
  Pixel Normals" output is omitted: our normal map is a material output, not a builtin; wire the perturbed normal
  into `Normal` instead. The node normalizes its inputs itself, so it also works in the vertex stage.
- **Double-click opens referenced files.** `Editor::referenced_asset(node)` is the single lookup: a
  `function::<path>` node → that function (if it exists); otherwise the first `ParamKind::Curve` param naming a
  loadable curve; otherwise the first `ParamKind::GlobalConst` param whose constant some table defines → that table.
  `user::*` nodes are not editor assets (no tab kind) and get nothing; neither do unset params or dangling targets.
  A primary double-click (`PointerClick.count >= 2`) on a node body — not on a pin or an inline editor — pushes
  `Action::OpenAsset`, which activates an existing tab or opens a new one. The Properties panel's single "Open
  function" / "Open curve" / "Open constant table" button uses the same lookup. Debug hook:
  `SHADER_EDITOR_INPUT=node_dblclick`.

**Consequences.**
- Function boundaries pass the whole struct by value; no per-field slicing (decision 0002 holds).
- A Make with nothing connected and a `Wrapped` material fed by it overrides the game's `StandardMaterial` values,
  unlike `Default` mode where unconnected pins keep them — documented in README.
- `WorldAlignedBlend` always defines both outputs; an unused `Angle` is a dead `let` (naga drops it).
- The stdlib module list grows by `material`, so every project gets `autogen/stdlib/material.wesl`.
