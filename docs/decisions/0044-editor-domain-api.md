# 0044: Editor domain API

Design reference: `AGENTS.md` and decisions 0043–0045. Extends
decision 0043 (the backend side) to the editor.

## Context

The editor was one crate, `shader_editor_app`, with closed enums for every open set (`Tab`, `ViewKind`, `Action`,
`PropTarget`, `NewKind`, `IconId`, `for_each_kind`, the main menu items) and the Material state spread over `Editor`.
A second domain (Light, Plan C) would have needed edits to all of them. Plan B dissolved the crate into
`editor_core` (domain-neutral shell and domain API), `node_graph_base` (the graph UI), the kits' editor halves
(`curve_kit::editor`, `texture_kit::editor`) and `material_domain`, with Material's behaviour unchanged.

## Decision

- **Crates and dependencies (design §5).** Backend side: the kits' models depend on `project_core` only, domain
  backends on the kits' models (`default-features = false`). Editor side, a fan-in on `editor_core`: the kits (with
  `editor`) depend on `editor_core` (and `project_core`), `node_graph_base` on `editor_core` only — the kits and the
  graph base are independent siblings — and a domain (`material_domain`) depends on `editor_core`,
  `node_graph_base`, the kits and its backend. Domains never depend on each other; `editor_core` depends on no
  kit, domain or `shader_*` crate, and none of Material's types (`AssetType`, `Registry`, `PinType`, `Code`,
  `wesl_lines`) is visible from it. `src/main.rs`'s `domains()` lists each `EditorDomain` on one line; the GUI
  (`editor_core::run`) and the CLI (the domains' `backend()`s) share it.
- **A registrar, not one big trait (design §6.1).** `EditorDomain` has four methods (`id`, `info` → `DomainInfo
  { title, icon }`, `backend`, `register`); `register` fills a `DomainRegistrar` with data and function pointers:
  `state`, `asset_kind(AssetKindDef { suffix, title, icon, templates: Vec<AssetTemplate>, open, row_menu })`,
  `editor_kind::<E>()`, `game_assets(GameAssetsDef { extensions, open, menu })`, `tree_extras`,
  `menu_items(MainMenu, fn)`, `parse_tab`, `tab_exists`, `legacy_tab_keys`, `steps`, `highlighter(lang, fn)`,
  `input_context`, `logic` (systems also run headless, added by `EditorCorePlugin`), `bevy` (window systems, added
  by `DomainWindowPlugin`), `doc_view`, `doc_overlay`, `quick_start(QuickStartDef)`. `DomainList::new` runs every
  `register` once before the app starts (an `Arc` in `Editor::domain_list`, also a resource) and panics on a domain
  id registered twice or two domains claiming legacy tab keys. A registrar keeps a domain's whole registration
  readable in one function and lets a kit offer `register::<D>(r)` helpers that add an asset kind, editor kinds,
  views and systems in one call; a trait with twenty default methods could not be composed that way. Asset kinds,
  editor kinds and menu items appear in registration order (Material registers the kits between its own kinds to keep
  the Assets menu order).
- **`DomainRegistrar::steps`.** A handler `fn(&mut Editor, &Payload) -> Handled` of the domain's `Action::Domain`
  payloads and deferred document steps, tried in registration order before the state's `DomainState::apply`, with
  the state in place (not taken). A kit's commands (`texture_kit`: path edits that check one file — one asset,
  undo / redo of them, `CreateTextureForFile`) thus work with or without a state of the domain.
