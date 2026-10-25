# QA follow-ups: node context menus, math conversion, const labels, name prefixes, node preview

Date: 2026-10-25. Source: QA list from the user (node graph editor section). Implementation must follow
`CLAUDE.md` (decisions, README, regenerating `examples/sample_game`, snapshots).

## Intent

QA work in the editor surfaced four friction points:

1. **Debugging intermediate values is slow.** To see what a node (usually a `Lerp` blending b/w masks or
   colors/textures) actually outputs, the user drags its output into the material's albedo and disconnects
   normal/ORM/etc., then restores everything. We want a one-keystroke "preview this node in game" that does not
   touch the wiring.
2. **Small graph edits need rewiring.** Swapping `Subtract` for `Add` means deleting and recreating a node.
3. **Constants are anonymous.** A `Constant` node shows only its value; the user wants to name it ("rim width").
4. **Reserved words block natural names.** `type`, `loop`, `default`, `mesh`… are rejected as file/function/
   parameter names because they clash with Rust/WGSL. Instead, generated code should never use user names bare.

Underlying all of this: a generic node context menu where nodes contribute their own actions and hotkeys.

## Goals / non-goals / anti-goals

Goals:
- Node context menu (RMB click on a node) with per-node actions and shown hotkeys; hotkeys work on the selection.
- Math "Convert to" between Add / Subtract / Multiply / Divide, keeping wiring.
- Editor-only header label for constant nodes.
- Context-dependent prefixes on all user-space identifiers in generated WESL **and** Rust; reserved-word checks
  removed from name validation.
- Exclusive node preview with three modes (`unlit` / `lit` / `full`), persisted in the asset file.

Non-goals (this round):
- The "PBR DTO" pin type (a value carrying all PBR outputs). Only the hook for it is designed now (see Preview).
- Previewing a specific pin; preview always uses the node's main output.
- Preview in function graphs.
- Arbitrary names (uppercase, spaces, non-ASCII). Names stay `[a-z][a-z0-9_]*`; only the *reserved word* rule goes.

Anti-goals (would be a failure even if it works):
- Preview silently shipping: a graph left in preview must be loudly visible in the editor and in CLI output.
- Preview requiring a game rebuild. It must work through WESL hot-reload alone (see Preview constraints).
- Codegen becoming non-deterministic or snapshot churn without a decision record.

## 1. Node context menu

- RMB *click* (not pan; reuse `RightPress::is_click`) on a node body opens a menu via `dialog::open_icon_menu`.
  Today only input pins have a menu (`Editor::pin_menu`); empty canvas RMB opens the palette — unchanged.
- Menu items are described in core as data, not Bevy types, so `shader_graph_core` stays Bevy-free:
  something like `NodeAction { id, label, icon: Option<&str>/enum, hotkey: Option<Hotkey>, op: NodeOp }`, where
  `NodeOp` is a small closed enum the editor knows how to apply (`ConvertKind(String)`, `TogglePreview`, …).
  `NodeDef` gets a default-empty method (e.g. `fn context_actions(&self, node: &Node) -> Vec<NodeAction>`);
  the stdlib builder (`def(..)`) gets a way to attach them.
- Common items for every node are added by the editor: **Exclusive preview** (`V`) for material graphs, and
  existing generic actions (copy / duplicate / delete with their current hotkeys) — builder may choose which
  generic items to include, but preview must be there.
- The hotkey is displayed in the menu row (right-aligned or in the label; the `open_icon_menu` item may need a
  hotkey field).
- **Hotkeys from the menu work while the node is selected** (no menu open). Dispatch: on key press, collect
  actions of selected nodes whose hotkey matches.
  - Multi-selection: a `ConvertKind` hotkey applies to every selected node that offers it (one undo step).
    `V` acts only when exactly one node is selected. *(my call)*
  - Hotkeys are ignored while a text field has focus (follow how existing `shortcuts` avoids typing conflicts).
- **`V` conflict** *(my call)*: holding `V` + LMB on empty canvas currently drops `param::Vector`
  (`hotkey_node`). Preview therefore toggles on **release** of `V`, and only if no canvas click happened while
  it was held.

## 2. Math "Convert to"

- `stdlib::Add`, `stdlib::Subtract`, `stdlib::Multiply`, `stdlib::Divide` each offer "Convert to → the other
  three" with hotkeys `Shift +`, `Shift -`, `Shift *`, `Shift /`.
- Key mapping *(my call)*: accept both the main row (`Shift`+`Equal`, `Shift`+`Minus`, `Shift`+`Digit8`,
  `Shift`+`Slash`) and the numpad (`Shift`+`NumpadAdd/Subtract/Multiply/Divide`).
