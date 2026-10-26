# 0029. Hot reload layout guard, bindings matched by position, non-2D images (E014)

**Context.** QA found three crashes:

1. **Game: hot reload with new bindings.** A running game gets a regenerated `.wesl` through Bevy's file watcher, but
   its material types are compiled Rust. When a graph gains a parameter or a texture binding, the new shader declares
   a binding the compiled `AsBindGroup` layout lacks. `create_render_pipeline` then fails with "Shader global
   ResourceBinding { group: 3, binding: 115 } is not available in the pipeline layout". Bevy 0.20's default
   `RenderErrorHandler` quits the app on any render error. Decision 0025 listed such edits as "rebuild needed", but did
   not say the game would crash.
2. **Game: another fixed texture node wired in.** Fixed textures are named after their node (`ft_<node>`), and
   `baked.ron` keys bindings by name. Wiring a second TextureSample node to the output in place of the first keeps the
   same binding numbers (`101/102`) but changes the name. The game warned "not in the manifest" / "not compiled into
   the game" and kept the old texture, so only texture *parameters* appeared to hot-reload.
3. **Editor: a cube map as a Texture asset.** Bevy loads a KTX2 cube map (an environment map's diffuse / specular)
   with a `Cube` view. Every UI image is bound as a 2D view, so the Texture tab's preview (a `UiMaterial`) failed
   bind group creation ("expects dimension = D2, but given a view with dimension = Cube") and the editor quit. A game
   material would fail the same way on its `texture_2d` binding.

**Decision.**
- **Layout feature.** `Ir::layout_feature()` = `SG_LAYOUT_` + 8 hex digits of FNV-1a (stable across Rust versions,
  unlike `DefaultHasher`). The hash covers the material kind, whether it has a vertex stage, the uniform fields
  (name, type, offset), the uniform size and the texture binding numbers. Texture identifiers are left out.
  - Every material WESL gets a guard after its bindings:
    `@if(!SG_LAYOUT_xxxxxxxx) const_assert material_layout_changed__rebuild_the_game;`.
  - Every generated Rust material has `pub const LAYOUT_FEATURE` and a `specialize` that pushes it to the vertex and
    fragment shader defs: `Material`, `MaterialExtension` and `MaterialExtension2d` alike. `specialize` is now always
    generated; Bevy calls it for the main, prepass and deferred pipelines.
  - Bevy maps a boolean shader def to a WESL feature; an unknown feature is disabled. A shader with another layout
    therefore fails at naga parsing: "no definition in scope for identifier:
    `material_layout_changed__rebuild_the_game`".
  - Bevy creates shader modules inside a validation error scope (`load_module`), so this error is logged and the
    material stops rendering. The game is not taken down. Pipeline creation, which Bevy does not scope, is never
    reached.
- **Compile check.** `check::Files::enable(feature)` adds features to every compile. Regeneration enables the layout
  feature of every material it compiles, like the game does. The spike test checks that the guard fires without it.
- **Bindings by position.** Each material entry in `baked.ron` gains `layout` (its layout feature) and `bindings` (its
  texture binding idents in binding order). Both fields are omitted when empty, and `MANIFEST_VERSION` stays 1: older
  readers ignore unknown fields. When the runtime manifest's `layout` equals the compiled one, `overlay` matches the
  compiled bindings to the runtime ones by position; otherwise it matches them by name, as before.
  - With equal layouts the binding numbers and types match, so wiring another fixed texture node in hot-reloads.
  - With different layouts the shader does not compile anyway. The game logs one warning per new layout: "material
    `x` changed its parameters, texture bindings or World Position Offset; its shader does not compile until the game
    is rebuilt".
  - Uniform names stay in the hash: values are applied by name, so a renamed parameter must not silently feed another
    field at the same offset.
- **E014.** `texture_diagnostics` reads the first 48 bytes of a `.ktx2` image (`texture::ktx2_not_2d`). A face count,
  layer count or depth above 1 is an error: "the image is a cube map (6 faces); only 2D textures can be sampled".
  As with E011–E013, the asset is left out of the manifest and its bindings get `@internal/default`, so the game never
  binds it. PNG is always 2D.
- **Editor preview.** `game_image::decode` runs `stack_layers`: a cube map, array, 3D or 1D image becomes one 2D image
  of mip 0 with its faces / layers / slices stacked top to bottom (truncating mip-major data, which starts with mip 0
  of every layer). The image tab, the Texture tab and node thumbnails all go through it.

**Consequences.**
- A structural edit while the game runs makes that material invisible until the rebuild (plus one Bevy shader error
  and one game warning) instead of quitting the game. After the rebuild it renders again (checked on the sample game).
- Generated Rust materials always import and implement `specialize` (one more method each).
- The editor still runs with Bevy's default render error policy (quit on any wgpu error). Other ways to feed the UI a
  non-2D view would still quit it; none are known.
