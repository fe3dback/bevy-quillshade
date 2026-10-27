# Implementation decisions (spec §18)

Short records in the "context → decision → consequences" format. Numbering is sequential; a reverted decision is not deleted but marked "superseded by NNNN".

| № | Topic |
| --- | --- |
| [0001](0001-wesl-version.md) | wesl 0.6 instead of 0.4 |
| [0002](0002-relative-imports.md) | Imports between autogen modules: `super::`, no inlining (spike M0) |
| [0003](0003-file-format.md) | RON schema and migrations |
| [0004](0004-stdlib-storage.md) | Stdlib storage: Rust node descriptions + `include_str!` WESL modules |
| [0005](0005-builtins-as-arguments.md) | Builtin values in functions are hidden arguments |
| [0006](0006-world-position-offset.md) | World Position Offset in v1 |
| [0007](0007-typed-float-literals.md) | Scalar literals with the `f` suffix |
| [0008](0008-editor-architecture.md) | Editor architecture: the model is the source of truth, model-level undo |
| [0009](0009-user-config.md) | User config |
| [0010](0010-v1-limitations.md) | v1 limitations and additional diagnostic codes |
| [0011](0011-named-routes.md) | Named routes (UE-like named reroutes): usages reference the declaration by node id |
| [0012](0012-inline-pin-editors.md) | Inline value editors on nodes, RMB pin menu, polymorphic pin type |
| [0013](0013-ui-icons.md) | UI icons: icon-rs SVGs rasterized at runtime with resvg, own icons drawn by hand |
| [0014](0014-tab-content-kinds.md) | Tab content kinds (graph, read-only text) and syntax-highlighted generated files (the new-kind recipe superseded by 0031) |
| [0015](0015-curve-assets.md) | Curve assets: `*.curve.ron`, per-curve WESL modules, the Sample Curve node, the curve editor |
| [0016](0016-main-menu-and-ui-revision.md) | Main menu and per-view toolbar, browser double click / dirty dots, tab close menu, Ctrl+LMB disconnect, curve drag modifiers, one accent colour |
| [0017](0017-global-consts.md) | Global constant tables: `*.consts.ron`, the Global* nodes (values inlined as literals), the table editor, clearing references on type change / delete |
| [0018](0018-welcome-about-config-location.md) | Welcome screen, Help → About, New project, config in `<OS config dir>/fe3dback/bevy-shader-editor/`, compiled-in logos |
| [0019](0019-quick-start-workspace-bevy-compact-tabs.md) | Help → Quick Start, `bevy = { workspace = true }` in the generated crate when the game declares Bevy in its workspace, compact wrapping tabs with short names |
| [0020](0020-identifier-prefixes.md) | Prefixed identifiers in generated WESL and Rust (`inp_`, `out_`, `fun_`, `dir_`, `mat_`, `crv_`), shape-only user names, Custom/user-node inputs keep the keyword check |
| [0021](0021-node-context-actions.md) | Node context actions as data on `NodeDef`, the node RMB menu, hotkeys of selected nodes (`Shift + - * /`, numpad), Convert to between Add / Subtract / Multiply / Divide, `V` acting on release |
| [0022](0022-node-preview.md) | Node preview: `Asset::preview` (node + mode), `ShaderOutputDto` / `NodeDef::preview_output`, the previewed node's upstream joins the Fragment live set, W107 / W108; editor: `V` / node menu toggle, in-memory regeneration of the asset, restore from disk on discard |
| [0023](0023-material-attributes.md) | Material Attributes (PBR): struct pin type + stdlib `material` module, Make / Break, Lerp over attributes, Pbr3d `Wrapped` output mode, preview of struct fields, `WorldAlignedBlend`, node double-click opens the referenced function / curve / constant table |
| [0024](0024-texture-assets.md) | Texture assets (`*.texture.ron`, one game file — one asset, E011 / E012 / E013 / W109 / W110), `ParamKind::Texture`, fixed textures as per-node bindings `ft_<node>` and hidden function inputs `ft_<fn>_<node>`, internal textures in `assets/shaders/internal/` owned through `.autogen`, old image-path defaults fail to load |
| [0025](0025-baked-materials.md) | Baked materials: `baked.ron` manifest compiled in via `include_str!` + runtime overlay, `BakedMaterials` / `BakedTextures` / `BakedMaterialsLoader` / progress messages, `load()` fills `BakedTextures` at once, failures → `@internal/default`, what hot-reloads and what needs a rebuild, `new` / `set_*` crate-internal |
| [0026](0026-game-assets-tree.md) | Game Assets tree and the virtual Internal folder, droppable game files (`DroppableFile`), auto-created Texture assets, undo scope, image / Texture tabs and thumbnails, Convert to Parameter; editor refactor seams (`state/*`, `TabDoc`, `NodePreview`, tree sources, `NodeOp::Convert`) |
| [0027](0027-qa-parameter-order-auto-load.md) | Material parameters sorted by name (no Order field; old key dropped on load), `BakedPlugin` starts loading when the game does not call `load()`, `load()` returns the texture count, Quick Start as step tabs, texture thumbnails 128×128 beside the output pins (`NodePreview::BESIDE_OUTPUTS`) |
| [0028](0028-texture-tab-easy-mode.md) | Texture tab Easy (use-case presets Pixel art / UI / 3D / Ground, no form) / Advanced (the form), R \| G \| B \| All channel view in grey, one-row header, image info in Properties (now the right sidebar, decision 0031); "Default" sampler = Bevy's `ImagePlugin::default()` (`linear()`, Clamp to edge), which turning it off starts from |
| [0029](0029-layout-guard-cube-maps.md) | Hot reload layout guard (`Ir::layout_feature` → `SG_LAYOUT_<hash>` shader def from the generated `specialize`, WESL `@if(!…) const_assert …` so a changed layout fails to compile instead of crashing), texture bindings matched by position (`layout` / `bindings` in `baked.ron`), E014 for KTX2 cube maps / arrays / 3D textures, editor preview stacks their faces |
| [0030](0030-baked-handles-by-path.md) | `BakedMaterials::by_path` (`UntypedHandle`, typed `get::<M>(id)`) and `BakedTextures::by_path` / `get(id)` keyed by asset id (`env/rock_wet`, `textures/rock_albedo`), kept in sync on hot reload, runtime-only textures included; `by_path` reserved at the root |
| [0031](0031-editor-kind-areas.md) | Editor kinds own their window areas: `editors/<kind>.rs`, `TabDoc` + `EditorAreas` (topbar / right sidebar, per-kind sidebar title) + `StatusInfo`, `AreaOf` containers toggled by `Display`, one list of kinds (`for_each_kind`), the recipe for a new kind (supersedes 0014's) |
| [0032](0032-input-contexts.md) | Input contexts via bevy_enhanced_input (`MainCtx` + one context per editor kind, combinations beat plain keys, Ctrl/Alt/Super block plain keys, typing gate, bindings re-armed on activation); UE5 graph gestures: Alt+LMB breaks pin / wire, plain drag from a connected input starts a new wire, Ctrl+drag moves wires (`MoveWires`, all or nothing), more hold-key nodes; full before → after table |
| [0033](0033-graph-editor-qa.md) | Graph editor QA: 5-unit grid snap (lead node of a drag, new / pasted nodes; frame size not snapped), header hue from route / label / global-constant names, selection / preview outlines with the preview pulse, dimmer grid, Delete removes box-enclosed comment frames (full containment + all member nodes), `Frame.hue`, the stale "Show WESL" root cause, pin hover halo (larger hit area skipped), FunctionInput `min` / `max`, dropping Function / Curve assets (only direct self-calls rejected) |
