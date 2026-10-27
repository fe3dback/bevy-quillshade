<p align="center"><img src="assets/logo.png" alt="Bevy Shader Editor logo" width="256"></p>

# shader-editor — a node-based shader editor for Bevy 0.20

> [!WARNING]
> **Early prototype — use at your own risk.** This is an unofficial, community-driven editor, not supported by the
> Bevy developers. It will most likely be discontinued once the official Bevy editor ships with a built-in material
> editor.

A counterpart of the UE5 Material Editor: material and function graphs are stored as RON next to the game
(`shaders/**`), and the editor generates WESL from them (`assets/shaders/autogen/**`) plus the Rust crate
`crates/shaders` with material types, baked material handles and `ShadersPlugin`. The UI is Bevy UI +
Feathers, the canvas is a local fork of [bevy_noodle](https://github.com/gold-silver-copper/bevy_noodle)
(`vendor/bevy_noodle`, see its `FORK.md`). Codegen and
validation also work without the GUI (CLI for CI).

Specification — [`docs/techdoc.md`](docs/techdoc.md), manual UI checklist —
[`docs/ui-checklist.md`](docs/ui-checklist.md), implementation decisions —
[`docs/decisions/`](docs/decisions/README.md).

## Installation

Requires Rust ≥ 1.97.1 and Bevy's system dependencies (Linux: `libudev`, `alsa-lib`, `libxkbcommon`,
`wayland`, a Vulkan driver; see [Bevy setup](https://bevy.org/learn/quick-start/getting-started/setup/)).

```sh
cargo build --release
./target/release/shader-editor path/to/game        # GUI
```

In debug builds the workspace uses `bevy/dynamic_linking`, so it is easier to run via
`cargo run -- path/to/game` (cargo sets up the path to `libbevy_dylib` itself).

### CLI (headless)

| Command | What it does |
| --- | --- |
| `shader-editor regen <project> [--compile]` | regenerate everything; prints `create/modify/delete path` |
| `shader-editor check <project> [--compile]` | validation only, writes nothing |
| `shader-editor diff <project>` | what `regen` would change |

Diagnostics are printed to stderr as `path:node[.pin]:CODE: text`. Exit code: `0` — no
errors, `1` — validation errors, `2` — the project failed to open / write error. `--compile` additionally
compiles the generated WESL (wesl 0.6 + naga) against the Bevy 0.20 sources from the cargo registry.

## Hooking it up to a game

A project is any directory with a `Cargo.toml` and `assets/`. The editor owns three trees:

```text
shaders/                      # graph sources, under VCS
  .shader-editor.ron          # project settings
  .nodes/*.ron                # user nodes (optional)
  env/rock_wet.material.ron
  common/wet_mask.function.ron
  common/pulse.curve.ron
  common/tuning.consts.ron    # global constants (no generated file of their own)
  textures/rock.texture.ron   # Texture asset: a game image + loader settings (no generated file)
assets/shaders/autogen/       # AUTOGEN WESL: stdlib/, functions/, curves/, one file per material
  baked.ron                   # baked manifest: every Texture asset + every material's values
  dir_env/mat_rock_wet.wesl
  functions/dir_common/fun_wet_mask.wesl
  curves/dir_common/crv_pulse.wesl
assets/shaders/internal/      # AUTOGEN internal textures (default.png) + `.autogen` marker file
crates/shaders/               # AUTOGEN Rust crate (src/materials/dir_env/mat_rock_wet.rs, src/baked/, …)
```

Generated names are prefixed by kind (decision 0020): directories `dir_`, material files/modules `mat_`, functions
`fun_` (`fn fun_wet_mask`), curve modules `crv_`, material parameters and function inputs `inp_`, function outputs
`out_`. So any `snake_case` name works, keywords included (a `type` parameter, a `loop/` directory).

The editor never touches the root `Cargo.toml`. Add the crate yourself (otherwise you get W105 with a hint):

```toml
[workspace]
members = ["crates/shaders"]

[dependencies]
bevy = { version = "0.20", features = ["file_watcher"] }   # file_watcher — for hot-reload
shaders = { path = "crates/shaders" }
```

The generated `crates/shaders/Cargo.toml` pins `bevy = { version = "0.20", default-features = false, features = [..] }`
(the features include `png`, `ktx2` and `zstd_rust` for Texture assets) and depends on `serde` and `ron = "0.12"`.
If the root `Cargo.toml` declares Bevy in `[workspace.dependencies]` (or takes `bevy = { workspace = true }` itself), it
uses `bevy = { workspace = true, features = [..] }` instead, so the game's Bevy version and features apply. The same
steps are in the editor under Help → Quick Start.

```rust
use bevy::prelude::*;
use bevy::sprite_render::SpriteMaterial;
use shaders::{BakedMaterials, BakedMaterialsLoader, BakedMaterialsProgress, BakedTextures};

fn main() {
    App::new()
        .add_plugins((DefaultPlugins, shaders::ShadersPlugin))
        .add_systems(Startup, (load, spawn).chain())
        .add_systems(Update, loading_screen)
        .run();
}

// Optional: without it loading starts in the first PreUpdate, but then `BakedTextures` is empty in Startup.
fn load(mut loader: BakedMaterialsLoader) {
    let textures = loader.load(); // idempotent
    info!("loading {textures} textures");
}

// Material handles exist from the start (reserved); `load()` fills `BakedTextures` at once.
fn spawn(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    baked: Res<BakedMaterials>,
    tex: Res<BakedTextures>,
) {
    let mesh = meshes.add(Sphere::new(1.0).mesh().uv(48, 24).with_generated_tangents().unwrap());
    commands.spawn((Mesh3d(mesh), MeshMaterial3d(baked.env.rock_wet.clone())));
    // Sprite2d materials go into `SpriteMaterial(..)`.
    commands.spawn((Sprite::from_image(tex.textures.sprite.clone()), SpriteMaterial(baked.ui.dissolve.clone())));
}

fn loading_screen(mut progress: MessageReader<BakedMaterialsProgress>) {
    for p in progress.read() {
        info!("textures: {} loaded, {} failed of {}", p.loaded, p.failed, p.total);
    }
}
```

- **Pbr3d** — `ExtendedMaterial<StandardMaterial, {Name}Ext>`: shadows, prepass and deferred
  work as with `StandardMaterial`; `.material()` applies blend mode, two-sided and unlit from the graph.
- **Unlit3d** — its own `impl Material` (`{Name}Material`).
- **Sprite2d** — `MaterialExtension2d` for `SpriteMaterial`.
- Parameters are fields prefixed with `inp_`: the `wetness` parameter is `RockWetParams::inp_wetness`; textures are
  fields of the main type (`RockWetExt::inp_albedo`). A fixed texture (a `TextureSample` / `TextureObject` node with
  its Texture property, design §1.3) is a field `ft_<node id>` (`ft_<function path>_<node id>` for a node inside a
  called function) after the texture parameters (bindings `101/102`, `103/104`, … in that order).
  `{Name}Params::default()` holds the graph's default values. The constructor `new` and the setters `set_inp_*` are
  crate-internal (`pub(crate)`): games take materials from `BakedMaterials`, whose values come from the graph (design
  §3.1).
  Type names (`RockWetExt`, `RockWetMaterial`, `RockWetParams`) are re-exported from the crate root, so the module
  path (`shaders::materials::dir_env::mat_rock_wet`) is rarely needed.
- **Baked materials** (design §3). Regeneration writes `assets/shaders/autogen/baked.ron`: every valid Texture asset (id
  → image path + loader settings, used or not) plus the internal textures, and every material (id → parameter defaults
  by `inp_*` name, Texture asset id of every texture binding). The crate embeds it as `shaders::COMPILED_MANIFEST`. A
  Texture asset with an error (E011, E012, E013, E014) is left out and the bindings using it get `@internal/default`
  (its `BakedTextures` field stays, so the game still compiles, and holds the default texture at runtime); a material
  that fails to build keeps its previous entry, like its Rust module (if a foreign file blocks `baked.ron`, E008, the
  crate compiles that file instead). `shaders::BakedMaterials` (a resource) holds a handle per material, nested by
  folder: `baked.env.rock_wet`, `baked.kw.dir_loop.mat_type`; `shaders::BakedTextures` an image handle per texture:
  `tex.textures.rock_albedo`, `tex.internal.default`. Field names are the plain folder / asset names; a Rust keyword
  gets its prefix (`dir_`, `mat_`, `tex_`), and a folder and an asset with the same name in one folder are both prefixed
  (`env.dir_water.x`, `env.mat_water`). Both resources also map asset ids — the path under `shaders/` without the
  extension, as in the manifest — to handles, for game data that names materials or textures (decision 0030):
  `baked.get::<RockWetMaterial>("env/rock_wet")` (typed; `None` for an unknown id or another material type; the map
  itself is `baked.by_path: HashMap<String, UntypedHandle>`) and `tex.get("textures/rock_albedo")` /
  `tex.by_path: HashMap<String, Handle<Image>>` (filled by `load()`, follows hot reloads, and also holds a texture
  created in the editor after the build that a material uses). `by_path` is reserved at the root: a root folder
  `by_path/` becomes `dir_by_path`.
- **Loading** (design §3.3): `ShadersPlugin` registers the materials and creates `BakedMaterials` with reserved handles
  (usable in `Startup`) and `BakedTextures` with default handles. `shaders::BakedMaterialsLoader` (a `SystemParam`)
  `load()`s once (idempotent; returns the number of textures being loaded, `BakedMaterialsProgress` `total`; optional —
  without a call `ShadersPlugin` starts in the first `PreUpdate`, decision 0027): it starts loading every texture of the
  compiled manifest with its Texture asset settings (`ImageLoaderSettings`: sampler, `is_srgb`, `expand_grayscale`) and
  fills `BakedTextures` at once (a system chained after it sees the real handles); on the next `PreUpdate` every
  material is built from the compiled values and inserted behind its handle (Bevy shows it once its images are loaded),
  and `baked.ron` itself is loaded as an asset. `progress()` / `is_ready()` give the state; the messages
  `BakedMaterialsProgress { loaded, failed, total }` (written whenever a count changes; `total` = valid Texture assets +
  internal textures) and `BakedMaterialsReady` (once, when `loaded + failed == total`) report it. A texture that fails
  to load (file deleted after regeneration, corrupt image) is logged as an error naming its id and path, counted as
  `failed`, and every material using it is rebuilt with `@internal/default`; `BakedMaterialsReady` is still sent. Use
  `BakedTextures` instead of `AssetServer::load` for files that have a Texture asset: Bevy keeps the settings of the
  first load of a path, so a plain `load` may drop the asset's settings.
- **Hot reload** (design §3.2, §3.3 step 4): the compiled manifest is the base, `baked.ron` is an overlay applied when
  it loads and whenever it changes on disk — that needs Bevy's `file_watcher` feature, which the game enables itself
  (see the `Cargo.toml` above; the editor never adds it). Without a rebuild of the game these changes apply: parameter
  values and the texture of a binding (the material is rebuilt behind the same handle); a Texture asset pointed at
  another file (loaded with its settings and swapped in `BakedTextures` and in the materials — entities that took the
  old image, e.g. a spawned `Sprite`, keep it); a sampler change (set on the loaded image, the materials using it are
  refreshed); a texture created in the editor and used by a material (loaded by path, but not in `BakedTextures`);
  another TextureSample / TextureObject node wired in place of the one the game was built with (texture bindings are
  matched by number, decision 0029). These need a rebuild (`cargo run` again): `is_srgb` / `expand_grayscale` (Bevy
  keeps the first load's settings), adding / removing / renaming / retyping a parameter or a texture binding, adding or
  removing World Position Offset, adding / removing / renaming a material or changing its kind or blend settings, new
  `BakedTextures` fields. A material whose bindings changed stops rendering until the rebuild instead of crashing the
  game: each generated material passes its `LAYOUT_FEATURE` (`SG_LAYOUT_<hash>`) as a shader def, and the WESL does not
  compile without its own (Bevy logs "no definition in scope for identifier:
  `material_layout_changed__rebuild_the_game`", the game logs that the material changed; decision 0029). Anything out of
  sync with the compiled code — a material or key the code does not know, a key the manifest lacks or retypes — keeps
  the compiled value and logs one warning per key ("rebuild the game"); a `baked.ron` that does not parse or has another
  version logs a warning and changes nothing. Textures loaded by a hot reload are not counted in
  `BakedMaterialsProgress` after `BakedMaterialsReady`; one that fails falls back to `@internal/default` as above.

Full example — [`examples/sample_game`](examples/sample_game): one mesh per reference
material, using only `BakedMaterialsLoader` / `BakedMaterials` / `BakedTextures` (no `assets.load`, no material
constructors; parameter values are the graph defaults). Its images are Texture assets in both formats:
`rock_albedo` / `water_normal` are uncompressed `.ktx2`, the rest `.png`. While the game is running, saving a graph in
the editor (Ctrl+S) rewrites the `.wesl` and `baked.ron`, and Bevy reloads the shader and the values without a restart.

## Editor

Without an open project the window shows the welcome screen: recent projects (click to open), Open Project, New
Project (sets up an existing game crate — a directory with `Cargo.toml`: creates `assets/` and `shaders/` in it) and
how to hook the generated crate into the game. Open / New Project show the system folder picker (on Linux through
xdg-desktop-portal); cancel it to type or paste the path instead (Browse... reopens it).

The menu_bar holds the logo and the main menu — **File** (Open Project, New Project, Close Project, Save, Save All,
Quit), **Edit** (Undo, Redo), **View** (Zen mode, then Show project, Show console, Show properties: checked while
on; the project hidden leaves its width to the editor, the console hidden keeps only its header bar with the counts,
the properties hidden leave the width to the editor content; Zen mode hides the left and right sidebars and the whole
console, whatever their own marks say, and turned off shows exactly the checked ones; all four are remembered),
**Assets** (create a Pbr3d/Unlit3d/Sprite2d material, a function, a curve, a global constant table, a folder;
Regenerate) and **Help** (Quick Start: how to hook a game up, one tab per step; Project Page; About: version, link,
author, license) — then Undo / Redo and, on the right, the project name and its error count (the full path is the
tooltip).

Under the tabs bar, the editor topbar holds the buttons of the active tab's kind (graph: Auto-layout, Comment, Fit,
a material's Preview mode group, then on the right icon-only generated WESL and — for a material — Rust; curve: path,
Select all, Delete, preview mode, generated WESL; constant table: path; generated file: path, Copy, Source,
Open externally; game image: path, size, channel, tiling, zoom; Texture asset: Easy / Advanced, presets, channel,
tiling, zoom). Under the editor content, the editor statusbar of a graph, curve or constant table shows
`● unsaved`, the asset's error / warning count (click: show the first problem) and its size (nodes, wires, comments
and generated WESL lines, points, constants).

On the left is the asset browser — the **Project** panel (the `shaders/` tree: double click opens an asset; RMB on a
folder or on the empty space below the tree creates a Pbr3d/Unlit3d/Sprite2d material, a function, a curve, a global
constant table, a folder; RMB on an asset: rename, move (also by dragging onto a folder), delete, show the generated
WESL/Rust in a read-only, syntax-highlighted tab), in the center the tabs and the canvas, on the right the right
sidebar of the active tab's kind, titled after it (graph: **Properties** — node parameters, input default values, asset
blend mode / two-sided / shading / output, or a selected comment frame's title and hue; **Curve**: the selected point
and hotkeys; **Constants**: usage; **Generated File**: source and line count; **Image**: its Texture asset;
**Texture**: image info, problems, hotkeys), at the bottom the log (click — focus the node). The browser is an
IDE-style file tree: folders with expand carets, an icon per asset kind (function, Pbr3d, Unlit3d, Sprite2d, curve,
constant table, texture); the active tab's asset is highlighted, unsaved assets get an orange dot after the name.

The Project tree ends with the virtual read-only **Internal** folder of the editor's built-in textures
(`@internal/default`; no rename / move / delete). Below it the read-only **Game Assets** panel lists the game's images
under `assets/` (`.png`, `.ktx2`; `assets/shaders/` — the editor's own output — is hidden) and the folders holding them;
it follows files added or removed on disk. RMB on an image → **Create Texture Asset** creates
`shaders/textures/<folders>/<name>.texture.ron` pointing at it (the folders under `assets/`, a leading `textures/`
dropped; names made snake_case, `_2`, `_3`… on a clash); the item is disabled when a Texture asset already points at the
file. **Drag an image onto a graph** (design §4.3): on empty canvas (or onto a node without a texture property) it adds
a `TextureSample` at the drop point referencing the image's Texture asset; onto a `TextureSample`, `TextureObject` or
texture parameter node it sets that node's texture. When no Texture asset points at the image yet, one is created with
default settings (same name rule as Create Texture Asset); Ctrl+Z reverts only the graph edit, the created asset stays.
A Texture asset or `Internal` → `default` dragged from the Project tree works the same way. A **Function** asset
dragged onto a graph adds its call node at the drop point (a function dropped into its own graph is refused, as in the
palette; indirect recursion is reported by regeneration as E006). A **Curve** asset dropped onto a `Sample Curve` node
sets its curve, anywhere else it adds a `Sample Curve` reading it. Each drop is one undo step (decision 0033).

Double-click an image to open it in a read-only **image tab**: the image fitted to the view (transparent parts over a
checkerboard), `WxH | format | mips` in its editor topbar, a load / decode error as text (decoded by Bevy, like the game
would); it follows changes of the file and is restored on the next launch. Its preview is the Texture tab's (below):
wheel zoom, drag pan, double-click fit, R | G | B | All and "Tile 3x3" (repeated: a game image has no sampler of its
own); each tab keeps its own zoom.

Double-click a Texture asset to open its **Texture tab**: on the left a form with the image path (relative to `assets/`,
applied on Enter; refused when another Texture asset already points at the file — on disk or in an open, unsaved tab, so
a file a tab moved away from stays taken until that tab is saved — or when the path breaks the E012 rules; "Create
Texture Asset" follows the same rule), sRGB and Expand grayscale (both need a rebuild in the game; grayscale expansion
is not previewed — the editor always expands) and the sampler. **Easy** mode (the editor topbar's Easy | Advanced; the
default unless the sampler is one no preset describes — then Easy is disabled) has no form: "Use for" **Default** (no
sampler stored: the game's default, with Bevy's `ImagePlugin::default()` Linear and Clamp to edge), **Pixel art**
(Nearest, Clamp to edge), **UI** (bilinear, Clamp to edge), **3D** (trilinear, Repeat) or **Ground** (trilinear + 16x
anisotropy, Repeat) sets the whole sampler for that use (sRGB stays; one undo step); one of them is always selected,
read back from the sampler in the file. **Advanced** is the form: path, sRGB, Expand grayscale, "Default" (= the app's
default sampler: with Bevy's `ImagePlugin::default()` Linear filters and Clamp to edge, *not* Repeat; turning it off
starts from those values) and every sampler field. The image path, size, format and mip count are
in the right sidebar. The preview is decoded with the current, unsaved settings: the wheel zooms around the pointer
(Nearest vs Linear shows up close), a drag pans, a double click fits again, "Tile 3x3" repeats the image so the address
modes show, **R | G | B | All** shows one channel in grey (compressed formats: tinted) or the image (preview only).
Invalid sampler settings (E013, e.g. anisotropy above 1 without Linear filters, inverted or negative LOD clamps) show
the error instead of the image; Clamp to border on a GPU without the `ADDRESS_MODE_CLAMP_TO_BORDER` feature is previewed
as Clamp to edge, with a note. Undo / Redo, Ctrl+S (saves, regenerates the baked manifest) work as for graphs. Built-in
textures (double-click in **Internal**) open in a read-only Texture tab. In a node's Properties the Texture property is
a picker of the project's Texture assets and the built-in ones, with an **Open texture** button. `TextureSample`,
`TextureObject`, `TextureSampleParameter2D` and `TextureObjectParameter` show a 128×128 thumbnail of their texture left
of the output pins (aspect kept; the built-in default without one; "n/a" for a missing Texture asset or an image that
does not load); it follows unsaved edits of an open Texture tab, and a `TextureSample` wired straight to a TextureObject
or texture parameter shows that node's texture.

Tabs and menus show icons too; Undo / Redo are icon-only buttons — hover a menu_bar or editor topbar button for its
name and hotkey. Main and RMB menus are Feathers menus: while one is open it has the keyboard (Up / Down, Home / End,
Enter or Space activate, Esc closes); a click outside only closes it.
Tabs are compact and wrap onto more rows when there are many; a tab shows the file name only (`rock_wet`), the full path
when several open tabs share that name, and the full path as its tooltip. RMB on a tab: Close / Close All / Close
Others / Close to the Left / Close to the Right / Close Unmodified. The bottom-right corner of the main view shows the
open document's type (`pbr3d`, `unlit3d`, `sprite2d`, `function`, `curve`, `consts`, `image`, `texture`, or
`wesl`/`rust` for a generated file) as a faint watermark over the view.
Generated-file tabs refresh after every regeneration; Ctrl+C there copies the whole file, and the editor topbar
has "Open in the default application". Icons come from [icon-rs](https://crates.io/crates/icon-rs), plus a few
drawn by hand in `crates/shader_editor_app/icons/`.
Validation runs live: nodes with errors are outlined in red, with warnings — in yellow.

On the canvas (decisions 0033–0035) node positions snap to a 10-unit grid when touched: a dragged node jumps from
grid point to grid point (in a multi-node drag the node under the pointer snaps and the others keep their offsets to
it), and new, pasted and duplicated nodes land on the grid; positions loaded from a file stay as they are until moved.
A selected node has a 3 px white outline. Named Route declarations and their usages, Constant nodes with a label and
`Global*` nodes get a header colour from their name (route, label or constant), so a route's declaration and usages
match. A pin's dot is 16 px inside a 24 px hit box; the dot under the pointer grows and lightens (also the pin a
dragged wire snaps to). **Comment frames** (hold C: around the selection, or with nothing selected an empty 400x300
frame at the cursor; the Comment button puts that one at the view centre): dragging one moves the nodes that lie
inside it when the drag starts (nested frames too); a selected frame gets a Hue field (0..360) and a "Default colour"
button in the right sidebar; a box selection that fully encloses a frame and covers all of its nodes marks it, and
Delete then removes the frame with the nodes (one undo step; Cut keeps the frame, and so does a frame around the
material output, which is never deleted).

Values are also edited right on the nodes (UE-style): an unconnected input shows a number field, X/Y fields, or a
color button (Float3/Float4/Color) that opens a color picker (values are linear; exact components are edited in
Properties); a wire hides the editor. Dragging a number field changes its value (one undo step per drag); inputs with
a known range (e.g. Lerp Alpha) show it as a bar and drag only within it, but a click into the field types any value,
also one outside the range. Constants and parameters edit their value under the title (ScalarParameter's range is its
Slider Min..Max). RMB on an input pin: the type of polymorphic
inputs (Float…Float4, e.g. Lerp A/B), Reset to default, Disconnect. RMB on a node: its own actions with their hotkeys
(Add / Subtract / Multiply / Divide: Convert to one of the other three, keeping wires and values; `TextureSample` /
`TextureObject`: **Convert to Parameter** (not in functions) → `TextureSampleParameter2D` / `TextureObjectParameter`
with the texture as its default, the mip mode and sampler type kept and a unique name `texture`, `texture_1`, …; back
with **Convert to Texture Sample** / **Convert to Texture Object**, which drops name and group), then Exclusive preview
(materials only), Copy / Duplicate / Delete; an unselected node is selected first. A node action's hotkey works on the
selected nodes without the menu (decision 0021).

**Exclusive node preview** (decision 0022, like UE's "Start Previewing Node"): `V` on the one selected node (on release;
`V` + LMB still drops a Vector parameter) or "Exclusive preview" in its menu shows that node's output on the material
instead of the material output — the running game picks it up through hot reload. The node gets a 6 px outline pulsing
between orange and dark red and a warning icon, the tab title says `(preview)`; `V` again or "Stop preview" turns it
off. Toggling (and the editor topbar's Preview mode group `Unlit` / `Lit` / `Full`, disabled until a node is
previewed but still showing the stored mode) is an undoable edit that marks the tab unsaved **and** regenerates that material's files from the
in-memory graph at once, without saving — including the asset's other unsaved edits; closing the tab without saving
regenerates them from disk again. Only nodes with an output
that converts to a color can be previewed; the preview is stored in the `.ron` on save, so `regen` / `check` produce
what the editor showed (W107).

**Curves** (`+Curve`) are 1D functions on the unit square: points `(t, value)`, both in `0..1`. A curve tab shows the
plot with draggable points; the `Catmull-Rom` / `Simple` switch in its editor topbar only changes the preview — the file
stores just the points. The `Sample Curve` node (Utility) picks a curve of the project in Properties, has a `Type`
(`Catmull-Rom` — smooth, may overshoot between points; `Simple` — linear) and returns the value at `T`; outside the
first/last point the curve holds the end values. The node shows a small read-only plot of its curve (with a line at `T`
while `T` is not connected; "n/a" when no curve is selected). Renaming a curve updates the nodes that use it.

**Global constants** (`+Global Consts`, decision 0017) are tables of named values shared by every shader: the tab
has an add form (name, type, Add or Enter) and the table (type select, value, delete). Types: `f32` (a number field
taking any value, dragged over `0..1`, a range that widens to a typed value outside it), `vec2`/`vec3`/`vec4`
(color picker + R/G/B/A fields, any value can be typed), `bool` (read as `1.0` / `0.0`),
`compileFlag` (a real `bool`, connects only to logic nodes: If (Bool), And, Or, Not). Names are unique in the whole
project. The nodes `GlobalConstF32`, `GlobalConstVec2/3/4`, `GlobalBool`, `GlobalCompileFlag` (Constants) pick a
constant of their type from any table in Properties and show its value read-only (number, color square, green `1` /
grey `0` square). Values are copied into the generated WESL as literals — there is no uniform: saving a table
regenerates the shaders that read it. New constants start at 1.0 (`true`). Changing a constant's type or deleting it
(also by undo / redo, or by deleting the table file), when nodes read it, asks first, then saves the table and clears
those references (the nodes output 1.0 / `true`, W106).

**Material Attributes (PBR)** (decision 0023, like UE's Material Attributes) pack a whole PBR layer into one pin of
type `MaterialAttributes (PBR)` — fields Base Color, Metallic, Specular, Roughness, Emissive Color, Opacity, Opacity
Mask, Normal (tangent space), Ambient Occlusion; World Position Offset is not part of it. `MakeMaterialAttributes
(PBR)` builds one (unconnected fields use fixed defaults — white, metallic 0, specular / roughness 0.5, normal
`(0, 0, 1)`, opacity / AO 1 — not the game's `StandardMaterial` values), `BreakMaterialAttributes (PBR)` splits one
(category Material Attributes). The ordinary `Lerp` blends two of them field by field with a scalar Alpha; mixing
attributes with a vector on the other side, or wiring them into Add / Multiply / If / …, is refused. Functions take
and return them (FunctionInput / FunctionOutput type `MaterialAttributes (PBR)`; an unconnected one means all
defaults), and reroutes / named routes pass them through. The struct lives in the stdlib module
`autogen/stdlib/material.wesl`. Pbr3d materials get an **Output** setting in Properties: `Default` (one pin per
attribute) or `Wrapped` — the output node then has a single `Material Attributes` input plus World Position Offset.
Switching drops the wires (and inline values) of pins that disappear, in one undo step. In `Wrapped` mode a connected
`attributes` writes every field active for the Shading / Blend settings, overriding `StandardMaterial`'s base color,
metallic and roughness; left unconnected it writes nothing. The Normal is always written there (no W103). Nodes that
output attributes (Make, a Lerp of them, a function returning them) can be previewed: `unlit` / `lit` show their Base
Color, `full` every field.

**WorldAlignedBlend** (Utility) blends by slope, like UE's node: `Mask` is 1 on surfaces facing `Up` (default
`(0, 1, 0)`) and 0 on steep ones — with the defaults (Blend Sharpness 10, Blend Bias -0.7) up to about 45° is 1 and
beyond about 53° is 0 — and `Angle` is the slope in degrees (0 = flat, 90 = wall). Normal defaults to the vertex
normal; wire a perturbed normal in for "pixel normals". `examples/sample_game` has a two-layer terrain
(`env/terrain_layers`, rock from the `common/rock_layer` function on the slopes, grass on top).

**Textures**: a Texture asset is picked in the Texture property of `TextureSample` and `TextureObject` (Texture), or
as the Default Texture of `TextureSampleParameter2D` / `TextureObjectParameter` (Parameters). `TextureSample`'s `Tex`
input is optional and overrides its property when connected (like UE); `TextureObject` outputs its texture as a
`Texture2D` for `TextureSample`, `TextureSize`, `FlipBook`, functions… Every such node with a fixed texture gets a
material binding of its own, named after the node, so picking another texture changes neither WESL nor Rust. Without
any texture a node samples the internal default texture `@internal/default` (a grey / white checkerboard, W110);
nodes that take a texture only through their `Tex` pin (`TextureSize`, `TexelSize`, `FlipBook`,
`NormalFromHeightmap`) still need it connected (E004).
`TextureSampleParameter2D` has the same MipValueMode (None / Bias / Level + `Mip` input) as `TextureSample`. Inside a
function a fixed texture becomes a hidden trailing texture + sampler argument `ft_<function path>_<node id>` (`/` →
`_`, after the builtin arguments, like UV or time); callers pass it on — a calling function as a hidden argument of its
own, a material as one binding per node of origin, however often the function is called. Two such names that collide
(`a/b_c` and `a_b/c` with the same node id) are E006.

**Double click on a node** that refers to another file — a function call, Sample Curve, a Global* node — opens that
function / curve / constant table (or switches to its tab); Properties shows the same as an "Open …" button. User
nodes are not editor assets and do nothing.

**Double click on a node header** renames a node with a name or label — a parameter, a function's input / output, a
Named Route declaration (its name; its usages follow), a Constant / Constant2–4Vector (its label; empty clears it) — in
a small dialog (Enter applies, Esc cancels; one undo step, a bad name shows up as the usual diagnostic). Other nodes'
headers act as their body: open the file they refer to, or nothing. **Double click on a comment frame's title** edits
the title the same way.

| Action | Keys |
| --- | --- |
| Save (and regenerate) / all | Ctrl+S / Ctrl+Shift+S |
| Undo / redo | Ctrl+Z / Ctrl+Y, Ctrl+Shift+Z |
| Copy / cut / paste / duplicate | Ctrl+C / Ctrl+X / Ctrl+V / Ctrl+D (in a generated-file tab Ctrl+C copies the file) |
| Select all (nodes / curve points) | Ctrl+A |
| Delete selection | Delete, Backspace |
| Focus selection / whole graph | F / Home |
| Node palette (search by name, category, synonyms: `*`, `+`, `lerp`) | Space, RMB click on the canvas |
| Palette filtered by wire type + auto-connect | release a wire on empty space |
| Node under the cursor: Constant/2/3/4Vector, Scalar/Vector parameter, TextureSample, Multiply, Add, Lerp, TexCoord, Reroute, Divide, OneMinus, Power, Normalize, Panner, If | `1` `2` `3` `4` `S` `V` `T` `M` `A` `L` `U` `R` `D` `O` `E` `N` `P` `I` + LMB |
| Rename a node (name / label) / a comment frame | double click on its header / title |
| Comment frame around the selection (nothing selected: an empty 400x300 frame at the cursor) | hold C (0.5 s; a tap does nothing) |
| Delete a comment frame together with its nodes | box-select around the whole frame, Delete |
| Reroute point on a wire / delete a wire | double click / RMB click or Alt+LMB on the wire |
| Pin menu: polymorphic type / reset value / disconnect | RMB click on an input pin |
| Node menu: node actions / copy / duplicate / delete | RMB click on a node |
| Convert the selected Add / Subtract / Multiply / Divide nodes to Add / Subtract / Multiply / Divide | Shift + `+` / `-` / `*` / `/` (main row or numpad, Shift optional on the numpad) |
| Exclusive preview of the selected node on / off (materials) | `V` (on release) |
| Disconnect every wire of a pin / a wire | Alt+LMB on the pin / on the wire |
| New wire from a pin (from a connected input too: connecting replaces its wire) | LMB drag from the pin onto another pin (the wire snaps to the nearest pin it can connect to within 40 px of the cursor) |
| Move a wire to another input / all wires of an output to another output (all or nothing, one undo step) | Ctrl+LMB drag from the pin onto the other pin (the wires follow the cursor; they snap only to pins they can move to, the nearest within 40 px) |
| Cancel a wire drag (a new wire or wires being moved) | Esc |
| Named route: a wire without a visible wire (Named Route + one usage per place) | palette: "Named Route", then "Named Route Usage: {name}" |
| Name a wire: a Named Route fed by it (listed first unless the query matches another node better; its rename dialog opens, as for every Named Route added from the palette) | release a wire from an output on empty space, pick "Named Route" |
| Auto-layout the selection (or the whole graph) | Ctrl+L |
| Close tab / next tab | Ctrl+W (or MMB on the tab) / Ctrl+Tab |
| Close several tabs (all, others, left/right, unmodified) | RMB on a tab |
| Open an asset | double click in the browser |
| Open a game image / a built-in texture | double click in Game Assets / in Project → Internal |
| Add a `TextureSample` for an image, or set a texture node's texture | drag an image (Game Assets), a Texture asset or `Internal` → `default` (Project) onto the canvas / onto the node |
| Add a function call / a `Sample Curve`, or set a `Sample Curve`'s curve | drag a Function / Curve asset (Project) onto the canvas / onto the node |
| Image / Texture tab preview: zoom / pan / fit | wheel (around the pointer) / drag / double click |
| Open the function / curve / constant table a node refers to | double click on the node |
| Scroll a generated file horizontally | Shift+wheel |
| Curve: add a point / move / multi-select | double click on empty space / drag a point / Shift+click |
| Curve drag: lock to the T / value axis, snap to the 0.1 grid | hold X / Y, hold Shift |
| Curve: nudge the selected points | arrow keys (Shift: ×10) |
| Curve: point menu / plot menu (add point, select all, preview mode) | RMB on a point / on empty space |
| Open project / quit | Ctrl+O / Ctrl+Q |
| Show / hide the project (left sidebar) / the console messages / the properties (right sidebar) | Ctrl+1 / Ctrl+2 (or the chevron in the console header) / Ctrl+3 |
| Zen mode: hide both sidebars and the whole console (Ctrl+1/2/3 still set what comes back) | `` ` `` |
| Move the keyboard focus over buttons and fields / press the focused button | Tab, Shift+Tab / Enter, Space |

Hotkeys belong to the active tab's editor kind (graph, curve, generated file) or are global (save, undo, tabs, open,
quit, the View toggles); none fires while a text field, the palette, a dialog, a menu or a select's list has the
keyboard, and Ctrl, Alt or Super held blocks the single-key ones (decisions 0032, 0034). Keyboard focus is Bevy's
default (decision 0035): a click focuses the button or field under it, a click on the canvas clears it; while a button
has the focus, Space and Enter press it instead of acting as hotkeys (so after a toolbar click or a menu pick, click
the canvas before Space opens the palette); Ctrl+Tab clears the focus. Pan — drag with the right
(or middle) mouse button (an RMB press that does not move is a click), zoom — wheel, box select — LMB on empty space;
the minimap in the bottom left is clickable. Unsaved
tabs are marked with `*` (browser rows with an orange dot); on close and on an external change of the file (git
checkout) the editor asks what to do. Window size, recent projects, open tabs and the View toggles are stored in
`<config dir>/fe3dback/bevy-shader-editor/config.ron`, where the config dir is `%APPDATA%` on Windows,
`~/Library/Application Support` on macOS and `$XDG_CONFIG_HOME` (else `~/.config`) elsewhere (decision 0018).

## File formats

Material (`*.material.ron`, abridged):

```ron
Asset(
  format_version: 1,
  kind: Material(Pbr3d(PbrSettings(blend: Mask, two_sided: false, shading: Lit))),
  name: "rock_wet",
  nodes: {
    "n01": Node(kind: "output::Pbr3d", pos: (1260.0, 0.0)),
    "n05": Node(kind: "param::Scalar", pos: (0.0, 810.0), params: {"name": Str("wetness"), "default": Float(0.3)}),
    "n09": Node(kind: "stdlib::Lerp", pos: (952.0, 216.0), inputs: {"a": Float(0.9), "b": Float(0.15)}),
  },
  edges: [Edge(from: ("n09", "out"), to: ("n01", "roughness"))],
  comments: [Frame(id: "c01", title: "Emissive pulse", rect: (322.0, 756.0, 1008.0, 351.0), nodes: [])],
  meta: Meta(description: "", tags: []),
)
```

- Node `kind`: `stdlib::*`, `param::*`, `output::*`, `function::<path>` (function call),
  `user::<id>` (user node).
- Constant nodes (`stdlib::Constant`, `Constant2Vector`/`3Vector`/`4Vector`) may have an optional `label: "…"` on the
  node (omitted when empty). It is editor-only: it replaces the value in the node title and never affects generated
  code.
- A comment frame `Frame(id, title, rect: (x, y, w, h), nodes)` may end with an optional `color: Some((r, g, b, a))`
  and an optional `hue: Some(200.0)` (degrees; set from the right sidebar, drawn at the editor's fixed saturation /
  lightness, takes precedence over `color`). Both are omitted when unset; frames are editor-only. `nodes` lists the
  member nodes at creation time (used when deleting a frame with its nodes).
- Pbr3d settings may end with `output: Wrapped` (`PbrSettings(blend: Opaque, two_sided: false, shading: Lit, output:
  Wrapped)`): the output node takes one `MaterialAttributes (PBR)` input plus WPO. Omitted for the default `Default`.
- A material may have an optional `preview: Preview(node: Some("n09"), mode: Unlit)` (omitted when no node is
  previewed and the mode is the default `Unlit`; modes `Unlit`, `Lit`, `Full`). While `node` is set, the outputs of
  that node replace the material output's in the generated WESL (W107); deleting the node clears it.
- A function (`*.function.ron`, `kind: Function`) declares inputs with `output::FunctionInput` nodes and
  outputs with `output::FunctionOutput` nodes. Changing a function regenerates the dependent materials.
  FunctionInput params: `name`, `type`, `default`, `required`, `order` (Sort Priority) and, for a `Float` input only,
  `min` / `max` (Slider Min / Max): when `min < max`, that input of every call node gets an inline number field
  dragged within this range (editor only, no codegen effect; omitted from the file unless set).
- A curve (`*.curve.ron`): `Curve(format_version: 1, name: "pulse", points: [(0.0, 0.0), (0.25, 1.0), (1.0, 0.0)])`
  — points sorted by `t`, clamped to `0..1`. It is generated into `autogen/curves/dir_<dir>/crv_<name>.wesl` (a
  const array of `(t, value, slope)` and `sample_curve_simple` / `sample_curve_catmullrom`); shaders import the one
  they sample.
- A global constant table (`*.consts.ron`): `GlobalConsts(format_version: 1, name: "tuning", consts:
  [GlobalConst(name: "glow_amount", ty: F32, value: Float(0.35)), ...])`, `ty` one of `F32`, `Vec2`, `Vec3`,
  `Vec4`, `Bool`, `CompileFlag` (values `Float`, `Float2`…`Float4`, `Bool`). Nodes store only the constant's name.
- A Texture asset (`*.texture.ron`): a proxy to an image of the game, `path` relative to `assets/` (`/`-separated,
  `.ktx2` or `.png` only), plus loader settings mirroring Bevy's `ImageLoaderSettings`. Fields at Bevy's default are
  omitted (`is_srgb: true`, `expand_grayscale: true`, `sampler: None` = the app's default sampler; inside
  `SamplerSettings`: address modes `ClampToEdge` (`Repeat`, `MirrorRepeat`, `ClampToBorder`), filters `Nearest`
  (`Linear`), `lod_min_clamp: 0.0`, `lod_max_clamp: 32.0`, `anisotropy_clamp: 1`). A game file may have only one
  Texture asset (E011); renaming or deleting the asset never touches the image. Nodes store the Texture asset's id
  (`textures/rock`, or an internal texture such as `@internal/default`); renaming the asset rewrites these
  references, deleting it clears them (the nodes then use the default texture, W110). A file from before Texture
  assets that stores a texture default as an image path (`default: Str("textures/rock_albedo.png")`) does not load:
  create a Texture asset for the image and put its id there.

  ```ron
  TextureAsset(
    format_version: 1,
    path: "textures/env/rock_albedo.ktx2",
    settings: TextureSettings(
      is_srgb: false,
      sampler: Some(SamplerSettings(
        address_mode_u: Repeat,
        address_mode_v: Repeat,
        mag_filter: Linear,
        min_filter: Linear,
      )),
    ),
  )
  ```
- User node (`shaders/.nodes/*.ron`):

  ```ron
  UserNode(
    id: "soft_clamp",
    name: "SoftClamp",
    category: "User",
    inputs: [UserPin(name: "x", ty: Float3)],
    output: Float3,
    template: "return x / (vec3<f32>(1.0) + abs(x));",
  )
  ```
- Asset, directory, parameter, function port and global constant names are `snake_case` (`[a-z][a-z0-9_]*`, no
  `__`, no trailing `_`); keywords are allowed because generated code prefixes them. Input names of Custom nodes
  and user nodes are the exception: they appear verbatim in your WGSL, so they must not be Rust/WGSL keywords or
  names of generated code and must not start with `n_`, `b_`, `sg_` or `_`.
- Autogen files start with the marker `AUTOGENERATED by shader-editor v1 … do not edit`; the editor does not
  overwrite files without the marker (E008), binary ones included, nor files it cannot read (E008). Binary files
  cannot carry the marker, so `assets/shaders/internal/` is owned through its marker file `.autogen`; a non-empty
  `internal/` without it, or a regular file at that path, is left alone (E008). Writes are atomic and happen only when
  the content changes; orphaned files are deleted.

## Diagnostics

| Code | Condition |
| --- | --- |
| E001 | missing or multiple output nodes |
| E002 | cycle |
| E003 | incompatible types (no allowed auto-conversion; a Global* node reading a constant of another type) |
| E004 | required input not connected (or a Named Route Usage without its declaration, a Sample Curve without a curve) |
| E005 | invalid or duplicate name (also a global constant defined in two tables) |
| E006 | nonexistent function / recursive functions / missing or broken curve / missing global constant / missing Texture asset / clashing hidden texture names of two functions |
| E007 | WESL of a Custom node does not parse |
| E008 | an autogen file without the marker (or one that cannot be read) blocks the write (or `assets/shaders/internal/` without its `.autogen` marker file, or not a directory) |
| E009 | unknown node / broken file |
| E010 | generated WESL failed wesl+naga (`--compile`, saving in the editor) |
| E011 | two or more Texture assets point at the same game file (reported at each of them) |
| E012 | invalid Texture asset path: empty, absolute, with `.` / `..` / empty components, `\` or `#`, or not `.ktx2` / `.png` |
| E013 | invalid Texture asset sampler settings: `anisotropy_clamp` 0, `anisotropy_clamp` above 1 while a mag / min / mipmap filter is not `Linear`, LOD clamps non-finite, negative or `lod_min_clamp > lod_max_clamp` (the asset is not loaded; materials use the default texture) |
| E014 | the image of a Texture asset is a KTX2 cube map, texture array or 3D texture: materials sample 2D textures only (the asset is not loaded; materials use the default texture; the editor previews the faces / layers stacked top to bottom) |
| W101–W105 | dead branch, inactive output pin, mesh attribute may be missing, vector truncation, `crates/shaders` not in `[workspace.members]` |
| W106 | a Global* node without a constant (outputs 1.0 / `true`) |
| W107 | node preview active (the material output is bypassed), or the stored preview ignored (missing node, no output that converts to a color) |
| W108 | node preview changes the generated Rust crate (extra parameters or builtins): rebuild the game, and again after turning preview off |
| W109 | the image of a Texture asset does not exist under `assets/` (it may be added later) |
| W110 | a texture node without a texture (no Texture property, no connected Tex, no parameter default): uses the internal default texture `@internal/default` |

An error blocks generation only of its own asset; the rest are regenerated.

## Development

```sh
cargo test --workspace                           # unit, pipeline, snapshot + WESL compilation, UI smoke
cargo clippy --workspace --all-targets -- -D warnings
INSTA_UPDATE=always cargo test -p shader_project --test snapshots   # accept codegen changes
cargo run -p shader_project --example make_sample -- examples/sample_game   # rebuild the example
cargo run -p shader_project --example png_to_ktx2 -- [--linear] in.png [out.ktx2]   # uncompressed RGBA8 KTX2
cd examples/sample_game && cargo clippy --workspace -- -D warnings  # the generated crate
```

The snapshot suite (`crates/shader_project/tests/snapshots/`) has one material per stdlib node
plus the reference graphs; every `.wesl` is compiled against the real Bevy 0.20 sources in the
forward, forward without UV/tangents and deferred (`PREPASS_PIPELINE`) variants. `SAMPLE_SCREENSHOT=out cargo run`
in `examples/sample_game` renders the scene, saves `out-1.png` 5 s and `out-2.png` 25 s (real time) after the first
frame and exits — edit and regenerate in between to check hot reload. `png_to_ktx2` (no external tools) wrote the
sample's `.ktx2` files: one mip, no supercompression, `R8G8B8A8_SRGB` or `_UNORM` with `--linear`; a test checks they
match their `.png`.

| Crate | Role |
| --- | --- |
| `shader_graph_core` | model, types and conversions, node registry, validation, IR (no Bevy) |
| `shader_stdlib` | standard library nodes, stdlib WESL modules, user nodes, internal textures |
| `shader_codegen_wesl` | WESL generation, compile check (wesl + naga) |
| `shader_codegen_rust` | generation of `crates/shaders` and of the baked manifest `baked.ron` (decision 0025) |
| `shader_project` | project layout, regeneration pipeline, atomic writes, asset operations, watcher, game image scan |
| `shader_editor_app` | GUI on Bevy + Feathers + bevy_noodle |
| `bevy_noodle` (`vendor/bevy_noodle`) | local fork of bevy_noodle 0.5.0, the node-graph canvas; changes from upstream in its `FORK.md` |

## Limitations

WPO is not visible to the prepass/shadows; function outputs of type Texture2D are not supported; user nodes have a
single output; Static Switch is out of v1; number fields on nodes drag at the wrong speed and the color picker opens off
its button when the canvas is zoomed (Bevy 0.20 bugs, see decision 0012); Custom-node and user-node input names cannot
be keywords (they are used unprefixed in the WGSL you write, decision 0020); `MaterialAttributes (PBR)` cannot be used
by Custom nodes, user nodes or parameters, and only Pbr3d has the `Wrapped` output (decision 0023). Textures: only
`.png` and `.ktx2` (Bevy 0.20 cannot read `.ktx` v1; use lowercase extensions — E012 accepts `.PNG`, but Bevy picks the
loader by a case-sensitive extension match); `is_srgb` / `expand_grayscale` changes need a rebuild of the game (Bevy
keeps the first load's settings), and the editor cannot preview `expand_grayscale: false`; a function that samples a
texture (MipValueMode None / Bias) cannot be used in World Position Offset — its `textureSample` would end up in the
vertex shader, which WGSL forbids, and live validation does not report it (a material's own texture nodes switch to
`textureSampleLevel` there); a texture that failed to load in the game is retried only when its path changes (decisions
0024, 0025). Details — [`docs/decisions/0010-v1-limitations.md`](docs/decisions/0010-v1-limitations.md).
