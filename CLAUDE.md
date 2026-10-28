# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`shader-editor`: a node-based shader editor for **Bevy 0.20**, modelled on UE5's Material Editor. Graphs live as RON
files in a game project (`shaders/**`); the tool generates WESL (`assets/shaders/autogen/**`) and a Rust crate
(`crates/shaders`) with material types, baked material / texture handles and `ShadersPlugin`. There is no in-editor
material preview — the game picks up regenerated `.wesl` and the baked manifest via Bevy's asset hot-reload. One binary:
no args → GUI; `regen` / `check` / `diff` → headless CLI.

Source of requirements: `docs/techdoc.md` (the spec; code comments cite it as `spec §<section>`). Deviations
from the spec and implementation choices are recorded in `docs/decisions/NNNN-*.md` — read the relevant one before
changing codegen, file format or editor architecture. Manual pre-release UI checklist: `docs/ui-checklist.md`.

## Environment

- Toolchain comes from the nix dev shell (`flake.nix`, `.envrc`): Rust ≥ 1.97.1 (MSRV, required by bevy_noodle),
  `kache` as rustc wrapper. `rust-toolchain.toml.disabled` is intentionally disabled. **Never run `nix` commands** —
  use plain `cargo`.
- Debug builds use `bevy/dynamic_linking`; start binaries through `cargo run` (it puts `libbevy_dylib` on the path).
  Running the built binary directly only works for release builds.
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
cargo run -- check examples/sample_game --compile        # headless validation + wesl/naga compile
cargo run -- regen examples/sample_game                  # regenerate; prints create/modify/delete lines
cargo run -- diff  examples/sample_game

INSTA_UPDATE=always cargo test -p shader_project --test snapshots  # accept intended codegen changes
```

CLI exit codes: `0` ok, `1` validation errors, `2` project failed to open / write error. Diagnostics go to stderr as
`path:node[.pin]:CODE: text`.

CI (`.github/workflows/ci.yml`, Linux/Windows/macOS) additionally runs `regen examples/sample_game` and fails on
`git diff` or a new untracked file (`git status --porcelain`) — **any codegen change must be followed by regenerating
the sample game and committing the result** (new files included), then
`cd examples/sample_game && cargo clippy --workspace --all-targets -- -D warnings` must pass on the generated crate.

`cargo run -p shader_project --example make_sample -- examples/sample_game` rewrites the sample's `shaders/` from the
fixture graphs in `crates/shader_project/src/fixtures.rs` and regenerates — it overwrites hand edits to fixture-named
assets, so check `git status` first.

## Architecture

Cargo workspace; root package is only the `shader-editor` binary (`src/main.rs`, clap). `examples/sample_game` is a
**separate workspace** (excluded from the root one) — build it from its own directory, ideally with
`CARGO_TARGET_DIR=<repo>/target/sample`.
`vendor/bevy_noodle` is a local fork of bevy_noodle 0.5.0 (workspace member, path dependency, same crate name,
decision 0034): canvas / gesture behaviour (drags, snapping, wire gestures, port looks) goes **into the fork**, never
into app-side workarounds; every fork change is appended to its `FORK.md` ("Changes from upstream", what and why), is
off by default and keeps upstream's tests passing, so an upgrade is: vendor the new release, re-apply the `FORK.md`
entries. The fork's own strict `clippy.toml` applies there (no `unwrap` / `expect` / indexing outside tests).

Crate dependency chain (lower crates never depend on higher ones; only the last one depends on Bevy):

```
shader_graph_core  →  shader_stdlib  →  shader_codegen_wesl  →  shader_codegen_rust  →  shader_project  →  shader_editor_app
(model, types,        (node defs,        (IR → WESL, wesl+naga   (IR → crates/shaders)    (project layout,     (Bevy UI + Feathers
 registry, diag, IR)   stdlib .wesl)      compile check)                                   regen, writer, ops)  + bevy_noodle canvas)
