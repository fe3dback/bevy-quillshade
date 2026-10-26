# 0025. Baked materials: compiled manifest, runtime overlay, loader and hot reload

**Context.** Design §3 (`docs/superpowers/specs/2026-10-26-baked-materials-design.md`). Every material and texture
was wired up by hand in game code (`RockWetExt::new(assets.load(..), ..)`). The generated crate should load
everything itself — textures with their Texture asset settings, then every material with its graph values — and
expose ready handles; data edits in the editor (parameter defaults, texture choice, sampler settings) should
hot-reload without a rebuild. Bevy 0.20 facts this relies on: `Assets::reserve_handle` + a later `insert` at that id
works (a mesh with an empty material handle is skipped until the asset appears); `AsBindGroup` waits silently and
forever for an image that failed to load; loader settings are not part of asset identity (the first load of a path
wins, `reload` reuses the original settings); a changed `Image.sampler` is not propagated to existing material bind
groups until the material asset itself changes. The generated crate uses `bevy` with `default-features = false`.

**Decision.**
- **Everything is compiled in; the manifest is an overlay.** Regeneration writes `assets/shaders/autogen/baked.ron`.
  The generated crate embeds that same file as its compiled snapshot —
  `pub const COMPILED_MANIFEST: &str = include_str!("../../../assets/shaders/autogen/baked.ron");` in `lib.rs` — so
  the data is not duplicated into Rust literals and both paths share one parser. `load()` uses only the snapshot,
  so the game works without `baked.ron` (missing or broken → warning, compiled data stays); at runtime `baked.ron` is
  loaded as an asset and applied on top whenever it loads or changes. After a rebuild both are identical.
- **Manifest** (`shader_codegen_rust::manifest`: `ManifestTexture`, `ManifestMaterial`, `manifest_ron`,
  `parse_manifest`, `MANIFEST_VERSION = 1`): first line the autogen marker as a `//` comment (so E008 protection,
  orphan cleanup and "no rewrite when unchanged" apply as to any autogen file; with `header_date` a date line follows),
  then `BakedManifest(version: 1, textures: { id: (path, settings) }, materials: { id: (params: { "inp_x":
  Float(..) }, textures: { "inp_albedo" | "ft_n02" | "ft_common_tex_layer_n02": texture id }) }, instances: {})`.
  `params` holds uniform parameters only, as `manifest::ParamValue` (`Float`, `Vec2`, `Vec3`, `Vec4`, `Color`, `Bool`,
  `Int`; core `Value` has no such RON shape), with the same broadcast rule as `{Name}Params::default()`; texture
  parameters and fixed textures are both in `textures`, keyed by binding ident (`Ir::textures`, decision 0024).
  Settings at their default are omitted. `instances` is reserved for material instances (non-goal for now).
- **Which textures.** Every Texture asset whose `AssetResult` is ok — used or not — plus the internal textures
  (`shaders/internal/<file>`). Assets with an error (E011, E012, E013, E005, E009) are left out; W109 (file missing)
  stays in. A binding whose texture id is not in the table is written as `@internal/default`.
- **`BakedTextures` fields ≠ manifest textures** (final review ruling, design §3.1 "all of them"). The struct has a
  field for every Texture asset that parses and has a valid name — E011 / E012 / E013 included — plus the internal
  textures, so an asset error never changes the crate's public shape (copying `sprite.texture.ron` would otherwise put
  E011 on both copies and break `tex.textures.sprite` in the game's code). A Texture asset whose RON does not parse
  (E009) is left out because its content is unknown (ruling of the final review; its id would come from the file
  name); one whose name is no identifier (E005) because it cannot name a Rust field. At runtime `load()`
  points a field without a manifest entry at the `@internal/default` handle (with a warning); it is not counted in
  the progress. If a later `baked.ron` brings an entry for such a field (the error was fixed in the editor), the hot
  reload loads it and fills the field.
- **Failed materials.** A material that fails to build keeps its previous manifest entry, read back from the old
  `baked.ron` (only if it has the marker and parses), mirroring how its Rust module is kept (`entry_from_old`).
  Without an old entry (manifest deleted) an empty `(params: {}, textures: {})` is written, so `BakedMaterials` and
  the manifest list the same ids; a failed material without an old module is in neither.
