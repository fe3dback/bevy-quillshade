# 0024. Texture assets, fixed textures (also inside functions), internal textures

**Context.** Design `docs/superpowers/specs/2026-10-26-baked-materials-design.md` §1–§2 (cited as design §N). The
texture `default` paths stored in `param::TextureSample2D` / `param::TextureObject` were never used by codegen: games
loaded every image by hand. Textures must become editor assets (a proxy to a game image plus Bevy's
`ImageLoaderSettings`), nodes must reference them, fixed (non-parameter) textures must work in materials and inside
functions like in UE, and a built-in default texture must exist for nodes without one. Constraints: no migration and
no `format_version` bump; images are never copied or duplicated; only `.ktx2` and `.png` (Bevy 0.20 cannot read
`.ktx` v1); core crates stay Bevy-free; the writer must keep its E008 guarantee for binary files too.

**Decision.**
- **Asset kind.** `AssetType::Texture`, suffix `texture` (`shaders/**/*.texture.ron`), model
  `shader_graph_core::texture::TextureAsset` with its own `format_version` probe (`TEXTURE_FORMAT_VERSION = 1`), kept
  in `AssetSet::textures` (like curves and constant tables, decisions 0015 / 0017). Not a graph; generates no WESL or
  Rust file of its own (`regen::wesl_file` / `module_of` are `None`); its only output is its entry in the baked
  manifest (decision 0025). There is no `name` field: the name is the file name.
- **RON shape** (struct names, 2-space indent, every field at its default omitted at all three levels):
  `TextureAsset(format_version: 1, path: "textures/env/rock_albedo.ktx2", settings: TextureSettings(is_srgb: false,
  sampler: Some(SamplerSettings(address_mode_u: Repeat, mag_filter: Linear, ...))))`.
  `TextureSettings { is_srgb (true), expand_grayscale (true), sampler: Option<SamplerSettings> }`; `sampler: None` is
  Bevy's `ImageSampler::Default` (the app's default sampler), `Some(..)` is an `ImageSamplerDescriptor`.
  `SamplerSettings` mirrors it: address modes u/v/w (`ClampToEdge`, `Repeat`, `MirrorRepeat`, `ClampToBorder`),
  mag / min / mipmap filter (`Nearest`, `Linear`), `lod_min_clamp` (0), `lod_max_clamp` (32), `anisotropy_clamp` (1)
  — the defaults are Bevy 0.20's descriptor defaults, so a `Some` sampler with no fields (`Some(SamplerSettings())`)
  still means "descriptor with Nearest filters", not the default sampler. `texture_format` is not mirrored. `path` is
  relative to the game's `assets/`.
- **One game file — one Texture asset.** Two or more assets with the same `path` get **E011 at every one of them**
  (the message lists the others), however the clash came about (creation or a later path edit). All of them are
  excluded from the manifest, so none silently wins Bevy's "first load wins". The comparison is an exact string
  match; the E012 rules below keep paths canonical (`./a.png`, `a//b.png` are invalid), so it cannot be dodged.
  Case-insensitive file systems are out of scope. The editor refuses to create a second asset for a file
  (`ops::create_texture`, Create Texture Asset, drop) or to re-point an asset at a claimed file (decision 0026).
- **E012** invalid path: empty, `\`, `#` (Bevy reads it as the asset label separator), absolute (leading `/` or a
  drive letter), empty / `.` / `..` components, or an extension other than `ktx2` / `png` (case-insensitive: `a.KTX2`
  passes, but Bevy matches loader extensions case-sensitively, so such a file may not load — use lowercase). The other texture checks are skipped after E012.
