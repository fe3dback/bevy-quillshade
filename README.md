<p align="center">
  <img src="assets/logo.png" alt="Quillshade logo" width="256">
</p>

<h1 align="center">Quillshade</h1>

<p align="center">
  <b>Standalone asset editor for <a href="https://bevy.org">Bevy</a>: author materials, textures, animations and other content.</b>
</p>

<p align="center">
  <img alt="Version 0.2" src="https://img.shields.io/badge/version-0.2-blue">
  <img alt="Bevy 0.20" src="https://img.shields.io/badge/bevy-0.20-232326">
  <img alt="Status: alpha" src="https://img.shields.io/badge/status-alpha-orange">
  <img alt="License: MIT" src="https://img.shields.io/badge/license-MIT-green">
</p>

> [!WARNING]
> **Alpha software — not stable, use at your own risk.** Quillshade is an unofficial, community-driven project and is
> not affiliated with or supported by the Bevy developers. File formats, generated code and the UI may change without
> notice between versions.
> Editor Code mostly written by LLMs

## What it is

Quillshade is an **external content tool** — closer to Blender than to the Unreal Editor. You run it next to your game,
it produces assets, and the game loads them like any other assets.

<p align="center">
  <img src="docs/images/workflow.svg" alt="Content tools (Blender, Quillshade) and a scene editor (Jackdaw / Bevy Editor) write files into the game repository; the game depends only on those files" width="100%">
</p>