- **Open sets become values.** `Tab { domain: DomainId, doc: Box<dyn TabDoc> }` (typed access `Tab::get::<T>()`,
  `Editor::active_doc::<T>()`); `ViewKindId(&'static str)` — a plain string constant per kind, namespaced
  `<domain>.<kind>` (`"material.graph"`, `"material.curve"`; core's own is `"text"`) is enough, is readable in logs
  and needs no interning; a kit's view ids are a per-domain associated constant so two domains' instantiations never
  share one. `AreaOf` / `ViewOf` carry it, and
  `DomainList::kind_defs()` (core's text kind first, then every domain's, a shared one once; `DomainList::new` panics
  when two different kinds, by `KindDef::type_id`, share a view id) replaces
  `for_each_kind`. `Submit::Create { kind: (DomainId, suffix), template }` replaces `NewKind`.
- **`Payload` / `PropKey`: value-compared dynamic types.** `Payload` (`Arc<dyn DynPayload>`) carries a domain's
  command or edit, `PropKey` (`Arc<dyn PropTarget>`) the field a property widget edits; both compare by value
  (`dyn_eq` downcasts the other side), so undo merging (`history::MergeKey::Prop`) and sidebar redraw keys work
  across two keys built separately from equal contents. `PartialEq<T: PropTarget> for PropKey` lets code write
  `field.target == CurveProp::Range` without wrapping. The core widget value is `PropValue`; each kind has its own
  target type (`GraphProp`, `CurveProp`, `ConstsProp`, `TextureProp`, `GradientProp`).
- **Actions.** Core's `Action` holds project / tab / dialog / View / navigation / save / Generate / folder /
  rename / move / delete commands plus two open doors: `Action::Tab(TabAction)` for the active tab's document and
  `Action::Domain(DomainId, Payload)` for a domain's own command (Material: `MaterialAction`). `TabAction` is the
  shared edit verbs (`Undo`, `Redo`, `Copy`, `Cut`, `Paste(Option<Vec2>)`, `Duplicate`, `DeleteSelection`,
  `SelectAll`, `CommitInline`), the property writes (`SetValue`, `SetComponent`) and `Doc(Payload)` for kind-specific
  edits. Core dispatches each to `TabDoc::apply` of the active tab; a verb or payload the document does not know
  answers `Handled::No` and is dropped (debug log), so an edit queued for a tab that is no longer active is never a
  panic. **Scope note:** graph edits are `TabAction::Doc(GraphEdit)` (the design's `TabAction::Graph` would have put
  a `node_graph_base` type into core); there is no `GraphEdit::Paste` because `TabAction::Paste(pos)` already carries
  the position.
- **Domain state inside `Editor` (design §6.3).** `Editor::domains: DomainStates` holds one `Box<dyn DomainState>`
  per domain (`get::<T>()` downcasts; `DomainState: Any`, so no `as_any` boilerplate). Core calls its hooks — `apply`,
  `after_apply`, `after_save`, `before_close`, `rescan`, `diagnostics`, `console_diagnostics`, `tree_items`,
  `generate`, `delete_message`, `rename_file`, `renamed`, `before_delete`, `delete_file`, `deleted`,
  `focus_diagnostic`, `files_changed` — through `Editor::with_domain(id, f)`, which passes a `DomainHooks` dispatch
  table while keeping the state inside `editor.domains`, so a hook and any nested `with_domain` call can access the
  state directly via `editor.domains.get::<T>()` / `editor.domains.get_mut::<T>()`. Console/logs lines written
  meanwhile carry the domain (`Message::domain`; `Editor::as_domain` tags lines of registrar callbacks the same way).
  Keeping the state in `Editor` rather than in Bevy resources keeps the editor logic testable without a world. Opening
  a project runs `generate` (after the first `rescan`) of every real domain with a crate contribution whose
  `crates/quillshade/src/<id>/mod.rs` is missing: any domain's save rewrites the umbrella `src/lib.rs`, which names
  every domain's module, so a module never generated would leave the crate uncompilable.
- **`after_save(editor, tabs: &[usize])`** runs once per domain and save operation (Save, Save All, Save
  All and Quit, Save in the close prompt, Save before a project switch / close) with that domain's saved tabs, so one
  Save All regenerates Material once. Save All also calls it for domains with no dirty tab (empty slice); creating an
  asset calls it with an empty slice to mean "follow as after a save".
- **Tab keys (design §6.6).** A tab is remembered by `TabKey { domain, key }` — its document's `config_key()`,
  which only that domain parses back (`parse_tab`) — stored in the user config as `"<domain>:<key>"`. Restoring
  skips, silently, a key of an unregistered domain, one no domain parses and one whose target `tab_exists` says is
  gone; the other tabs and the active tab restore. A key without a valid `<domain>:` prefix (configs written before
  domains) is read as the key of the domain that called `legacy_tab_keys` (Material), so existing users keep their
  tabs. The navigation history (decision 0041) uses the same keys.
- **Per-domain folders.** Each domain's Project tree is `quillshade/<domain>/`; folder operations are core flows
  parameterized by the domain (`TreeId::Project(DomainId)`, `Action::SelectFolder(domain, path)`,
  `Submit::Folder(domain)`, `DeleteFolder(domain, path)`), with one selected folder per domain
  (`Editor::current_folders`) where that domain's "New …" items create assets, under `project_core`'s identifier
  rule for every domain.
- **`IconRef`.** An open handle to any `&'static icon_rs::Icon`, equal by name. Core keeps `IconId` for the chrome
  icons; a domain declares `pub const X: IconRef = IconRef::new(&icon_rs::Y)` (or a `custom(...)` SVG of its own,
  decision 0013) next to its code (Material: `material_domain::icons`, `MATERIAL_ICONS`). Asset kinds, templates,
  domains, tree rows and menu entries carry an `IconRef`. Names must be unique across all sets (the rasterizer cache
  is keyed by them).
- **Text viewer.** `TextDoc` is core's read-only view of a generated file, highlighted by the highlighter registered
  for its language: `highlight(list: &DomainList, text, lang)` takes the list (no global registry); Rust is core's
  own, WESL is registered by Material. "Show generated …" items come from the asset kind's `row_menu`.
- **Core property widgets (design §6.2).** The number / vector / color / checkbox / select / text fields and all
  their observers are core's (`editor_core::ui::props`, `PropWidgetsPlugin`, part of `DomainWindowPlugin`), so a
  kit's right sidebar works without Material. A field's final edit triggers `FieldEditEnded { source }`; core knows
  no special place for a field, and `node_graph_base::InlineEditsPlugin` (added once for all graph domains) turns
  it into `TabAction::CommitInline` for a field inside a canvas node. Material's `PinType`-aware widgets stay in
  `material_domain::ui::props`.
- **`DomainWindowPlugin`.** Adds the core property observers, then every domain's `bevy` hooks; `UiPlugin` includes
  it, and headless UI tests add it (with `domain_doc_views`) to get the domains' window parts exactly as the window
  does.
- **The kits' `editor` feature (design §7).** `curve_kit` / `texture_kit` are a Bevy-free model by default for
  backends (`default-features = false`) and, with `editor` (their default), the editor generic over a zero-sized
  domain type. Every type, system and observer is generic over it, so two domains get two editors side by side and
  the domain folder decides which one a `*.curve.ron` / `*.texture.ron` is (composition, never a copied module).
  Their `dynamic_linking` feature is not a default; the root package's `dynamic_linking` turns it on.
- **`CurveDomain`**: `DOMAIN`, `SUFFIX`, `VIEW`, `topbar_extras(&CurveDoc<Self>)`, `sidebar_extras`, `validate`
  and `row_menu(&Editor, &AssetRef)` (Material: its generated WESL and source). The copied points live in
  `Editor::clipboards` (`Clipboards`, one value per type, lent to documents as `DocCtx::clipboards`), as a
  `CurveClipboard { points, pastes }` with its own paste-offset counter.
- **`TextureDomain`**: `DOMAIN`, `SUFFIX`, `VIEW`, `IMAGE_VIEW`, `POLICY: TexturePolicy { extensions, shape,
  internal, presets }`, and hooks with defaults: `topbar_extras`, `row_menu`, `disk_owners(&Editor, file)` (which
  Texture assets point at a game file; Material answers from its last scan), `problems(&Editor, &AssetRef)` (the
  right sidebar's list; default: the domain's diagnostics of the asset) and `created(&mut Editor, &AssetRef)`
  (after "Create Texture Asset"; default: `after_save` with no tabs, which regenerates Material); opt-in constants,
  all off / Material's by default: `NEW_TEMPLATE` (a "New Texture..." template making an asset without an image,
  Light), `PREVIEW_FIRST_LAYER` (the preview shows a cube map's first face instead of every face stacked, Light,
  design §7.2) and `SETTINGS_NOTE` (how the settings reach the game, under the right sidebar's description).
  Built-in textures are `InternalTexture { id, file, bytes, settings }` (`@internal/<name>`), listed by
  `POLICY.internal`; the kit shows them in the domain's virtual Internal folder and opens them read-only.
- **`GraphDomain` (design §7.3).** UI and gestures are `node_graph_base`'s; the model stays the domain's. Besides
  the design's methods (`view`, `node_look`, `can_connect`, `plan_move_wires`, `header_action`, `pin_menu`,
  `node_menu`, `hotkey_actions`, `placement_hotkeys`, `palette` with each `PaletteEntry`'s `pick`, `drop`, `sidebar`,
  `topbar_extras`) it has `type Doc`, `const VIEW`, `diagnostics` (canvas marks, read every frame), `open_action`
  (body double click / Open in tab), `place` (a placement hotkey's node), `frame_prop` (a comment frame field's
  `PropKey`), `previews` (in-node preview rows) and the defaulted `bindings` / `build` / `input` for the domain's
  own graph keys and topbar systems. `topbar_extras()` takes no arguments: the scenes are spawned once with the
  editor topbar and fill themselves (Material: `GraphTopbarDynamic`). Several graph domains coexist: every canvas
  entity is tagged `GraphOf(G::VIEW)` and each domain's systems touch only their own; `GraphViews` lists all graph
  views so a camera request is dropped only when no graph tab is active. The layered auto-layout algorithm is the
  base's (`layout::auto_layout`); `auto_layout` / `node_height` as `Editor` operations stay Material's, as node
  heights come from Material's node signatures.