- Converting replaces `kind` in place: same node id, position, edges, inline input overrides and params (these
  four share A/B pins; any pin id that doesn't exist on the target is dropped with its edge). One undo step.
  Converting to the node's own kind is not offered.
- Check the actual stdlib ids/pins before implementing; if other binary math nodes share the A/B shape, the
  builder may extend the set, but the four above are required.

## 3. Const label

- Nodes: `stdlib::Constant`, `Constant2Vector`, `Constant3Vector`, `Constant4Vector` (the "const" family).
  `stdlib::Global*` nodes are **not** included — they already show the global's name.
- Properties panel shows a free-text "Label" field for these nodes. Non-empty label replaces the header title
  (default title when empty). Any text allowed (it is never emitted).
- Editor-only: stored in the `.material.ron` / `.function.ron` (so it survives reload) but ignored by IR and
  codegen — generated output must be byte-identical with or without a label.
- Storage shape is the builder's call (an optional `label` field on `Node` with `skip_serializing_if` is the
  natural fit; a param would leak into codegen). Old files must still load; new optional fields with
  `#[serde(default)]` don't need a `format_version` bump (decision 0003) — confirm and record.

## 4. Name prefixes and reserved words

Decided with the user:
- Name validation (`shader_graph_core::ident::check_ident`) keeps `[a-z][a-z0-9_]*`, but **drops the
  `RESERVED` list**. Reserved *prefixes* (`n_`, `b_`, `sg_`, `_`) can also be dropped because user names will
  always be prefixed in output; `__` / trailing `_` stay forbidden *(my call)*. Applies everywhere `check_ident`
  is used for asset names, directory components, material parameter names and function ports.
- **Exception (decided by the user):** input names of `stdlib::Custom` nodes and of user nodes
  (`shaders/.nodes/*.ron`) are referenced by bare name inside user-written WGSL (`return a * b;`). They keep
  the reserved-word check and stay **unprefixed** in output — current behaviour unchanged, no migration. So
  `check_ident` splits into a plain-shape check and a shape + reserved-words check (or equivalent).
- Generated code **always** prefixes user-space identifiers, statically (not only when they collide), with a
  prefix depending on the context. Same prefixes in WESL and Rust:

| What | Prefix | WESL example | Rust example |
|---|---|---|---|
| material parameter (uniform field, texture/sampler) | `inp_` | `params.inp_wetness`, `inp_albedo` texture | field `inp_wetness`, arg `new(inp_albedo, ..)`, `set_inp_wetness` |
| function name | `fun_` | `fn fun_wet_mask(..)` | — |
| function input port | `inp_` | `inp_strength` | — |
| function output port (result struct field) | `out_` | `.out_mask` | — |
| module / file / directory path component | builder's call (e.g. `mat_`, `dir_`) | `super::dir_common::fun_…` | `mod mat_rock_wet` |

- Rust type names (`RockWetMaterial`, `RockWetExt`, `RockWetParams`, function result structs `…Out`) are
  PascalCase with a suffix and cannot clash with keywords — keep them unprefixed. Root re-exports
  (`shaders::RockWetMaterial`) stay, so module prefixes are mostly invisible to game code.
- Whether generated *file names* (`autogen/env/rock_wet.wesl`, `SHADER_PATH`) get the prefix too is the
  builder's call; the requirement is that any user name — including `loop`, `mesh`, `self`, `super`, `crate`,
  `type` — produces WESL that compiles with wesl+naga and a Rust crate that passes clippy.
