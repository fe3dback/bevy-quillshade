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
| [0014](0014-tab-content-kinds.md) | Tab content kinds (graph, read-only text) and syntax-highlighted generated files |
| [0015](0015-curve-assets.md) | Curve assets: `*.curve.ron`, per-curve WESL modules, the Sample Curve node, the curve editor |
| [0016](0016-main-menu-and-ui-revision.md) | Main menu and per-view toolbar, browser double click / dirty dots, tab close menu, Ctrl+LMB disconnect, curve drag modifiers, one accent colour |
| [0017](0017-global-consts.md) | Global constant tables: `*.consts.ron`, the Global* nodes (values inlined as literals), the table editor, clearing references on type change / delete |
