# 0026. Game Assets tree, droppable game files, image / Texture tabs, editor refactor seams

**Context.** Design §4 (`docs/superpowers/specs/2026-10-26-baked-materials-design.md`). Texture assets (decision
0024) need editor support: browsing the game's images, creating Texture assets for them, dragging them into graphs,
a read-only image view, a Texture asset tab with a preview, thumbnails in texture nodes, and "Convert to Parameter".
The editor had grown per-kind `match` arms everywhere (a 4000-line `state.rs`, ~20 `Tab` matches, two copies of the
in-node preview logic, a sidebar that assumed one tree), so a refactoring step came first; it changed no behaviour.

**Decision.**
- **Refactor seams** (Phase A):
  - `state.rs` is split into `state/{mod, types, tabs, regen, watch, apply, graph_ops, asset_ops, curve_ops,
    consts_ops, texture_ops, tests}.rs` along its existing sections; the public API of `crate::state` is unchanged.
  - `doc_kind::TabDoc` (asset, view, title, icon, watermark label, dirty / read-only, revision, save, changed on disk,
    reload, keep mine, rename, config key, `override_into`), reached through `Tab::doc()` / `doc_mut()` (decision
    0014). Deviations from the plan: `rename` takes `&Project` (a text tab recomputes its file path), `kind_label`
    was added (the watermark), `keep_mine` replaced the per-kind KeepMine match, and `override_into(&mut AssetSet)`
    replaced the per-kind `overridden_set` match. `Tab::owner()` names the asset of a generated-file tab too.
    `revisions` still matches per kind on purpose (curve revisions also change on selection, which must not
    re-validate).
  - `ui::node_preview::NodePreview` — the generic in-node preview (not decision 0022's exclusive material preview):
    `type Key`, `type Draw: SystemParam` (each kind gets exactly the queries / resources it needs), `const SIZED`,
    `applies(def)`, `layout(def)`, `content(def)`, `key(&Editor, &Doc, &NodeId) -> Option<Key>` and
    `draw(Option<&Key>, PreviewTarget, &mut Draw::Item) -> bool`. The mechanism owns the shared "n/a" label (shown for
    a `None` key or a draw that returns `false`), recomputes keys only when `Editor` changed, on a new row or (for
    `SIZED` kinds) a resize, redraws only when the key or size changed, and never rebuilds the canvas. A kind is one
    line in `node_preview::kinds()`; the curve, global-constant and texture previews are its three kinds.
  - `ui::tree`: `TreeSource { id, rows, read_only }`, `TreeId { Project, GameAssets }`, `TreeItem { Folder, Asset,
    GameFile, Internal }`, the `TreeRow` component, collapse state per tree (`Editor::collapsed: BTreeMap<TreeId,
    BTreeSet<String>>`, `Action::ToggleFolder(TreeId, String)`), and pure row logic (`editable`, `row_label`,
    `click_actions`, `row_menu`, `drop_action`) that the `browser.rs` observers call and `state::tests` test.
  - `NodeOp::Convert(Conversion { kind, params, unique_name, pins })` replaces `NodeOp::ConvertTo(kind)` (decision
    0021). Empty `params` keeps every same-named param (the arithmetic converts); a non-empty list copies **only** the
    listed `(from, to)` params, the target's others keep their defaults. `unique_name = (param, base)` fills `param`
    with `base`, `base_1`, … unique among the asset's parameter nodes (resolved params, so a parameter keeping its
    default name counts); `pins` renames output pins so edges survive.
  - `canvas::window_to_graph` honours `UiScale` (needed for drop positions).
- **Sidebar.** The "Assets" panel is now **Project**; below it the read-only **Game Assets** panel (fixed 60 / 40
  split, both scroll). Game Assets (`GameAssetsTree`, rows from `shader_project::game_assets::scan_images`): files
  with a droppable extension (`png`, `ktx2`, any case) under the game's `assets/` and the folders holding them,
  folders ordered by path components; `assets/shaders/` (our `autogen/` and `internal/` output, decision 0024) and
  hidden names (`.…`) are skipped. No create / rename / move / delete; RMB on an image offers only **Create Texture
  Asset**, disabled with a tooltip naming the asset when the file is already claimed. The project watcher has a
  second, best-effort root `assets/` (a huge tree hitting the OS watch limit does not stop `shaders/` from being
  watched); events under `assets/shaders/` are dropped in the callback, so regeneration never triggers a refresh loop;
  changes under `assets/` only rescan the image list (and re-validate, for W109), not the whole project.
- **Internal folder.** The Project tree ends with a virtual read-only folder labelled `Internal` (key
  `tree::INTERNAL_FOLDER = "@internal"`, which `check_ident` can never produce for a real folder) holding
  `TreeItem::Internal("@internal/default")`: double click opens a read-only Texture tab, it can be dragged onto a
  graph, no menu, no rename / move / delete, nothing can be dropped onto it.