- Unprefixed Custom/user-node input names must not collide with the prefixed or generated names around them
  (they're locals/parameters of the generated helper function — check how `user.rs` / `utility.rs` emit them).
- Generated names the codegen itself introduces (`n_{id}_{pin}`, `b_*`, `sg_*`, `params`, `pbr_input`, …) must
  stay distinct from every prefix above.
- This is a **breaking change of the generated crate API**: regenerate `examples/sample_game`, update
  `examples/sample_game/src` for new field/arg/setter names, update README's generated-API section, record a
  decision. Snapshots will change wholesale (`INSTA_UPDATE=always`).
- Tests: add fixture(s) using keyword names (`type` param, `loop` material, `self` function, a `mesh` directory)
  that go through the full snapshot + WESL compile + Rust clippy path.

## 5. Exclusive node preview

### Behaviour
- Toggled from any node's context menu ("Exclusive preview") or with `V` on the selected node (see §1).
  Material graphs only; not offered in function graphs, and not on the MaterialOutput node itself.
- At most one node per graph is previewed. Enabling it on another node moves it there; toggling the previewed
  node turns preview off.
- Persisted in the material `.ron` so CLI `regen`/`check` produce exactly what the editor showed.
- **Applying it (decided by the user):** toggling preview (`V` / menu) or changing the preview mode is an
  ordinary document edit (undoable, marks the tab unsaved) **and** immediately regenerates *this asset's* WESL
  from the in-memory document, without saving. Today regen reads only `Editor::disk` after save, so this needs a
  new path: regenerate one asset from its in-memory state (including its other unsaved edits — accepted by the
  user), dependencies (functions, curves, consts) as currently loaded. Until the user saves, disk and generated
  WESL disagree and a CLI `regen` would undo the preview — accepted.
  - Turning preview off regenerates the same way (back to the normal output of the in-memory graph).
  - *(my call)* Closing the tab / discarding unsaved changes after such an in-memory regen regenerates that asset
    from disk again, so the generated WESL matches the file.
  - *(my call)* Other edits while preview is on do not auto-regenerate; they follow the usual `Ctrl+S` flow.
  - The in-memory regen must not touch the generated Rust (see Preview constraints) and goes through the same
    writer (atomic, marker check, write-only-if-changed).
- Material properties panel gets a select **Preview mode: `unlit` | `lit` | `full`**, also persisted.
  Default `unlit` *(my call — masks are the most common case)*. The mode is remembered even when no node is
  previewed.
- Previewed node is visually distinct on the canvas: thick/accent border and a warning icon in its header
  (exact look is the builder's call; must be obvious at normal zoom and in the minimap if cheap).
- While a preview is active the editor shows it (e.g. in the tab title or a banner) and validation emits a new
  warning code (next free `W1xx`) "preview active on node X (mode Y)" so `check`/`regen` print it and CI logs
  show it. It's a warning, not an error — regen still writes.
- Storage shape is the builder's call (e.g. on the material settings or `Meta`, `preview: Option<NodeId>` +
  `preview_mode`); deleting the previewed node clears it; a dangling id on load is ignored with a warning.

### Node → `ShaderOutputDto`
- New concept in core: `ShaderOutputDto` — a set of optional material-output slots (`albedo` and, for future
  use, the others matching `output::*` pins: metallic, roughness, normal, emissive, ORM-like, WPO, opacity…).
- `NodeDef` gets a default method that builds it from the node's emitted outputs: **default = first output,
  converted to the albedo type, into `albedo`; everything else empty**. Nodes may override to fill more slots.
- Future PBR DTO pin type: a node outputting it maps the whole value into the DTO 1:1. Not built now; the trait
  shape must allow it without redesign.
- Conversion of the main output to albedo uses the existing edge conversions
  (`shader_graph_core::types::conversion`): Float → `(v,v,v)`, Float2 → `(x,y,0)`, Float3 as is, Float4/Color →
  `.rgb` (with Float4 albedo for Sprite2d). Alpha is forced to 1 *(my call)*. Types with no conversion
  (textures, samplers, bool…) → the preview item is disabled with a tooltip.

### Codegen per mode
The previewed node's DTO replaces what the MaterialOutput node would provide:
- `full`: DTO used 1:1; empty slots get the material output defaults.
- `lit`: only `dto.albedo`, every other output default, generated with the material's normal (lit) template.
- `unlit`: only `dto.albedo`, simplest template — fragment returns `vec4(albedo, 1.0)`.
- Per kind:
  - Pbr3d: as above. `unlit` must be done **in WESL only** (e.g. bypass lighting in the extension fragment,
    or set the unlit flag in the shader) — not via `StandardMaterial::unlit` in Rust. Prepass/deferred variants
    must still compile.
  - Unlit3d: `lit` = `unlit` (no lighting exists); `full` uses color/opacity/WPO slots if present.
  - Sprite2d: all modes write `color = vec4(albedo, 1.0)` (Sprite has no lighting); `full` may use UV slot.
- Only the previewed node's upstream subgraph needs emitting; dead nodes may be dropped or kept — builder's call
  as long as output is deterministic and compiles.

### Preview constraints
- **The generated Rust crate must not change when preview is toggled** *(my call, follows from "look quickly in
  game")*: same types, same `Params` layout and bindings, same alpha mode. Only `.wesl` changes, so the running
  game picks it up via asset hot-reload without recompiling. Consequence: the uniform block and all
  texture/sampler bindings stay declared in preview WESL even if unused, and the `min_size` test still holds.
- Errors in the non-previewed part of the graph (e.g. a disconnected required pin elsewhere) should not block
  preview if they're not upstream of the previewed node — builder's call whether that's feasible this round.
- Snapshot coverage: a fixture with preview active in each mode × each material kind, compiled in all
  forward/deferred variants like other snapshots.

## Docs & bookkeeping

- New decisions (`docs/decisions/NNNN-*.md` + index): node context actions & hotkeys; identifier prefixes and
  dropped reserved words (supersedes the reserved-word part of identifier rules); node preview (modes, DTO,
  WESL-only constraint, storage).
- README: hotkeys (`V`, `Shift +-*/`, node RMB menu), file format additions, new diagnostic code, generated API
  names, removed reserved-word limitation.
- `docs/ui-checklist.md`: add manual checks for node menu, convert, label, preview highlight.
- Regenerate sample game and commit; sample game clippy must pass.

## Left to the builder
Exact data types and names for actions/DTO/storage; menu visuals and preview highlight look; module/file prefix
choice; whether generic copy/duplicate/delete appear in the node menu; dead-node handling in preview;
partial-error tolerance in preview; order of implementation (suggested: prefixes → context menu infra →
convert & label → preview).
