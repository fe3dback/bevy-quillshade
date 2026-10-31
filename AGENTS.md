# AGENTS.md — Bevy Quillshade (`bevy-shader-editor` / `shader-editor`)

This document serves as the primary architectural guide, codebase map, glossary, and operational runbook for AI agents working on **Bevy Quillshade** (`bevy-shader-editor` workspace, binary name `shader-editor`).

---

## 1. What This Project Is & How It Works

**Bevy Quillshade** is an offline, multi-domain visual authoring tool and deterministic code generator for **Bevy 0.20**. Instead of interpreting graphs at runtime, the editor (and headless CLI) compiles human-readable RON source assets (`quillshade/<domain>/...`) directly into idiomatic, strongly-typed Rust crates and WESL shaders inside the target game's workspace (`crates/quillshade/`).

### Core Pipeline
1. **Source Authoring (`quillshade/<domain>/`)**:
   - **Material Domain (`quillshade/material/`)**: Node-based shader graphs (`.material.ron`, `.function.ron`), shared constants (`.consts.ron`), 1D transfer curves (`.curve.ron`), 1D OKLab gradients (`.gradients.ron`), and texture import policies (`.texture.ron`).
   - **Light Domain (`quillshade/light/`)**: Lighting, skybox, fog, and camera post-processing environments (`.environment.ron`), transition curves (`.curve.ron`), and cube-map texture policies (`.texture.ron`).
   - **Project Domain (`quillshade/project.ron`)**: Global project settings and cross-domain codegen orchestration.
2. **Validation & IR Lowering (`shader_graph_core`, `light_domain`)**:
   - Parses RON files, checks format versions and `snake_case` identifiers (`E001`–`E014`, `W101`–`W112`, `LE001`–`LE003`, `LE009`, `LE011`–`LE014`, `LW101`–`LW103`, `LW109`), runs topological sort and cycle detection, resolves port types, and packs scalar/vector uniforms into deterministic 16-byte-aligned GPU uniform structs (`UniformLayout`).
3. **Code Generation & Asset Baking (`shader_codegen_wesl`, `shader_codegen_rust`, `shader_project`, `light_domain`)**:
   - **WESL (`shader_codegen_wesl`)**: Emits `.wesl` vertex/fragment shaders (PBR `ExtendedMaterial<StandardMaterial, <Pascal>Ext>`, Unlit `Material`, or `MaterialExtension2d`) and reusable WESL function/curve modules (`autogen/functions/`, `autogen/curves/`), optionally verified against Bevy 0.20's `bevy_pbr` WESL headers via `wesl::Wesl::compile`.
   - **Rust (`shader_codegen_rust`)**: Emits Bevy `AsBindGroup` structs, builder methods, typed parameter handles, and the `baked` runtime loader module (`baked/mod.rs` + `baked.ron`), which hot-reloads scalar/color/texture parameters in-game **without** recompiling Rust or shaders unless the `SG_LAYOUT_<16-hex-hash>` structural guard changes.
   - **Light Runtime (`light_domain`)**: Emits `crates/quillshade/src/light/mod.rs` (`Light` `SystemParam`, `Preset`/`Curve`/`State` enums, `BlendedSkyboxPlugin` with embedded `blended_skybox.wesl`, and `EnvironmentValues` interpolation engine) and `assets/quillshade/light/autogen/baked.ron` for hot-reloading environments, curves, and cube-map texture settings.
   - **Baked Textures (`assets/quillshade/`)**: Bakes 1D gradients into a $128 \times 128$ sRGB RGBA8 PNG atlas (`assets/quillshade/material/gradients/<path>.png`) and writes internal fallback textures (`assets/quillshade/material/internal/`).
   - **Umbrella Crate (`crates/quillshade/`)**: `project_core::umbrella` assembles a single game-facing crate (`quillshade`) that re-exports `quillshade::material::*` and `quillshade::light::*` along with `QuillshadePlugin`.

---

## 2. Key Terms & Domain Glossary

| Term | Meaning & Where It Lives |
| :--- | :--- |
| **Domain (`EditorDomain` / `DomainBackend`)** | A vertical slice of functionality. Headless codegen implements `project_core::regen::DomainBackend`; GUI implements `editor_core::domain::EditorDomain`. Currently registered domains: `material`, `light`, and `project` (built-in). |
| **Umbrella Crate (`crates/quillshade`)** | The single generated crate inside the user's game workspace (`examples/sample_game/crates/quillshade`). Contains per-domain modules (`src/material/`, `src/light/`) and `QuillshadePlugin`. |
| **`Asset` (Material Graph)** | The unified AST in `shader_graph_core::model::Asset` representing either a `Material` (`AssetKind::Material(MaterialKind)`) or a `Function` (`AssetKind::Function`). Contains `nodes: BTreeMap<String, Node>`, `edges: Vec<Edge>`, and `comments: BTreeMap<String, CommentFrame>`. |
| **IR (`Ir`)** | Lowered, type-checked representation in `shader_graph_core::ir::Ir`. Produced by `build_ir`. Holds topologically sorted `Stmt`s, `UniformLayout`, texture bindings, and call dependencies. |
| **`UniformLayout` & `SG_LAYOUT_<hash>`** | Deterministic packing of material `Param` nodes (`vec4` $\rightarrow$ `vec3+pad` $\rightarrow$ `vec2` $\rightarrow$ scalars, padded to 16 bytes) at `@group(#{MATERIAL_BIND_GROUP}) @binding(100)`. Its FNV-1a 64-bit structural hash is embedded as `SG_LAYOUT_<16-hex>` in both `<slug>.rs` and `baked.ron`. Parameter value tweaks keep the hash identical (instant hot-reload via `baked.ron`); adding/removing/retyping params or changing pipeline state changes the hash. |
| **`NodeDef` & `Registry`** | Static or dynamic definition of a shader graph node (`shader_graph_core::registry::NodeDef`). Includes `stdlib::*` (built-in nodes in `shader_stdlib`), `param::*`, `output::*`, `function::<sub/name>` (project `.function.ron` graphs), and `user::<id>` (`quillshade/material/.nodes/*.ron`). |
| **`Editor` & `DomainState`** | Central editor state in `editor_core::state::editor::Editor`. Owns shell state (`tabs`, `config`, `messages`, `project`, `nav`) and a type-erased `DomainStates` (`BTreeMap<DomainId, Box<dyn DomainState>>`). |
| **`TabDoc` & `EditorAreas`** | Declarative traits registered by domains in `DomainRegistrar`. `TabDoc` (`editor_core::doc_kind::TabDoc`) defines how an open tab document behaves for save/undo/dirty/status. `EditorAreas` (`editor_core::editors::EditorAreas`) defines the `ViewKindId`, `EditorTopbar`, and `RightSidebar` scenes for an editor kind. |
| **Kit Crates (`curve_kit`, `texture_kit`)** | Reusable, domain-agnostic crates parameterized by a marker trait (`CurveDomain`, `TextureDomain`) so both `material` and `light` domains can share curve and texture data models, RON I/O, validation, and Bevy UI editors without colliding ECS components/resources. |
| **8 Shell Regions** | The fixed UI hierarchy spawned by `editor_core::ui::shell`: **mainbar**, **secondbar**, **left sidebar**, **editor topbar**, **editor content**, **editor statusbar**, **right sidebar**, and **console/logs**. |