- **Standalone.** A separate application that knows nothing about your game: not its components, systems, resources
  or scenes. It complements a scene editor such as [Jackdaw](https://github.com/jbuehler23/jackdaw) or the official
  Bevy editor instead of replacing it.
- **No levels, no scenes.** Placing entities and building levels is a scene editor's job; Quillshade does not do it
  and never will.
- **Not part of your build.** The game does not depend on Quillshade and does not know it exists. It depends only on
  the generated output — a plain Rust crate (Bevy, serde, ron), WESL shaders and data files — committed to your
  repository like hand-written code. Remove the editor and the game still builds.

In terms of data editing it is closer to a runtime inspector such as `WorldInspector`
([bevy-inspector-egui](https://github.com/jakobhellermann/bevy-inspector-egui)) — except it edits files on disk
instead of a running world:

<p align="center">
  <img src="docs/images/concept.svg" alt="A runtime inspector edits game data inside the game process; Quillshade is a separate app that writes generated assets, which the game builds and hot-reloads" width="100%">
</p>


## Quick Start

### Download

Prebuilt binaries for Linux (x86_64), Windows (x86_64) and macOS (Apple Silicon, Intel) are attached to every
[release](https://github.com/fe3dback/bevy-quillshade/releases).

### Installation from source

Requires Rust ≥ 1.97.1 and the same system dependencies as Bevy itself (see
[Bevy setup](https://bevy.org/learn/quick-start/getting-started/setup/)).

```sh
git clone https://github.com/fe3dback/bevy-quillshade
cd bevy-quillshade
# Without the default `dynamic_linking` feature (fast builds while developing) the binary runs on its own.
cargo build --release --no-default-features
./target/release/shader-editor
```

### 1. Project

**File → New Project…** on your game crate (the directory with its `Cargo.toml`).

```text
my_game/
  Cargo.toml                 # yours: the editor never edits it
  quillshade/
    project.ron              # optional: which domains are enabled
    material/                # your graphs: *.material.ron, *.function.ron, *.texture.ron, ...
    light/                   # environments, curves and cube maps: *.environment.ron, *.curve.ron, *.texture.ron
  assets/quillshade/material/autogen/    # generated WESL + baked.ron (do not edit)
  assets/quillshade/material/internal/   # built-in textures (generated)
  assets/quillshade/light/autogen/       # generated light baked.ron (do not edit)
  crates/quillshade/         # generated Rust crate, one module per domain (do not edit)
```

Sources live in `quillshade/<domain>/` (`material`, `light`); `quillshade/project.ron` can switch a domain off
(`domains: { "material": (enabled: false) }`, a missing entry means enabled). Without the file every default applies;
a `project.ron` that does not parse is an error — the CLI exits with code 2 and the editor refuses to open the project,
naming the file — rather than being replaced by defaults. The editor re-reads the file when it changes on disk.
In the editor, File → Project Settings... edits it: the
header date and each domain's "Used in game" (enabled by default). Saving writes the file and rewrites
`crates/quillshade/Cargo.toml` and `src/lib.rs` (unchecking a domain deletes its generated `crates/quillshade/src/<domain>/`
module, removes its plugin from `QuillshadePlugin`, and disables the domain in the editor while keeping its
`quillshade/<domain>/` source files); an outside edit reloads a clean Project Settings tab and asks Keep mine / Reload
for one with unsaved changes.

### 2. Cargo.toml

Add the generated crate to the game:

```toml
[workspace]
members = ["crates/quillshade"]

[dependencies]
bevy = { version = "0.20", features = ["file_watcher"] }   # file_watcher: hot reload
quillshade = { path = "crates/quillshade" }
```

### 3. Plugin

Add `QuillshadePlugin` after `DefaultPlugins`:

```rust
use bevy::prelude::*;

fn main() {
    App::new()
        .add_plugins((DefaultPlugins, quillshade::QuillshadePlugin))
        .add_systems(Startup, spawn)
        .run();
}
```

### 4. Spawn

Every material is a field of `BakedMaterials`, by its path under `quillshade/material/`:

```rust
use quillshade::material::BakedMaterials;

fn spawn(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, baked: Res<BakedMaterials>) {
    // Pbr3d needs tangents for a Normal input.
    let sphere = Sphere::new(1.0).mesh().uv(48, 24).with_generated_tangents().unwrap();
    commands.spawn((Mesh3d(meshes.add(sphere)), MeshMaterial3d(baked.env.rock_wet.clone())));
    commands.spawn((Camera3d::default(), Transform::from_xyz(0.0, 1.0, 4.0)));
}

// Unlit3d: MeshMaterial3d(baked.fx.hologram.clone()) as well.
// Sprite2d: (Sprite::from_image(image), SpriteMaterial(baked.ui.dissolve.clone())).
// By id, e.g. from your own asset data (None for an unknown id or another type):
// baked.get::<RockWetMaterial>("env/rock_wet"), tex.get("textures/rock_albedo").
```

## Features

### Material Editor

Node-based material authoring in the spirit of the UE5 Material Editor. Graphs are saved as RON, and every save
regenerates WESL shaders and a typed Rust crate that the game hot-reloads.

<p align="center">
  <img src="docs/images/ui_mat_editor.png" alt="Material graph editor" width="100%">
</p>

- **Three material kinds**: PBR 3D (an `ExtendedMaterial` over `StandardMaterial`, so shadows, prepass and deferred
  work out of the box), Unlit 3D and Sprite 2D. World Position Offset is supported too.
- **130+ built-in nodes**: math, vectors, colors, coordinates, textures, noise, masks and more. Implicit type
  conversions are applied on the wires.
- **Material Attributes**: make, break and blend whole PBR layers to build layered materials.
- **Functions**: reusable sub-graphs with their own inputs and outputs, shared across materials.
- **Custom code**: a Custom node with inline WESL, plus project-wide user nodes.
- **Curves**: a curve editor (Catmull-Rom or linear) whose curves are sampled from shaders.
- **Gradients**: collections of up to 128 gradients each (OKLab interpolation), baked into one texture per collection.
- **Textures**: Texture assets for the game's `.png` / `.ktx2` images, with loader and sampler settings and a preview.
- **Global constants**: named tables of values, inlined into every material that reads them.
- **Scene depth**: SceneDepth and DepthFade (soft particles, shorelines) read the camera's depth prepass in
  translucent 3D materials; add `DepthPrepass` to the camera.
- **Comfortable canvas**: comments, named routes, auto-layout, minimap, node preview, copy / paste,
  undo / redo, Back / Forward navigation.
- **Live validation**: errors and warnings are pinned to their node and pin, and an error blocks only its own asset.
- **Readable output**: view the generated WESL and Rust right in the editor. The game gets typed material structs and
  baked material and texture handles, and hot-reloads both the shaders and the baked values.
- **Headless CLI**: `regen`, `check` and `diff` for CI; `--domain <id>` (`material`, `light`; repeatable) limits a
  run to those domains (default: all), e.g. `shader-editor check my_game --domain light`.

<table>
  <tr>
    <td width="50%"><img src="docs/images/ui_mat_funcs.png" alt="Function graph"><p align="center">Functions</p></td>
    <td width="50%"><img src="docs/images/ui_curve_editor.png" alt="Curve editor"><p align="center">Curves</p></td>
  </tr>
  <tr>
    <td><img src="docs/images/ui_gradients.png" alt="Gradient collection editor"><p align="center">Gradients</p></td>
    <td><img src="docs/images/ui_texture_ed.png" alt="Texture asset editor"><p align="center">Textures</p></td>
  </tr>
  <tr>
    <td><img src="docs/images/ui_gen_wesl.png" alt="Generated WESL"><p align="center">Generated WESL</p></td>
    <td><img src="docs/images/ui_gen_rust.png" alt="Generated Rust"><p align="center">Generated Rust</p></td>
  </tr>
</table>

#### Planned

- **Instanced materials**: material instances that share draw calls (draw-call optimization).

### Light

Author complete lighting, skybox, fog, and camera post-processing environments (`quillshade/light/*.environment.ron`),
transition curves (`quillshade/light/*.curve.ron`), and cube-map Texture assets (`quillshade/light/*.texture.ron`,
KTX2 cube maps), and switch or transition smoothly between presets at runtime.

- **Environment presets (`*.environment.ron`)**:
  - **Sky & IBL**: camera `ClearColor`, optional `.ktx2` cube-map skybox (brightness in cd/m², yaw rotation), and
    `EnvironmentMapLight` (intensity, yaw rotation, procedural hemispherical gradient `sky_top` / `sky_horizon` /
    `ground` colors when no cube map is set, or the cube map).
  - **Ambient & Directional Lights**: `GlobalAmbientLight` (color, brightness) and any number of `DirectionalLight`
    slots (name, color, illuminance in lux, azimuth/elevation angles, cascade shadows, contact shadows, shadow biases).
  - **Camera Exposure, Tonemapping & Color Grading**: `Hdr`, `Exposure` (`ev100`), `Tonemapping`, `DebandDither`, and
    full 4-section `ColorGrading` (`global`, `shadows`, `midtones`, `highlights`).
  - **Distance Fog**: `DistanceFog` with `Linear`, `Exponential`, `ExponentialSquared`, or `Atmospheric` falloff and
    smooth cross-mode interpolation.
- **Transition curves (`*.curve.ron`)**: authored with the curve editor (`curve_kit`), evaluated with Catmull-Rom
  cubic Hermite interpolation at runtime, and implementing `bevy::math::curve::Curve<f32>`.
- **Typed runtime API (`quillshade::light`)**:
  - Autogenerated `Preset`, `Curve`, and `State` (`#[derive(States)]`) enums (`State::None`, `State::Transitioning`,
    and one variant per preset, supporting nested subdirectories such as `outdoor/day` → `OutdoorDay`).
  - `Light` (`SystemParam`): `light.set_immediate(Preset::Day)`,
    `light.transition_to(Preset::Night, Curve::Smooth, Duration::from_secs(3))`,
    `light.transition_linear_to(Preset::Night, Duration::from_secs(3))`, plus `prev_preset()`, `next_preset()`,
    `current_preset()`, `progress()`, `sampled_progress()`, `is_transitioning()`, and `current_values()`.
  - State hooks & messages: `OnExit(State::Day)` fires immediately when a transition begins (`t = 0.0`);
    `OnEnter(State::Night)` fires when the transition finishes (`t = 1.0`), alongside `LightTransitionStarted` and
    `LightTransitionFinished` messages.
  - **Mid-transition interruption & virtual time**: calling `transition_to` while a transition is in flight smoothly
    interpolates from the current mid-transition snapshot toward the new target preset while preserving `prev_preset()`.
    Transitions advance on `Time<Virtual>`, respecting game pause and time scaling.
  - **GPU dual-cubemap blended skybox**: custom `BlendedSkyboxPlugin` (`blended_skybox.wesl`) cross-fades two skybox
    cube maps on the GPU (and alpha-fades against `ClearColor` when transitioning to or from `cubemap: None`).
  - **Hot-reload**: `assets/quillshade/light/autogen/baked.ron` hot-reloads environment values, curves, and cube-map
    texture settings while the game is running without recompiling Rust.
- **Validation (`LE*` / `LW*`) & Game Code Conflict Scanner (`LW102`)**: validates environments, curves, and cube maps
  (`LE001`–`LE003`, `LE009`, `LE011`–`LE014`, `LW101`–`LW103`, `LW109`), including scanning game `.rs` files for manual
  `DirectionalLight`, `Skybox`, `DistanceFog`, `Exposure`, `Tonemapping`, `ColorGrading`, or `ClearColor` usages
  (`LW102`, clickable in the editor console to open the offending source file).

### Animation Graph

> [!NOTE]
> **Planned for a future release.**

## Workspace crates

- `project_core` — domain-neutral backend: project layout, `project.ron`, atomic writer, regen driver, the umbrella
  `crates/quillshade`. `curve_kit` and `texture_kit` — models shared by domains (curves, texture assets), without
  Bevy; their `editor` feature (default on, off for generator backends) adds the curve editor every domain with
  curves registers (`CurveDomain`) and the Texture / game image tabs every domain with Texture assets registers
  (`TextureDomain`, its `TexturePolicy` decides the image formats, the built-in textures and the Easy presets).
- `shader_graph_core` → `shader_stdlib` → `shader_codegen_wesl` → `shader_codegen_rust` → `shader_project` — the
  Material domain's backend (model, node library, WESL and Rust codegen, regeneration), without Bevy.
- `editor_core` — the editor shell (Bevy UI + Feathers): window layout (the mainbar with the domain tabs, the
  secondbar with Back / Forward and the tabs), menus, trees, the console filtered by the active domain, dialogs, user
  config, undo history, property widgets, the generated-file viewer, tools (singleton tabs such as Project Settings,
  in the built-in Project domain) and the domain API (`EditorDomain`, `DomainRegistrar`, `DomainState`). It knows no
  domain.
- `node_graph_base` — the node graph UI every graph domain shares: the canvas over the `vendor/bevy_noodle` fork,
  its gestures, the palette, the minimap, in-node previews and the graph editor tools, behind `GraphDomain`.
- `light_domain` — the Light domain: environments, transition curves (`curve_kit`), and cube-map Texture assets
  (`texture_kit`); its backend is Bevy-free, its `editor` feature (default on) adds `LightDomain`.
- `material_domain` — the Material domain of the editor: material and function graphs, curves and Texture assets
  (through the kits), constant tables, gradient collections and their editors. `src/main.rs` lists the domains (one
  line each) for the GUI and the CLI. See decision 0044 (`docs/decisions/`).
