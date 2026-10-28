# 0037. Gradient collections (`*.gradients.ron`), the Sample Gradient node, the gradients editor

**Context.** Design `docs/superpowers/specs/2026-10-28-gradient-collections-design.md` (cited as design §N). The user
wants to color a landscape by height: height becomes `t`, `t` picks a color from a hand-made gradient, and the color
is blended into the textures. UE does this with Curve Linear Color + Curve Atlas (a texture whose rows are color
curves). Curves (decision 0015) are scalar and are generated as WESL constant arrays. Constraints:
- a color edit must never regenerate or recompile a shader;
- reordering or clearing gradients must never rewrite graph files;
- no hand-built look-alike of the Feathers color picker (decision 0034);
- core crates stay Bevy-free, and codegen stays byte-deterministic on every CI OS.

**Decision.**
- **A baked texture, not a constant array.** A collection is baked into one 128×128 PNG: one row per slot, 128
  samples of `t` per row. A material samples it like a texture. The reasons:
  - Editing colors changes only the PNG. The game hot-reloads an image and recompiles no shader, which follows the
    philosophy of fixed textures (decision 0024).
  - A sample costs one `textureSampleLevel`, however many stops the gradient has. A constant array would need a
    per-pixel loop over up to 32 stops, which is expensive on a full-screen landscape.
  - One binding serves all 128 gradients of a collection.

  Accepted costs: see Limitations below.
- **Asset.**
  - Kind: `AssetType::Gradients`, suffix `gradients` (`shaders/**/*.gradients.ron`). User-facing name "Gradient
    Collection"; icon-rs `APPEARANCE` (decision 0013).
  - Model: `shader_graph_core::gradient::GradientCollection { format_version, slots }`. It has its own probe,
    `GRADIENTS_FORMAT_VERSION = 1`, and is kept in `AssetSet::gradients`. It is not a graph: no WESL or Rust file
    (`regen::wesl_file` / `module_of` are `None`).
  - No `name` field: the name is the file name, as for a Texture asset.
  - RON shape: struct names, 2-space indent, `depth_limit(2)`. Each `Slot(...)` is on one line with its stops inline.
    Defaults are omitted: an empty `name`, empty `stops`, `mid: 0.5`.

  ```ron
  GradientCollection(
    format_version: 1,
    slots: [
      Slot(id: 1007, name: "Terrain height", stops: [Stop(t: 0.0, color: (0.027, 0.147, 0.027, 1.0)), Stop(t: 0.8, color: (0.807, 0.807, 0.807, 1.0), mid: 0.3), ...]),
      Slot(id: 1014),
      ...
  ```
- **Slots.**
  - There are exactly `SLOTS` = 128 slots: `Slot { id, name, stops }`.
  - `id` is a `u32` in `1..=i32::MAX`. `0` means "none", and the range fits the node's `Value::Int`. Ids are unique
    within the collection and never change.
  - A new collection gets random ids (`GradientCollection::random()`): splitmix64 seeded from the std hasher's random
    keys, with no new dependency. `new(next_id)` takes any id sequence; the sample fixture uses it, so `make_sample`
    is reproducible.
  - Nodes store the slot id, so slots can be reordered freely. `name` is editor-only.
  - A slot holds 0..=32 stops. Zero stops make an empty slot.
- **Stops and loading.** `Stop { t, color, mid }`: `t` in `0..1`, `color` linear RGBA in `0..1` (the project's color
  convention, decision 0036).

  Loading runs four steps:
  1. the version probe;
  2. parsing;
  3. `normalize`: clamps `t`, `mid` and the channels, drops a stop with a non-finite `t` (a NaN `t` is not a load
     error), sets a NaN channel to 0 and a NaN `mid` to 0.5, and stable-sorts the stops by `t`;
  4. the structural check, which therefore counts the 32-stop limit after normalization.

  Structural errors fail the load with E009 `cannot load: …` and a precise message:
  - `gradient collection has N slots, expected 128`;
  - `slot P: id X must be in 1..=2147483647`;
  - `slot P: duplicate id X`;
  - `slot P: N stops, at most 32 allowed`.

  A bad asset path is E005 (`validate_path`). Loading never panics.