## Known limits (as written; status after decision 0045)

Superseded in part by decision 0045 (tools, the project domain, the mainbar / secondbar layout, asset-kind defaults,
domain-typed kit edits, the Light domain); the limits below keep their original wording with their status.

- With two domains that register `texture_kit`, folder menus and the Game Assets union show one unlabelled
  "Create Texture Asset" item per domain, and a game file opens in both; the domain bar's per-domain filter fixes it.
  **Resolved** (decision 0045): the left sidebar, the Game Assets rows, their menu and double click follow the active
  domain only.
- `CurveAction` / `CurveProp` and the texture kit's edits are not tagged with a domain: a stale edit lands on the
  active curve or Texture tab of whatever domain is active (Material-only today, so unreachable). **Resolved**
  (decision 0045): `CurveEdit<D>`, `TextureEdit<D>`, `PreviewChannel<D>`, `KitProp<D, P>`.
- A kit's `register` adds no tab-key parsing or tree rows: the domain wires them by hand — a kit-only domain passes
  `curve_kit::editor::parse_key::<D>` / `texture_kit::editor::parse_key::<D>` (and `tab_exists`) to `parse_tab` /
  `tab_exists`; Material's own key parser routes curve and Texture keys to the kits' `open` / `image_tab` — and lists
  the kit's assets in its `DomainState::tree_items`. Nothing checks that a key's domain prefix matches the folder of
  the asset it names. **Resolved** (decision 0045): core's asset-kind defaults give tree rows and source-path keys,
  `texture_kit::editor::register` wires its own `@game/…` / `@internal/…` keys, and `parse_tab` / `tab_exists` are
  lists. Still open: nothing checks a key's domain prefix against the asset's folder.