```

Pipeline (`shader_project::regen::regenerate`): load assets → validate → `build_ir` → WESL + Rust codegen → batched
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
  `autogen/stdlib/`. User nodes (`shaders/.nodes/*.ron`, kind `user::<id>`) go through the same trait
  (`shader_stdlib::user`).
- **Curves** (`shaders/**/*.curve.ron`, decision 0015) are not graphs: `shader_graph_core::curve::Curve`,
  `AssetType::Curve`, kept in `AssetSet::curves` (not `assets`). Each becomes its own module
  `autogen/curves/dir_…/crv_<name>.wesl`; the `stdlib::SampleCurve` node (param of `ParamKind::Curve`) imports
  `sample_curve_simple`/`sample_curve_catmullrom` from it under a per-curve alias (`curve_fn_alias`).
  `curve_refs` finds the nodes referring to a curve (deps, rename). `Curve::sample` is the reference math, in the output
  range `min..max` (decision 0038) baked into the module's `POINTS` — the stored points stay normalized.
- **Global constant tables** (`shaders/**/*.consts.ron`, decision 0017) are not graphs either:
  `shader_graph_core::consts::GlobalConsts`, `AssetType::Consts`, `AssetSet::consts`; they generate no file. The
  `stdlib::Global*` nodes (param `const`, `ParamKind::GlobalConst(ty)`) inline the value as a literal through
  `Env::consts`; `regen::collect_consts` resolves names (unique project-wide), `const_refs` finds readers (deps,
  clearing on type change / delete). `regen::wesl_file` / `module_of` are `None` for tables.
- **Texture assets** (`shaders/**/*.texture.ron`, decision 0024): `shader_graph_core::texture::TextureAsset` (image
  path under the game's `assets/`, `.ktx2` / `.png` only, + loader settings), `AssetType::Texture`,
  `AssetSet::textures`; no file of their own. One game file — one asset (E011 on every sharer); E012 path rules,
  E013 sampler rules, E014 KTX2 cube map / array / 3D, W109 missing file
  (`regen::texture_diagnostics`). Nodes reference them by id through
  `ParamKind::Texture` (`texture_refs`: deps, rename rewrite, delete clears → W110 + `@internal/default`; missing →
  E006); old image-path defaults fail in `assets::load`. **Internal textures** (`shader_stdlib::internal`,
  `@internal/default`) are embedded PNGs written to `assets/shaders/internal/` on every regen.
- **Gradient collections** (`shaders/**/*.gradients.ron`, decision 0037) are not graphs either:
  `shader_graph_core::gradient::GradientCollection` (exactly 128 slots with stable random ids `1..=i32::MAX`, ≤ 32
  stops, `mid` on the left stop; `Slot::sample` = OKLab reference math in `f64`), `AssetType::Gradients`,
  `AssetSet::gradients`; no WESL / Rust file. Regen bakes each into a 128×128 sRGB PNG (`bake_rgba8`,
  `shader_project::gradient_png`, byte-deterministic `png` + fdeflate) in `assets/shaders/gradients/<path>.png`,
  owned like `internal/` through `.autogen` (only while the project has a collection), listed in the manifest as
  `@gradients/<path>` (`BakedTextures.gradients`). `stdlib::SampleGradient` (params `ParamKind::GradientCollection` +
  `GradientSlot`) samples it through `texture.wesl::sg_sample_gradient` at texel centres with a row literal = the
  slot's position (`GradientLookup`, `Env::gradients`); one binding `gc_<path>` per collection per material
  (`TextureSource::Gradient`), hidden trailing `gc_*` arguments in functions (`FunctionSig::gradients`, after `ft_*`),
  E006 on identifier clashes. W111: no gradient (white, no binding) or an empty one (still sampled, so slot contents
  never change code). `gradient_refs`: deps (a reorder regenerates the row literal only), rename rewrite, delete
  clears collection + slot. Color / stop / slot edits change only the PNG.
- **Baked manifest** (decision 0025): regen writes `assets/shaders/autogen/baked.ron` (`shader_codegen_rust::manifest`:
  every valid Texture asset and gradient collection + every material's values / texture ids). The generated crate embeds
  it (`COMPILED_MANIFEST`, `include_str!`) and overlays the runtime file on hot reload;
  `crates/shaders/src/baked/mod.rs` is the static template `crates/shader_codegen_rust/templates/baked_mod.rs`
  (`BakedMaterialsLoader`, progress, overlay), `baked/{materials,textures}.rs` come from `shader_codegen_rust::baked`.
  Generated `new` / `set_*` are `pub(crate)`; games use `BakedMaterials` / `BakedTextures` (nested fields, or `by_path`
  / `get` by asset id, decision 0030). **Layout guard** (decision 0029): every material WESL compiles only with its
  `Ir::layout_feature()` (`SG_LAYOUT_<hash>`), which the generated `specialize` enables, so a hot-reloaded shader with
  other bindings fails to compile instead of crashing the game; `check::Files::enable` does the same for the compile
  check. The manifest's `layout` / `bindings` let the runtime match texture bindings by position.
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
  internal textures (`shader_stdlib::internal`) go to `assets/shaders/internal/` and baked gradient collections to
  `assets/shaders/gradients/`, each owned through the marker file `.autogen` (no marker in a non-empty dir → E008,
  nothing touched there). The editor never edits the game's root
  `Cargo.toml` (W105 instead).
- **Errors block only their own asset**; the rest of the project still regenerates.
- **Material kinds**: Pbr3d → `ExtendedMaterial<StandardMaterial, {Name}Ext>` (shadows/prepass/deferred for free);
  Unlit3d → own `impl Material`; Sprite2d → `MaterialExtension2d` over `SpriteMaterial`. World Position Offset generates
  a custom vertex shader copied from Bevy 0.20 `mesh.wesl` (decision 0006) — fragile across Bevy patch releases.

### Editor (`crates/shader_editor_app`)

Window layout. Requests like "add X to the editor topbar" use exactly these names; use them in code comments, docs
and commit messages too:

```
┌──────────────────────────────────────────────────────────────────────────────┐
│ menu_bar                                                                     │
├────────────────┬───────────────────────────────────────┬─────────────────────┤
│ left sidebar   │ tabs bar                              │ right sidebar       │
│                ├───────────────────────────────────────┤                     │
│ ┌────────────┐ │ editor topbar                         │ editor kind         │
│ │project tree│ ├───────────────────────────────────────┤ additional content  │
│ │            │ │ editor content                        │ (per editor kind)   │
│ └────────────┘ │                                       │                     │
│ ┌────────────┐ ├───────────────────────────────────────┤                     │
│ │assets tree │ │ editor statusbar                      │                     │
│ │            │ ├───────────────────────────────────────┤                     │
│ │            │ │ console/logs                          │                     │
│ └────────────┘ │                                       │                     │
└────────────────┴───────────────────────────────────────┴─────────────────────┘
```

- **menu_bar** — global: logo, main menu (File / Edit / View / Assets / Help), global actions; never buttons of one
  editor kind.
- **left sidebar** — global: **project tree** (`shaders/**` assets) above **assets tree** (the game's `assets/`,
  read-only, plus the virtual Internal folder).
- **tabs bar** — open tabs; visually separate from the editor topbar below it.
- **editor topbar** / **editor content** / **editor statusbar** / **right sidebar** — owned by the active tab's *editor
  kind* (graph, text, curve, consts, image, texture, gradients — `tab::Tab` / `ViewKind`): its buttons and info, its
  main view, its status line, its additional content (properties of the selection, hints…).
- **console/logs** — global: diagnostics and messages, under the editor statusbar.

- **Editor kinds** (`editors/`, decisions 0031, 0032): one module per kind (`editors/<kind>.rs`: graph, text, curve,
  consts, image, texture, gradients) holds **every trait impl of that kind** — `doc_kind::TabDoc` on its document type,
  and on a zero-sized `<Kind>Editor`:
  - `editors::EditorAreas`: `VIEW`, `SIDEBAR_TITLE`, `topbar()` / `sidebar()` → `Option<impl Scene>` (`None`: the strip
    is hidden for the kind), `build(app)` registering the kind's redraw systems, `input(app)` its input context.
  - `editors::EditorInput` (kinds with hotkeys: graph, curve, text, gradients): `type Ctx` and `bindings()`.

  `editors::for_each_kind` (`KindVisitor`) is the one list of kinds; `AreasPlugin` and `InputContextsPlugin` both
  visit it. The editor statusbar comes from `TabDoc::status` → `StatusInfo` (filled from a `StatusCtx` by
  `Editor::status_info`). Adding a kind: decision 0031's recipe.
- **Areas** (`ui/areas.rs`): each area is spawned once in its strip (`EditorTopbar`, `RightSidebar`,
  `EditorStatusbar`) inside an `AreaOf(ViewKind)` container; `show_active_areas` sets `Node::display` (and
  `Visibility::Hidden`, which Bevy's tab navigation needs; written only on change) so only the active kind's shows and a
  strip it has nothing for takes no space.
  Bodies (`*ViewBar`, `*SidebarBody`) are refilled by the kind's systems keyed on what they show (sidebars:
  `(ui.props, active)` via `SidebarKey`), never every frame. Property widgets and their observers stay in
  `ui/props.rs`; editor content views stay in `ui::DocArea`.
- **Input contexts** (`editors/main_input.rs`, decisions 0032, 0034, 0035, 0041): bevy_enhanced_input. One `InputRoot`
  entity carries `MainCtx` (save, undo / redo, close / next tab, open, quit, the View toggles Ctrl+1
  `ToggleLeftSidebar` / Ctrl+2 `ToggleLog` / Ctrl+3 `ToggleProperties` / `` ` `` `ToggleZen`, `NavBack` / `NavForward`
  on Ctrl+[ / Ctrl+], the mouse side buttons and the Browser keys — `on_nav` drops them while typing or with L / R / M
  held, as the gate hides only the keyboard; always active, priority 2) and one context per
  kind (`GraphCtx` — palette on Space, Esc `CancelWire`, …; `CurveCtx`, `TextCtx`, `GradientsCtx`; priority 1) that
  `sync_input_context` activates only while the active tab is of that kind — so a hotkey needs no "is this my tab"
  guard. The shared edit actions (`EditCopy`, `EditDelete`, …) are bound by each kind that supports them and have one
  global handler each. Build actions with `hotkey::<A>()` (`consume_input` + `require_reset`) and the `ctrl` /
  `ctrl_shift` / `shift` / `plain` binding helpers ("Ctrl" = Ctrl or Super); handlers observe `On<Start<A>>` (`on_combo`
  / `on_plain`) and push `state::Action`s; a hold-to-fire key is `held_hotkey::<A>(bindings, secs)` (a one-shot `Hold`,
  handlers observe `Fire`: hold C `AddComment`, which arms only on a fresh C press so a C left down after Ctrl+C never
  fires). Plain-key handlers ignore the key while Ctrl, Alt or Super is held (`keys::no_command_mods`). `gate_keyboard`
  (PreUpdate, before `InputFocusSystems::Dispatch`) hides the keyboard from all contexts while typing (text field
  focused, palette, dialog, menu or popup open; a scrubbed number field only while LMB is held, `typing_into`; Space /
  Enter while a focused button or checkbox takes them, `PRESS_KEYS`, and arrows / Home / End / Space / Enter while a
  focused list box takes them, `LIST_KEYS`: the widget wins). Keyboard focus is Bevy's default (decision 0035): Tab
  walks every tabbable widget, a canvas click clears the focus; only additions are kept — `tab_into_number_input`,
  `drop_hidden_focus`, the palette's modal `TabGroup`, `Visibility::Hidden` on hidden areas — plus one exception,
  Ctrl+Tab (`NextTab`) clears the focus (`drop_focus`). Activating a context re-inserts its bindings so a key held
  across a tab switch does not fire (relies on bevy_enhanced_input internals — re-check `tests/input_contexts.rs` on
  upgrades). Mouse gestures stay picking observers and read modifiers via `keys::mods()` (`keys.rs`, below both `ui` and
  `editors`).

- The model (`shader_graph_core::Asset` inside `doc::Doc`) is the source of truth. Widgets push `state::Action`s →
  `Editor::apply` mutates documents → the bevy_noodle canvas is **rebuilt from the model** on revision change (only when
  no gesture is active). Canvas gestures arrive as `EditApplied` with `EditOrigin::Interaction` and are translated into
  Actions; edits the canvas makes during its own rebuild are ignored.
- Undo/redo/copy/paste are model-level (snapshots of `Asset`, 200 steps, consecutive edits of one field merged), not
  bevy_noodle's scene history.
- Connection validity is decided by `doc::connection_error` via a custom `ConnectionCheck` (bevy_noodle ports are all
  `PortType::ANY`).
- Ctrl+drag from a pin is the fork's move-wires gesture (`CanvasInteraction::move_wires_keys`, design §5.3): it lifts
  the pin's wires (`MovingEdges`) without touching the model; candidate pins come from a `MoveCheck` observer running
  `Editor::plan_move_wires` (`canvas::check_move`), and the drop's `GraphEdit::MoveEdges` is vetoed and turned into one
  `Action::MoveWires` (`canvas::request_move_wires`); a refused drop on a pin logs why
  (`ui::input::lifted_wires_dropped`). Esc (`GraphCtx` `CancelWire`) ends any wire drag by despawning
  `GraphQuery::wire_of(canvas)`, as noodle's own cancel does.
- Tabs are `tab::Tab` (graph `Doc`, read-only `TextDoc` of a generated file, `CurveDoc`, `ConstsDoc`, read-only
  `ImageDoc` of a game image decoded through `game_image::GameImageCache`, `TextureDoc` of a Texture asset — read-only
  for a built-in `@internal/…` texture, `GradientsDoc` of a gradient collection), each implementing `doc_kind::TabDoc`
  (title, icon, watermark, dirty, save, reload, rename, config key, `override_into` for validation, editor statusbar
  data `status` → `StatusInfo`; `Tab::doc()`); `Editor::doc()` is `None` on a non-graph tab, `Editor::curve_doc()` /
  `consts_doc()` / `texture_doc()` / `gradients_doc()` give the others. The editable kinds share `history::History<T>`.
  Each content kind has a view in `ui::DocArea` tagged `ViewOf(ViewKind)` (decision 0031 lists what a new kind needs).
  WESL/Rust highlighting is our own lexer (`highlight.rs`).
- Module seams (decision 0026): `state/*` (types, tabs, regen, watch, apply, `*_ops`, tests), `doc_kind.rs` (`TabDoc`),
  `editors/` (see "Editor kinds" above; the document types stay in `doc.rs`, `curve_doc.rs`, `tab.rs`, …;
  `editors/mod.rs` holds the shared traits and data), `ui/node_preview.rs` (generic in-node preview: curve / const /
  texture / gradient kinds in `kinds()`), `ui/tree.rs` (`TreeSource`: Project + read-only Game Assets with the virtual
  Internal folder), `game_image.rs` (`GameImageCache`: game images and internal textures decoded with
  `Image::from_buffer`), `droppable.rs` (`DroppableFile` / `Dropped`: dragging images, Texture, Function, Curve and
  gradient collection assets onto a graph; a function dropped into itself is refused through `search::is_self_call`, as
  in the palette), `state/nav.rs` (`NavHistory`, decision 0041: the in-memory Back / Forward list of tabs and graph
  camera views, fed by `Editor::follow_active` and the canvas's settled-view polling).
- Canvas details (decisions 0033, 0034, 0035): node positions snap to `doc::GRID_SNAP` (10) — pointer drags through the
  bevy_noodle fork's `CanvasInteraction::snap` (the grabbed node; `attach_canvas`), new / pasted nodes in
  `Editor::add_node` / `paste`; loaded positions are never rewritten. Comment frames are `NodeFrame`s: dragging one
  moves the nodes inside it in the same edits. Node emphasis (selection, pulsing preview, diagnostics) is a bevy_ui
  `Outline` written by `node_outlines` every frame, and pin hover the fork's `PortHoverStyle` on the canvas
  (`rebuild_canvas` carries `Hovered` to a rebuilt pin's new port) — neither rebuilds the canvas. Pins are the
  fork's `kit::port_sized` with a `kit::PortSize` of `canvas::PIN_PHYSICAL` (24: the port box — hit area, wire
  anchor, row room; node sizes, the reroute's too, derive from it) and `canvas::PIN_VISUAL` (16: the drawn `PortDot`,
  which wires touch and hover grows),
  and a dragged or lifted wire takes the nearest pin it may end on within `canvas::WIRE_TARGET_RADIUS` (40) screen px
  unless the pointer is on a refused pin (the fork's `CanvasInteraction::wire_target_radius`, `attach_canvas`). A
  comment with nothing selected is a 400x300 frame at the cursor (`Doc::add_frame`, `EMPTY_FRAME_SIZE`;
  `AddComment(None)` = the view centre, `comment_at_view_centre`), as `doc::bounds(∅)` means "all nodes" (paste). A box
  that fully encloses a comment frame and selects all of its `Frame::nodes` marks it in `Doc::enclosed_frames`, so
  Delete removes it with the nodes.
- UI is pure Bevy UI + Feathers (`bsn!`); **no egui**. Workarounds for Feathers 0.20 bugs are in `fixes.rs`.
- **Feathers widgets, no look-alikes** (decision 0034): use `ui::widgets` / Feathers containers and controls — never a
  hand-built widget where Feathers 0.20 has one. Panels are `pane_box` (+ `pane_title`) and the right sidebar's groups
  `subpane_box`; joined buttons are `button_group` (`GroupButton`, `Caption`); labelled bools the captioned
  `widgets::checkbox`; numbers `FeathersNumberInput` with a `SoftLimit` (`props::number_ranged` / `number_input`;
  `HardLimit` only for a real bound). Menus are Feathers menus (`ui::main_menu` + `fill_main_menus`,
  `dialog::open_menu_entries` / `open_icon_menu`) behind a transparent click barrier (`dialog::menu_barrier`), so an
  outside click only closes them; one menu is open at a time. The kept hand-built widgets (tabs, tree rows, log rows,
  palette, tooltip, `scroll_column`) and why are listed in decision 0034.
- **View settings** (decision 0035): `UserConfig::show_left_sidebar` / `show_log` / `show_properties` / `zen_mode`
  (View menu, Ctrl+1 / 2 / 3 / `` ` ``, `Action::ToggleLeftSidebar` / `ToggleLog` / `ToggleProperties` / `ToggleZen`,
  saved at once). Zen mode never rewrites the other flags: `UserConfig::panel_visible(Panel)` (`!zen_mode && own
  flag`) is the single decision point, read by `ui::show_left_sidebar`, `log::show_log` and `show_active_areas`
  (hidden: `Display::None` + `Visibility::Hidden` through `areas::display`). `Panel::Console` (the console with its
  header) is hidden only by Zen, `Panel::Log` (its messages) also by `show_log` off.
- `ui/image_preview.rs` (`ImagePreviewPlugin`, `doc_kind::PreviewDoc`): the zoom / pan / fit / channel / tiling preview
  shared by the image and Texture tabs; one camera per tab, keyed by `(ViewKind, image path)`.
- Gradients tab (decision 0037): `gradients_doc.rs` (`GradientsDoc`, `History<GradientCollection>`, editor-only slot /
  `GradientSel` selection), `editors/gradients.rs` (editor topbar, right sidebar with one persistent
  `FeathersListView` of 128 slot rows, `GradientsCtx`), `ui/gradient_view.rs` (bar, stop handles, midpoint diamonds;
  a stop drag starts after `DRAG_THRESHOLD`, 3 px), `ui/gradient_strip.rs` (`GradientStrip`: every gradient preview
  rasterizes the baked row, so it shows what the game samples), `ui/gradient_preview.rs` (the Sample Gradient node
  strip). Stop colors use **one hidden `FeathersColorInput`** (`StopColorInput`) placed over the selected stop and
  opened with `MenuEvent` `Open` (`open_stop_color`); `ValueChange<Color>` → linear `SetStopColor` merged until
  `is_final`; `dialog::menu_barrier` while its popup is open.
- Graph editor topbar: static tools plus `GraphTopbarDynamic` (`redraw_graph_topbar`: Preview mode `button_group`,
  generated WESL / Rust buttons). Rename by double click: `Editor::rename_target` / `header_action`, the
  `canvas::NodeHeader` / `FrameTitleStrip` markers, `Action::TextDialog` → `Submit::SetText` (the sidebar's
  `set_value`). After a wire drop from an output the palette sorts the Named Route declaration first
  (`palette::results_for`, a tie-break after the search score); a declaration added from the palette opens that rename
  dialog (`Action::AddNodeAndRename`). A node added from a wire is one undo step (`Editor::connect_checked`).
- `EditorCorePlugin` runs without a window — logic is tested in `state::tests` and `tests/ui_smoke.rs`.
- User config: `<OS config dir>/fe3dback/bevy-shader-editor/config.ron` (window size, recent projects, open tabs, View
  flags; `config::os_config_dir`, decision 0018). Version/URL/author/disclaimer constants (`EDITOR_VERSION`, …) are in
  `shader_editor_app`'s `lib.rs`; the logos are `assets/logo*.png`, compiled in (`ui::logo`).

### Icons (decision 0013)

Icons are SVG from the `icon-rs` crate (pinned `=0.4.5`, browse at https://rvdende.github.io/icon-rs/) plus our own
hand-drawn ones, rasterized at runtime with resvg (`crates/shader_editor_app/src/ui/icons.rs`). Never add PNG icons,
`ImageNode`s with asset paths or Feathers' `icon()` for this.

- **Use**: `@icon(IconId::X, 16.0, TEXT)` inside `bsn!` (16 px is the UI standard; the color tints line/glyph icons
  and only sets the alpha of solid/sketch ones). Ready-made widgets in `ui/mod.rs`: `icon_tool_button(id, label,
  action, tip)`, icon-only `icon_button(id, action, tip)` (always give a tooltip naming the action + hotkey);
  context menus: `dialog::open_icon_menu` with `(Option<IconId>, label, action)` items. Any widget gets a hover hint
  with the `ui::tooltip::Tooltip("…")` component. Asset-kind icons: `IconId::for_asset` / `IconId::for_new`.
- **Add an icon-rs icon**: add an `IconId` variant, map it in `IconId::source()` to the constant (`icon_rs::FOLDER`,
  never `icon_rs::get("…")`), and append it to `IconId::ALL` (bump the array length) — the tests rasterize every
  entry. To find a name: `grep '=> (' ~/.cargo/registry/src/*/icon-rs-0.4.5/src/generated.rs`; SVGs are in that
  crate's `icons/`. Prefer icon-rs; draw only what it lacks. Don't fork icon-rs or copy its `xtask` generator.
- **Draw our own**: put `name.svg` in `crates/shader_editor_app/icons/` (rules in its `README.md`), copying the root
  `<svg>` element of a similar icon-rs icon: 24x24 viewBox, `fill="none" stroke="#262626"
  stroke-linejoin="round" stroke-linecap="round"`, `stroke-width` 1.75 for line icons / 1.1 for solid ones, drawing
  inside 1.5..22.5. Colors only from `Palette::LIGHT`: ink `#262626`; faces `#ffffff #d4d4d4 #a4a4a4 #747474` (solid
  only); accent `#2563eb` (the accent slot, rendered in `ui::ACCENT`) for the one modifier detail (the plus, the arrow),
  single marks stay ink-only. No `id`s, no gradients/`<defs>`. Then add
  `const X: Icon = custom("name", Kind::Line, include_str!("../../icons/name.svg"));`, the `IconId` variant, and the
  const to the `custom_icons_use_only_palette_colours` test list.
