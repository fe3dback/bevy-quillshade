# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`shader-editor`: a node-based shader editor for **Bevy 0.20**, modelled on UE5's Material Editor, grown into a
multi-domain content tool (domains `material` and `light`, plus the built-in `project` domain). Graphs live as RON
files in a game project (`quillshade/material/**`); the tool generates WESL (`assets/quillshade/material/autogen/**`)
and a Rust crate (`crates/quillshade`) with material types, baked material / texture handles and `QuillshadePlugin`.
Light environments (`quillshade/light/**`) generate `crates/quillshade/src/light/`. There is no in-editor material
preview — the game picks up regenerated `.wesl` and the baked manifest via Bevy's asset hot-reload. One binary:
no args → GUI; `regen` / `check` / `diff` → headless CLI.

Source of requirements: `docs/techdoc.md` (the spec; code comments cite it as `spec §<section>`). Deviations
from the spec and implementation choices are recorded in `docs/decisions/NNNN-*.md` — read the relevant one before
changing codegen, file format or editor architecture. Decisions below 0043 were removed in commit 195c3ae and live
only in git history: `git show 195c3ae^:docs/decisions/<file>` (`git show --stat 195c3ae` lists the file names).
Manual pre-release UI checklist: `docs/ui-checklist.md`.

## Environment

- Toolchain comes from the nix dev shell (`flake.nix`, `.envrc`): Rust ≥ 1.97.1 (MSRV, required by bevy_noodle),
  `kache` as rustc wrapper. `rust-toolchain.toml.disabled` is intentionally disabled. **Never run `nix` commands** —
  use plain `cargo`.
- `bevy/dynamic_linking` is the default feature `dynamic_linking` of the root package, `editor_core`, `node_graph_base`
  and `material_domain` (their workspace dependencies have `default-features = false`, so the root feature decides; the
  `dynamic_linking` of the kits and `light_domain`, e.g. `curve_kit`'s, is not a default and only the root feature turns
  it on): start such binaries through `cargo run` (it puts `libbevy_dylib` on the path). A standalone binary needs
  `cargo build --release --no-default-features` (what the release job builds).
- GUI windows need the Bevy runtime libs (libxkbcommon, wayland, vulkan-loader) on `LD_LIBRARY_PATH` — the nix shell
  provides them; software Vulkan (lavapipe) is fine for screenshots.

## Commands

```sh
cargo test --workspace                                   # everything: unit, pipeline, snapshots + WESL, UI smoke
cargo test -p shader_graph_core <test_name>              # single unit test in a crate
cargo test -p shader_project --test pipeline <test_name> # single integration test
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check                                  # rustfmt.toml: max_width = 120

cargo run -- examples/sample_game                        # GUI
cargo run -- --width 1280 --height 720 examples/sample_game  # GUI at a fixed window size (logical px, not saved)
cargo run -- check examples/sample_game --compile        # headless validation + wesl/naga compile
cargo run -- regen examples/sample_game                  # regenerate; prints create/modify/delete lines
cargo run -- diff  examples/sample_game
cargo run -- regen examples/sample_game --domain material  # only the named domain(s); repeatable, none = all
cargo run -- check examples/sample_game --domain light     # Light only (`project` is not a CLI domain: exit 2)

INSTA_UPDATE=always cargo test -p shader_project --test snapshots  # accept intended codegen changes
```

CLI exit codes: `0` ok, `1` validation errors, `2` project failed to open / write error. Diagnostics go to stderr as
`path:node[.pin]:CODE: text` (path like `quillshade/material/…`). `--domain <id>` (regen / check / diff, repeatable)
limits the run to those domains; an unknown id exits 2 with `unknown domain` and writes nothing; the umbrella files are
written regardless.