---

## 3. Repository File Tree & Key Code Locations

```text
bevy-quillshade/
├── Cargo.toml                     # Workspace manifest (12 crates + root binary `shader-editor`)
├── flake.nix / .envrc             # Nix flake devShell (Rust, Vulkan loader, Wayland, libxkbcommon, ALSA)
├── AGENTS.md                      # Agent instructions & architectural reference
├── AUDIT.md                       # Technical audit findings & improvement candidates
├── src/
│   └── main.rs                    # Binary entry point: CLI arg parsing (`check`/`regen`/`diff` vs GUI launch),
│                                  # registers `MaterialDomain` and `LightDomain` into `editor_core::run`
├── examples/
│   └── sample_game/               # Standalone Bevy 0.20 game workspace consuming generated `crates/quillshade`
│       ├── quillshade/            # Authoring source files (`project.ron`, `material/**`, `light/**`)
│       ├── assets/                # Game textures (`assets/textures/**`) + generated `assets/quillshade/**`
│       └── crates/quillshade/     # Generated umbrella crate (`src/lib.rs`, `src/material/**`, `src/light/**`)
├── docs/
│   ├── techdoc.md                 # End-user & developer specification (`spec §N`)
│   ├── ui-checklist.md            # Pre-release manual UI verification checklist
│   └── decisions/                 # ADRs: 0043 (multi-domain layout), 0044 (domain API), 0045 (shell/tools)
├── vendor/
│   └── bevy_noodle/               # Vendored fork of `bevy_noodle` node-graph canvas widget (see FORK.md)
└── crates/
    ├── project_core/              # Foundation: project layout, RON I/O, identifiers, diagnostics, atomic writes, umbrella gen
    ├── curve_kit/                 # Generic 1D curves (`Curve`, evaluation, optional `editor` UI)
    ├── texture_kit/               # Generic texture policies & validation (`TextureAsset`, `TexturePolicy`, optional `editor` UI)
    ├── shader_graph_core/         # Material graph AST (`model.rs`), IR builder & type/layout checker (`ir.rs`), `Registry`
    ├── shader_stdlib/             # 70+ built-in shader nodes (`math`, `vector`, `color`, `coords`, `texture`, `scene`, `utility`, etc.) + user nodes
    ├── shader_codegen_wesl/       # Lowering `Ir` -> WESL shader code + `wesl` compilation verifier (`check.rs`)
    ├── shader_codegen_rust/       # Lowering `Ir` -> Rust `MaterialExtension`/`Material` structs + `baked/mod.rs` (`templates/baked_mod.rs`)
    ├── shader_project/            # Material domain headless pipeline (`MaterialBackend`), scoped incremental regen, fixtures
    ├── light_domain/              # Light domain backend (`src/backend/`) & optional `editor` UI (`src/editor/`)
    ├── editor_core/               # Domain-agnostic Bevy 0.20 GUI shell, ECS scheduling, tabs, undo history, menus, dialogs, theme
    ├── node_graph_base/           # Shared node-graph canvas glue over `bevy_noodle` (`GraphDomain`, minimap, palette, comment frames)
    └── material_domain/           # Material domain GUI (`MaterialDomain`, `MaterialState`, node graph ops, gradient editor, WESL/Rust preview)
```

### Detailed Module Map by Crate

#### 1. `crates/project_core` (Headless Foundation)
- [`lib.rs`](file:///home/neo/code/fe3dback/bevy-quillshade/crates/project_core/src/lib.rs) & [`regen.rs`](file:///home/neo/code/fe3dback/bevy-quillshade/crates/project_core/src/regen.rs): Defines `DomainBackend`, `DomainPlan`, `DomainOutput`, `OutFile`, `OutRoot`, `run`, `commit`, `write_umbrella`.
- [`layout.rs`](file:///home/neo/code/fe3dback/bevy-quillshade/crates/project_core/src/layout.rs): `Project` path resolver, `ProjectSettings` (`quillshade/project.ron`), `save_settings`, `workspace_problem`.
- [`umbrella.rs`](file:///home/neo/code/fe3dback/bevy-quillshade/crates/project_core/src/umbrella.rs): `CrateContribution`, generates `crates/quillshade/Cargo.toml` and `crates/quillshade/src/lib.rs` (`QuillshadePlugin`).
- [`writer.rs`](file:///home/neo/code/fe3dback/bevy-quillshade/crates/project_core/src/writer.rs): `WriteBatch` content-addressed staged file writer with temp-file rename and orphan cleanup.
- [`ident.rs`](file:///home/neo/code/fe3dback/bevy-quillshade/crates/project_core/src/ident.rs): Validates lower `snake_case` identifiers (`check_ident`, `check_path`, `to_snake_case`).
- [`ron_io.rs`](file:///home/neo/code/fe3dback/bevy-quillshade/crates/project_core/src/ron_io.rs): Version probing (`probe_version`) and `LoadError`.
- [`diag.rs`](file:///home/neo/code/fe3dback/bevy-quillshade/crates/project_core/src/diag.rs): Structured `Diagnostic` and `Level` (`Error`, `Warning`).

#### 2. `crates/curve_kit` & `crates/texture_kit` (Reusable Domain Kits)
- [`crates/curve_kit/src/lib.rs`](file:///home/neo/code/fe3dback/bevy-quillshade/crates/curve_kit/src/lib.rs): `Curve`, `Interp` (`Simple`, `CatmullRom`), `Curve::sample`, `range_error`.
- [`crates/curve_kit/src/editor/`](file:///home/neo/code/fe3dback/bevy-quillshade/crates/curve_kit/src/editor/mod.rs): Generic Bevy curve editor (`mod.rs`, `doc.rs`, `view.rs`, `sidebar.rs`, `ops.rs`, `input.rs`).
- [`crates/texture_kit/src/lib.rs`](file:///home/neo/code/fe3dback/bevy-quillshade/crates/texture_kit/src/lib.rs): `TextureAsset`, `TexturePolicy`, `TextureShape` (`Plain2d`, `CubeMap`), `TextureSettings`, `SamplerSettings`, `problems`, `read_ktx2_header`, `shape_problem`.
- [`crates/texture_kit/src/editor/`](file:///home/neo/code/fe3dback/bevy-quillshade/crates/texture_kit/src/editor/mod.rs): Generic texture policy & preview UI (`mod.rs`, `doc.rs`, `form.rs`, `image_tab.rs`, `preview.rs`, `cache.rs`, `ops.rs`).

#### 3. `crates/shader_graph_core` & `crates/shader_stdlib` (Material Graph Engine)
- [`crates/shader_graph_core/src/model.rs`](file:///home/neo/code/fe3dback/bevy-quillshade/crates/shader_graph_core/src/model.rs): `Asset`, `AssetKind`, `MaterialKind`, `Node`, `Edge`, `Value`, `CommentFrame`.
- [`crates/shader_graph_core/src/types.rs`](file:///home/neo/code/fe3dback/bevy-quillshade/crates/shader_graph_core/src/types.rs): `PinType`, implicit broadcast/conversion rules (`conversion`).
- [`crates/shader_graph_core/src/registry.rs`](file:///home/neo/code/fe3dback/bevy-quillshade/crates/shader_graph_core/src/registry.rs): `Registry`, `NodeDef`, `EmitCtx`, `ParamKind`, reference finders (`curve_refs`, `const_refs`, `texture_refs`, `gradient_refs`).
- [`crates/shader_graph_core/src/ir.rs`](file:///home/neo/code/fe3dback/bevy-quillshade/crates/shader_graph_core/src/ir.rs): `build_ir`, `Ir`, `Stmt`, `UniformLayout` packing, function signature resolution (`declared_function_sig`), diagnostic checks (`E001`–`E014`, `W101`–`W112`).
- [`crates/shader_graph_core/src/gradient.rs`](file:///home/neo/code/fe3dback/bevy-quillshade/crates/shader_graph_core/src/gradient.rs): `GradientCollection` (128 slots per `.gradients.ron`), OKLab interpolation (`Slot::sample`), $128 \times 128$ RGBA8 baker (`bake_rgba8`).
- [`crates/shader_stdlib/src/nodes/`](file:///home/neo/code/fe3dback/bevy-quillshade/crates/shader_stdlib/src/nodes/mod.rs): Built-in `stdlib::*` node catalog (`attributes.rs`, `color.rs`, `constants.rs`, `coords.rs`, `math.rs`, `output.rs`, `scene.rs`, `texture.rs`, `utility.rs`, `vector.rs`) and `user.rs` custom WGSL nodes.

#### 4. `crates/shader_codegen_wesl`, `crates/shader_codegen_rust`, `crates/shader_project` (Material Codegen)
- [`crates/shader_codegen_wesl/src/lib.rs`](file:///home/neo/code/fe3dback/bevy-quillshade/crates/shader_codegen_wesl/src/lib.rs): `emit_material_wesl`, `emit_function_wesl`, `emit_curve_wesl`. Handles PBR `ExtendedMaterial`, Unlit `Material`, `Sprite2d`, vertex displacement (`wpo`), prepass depth (`SceneDepth`, `DepthFade`), and gradient atlas sampling.
- [`crates/shader_codegen_wesl/src/check.rs`](file:///home/neo/code/fe3dback/bevy-quillshade/crates/shader_codegen_wesl/src/check.rs): `BevySources::discover` and WESL/naga compilation verifier against Bevy 0.20 shader headers.
- [`crates/shader_codegen_rust/src/lib.rs`](file:///home/neo/code/fe3dback/bevy-quillshade/crates/shader_codegen_rust/src/lib.rs): `emit_material_rs`, `emit_materials_mod`, `emit_material_domain_mod`.
- [`crates/shader_codegen_rust/src/baked.rs`](file:///home/neo/code/fe3dback/bevy-quillshade/crates/shader_codegen_rust/src/baked.rs), [`manifest.rs`](file:///home/neo/code/fe3dback/bevy-quillshade/crates/shader_codegen_rust/src/manifest.rs), & [`templates/baked_mod.rs`](file:///home/neo/code/fe3dback/bevy-quillshade/crates/shader_codegen_rust/templates/baked_mod.rs): Generates `crates/quillshade/src/material/baked/{mod,materials,textures}.rs` and `assets/quillshade/material/autogen/baked.ron`.
- [`crates/shader_project/src/regen.rs`](file:///home/neo/code/fe3dback/bevy-quillshade/crates/shader_project/src/regen.rs) & [`backend.rs`](file:///home/neo/code/fe3dback/bevy-quillshade/crates/shader_project/src/backend.rs): `plan_set`, `regenerate_set`, `Scope` (`Full`, `ManifestOnly`, `Affected`), `DepGraph`, and `MaterialBackend`.
- [`crates/shader_project/examples/make_sample.rs`](file:///home/neo/code/fe3dback/bevy-quillshade/crates/shader_project/examples/make_sample.rs): Writes the sample game's `quillshade/material/` fixtures and regenerates.

#### 5. `crates/light_domain` (Light & Environment Domain)
- [`crates/light_domain/src/backend/`](file:///home/neo/code/fe3dback/bevy-quillshade/crates/light_domain/src/backend/mod.rs): Bevy-free headless backend (`mod.rs`, `model.rs`, `codegen.rs`, `validate.rs`, `templates/light_mod.rs`, `templates/blended_skybox.wesl`). Defines `Environment` (`.environment.ron`), `LIGHT_TEXTURES`, `LightAssets`, `validate` (`LE001`–`LE003`, `LE009`, `LE011`–`LE014`, `LW101`–`LW103`, `LW109`), and `LightBackend` (emits `crates/quillshade/src/light/mod.rs` and `assets/quillshade/light/autogen/baked.ron`).
- [`crates/light_domain/src/editor/`](file:///home/neo/code/fe3dback/bevy-quillshade/crates/light_domain/src/editor/mod.rs): GUI domain implementation (`mod.rs`, `state.rs`, `curve.rs`, `environment.rs`, `form.rs`, `texture.rs`). Defines `LightDomain`, `LightState`, `LightCurve`, `EnvironmentDoc`, `EnvironmentEditor`, and `LightTexture`.
- [`crates/light_domain/examples/make_sample.rs`](file:///home/neo/code/fe3dback/bevy-quillshade/crates/light_domain/examples/make_sample.rs): Writes `examples/sample_game`'s `sky_cube.ktx2`, `sky.texture.ron`, `day.environment.ron`, `night.environment.ron`, and `smooth.curve.ron`.

#### 6. `crates/editor_core`, `crates/node_graph_base`, `crates/material_domain` (GUI Editor)
- [`crates/editor_core/src/app.rs`](file:///home/neo/code/fe3dback/bevy-quillshade/crates/editor_core/src/app.rs): `run` Bevy app builder, `EditorCorePlugin`, `apply_actions`, and shell `debug_screenshot` hook.
- [`crates/editor_core/src/domain/`](file:///home/neo/code/fe3dback/bevy-quillshade/crates/editor_core/src/domain/mod.rs): `EditorDomain`, `DomainRegistrar`, `DomainList`, `DomainState`, `ToolDef`, `ProjectDomain` (`mod.rs`, `list.rs`, `project.rs`, `registrar.rs`, `state.rs`, `tool.rs`).
- [`crates/editor_core/src/state/`](file:///home/neo/code/fe3dback/bevy-quillshade/crates/editor_core/src/state/mod.rs): `Editor` state (`editor.rs`), central dispatcher (`apply.rs`), asset operations (`assets.rs`), domain switching (`domain_switch.rs`), asset-kind defaults (`kinds.rs`), tab management (`tabs.rs`), file watcher (`watch.rs`).
- [`crates/editor_core/src/ui/`](file:///home/neo/code/fe3dback/bevy-quillshade/crates/editor_core/src/ui/mod.rs): Window shell (`shell.rs`), areas (`areas.rs`), project/assets browser (`browser.rs`, `tree.rs`), dialogs & menus (`dialog.rs`, `menu.rs`), icons (`icons.rs`), console/logs (`log.rs`), property widgets (`props.rs`), tabs (`tabs.rs`), text viewer (`text_view.rs`), welcome screen (`welcome.rs`), Feathers widgets (`widgets.rs`).
- [`crates/node_graph_base/src/`](file:///home/neo/code/fe3dback/bevy-quillshade/crates/node_graph_base/src/lib.rs): Shared node-graph canvas over `bevy_noodle` (`canvas.rs`, `domain.rs`, `edit.rs`, `editor.rs`, `input.rs`, `layout.rs`, `palette.rs`, `preview.rs`, `view.rs`).
- [`crates/material_domain/src/`](file:///home/neo/code/fe3dback/bevy-quillshade/crates/material_domain/src/lib.rs): `MaterialDomain` (`domain.rs`), `MaterialState` (`material_state.rs`), `MaterialGraph` (`graph_domain.rs`), documents (`doc.rs`, `consts_doc.rs`, `gradients_doc.rs`), kits (`curve.rs`, `texture.rs`), editors (`editors/{graph,consts,gradients}.rs`), operations (`ops/{apply,asset_ops,consts_ops,droppable,gradient_ops,graph_ops,nav,regen,tabs,types,watch}.rs`), and UI (`ui/`).

---

## 4. Editor Architecture & Icons

### Editor (`editor_core`, `node_graph_base`, kit editors, `material_domain`, `light_domain`; decisions 0044, 0045)

#### Window Layout (8 Regions)
Requests like "add X to the editor topbar" use these exact region names; use them in code comments, docs, and commit messages as well:

```text
┌──────────────────────────────────────────────────────────────────────────────┐
│ mainbar: logo, main menu │ domain tabs             project name, error count │
├──────────────────────────────────────────────────────────────────────────────┤
│ secondbar: main actions │ domain actions │ tabs (wrapping, grows by rows)    │
├────────────────┬───────────────────────────────────────┬─────────────────────┤
│ left sidebar   │ editor topbar                         │ right sidebar       │
│ ┌────────────┐ ├───────────────────────────────────────┤                     │
│ │project tree│ │ editor content                        │ editor kind         │
│ │            │ │                                       │ additional content  │
│ └────────────┘ │                                       │ (per editor kind)   │
│ ┌────────────┐ ├───────────────────────────────────────┤                     │
│ │assets tree │ │ editor statusbar                      │                     │
│ │            │ ├───────────────────────────────────────┤                     │
│ │            │ │ console/logs                          │                     │
│ └────────────┘ │                                       │                     │
└────────────────┴───────────────────────────────────────┴─────────────────────┘
```

- **mainbar** — global: logo, main menu (`File` / `Edit` / `View` / `Assets` / `Help`), then after a separator the **domain tabs** (one per real domain, name only, the active one brighter, disabled ones dimmed with a tooltip, the project domain button hidden visually, hidden while no project is open), then the project name and error count (all enabled domains); never buttons of one editor kind.
- **secondbar** — global, full window width under the mainbar: **main actions** (`Back` / `Forward`), **domain actions** (a reserved slot, empty and hidden for now), then the **tabs**: open tabs of every domain (other real domains' greyed, tool tabs blue), one row high, growing by whole rows; visually separate from the editor topbar below it.
- **left sidebar** — the active domain's (`ui::tree::left_panes`): its **project tree** (`quillshade/<domain>/**` assets, root label = the domain id, e.g. `material`) above its **assets tree** (the game's `assets/` without `assets/quillshade/`, filtered by the domain's Game Assets extensions, read-only, hidden for a domain without them; the virtual `Internal` folder ends the Material project tree); with the project domain active, its `Tools` list instead. The `Assets` menu shows the active domain's "New …" items, `New folder`, and `Generate`.
- **editor topbar** / **editor content** / **editor statusbar** / **right sidebar** — owned by the active tab's *editor kind* (Material's: `material.graph`, `material.curve`, `material.consts`, `material.image`, `material.texture`, `material.gradients`; Light's: `light.environment`, `light.curve`, `light.texture`, `light.image`; the project domain's: `project.settings`; core's: `text` — a `ViewKindId`): its buttons and info, its main view, its status line, and its additional content (properties of the selection, problems, hotkeys).
- **console/logs** — global, under the editor statusbar: diagnostics and messages of the active domain (plus global and project lines; the header's `"All domains"` checkbox shows every domain's).

#### Core Editor Concepts
- **Domains & Tools**: `src/main.rs`'s `domains()` lists every real `EditorDomain` (`MaterialDomain`, `LightDomain`); `DomainList::new` appends the built-in `ProjectDomain` (`DomainId::PROJECT`). A **tool** is a singleton tab without an asset (`DomainRegistrar::tool(ToolDef { id, title, open, menu })`), unique per `(domain, id)`, keyed `"@tool/<id>"` (never persisted to user config). Activating a project tool tab keeps `Editor::active_domain` unchanged (**overlay rule**).
- **Actions & Dispatch**: Widgets push `Action`s $\rightarrow$ `editor_core::apply_actions` $\rightarrow$ `Editor::apply` mutates documents $\rightarrow$ views redraw from the model. `Action::Tab(TabAction)` dispatches shared edit verbs (`Undo`, `Redo`, `Copy`, `Cut`, `Paste`, `Duplicate`, `DeleteSelection`, `SelectAll`, `CommitInline`), `SetValue`/`SetComponent` of a `PropKey`, and `Doc(Payload)` to the active tab's `TabDoc::apply`. Kit edits and property keys are domain-tagged (`CurveEdit<D>`, `TextureEdit<D>`, `PreviewChannel<D>`, `KitProp<D, P>`).
- **Feathers Widgets & Input Contexts**: UI is pure Bevy UI + Feathers (`bsn!`); **no egui**. Panels use `pane_box`/`subpane_box`, button groups use `button_group`, checkboxes use `widgets::checkbox`, and numbers use `FeathersNumberInput` (`props::number_ranged` / `number_input`). Hotkeys use `bevy_enhanced_input` with `MainCtx` (priority 2) and per-kind contexts (`GraphCtx<G>`, `CurveCtx<D>`, `TextCtx`, `GradientsCtx`, priority 1) synchronized to the active tab by `sync_input_context` and gated while typing by `gate_keyboard`.

### Icons (`IconId`, `IconRef`, `icon-rs`)

Icons are SVGs from the `icon-rs` crate (pinned `=0.4.5`, browse at https://rvdende.github.io/icon-rs/) plus custom hand-drawn SVGs, rasterized at runtime with `resvg` ([`crates/editor_core/src/ui/icons.rs`](file:///home/neo/code/fe3dback/bevy-quillshade/crates/editor_core/src/ui/icons.rs)). Never add PNG icons, `ImageNode`s with asset paths, or Feathers' `icon()` for UI icons.

- **`IconId` vs `IconRef`**: The icon set is open. `IconRef` is a handle to any `&'static Icon` (compared by name, which must be unique across all sets since the rasterizer cache is keyed by name). Core's chrome icons are the `IconId` enum (`IconRef::from(IconId::X)`); a domain's icons are `IconRef` constants next to its code (Material: [`crates/material_domain/src/icons.rs`](file:///home/neo/code/fe3dback/bevy-quillshade/crates/material_domain/src/icons.rs), `MATERIAL_ICONS`; Light: `LIGHT_ICONS`; kits: `CURVE_ICON`, `TEXTURE_ICON`). Asset kinds, templates, domains, tree rows, and menu entries carry an `IconRef`.
- **Usage in UI**: Use `@icon(IconId::X, 16.0, TEXT)` (accepts any `impl Into<IconRef>`) inside `bsn!` (`16.0` px is the UI standard; the color tints line/glyph icons and sets only the alpha of solid/sketch icons). Ready-made helpers in `editor_core::ui`: `icon_tool_button(id, label, action, tip)`, icon-only `icon_button(id, action, tip)` (always provide a tooltip naming the action + hotkey), and `dialog::open_icon_menu` with `(Option<IconRef>, label, action)` items.
- **Adding an `icon-rs` Icon**: A core chrome icon is an `IconId` variant mapped in `IconId::source()` to the constant (e.g. `icon_rs::FOLDER`, never `icon_rs::get("…")`) and appended to `IconId::ALL`; a domain icon is `pub const X: IconRef = IconRef::new(&icon_rs::Y)` added to the domain's icon list and verified by `assert_icons_rasterize`.
- **Custom SVG Rules**: Place `name.svg` in `crates/editor_core/icons/` (core) or `crates/material_domain/icons/` (Material):
  - `24x24` `viewBox`, `fill="none" stroke="#262626" stroke-linejoin="round" stroke-linecap="round"`, `stroke-width` `1.75` for line icons / `1.1` for solid ones, drawing inside `1.5..22.5`.
  - Colors **only** from `Palette::LIGHT`: ink `#262626`; faces `#ffffff`, `#d4d4d4`, `#a4a4a4`, `#747474` (solid only); accent `#2563eb` (rendered in `theme::ACCENT`) for the single modifier detail (e.g. plus or arrow; single marks stay ink-only). No `id`s, gradients, or `<defs>`.
  - Register via `const X: Icon = custom("name", Kind::Line, include_str!("../../icons/name.svg"));` and add it to the crate's `custom_icons_use_only_palette_colours` test list.

---

## 5. Building, Testing, Linting & Running (in `nono` Sandbox)

### Sandbox Environment Notes (`nono`)
- **Git Config**: `~/.gitconfig` and `~/.config/git` are blocked by `nono`. Prefix git commands with:
  ```bash
  XDG_CONFIG_HOME=/tmp GIT_CONFIG_GLOBAL=/dev/null git status
  ```
- **Editor User Config**: `~/.config/fe3dback/bevy-shader-editor` is outside the sandbox write allowlist. Pass `XDG_CONFIG_HOME=/tmp/qs-config` when running the GUI binary to avoid permission warnings and keep test runs isolated.
- **Nix Runtime Libraries & Software Vulkan (Mesa Lavapipe)**:
  GUI runs (`shader-editor`, UI tests, `sample_game`) require `LD_LIBRARY_PATH` (for `libudev`, `libasound`, `libvulkan`, `libxkbcommon`, `libwayland-client`) and `VK_ICD_FILENAMES` (pointing to Mesa's `lvp_icd.x86_64.json` software rasterizer in `/nix/store` if hardware GPU is unavailable or headless determinism is needed):
  ```bash
  export LD_LIBRARY_PATH="/nix/store/cgzr5afjdbx30j7g44z2h92i8h3828k2-systemd-minimal-libs-262/lib:/nix/store/9rpg951mvn34kcnp82c4mfvnr5ssxblf-alsa-lib-1.2.16.1/lib:/nix/store/la62431p5qf1ggr3rdzql9dksv5fhicc-vulkan-loader-1.4.363.0/lib:/nix/store/mp9krn4yrm1hwh657y1aa2jv0x97fbin-libxkbcommon-1.13.2/lib:/nix/store/8vz293r6dqfci9p6m955yziz2agxx990-wayland-1.26.0/lib"
  export VK_ICD_FILENAMES="/nix/store/v2isk5r5fzlriczi14ckn20hf45079di-mesa-26.1.2/share/vulkan/icd.d/lvp_icd.x86_64.json"
  export XDG_CONFIG_HOME="/tmp/qs-config"
  ```
  *(If `/nix/store` hashes change after a flake update, inspect `.direnv/flake-profile-*.rc` for `LD_LIBRARY_PATH` and run `find /nix/store -maxdepth 4 -name "lvp_icd*.json"` for `VK_ICD_FILENAMES`.)*

### Verification Commands (Must Pass Before Finishing Work)
```bash
# 1. Formatting check
cargo fmt --all --check

# 2. Workspace Clippy (strict -D warnings)
cargo clippy --workspace --all-targets -- -D warnings

# 3. Feature-gating check for headless kit/domain crates
cargo clippy -p curve_kit -p texture_kit -p light_domain --no-default-features --all-targets -- -D warnings

# 4. Full test suite (unit, integration, WESL compile, snapshot, and headless UI tests)
cargo test --workspace --no-fail-fast

# 5. Headless CLI verification against sample_game (including WESL shader compilation against Bevy 0.20)
cargo run -- check examples/sample_game --compile
cargo run -- diff examples/sample_game

# 6. Sample game workspace verification (uses separate target dir)
cd examples/sample_game
CARGO_TARGET_DIR=/home/neo/code/fe3dback/bevy-quillshade/target/sample cargo clippy --workspace --all-targets -- -D warnings
CARGO_TARGET_DIR=/home/neo/code/fe3dback/bevy-quillshade/target/sample cargo test --workspace
```

---

## 6. How to Take Screenshots in the Editor & Sample Game

Both `shader-editor` and `examples/sample_game` have built-in, deterministic screenshot and scripted-input pipelines controlled via environment variables.

### A. Editor Screenshots (`shader-editor`) — Frame-Exact Timeline

Setting `SHADER_EDITOR_SCREENSHOT=/abs/path/to/out.png` activates the frame-driven automation pipeline in [`crates/editor_core/src/app.rs`](file:///home/neo/code/fe3dback/bevy-quillshade/crates/editor_core/src/app.rs#L190-L247), [`crates/editor_core/src/demo.rs`](file:///home/neo/code/fe3dback/bevy-quillshade/crates/editor_core/src/demo.rs#L28-L215), [`crates/material_domain/src/lib.rs`](file:///home/neo/code/fe3dback/bevy-quillshade/crates/material_domain/src/lib.rs#L113-L217), and [`crates/material_domain/src/demo.rs`](file:///home/neo/code/fe3dback/bevy-quillshade/crates/material_domain/src/demo.rs#L138-L540):

| Render Frame | What Happens (`Local<u32>` frame counter in `Update`) | Controlling Env Vars |
| :--- | :--- | :--- |
| **Frame 2** | Opens the requested file/tab, selects the target node/frame/gradient stop, and switches the active domain. | `SHADER_EDITOR_OPEN`, `SHADER_EDITOR_SELECT`, `SHADER_EDITOR_DOMAIN` |
| **Frame 140** | Applies custom canvas zoom (`CanvasView.zoom`) before any demo/input runs (`START - 20`). | `SHADER_EDITOR_ZOOM=<f32>` (e.g. `1.5`, `0.5`) |
| **Frame 150** | Opens modal dialogs, dropdown menus, quick-start guides, or context popups. | `SHADER_EDITOR_DEMO=<mode>` |
| **Frames 160–200** | Builds a scripted sequence of winit `CursorMoved` / `MouseButtonInput` / `KeyboardInput` / `MouseWheel` steps on **Frame 160** (`START`) and replays 1 step per frame. Logs final state at **Frame 200** (`START + 40`). | `SHADER_EDITOR_INPUT=<gesture>`, `SHADER_EDITOR_INPUT_*` |
| **Frame 240** | Spawns `Screenshot::primary_window()` and observes `save_to_disk(path)`. | `SHADER_EDITOR_SCREENSHOT=<path.png>` |
| **Frame 300** | Emits `AppExit::Success` to cleanly terminate the editor process. | *(automatic when `SHADER_EDITOR_SCREENSHOT` is set)* |

#### Supported Environment Variables for `shader-editor`
- **`SHADER_EDITOR_SCREENSHOT=/path/to/output.png`**: Enables the pipeline, captures at frame 240, exits at frame 300.
- **`SHADER_EDITOR_DOMAIN=<material|light|project>`**: Switches domain at frame 2.
  - *Tip for opening a specific Light asset on startup*: Seed `$XDG_CONFIG_HOME/fe3dback/bevy-shader-editor/config.ron` before launching:
    ```ron
    (
        recent: [],
        last_project: Some("/home/neo/code/fe3dback/bevy-quillshade/examples/sample_game"),
        open: ["light:quillshade/light/day.environment.ron"],
        active_tab: Some("light:quillshade/light/day.environment.ron"),
        active_domain: Some("light"),
    )
    ```
- **`SHADER_EDITOR_OPEN=<spec>`** (Material domain, frame 2):
  - `<rel_path>.material.ron` or `<rel_path>.function.ron` — opens graph view (e.g. `env/rock_wet.material.ron`).
  - `<rel_path>#wesl` or `<rel_path>#rust` — opens generated WESL or Rust code preview tab.
  - `common/terrain.gradients.ron` — opens gradient editor.
  - `common/palette.consts.ron` or `common/pulse.curve.ron` — opens consts or curve editor.
  - `@game/textures/rock_albedo.ktx2` — opens game texture preview & policy inspector.
  - `@internal/default` — opens internal texture preview.
- **`SHADER_EDITOR_SELECT=<spec>`** (Material domain, frame 2):
  - Node kind substring (e.g. `stdlib::Lerp`, `Parameter`, `Output`) — selects the first matching node and centers the canvas on it.
  - Comment frame ID (e.g. `c01`) — selects the comment frame.
  - Gradient selection `<slot>[:s<stop>|:m<segment>]` (e.g. `1:s2` or `0:m0`) — selects gradient slot and stop/midpoint.
- **`SHADER_EDITOR_DEMO=<mode>`** (Frame 150):
  - Shell demos: `about`, `quick_start` (or `quick_start:2`), `open_project`, `welcome`, `menu` or `menu:<name>` (`file`, `edit`, `view`, `assets`, `help`), `tab_menu`, `settings_dirty`.
  - Material demos: `palette`, `wire`, `dialog`, `texture_channel:0`..`4`, `gradient_color`, `game_file_menu`, `game_file_menu:2`.
- **`SHADER_EDITOR_INPUT=<gesture>`** (Frames 160–200):
  - Shell gestures: `menu_click`, `menu_close`, `hover`, `tab[:N]`.
  - Material gestures: `drag`, `rmb_click`, `rmb_pan`, `mmb_pan`, `slider`, `color`, `pin_rmb`, `node_rmb`, `node_dblclick`, `frame_dblclick`, `hover`, `curve_drag`, `curve_add`, `texture_zoom`, `drop_game_file`, `ctrl_drag_wire`, `ctrl_wire_hold`, `wire_drag`, `wire_hold`, `alt_click`, `box_delete`, `box_delete_partial`, `type_field`.
- **Auxiliary `SHADER_EDITOR_INPUT_*` Environment Variables**:
  - **`SHADER_EDITOR_INPUT_NODE=<id>`**: Target node ID for `drag`, `node_rmb`, `node_dblclick`, or drop target for `drop_game_file` (default: canvas center / first unselected node).
  - **`SHADER_EDITOR_INPUT_PIN=<node>.<pin>`**: Target or source pin for `hover`, `pin_rmb`, `alt_click`, `wire_drag`, `wire_hold`, `ctrl_drag_wire`, and `ctrl_wire_hold` (default: topmost connected input).
  - **`SHADER_EDITOR_INPUT_TO=<node>.<pin>`**: Destination pin for `wire_drag`, `wire_hold`, `ctrl_drag_wire`, and `ctrl_wire_hold` (default: nearest pin of the same direction on another node).
  - **`SHADER_EDITOR_INPUT_AT=<spec>`**:
    - `header` (with `node_dblclick`): Double-clicks the node's header strip (opens referenced file, rename dialog, or node comment dialog).
    - `<dx>,<dy>` (with wire drags): Ends the drag that many logical px from the start pin on empty canvas.
    - `to:<dx>,<dy>` (with wire drags): Ends the drag that many logical px from the target pin (tests snap radius).
  - **`SHADER_EDITOR_INPUT_FILE=<path>`**: Source tree row for `drop_game_file` — either a Game Assets path under `assets/` or a Project asset path `quillshade/material/<path>.<kind>.ron` (e.g. `quillshade/material/common/pulse.curve.ron`).
  - **`SHADER_EDITOR_INPUT_TIP=<prefix>`**: Target widget for `hover` whose tooltip starts with `<prefix>` (default without `PIN` or `TIP`: the secondbar's Back button).
  - **`SHADER_EDITOR_INPUT_FRAME=<id>`**: Target comment frame ID for `frame_dblclick`, `box_delete`, and `box_delete_partial` (default: first comment frame).
  - **`SHADER_EDITOR_INPUT_MENU=<name>`**: Target mainbar menu (`file`, `edit`, `view`, `assets`, `help`) for `menu_click` and `menu_close` (default: `file`).
  - **`SHADER_EDITOR_INPUT_TRACE=1`**: Enables verbose per-step trace logging during scripted input playback.

#### Example One-Liner to Capture an Editor Screenshot
```bash
LD_LIBRARY_PATH="/nix/store/cgzr5afjdbx30j7g44z2h92i8h3828k2-systemd-minimal-libs-262/lib:/nix/store/9rpg951mvn34kcnp82c4mfvnr5ssxblf-alsa-lib-1.2.16.1/lib:/nix/store/la62431p5qf1ggr3rdzql9dksv5fhicc-vulkan-loader-1.4.363.0/lib:/nix/store/mp9krn4yrm1hwh657y1aa2jv0x97fbin-libxkbcommon-1.13.2/lib:/nix/store/8vz293r6dqfci9p6m955yziz2agxx990-wayland-1.26.0/lib" \
VK_ICD_FILENAMES="/nix/store/v2isk5r5fzlriczi14ckn20hf45079di-mesa-26.1.2/share/vulkan/icd.d/lvp_icd.x86_64.json" \
XDG_CONFIG_HOME="/tmp/qs-config" \
SHADER_EDITOR_SCREENSHOT="/tmp/editor_shot.png" \
SHADER_EDITOR_OPEN="env/rock_wet.material.ron" \
SHADER_EDITOR_SELECT="stdlib::Lerp" \
cargo run -- --width 1280 --height 720 examples/sample_game
```

#### Before / After Visual Regression Diff Workflow (`magick compare`)
Whenever modifying a module that affects editor views, widgets, canvas rendering, curves, textures, gradients, or domains:
1. **Before the step**: Capture baseline screenshot(s) (`before_<module>.png`) for the affected view(s).
2. **After the step**: Capture post-change screenshot(s) (`after_<module>.png`) with the exact same env vars and window size.
3. **Compare with ImageMagick**:
   ```bash
   magick compare -metric AE before_<module>.png after_<module>.png diff_<module>.png
   ```
   - If the output is `0` (0 differing pixels), visual parity is confirmed.
   - If the output is `> 0`, inspect `before_<module>.png`, `after_<module>.png`, and `diff_<module>.png` using the `read_file` image viewer tool to determine what changed and whether the difference is expected or a regression.

### B. Sample Game Screenshots (`examples/sample_game`) — Real-Time Trigger
Implemented in [`examples/sample_game/src/main.rs`](file:///home/neo/code/fe3dback/bevy-quillshade/examples/sample_game/src/main.rs#L25-L87):
- Setting `SAMPLE_SCREENSHOT=/tmp/sample_out` switches the window to `PresentMode::AutoNoVsync` + `WinitSettings::continuous()`, captures `/tmp/sample_out-1.png` at **`+5.0s`** real elapsed time and `/tmp/sample_out-2.png` at **`+25.0s`** real elapsed time, and exits automatically once the second screenshot finishes writing to disk.

---

## 7. Project Conventions, Workflow & Guardrails

1. **English-Only Code & Documentation**:
   - **All** code comments, docstrings (`///`, `//!`), diagnostic messages, commit messages, and **all Markdown documentation files (`.md`) in the repository** (`README.md`, `AGENTS.md`, `AUDIT.md`, `docs/**/*.md`, `vendor/bevy_noodle/FORK.md`) **must be written in English only**.
2. **Development Workflow (Sub-Agent Driven Flow)**:
   - **Dedicated Git Branch**: Perform multi-step feature or refactoring work in a dedicated git branch.
   - **Sequential Execution**: Execute implementation steps strictly sequentially (only read-only research subagents may run in parallel).
   - **Sub-Agent Driven Implementation & Code Review**: Delegate implementation stages to subagents (`invoke_subagent`), and have the main/lead agent perform a thorough code review (`git diff`, `cargo fmt --all --check`, `cargo clippy`, `cargo test`) after each stage before committing and moving to the next stage.
3. **Design Traceability**: Reference ADRs (`docs/decisions/NNNN-*.md`) or spec sections (`spec §N`) in module-level docstrings when touching architectural seams. Keep `README.md` up to date when user-facing behavior changes.
4. **Never Hand-Edit Generated Files**:
   - Everything under `examples/sample_game/crates/quillshade/**`, `examples/sample_game/assets/quillshade/**`, and `crates/shader_project/tests/snapshots/*.snap` is generated. Modify the generator crates (`project_core`, `shader_codegen_*`, `shader_project`, `light_domain`) and regenerate (`cargo run -- regen examples/sample_game`).
5. **Vendored `bevy_noodle`**:
   - If modifying `vendor/bevy_noodle`, document the patch and rationale in [`vendor/bevy_noodle/FORK.md`](file:///home/neo/code/fe3dback/bevy-quillshade/vendor/bevy_noodle/FORK.md).
6. **Determinism**:
   - Use `BTreeMap` / `BTreeSet` (never `HashMap` / `HashSet` iteration order) in any code path that feeds RON serialization, IR construction, `UniformLayout`, WESL emission, Rust codegen, or `baked.ron`.
