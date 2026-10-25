# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`shader-editor`: a node-based shader editor for **Bevy 0.20**, modelled on UE5's Material Editor. Graphs live as RON
files in a game project (`shaders/**`); the tool generates WESL (`assets/shaders/autogen/**`) and a Rust crate
(`crates/shaders`) with material types, constructors and `ShadersPlugin`. There is no in-editor preview — the game
picks up regenerated `.wesl` via Bevy's asset hot-reload. One binary: no args → GUI; `regen` / `check` / `diff` →
headless CLI.

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
cargo test --workspace                                   # everything (unit, pipeline, snapshots + WESL compile, UI smoke)
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
`git diff` — **any codegen change must be followed by regenerating the sample game and committing the result**, then
`cd examples/sample_game && cargo clippy --workspace --all-targets -- -D warnings` must pass on the generated crate.

`cargo run -p shader_project --example make_sample -- examples/sample_game` rewrites the sample's `shaders/` from the
fixture graphs in `crates/shader_project/src/fixtures.rs` and regenerates — it overwrites hand edits to fixture-named
assets, so check `git status` first.

## Architecture

Cargo workspace; root package is only the `shader-editor` binary (`src/main.rs`, clap). `examples/sample_game` is a
**separate workspace** (excluded from the root one) — build it from its own directory, ideally with
`CARGO_TARGET_DIR=<repo>/target/sample`.

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
  `@binding(100)`, textures/samplers `101/102`, `103/104`, …
- **Node registry**: every node implements `NodeDef` (`shader_graph_core::registry`). Stdlib nodes are Rust code built
  with the closure builder in `crates/shader_stdlib/src/def.rs` (`def(id, name, cat).kw(..).param(..).build(sig, emit)`)
  and registered per category in `crates/shader_stdlib/src/nodes/*.rs`. Reusable WESL helpers live in
  `crates/shader_stdlib/wesl/*.wesl`, embedded via `include_str!` (`MODULES` in `lib.rs`) and always written to
  `autogen/stdlib/`. User nodes (`shaders/.nodes/*.ron`, kind `user::<id>`) go through the same trait
  (`shader_stdlib::user`).
- **Curves** (`shaders/**/*.curve.ron`, decision 0015) are not graphs: `shader_graph_core::curve::Curve`,
  `AssetType::Curve`, kept in `AssetSet::curves` (not `assets`). Each becomes its own module
  `autogen/curves/<path>.wesl`; the `stdlib::SampleCurve` node (param of `ParamKind::Curve`) imports
  `sample_curve_simple`/`sample_curve_catmullrom` from it under a per-curve alias (`curve_fn_alias`).
  `curve_refs` finds the nodes referring to a curve (deps, rename). `Curve::sample` is the reference math.
- **Node kinds** are string paths: `stdlib::*`, `param::*`, `output::*`, `function::<sub/name>`, `user::<id>`.
- **Type conversions** (broadcast, truncation, Color↔Float4, Int→Float) are applied in codegen on the edge, not as
  inserted nodes (`shader_graph_core::types::conversion`); truncation emits W104.
- **Functions** are separate WESL modules imported by materials via relative `super::` paths (`codegen_wesl::paths`) —
  no inlining (decision 0002). Builtins a function needs (UV, time, world position…) become trailing hidden `b_*`
  parameters filled in by the caller (decision 0005). Changing a function regenerates its dependents (`DepGraph`).
- **Writer** (`shader_project::writer`): all files go to temp first, then a series of renames; files are rewritten only
  if content changed (avoids spurious hot-reloads); orphans with the autogen marker are deleted; files *without* the
  `// AUTOGENERATED by shader-editor v…` marker are never touched (E008). The editor never edits the game's root
  `Cargo.toml` (W105 instead).
- **Errors block only their own asset**; the rest of the project still regenerates.
- **Material kinds**: Pbr3d → `ExtendedMaterial<StandardMaterial, {Name}Ext>` (shadows/prepass/deferred for free);
  Unlit3d → own `impl Material`; Sprite2d → `MaterialExtension2d` over `SpriteMaterial`. World Position Offset generates
  a custom vertex shader copied from Bevy 0.20 `mesh.wesl` (decision 0006) — fragile across Bevy patch releases.

### Editor (`crates/shader_editor_app`)