- **Droppable files.** `droppable::DroppableFile { matches, node_kind, param, param_kind, resolve }` (`param_kind` is an
  addition to the plan: a drop onto an existing node fills that node's first param of this kind, so the trait stays
  generic). `ImageFile` is the only implementation: `stdlib::TextureSample`, param `texture`, `ParamKind::Texture`;
  `resolve` returns the Texture asset claiming the file or creates one. `Dropped { GameFile(path), Texture(id) }`
  (`Dropped::from_row`) also accepts a Texture asset of the Project tree or an `Internal` row; anything else (folders,
  materials, functions, other files) is ignored. The canvas observer reacts to Bevy picking's `PointerDragDrop`
  bubbling to the canvas, finds the node with `GraphQuery::node_of`, and pushes `Action::Drop { item, pos, node }`:
  onto a node with a param of `param_kind()` → that param is set; otherwise (empty canvas or another node) → a new
  node at the drop point. Each is one undo step of the graph.
- **Auto-created Texture assets** (`game_assets::new_texture_id_for_file`, shared by the menu, the image tab and the
  drop): `shaders/textures/<folders under assets/>/<file stem>.texture.ron`, every component made snake_case by
  `texture::asset_name_from_file_stem`, **a leading `textures/` folder of the game path dropped** (controller
  ruling: the design's example `assets/textures/env/Rock-Albedo.ktx2` → `shaders/textures/env/rock_albedo` wins over
  its literal "same subfolders" rule, which would give `textures/textures/…`; the prefix is matched case-sensitively),
  `_2`, `_3`, … on a clash with any existing asset. Default settings.
- **Undo scope.** Creating a Texture asset (drop, menu, image tab) is not part of any document's history: Ctrl+Z after
  a drop reverts only the node / param edit, the `.texture.ron` stays (an unused Texture asset is harmless; no
  cross-file undo layer).
- **One-file claims in the editor.** `Editor::texture_owner` / `texture_for_file` treat a file as claimed when another
  asset points at it **on disk or in an open, unsaved Texture tab** (the union): a file an unsaved tab moved away from
  stays taken until that tab is saved. Path edits (`TextureAction::SetPath`, also undo / redo), Create Texture Asset
  and drops all use it, so the editor cannot write an E011 pair; a hand-edited one still gets E011.
- **Images from the game dir.** The editor's `AssetServer` serves the editor's own assets, so
  `game_image::GameImageCache::get(project, key, settings, images)` reads bytes and decodes them with
  `Image::from_buffer` (Bevy's own decoders and features — no format-specific code). The key is a path under
  `assets/` (checked with the E012 rules first, so nothing outside `assets/` is read) or an `@internal/…` id (decoded
  from the embedded bytes). Entries are cached per (key, settings), invalidated on access by a file stamp (mtime +
  size), and freed when only the cache still holds the handle. Settings with E013 return that error instead of an
  image; `ClampToBorder` without the device feature is previewed as `ClampToEdge` with a note. Limitation (controller
  ruling): `expand_grayscale: false` cannot be previewed — Bevy 0.20's `from_buffer` always expands and the variant
  taking the flag is `pub(crate)`; the Texture tab says so next to the toggle. The info line is ASCII
  `WxH | format | mips` (the UI font has no `×` / `·`).
- **Tabs.** A read-only `ImageDoc` (`ViewKind::Image`, config key `@game/<path>`, `TabKey::GameImage`) for a game
  image; a `TextureDoc` (`ViewKind::Texture`, key `shaders/….texture.ron`, or `@internal/<name>` → `TabKey::Internal`,
  read-only) for a Texture asset with `History<TextureAsset>` undo, a settings form, and a zoomable / pannable preview
  with "Tile 3x3" (an `ImageNode::rect` beyond `0..1` shows the address modes). `ImageNode` samples with the image's
  own sampler, so Nearest vs Linear shows when zoomed.
- **Texture thumbnails** (`ui::texture_preview::TexturePreview`): 128×128 left of the output pins since decision 0027 (64×64 under the title before), aspect kept, in every node with a
  `ParamKind::Texture` param; the key is (image key, settings, file stamp) from `Editor::texture_image` (built-in
  first, then open Texture tabs, then disk), so unsaved tab edits show without a canvas rebuild. A `TextureSample`
  whose `tex` is wired straight to a node with a texture param shows that node's texture (one hop only).
- **Convert to Parameter** (decision 0021 actions): `TextureSample` ↔ `param::TextureSample2D` and `TextureObject` ↔
  `param::TextureObject`, mapping `texture ↔ default` plus `mip_mode` / `sampler_type`, unique name `texture`, …;
  name / group / order are dropped on the way back. Conversions to a parameter node are hidden in function graphs
  (functions cannot have parameters), in the menu and for hotkeys.

**Consequences.**
- A new tab kind implements `TabDoc` and adds arms to `Tab::doc()` / `doc_mut()`, a view, a `TabKey` form and the
  props branch; a new node thumbnail is a `NodePreview` kind; a new droppable game file kind is a `DroppableFile`
  in `droppables()` plus a scan filter.
- A game image replaced on disk shows in a thumbnail at the next editor change (the watcher bypasses change
  detection); the image and Texture tabs pick it up within a frame.
- Converting a `TextureSample` whose `Tex` is wired drops that wire (the parameter has no `tex` input) and the
  parameter's default stays empty (W110); undo restores it.
- Texture assets, game image tabs and Sprite2d materials share icon-rs `IMAGE` (the only image-like icon in 0.4.5).