- **Midpoint model** (Photoshop / Krita, the user's choice).
  - Each segment has one `mid` in `MID_MIN..=MID_MAX` (`0.01..=0.99`, default 0.5). It is the point, relative to the
    segment, where the two colors mix 50/50.
  - `mid` is stored on the segment's **left stop** and is ignored on the last stop. It travels with its stop when a
    drag re-sorts the stops; it is not reset.
  - Between stops `a` and `b`, with `x = (t - a.t) / (b.t - a.t)`, the remapped factor is
    `f = x < m ? 0.5·x/m : 0.5 + 0.5·(x - m)/(1 - m)`.
  - There is no hardness value. A hard edge is two stops at the same `t`, and at exactly that `t` the later stop wins.
    A span below `1e-6` counts as such a step.
  - Before the first stop the color is the first stop's; at or after the last stop it is the last stop's.
- **Color math** (`Slot::sample`, the reference for baking and every editor preview):
  - RGB is interpolated in **OKLab**: the two linear colors are converted to OKLab, lerped by `f`, converted back,
    and out-of-gamut results are clamped to `0..1`. Alpha is a plain lerp by `f`.
  - The math runs in `f64` behind the `f32` API. This keeps libm `powf` / `cbrt` differences between Linux, Windows
    and macOS far from an 8-bit rounding boundary, so the PNG is byte-identical on every OS.
  - At `f <= 0` and `f >= 1` the stored colors are returned exactly. Equal RGB on both ends skips OKLab, because the
    OKLab round trip is not exact in floating point.
  - A NaN `t` samples like a `t` before the first stop (its color); `±inf` gives the last / first stop's color.
- **Baking** (`GradientCollection::bake_rgba8`, `shader_project::gradient_png`):
  - Pixel `x` of row `r` is `sample(slot r, x / 127)`, so pixel 0 is `t = 0` and pixel 127 is `t = 1`.
  - RGB is **sRGB-encoded** (`linear_to_srgb`, then `round(c·255)`). The image is loaded as sRGB, so a sample returns
    linear values again. Alpha is `round(a·255)`, without a transfer function.
  - An empty slot is opaque white.
  - The PNG is 128×128 RGBA8 with no sRGB / gAMA chunk. It is encoded with `png` 0.18, `FdeflateUltraFast` and
    `Filter::Adaptive`. flate2 is avoided because its output depends on which backend feature unification picks.
  - `png` is a dependency of `shader_project` only.
- **Sampling.**
  - The stdlib helper `texture.wesl::sg_sample_gradient(tex, samp, t, v)` returns
    `textureSampleLevel(tex, samp, vec2(saturate(t) * 0.9921875 + 0.00390625, v), 0.0)`. That is `u = t·127/128 +
    0.5/128`, the texel centres, so `t = 0` and `t = 1` hit the first and last texel exactly.
  - `v = (row + 0.5) / 128` is a typed literal (`wesl_scalar`, decision 0007), exact in `f32`.
  - Level 0 is explicit: the texture has no mipmaps, and the call is also valid in the vertex stage (World Position
    Offset compiles in the forward, no-UV and deferred variants).
- **Row = slot position.** `build_ir` resolves the slot id to its current position. It reads only the position, never
  whether the slot has stops.
  - `DepGraph` has a user → collection edge for every reference. Reordering slots therefore regenerates the dependents,
    and only the row literal in their WESL changes. No graph file and no Rust file is rewritten.
  - A color, stop, midpoint or name edit, emptying a slot or filling it changes **only the PNG**. WESL, Rust, the
    manifest, the bindings and the layout hash (decision 0029) stay byte-identical. Pipeline tests check this
    (`gradient_content_edits_touch_only_png`, `gradient_reorder_changes_only_row`).
- **Bindings.**
  - There is **one binding per collection per material**, not one per node (unlike fixed textures, decision 0024).
    `TextureSource::Gradient(collection)` entries in `Ir::textures` come after texture parameters and fixed textures,
    sorted by collection id.
  - The identifier is `gc_<collection path, / → _>`, plus `gc_…_sampler`. `gc_` is neither a decision 0020 user prefix
    nor a generator name (`n_`, `b_`, `sg_`, `ft_`).
  - The WESL binding has no owning node in the line → node map.
  - The Rust field is ``/// Gradient collection `<id>`; …`` followed by
    `#[texture(n)] #[sampler(n+1)] pub gc_*: Handle<Image>`. `new` fills it with `Handle::default()`, and
    `from_values` uses `tex(v.texture(BAKED_ID, "gc_…"))`. The layout guard and the manifest's `bindings` follow from
    `Ir::textures` as for other textures.
- **Inside functions.**
  - A function that uses a collection gets one hidden trailing texture + sampler parameter per collection, named like
    the binding (`gc_*`). They come after `inp_*`, `b_*` and the hidden fixed textures `ft_*`.
  - `FunctionSig::gradients` is sorted and transitive through nested calls.
  - A caller passes its own binding, or its own hidden parameter of the same name. A material therefore has a single
    binding for a collection, however many nodes and functions use it.
  - **Collision rule.** Gradient identifiers share one map with the fixed-texture identifiers. A clash is E006 ("…
    get the same identifier `gc_a_b_c`; rename one of them"). Possible cases: `a/b_c` vs `a_b/c`, or `x` vs
    `x_sampler`.
- **Sample Gradient node** (`stdlib::SampleGradient`, category Color):
  - parameters: `collection` (`ParamKind::GradientCollection`, `Value::Str`, `""` = none) and `gradient`
    (`ParamKind::GradientSlot`, `Value::Int` slot id, `0` = none);
  - input `t` (Float, default 0, range `0..1`);
  - outputs `rgb` (Float3), `a` (Float) and `rgba` (Float4), all linear. The pins are named and ordered like
    `TextureSample`'s outputs (rgb, a, rgba), not like the mockup's RGBA / RGB / A listing (controller ruling), so
    `rgb` is the first output.

  Diagnostics:

  | Case | Diagnostic | Output |
  | --- | --- | --- |
  | no collection or no gradient selected | W111 "no gradient selected; outputs white" | `vec4<f32>(1.0)`; no binding, no stdlib import |
  | the selected slot is empty | W111 "gradient is empty; outputs white" | still samples its row (baked white): code never depends on slot contents (the empty-slot rule) |
  | the collection is missing, does not load (E009) or has a bad path (E005) | E006 "gradient collection `x` does not exist or cannot be loaded" | — |
  | the slot id is not in the collection (a negative id included) | E006 "gradient #id is not in collection `x`" | — |
  | two collections or a collection and a fixed texture get the same identifier | E006 | — |
- **Owned `gradients/` directory.**
  - Location: `assets/shaders/gradients/<collection path>.png`.
  - Ownership: like `internal/` (decision 0024), the directory is owned through the `.autogen` marker file
    (`owned_binary_dir`; a PNG cannot carry the text marker). The E008 and orphan rules are shared by both
    directories.
  - The marker is written, and a foreign directory reported (one project-level E008, nothing written or deleted
    there), **only while the project has at least one collection**. A project without collections gets no new
    files. With none left, a full regen deletes the last PNG and the marker as orphans. An unrelated
    `assets/shaders/gradients/` of the game is no error.
  - Every regeneration, of any scope, writes the PNG of each collection without errors. Orphans are deleted only by a
    full regen.
  - A collection with errors keeps its old PNG but leaves the manifest; its users fail with E006 anyway.
  - With a foreign directory, collections are left out of the manifest, and their bindings fall back to
    `@internal/default`. The `BakedTextures` fields stay, so the crate's shape does not depend on the directory.
- **Manifest and `BakedTextures`** (decisions 0025, 0030).
  - Each loadable collection is a texture entry: id `@gradients/<collection path>`, path
    `shaders/gradients/<path>.png`, settings `is_srgb: true`, `Linear` mag / min, `ClampToEdge`, no mipmaps. The id
    never collides with a Texture asset id, because asset paths never start with `@`.
  - The material entry maps `gc_*` → that id.
  - `BakedTextures` has a reserved root group `gradients`, like `internal`, nested by path:
    `tex.gradients.common.terrain`. A project folder `gradients/` becomes `dir_gradients`, and a root Texture asset
    `gradients` becomes `tex_gradients`. Fields exist for every collection that parses and has a valid path.
- **References** (`registry::gradient_refs`, like `curve_refs` / `texture_refs`):
  - `ops::rename` rewrites the collection id in every graph file and in open documents (`follow_ref_rewrite`). Slot
    ids are kept.
  - `ops::delete` clears the collection to `""` and every `GradientSlot` param of the same node to `0`, giving W111.
    `clear_gradient_refs` is generic over the param kinds.
  - In the editor, picking another collection (the select, or a drop onto the node) resets the slot to `0` in the same
    undo step. The node's own collection (e.g. dropped onto it again) keeps the slot.
- **Editor kind** (decisions 0031, 0032).
  - Types: `ViewKind::Gradients`, `Tab::Gradients(GradientsDoc)`, `editors/gradients.rs` (`GradientsEditor`, right
    sidebar title "Gradients", watermark `gradients`) and the input context `GradientsCtx`.
  - Undo: `GradientsDoc` keeps a `History<GradientCollection>`, a snapshot of the whole collection. Drags and name
    typing merge by key. The selected slot and the stop / midpoint selection are editor-only (not saved, not
    undoable). Undo / redo selects the slot that changed.
  - Editor content (`ui/gradient_view.rs`): the selected slot's bar over a checkerboard, stop handles above it,
    midpoint diamonds below it (hidden on a hard step), a `t` ruler, and the selected stop's `T` + color swatch or the
    selected diamond's `Midpoint`.
  - Editor topbar: Duplicate (into the next empty slot), Clear, Move up / Move down. Editor statusbar:
    `N / 128 used · 128×128 RGBA8`.
  - Right sidebar: the slot's Name field (a placeholder overlay, since Bevy 0.20 text inputs have none), then all 128
    slots as one persistent `FeathersListView` of `FeathersListRow`s. The list is spawned once, so its scroll offset
    survives clicks. The hotkeys follow.
  - Keys: while the list has the focus its keys (arrows, Home / End, Space / Enter: `LIST_KEYS`) win over hotkeys, as
    for `PRESS_KEYS` (decision 0035).
  - Gestures: a stop drag starts only after `DRAG_THRESHOLD` = 3 logical px (Bevy has none), and a stop drag is one
    undo step. A click on the bar or the handle strip adds a stop with the color sampled there; past 32 stops the
    click is refused with a log message. A press there that moves 3 px or more (`CanvasDragged`) adds no stop:
    bevy_picking still sends `PointerClick` when such a drag is released over the canvas.
  - Every gradient strip (slot lists, the bar, the node preview) rasterizes the **baked row** (`ui::gradient_strip`),
    filtered linearly in linear space, so it shows what the game samples.
- **Stop color picker** (Feathers, decision 0034).
  - The mechanism: a single hidden `FeathersColorInput` (`StopColorInput`), a child of the canvas, is positioned
    absolutely over the selected stop's handle and the bar. `open_stop_color` opens it programmatically with
    `MenuEvent { action: Open(NavAction::First) }`.
  - Placement: Feathers' `Popover` puts the picker below the bar, right-aligned to the stop (Bottom End), or above the
    handle as a fallback.
  - Visibility: the hidden root keeps its button out of picking and Tab. The popup carries its own
    `Visibility::Visible`.
  - Rejected: a `FeathersColorInput` per handle, because its `ActivateOnPress` menu button fights the drag and brings
    a button look. An own popup built from Feathers color widgets was not needed.
  - Click vs drag: the 3 px threshold above separates them. `PointerClick` arrives before `PointerDragEnd`, so the
    click handler sees whether the stop moved. A click on the handle, or on the stop row's swatch, opens the picker.
  - Color in: the stored linear color as `Color::linear_rgba`. Color out: `ValueChange<Color>` → `to_linear()` →
    `SetStopColor(i, rgba, Some("color"))`. The picker's own `ColorInputValue` is re-inserted with the exact `Color` it
    sent, so the hue of a grey survives.
  - Undo: edits merge until Feathers' `is_final` (end of a drag, a hex entry, a recent color) or until the popup
    closes. That gives one undo step per drag.
  - Menu rules: `dialog::menu_barrier` (z 94, under the popup's z 100) is up while the popup is open, so an outside
    click only closes it. Only one picker opens at a time.
  - Esc drops the focus instead of focusing the hidden button.
  - The picker closes when its stop stops being the selected stop of an active gradients tab.
- **Graph integration.**
  - Properties: a select of the project's collections (a missing one stays visible as "`<path>` (missing)"), then a
    `FeathersListView` slot picker: "None" (white), then every non-empty slot as a strip with its position and name.
    The picked slot is listed even when it is empty.
  - Node preview: a full-width strip above the pins (`ui/gradient_preview.rs`, the 4th entry of
    `node_preview::kinds()`). Its key is the picked slot's baked row, from an open tab's unsaved data first. Without a
    pick it shows "n/a".
  - Navigation: "Open gradients" and a double click on the node open the collection.
  - Drag & drop: a collection dragged from the Project tree adds a Sample Gradient with no gradient picked, or sets
    the collection of the Sample Gradient it is dropped on.
- **Node title deviation** (design §6 asked for the gradient's name or `#<position>`). The title is
  "`<collection name>` (Sample Gradient)", the existing Sample Curve / Global* / texture-node rule. The canvas rebuilds
  only on graph revisions, not on edits of a collection, so a gradient name in the title would go stale. The gradient's
  name shows in the properties slot list.
- **Sample game.** The fixture `common/terrain` (`fixtures::gradients()`, ids from a fixed sequence, colors given as
  sRGB hex and stored linear) has slot 0 "Terrain height" and slot 1 "Water depth". `env/terrain_layers` multiplies
  its grass base color by slot 0 sampled at `saturate(world_position.y * 0.25 + 0.5)`.

**Consequences.**
- Another non-graph asset kind (after curves, constant tables and Texture assets) exists: in the browser
  (`New Gradient Collection...`), in `regen` / `check` / `diff`, in the baked manifest and in `BakedTextures`.
- Running game, no rebuild needed: color, stop, midpoint and slot edits hot-reload as an image ("Reloaded
  shaders/gradients/common/terrain.png", no shader reload). So does a slot reorder (a WESL row literal).
- Rebuild needed: the first use of a collection in a material (a new binding: the layout guard of decision 0029
  stops the material until the rebuild). A new collection's `BakedTextures` field also appears only after a rebuild.
- Limitations:
  - resolution of 1/128 in `t`: a hard edge smears over about one texel, and the editor previews show it the same
    way;
  - 8-bit channels: slight banding is possible in dark, smooth gradients;
  - static selection: the slot is fixed at generation time; there is no material parameter or per-instance index;
  - no HDR colors; interpolation is always OKLab; the size is always 128 × 128.
- Known rough edges:
  - for a stop near the left edge the picker extends over the left sidebar (Feathers' placement; it stays inside the
    window);
  - picking a slot in a node's properties refills the right sidebar, so a long slot list scrolls back to the top;
  - a drag that starts and ends on the empty bar still adds a stop;
  - a PNG blocked by E008 per file is still listed in the manifest of that run (the same ordering as an E008-blocked
    material file).