- **Review a new icon** before using it: render it at 16 and 32 px on the dark panel color, e.g. with a scratch crate
  using `icon-rs` + `resvg` (`icon.recolor(&Palette::DARK)` → `usvg::Tree` → `resvg::render`), next to a neighbouring
  icon-rs icon to compare weight; then check it in the real UI with `SHADER_EDITOR_SCREENSHOT` (see Visual checks).

## Testing notes

- `crates/shader_project/tests/snapshots.rs` auto-generates one material per registered stdlib node plus the composite
  fixtures, compares WESL/Rust byte-for-byte with `tests/snapshots/`, and compiles every `.wesl` with wesl 0.6 + naga 30
  against the **real Bevy 0.20 WESL sources from `~/.cargo/registry`** in forward, forward-without-UV/tangents and
  deferred (`PREPASS_PIPELINE`) variants. Adding a stdlib node therefore adds a new snapshot. If Bevy sources are absent
  from the registry the compile step is skipped with a warning — don't mistake that for a pass.
- naga rejects abstract literals outside const context, so scalar defaults are emitted as typed literals `0.5f`
  (`value::wesl_scalar`, decision 0007). Log/Log2 default to 1.0 because naga const-folds `log(0.0)` into an error.
- Codegen must be deterministic (byte-identical output for the same graph; node ids sorted in `BTreeMap`, emitted names
  `n_{id}_{pin}`).

