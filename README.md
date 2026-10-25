<p align="center"><img src="assets/logo.png" alt="Bevy Shader Editor logo" width="256"></p>

# shader-editor — a node-based shader editor for Bevy 0.20

> [!WARNING]
> **Early prototype — use at your own risk.** This is an unofficial, community-driven editor, not supported by the
> Bevy developers. It will most likely be discontinued once the official Bevy editor ships with a built-in material
> editor.

A counterpart of the UE5 Material Editor: material and function graphs are stored as RON next to the game
(`shaders/**`), and the editor generates WESL from them (`assets/shaders/autogen/**`) plus the Rust crate
`crates/shaders` with material types, constructors and `ShadersPlugin`. The UI is Bevy UI +
Feathers, the canvas is [bevy_noodle](https://github.com/gold-silver-copper/bevy_noodle). Codegen and
validation also work without the GUI (CLI for CI).

Specification — [`docs/techdoc.md`](docs/techdoc.md), manual UI checklist — [`docs/ui-checklist.md`](docs/ui-checklist.md), implementation decisions —
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
assets/shaders/autogen/       # AUTOGEN WESL: stdlib/, functions/, curves/, one file per material
crates/shaders/               # AUTOGEN Rust crate
```

The editor never touches the root `Cargo.toml`. Add the crate yourself (otherwise you get W105 with a hint):

```toml
[workspace]
members = ["crates/shaders"]

[dependencies]
bevy = { version = "0.20", features = ["file_watcher"] }   # file_watcher — for hot-reload
shaders = { path = "crates/shaders" }
```

The generated `crates/shaders/Cargo.toml` pins `bevy = { version = "0.20", default-features = false, features = [..] }`.
If the root `Cargo.toml` declares Bevy in `[workspace.dependencies]` (or takes `bevy = { workspace = true }` itself), it
uses `bevy = { workspace = true, features = [..] }` instead, so the game's Bevy version and features apply. The same
steps are in the editor under Help → Quick Start.

```rust
use bevy::prelude::*;
use bevy::sprite_render::SpriteMaterial;
use shaders::{DissolveMaterial, RockWetExt, RockWetMaterial};

fn main() {
    App::new().add_plugins((DefaultPlugins, shaders::ShadersPlugin)).add_systems(Startup, setup).run();
}

fn setup(
    mut commands: Commands,
    assets: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut rock: ResMut<Assets<RockWetMaterial>>,
    mut dissolve: ResMut<Assets<DissolveMaterial>>,
) {
    // Pbr3d: `{Name}Ext` + `{Name}Material = ExtendedMaterial<StandardMaterial, {Name}Ext>`.
    // `new` takes every graph parameter in the order set in the editor.
    let mat = RockWetExt::new(assets.load("textures/rock_albedo.png"), assets.load("textures/rock_normal.png"), LinearRgba::WHITE, 0.4, 1.5);
    let mesh = meshes.add(Sphere::new(1.0).mesh().uv(48, 24).with_generated_tangents().unwrap());
    commands.spawn((Mesh3d(mesh), MeshMaterial3d(rock.add(mat.material()))));

    // Sprite2d: an extension of `SpriteMaterial`, the component goes next to `Sprite`.
    commands.spawn((
        Sprite::from_image(assets.load("textures/sprite.png")),
        SpriteMaterial(dissolve.add(DissolveMaterial::new(0.35, LinearRgba::rgb(1.0, 0.6, 0.1), 0.06))),
    ));
}
```

- **Pbr3d** — `ExtendedMaterial<StandardMaterial, {Name}Ext>`: shadows, prepass and deferred
  work as with `StandardMaterial`; `.material()` applies blend mode, two-sided and unlit from the graph.
- **Unlit3d** — its own `impl Material` (`{Name}Material::new(..)`).
- **Sprite2d** — `MaterialExtension2d` for `SpriteMaterial`.
- Parameters are changed with `set_*` setters; `{Name}Params::default()` holds the graph's default values.

Full example — [`examples/sample_game`](examples/sample_game): one mesh per reference
material. While the game is running, saving a graph in the editor (Ctrl+S) rewrites the `.wesl`, and
Bevy reloads the shader without a restart.

## Editor

Without an open project the window shows the welcome screen: recent projects (click to open), Open Project, New
Project (sets up an existing game crate — a directory with `Cargo.toml`: creates `assets/` and `shaders/` in it) and
how to hook the generated crate into the game. Open / New Project show the system folder picker (on Linux through
xdg-desktop-portal); cancel it to type or paste the path instead (Browse... reopens it).

The toolbar holds the logo and the main menu — **File** (Open Project, New Project, Close Project, Save, Save All,
Quit), **Assets** (create a Pbr3d/Unlit3d/Sprite2d material, a function, a curve, a global constant table, a folder;
Regenerate) and **Help** (Quick Start: how to hook a game up; Project Page; About: version, link, author, license) — then Undo /
Redo and the actions of the active tab (graph: Auto-layout, Comment, Fit; curve: Select all, Delete; constant table:
Add; generated file: Copy, Source, Open externally).

On the left is the asset browser (the `shaders/` tree: double click opens an asset; RMB on a folder or on the empty
space below the tree creates a Pbr3d/Unlit3d/Sprite2d material, a function, a curve, a global constant table, a
folder; RMB on an asset:
rename, move (also by dragging onto a folder), delete, show the generated WESL/Rust in a read-only,
syntax-highlighted tab), in the center the tabs and the canvas, on the right the properties (node parameters, input
default values, asset blend mode / two-sided / shading), at the bottom the log (click — focus the node).
The browser is an IDE-style file tree: folders with expand carets, an icon per asset kind (function, Pbr3d,
Unlit3d, Sprite2d, curve, constant table); the active tab's asset is highlighted, unsaved assets get an orange dot after the name.
Tabs and menus show icons too; Undo / Redo are icon-only buttons — hover a toolbar button for its name and hotkey.
Tabs are compact and wrap onto more rows when there are many; a tab shows the file name only (`rock_wet`), the full
path when several open tabs share that name, and the full path as its tooltip. RMB on a tab: Close / Close All / Close Others / Close to the Left / Close to the Right / Close Unmodified. The bottom-right corner of the main view shows the open document's type (`pbr3d`, `unlit3d`, `sprite2d`,
`function`, `curve`, `consts`, or `wesl`/`rust` for a generated file) as a faint watermark over the view.
Generated-file tabs refresh after every regeneration; Ctrl+C there copies the whole file, and the header has
"Open in the default application". Icons come from [icon-rs](https://crates.io/crates/icon-rs), plus a few
drawn by hand in `crates/shader_editor_app/icons/`.
Validation runs live: nodes with errors are outlined in red, with warnings — in yellow.

Values are also edited right on the nodes (UE-style): an unconnected input shows a slider (inputs with a known
range, e.g. Lerp Alpha), a number field, X/Y fields, or a color button (Float3/Float4/Color) that opens a color picker (values are linear;
exact components are edited in Properties); a wire hides the editor. Constants and parameters edit their value under the
title (ScalarParameter's slider spans Slider Min..Max). RMB on an input pin: the type of polymorphic inputs
(Float…Float4, e.g. Lerp A/B), Reset to default, Disconnect.

**Curves** (`+Curve`) are 1D functions on the unit square: points `(t, value)`, both in `0..1`. A curve tab shows
the plot with draggable points; the `Catmull-Rom` / `Simple` switch in its header only changes the preview — the file stores just the points. The `Sample Curve` node (Utility) picks a curve of the project in
Properties, has a `Type` (`Catmull-Rom` — smooth, may overshoot between points; `Simple` — linear) and returns the
value at `T`; outside the first/last point the curve holds the end values. The node shows a small read-only plot of
its curve (with a line at `T` while `T` is not connected; "n/a" when no curve is selected). Renaming a curve updates the nodes that
use it.

**Global constants** (`+Global Consts`, decision 0017) are tables of named values shared by every shader: the tab
has an add form (name, type, Add or Enter) and the table (type select, value, delete). Types: `f32` (slider `0..1`),
`vec2`/`vec3`/`vec4` (color picker + R/G/B/A fields, any value can be typed), `bool` (read as `1.0` / `0.0`),
`compileFlag` (a real `bool`, connects only to logic nodes: If (Bool), And, Or, Not). Names are unique in the whole
project. The nodes `GlobalConstF32`, `GlobalConstVec2/3/4`, `GlobalBool`, `GlobalCompileFlag` (Constants) pick a
constant of their type from any table in Properties and show its value read-only (number, color square, green `1` /
grey `0` square). Values are copied into the generated WESL as literals — there is no uniform: saving a table
regenerates the shaders that read it. New constants start at 1.0 (`true`). Changing a constant's type or deleting it
(also by undo / redo, or by deleting the table file), when nodes read it, asks first, then saves the table and clears
those references (the nodes output 1.0 / `true`, W106).

| Action | Keys |
| --- | --- |
| Save (and regenerate) / all | Ctrl+S / Ctrl+Shift+S |
| Undo / redo | Ctrl+Z / Ctrl+Y, Ctrl+Shift+Z |
| Copy / cut / paste / duplicate | Ctrl+C / Ctrl+X / Ctrl+V / Ctrl+D (in a generated-file tab Ctrl+C copies the file) |
| Select all (nodes / curve points) | Ctrl+A |
| Delete selection | Delete, Backspace |
| Focus selection / whole graph | F / Home |
| Node palette (search by name, category, synonyms: `*`, `+`, `lerp`) | Tab, RMB click on the canvas |
| Palette filtered by wire type + auto-connect | release a wire on empty space |
| Node under the cursor: Constant/2/3/4Vector, Scalar/Vector parameter, TextureSample, Multiply, Add, Lerp, TexCoord, Reroute | `1` `2` `3` `4` `S` `V` `T` `M` `A` `L` `U` `R` + LMB |
| Comment frame around the selection | C |
| Reroute point on a wire / delete a wire | double click / RMB click on the wire |
| Pin menu: polymorphic type / reset value / disconnect | RMB click on an input pin |
| Disconnect every wire of a pin / a wire | Ctrl+LMB on the pin / on the wire |
| Named route: a wire without a visible wire (Named Route + one usage per place) | palette: "Named Route", then "Named Route Usage: {name}" |
| Auto-layout the selection (or the whole graph) | Ctrl+L |
| Close tab / next tab | Ctrl+W (or MMB on the tab) / Ctrl+Tab |
| Close several tabs (all, others, left/right, unmodified) | RMB on a tab |
| Open an asset | double click in the browser |
| Scroll a generated file horizontally | Shift+wheel |
| Curve: add a point / move / multi-select | double click on empty space / drag a point / Shift+click |
| Curve drag: lock to the T / value axis, snap to the 0.1 grid | hold X / Y, hold Shift |
| Curve: nudge the selected points | arrow keys (Shift: ×10) |
| Curve: point menu / plot menu (add point, select all, preview mode) | RMB on a point / on empty space |
| Open project / quit | Ctrl+O / Ctrl+Q |

Pan — drag with the right (or middle) mouse button (an RMB press that does not move is a click), zoom — wheel, box select — LMB on empty space; the minimap
in the bottom left is clickable. Unsaved tabs are marked with `*` (browser rows with an orange dot); on close and on an external change
of the file (git checkout) the editor asks what to do. Window size, recent projects and open
tabs are stored in `<config dir>/fe3dback/bevy-shader-editor/config.ron`, where the config dir is `%APPDATA%` on Windows,
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
- A function (`*.function.ron`, `kind: Function`) declares inputs with `output::FunctionInput` nodes and
  outputs with `output::FunctionOutput` nodes. Changing a function regenerates the dependent materials.
- A curve (`*.curve.ron`): `Curve(format_version: 1, name: "pulse", points: [(0.0, 0.0), (0.25, 1.0), (1.0, 0.0)])`
  — points sorted by `t`, clamped to `0..1`. It is generated into `autogen/curves/<path>.wesl` (a const array of
  `(t, value, slope)` and `sample_curve_simple` / `sample_curve_catmullrom`); shaders import the one they sample.
  Materials may not live in `shaders/curves/` (like `stdlib/` and `functions/`).
- A global constant table (`*.consts.ron`): `GlobalConsts(format_version: 1, name: "tuning", consts:
  [GlobalConst(name: "glow_amount", ty: F32, value: Float(0.35)), ...])`, `ty` one of `F32`, `Vec2`, `Vec3`,
  `Vec4`, `Bool`, `CompileFlag` (values `Float`, `Float2`…`Float4`, `Bool`). Nodes store only the constant's name.
- User node (`shaders/.nodes/*.ron`):
  `UserNode(id: "soft_clamp", name: "SoftClamp", category: "User", inputs: [UserPin(name: "x", ty: Float3)], output: Float3, template: "return x / (vec3<f32>(1.0) + abs(x));")`.
- Asset and parameter names are `snake_case` (`[a-z][a-z0-9_]*`) and not Rust/WGSL keywords.
- Autogen files start with the marker `AUTOGENERATED by shader-editor v1 … do not edit`; the editor does not
  overwrite files without the marker (E008). Writes are atomic and happen only when the content changes;
  orphaned files are deleted.

## Diagnostics

| Code | Condition |
| --- | --- |
| E001 | missing or multiple output nodes |
| E002 | cycle |
| E003 | incompatible types (no allowed auto-conversion; a Global* node reading a constant of another type) |
| E004 | required input not connected (or a Named Route Usage without its declaration, a Sample Curve without a curve) |
| E005 | invalid or duplicate name (also a global constant defined in two tables) |
| E006 | nonexistent function / recursive functions / missing or broken curve / missing global constant |
| E007 | WESL of a Custom node does not parse |
| E008 | an autogen file without the marker blocks the write |
| E009 | unknown node / broken file |
| E010 | generated WESL failed wesl+naga (`--compile`, saving in the editor) |
| W101–W105 | dead branch, inactive output pin, mesh attribute may be missing, vector truncation, `crates/shaders` not in `[workspace.members]` |
| W106 | a Global* node without a constant (outputs 1.0 / `true`) |

An error blocks generation only of its own asset; the rest are regenerated.

## Development

```sh
cargo test --workspace                           # unit, pipeline, snapshot + WESL compilation, UI smoke
cargo clippy --workspace --all-targets -- -D warnings
INSTA_UPDATE=always cargo test -p shader_project --test snapshots   # accept codegen changes
cargo run -p shader_project --example make_sample -- examples/sample_game   # rebuild the example
cd examples/sample_game && cargo clippy --workspace -- -D warnings  # the generated crate
```

The snapshot suite (`crates/shader_project/tests/snapshots/`) has one material per stdlib node
plus the reference graphs; every `.wesl` is compiled against the real Bevy 0.20 sources in the
forward, forward without UV/tangents and deferred (`PREPASS_PIPELINE`) variants. `SAMPLE_SCREENSHOT=out cargo run`
in `examples/sample_game` renders the scene, saves `out-1.png`/`out-2.png` and exits.

| Crate | Role |
| --- | --- |
| `shader_graph_core` | model, types and conversions, node registry, validation, IR (no Bevy) |
| `shader_stdlib` | standard library nodes, stdlib WESL modules, user nodes |
| `shader_codegen_wesl` | WESL generation, compile check (wesl + naga) |
| `shader_codegen_rust` | generation of `crates/shaders` |
| `shader_project` | project layout, regeneration pipeline, atomic writes, asset operations, watcher |
| `shader_editor_app` | GUI on Bevy + Feathers + bevy_noodle |

## Limitations

WPO is not visible to the prepass/shadows; function outputs of type Texture2D are not supported; user nodes
have a single output; Static Switch is out of v1; there is no native folder picker (the path is typed in,
with a list of recent projects); sliders on nodes drag at the wrong speed and the color picker opens off its button when the canvas
is zoomed (Bevy 0.20 bugs, see decision 0012). Details — [`docs/decisions/0010-v1-limitations.md`](docs/decisions/0010-v1-limitations.md).