- **E013** invalid sampler settings (`SamplerSettings::check`): `anisotropy_clamp` 0, `anisotropy_clamp` above 1
  while any of mag / min / mipmap filter is not `Linear`, LOD clamps non-finite, negative or `lod_min_clamp >
  lod_max_clamp`. These are wgpu's `create_sampler` rules, except that non-finite clamps are refused although
  wgpu-core 30 accepts them (its checks are `<` comparisons, which NaN passes) — deliberately a little stricter.
  Anisotropy above 16 is allowed (wgpu clamps it). Without E013 such a sampler would only fail in the running game.
  An asset with E013 is left out of the manifest like E011 / E012.
- **W109** the image does not exist under `assets/`: a warning only (the file may be added later); the asset stays in
  the manifest and the game counts it as failed and uses the default texture (decision 0025). A bad asset path is
  E005, an unreadable file E009, as for other kinds.
- **References.** `ParamKind::Texture` stores the asset id as `Value::Str` (`textures/env/rock`,
  `@internal/default`, or `""` = none), like curves. `registry::texture_refs` finds them; `DepGraph` makes the graph
  depend on the asset (not on `@internal/*`); `ops::rename` rewrites references in every graph file, `ops::delete`
  clears them (the nodes then fall back to the default texture, W110); the editor applies the same to open documents.
  Missing asset → E006 (as for curves), through `Env::textures: &dyn TextureLookup` (filled by `regen::texture_ids`:
  every Texture asset, also those with their own errors, plus the internal ids).
- **Nodes.** `stdlib::TextureSample` gains a `texture` property; its `tex` pin becomes optional and overrides the
  property when connected (UE). New `stdlib::TextureObject`: `texture` property → `Texture2D` output. The `default`
  of `param::TextureSample2D` / `param::TextureObject` becomes `ParamKind::Texture`; `param::TextureSample2D` gains
  TextureSample's `mip_mode` param and `mip` pin (shared helpers), so the two convert into each other losslessly.
- **No texture → `@internal/default` + W110**, not E004, for nodes with a texture property or parameter default.
  Nodes without one (`FlipBook`, `NormalFromHeightmap`, `TextureSize`, `TexelSize`) keep a required `tex` pin and E004.
- **No new pin type.** `Texture2D` stays the pin that carries a texture + sampler; the "texture data type" is the
  Texture asset plus `ParamKind::Texture`.
- **Old files.** No migration, `FORMAT_VERSION` stays 1. `shader_project::assets::load` runs
  `shader_stdlib::nodes::texture::check_texture_refs`: a `ParamKind::Texture` value containing `.` (an old image path
  such as `textures/rock_albedo.png`; asset ids never contain `.`) fails the load with a message naming the node and
  saying that a texture is now a Texture asset reference; regen reports it as the usual E009 "cannot load".
- **Fixed textures in materials** (design §1.3). A fixed texture (a `texture` property, or the default texture) is a
  material binding **per node**, not per Texture asset, named `ft_{node id}` (`registry::fixed_texture_ident`). The
  texture id never appears in WESL or Rust, so swapping a node's texture changes only the manifest and hot-reloads.
  The IR owns the full binding list: `Ir::textures: Vec<TextureBinding { ident, source, texture, binding }>`,
  `TextureSource::Param(name)` (the bare parameter name; `ident` is `inp_<name>`) or `Fixed { node, origin }`; texture
  parameters first (by name, decision 0027), then fixed textures sorted by `(origin, node)`; texture binding
  `FIRST_TEXTURE_BINDING + 2 i` (101, 103, …), sampler + 1. WESL `write_bindings`, `binding_of` and the Rust
  `#[texture]` / `#[sampler]` fields all read it. One entry per node however often it is used, collected from every
  stage. The resolved id is stored (`MaterialParam::default` too), never `""`.
