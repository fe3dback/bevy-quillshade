# Spec: Shader Editor Toolset

Oct 25, 2026 · @fe3dback

## 1. Goal and context

We need a desktop node-based shader editor for a game on Bevy 0.20 — a functional counterpart of the Unreal Engine 5 Material Editor that covers \~90% of typical material work. The editor does not render a preview: it saves the graph and on every change generates WESL shaders and a Rust crate, and the game picks them up through asset hot-reload.

The result is a toolset of three parts:

- **Editor** (GUI): opens a game "project", creates and edits materials and functions as node graphs.
- **Code generator**: turns graphs into WESL (`assets/shaders/autogen/**`) and into the Rust crate `crates/shaders` with a `Plugin`, material types and constructors.
- **Standard node library**: math, vectors, textures, coordinates, utilities — a set comparable to the UE5 palette.

Customer: a game developer on Bevy 0.20. Contractor: the team/agents implementing the toolset according to this spec. Architecture within the given constraints is up to the contractor; mandatory decisions are marked **MUST**, recommendations **SHOULD**.

## 2. Terms

| Term | Meaning |
| --- | --- |
| Project | A directory with a `Cargo.toml` file and an `assets/` subdirectory. The game root. |
| Editor asset | A graph file in `{project}/shaders/**`: a material or a function. A source edited only by the editor. |
| Material | A graph with mandatory output nodes (Base Color, Roughness, …) from which one WESL shader and one Rust material type are generated. |
| Function | A reusable subgraph with Function Input / Function Output nodes; generated as a `fn` in WESL. The counterpart of a Material Function in UE. |
| Parameter | A material input node whose value is set from the game (`Scalar`, `Vector`, `Color`, `Texture`, `Bool`). Goes into the bind group and into the Rust type's constructor. |
| Constant | A node with a literal; inlined into WESL, does not appear in Rust. |
| Pin / port | An input or output of a node with a data type. |
| Autogen | Generated files: `assets/shaders/autogen/**` and `crates/shaders/**`. Never edited by hand. |
| WESL | A standardized extension of WGSL (modules, `import`, `@if`); since Bevy 0.20 the only shader dialect of Bevy. [wesl-lang.dev](https://wesl-lang.dev) |
| Stdlib | The editor's built-in node library and the WESL functions behind it. |
| IR | Intermediate representation of the graph (a typed DAG) between the editor model and the code generators. |

## 3. Scope

**In scope:**

- Opening/creating a project, an editor asset file browser, creating the `shaders/` directory and nested subdirectories.
- Node-based editing of materials and functions: node palette, search, type-checked connections, undo/redo, copy/paste, comment frames, reroute.
- Asset types: 3D PBR material (on top of `StandardMaterial`), Unlit/Custom material, 2D material (sprite/mesh2d), function.
- The standard node library (section 8).
- Generation of Bevy 0.20 compatible WESL into `assets/shaders/autogen/`.
- Generation of the Rust crate `crates/shaders` with a `Plugin`, material types and constructors.
- Automatic regeneration on save, atomic writes, removal of orphaned autogen files.
- A headless CLI to regenerate the whole project (for CI and builds without the GUI).
- Graph validation with clear errors in the UI.

**Out of scope:**

- Live preview / rendering the material inside the editor.
- Hot-reload on the game side (Bevy's stock asset file watcher is used; the editor only writes files).
- Post-process materials, compute shaders, UE decal/volumetric domains.
- Editing previously hand-written `.wesl` as graphs (reverse translation).
- Support for Bevy versions other than 0.20.x, and WebGL2 (`SIXTEEN_BYTE_ALIGNMENT` may be ignored in v1).

## 4. Target environment and dependencies

The game's target platform is [Bevy 0.20](https://bevy.org/news/bevy-0-20/) (released October 8, 2026), in which WESL became the only shader dialect and the old `#import`/`#ifdef` preprocessor was removed. The editor is a separate native application (Windows, Linux, macOS) and is not added to the game project as a dependency.

| Component | Version / source | Role |
| --- | --- | --- |
| Rust | ≥ 1.97.1 (required by bevy\_noodle) | language of the whole toolset |
| Bevy | 0.20.x | target engine; the editor is also built on Bevy (see below) |
| [bevy\_noodle](https://github.com/gold-silver-copper/bevy_noodle) | 0.5, MIT | graph canvas: nodes, ports, edges, undo/redo, save/load, subgraph, minimap, auto-layout |
| [wesl](https://docs.rs/wesl/) (wesl-rs) | 0.4.x | validation and compilation of the generated WESL in tests and the CLI |
| naga | the version from Bevy 0.20 | final WGSL validation after wesl |
| serde + ron | current | graph serialization |
| notify | current | project file watcher (external changes) |
| wgsl-analyzer | optional | LSP for manual debugging of the generated WESL |

**UI decision (accepted by the customer).** The original "UI on egui" requirement is dropped: bevy\_noodle is a headless graph library on **Bevy UI** (nodes and ports are Bevy UI entities, styled with Feathers and `bsn!`), not an egui widget. The whole editor interface MUST be built on Bevy UI + Feathers; egui and bevy\_egui are not part of the project.

- Graph canvas — bevy\_noodle (features `default_style`, `scene`): undo/redo, copy/paste, RON save/load, subgraph, type auto-conversions, minimap, auto-layout.
- Panels (asset browser, properties, palette, error log) — Feathers widgets from Bevy 0.20: list view, dropdown, color input, number input, tabs, menu.
- One UI system in the window gives a single focus, input and theme; a material preview, if needed later, is rendered by the same engine as the game.

bevy\_noodle SHOULD be added as a git dependency with a pinned `rev`; if changes are required, it is vendored into `vendor/bevy_noodle` as a path dependency with patches in separate commits so they can be sent upstream.

## 5. Project and file structure

A project is any directory that has a `Cargo.toml` file and an `assets/` subdirectory. The editor MUST refuse to open a directory without these markers and show the reason. Workspace projects (`[workspace]` at the root) are opened at the workspace root; `assets/` is looked up there as well.

The editor owns three trees inside the project:

```markdown
{project}/
├── Cargo.toml
├── assets/
│   └── shaders/
│       └── autogen/                 # AUTOGEN: WESL, overwritten
│           ├── stdlib/              # stdlib WESL modules (math.wesl, uv.wesl, ...)
│           ├── functions/{sub}/{name}.wesl
│           └── {sub}/{name}.wesl    # one file per material
├── crates/
│   └── shaders/                     # AUTOGEN: Rust lib crate
│       ├── Cargo.toml
│       └── src/
│           ├── lib.rs               # pub struct ShadersPlugin + re-exports
│           └── materials/{sub}/{name}.rs
└── shaders/                         # SOURCES: editor graphs, under VCS
    ├── .shader-editor.ron           # project settings (format version, codegen options)
    ├── {sub}/{name}.material.ron
    └── {sub}/{name}.function.ron
```

Rules:

- The editor creates `shaders/` on the first save if it does not exist. Any nesting of subdirectories is allowed; the `{sub}` structure is mirrored in autogen.
- Asset name = file name without the suffix. It MUST match `[a-z][a-z0-9_]*` (snake\_case), because it becomes a WESL module identifier and a Rust module identifier. The path `{sub}/{name}` MUST be unique among materials and among functions.
- `assets/shaders/autogen/` and `crates/shaders/` are fully managed by the editor: the editor does not touch any file without the `// AUTOGENERATED by shader-editor vX — do not edit` marker and reports it as a conflict.
- The editor MUST NOT edit the root `Cargo.toml`. If `crates/shaders` is not listed in `[workspace.members]`, the editor shows warning W105 with a ready-to-paste line, and generation proceeds as usual; hooking the crate up to the game is up to the developer.
- The editor does not create or delete files outside `shaders/`, `assets/shaders/autogen/`, `crates/shaders/`.

## 6. Data model

An asset source is a RON file with a versioned schema; the editor model does not depend on bevy\_noodle (bevy\_noodle is only the view), so that codegen and the CLI work without the GUI.

### 6.1 Asset types

| Type | File | What is generated | Mandatory output nodes |
| --- | --- | --- | --- |
| `Material::Pbr3d` | `*.material.ron` | WESL fragment (+ vertex with WPO) on top of `StandardMaterial`; Rust: `ExtendedMaterial<StandardMaterial, {Name}Ext>` | `MaterialOutput` (PBR), exactly one |
| `Material::Unlit3d` | `*.material.ron` | WESL fragment with its own color output; Rust: `impl Material` | `MaterialOutput` (Unlit), exactly one |
| `Material::Sprite2d` | `*.material.ron` | WESL fragment for `MaterialExtension2d`; Rust: `SpriteMaterial`/`ExtendedMaterial2d` | `MaterialOutput` (2D), exactly one |
| `Function` | `*.function.ron` | WESL `fn` in the functions module | ≥ 1 `FunctionOutput`, ≥ 0 `FunctionInput` |

The material type is chosen at creation and stored in the file; changing the type is an explicit operation that checks output node compatibility.

### 6.2 File schema (simplified)

```markdown
Asset(
  format_version: 1,
  kind: Material(Pbr3d(blend: Opaque, two_sided: false, shading: Lit)),
  name: "rock_wet",
  nodes: { "n1": Node(kind: "stdlib::TextureSample", pos: (120, 40), params: {...}),
           "n2": Node(kind: "param::Texture", pos: (0, 40), params: { name: "Albedo", default: "textures/rock.png" }), ... },
  edges: [ Edge(from: ("n2", "out"), to: ("n1", "tex")), ... ],
  comments: [ Frame(title: "UV", rect: (...), nodes: ["n3","n4"]) ],
  meta: ( description: "", tags: [] ),
)
```

- Node identifiers are stable (ULID/UUID) and never reused; positions are stored so that the VCS diff is readable (keys sorted, `ron` with `PrettyConfig`, fixed field order).
- A node `kind` is a string path (`stdlib::Add`, `param::Scalar`, `function::{sub}/{name}`, `output::Pbr3d`), which allows adding nodes without changing the format.
- Node parameters (`params`) are typed by the node description from the registry (section 7); an unknown node is a load error with a hint, and the file is not corrupted.
- `format_version` MUST be increased on incompatible changes; the editor MUST be able to migrate from every previous version.

### 6.3 Internal representation

Model → **IR**: a typed DAG after type resolution, function inlining (or `fn` calls), and insertion of conversions. Both generators (WESL, Rust) read only the IR, not the editor model. The IR also provides: the list of material parameters with types and order (for the bind group and the constructor), the list of used stdlib modules (for `import`), flags (`uses_vertex_normal`, `uses_wpo`, `alpha_mode`).

## 7. Node and type system

Pin types follow UE semantics: f32 scalar and vectors with implicit widening, plus object types for textures. All nodes are described declaratively in the **node registry** (name, category, pins, parameters, WESL template), so the stdlib can grow without changes to the UI and the generators.

### 7.1 Data types

| Pin type | WESL | Rust (in parameters) | Pin color |
| --- | --- | --- | --- |
| `Float` | `f32` | `f32` | grey |
| `Float2` | `vec2<f32>` | `Vec2` | green |
| `Float3` | `vec3<f32>` | `Vec3` | yellow |
| `Float4` | `vec4<f32>` | `Vec4` | pink |
| `Color` | `vec4<f32>` (linear) | `LinearRgba` | pink with an outline |
| `Bool` | `bool` | `bool` → `u32` in the uniform | red |
| `Int` | `i32` | `i32` | light blue |
| `Texture2D` | `texture_2d<f32>` + `sampler` | `Handle<Image>` | blue |
| `Any` (polymorphic) | inferred | — | white |

### 7.2 Connection and conversion rules

- `Float` → `FloatN`: broadcast (`vec3(x)`), automatic.
- `FloatN` → `FloatM`, N > M: truncation (`.xy`, `.xyz`), automatic — as in UE.
- `FloatN` → `FloatM`, N < M: **forbidden**; the UI suggests inserting `AppendVector`/`MakeFloat4`.
- `Color` ↔ `Float4`/`Float3`: automatic (`Float3` takes `.rgb`, in the other direction `a = 1.0`).
- `Int` → `Float`: automatic (`f32(x)`); `Float` → `Int`: only through an explicit `Truncate`/`Round` node.
- `Bool` connects only to `Bool` inputs (`If`, `BoolParameter`, logic).
- `Texture2D` connects only to `Texture2D` inputs (`TextureSample`, a `FunctionInput` of texture type).
- Polymorphic nodes (`Add`, `Multiply`, `Lerp`, …) infer the type by the "maximum input dimension" rule; a scalar input is widened. An unconnected input takes its default value, editable right on the node.
- Cycles are forbidden: checked in the bevy\_noodle `ConnectionCheck` observer before the edge is created. One input — at most one edge; one output — any number.

### 7.3 Output nodes per asset type

**`MaterialOutput (Pbr3d)`** — exactly one, cannot be deleted. Pins and their defaults when unconnected:

| Pin | Type | Default | Where it goes in WESL |
| --- | --- | --- | --- |
| Base Color | `Float3` | from `StandardMaterial` | `pbr_input.material.base_color.rgb` |
| Metallic | `Float` | from `StandardMaterial` | `pbr_input.material.metallic` |
| Specular | `Float` | 0.5 | `pbr_input.material.reflectance` |
| Roughness | `Float` | from `StandardMaterial` | `pbr_input.material.perceptual_roughness` |
| Emissive | `Float3` | 0 | `pbr_input.material.emissive.rgb` |
| Opacity | `Float` | 1.0 | `base_color.a` (active when blend ≠ Opaque) |
| Opacity Mask | `Float` | 1.0 | `alpha_discard` cutoff (active when blend = Mask) |
| Normal | `Float3` | vertex normal | `pbr_input.N` (tangent space → world via TBN) |
| World Position Offset | `Float3` | 0 | vertex shader, offset of `world_position` |
| Ambient Occlusion | `Float` | 1.0 | `pbr_input.diffuse_occlusion` |

Node settings (not pins): Blend Mode (`Opaque | Mask | Blend | Add | Multiply`), Two Sided, Shading (`Lit | Unlit`). Inactive pins are dimmed.

**`MaterialOutput (Unlit3d)`**: `Color: Float3`, `Opacity: Float`, `Opacity Mask: Float`, `World Position Offset: Float3`. **`MaterialOutput (Sprite2d)`**: `Color: Float4` (defaults to `sample_final_color(uv)`), `UV override: Float2`.

**Functions**: `FunctionInput` (name, type, default value, "required" flag, sort order) and `FunctionOutput` (name, type). A function MUST have ≥ 1 output; input/output names are unique within the function. Inputs/outputs of type `Texture2D` are allowed — a `texture_2d<f32>` + `sampler` parameter is generated.

## 8. Standard node library

The set below is the minimum for v1 (MUST); names and categories follow UE5 so that users familiar with it can find their way without documentation. Every node has a WESL implementation: either a builtin WGSL function or a `fn` in an `autogen/stdlib/*.wesl` module (always generated by the editor, even if unused).

| Category | Nodes |
| --- | --- |
| **Constants** | Constant, Constant2Vector, Constant3Vector, Constant4Vector, Time (`globals.time`), DeltaTime, Pi, TwoPi, ViewSize |
| **Parameters** | ScalarParameter, VectorParameter (Float2/3/4), ColorParameter, TextureSampleParameter2D, TextureObjectParameter, BoolParameter. Fields: name (unique within the material), default value, group, min/max for the UI |
| **Math (scalar/per-component)** | Add, Subtract, Multiply, Divide, Power, Sqrt, Abs, Floor, Ceil, Round, Truncate, Frac, Fmod, Sign, Min, Max, Clamp, Saturate, OneMinus, Lerp, Step, SmoothStep, Exp, Exp2, Log, Log2, Sin, Cos, Tan, ArcSin, ArcCos, ArcTan, ArcTan2, Remap (ValueRange→TargetRange), DDX, DDY |
| **Vector** | Normalize, NormalStrength (tangent-space normal: XY × strength, renormalized), Length, Distance, DotProduct, CrossProduct, Reflect, Refract, AppendVector, ComponentMask (xyzw checkboxes), BreakOutFloat2/3/4, MakeFloat2/3/4, Swizzle, TransformVector (Tangent/Local/World/View) |
| **Coordinates** | TextureCoordinate (uv0/uv1, tiling), WorldPosition, ObjectPosition, VertexNormalWS, VertexTangentWS, VertexColor, CameraPositionWS, CameraVectorWS, ScreenPosition, PixelDepth, ObjectScale, Panner, Rotator, ScaleUVsByCenter |
| **Texture** | TextureSample (tex, uv, bias/level → rgb, r, g, b, a, rgba), TextureSize, TexelSize, FlipBook (atlas animation), NormalFromHeightmap (SHOULD) |
| **Utility** | If (A, B, A>B, A==B, A\<B), Fresnel, Desaturation, Blend (Overlay/Screen/Multiply/SoftLight), BlackBody (SHOULD), Noise (simple value/perlin, SHOULD), FunctionCall (any project function), Custom (hand-written WESL fragment: inputs by name, output type), Reroute, Comment (frame) |
| **Output** | MaterialOutput (per asset type), FunctionInput, FunctionOutput |

Requirements:

- All polymorphic math nodes work for `Float`…`Float4` following the rules of section 7.2.
- `TextureSample` without a connected `uv` uses `uv0`; without a connected texture — a validation error.
- `Custom` MUST be emitted as a standalone `fn` with named arguments, not as an inline substitution, so that WESL errors point to the node.
- Coordinates nodes use only what is available through Bevy's `VertexOutput` (`bevy_pbr::render::forward_io`) and `bevy_pbr::mesh_view_bindings`; a node requiring an attribute that may be unavailable (e.g. uv1 on a mesh without it) is flagged with a warning, not an error.
- The node registry SHOULD support user nodes from `shaders/.nodes/*.ron` (name, pins, WESL template) without rebuilding the editor.

## 9. User interface

The layout is as in the UE5 Material Editor: asset browser on the left, canvas in the center, properties on the right, error log at the bottom. The whole interface is Bevy UI + Feathers, the canvas is bevy\_noodle; egui is not used (section 4).

**Panels and functions:**

1. **Project / asset browser**: the `shaders/` tree with subdirectories; create a material (choosing the type), a function, a folder; rename, move (drag), delete (with confirmation and removal of autogen); open in a tab. The "Show generated WESL/Rust" context menu opens the file in an external program.
2. **Canvas** (bevy\_noodle): pan/zoom, box select, drag, connecting by dragging from a port, auto-conversions on connect, reroute points, comment frames, minimap, auto-layout of the selection, highlighting of compatible ports while dragging a wire. Several tabs with open assets.
3. **Node palette / search**: on RMB on the canvas or `Tab` — a popup with search by name, category and synonyms (UE-like: "\*" = Multiply, "lerp", "+"). When invoked from a dragged wire — filtered by compatible type, with auto-connect.
4. **Properties**: parameters of the selected node (default values, parameter name, groups, output node settings); asset properties (type, blend mode, description).
5. **Error / warning log**: results of validation and codegen; clicking an error focuses the node and highlights the pin.

**Commands and hotkeys** (SHOULD match UE where possible): Ctrl+S save (and regenerate), Ctrl+Z/Y, Ctrl+C/V/X, Delete, Ctrl+D duplicate, F focus on the selection, Home — the whole graph, `1`/`2`/`3`/`4`+LMB — Constant/2/3/4Vector, `S`+LMB — ScalarParameter, `T`+LMB — TextureSample, `M`/`A`/`L`+LMB — Multiply/Add/Lerp.

**State**: unsaved changes are marked in the tab title; on close — a prompt. External changes to a file (git checkout) — an offer to reload. Window settings and open tabs are saved in the user config (not in the project).

**Not required**: live preview, a shader statistics panel, editing WESL in the editor itself.

## 10. WESL generation

The generator MUST produce WESL that Bevy 0.20 compiles without edits: the `import a::b::{c, d};` syntax, conditional compilation `@if(FLAG)`/`@else`, the material bind group via `constants::MATERIAL_BIND_GROUP`, the `.wesl` extension. The reference is [extended\_material.wesl](https://github.com/bevyengine/bevy/blob/latest/assets/shaders/extended_material.wesl) and `shader_material.rs` from the Bevy examples.

### 10.1 Pipeline

1. Load the asset and all functions it references (transitively).
2. Validation (section 13); on error no files are written and the previous autogen stays.
3. Build the IR: type inference, conversion insertion, topological sort.
4. Emission: each node → one `let nN_out: T = ...;` line or a `fn` call; names are deterministic (`n_{id}_{pin}`) so the autogen diff is stable.
5. Write: only if the content changed (hash comparison), atomically (section 12).

### 10.2 Structure of a `Pbr3d` material file

```markdown
// AUTOGENERATED by shader-editor v1 from shaders/env/rock_wet.material.ron — do not edit
import bevy_pbr::render::{pbr_fragment::pbr_input_from_standard_material, pbr_functions::alpha_discard};
@if(PREPASS_PIPELINE) import bevy_pbr::{prepass::io::{VertexOutput, FragmentOutput}, deferred::functions::deferred_output};
@else import bevy_pbr::render::{forward_io::{VertexOutput, FragmentOutput}, pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing}};
import super::super::stdlib::math::{remap, saturate3};        // only the stdlib modules in use
import super::super::functions::common::wet_mask::wet_mask;    // project functions

struct RockWetParams { tint: vec4<f32>, wetness: f32, speed: f32, _pad0: f32, _pad1: f32 }
@group(constants::MATERIAL_BIND_GROUP) @binding(100) var<uniform> params: RockWetParams;
@group(constants::MATERIAL_BIND_GROUP) @binding(101) var albedo: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(102) var albedo_sampler: sampler;

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    // --- graph ---
    let n_01_uv: vec2<f32> = in.uv;
    let n_02_rgba: vec4<f32> = textureSample(albedo, albedo_sampler, n_01_uv);
    ...
    // --- outputs ---
    pbr_input.material.base_color = vec4<f32>(n_17_out, pbr_input.material.base_color.a);
    pbr_input.material.perceptual_roughness = n_21_out;
    pbr_input.material.base_color = alpha_discard(pbr_input.material.flags, pbr_input.material.alpha_cutoff, pbr_input.material.base_color);
    var out: FragmentOutput;
    @if(PREPASS_PIPELINE) { out = deferred_output(in, pbr_input); }
    @else { out.color = apply_pbr_lighting(pbr_input); out.color = main_pass_post_lighting_processing(pbr_input, out.color); }
    return out;
}
```

Rules:

- **Bindings**: the parameter uniform struct is always `@binding(100)`; textures/samplers are pairs `101/102`, `103/104`, … in parameter declaration order. `Unlit3d`/`Sprite2d` use the same numbering. Uniform struct fields are ordered by decreasing size (vec4, vec3+pad, vec2, f32), a `Bool` field → `u32`; the struct is padded to a multiple of 16 bytes.
- **Normal**: if the Normal pin is connected, `pbr_input.N = apply_normal_mapping(...)`/TBN from `in.world_tangent` is generated; a mesh without tangents — a warning.
- **World Position Offset**: when the pin is connected, a `@vertex fn vertex(...)` is also generated, repeating the stock `bevy_pbr::render::mesh::vertex` with the offset added; otherwise no vertex shader is generated.
- **Functions**: each project function → `fn {name}(args...) -> T` (or a `{Name}Out` struct with several outputs) in `autogen/functions/{sub}/{name}.wesl`; materials import them by the relative path `super::…`. Functions are not inlined.
- **Stdlib**: `autogen/stdlib/{module}.wesl` is regenerated on every project regeneration from the editor's resources; the stdlib version is written into the header.
- **Import paths**: resolution of relative `super::` imports between files in `assets/` MUST be verified on Bevy 0.20 at the very start of the work (spike, section 16); if Bevy's resolver does not support it, a fallback is allowed — inlining stdlib/functions into the material file with deduplication.
- **Determinism**: the same graph → a byte-for-byte identical file (no timestamps in the body; a date only in the header comment, optional and switchable off).
- A generated file MUST pass `wesl` 0.4 (compilation to WGSL with `PREPASS_PIPELINE` = false/true) and naga validation in tests.

## 11. Generation of the Rust crate `crates/shaders`

The `shaders` crate is a regular lib that the game hooks up with one line `app.add_plugins(shaders::ShadersPlugin)` and then creates materials through constructors with mandatory arguments. The crate MUST compile with `cargo check` against `bevy = "0.20"` without warnings with `#![deny(warnings)]` off, but `#![allow(clippy::all)]` is not allowed — the code must be clean.

### 11.1 Structure

```markdown
crates/shaders/
├── Cargo.toml            # [package] name = "shaders"; [dependencies] bevy = { version = "0.20", default-features = false, features = ["bevy_pbr", "bevy_sprite_render", ...] }
└── src/
    ├── lib.rs            # pub struct ShadersPlugin; pub mod materials; pub use materials::*; pub const AUTOGEN_VERSION
    └── materials/
        ├── mod.rs        # pub mod env; pub mod ui; ...
        └── env/
            ├── mod.rs
            └── rock_wet.rs
```

### 11.2 Example of a generated module (`Pbr3d`)

```markdown
// AUTOGENERATED by shader-editor v1 from shaders/env/rock_wet.material.ron — do not edit
use bevy::prelude::*;
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;

pub const SHADER_PATH: &str = "shaders/autogen/env/rock_wet.wesl";

#[derive(ShaderType, Debug, Clone, Copy, PartialEq)]
pub struct RockWetParams { pub tint: Vec4, pub wetness: f32, pub speed: f32, pub _pad0: f32, pub _pad1: f32 }

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub struct RockWetExt {
    #[uniform(100)] pub params: RockWetParams,
    #[texture(101)] #[sampler(102)] pub albedo: Handle<Image>,
}

impl MaterialExtension for RockWetExt {
    fn fragment_shader() -> ShaderRef { SHADER_PATH.into() }
    fn deferred_fragment_shader() -> ShaderRef { SHADER_PATH.into() }
    // fn vertex_shader() — only if the graph uses World Position Offset
}

pub type RockWetMaterial = ExtendedMaterial<StandardMaterial, RockWetExt>;

impl RockWetExt {
    /// All graph parameters are mandatory; order = parameter order in the editor.
    pub fn new(albedo: Handle<Image>, tint: LinearRgba, wetness: f32, speed: f32) -> Self { ... }
    /// Full material with a default StandardMaterial and blend/cull from the graph settings.
    pub fn material(self) -> RockWetMaterial { ExtendedMaterial { base: StandardMaterial { alpha_mode: AlphaMode::Opaque, ..default() }, extension: self } }
}

pub(crate) fn register(app: &mut App) { app.add_plugins(MaterialPlugin::<RockWetMaterial>::default()); }
```

`lib.rs` contains `ShadersPlugin`, whose `build` calls each module's `register`. The game: `materials.add(RockWetExt::new(h, LinearRgba::WHITE, 0.3, 1.0).material())`, then `MeshMaterial3d(handle)`.

### 11.3 Rules

- **The `new` constructor** takes exactly all material parameters (including textures as `Handle<Image>`), in the order set in the editor (the "order" field of a parameter; defaults to creation order). The graph's default values are available through `impl Default` only for the `Params` struct, not for textures.
- Argument types: `Scalar → f32`, `Vector → Vec2/Vec3/Vec4`, `Color → LinearRgba` (converted to `Vec4` when written into the uniform), `Bool → bool` (stored as `u32`), `Texture → Handle<Image>`.
- For each parameter a `set_{name}(&mut self, v)` is generated; identifier names are snake\_case from the parameter name; conflicts and Rust/WGSL keywords are a validation error.
- `Unlit3d` → `impl Material` (not an extension), `AlphaMode` from the graph settings; `Sprite2d` → `impl MaterialExtension2d`, plus `SpriteMaterialPlugin`/`Material2dPlugin` in `register`.
- The crate does not depend on the editor or its crates; its only dependency is `bevy` (minimal features) and, if needed, `bytemuck`.
- Explicit padding fields are generated for `Params` structs; the layout MUST match the WESL struct (single source of truth — the IR; a test compares offsets via `ShaderType::min_size`).
- The crate's `Cargo.toml` is overwritten completely; user edits to it are not supported (documented).

## 12. Automation and file writes

Regeneration runs on every asset save and covers exactly what depends on it; a game with Bevy's asset file watcher enabled picks up the new `.wesl` without a restart.

- **Triggers**: saving an asset (Ctrl+S); renaming/moving/deleting an asset; changing project settings; the "Regenerate all" command; CLI `shader-editor regen <project>`.
- **Incrementality**: when a function is saved, it and all materials/functions using it are regenerated (the reverse dependency graph is kept in memory and recomputed when the project is opened).
- **Atomicity**: write to a temporary file in the same directory + `rename`; on an error in any file of the change batch no file is replaced (first all files go to temp, then a series of renames).
- **Minimal noise**: a file is not rewritten if its content has not changed — otherwise hot-reload in the game fires for nothing.
- **Cleanup**: autogen files that no longer have a source are deleted (only if they carry the AUTOGENERATED marker); empty directories are deleted.
- **Write order**: stdlib and functions first, then materials, then the Rust crate; `crates/shaders/src/materials/mod.rs` and `lib.rs` are rebuilt from the full asset list.
- **External changes**: the editor watches (`notify`) `shaders/` and offers to reload an asset changed externally; it does not watch autogen.
- **Autosave** of a graph draft (SHOULD) — into the user cache, not the project; does not trigger regeneration.

## 13. Validation and error handling

Validation runs on every graph change (live highlighting) and before codegen (blocking); every message is tied to a node and, where possible, to a pin, and has a stable code.

| Code | Level | Condition |
| --- | --- | --- |
| E001 | error | `MaterialOutput`/`FunctionOutput` missing or duplicated |
| E002 | error | Cycle in the graph |
| E003 | error | Incompatible pin types without an allowed auto-conversion |
| E004 | error | A required input is not connected (e.g. the texture of `TextureSample`) |
| E005 | error | Parameter/asset name invalid or duplicated (snake\_case, Rust/WGSL keywords) |
| E006 | error | Reference to a nonexistent function or recursive function calls |
| E007 | error | A `Custom` node contains WESL that fails the `wesl` parser |
| E008 | error | An autogen file without the AUTOGENERATED marker blocks the write |
| W101 | warning | Node without influence on the output (dead branch) — not included in codegen |
| W102 | warning | Output pin inactive under the current Blend Mode (e.g. Opacity with Opaque) |
| W103 | warning | A vertex attribute that the mesh may lack is used (uv1, tangent, color) |
| W104 | warning | Vector truncation on auto-conversion (`Float4 → Float2`) |
| W105 | warning | crates/shaders is not listed in \[workspace.members\] of the root Cargo.toml — a line to paste is shown, generation is not blocked |

Behavior:

- Errors block generation only for the affected asset; the other assets of the project are regenerated.
- `wesl`/naga compile errors after generation (which should not happen, but) are shown in the log with the line of the generated file and a back-reference to the node via a `line → node id` map.
- I/O errors (no permissions, disk) — a modal message; the graph stays unsaved, no data is lost.
- The CLI returns a non-zero code on any error and prints messages in the `path:node:code: text` format, suitable for CI.

## 14. Architecture (recommendation)

The recommended split is a Cargo workspace of six crates; the only mandatory requirement here is that codegen and validation MUST work without the GUI (CLI), everything else is SHOULD.

&#91;embedded content: toolset architecture · 6 crates, 2 outputs into the game project\]

The editor and the CLI contain no generation logic — both call the core; the generators read only the IR, so the editor file format can change without touching the generators, and vice versa.

- `shader_graph_core` does not depend on Bevy (only `serde`, `ron`, `thiserror`), so that the CLI builds fast and can be used in the game project's `build.rs` if desired.
- `shader_stdlib` registers nodes in the core registry through a single trait `NodeDef { id, category, inputs, outputs, params, emit(&IrCtx) -> WeslExpr }`; user nodes from `shaders/.nodes/*.ron` use the same trait through a data-driven implementation.
- `shader_editor_app`: Bevy 0.20 + `bevy_noodle` (feature `default_style`, `scene` for undo/copy/save), Feathers for the panels. "Model ↔ bevy\_noodle entities" sync is one-way: the core model is the source of truth, the canvas is rebuilt on `EditApplied`.
- If bevy\_noodle needs changes (e.g. type labels on ports, pin color scheme, search panel), the library is vendored into `vendor/bevy_noodle` as a path dependency; patches go in separate commits with descriptions for an upstream PR.
- One `shader-editor` binary with subcommands: no arguments — GUI; `regen <project>`, `check <project>` (validation without writing), `diff <project>` (what would change).

## 15. Testing and quality control

The main quality criterion is that the generated code compiles and works in Bevy 0.20 without manual edits; this is checked automatically on every commit.

| Level | What is checked | Tool |
| --- | --- | --- |
| Unit (core) | RON load/save, version migrations, type inference, auto-conversions, cycle detection, E/W codes | `cargo test`, fixtures in `tests/fixtures/*.ron` |
| Snapshot (codegen) | Byte-for-byte WESL and Rust output for a set of reference graphs (one per stdlib node + 10–15 composite materials) | [insta](https://crates.io/crates/insta) |
| WESL compilation | Every snapshot `.wesl` is compiled with `wesl` 0.4 using a resolver that emulates the `bevy_pbr` modules (stubs), with `PREPASS_PIPELINE` = 0 and 1; the result is validated by naga | `wesl`, `naga` |
| Rust compilation | The generated `crates/shaders` passes `cargo check` and `cargo clippy -D warnings` against `bevy = 0.20` | CI job |
| Integration | The example game `examples/sample_game` adds `ShadersPlugin`, spawns a mesh with each reference material, renders 1 frame headless and compares it with a reference image (PSNR tolerance) | Bevy headless render + image diff |
| Hot-reload | A test changes a graph through the CLI and checks that the `.wesl` is rewritten only when the content changes (mtime/hash) | integration test |
| UI | Canvas smoke tests: create a node, connect, undo, save — through the `bevy_noodle` API without a window; a manual checklist before release | `cargo test`, checklist |

Additionally: `cargo fmt`/`clippy` for the whole workspace; CI on Linux, Windows, macOS; MSRV 1.97.1 pinned in `rust-toolchain.toml`.

## 16. Implementation stages

The core and codegen are built before the UI so that the generated WESL is checked against real Bevy 0.20 from the first weeks; the customer did not set deadlines for the stages, and none are assigned here.

&#91;embedded content: stages M0–M5 · 3 checkpoints\]

Each stage ends with green CI and a short demo; the checkpoints after M0, M1 and M4 are a reason to stop and revisit the decisions of section 18, not a formality. M0 is intentionally small: three technical questions on which the whole autogen structure depends.

## 17. Acceptance criteria

The work is accepted when each item below is reproducible on a clean machine following the README instructions in a single pass.

- [ ] The editor opens a directory with `Cargo.toml` + `assets/` and refuses directories without them with a clear message.
- [ ] In the asset browser a material (Pbr3d, Unlit3d, Sprite2d) and a function are created in an arbitrary subdirectory of `shaders/`; renaming and moving update autogen.
- [ ] All stdlib nodes from section 8 are available through search, connect according to the rules of 7.2, auto-conversions are inserted, cycles and incompatible types are rejected with an error code.
- [ ] A function with 2 inputs and 2 outputs is called from a material; changing the function regenerates the dependent materials.
- [ ] The reference material "rock\_wet" (2 textures, 1 color, 2 scalars, Normal, Roughness, Emissive, Opacity Mask) after Ctrl+S produces `assets/shaders/autogen/env/rock_wet.wesl` and `crates/shaders/src/materials/env/rock_wet.rs`; `RockWetExt::new` requires exactly 5 arguments in the given order.
- [ ] `crates/shaders` builds with `cargo check` and `clippy -D warnings` against Bevy 0.20 without edits.
- [ ] The example game `examples/sample_game` renders the reference materials; editing a graph in the editor while the game is running updates the picture without a restart (hot-reload).
- [ ] All `.wesl` from the snapshot suite compile with `wesl` 0.4 + naga with `PREPASS_PIPELINE` 0 and 1.
- [ ] Regenerating again without changes does not rewrite any file (checked by mtime).
- [ ] Undo/redo, copy/paste, comment frames, reroute, minimap, node search with auto-connect work according to the UI checklist.
- [ ] `shader-editor regen`, `check`, `diff` work without a display and return a non-zero code on validation errors.
- [ ] CI is green on Linux, Windows, macOS; the README describes installation, hooking up `ShadersPlugin`, the file format and limitations.

## 18. Open questions and decisions left to the contractor

All three questions to the customer are resolved (answers below); the contractor decides the rest and records it in `docs/decisions/`.

**Customer decisions:**

1. **egui or Bevy UI.** Resolved: the whole interface is Bevy UI + Feathers, egui is dropped (section 4).
2. **Default material model — resolved.** Pbr3d = `ExtendedMaterial<StandardMaterial, Ext>` (the game gets all of `StandardMaterial`'s functionality for free: shadows, prepass, deferred). Rejected alternative — a fully custom `impl Material` with a manual call to `pbr_input_from_standard_material`; gives more control but duplicates the `StandardMaterial` bindings.
3. **Behavior when `crates/shaders` is missing from the workspace.** Resolved: the editor does not touch the root `members`; if `Cargo.toml` does not list the crate in \`\[workspace.members\]\`, warning W105 with instructions is shown, and code is generated as usual.

**Left to the contractor:**

- The exact RON file schema and the migration approach.
- How stdlib templates are stored (embedded strings, `include_str!`, or a custom DSL).
- Inlining functions instead of `fn` calls, if Bevy's import resolver does not support `super::` between files in `assets/` (spike in M0).
- Implementing World Position Offset in v1, or moving it to v1.1 if copying Bevy's stock vertex shader proves fragile.
- Format and location of the editor's user config.
- Static Switch / permutation parameters (shader defines via `Material::specialize`) — out of v1, but the data model must not preclude them.

**Risks:**

| Risk | Impact | Mitigation |
| --- | --- | --- |
| Bevy 0.20's WESL import resolver does not support relative imports between user files | stdlib/functions cannot be split into separate modules | spike in M0; fallback — inlining with deduplication |
| bevy\_noodle 0.5 is a young library (85 commits, 1 maintainer) | blocking canvas bugs | vendoring with patches; an adapter API layer so the library can be replaced |
| Changes to internal `bevy_pbr` modules (`pbr_input_from_standard_material`, `forward_io`) in 0.20.x patch releases | the generated WESL stops compiling | snapshot tests compile against a pinned version; the Bevy version is written into the autogen header |
| Uniform struct layout diverges between WESL and Rust | render artifacts without compile errors | a single layout source in the IR + a `min_size` test; explicit padding fields |