### Visual checks (debug env hooks)

- Editor: `SHADER_EDITOR_SCREENSHOT=out.png [SHADER_EDITOR_OPEN=env/rock_wet.material.ron[#wesl|#rust]|@game/textures/rock_albedo.ktx2|@internal/default|common/terrain.gradients.ron]
  [SHADER_EDITOR_SELECT=stdlib::Lerp|c01|<slot>[:s<stop>|:m<segment>]]
  [SHADER_EDITOR_DEMO=palette|wire|dialog|about|quick_start[:N]|texture_channel:N|open_project|welcome|menu[:<name>]|tab_menu|game_file_menu[:2]|gradient_color]
  cargo run -- examples/sample_game` — renders, saves the PNG and exits. `SHADER_EDITOR_SELECT` takes a node kind
  (its first node) or a comment frame id; on a gradients tab a slot position (`3`), with `:s<stop>` / `:m<segment>`
  also selecting that stop / midpoint (`0:s1`, `0:m2`). `menu:<name>` opens that main menu (`file`, `edit`, `view`,
  `assets`, `help`); `game_file_menu` opens the RMB menu of the first game image that has a Texture asset (its item
  disabled), `game_file_menu:2` of the first without; `gradient_color` (on a gradients tab) selects stop 0 of the
  selected slot and opens its Feathers color picker.
  `SHADER_EDITOR_INPUT=drag|rmb_click|rmb_pan|mmb_pan|slider|color|pin_rmb|node_rmb|node_dblclick|frame_dblclick|hover|curve_drag|curve_add|texture_zoom|drop_game_file|ctrl_drag_wire|ctrl_wire_hold|wire_drag|wire_hold|alt_click|box_delete|box_delete_partial|type_field|menu_click|menu_close|tab[:N]`
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
    (default: the first one) or a Project asset as `shaders/<path>.<kind>.ron` (e.g.
    `shaders/common/pulse.curve.ron`) — onto `SHADER_EDITOR_INPUT_NODE` (default: the canvas centre) and logs the
    graph's texture / curve references and function calls; it may create a Texture asset, so run it on a copy of the
    sample game.
  - `hover` rests on the menu_bar's Back button (its tooltip), on the widget whose tooltip starts with
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
    `SHADER_EDITOR_INPUT_AT=header` its header (the rename dialog of a named / labelled node, design §5.5);
    `frame_dblclick` double-clicks the title strip of the first comment frame (or `SHADER_EDITOR_INPUT_FRAME=<id>`).
    Both log the active tab and the open dialog.
  - `type_field` clicks the right sidebar's Description field of the open graph, types `ab` and logs the focus and
    the field's text.
  - `menu_click` clicks the menu_bar menu `SHADER_EDITOR_INPUT_MENU=<name>` (default `file`); `menu_close` then clicks
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
- Never hand-edit autogen output (`examples/sample_game/assets/shaders/{autogen,internal,gradients}/**`,
  `examples/sample_game/crates/shaders/**`, `crates/shader_project/tests/snapshots/*.snap`) — change the generator and
  regenerate.
