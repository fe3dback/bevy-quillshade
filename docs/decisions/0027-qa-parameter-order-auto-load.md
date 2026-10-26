# 0027. Parameters sorted by name, optional `load()`, Quick Start steps, texture thumbnails beside the outputs

**Context.** QA feedback on the baked materials work (decisions 0024–0026):
- The **Order** field of material parameter nodes set the argument order of the generated `new(..)` (spec §11.3, "the
  order set in the editor"). Since decision 0025 games take materials from `BakedMaterials`; `new` / `set_*` are
  crate-internal, so the field only reordered code nobody calls (and the texture bindings).
- `BakedMaterialsLoader::load()` had to be called by the game; without it the reserved handles stayed empty. Its
  result told nothing about the work started.
- Help → Quick Start was one long page; the loader step looked mandatory.
- The 64×64 thumbnail of texture nodes sat in its own row above six output pins whose labels leave the left half of
  the node empty, so the node was tall and the picture small.

**Decision.**
- **Parameter order.** `param::*` nodes have no `order` property any more. `build_ir` sorts material parameters by
  name (ties: node id, only possible with a duplicate name, which is E005 anyway). This fixes the constructor
  arguments, the uniform field order among same-size fields and the texture bindings `101/102`, … of texture
  parameters. `Asset::normalize` drops a stored `order` from `param::*` nodes, so old files load and lose the key on
  the next save; `format_version` stays 1. `FunctionInput` / `FunctionOutput` keep their **Sort Priority**: it orders
  the pins of a function call and the arguments of the generated `fn`, which users do see.
- **`load()` is optional.** `BakedPlugin` runs `auto_load` in `PreUpdate` (before the material insertion) while
  nothing started loading, so a game that never calls `load()` gets the same result one frame later. Calling it in
  `Startup` is still needed to read `BakedTextures` there and to know the total up front. `load()` returns the number
  of textures being loaded (`BakedMaterialsProgress::total`; a repeated call returns the current total). A game can no
  longer postpone loading past the first frame.
- **Quick Start** is a set of seven tabs (`Dialog::QuickStart(step)`, `Action::QuickStart(step)` opens or switches;
  Back / Next / Close in the footer): Project, Material, Cargo.toml, Plugin, Spawn, Loading (marked optional),
  Hot reload, each with a longer code example. `SHADER_EDITOR_DEMO=quick_start:N` opens step N.
- **Thumbnails beside the outputs.** `NodePreview::BESIDE_OUTPUTS` puts a preview kind in a fixed-width column left
  of the node's output pins (`node_preview::previews` splits the rows); the texture thumbnail uses it at 128×128. A
  node without outputs falls back to a row. No bevy_noodle change was needed: a port finds its node through its
  ancestors and is positioned against its own row, and the output column ends at the node's content edge, so the
  ports stay on the border.

**Consequences.**
- Spec §11.3 / §17 ("`new` in the order set in the editor", "`RockWetExt::new` requires 5 arguments in the given
  order") now read "sorted by name": rock_wet's order is `albedo, normal_map, speed, tint, wetness`. Snapshots and the
  sample's generated crate changed accordingly. Renaming a parameter can move its binding number — harmless, since the
  WESL and Rust are regenerated together.
- Texture nodes are wider and taller than other nodes; graphs laid out for the old size may overlap until moved or
  auto-laid out.