- The model (`shader_graph_core::Asset` inside `doc::Doc`) is the source of truth. Widgets push `state::Action`s →
  `Editor::apply` mutates documents → the bevy_noodle canvas is **rebuilt from the model** on revision change (only when
  no gesture is active). Canvas gestures arrive as `EditApplied` with `EditOrigin::Interaction` and are translated into
  Actions; edits the canvas makes during its own rebuild are ignored.
- Undo/redo/copy/paste are model-level (snapshots of `Asset`, 200 steps, consecutive edits of one field merged), not
  bevy_noodle's scene history.
- Connection validity is decided by `doc::connection_error` via a custom `ConnectionCheck` (bevy_noodle ports are all
  `PortType::ANY`).
- Tabs are `tab::Tab` (graph `Doc`, read-only `TextDoc` of a generated file, `CurveDoc`); `Editor::doc()` is
  `None` on a non-graph tab, `Editor::curve_doc()` gives the curve one. Both document kinds share
  `history::History<T>`. Each content kind has a view in `ui::DocArea` tagged `ViewOf(ViewKind)` (decision 0014 lists what
  a new kind needs). WESL/Rust highlighting is our own lexer (`highlight.rs`).
- UI is pure Bevy UI + Feathers (`bsn!`); **no egui**. Workarounds for Feathers 0.20 bugs are in `fixes.rs`.
- `EditorCorePlugin` runs without a window — logic is tested in `state::tests` and `tests/ui_smoke.rs`.
- User config: `$XDG_CONFIG_HOME/shader-editor/config.ron` (window size, recent projects, open tabs).

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
  only); accent `#2563eb` (the accent slot, rendered in `ui::ACCENT`) for the one modifier detail (the plus, the arrow), single marks stay ink-only. No `id`s, no
  gradients/`<defs>`. Then add `const X: Icon = custom("name", Kind::Line, include_str!("../../icons/name.svg"));`,
  the `IconId` variant, and the const to the `custom_icons_use_only_palette_colours` test list.
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

- Editor: `SHADER_EDITOR_SCREENSHOT=out.png [SHADER_EDITOR_OPEN=env/rock_wet.material.ron[#wesl|#rust]]
  [SHADER_EDITOR_SELECT=stdlib::Lerp] [SHADER_EDITOR_DEMO=palette|wire|dialog|menu|tab_menu] cargo run -- examples/sample_game`
  — renders, saves the PNG and exits. `SHADER_EDITOR_INPUT=drag|rmb_click|rmb_pan|mmb_pan|slider|color|pin_rmb|hover|curve_drag|curve_add`
  (+ `SHADER_EDITOR_INPUT_NODE=<id>`, `SHADER_EDITOR_INPUT_TRACE=1`, `SHADER_EDITOR_ZOOM=<f32>`) plays a scripted mouse
  gesture through winit's messages and logs the outcome (`demo` lines; `slider`/`color` also log the edited value).
- Sample game: `cd examples/sample_game && SAMPLE_SCREENSHOT=out cargo run` → `out-1.png`, `out-2.png` (used to verify
  hot reload). Render comparison (PSNR) is not automated.

## File format (`*.material.ron` / `*.function.ron`; curves: decision 0015)

Serialized `Asset { format_version, kind, name, nodes, edges, comments, meta }` with struct names, 2-space indent,
nodes in a `BTreeMap` by id. Asset name comes from the file name (file wins over the `name` field); names must be
`[a-z][a-z0-9_]*` and not Rust/WGSL keywords. Loading probes `format_version` first; bump it on incompatible changes and
add a migration in `Asset::from_ron` (current version: 1). See decision 0003.

## Rules

- Write all documentation and code comments **in English only** — including `docs/decisions/` entries. Refer to spec
  sections as `spec §N` in comments.
- Keep `README.md` up to date when a change affects anything it documents: CLI commands/flags, hotkeys, file formats,
  diagnostic codes, generated API, crate roles, limitations, dev commands.
- Record non-obvious implementation choices or deviations from the spec as a new `docs/decisions/NNNN-*.md` and add it
  to `docs/decisions/README.md` (never delete an old decision — mark it "superseded by NNNN").
- Never hand-edit autogen output (`examples/sample_game/assets/shaders/autogen/**`, `examples/sample_game/crates/shaders/**`,
  `crates/shader_project/tests/snapshots/*.snap`) — change the generator and regenerate.