- **Generated Rust.** Fixed fields follow the texture parameters; `new` (crate-internal, decision 0025) fills them with
  `Handle::default()` (Bevy's 1×1 white image). A material without any parameter derives `Default` instead of
  getting a `Default` impl, because clippy's `new_without_default` fired on a fixed-texture-only material.
- **Fixed textures inside functions.** A function's fixed texture becomes a **hidden trailing texture + sampler
  parameter**, exactly like builtins (decision 0005): `FunctionSig::textures: Vec<HiddenTexture { origin, node,
  texture }>`, after the `b_*` parameters, sorted by `(origin, node)`, transitive through nested calls (functions are
  built dependencies-first). The caller passes its own operand of the same name: a hidden parameter of its own in a
  function, a module-level binding in a material. A material gets **one binding per origin node, not per call**
  (`EmitSink::fixed_textures` is keyed by `(origin, node)`), so a function called twice, or through another
  function, still costs one binding.
- **Identifier.** `ft_{function path with / → _}_{node}` (`ft_common_tex_layer_n02`). `ft_` is none of the user
  prefixes of decision 0020 nor a generator name (`n_`, `b_`, `sg_`, …). Node ids are `[a-z0-9]+`, so a material's own
  `ft_{node}` (no `_` after the prefix) never equals a function-origin one. Two origins collide only when their paths
  differ by `/` vs `_` (`a/b_c` and `a_b/c`) and the node ids match: `build_ir` checks all texture and `_sampler`
  names of an asset and reports that as **E006** ("rename one of the functions"), so no duplicate binding is ever
  generated. A `__` separator would be collision-free (`check_ident` forbids `__` in names) but rustc's
  `non_snake_case` lint rejects `__` in the generated struct field, which fails the game's clippy; a
  `dir_…_fun_…` spelling is not collision-free either.
- **Internal textures** (design §2). `shader_stdlib::internal::INTERNAL_TEXTURES`: `InternalTexture { id, file,
  bytes, settings }`, ids under `@internal/` (`INTERNAL_PREFIX`); only `DEFAULT_TEXTURE = "@internal/default"` now — a
  64×64 grey / white checkerboard of 8×8 squares (`crates/shader_stdlib/textures/default.png`, written by a test-side
  encoder and checked against it), sRGB, Repeat + Linear sampler. Every regeneration (any scope) writes them to
  `assets/shaders/internal/<file>`; in the manifest their path is `shaders/internal/<file>`.
- **Ownership of `internal/`.** Binary files cannot carry the text marker, so `assets/shaders/internal/` is owned
  through a marker file `.autogen` holding the usual marker line. If the directory is missing or empty it is claimed
  silently (a fresh project; nothing foreign is at risk). A non-empty directory without a valid `.autogen` is a
  project-level **E008**: nothing there is written or deleted, the rest regenerates. Only a missing path counts as
  missing: a regular file at `internal/` or an unreadable directory is E008 too (final review fix), and so is any
  generated path that exists but cannot be read (permissions, a directory in the way) — never replaced. In an owned `internal/`, a full
  regeneration deletes files nothing produces. The writer works on bytes (`WriteBatch.writes: Vec<(PathBuf,
  Vec<u8>)>`), compares bytes (unchanged files are not rewritten), and a foreign *binary* file at a generated path is
  now E008 too (it used to be overwritten because `read_to_string` failed).

**Consequences.**
- A fifth asset kind. Settings edits change only the manifest; picking another texture for a node or a function's
  node changes neither WESL nor Rust (tested by `swap_fixed_texture_keeps_codegen`).
- Function signatures depend on the fixed textures they (transitively) contain, like they do on builtins.
- Sample fixtures: `textures/water_normal` uses Repeat **and Linear** filters, because a `Some` sampler defaults to
  Nearest and the water would look pixelated.
- Known gaps: a function is emitted without knowing its caller's stage, so a texture it samples with MipValueMode
  None / Bias (`textureSample` / `textureSampleBias`) breaks the vertex shader when the function feeds World Position
  Offset (a material's own texture nodes emit `textureSampleLevel` in the vertex stage); live validation does not
  diagnose it. This predates fixed textures (texture function inputs behave the same). E012
  accepts uppercase extensions although Bevy picks loaders by a case-sensitive extension match (`#` in a path is E012
  since the final review).
