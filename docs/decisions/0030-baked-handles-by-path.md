# 0030. `BakedMaterials` / `BakedTextures` handles by asset id

**Context.** Games with their own asset types (level data, item definitions, …) need to name a material or a texture
in data and resolve it at runtime. The nested fields of decision 0025 (`baked.env.rock_wet`) are only reachable from
code. The hidden `BakedTextures::get` matched on the compiled ids; materials had nothing.

**Decision.**
- Both resources get a root field `by_path`, keyed by asset id as in the manifest: the path under `shaders/` without
  the extension (`env/rock_wet`, `textures/rock_albedo`, `@internal/default`).
- **Materials.** The field is `by_path: HashMap<String, UntypedHandle>`, because the materials have different asset
  types. `reserve` fills it once: the handles are reserved and never change. Typed access is
  `BakedMaterials::get::<M>(id) -> Option<Handle<M>>` (`UntypedHandle::try_typed`), which is `None` for an unknown id
  or another `M`.
- **Textures.** The field is `by_path: HashMap<String, Handle<Image>>`, and `BakedTextures::get(id)` is now public and
  reads it.
  - `set` writes the field (if any) and the map. A texture that has no field yet (created in the editor after the
    build and used by a material, loaded by path) is therefore in the map too.
  - The new `unset` (a texture leaves the manifest) sets a field back to the default texture, and removes a
    field-less texture from the map.
  - The map is empty until `load()` (`Default`). The old field lookup stays as the hidden `field(id)` for the crate's
    own test.
- **Naming.** `by_path` is reserved at the root of both structs, like `internal` for textures: a root folder or asset
  named `by_path` gets its prefix (`dir_by_path`, `mat_by_path`, `tex_by_path`). Maps are `std::collections::HashMap`
  with `String` keys, so `get(&str)` works and textures added at runtime fit.

**Consequences.** A game can store `"env/rock_wet"` in its own assets and call `baked.get::<RockWetMaterial>(..)` or
`tex.get(..)`. A game that needs any material type behind one id uses `baked.by_path[id]` (an `UntypedHandle`).
`by_path` is a public field: a game that writes into it only breaks its own lookups.