- The kit `register` helpers (`curve_kit::editor::register`, `texture_kit::editor::register`) wire no tab keys, tree
  rows or rename / delete: `DomainState::rename_file` / `delete_file` default to `Err`, so a kit-only domain must
  implement them. **Resolved** (decision 0045): `rename_file` / `delete_file` default to core's file operations.
- Core does not rename navigation history entries when an asset is renamed; only Material calls `nav.rename`.
  **Resolved** (decision 0045): `Editor::rename` renames the history and `last_tabs` for every domain
  (`TabKey::renamed`).
- Plan C adds the domain bar (bottom of the window, design §8.1) with greyed tabs of other domains and a last
  active tab per domain, the Project pseudo-domain and its Project Settings tab, the registrar's `project_settings`
  / `user_settings` sections and `sidebar_pane`, the per-domain Assets menu, the console filter by domain, and the
  Light domain. **Done differently** (decision 0045): domain tabs in the mainbar instead of a domain bar; greyed tabs,
  last tabs, the project domain with Project Settings as a tool, the per-domain Assets menu, the console filter and
  the Light domain are built; the `project_settings` / `user_settings` sections and `sidebar_pane` are deferred.

## Consequences

A new domain is a crate with one `EditorDomain` and one line in `src/main.rs`; no core enum or `match` changes.
Curves and Texture assets of a new domain are one `register::<D>` call each. The records that described the old
single-crate module layout — decisions 0013 (icons: `IconId` only), 0026 (module seams), 0031 (editor kinds and
`for_each_kind`), 0032 (input contexts per `ViewKind`) and 0034 (where the canvas and widgets live) — are superseded
in part by this one (paths, closed enums, the kind list); they are no longer in the tree (removed in commit
`195c3ae`, kept in git history), and their rules that did not change (icon review, Feathers widgets, input contexts
and their priorities) still apply as `AGENTS.md` states them.