CI (`.github/workflows/ci.yml`): job `lint` (Linux: fmt, clippy, clippy of `curve_kit` + `texture_kit` + `light_domain`
with `--no-default-features` — the workspace build unifies features and so never builds the kits' Bevy-free models as
the backends use them —, clippy and `cargo test` of the sample's generated crate) and job
`test` (Linux/Windows/macOS: tests, `check --compile`, then `regen examples/sample_game`, failing on `git diff` or a
new untracked file, `git status --porcelain`) — **any codegen change must be followed by regenerating the sample game
and committing the result** (new files included), then
`cd examples/sample_game && cargo clippy --workspace --all-targets -- -D warnings` must pass on the generated crate.
Only master saves the `Swatinem/rust-cache` caches (branches and PRs restore them). Doctests need the toolchain's
`rustc --print target-libdir` on the loader path under `dynamic_linking` (cargo adds it for test binaries, not for
rustdoc). Pushing a semver tag (`0.3.0` / `v0.3.0`; `-suffix` = prerelease) also runs `dist` (release binaries without
`dynamic_linking` for Linux x86_64, Windows x86_64, macOS arm64 / x86_64; `SHADER_EDITOR_VERSION` = the tag without `v`
becomes `EDITOR_VERSION`) and `release` (`gh release create` with `GITHUB_TOKEN`, generated notes).

`cargo run -p shader_project --example make_sample -- examples/sample_game` rewrites the sample's
`quillshade/material/` from the fixture graphs in `crates/shader_project/src/fixtures.rs` and regenerates — it
overwrites hand edits to fixture-named assets, so check `git status` first.
`cargo run -p light_domain --example make_sample -- examples/sample_game` writes the sample's Light sources
(`quillshade/light/{sky.texture.ron, day.environment.ron}`) and the cube map `assets/textures/sky_cube.ktx2`; it does
not regenerate (a Light-only run would drop Material from the umbrella crate), so run `regen` after it. Its test
`sample_files_are_up_to_date` checks the committed files.

## Architecture

Cargo workspace; root package is only the `shader-editor` binary (`src/main.rs`, clap). `examples/sample_game` is a
**separate workspace** (excluded from the root one) — build it from its own directory, ideally with
`CARGO_TARGET_DIR=<repo>/target/sample`.
`vendor/bevy_noodle` is a local fork of bevy_noodle 0.5.0 (workspace member, path dependency, same crate name,
decision 0034): canvas / gesture behaviour (drags, snapping, wire gestures, port looks) goes **into the fork**, never
into app-side workarounds; every fork change is appended to its `FORK.md` ("Changes from upstream", what and why), is
off by default and keeps upstream's tests passing, so an upgrade is: vendor the new release, re-apply the `FORK.md`
entries. The fork's own strict `clippy.toml` applies there (no `unwrap` / `expect` / indexing outside tests).

Crate dependencies (lower crates never depend on higher ones; design §5, decisions 0043, 0044; each arrow points
from a crate to one it depends on, as in the `Cargo.toml` files):

```
backend (Bevy-free; the main path — a crate may also use any crate further right; project_core is the bottom):
  shader_project → shader_codegen_rust → shader_codegen_wesl → shader_stdlib → shader_graph_core
  shader_graph_core → curve_kit, texture_kit (models only, `default-features = false`; re-exported) → project_core
  (Material backend: regen, writer, ops; IR → crates/quillshade; IR → WESL + wesl/naga check; node defs; model, IR)

editor (Bevy), a fan-in on editor_core:
  editor_core      → project_core              (shell, Editor, dispatch, domain API; no kit, domain or shader_* crate)
  curve_kit        → editor_core, project_core (feature `editor`: CurveDomain)
  texture_kit      → editor_core, project_core (feature `editor`: TextureDomain)
  node_graph_base  → editor_core               (graph canvas over vendor/bevy_noodle, GraphDomain)
  material_domain  → editor_core, node_graph_base, curve_kit + texture_kit (`editor`), shader_project and the other
                     shader_* crates, project_core
  light_domain     → texture_kit (`editor` with its own `editor` feature), editor_core (`editor`), project_core
                     (backend Bevy-free: environments, cube-map textures, `crates/quillshade/src/light/`)
  src/main.rs      → material_domain, light_domain, node_graph_base, editor_core, shader_project, project_core
                     (the domain list; curve_kit / texture_kit only to name their `dynamic_linking` features)
```

The kits and `node_graph_base` are independent siblings: each builds on `editor_core`, never on each other;
only a domain combines them. Only the editor crates (`editor_core`, `node_graph_base`, the kits and `light_domain`
with `editor`, `material_domain`) depend on Bevy. Domains never depend on each other: `light_domain` names no
`material_domain` or `shader_*` crate.

**Multi-domain layout** (decision 0043; design `docs/superpowers/specs/2026-10-28-multi-domain-editor-design.md`).
`project_core` is domain-neutral and depends on no workspace crate: `DomainId`, generic `Diagnostic` / `Level`,
`ron_io`, `marker`, `writer`, `files`, `ident::to_snake_case`, `layout` (`Project`, `ProjectSettings`), `umbrella`,
`regen` (`DomainBackend`, `DomainPlan`, `run`, `commit`), `watch::ProjectWatcher`, `game_files::scan`. `curve_kit` and
`texture_kit` (`TexturePolicy { extensions, shape, internal, presets }`, `check_path`, `InternalTexture`,
`INTERNAL_PREFIX`, `new_texture_path_for_file`) are domain-shared models that depend only on `project_core`. Each kit's
`editor` feature (default on; backends use `default-features = false`) is its editor, generic over a domain's
`CurveDomain` / `TextureDomain` and added by `<kit>::editor::register::<D>`: the curve editor (`CurveDoc<D>`, view,
sidebar, `CurveCtx<D>`, the curve clipboard in `Editor::clipboards`); the texture editor (`TextureDoc<D>` with the form
and Easy / Advanced presets only when `POLICY.presets`, `ImageDoc<D>` of a game file, the shared preview
`ImagePreviewPlugin<D>` / `PreviewDoc`, `GameImageCache<D>`, "Create Texture Asset" for the game files of
`POLICY.extensions`, the virtual Internal folder of `POLICY.internal`; path edits, undo / redo and creation run in the
kit's step handler, `DomainRegistrar::steps`). Material's `TextureDomain::POLICY` is `MATERIAL_TEXTURES` plus
`shader_stdlib::internal::internal_textures`. A game project holds sources in `quillshade/<domain>/` (Material:
`quillshade/material/**`, Light: `quillshade/light/**`), settings in `quillshade/project.ron`
(`domains: { "<id>": (enabled: bool) }`, missing = enabled; a missing file means defaults, one that does not parse is
`OpenError::Settings` — CLI exit 2, the editor refuses the project — and the editor re-reads it on change, keeping the
old settings on a broken edit), generated assets in `assets/quillshade/<domain>/` and one generated crate
`crates/quillshade` (core writes its `Cargo.toml` and `src/lib.rs` with `QuillshadePlugin`; each domain writes
`src/<id>/`, Material: `QuillshadeMaterialInternalPlugin`, Light: `QuillshadeLightInternalPlugin`; a disabled domain
keeps its `pub mod`, only its plugin line goes). The game's root `Cargo.toml` needs
`members = ["crates/quillshade"]` and `quillshade = { path = "crates/quillshade" }` (W105). The Material backend is
`shader_project::backend::MaterialBackend`, the Light backend `light_domain::backend::LightBackend`; there is no
migration from the old `shaders/` layout. `texture_kit`'s Bevy-free `load` / `save` / `problems` (the texture rules
in one order with one text each) are shared by both: each domain maps a `ProblemKind` to its own code. Project
Settings saves with `project_core::layout::save_settings` and rewrites only the umbrella files with
`project_core::regen::write_umbrella`.

Pipeline: the CLI runs `project_core::regen::run` over the `DomainBackend`s (each domain's `plan`, then one `commit`);
the editor runs `shader_project::regen::regenerate_set` (`plan_set` + `project_core::regen::commit`). Material's
`plan_set`: load assets → validate → `build_ir` → WESL + Rust codegen; `commit`: compare, E008, orphan scan, batched
atomic write. Key facts that span several files:

- **Generators read only the IR** (`shader_graph_core::ir`), never the editor model. The IR also owns the uniform
  layout (fields sorted vec4 → vec3+pad → vec2 → scalars, padded to 16 bytes); the WESL struct and the Rust `Params`
  struct are both derived from it, and the generated crate has a `min_size` test checking they match. Bindings: uniform
  `@binding(100)`, textures/samplers `101/102`, `103/104`, … from `Ir::textures` (texture parameters `inp_*`, then
  fixed textures `ft_*` — one per node, also per node inside a called function —, then gradient collections `gc_*` —
  one per collection; read by WESL, Rust and `binding_of`).
- **Node registry**: every node implements `NodeDef` (`shader_graph_core::registry`). Stdlib nodes are Rust code built
  with the closure builder in `crates/shader_stdlib/src/def.rs` (`def(id, name, cat).kw(..).param(..).build(sig, emit)`)
  and registered per category in `crates/shader_stdlib/src/nodes/*.rs`. Reusable WESL helpers live in
  `crates/shader_stdlib/wesl/*.wesl`, embedded via `include_str!` (`MODULES` in `lib.rs`) and always written to
  `autogen/stdlib/`. User nodes (`quillshade/material/.nodes/*.ron`, kind `user::<id>`) go through the same trait
  (`shader_stdlib::user`).
- **Curves** (`quillshade/material/**/*.curve.ron`, decision 0015) are not graphs: `shader_graph_core::curve::Curve`,
  `AssetType::Curve`, kept in `AssetSet::curves` (not `assets`). Each becomes its own module
  `autogen/curves/dir_…/crv_<name>.wesl`; the `stdlib::SampleCurve` node (param of `ParamKind::Curve`) imports
  `sample_curve_simple`/`sample_curve_catmullrom` from it under a per-curve alias (`curve_fn_alias`).
  `curve_refs` finds the nodes referring to a curve (deps, rename). `Curve::sample` is the reference math, in the output
  range `min..max` (decision 0038) baked into the module's `POINTS` — the stored points stay normalized.
- **Global constant tables** (`quillshade/material/**/*.consts.ron`, decision 0017) are not graphs either:
  `shader_graph_core::consts::GlobalConsts`, `AssetType::Consts`, `AssetSet::consts`; they generate no file. The
  `stdlib::Global*` nodes (param `const`, `ParamKind::GlobalConst(ty)`) inline the value as a literal through
  `Env::consts`; `regen::collect_consts` resolves names (unique project-wide), `const_refs` finds readers (deps,
  clearing on type change / delete). `regen::wesl_file` / `module_of` are `None` for tables.
- **Texture assets** (`quillshade/material/**/*.texture.ron`, decision 0024): `shader_graph_core::texture::TextureAsset`
  (image path under the game's `assets/`, `.ktx2` / `.png` only, + loader settings), `AssetType::Texture`,
  `AssetSet::textures`; no file of their own. One game file — one asset (E011 on every sharer); E012 path rules,
  E013 sampler rules, E014 KTX2 cube map / array / 3D, W109 missing file
  (`regen::texture_diagnostics`). Nodes reference them by id through
  `ParamKind::Texture` (`texture_refs`: deps, rename rewrite, delete clears → W110 + `@internal/default`; missing →
  E006); old image-path defaults fail in `assets::load`. **Internal textures** (`shader_stdlib::internal`,
  `@internal/default`) are embedded PNGs written to `assets/quillshade/material/internal/` on every regen.
- **Gradient collections** (`quillshade/material/**/*.gradients.ron`, decision 0037) are not graphs either:
  `shader_graph_core::gradient::GradientCollection` (exactly 128 slots with stable random ids `1..=i32::MAX`, ≤ 32
  stops, `mid` on the left stop; `Slot::sample` = OKLab reference math in `f64`), `AssetType::Gradients`,
  `AssetSet::gradients`; no WESL / Rust file. Regen bakes each into a 128×128 sRGB PNG (`bake_rgba8`,
  `shader_project::gradient_png`, byte-deterministic `png` + fdeflate) in
  `assets/quillshade/material/gradients/<path>.png`, owned like `internal/` through `.autogen` (only while the project
  has a collection), listed in the manifest as `@gradients/<path>` (`BakedTextures.gradients`). `stdlib::SampleGradient`
  (params `ParamKind::GradientCollection` + `GradientSlot`) samples it through `texture.wesl::sg_sample_gradient` at
  texel centres with a row literal = the slot's position (`GradientLookup`, `Env::gradients`); one binding `gc_<path>`
  per collection per material (`TextureSource::Gradient`), hidden trailing `gc_*` arguments in functions
  (`FunctionSig::gradients`, after `ft_*`), E006 on identifier clashes. W111: no gradient (white, no binding) or an
  empty one (still sampled, so slot contents never change code). `gradient_refs`: deps (a reorder regenerates the row
  literal only), rename rewrite, delete clears collection + slot. Color / stop / slot edits change only the PNG.
- **Baked manifest** (decision 0025): regen writes `assets/quillshade/material/autogen/baked.ron`
  (`shader_codegen_rust::manifest`: every valid Texture asset and gradient collection + every material's values /
  texture ids). The generated crate embeds it (`COMPILED_MANIFEST`, `include_str!`) and overlays the runtime file on hot
  reload; `crates/quillshade/src/material/baked/mod.rs` is the static template
  `crates/shader_codegen_rust/templates/baked_mod.rs` (`BakedMaterialsLoader`, progress, overlay),
  `baked/{materials,textures}.rs` come from `shader_codegen_rust::baked`. Generated `new` / `set_*` are `pub(crate)`;
  games use `BakedMaterials` / `BakedTextures` (nested fields, or `by_path` / `get` by asset id, decision 0030).
  **Layout guard** (decision 0029): every material WESL compiles only with its `Ir::layout_feature()`
  (`SG_LAYOUT_<hash>`), which the generated `specialize` enables, so a hot-reloaded shader with other bindings — or
  another blend mode / two-sided / Pbr3d shading, which live in the compiled Rust — fails to compile instead of crashing
  the game or running with the old pipeline state (the material is then not drawn); the editor logs a "rebuild the game"
  warning whenever a regen rewrites the generated crate's `.rs` files (`Editor::note_rust_changes`).
  `check::Files::enable` does the same for the compile check. The manifest's `layout` / `bindings` let the runtime match
  texture bindings by position.
- **Scene textures** (Scene category: `stdlib::SceneDepth`, `DepthFade`): reads of Bevy's view bind group
  (`depth_prepass_texture`), not material bindings — no `Ir::textures` entry, no hidden function argument. The stdlib
  module `scene.wesl` reads them under `@if(DEPTH_PREPASS && !PREPASS_PIPELINE)` with a fallback (far plane), by
  `textureLoad` (multisampled under MSAA). Nodes call `EmitCtx::scene_texture`, which records `SceneTexture`
  (→ `FunctionSig::scene` → callers, `Ir::scene` → the generated material's doc names the camera component) and
  reports E009 (vertex stage, Sprite2d) / W112 (opaque or masked material: it reads its own depth). The compile
  check's `Variant::Forward3dPrepass` (prepass defs + `MULTISAMPLED`) is the only one compiling the real branch. World
  Position Offset materials have no prepass vertex shader yet, so a camera with `DepthPrepass` breaks them — the
  sample camera has none.
- **Node kinds** are string paths: `stdlib::*`, `param::*`, `output::*`, `function::<sub/name>`, `user::<id>`.
- **Material Attributes** (decision 0023): `PinType::MaterialAttributesPbr` is the WESL struct of the stdlib module
  `material.wesl`; field ids / defaults / activity live in `shader_graph_core::attributes` (shared by Make/Break, the
  output node and `build_ir`). Any module naming the struct imports it — registered only in `EmitCtx::define_typed`,
  `build_ir` step 6 (unconnected struct input → default constructor) and step 7 (a function signature using it). Pbr3d
  `PbrSettings::output` = `Wrapped` turns `output::Pbr3d` into `attributes` + WPO, expanded into the same per-field
  output keys as `Default`. Only `Lerp` (`PinDef::attributes`) accepts the struct among polymorphic nodes.
- **Node context actions** (decision 0021) are data on `NodeDef` (node RMB menu, `Shift + - * /` convert, `V`). A
  material can store a **node preview** (`Asset::preview`, decision 0022): the node's output replaces the material
  output in the generated WESL (W107/W108); `Node::label` is an optional title for Constant nodes.
- **Type conversions** (broadcast, truncation, Color↔Float4, Int→Float) are applied in codegen on the edge, not as
  inserted nodes (`shader_graph_core::types::conversion`); truncation emits W104.
- **Functions** are separate WESL modules imported by materials via relative `super::` paths (`codegen_wesl::paths`) —
  no inlining (decision 0002). Builtins a function needs (UV, time, world position…) become trailing hidden `b_*`
  parameters filled in by the caller (decision 0005); its fixed textures follow as hidden `ft_<fn>_<node>` texture +
  sampler parameters (`FunctionSig::textures`, decision 0024), then one `gc_<path>` pair per gradient collection
  (`FunctionSig::gradients`, decision 0037). Changing a function regenerates its dependents
  (`DepGraph`).
- **Writer** (`shader_project::writer`): all files go to temp first, then a series of renames; files are rewritten only
  if content changed (avoids spurious hot-reloads); orphans with the autogen marker are deleted; files *without* the
  `// AUTOGENERATED by shader-editor v…` marker are never touched (E008), binary ones included. Contents are bytes:
  internal textures (`shader_stdlib::internal`) go to `assets/quillshade/material/internal/` and baked gradient
  collections to `assets/quillshade/material/gradients/`, each owned through the marker file `.autogen` (no marker in
  a non-empty dir → E008, nothing touched there). The editor never edits the game's root `Cargo.toml` (W105
  instead).
- **Errors block only their own asset**; the rest of the project still regenerates.
- **Material kinds**: Pbr3d → `ExtendedMaterial<StandardMaterial, {Name}Ext>` (shadows/prepass/deferred for free);
  Unlit3d → own `impl Material`; Sprite2d → `MaterialExtension2d` over `SpriteMaterial`. World Position Offset generates
  a custom vertex shader copied from Bevy 0.20 `mesh.wesl` (decision 0006) — fragile across Bevy patch releases.

**Light domain** (`light_domain`, decision 0045, design §9.2). `backend` (Bevy-free): `quillshade/light/**` holds
`*.environment.ron` (`Environment { format_version: 1, cubemap: Option<String>, skybox_brightness, env_intensity }`,
the cube map a Light Texture asset id) and `*.texture.ron` through `texture_kit` under `LIGHT_TEXTURES` (`.ktx2`, cube
maps only, no internal textures, no presets). `validate` reports `L`-prefixed codes (decision 0043): `LE001` unknown
cube map texture, `LW101` no cube map, `LE009` file does not load, and the texture kit's `problems` mapped to `LE011`
(one file shared), `LE012` (path), `LE013` (sampler), `LE014` (not a cube map), `LW109` (file missing). `LightBackend`
writes one file, `crates/quillshade/src/light/mod.rs` (`ENVIRONMENTS`: the asset paths of the environments without
errors, sorted; an empty `QuillshadeLightInternalPlugin`), and owns `assets/quillshade/light/` (nothing written there
yet). `rewrite_texture_refs` follows a texture rename / delete. `editor` (feature, default on): `LightDomain`,
`LightState` (assets + diagnostics, revalidated live with open tabs' unsaved edits), the environment form
(`EnvironmentDoc`, `"light.environment"`), `LightTexture: TextureDomain` (`NEW_TEMPLATE`, `PREVIEW_FIRST_LAYER`,
its own `SETTINGS_NOTE`); icons `LIGHT_ICON` (`icon_rs::THEME_LIGHT`) and `ENVIRONMENT_ICON` (`icon_rs::PUBLIC`).

### Editor (`editor_core`, `node_graph_base`, kit editors, `material_domain`, `light_domain`; decisions 0044, 0045)

Window layout. Requests like "add X to the editor topbar" use exactly these names; use them in code comments, docs
and commit messages too:

```
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

- **mainbar** — global: logo, main menu (File / Edit / View / Assets / Help), then after a separator the **domain
  tabs** (one per domain, name only, the active one brighter, the project domain last, hidden while no project is
  open; design §14.3), then the project name and error count (all domains); never buttons of one editor kind.
- **secondbar** — global, the full window width under the mainbar: **main actions** (Back / Forward), **domain
  actions** (a reserved slot, empty and hidden for now), then the **tabs**: open tabs of every domain (other real
  domains' greyed, tool tabs blue), one row high, growing by whole rows; visually separate from the editor topbar
  below it.
- **left sidebar** — the active domain's (design §14.4, `ui::tree::left_panes`): its **project tree**
  (`quillshade/<domain>/**` assets, root label = the domain id, Material: `material`) above its **assets tree** (the
  game's `assets/` without `assets/quillshade/`, filtered by the domain's Game Assets extensions, read-only, hidden
  for a domain without them; the virtual Internal folder ends the Material project tree); with the project domain
  active, its Tools list instead. The Assets menu is the active domain's "New …" items, New folder and Generate.
- **editor topbar** / **editor content** / **editor statusbar** / **right sidebar** — owned by the active tab's *editor
  kind* (Material's: graph, curve, consts, image, texture, gradients; Light's: environment, texture, image; the
  project domain's: settings; core's: text — a `ViewKindId`): its buttons and info, its main view, its status line,
  its additional content (properties of the selection, hints…).
- **console/logs** — global, under the editor statusbar: diagnostics and messages of the active domain (plus global
  and project lines; the header's "All domains" checkbox shows every domain's).

**Crates.** `editor_core` is the shell and the domain API; it depends on no kit, domain or `shader_*` crate.
`node_graph_base` is the graph UI behind `GraphDomain`. `curve_kit::editor` / `texture_kit::editor` are the curve and
Texture / image editors behind `CurveDomain` / `TextureDomain`. `material_domain` is everything Material
(`MaterialDomain`), `light_domain`'s `editor` module everything Light (`LightDomain`). Domains never depend on each
other.

- **Domains** (design §6.1, `editor_core::domain`): `src/main.rs`'s `domains()` lists every real `EditorDomain`
  (`MaterialDomain`, `LightDomain`; `id`, `info` → `DomainInfo { title, icon, summary }` (summary: its line on the
  welcome screen), `backend`, `register`; one line each, shared by the GUI's `editor_core::run(domains, …)` and the
  CLI's backends); `DomainList::new` appends the built-in `ProjectDomain` (`DomainId::PROJECT`, reserved, always last,
  no backend, unknown to the CLI; `real_ids()` skips it). `register` fills a `DomainRegistrar`: `state` (its
  `DomainState`), `asset_kind(AssetKindDef { suffix, title, icon, templates, open, row_menu })` (the "New …"
  `AssetTemplate`s of the Assets menu and the folder menus; Material registers the kits' kinds between its own to keep
  the menu order), `editor_kind::<E>()`, `game_assets(GameAssetsDef { extensions, open, menu })`, `tree_extras` (virtual
  folders: the Internal folder, through `texture_kit`), `menu_items(MainMenu, fn)` (shown only while the domain is
  active, the project domain's always; tool items likewise, File's right after Close Project), `tool(ToolDef)` (see
  Tools below), `parse_tab` / `tab_exists` (lists: a domain's and its kits' handlers, then core's defaults; each
  `parse_tab` must claim keys no other handler or the default takes — `None` means "not mine" —, and with no
  `tab_exists` handler a key exists when a file of a registered kind has that source path) / `legacy_tab_keys`, `steps`
  (handlers of the domain's `Action::Domain` payloads and deferred document steps, tried before its state's `apply`, so
  a kit's commands work without a domain state), `highlighter(lang, fn)` (Material: WESL), `input_context`, `logic`
  (systems also needed headless: added by `EditorCorePlugin`), `bevy` (window systems: added by `DomainWindowPlugin`,
  part of `UiPlugin`; headless UI tests add the same plugin and `domain_doc_views`), `doc_view` / `doc_overlay` (scenes
  of the editor content), `quick_start` (its Help → Quick Start steps, titled `"<Domain>: <step>"`, after core's
  `editor_core::quick_start` crate / plugin steps). `DomainList::new` runs every `register` once (an `Arc` in
  `Editor::domain_list`, also a resource); it answers `kind_defs()`, `crate_contributions()` (the umbrella crate the
  editor regenerates), `parse_tab_key`, `highlighter`, `menu_items`, `game_assets`, `quick_starts`, `tools`. Material's
  registration is `material_domain::domain` (`MaterialDomain`; `material_only()` / `test_editor()` for tests), Light's
  `light_domain::editor` (`LightDomain`).
- **Asset-kind defaults** (decision 0045, `editor_core::state::kinds`): from a domain's registered asset kinds core
  gives its Project tree rows (`scan`, cached in `Editor::kind_files` at rescan; `DomainState::tree_items` returns
  `Option`, `None` = this default), its tab keys (a key equal to a kind file's source path opens through the kind's
  `open`) and file rename / delete (the defaults of `DomainState::rename_file` / `delete_file`). `Editor::rename`
  renames the navigation history and `last_tabs` of every domain (`TabKey::renamed`, which also rewrites
  `<key>#<view>` keys of views derived from an asset, Material's generated files). Light uses all of them; Material
  keeps its own tree rows, rename / delete and generated-file keys. On project open core runs `generate` for every
  real domain whose `crates/quillshade/src/<id>/mod.rs` is missing (the umbrella names every domain's module).
- **Tools and the active domain** (decision 0045, design §14.1, §14.2): a tool is a singleton tab without an asset
  (`DomainRegistrar::tool(ToolDef { id, title, open, menu })`, `TabDoc::tool()`), unique per `(domain, id)`;
  `Action::OpenTool { domain, id, arg }` activates the open tab or opens one, then hands `arg` to it as
  `TabAction::Doc`. Its key is `"@tool/<id>"` (Back / Forward reopen it); tool tabs are never written to the user
  config. The project domain's tool is Project Settings (`editor_core::settings`, `SETTINGS_TOOL`, kind
  `"project.settings"`: General + one "Used in game" per real domain; save = `save_settings` + `write_umbrella`;
  an outside edit of `project.ron` reloads a clean tab and asks Keep mine / Reload for a dirty one —
  `Action::Reload(TabKey)` / `KeepMine(TabKey)` address any tab by key). `Editor::active_domain` changes when a real
  domain's tab is activated, on `Action::SwitchDomain` (a domain tab; activates that domain's `last_tabs` entry, or
  shows its empty hint; the project domain with none opens Project Settings) and on Back / Forward; **overlay
  rule**: a project tool's tab keeps the active domain, so the left sidebar, Assets menu, console filter, greying
  (`Editor::is_greyed`) and input context stay its. Closing the active tab activates the nearest tab of the active
  domain or a project tool (`close_successor`), never another domain's. Console filter: `Editor::console_domain` /
  `console_messages` / `console_diagnostics`, `Action::ToggleConsoleAll`.
- **Domain state** (design §6.3): `Editor::domains: DomainStates`, one `Box<dyn DomainState>` per domain
  (`get::<T>()`; Material: `editor.domains.material()` via `MaterialStates`). Core calls its hooks — `apply`,
  `after_apply`, `after_save(editor, &[tab])` (once per domain and save operation, with its saved tabs: Save, Save
  All, the close prompt's Save, project switch / close), `before_close`, `rescan`, `diagnostics` /
  `console_diagnostics`, `tree_items`, `generate`, `delete_message`, `rename_file` / `renamed`, `before_delete` /
  `delete_file` / `deleted`, `focus_diagnostic`, `files_changed` — through `Editor::with_domain`, which takes the
  state out for the call. Material's hooks lend it back (`MaterialState::lent`; `self` is a placeholder meanwhile, a
  state not given back panics) because its `Editor` methods read it there. Console/logs lines written by a hook or a
  registrar callback (`Editor::as_domain`) carry the domain (`Message::domain`).
- **Actions** (design §6.2, `editor_core::action`): widgets push `Action`s → `editor_core::apply_actions` →
  `Editor::apply` mutates documents → views redraw from the model. Core's `Action` holds project / tab / dialog / View /
  nav / save / Generate / folder / rename / move / delete commands plus `Action::Tab(TabAction)` (for the active tab),
  `Action::Domain(DomainId, Payload)` (Material: `ops::MaterialAction`, `From<MaterialAction> for Action`,
  `ops::material_action` unwraps one), `OpenTool`, `SwitchDomain` and `ToggleConsoleAll`. `TabAction`: the shared edit
  verbs (`Undo`, `Redo`, `Copy`, `Cut`, `Paste(pos)`, `Duplicate`, `DeleteSelection`, `SelectAll`, `CommitInline`),
  `SetValue` / `SetComponent` of a `PropKey`, and `Doc(Payload)` for kind-specific edits (`GraphEdit`, `CurveEdit<D>`,
  `TextureEdit<D>`, …); each goes to the active tab's `TabDoc::apply`, and a verb or payload the document does not know
  is dropped (debug log). Steps that need other tabs or a dialog go through `DocCtx::defer`: right after the dispatch
  they reach the domain's `steps` handlers, then its `DomainState::apply`. `Payload` and `PropKey`
  (`editor_core::dynval`) are type-erased values compared by value (`PartialEq<T: PropTarget> for PropKey`); each kind
  has its own property target type (`GraphProp`, `ConstsProp`, `GradientProp` in `material_domain::ops::types`,
  `EnvironmentProp` in `light_domain`, `SettingsProp` in core). The kits' edits are typed by domain (decision 0045):
  `CurveEdit<D>`, `TextureEdit<D>`, `PreviewChannel<D>` and property keys `editor_core::dynval::KitProp<D, P>` over
  `CurveProp` / `TextureProp` (`curve_key::<D>` / `texture_key::<D>`), so a kit document drops another domain's edit.
  Material's `Editor` methods are extension traits in `material_domain::ops` (`MaterialEditor`, `MaterialApply`,
  `MaterialGraphOps`, `MaterialRegen`, … — `use crate::ops::prelude::*`).
- **Tabs** (`editor_core::tab`): `Tab { domain, doc: Box<dyn TabDoc> }` (`Tab::get::<T>()`, `Editor::active_doc::<T>()`;
  Material adds `doc()` / `curve_doc()` / `consts_doc()` / … in `MaterialEditor`). `doc_kind::TabDoc` gives title, icon,
  watermark, dirty, save / reload with a `DocCtx` (project, the domain's state, console lines, `Editor::clipboards`),
  rename, `config_key`, `status` → `StatusInfo` (the editor statusbar, from a `StatusCtx`), `apply` and
  `current_value(&PropKey)`. Documents: Material's graph `doc::Doc`, `ConstsDoc`, `GradientsDoc`; Light's
  `EnvironmentDoc`; `curve_kit`'s `CurveDoc<D>`; `texture_kit`'s `TextureDoc<D>` (read-only for a built-in `@internal/…`
  texture) and `ImageDoc<D>` of a game image; core's `SettingsDoc` (a tool) and read-only `TextDoc` of a generated file
  (its highlighter comes from the `DomainList`: Rust is core's `highlight.rs`, WESL Material's `highlight_wesl.rs`; the
  view is `ui::text_view`, the kind `editors::text`). The editable kinds share `history::History<T>`. Open tabs are
  remembered as `TabKey { domain, key }`, `"<domain>:<key>"` in the user config; a key without a prefix is read as
  Material's (configs from before domains), an unknown domain, a key no domain parses or a missing target is skipped
  silently. Tool tabs are never saved; per project the config also keeps `active_domain` and `last_tabs` (each domain's
  last active tab key).
- **Editor kinds** (`editor_core::editors`, decision 0031): a kind is a zero-sized type implementing `EditorAreas`
  (`VIEW: ViewKindId`, `SIDEBAR_TITLE`, `topbar()` / `sidebar()` → `Option<impl Scene>` — `None`: the strip is hidden
  for the kind —, `build(app)` its redraw systems, `input(app)` its input context) and, with hotkeys, `EditorInput`
  (`type Ctx`, `bindings()`). Domains register kinds (`DomainRegistrar::editor_kind`); `DomainList::kind_defs()` is the
  one list (`EditorKinds`, core's text kind first), read by `AreasPlugin` and `InputContextsPlugin`. Material's kinds
  are in `material_domain::editors` (graph through `node_graph_base`, consts, gradients) and the kits (`CurveEditor<D>`,
  `TextureEditor<D>`, `ImageEditor<D>`); Light's are `EnvironmentEditor` (`light_domain/src/editor/form.rs`) and the
  texture kit's. `ViewKindId`s are string constants (`"material.graph"`, `"material.consts"`, `"material.gradients"`,
  `"material.curve"`, `"material.texture"`, `"material.image"`, `"light.environment"`, `"light.texture"`,
  `"light.image"`, `"project.settings"`, core's `"text"`); a kit's are per domain. Namespace a view id
  `<domain>.<kind>`: `DomainList::new` panics when two different kinds (`KindDef::type_id`) share one id; the same kind
  registered twice is listed once.
- **Areas** (`editor_core::ui::areas`): each area is spawned once in its strip (`EditorTopbar`, `RightSidebar`,
  `EditorStatusbar`) inside an `AreaOf(ViewKindId)` container; `show_active_areas` sets `Node::display` (and
  `Visibility::Hidden`, which Bevy's tab navigation needs; written only on change) so only the active kind's shows
  and a strip it has nothing for takes no space. Bodies (`*ViewBar`, `*SidebarBody`) are refilled by the kind's
  systems keyed on what they show (sidebars: `(ui.props, active)` via `SidebarKey`), never every frame. Editor
  content views are tagged `ViewOf(ViewKindId)` inside `ui::DocArea`.
- **Property widgets** (`editor_core::ui::props`): the core fields edit an opaque `PropKey`, and all their observers
  (select, text, number, color, checkbox) are core's (`PropWidgetsPlugin`, part of `DomainWindowPlugin`), so a
  kit's fields work without Material. A field's final edit triggers `FieldEditEnded`;
  `node_graph_base::InlineEditsPlugin` turns it into `CommitInline` for a field inside a canvas node. The Material
  ones (`value_widget`, `select_w`, `pin_kind`) are in `material_domain::ui::props`, inline node editors in
  `inline.rs`.
- **Input contexts** (`editor_core::main_input`, decisions 0032, 0034, 0035, 0041): bevy_enhanced_input. One
  `InputRoot` entity carries `MainCtx` (save, undo / redo, close / next tab, open, quit, the View toggles Ctrl+1
  `ToggleLeftSidebar` / Ctrl+2 `ToggleLog` / Ctrl+3 `ToggleProperties` / `` ` `` `ToggleZen`, `NavBack` /
  `NavForward` on Ctrl+[ / Ctrl+], the mouse side buttons and the Browser keys — `on_nav` drops them while typing or
  with L / R / M held, as the gate hides only the keyboard; always active, priority 2) and one context per kind
  (`node_graph_base::GraphCtx<G>` — palette on Space, Esc `CancelWire`, …, plus the domain's `GraphDomain::bindings`;
  `curve_kit`'s `CurveCtx<D>`, core's `TextCtx`, Material's `GradientsCtx`; priority 1) that `sync_input_context`
  activates only while the active tab is of that kind — so a hotkey needs no "is this my tab" guard. A domain may
  add a domain-level context (`DomainRegistrar::input_context`). The shared edit actions (`EditCopy`, `EditDelete`,
  …) are bound by each kind that supports them and have one global handler each. Build actions with `hotkey::<A>()`
  (`consume_input` + `require_reset`) and the `ctrl` / `ctrl_shift` / `shift` / `plain` binding helpers ("Ctrl" =
  Ctrl or Super); handlers observe `On<Start<A>>` (`on_combo` / `on_plain`) and push `Action`s; a hold-to-fire key is
  `held_hotkey::<A>(bindings, secs)` (a one-shot `Hold`, handlers observe `Fire`: hold C `AddComment`, which arms
  only on a fresh C press so a C left down after Ctrl+C never fires). Plain-key handlers ignore the key while Ctrl,
  Alt or Super is held (`keys::no_command_mods`). `gate_keyboard` (PreUpdate, before `InputFocusSystems::Dispatch`)
  hides the keyboard from all contexts while typing (text field focused, palette, dialog, menu or popup open; a
  scrubbed number field only while LMB is held, `typing_into`; Space / Enter while a focused button or checkbox takes
  them, `PRESS_KEYS`, and arrows / Home / End / Space / Enter while a focused list box takes them, `LIST_KEYS`: the
  widget wins). Keyboard focus is Bevy's default (decision 0035): Tab walks every tabbable widget, a canvas click
  clears the focus; only additions are kept — `tab_into_number_input`, `drop_hidden_focus`, the palette's modal
  `TabGroup`, `Visibility::Hidden` on hidden areas — plus one exception, Ctrl+Tab (`NextTab`) clears the focus
  (`drop_focus`). Activating a context re-inserts its bindings so a key held across a tab switch does not fire
  (relies on bevy_enhanced_input internals — re-check `crates/material_domain/tests/input_contexts.rs` on upgrades).
  Mouse gestures stay picking observers and read modifiers via `keys::mods()` (`editor_core::keys`).
- **Graph UI** (`node_graph_base`, design §7.3): `register_graph::<G>(r)` adds a domain's graph kind
  (`GraphEditor<G>`: editor topbar tools, right sidebar with the comment frame / multi-selection subpanes,
  `GraphCtx<G>`), its canvas view and minimap, the canvas sync (`CanvasPlugin<G>`, also headless), gestures
  (`input::InputPlugin<G>`) and palette (`palette::PalettePlugin<G>`). The model stays in the domain behind
  `GraphDomain` (`view`, `node_look`, `diagnostics`, `can_connect`, `plan_move_wires`, `header_action`,
  `open_action`, `pin_menu`, `node_menu`, `hotkey_actions`, `placement_hotkeys`, `place`, `palette`, `drop`,
  `sidebar`, `frame_prop`, `topbar_extras()`, `previews`, `bindings` / `build` / `input`); node and pin ids are
  opaque strings, edits come back as `GraphEdit`s in `TabAction::Doc` (pasting is `TabAction::Paste`). Every canvas
  entity carries `GraphOf(G::VIEW)` so several graph domains never touch each other's; `GraphViews` lists them all.
  Material implements it in `graph_domain.rs` (`MaterialGraph`); its look (pin and header colours) is `canvas.rs`,
  its palette entries `ui/palette.rs` + `search.rs`, its in-node preview kinds `ui/node_preview.rs` (`kinds()`, over
  the base's `preview::NodePreview`), its placement data `ui/input.rs`.
- The model (`shader_graph_core::Asset` inside `doc::Doc`) is the source of truth; the bevy_noodle canvas is
  **rebuilt from the model** on revision change (`rebuild_canvas`, only when no gesture is active). Canvas gestures
  arrive as `EditApplied` with `EditOrigin::Interaction` and are translated into `GraphEdit`s; edits the canvas makes
  during its own rebuild are ignored.
- Undo/redo/copy/paste are model-level (snapshots of `Asset`, 200 steps, consecutive edits of one field merged — the
  merge key is a `history::MergeKey`, a property's `PropKey` compared by value), not bevy_noodle's scene history.
  The curve clipboard is `CurveClipboard` in `Editor::clipboards` (one value per type, shared by all tabs).
- Connection validity is decided by Material's `doc::connection_error` through `GraphDomain::can_connect` (the base's
  `ConnectionCheck`; bevy_noodle ports are all `PortType::ANY`).
- Ctrl+drag from a pin is the fork's move-wires gesture (`CanvasInteraction::move_wires_keys`, design §5.3): it lifts
  the pin's wires (`MovingEdges`) without touching the model; candidate pins come from a `MoveCheck` observer running
  `GraphDomain::plan_move_wires` (`canvas::check_move`; Material: `Editor::plan_move_wires`), and the drop's
  `NoodleEdit::MoveEdges` is vetoed and turned into one `GraphEdit::MoveWires` (`canvas::request_move_wires`); a
  refused drop on a pin logs why (`input::lifted_wires_dropped`). Esc (`GraphCtx` `CancelWire`) ends any wire drag by
  despawning `GraphQuery::wire_of(canvas)`, as noodle's own cancel does.
- Module seams (Material, decision 0026 as amended by 0044): `ops/*` (commands, apply, regen, watch, tabs, nav,
  `*_ops`, `droppable.rs`: `DroppableFile` / `Dropped`, dragging images, Texture, Function, Curve and gradient
  collection assets onto a graph — a function dropped into itself is refused through `search::is_self_call`, as in
  the palette), `material_state.rs`, `editors/` (graph, consts, gradients), the documents (`doc.rs`, `consts_doc.rs`,
  `gradients_doc.rs`), `curve.rs` (`MaterialCurve`), `texture.rs` (`MaterialTexture`, its `TextureDomain` with the
  aliases of `texture_kit::editor`'s types), `tab.rs` (generated-file keys, `override_into` for validation),
  `tests/` (the Material editor tests). Core: `state/*` (`Editor`, dispatch, tabs, assets, watch, tests with
  test-only domains), `nav.rs` (`NavHistory`, decision 0041: the in-memory Back / Forward list of tabs and graph
  camera views, fed by `Editor::follow_active` and the canvas's settled-view polling), `ui/tree.rs` (`TreeSource`:
  each domain's Project tree from its folders, `DomainState::tree_items` and `tree_extras`, plus the read-only Game
  Assets), `ui/browser.rs`, `ui/shell.rs` (layout, `UiPlugin`, `DomainWindowPlugin`), `ui/dialog.rs`, `ui/log.rs`,
  `ui/welcome.rs`.
- Canvas details (decisions 0033, 0034, 0035): node positions snap to `node_graph_base::layout::GRID_SNAP` (10) —
  pointer drags through the bevy_noodle fork's `CanvasInteraction::snap` (the grabbed node; `attach_canvas`), new /
  pasted nodes in Material's `Editor::add_node` / paste; loaded positions are never rewritten. Comment frames are
  `NodeFrame`s: dragging one moves the nodes inside it in the same edits. Node emphasis (selection, pulsing preview,
  diagnostics) is a bevy_ui `Outline` written by `canvas::node_outlines` every frame, and pin hover the fork's
  `PortHoverStyle` on the canvas (`rebuild_canvas` carries `Hovered` to a rebuilt pin's new port) — neither rebuilds
  the canvas. Pins are the fork's `kit::port_sized` with a `kit::PortSize` of `canvas::PIN_PHYSICAL` (24: the port
  box — hit area, wire anchor, row room; node sizes, the reroute's too, derive from it) and `canvas::PIN_VISUAL` (16:
  the drawn `PortDot`, which wires touch and hover grows), and a dragged or lifted wire takes the nearest pin it may
  end on within `canvas::WIRE_TARGET_RADIUS` (40) screen px unless the pointer is on a refused pin (the fork's
  `CanvasInteraction::wire_target_radius`, `attach_canvas`). A comment with nothing selected is a 400x300 frame at
  the cursor (`Doc::add_frame`, `EMPTY_FRAME_SIZE`; `AddComment(None)` = the view centre,
  `editor::comment_at_view_centre`), as `doc::bounds(∅)` means "all nodes" (paste). A box that fully encloses a
  comment frame and selects all of its `Frame::nodes` marks it in `Doc::enclosed_frames` (`layout::enclosed`), so
  Delete removes it with the nodes. Auto-layout: the base's `layout::auto_layout`, called by Material's
  `autolayout.rs` with its node heights.
- UI is pure Bevy UI + Feathers (`bsn!`); **no egui**. Workarounds for Feathers 0.20 bugs are in `editor_core::fixes`.
- **Feathers widgets, no look-alikes** (decision 0034): use `editor_core::ui::widgets` / Feathers containers and
  controls — never a hand-built widget where Feathers 0.20 has one. Panels are `pane_box` (+ `pane_title`) and the
  right sidebar's groups `subpane_box`; joined buttons are `button_group` (`GroupButton`, `Caption`); labelled bools
  the captioned `widgets::checkbox`; numbers `FeathersNumberInput` with a `SoftLimit` (`props::number_ranged` /
  `number_input`; `HardLimit` only for a real bound). Menus are Feathers menus (`ui::shell::main_menu` +
  `fill_main_menus`, `dialog::open_menu_entries` / `open_icon_menu`) behind a transparent click barrier
  (`dialog::menu_barrier`), so an outside click only closes them; one menu is open at a time. The kept hand-built
  widgets (tabs, tree rows, log rows, palette, tooltip, `scroll_column`) and why are listed in decision 0034.
- **View settings** (decision 0035): `UserConfig::show_left_sidebar` / `show_log` / `show_properties` / `zen_mode`
  (View menu, Ctrl+1 / 2 / 3 / `` ` ``, `Action::ToggleLeftSidebar` / `ToggleLog` / `ToggleProperties` / `ToggleZen`,
  saved at once). Zen mode never rewrites the other flags: `UserConfig::panel_visible(Panel)` (`!zen_mode && own
  flag`) is the single decision point, read by `ui::show_left_sidebar`, `log::show_log` and `show_active_areas`
  (hidden: `Display::None` + `Visibility::Hidden` through `areas::display`). `Panel::Console` (the console with its
  header) is hidden only by Zen, `Panel::Log` (its messages) also by `show_log` off.
- `texture_kit::editor::preview` (`ImagePreviewPlugin<D>`, `PreviewDoc`): the zoom / pan / fit / channel / tiling
  preview shared by the image and Texture tabs; one camera per tab, keyed by `(ViewKindId, image path)`. Game images
  and built-in textures are decoded with `Image::from_buffer` by `GameImageCache<D>`.
- Gradients tab (decision 0037): `gradients_doc.rs` (`GradientsDoc`, `History<GradientCollection>`, editor-only slot /
  `GradientSel` selection), `editors/gradients.rs` (editor topbar, right sidebar with one persistent
  `FeathersListView` of 128 slot rows, `GradientsCtx`), `ui/gradient_view.rs` (bar, stop handles, midpoint diamonds;
  a stop drag starts after `DRAG_THRESHOLD`, 3 px), `ui/gradient_strip.rs` (`GradientStrip`: every gradient preview
  rasterizes the baked row, so it shows what the game samples), `ui/gradient_preview.rs` (the Sample Gradient node
  strip). Stop colors use **one hidden `FeathersColorInput`** (`StopColorInput`) placed over the selected stop and
  opened with `MenuEvent` `Open` (`open_stop_color`); `ValueChange<Color>` → linear `SetStopColor` merged until
  `is_final`; `dialog::menu_barrier` while its popup is open.
- Graph editor topbar: the base's static tools plus Material's `GraphTopbarDynamic` (`editors/graph.rs`,
  `redraw_graph_topbar`: Preview mode `button_group`, generated WESL / Rust buttons; spawned through
  `GraphDomain::topbar_extras`). Header double click: `GraphDomain::header_action` (Material's
  `Editor::header_action`) — open the referenced file, else rename (`Editor::rename_target`), else edit the
  editor-only `Node::comment` `GraphProp::NodeComment`, drawn above the node by `canvas::node_comment`; the body's
  double click and the node menu's Open in tab share `GraphDomain::open_action`), the `canvas::NodeHeader` /
  `FrameTitleStrip` markers, `MaterialAction::TextDialog` → `Submit::SetText` (the sidebar's `set_value`). After a
  wire drop from an output the palette sorts the Named Route declaration first (`PaletteEntry::lead`, a tie-break
  after the search score in `node_graph_base::palette`); a declaration added from the palette opens that rename
  dialog (`MaterialAction::AddNodeAndRename`). A node added from a wire is one undo step (`Editor::connect_checked`).
- `EditorCorePlugin` (`editor_core::EditorPlugins`) runs without a window — logic is tested in `editor_core`'s
  `state::tests` (test-only domains), `material_domain`'s `tests` module, `crates/material_domain/tests/ui_smoke.rs`,
  `node_graph_base`'s `tests/minimal_domain.rs` (a graph domain without Material), `curve_kit`'s `tests/two_domains.rs`,
  `texture_kit`'s `tests/policy_editor.rs` (two kit domains side by side) and `light_domain`'s `tests/editor.rs` (beside
  a test-only Material stand-in) and `tests/pipeline.rs` (the backend; `sample_game_light_is_clean`).
- User config: `<OS config dir>/fe3dback/bevy-shader-editor/config.ron` (window size, recent projects, open tabs as
  `"<domain>:<key>"`, the active domain and last tab per domain, View flags; `editor_core::config::os_config_dir`,
  decision 0018). Version/URL/author/disclaimer constants (`EDITOR_VERSION`, …) are in `editor_core`'s `lib.rs`; the
  logos are `assets/logo*.png`, compiled in (`editor_core::logo`).

### Icons (decision 0013, `IconRef`: decision 0044)

Icons are SVG from the `icon-rs` crate (pinned `=0.4.5`, browse at https://rvdende.github.io/icon-rs/) plus our own
hand-drawn ones, rasterized at runtime with resvg (`crates/editor_core/src/ui/icons.rs`). Never add PNG icons,
`ImageNode`s with asset paths or Feathers' `icon()` for this. The set is open: an `IconRef` is a handle to any
`&'static Icon` (equal by name); core's chrome icons are the `IconId` enum (`IconRef::from(IconId::X)`), a domain's
are `IconRef` constants next to its code (Material: `crates/material_domain/src/icons.rs`, `MATERIAL_ICONS`,
`icons::for_asset` / `for_id`; a kit's: `CURVE_ICON`, `TEXTURE_ICON`). Asset kinds, templates, domains, tree rows
and menu entries carry an `IconRef`. Names must be unique across all sets (the rasterizer cache is keyed by them).

- **Use**: `@icon(IconId::X, 16.0, TEXT)` (any `impl Into<IconRef>`) inside `bsn!` (16 px is the UI standard; the
  color tints line/glyph icons and only sets the alpha of solid/sketch ones). Ready-made widgets in
  `editor_core::ui`: `icon_tool_button(id, label, action, tip)`, icon-only `icon_button(id, action, tip)` (always
  give a tooltip naming the action + hotkey); context menus: `dialog::open_icon_menu` with
  `(Option<IconRef>, label, action)` items. Any widget gets a hover hint with the `tooltip::Tooltip("…")` component.
- **Add an icon-rs icon**: a core (chrome) icon is an `IconId` variant — map it in `IconId::source()` to the constant
  (`icon_rs::FOLDER`, never `icon_rs::get("…")`) and append it to `IconId::ALL` (bump the array length); a
  domain's is `pub const X: IconRef = IconRef::new(&icon_rs::Y)` added to its list (Material: `MATERIAL_ICONS`). The
  tests rasterize every entry of both lists (`assert_icons_rasterize`). To find a name:
  `grep '=> (' ~/.cargo/registry/src/*/icon-rs-0.4.5/src/generated.rs`; SVGs are in that crate's `icons/`. Prefer
  icon-rs; draw only what it lacks. Don't fork icon-rs or copy its `xtask` generator.
- **Draw our own**: put `name.svg` in `crates/editor_core/icons/` (core) or `crates/material_domain/icons/`
  (Material; rules in each `README.md`), copying the root `<svg>` element of a similar icon-rs icon: 24x24 viewBox,
  `fill="none" stroke="#262626" stroke-linejoin="round" stroke-linecap="round"`, `stroke-width` 1.75 for line icons /
  1.1 for solid ones, drawing inside 1.5..22.5. Colors only from `Palette::LIGHT`: ink `#262626`; faces `#ffffff
  #d4d4d4 #a4a4a4 #747474` (solid only); accent `#2563eb` (the accent slot, rendered in `theme::ACCENT`) for the one
  modifier detail (the plus, the arrow), single marks stay ink-only. No `id`s, no gradients/`<defs>`. Then add
  `const X: Icon = custom("name", Kind::Line, include_str!("../../icons/name.svg"));` (an `IconId` variant for a core
  icon, an `IconRef` constant for a domain's) and the const to that crate's `custom_icons_use_only_palette_colours`
  test list.
- **Review a new icon** before using it: render it at 16 and 32 px on the dark panel color, e.g. with a scratch crate
  using `icon-rs` + `resvg` (`icon.recolor(&Palette::DARK)` → `usvg::Tree` → `resvg::render`), next to a neighbouring
  icon-rs icon to compare weight; then check it in the real UI with `SHADER_EDITOR_SCREENSHOT` (see Visual checks).

## Testing notes

- `crates/shader_project/tests/snapshots.rs` auto-generates one material per registered stdlib node plus the composite
  fixtures, compares WESL/Rust byte-for-byte with `tests/snapshots/`, and compiles every `.wesl` with wesl 0.6 + naga 30
  against the **real Bevy 0.20 WESL sources from `~/.cargo/registry`** in forward, forward-without-UV/tangents,
  forward with prepasses + MSAA (3D only) and deferred (`PREPASS_PIPELINE`) variants. Adding a stdlib node therefore
  adds a new snapshot. If Bevy sources are absent from the registry the compile step is skipped with a warning — don't
  mistake that for a pass.
- naga rejects abstract literals outside const context, so scalar defaults are emitted as typed literals `0.5f`
  (`value::wesl_scalar`, decision 0007). Log/Log2 default to 1.0 because naga const-folds `log(0.0)` into an error.
- Codegen must be deterministic (byte-identical output for the same graph; node ids sorted in `BTreeMap`, emitted names
  `n_{id}_{pin}`).

### Visual checks (debug env hooks)

- The hooks live in `editor_core` (`app.rs`: the screenshot and the shell's `SHADER_EDITOR_DEMO`s — about,
  quick_start, open_project, welcome, menu, tab_menu, settings_dirty; `demo.rs`: the shell gestures menu_click /
  menu_close / tab / hover without a pin, and the `Emit` / `Step` helpers) and in `material_domain` (`lib.rs`
  `debug_screenshot`: `SHADER_EDITOR_OPEN` / `SELECT` and the other demos; `demo.rs`: every other gesture).
- Editor: `SHADER_EDITOR_SCREENSHOT=out.png [SHADER_EDITOR_OPEN=env/rock_wet.material.ron[#wesl|#rust]|`
  `@game/textures/rock_albedo.ktx2|@internal/default|common/terrain.gradients.ron]
  [SHADER_EDITOR_SELECT=stdlib::Lerp|c01|<slot>[:s<stop>|:m<segment>]]
  [SHADER_EDITOR_DEMO=palette|wire|dialog|about|quick_start[:N]|texture_channel:N|open_project|welcome|`
  `menu[:<name>]|tab_menu|game_file_menu[:2]|gradient_color|settings_dirty]
  cargo run -- examples/sample_game` — renders, saves the PNG and exits. `SHADER_EDITOR_SELECT` takes a node kind
  (its first node) or a comment frame id; on a gradients tab a slot position (`3`), with `:s<stop>` / `:m<segment>`
  also selecting that stop / midpoint (`0:s1`, `0:m2`). `menu:<name>` opens that main menu (`file`, `edit`, `view`,
  `assets`, `help`); `game_file_menu` opens the RMB menu of the first game image that has a Texture asset (its item
  disabled), `game_file_menu:2` of the first without; `gradient_color` (on a gradients tab) selects stop 0 of the
  selected slot and opens its Feathers color picker; `settings_dirty` (with `SHADER_EDITOR_DOMAIN=project`, which
  opens Project Settings) edits that tab without saving. `SHADER_EDITOR_DOMAIN=<id>` (`material`, `light`, `project`;
  only with `SHADER_EDITOR_SCREENSHOT`) clicks that domain tab on frame 2, after `SHADER_EDITOR_OPEN`; an unknown id
  logs an error. `SHADER_EDITOR_OPEN` is Material's: open a Light tab through a seeded user config (`XDG_CONFIG_HOME`
  with `open: ["light:quillshade/light/day.environment.ron"]`, `active_domain: Some("light")`).
  `SHADER_EDITOR_INPUT=drag|rmb_click|rmb_pan|mmb_pan|slider|color|pin_rmb|node_rmb|node_dblclick|`
  `frame_dblclick|hover|curve_drag|curve_add|texture_zoom|drop_game_file|ctrl_drag_wire|ctrl_wire_hold|wire_drag|`
  `wire_hold|alt_click|box_delete|box_delete_partial|type_field|menu_click|menu_close|tab[:N]`
  (+ `SHADER_EDITOR_INPUT_NODE=<id>`, `SHADER_EDITOR_INPUT_TRACE=1`, `SHADER_EDITOR_ZOOM=<f32>`) plays a scripted mouse
  gesture through winit's messages and logs the outcome (`demo` lines; `slider`/`color` also log the edited value;
  `texture_zoom` works on an image or Texture tab).
  - `slider` drags the topmost number input with a soft limit across most of its on-screen width (scaled by the
    canvas zoom) and logs its value and its node's position (a scrub must not move the node): an inline field on a
    node counts only when it lies wholly inside the visible canvas (at the default fit zoom of rock_wet that is
    `n05.default`; at `SHADER_EDITOR_ZOOM=1` `n06.default`) — the hook does not pan, so bring the field on screen
    with `SHADER_EDITOR_ZOOM`. With `SHADER_EDITOR_SELECT=c01` on rock_wet it drags that comment's Hue field in the
    right sidebar.
  - `drop_game_file` drags the tree row `SHADER_EDITOR_INPUT_FILE` — a Game Assets file as `<path under assets/>`
    (default: the first one) or a Project asset as `quillshade/material/<path>.<kind>.ron` (e.g.
    `quillshade/material/common/pulse.curve.ron`) — onto `SHADER_EDITOR_INPUT_NODE` (default: the canvas centre) and
    logs the graph's texture / curve references and function calls; it may create a Texture asset, so run it on a copy
    of the sample game.
  - `hover` rests on the secondbar's Back button (its tooltip), on the widget whose tooltip starts with
    `SHADER_EDITOR_INPUT_TIP=<prefix>` or, with `SHADER_EDITOR_INPUT_PIN=<node>.<pin>`, on that pin (hovered pins are
    logged).
  - `ctrl_drag_wire` holds Ctrl and drags from `SHADER_EDITOR_INPUT_PIN=<node>.<pin>` (default: the topmost connected
    input) onto `SHADER_EDITOR_INPUT_TO=<node>.<pin>` (default: the nearest pin of the same direction on another node);
    `wire_drag` is the same without Ctrl, `wire_hold` keeps the button down (the shot shows the dragged wire);
    `ctrl_wire_hold` is `ctrl_drag_wire` keeping Ctrl and the button down (the shot shows the lifted wires
    mid-gesture, design §5.3; logs the lifted wires, the pin they snap to and the count of new wires dragged);
    `SHADER_EDITOR_INPUT_AT=<dx>,<dy>` ends these drags that many logical px from the start pin instead (empty
    canvas), `SHADER_EDITOR_INPUT_AT=to:<dx>,<dy>` that far from the target pin (its snap radius, design §6.3).
    `alt_click` Alt+clicks the `SHADER_EDITOR_INPUT_PIN` pin. All log the wires at both pins before and after.
  - `node_dblclick` double-clicks the middle of an unselected node (or `SHADER_EDITOR_INPUT_NODE`), with
    `SHADER_EDITOR_INPUT_AT=header` its header (opens the referenced file, else the rename dialog of a named /
    labelled node, design §5.5, else the node comment dialog);
    `frame_dblclick` double-clicks the title strip of the first comment frame (or `SHADER_EDITOR_INPUT_FRAME=<id>`).
    Both log the active tab and the open dialog.
  - `type_field` clicks the right sidebar's Description field of the open graph, types `ab` and logs the focus and
    the field's text.
  - `menu_click` clicks the mainbar menu `SHADER_EDITOR_INPUT_MENU=<name>` (default `file`); `menu_close` then clicks
    the canvas (lands on the menu's barrier). Both log each main menu's open and active (`Pressed`) state.
  - `tab[:N]` presses Tab N times (default 1, at most 30) and logs after each press the focused entity, its tooltip
    and action, whether a `Display::None` ancestor hides it and the focus ring; the shot shows the ring.
  - `box_delete` drags a selection box from outside the first comment frame (or `SHADER_EDITOR_INPUT_FRAME=<id>`) past
    its far corner and presses Delete; `box_delete_partial` ends the box in the frame's middle. Both log frames, node
    count, selection and enclosed frames before and after.
- Sample game: `cd examples/sample_game && SAMPLE_SCREENSHOT=out timeout 180 cargo run` → `out-1.png` 5 s and
  `out-2.png` 25 s (real time) after the first frame, then exits; edit + `regen` in between to verify hot reload (the
  window updates continuously and without vsync in this mode, so it works unfocused under lavapipe too). Render
  comparison (PSNR) is not automated.

## File format (`*.material.ron` / `*.function.ron`; curves, textures, gradients: decisions 0015, 0024, 0037)

Serialized `Asset { format_version, kind, name, nodes, edges, comments, meta, preview }` (`preview` omitted when
default, decision 0022) with struct names, 2-space indent, nodes in a `BTreeMap` by id. Asset name comes from the file
name (file wins over the `name` field); names must match `[a-z][a-z0-9_]*` (keywords are fine: generated identifiers and
paths are prefixed `inp_`/`out_`/`fun_`/`dir_`/`mat_`/`crv_`, decision 0020; only user path components are prefixed —
`autogen/stdlib/`, `autogen/functions/` and `autogen/curves/` stay unprefixed; only Custom/user-node input names keep
the keyword check). Loading probes `format_version` first; bump it on incompatible changes and add a migration in
`Asset::from_ron` (current version: 1). See decision 0003.

## Rules

- Write all documentation and code comments **in English only** — including `docs/decisions/` entries. Refer to spec
  sections as `spec §N` in comments.
- Keep `README.md` up to date when a change affects anything it documents: CLI commands/flags, hotkeys, file formats,
  diagnostic codes, generated API, crate roles, limitations, dev commands.
- Record non-obvious implementation choices or deviations from the spec as a new `docs/decisions/NNNN-*.md` and add it
  to `docs/decisions/README.md` (never delete an old decision — mark it "superseded by NNNN").
- Never hand-edit autogen output (`examples/sample_game/assets/quillshade/material/{autogen,internal,gradients}/**`,
  `examples/sample_game/crates/quillshade/**`, `crates/shader_project/tests/snapshots/*.snap`) — change the generator
  and regenerate.