- **Generated files.** `crates/shaders/src/baked/mod.rs` is static code from
  `crates/shader_codegen_rust/templates/baked_mod.rs` (`include_str!`, only `{{GENERATOR_VERSION}}` substituted);
  `baked/materials.rs` and `baked/textures.rs` come from `shader_codegen_rust::baked`. Each material module gets
  `pub const BAKED_ID` and `from_values(&MaterialValues, &dyn Fn(&str) -> Handle<Image>)` (missing / mistyped value →
  `Params::default()` field + warning; missing texture key → `@internal/default` + warning — never a compiled texture
  id, so swapping a texture keeps the Rust unchanged). `crates/shaders/Cargo.toml` adds Bevy features `bevy_log`
  (`warn!` needs it; it is optional under `default-features = false`), `png`, `ktx2`, `zstd_rust`, and the deps
  `serde = "1"` (derive) and `ron = "0.12"` (Bevy 0.20's own version). The generated crate has a test
  `compiled_manifest_parses` (the snapshot parses and holds every compiled material; every manifest texture has a
  `BakedTextures` field — not the reverse, see above).
- **Public API** (design §3.1): `ShadersPlugin` = the material plugins + `BakedPlugin`. `BakedMaterials` (resource,
  created in `BakedPlugin::finish` with a reserved handle per material, so it exists from app start, before `load()`)
  and `BakedTextures` (resource, default handles until `load()`), nested per folder: `baked.env.rock_wet`,
  `tex.textures.rock_albedo`, `tex.internal.default`; the group types live in `baked::materials::dir_*` /
  `baked::textures::*` modules (unique by construction; games never name them). `BakedMaterialsLoader`
  (`SystemParam`: `ResMut<BakedState>`, `Res<AssetServer>`, `ResMut<BakedTextures>`) with `load()` (idempotent),
  `progress()`, `is_ready()`. Messages `BakedMaterialsProgress { loaded, failed, total }` and `BakedMaterialsReady`.
  Hidden but `pub` (`#[doc(hidden)]`): `BakedPlugin` (headless tests; without `DefaultPlugins` the image loader must
  be registered by hand, `bevy_render` normally does it), `BakedState` (the design's `BakedShaderAssets`, `pub` only
  because the `SystemParam` derive exposes it), `load_from_str` (test hook), `BakedTextures::get`,
  `BakedMaterials::unresolved` (test hook).
- **`new` / `set_*` are `pub(crate)`** (with `#[allow(dead_code)]`: nothing in the crate calls them), `from_values` /
  `reserve` / `IDS` / `BakedTextures::set` too. `Ext`, the `*Material` aliases and `*Params` stay `pub` (they are the
  field types). Games no longer construct or animate materials.
- **Field names.** The plain folder / asset name. Unlike decision 0020's static prefixes, these are game-facing field
  names, so a prefix is added only when needed: a Rust keyword (strict or reserved, 2024 edition) gets `dir_` (folder),
  `mat_` (material) or the new `tex_` (texture); a clash inside one struct (a folder and an asset of the same name, or
  a prefixed name meeting a plain one) prefixes every unprefixed member of that struct, repeated until unique
  (`env.dir_water.x` + `env.mat_water`, `kw.dir_loop.mat_type`). `internal` is reserved at the root of
  `BakedTextures` for the internal textures: a project folder `internal/` becomes `dir_internal`, a root texture
  `internal` `tex_internal`.
- **Runtime flow** (design §3.3), all in the template:
  1. `BakedPlugin::build` registers `Assets<M>` for every material only if missing (a second `init_asset` would
     replace the storage), the manifest asset + `ManifestLoader`, both messages, `BakedState`, `BakedTextures`, and
     the `PreUpdate` chain `insert_materials → apply_manifest → track_progress → apply_samplers`; `finish` inserts
     `BakedMaterials::reserve(world)`.
  2. `load()` parses the compiled manifest, starts every texture with `load_builder().with_settings(..)`
     (`is_srgb`, `expand_grayscale`, `sampler: None → ImageSampler::Default`, `Some → Descriptor`) and **fills
     `BakedTextures` at once**, warns about materials missing on either side, starts the typed load of `baked.ron`, and
     marks the materials for building. Controller ruling: the design ("filled by `load()`", "valid from the first
     frame", a sprite spawned in `Startup`) wins over the plan's "`load()` only sets a request flag"; a system chained
     after `load()` in `Startup` already sees the real image handles.
  3. The materials are inserted behind their reserved handles by the exclusive `insert_materials` on the next
     `PreUpdate` (it needs every `Assets<M>`, which a `SystemParam` cannot list generically). The design's "insert
     immediately instead of at a last stage" holds in the sense that matters: image handles exist before the images
     load and Bevy shows each material once its images arrive; nothing waits for "all textures loaded".
  4. `track_progress`: `Loaded` → loaded; `Failed` → error log naming id and path, counted failed, every material
     using it is rebuilt with the `@internal/default` handle (a failed default → `Handle::default()`, Bevy's white
     image), so nothing waits forever. `BakedMaterialsProgress` is written whenever it changes (first `0/0/total`),
     `BakedMaterialsReady` once when `loaded + failed == total` — with failures too. `total` = textures of the compiled
     manifest; textures loaded by a later hot reload are not reported after Ready.
- **Hot reload** (`apply_manifest`, `overlay`, `apply_samplers`). On `Added` / `Modified` / `LoadedWithDependencies`
  of the tracked handle the effective state is recomputed as **compiled snapshot overlaid by the current manifest** —
  a pure function, so a key that disappears returns to its compiled value, not to the previous reload's. Out of sync:
  a material or key unknown to the code is ignored, a missing key keeps the compiled value, a retyped value keeps the
  compiled one, a binding naming a texture in neither manifest gets `@internal/default`; each logs one warning per key
  for the whole run (`BakedState::warned`, never cleared), worded "(rebuild the game)" where a rebuild fixes it.
  Changed values / texture choice → `from_values` again and `insert` at the same handle. A Texture asset re-pointed
  to another file → new load with its settings, `BakedTextures` and the materials updated (entities holding the old
  image, e.g. a spawned `Sprite`, keep it); the new entry starts with `sampler_pending`, because if the file is loaded
  already Bevy hands back that image and ignores the new settings — its sampler is then set like a sampler change. Sampler changed → `Assets<Image>::get_mut(..).sampler`, applied once the
  image is in `Assets` (`sampler_pending`), then every material using it is re-inserted, because Bevy only rebuilds
  bind groups when the material changes (verified in the sample run). `is_srgb` / `expand_grayscale` changed → no
  reload, one warning (Bevy keeps the first load's settings). A texture created in the editor and referenced by a
  material is loaded by path but has no `BakedTextures` field until a rebuild; a texture dropped from the effective
  state sets its field (if any) back to the default texture. A manifest that fails to load (bad RON,
  other `version`) → warning, nothing changes. Watching `baked.ron` needs Bevy's `file_watcher`, which the game
  enables itself (the editor never edits its root `Cargo.toml`).
- **Rebuild needed** only for structure: a parameter or fixed-texture binding added / removed / retyped, a material
  added / removed / renamed or its kind / blend settings changed, new `BakedTextures` fields, loader-only settings.
- **Manifest loader extension.** `ManifestLoader::extensions()` is `["baked.ron"]`, but Bevy's full extension of
  `baked.ron` is `ron` (the text after the first `.` of the file name), so the loader never matches by extension. All
  our loads are typed (`server.load::<BakedManifest>` via the handle type), so they work. On a failed reload Bevy's
  `reload` also tries an untyped load and logs an extra "could not find an asset loader" error next to ours (noise on
  failures only). Latent hazard: a game that registers its own loader for `ron` could get that loader on such an
  untyped load. Renaming the manifest to `*.baked.ron` would remove both; not done now.
- **`load_from_str`** (test hook) uses the given text as snapshot *and* runtime manifest: it does not load
  `baked.ron`, which would otherwise overlay the test's input.

**Consequences.**
- Game code shrinks to `loader.load()` + handles from `BakedMaterials` / `BakedTextures` (the sample game uses
  nothing else). Using `AssetServer::load` on a file that has a Texture asset can win Bevy's "first load wins" race
  and drop the asset's settings — the README says to use `BakedTextures`.
- If `baked.ron` is blocked by a foreign file (E008), the crate's `include_str!` compiles that file; the generated
  `compiled_manifest_parses` test would catch a mismatch, the build would not.
- A texture that failed is not retried until its path changes in the manifest. Its `BakedTextures` field keeps the
  failed handle (materials switch to the default texture, a `Sprite` built from the field shows nothing).
- Progress / Ready cover only the initial load; a hot reload before Ready that adds a texture changes the counts.
- The sample game's `SAMPLE_SCREENSHOT` mode is wall-clock based (`Time<Real>`, shots 5 s and 25 s after the first
  frame) and keeps updating unfocused / without vsync, so a parameter and a sampler hot reload can be checked between
  the two shots.
