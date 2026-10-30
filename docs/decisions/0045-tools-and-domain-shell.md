# 0045: Tools, the project domain and the domain shell; the Light domain

Design reference: `AGENTS.md` and decisions 0043–0045. Extends decisions 0043 (backend layout) and 0044 (editor domain API); supersedes in part 0044's
"Known limits" and the domain bar of the initial multi-domain design.

## Context

Decision 0044 made the editor domain-neutral, but still showed every domain's tabs, menu items, tree and console lines
mixed together in one shell (Material was the only domain, so nobody noticed). A
second domain (Light) and a Project Settings tab needed three things the API lacked: tabs that are not assets
("tools"), an explicit active domain with a way to switch it, and a shell that filters by it. The owner changed the
layout twice while this was built; this record describes the final state.

## Decision

### Tools (design §14.1)

- A **tool** is a singleton tab without an asset path. A domain registers it with `DomainRegistrar::tool(ToolDef { id,
  title, open, menu })` (`editor_core::domain::tool`); `open: fn(&mut Editor) -> Box<dyn TabDoc>` makes its document,
  `menu: Option<MainMenu>` adds an item `"{title}..."` to that main menu. The document is an ordinary `TabDoc`
  (`TabDoc::tool() -> Some(id)`) shown by an ordinary editor kind.
- **Uniqueness is per `(domain, id)`**: two tools of one domain with one id panic at registration; the same id in two
  domains (a kit registering a tool for each of its domains) gives two tabs. `Action::OpenTool { domain, id, arg }`
  activates the open tab or opens a new one.
- **The argument** (`arg: Option<Payload>`) is delivered as `TabAction::Doc(arg)` to the tool's document after it is
  active, whether it was just opened or already open, followed by the domain's `after_apply`. A tool therefore needs
  no second entry point for "open with this file".
- **Key and persistence.** A tool tab's `TabKey` is `"@tool/<id>"` in its domain (`TOOL_KEY_PREFIX`); no asset key
  starts with `@tool/`. Back / Forward use it (a closed tool reopens), but tool tabs are **never written to the user
  config**: not in `open`, not as the active tab, not in `last_tabs`; a `@tool/` key found there is skipped.
- **Menu visibility.** A real domain's tool items and `menu_items` show only while that domain is active; the project
  domain's always. In File, tool items follow Close Project; in other menus they come first, then a separator.

### The project domain and the overlay rule (design §14.2)

- `DomainId::PROJECT` (`"project"`) is reserved: `DomainList::new` panics if a domain passed to it uses it and appends
  the built-in `ProjectDomain` itself, always last. `src/main.rs` lists only real domains. The project domain has no
  backend, asset kinds, folder or Game Assets; the CLI does not know it (`--domain project` is an unknown domain,
  exit 2). `DomainList::real_ids()` skips it.
- **Overlay rule (owner).** Activating a tab of a real domain makes that domain active; activating a project tool's
  tab does **not** change the active domain. While a project tool is shown, the left sidebar, the Assets menu, the
  console filter, the greying of tabs and the domain input context stay the active domain's.
- **`Editor::active_domain`** is explicit state (needed by the overlay rule). It changes when a real domain's tab is
  activated, when a domain tab is clicked (`Action::SwitchDomain`) and on Back / Forward (they go through the same
  activation). **`Editor::last_tabs`** remembers each domain's last active tab key; switching to a domain activates
  it, or shows the domain's empty hint when it has none; switching to the project domain with no remembered tab opens
  Project Settings. Switching to the active domain does nothing.
- **Close successor.** Closing the active tab activates the nearest staying tab of the active domain or a project
  tool, looking right first, then left, else none: closing never jumps to another domain.
- **User config.** Per project, `ProjectTabs` stores `active_domain` and `last_tabs` (domain id → key, tool keys
  excluded). A saved domain no longer registered falls back to the first real domain; a restored active tab that
  would be greyed under the active domain gives way to that domain's last tab, or none. Old configs (a `maximized`
  field, no `active_domain` / `last_tabs`, a `project:` key in `open`) open without a dialog.

### Window layout: mainbar and secondbar (design §14.3, owner change)

The design's **domain bar** at the bottom of the window (§2, §8.1) is not built. The first version put the domain
tabs at the left of a full-width tabs bar; the owner then moved them into the top bar and renamed the areas
(2026-10-30). The final layout:

- **mainbar** (was "menu_bar"): logo, main menu, a separator (`MainBarSeparator`), the **domain tabs**, then the
  project name (`ProjectText`), a separator (`MainBarSeparator`) and the error count (`ProjectErrorsText`), on
  `MAINBAR_BG` (darker than all other editor surfaces). Domain tabs are custom full-height rectangular buttons
  (`BorderRadius::ZERO`), one per domain in registration order, the project domain last; each shows only its name (no
  icon), the inactive ones transparent with dim text, the active one with `DOMAIN_TAB_ACTIVE_BG` (warm orange matching
  the accent palette) and near-white text (`DOMAIN_TAB_ACTIVE_TEXT`). They are tabbable, hidden (with the separators
  and the error count) while no project is open, and updated in place (not respawned) so keyboard focus survives a
  switch. The mainbar is no taller than before (`31px`).
- **secondbar** (was "tabs bar"): the full window width under the mainbar, above both sidebars, in three sections
  `{main actions} | {domain actions} | {tabs}`, separated only between non-empty sections: **main actions** = Back /
  Forward (moved from the mainbar, unchanged); **domain actions** = a reserved slot for a domain's main actions, an
  empty container with `display: None` and no registrar API yet; **tabs** = all tabs of all domains. The secondbar is
  one tab row high and grows by whole rows (`flex_wrap`, fixed tab height) so no tab is ever hidden behind a scroll.
- **Tab look.** An asset tab has its icon and label; a **tool tab** has no icon and a dark-blue background; a tab of
  a real domain other than the active one is **greyed** (text, icon and close cross) but stays clickable and
  activating it switches the domain; project tool tabs are never greyed. Ctrl+Tab cycles through all tabs.
- Zen mode hides neither bar.
- **Theme tokens** (`editor_core::theme`): `MAINBAR_BG` (mainbar background, darker than all other surfaces),
  `MAINBAR_SEPARATOR` (vertical divider in the mainbar), `TAB_BG` (an asset tab), `TOOL_TAB_BG` / `TOOL_TAB_ACTIVE_BG`
  (dark blue), `TAB_GREY` (translucent, because a solid icon's tint sets only its alpha — an opaque grey would leave
  Material's orange icons at full colour), `DOMAIN_TAB_ACTIVE_BG` (warm orange fill of the active domain tab),
  `DOMAIN_TAB_TEXT` / `DOMAIN_TAB_ACTIVE_TEXT` (neutral text, the active one brighter).
- **Left sidebar, menus, welcome** (design §14.4): the left sidebar shows only the active domain's project tree and
  Game Assets (hidden for a domain without them); with the project domain active it lists the project tools. The
  Assets menu is the active domain's "New …" items, New folder (when it has asset kinds), a separator and Generate.
  Game file menus and double clicks use the active domain's `GameAssetsDef` only, so two texture domains no longer
  show two "Create Texture Asset" items. Help → Quick Start starts with core's Crate / Plugin steps, then each
  domain's steps titled `"<Domain>: <step>"`; the welcome screen lists each real domain's `DomainInfo::summary`.

### Console filter (design §6.4)

The console/logs shows the active real domain's diagnostics and messages (`Editor::console_domain`), plus global lines
and the project domain's (they concern every domain); a captioned "All domains" checkbox in the console header
(`Action::ToggleConsoleAll`) lifts the filter, as does the project domain being active. A project tool's tab keeps the
active domain's filter (overlay rule). The mainbar's error count stays the total of all domains. Focusing a
diagnostic of another domain switches to it through the normal open path.

### Project Settings (design §14.2)

- One project tool, `settings` ("Project Settings", File menu, right after Close Project), view kind
  `"project.settings"`, in `editor_core::settings`. General: project path, format version (read-only), "Header date";
  then one subpane per real domain with its "Used in game" checkbox. Typed per-domain settings are deferred.
- **Saving** writes `quillshade/project.ron` with `project_core::layout::save_settings` (atomic, the same text as
  `ensure_layout`), then rewrites the umbrella crate's `Cargo.toml` and `src/lib.rs` with
  `project_core::regen::write_umbrella` (no domain outputs, no orphan scan; blocked files are E008 lines). A disabled
  domain keeps its `pub mod`; only its plugin line goes.
- Undo / redo through `History<ProjectSettings>`; turning a domain back on removes the entry the saved file did not
  have, so the tab returns to clean.

### Outside edits by tab key

`Action::Reload(TabKey)` and `Action::KeepMine(TabKey)` address a tab by its key, so tool tabs take part:
after any watched change (`project.ron` or a domain file) every tab whose `changed_on_disk` is true reloads when clean,
and a dirty one gets one "Keep mine" / "Reload" prompt. An unparsable `project.ron` keeps the previous settings and
the tab, with a warning.

### Asset-kind defaults and their contracts (design §14.4)

- **Core defaults.** A domain that does not override them gets, from its registered asset kinds: the Project tree rows
  (`state::kinds::scan`, cached in `Editor::kind_files` at rescan; `DomainState::tree_items` now returns `Option`,
  `None` = default), tab keys (a key equal to a kind file's source path opens through the kind's `open`), and file
  rename / delete (`DomainState::rename_file` / `delete_file` default to them). Core renames the navigation history
  and `last_tabs` on every rename.
- **`parse_tab` handlers are a list** (a domain's and its kits'), tried in registration order, then the default
  `source_rel` fallback. `None` means "not mine" — and also "failed" — so **handlers must claim disjoint keys**; a key
  claimed by two handlers that fails to open is tried, and reported, twice. Material therefore parses only its
  generated-file keys (`#wesl` / `#rust`), `texture_kit` its `@game/…` / `@internal/…` keys, and source paths go to
  the default.
- **`tab_exists` without a handler** now means "a file of a registered kind exists at this source path"; a domain
  with custom keys needs its own handler.
- **`<key>#<view>`.** A view derived from an asset's tab (Material's generated files) is keyed `<asset key>#<view>`;
  `TabKey::renamed` rewrites both forms on a rename, in the same domain only.
- **Domain-typed kit edits.** The kits' edit payloads carry their domain type: `CurveEdit<D>`, `TextureEdit<D>`,
  `PreviewChannel<D>`, and one `editor_core::dynval::KitProp<D, P>` for property keys (`CurveProp` / `TextureProp` are
  no longer `PropTarget`s, so an untagged key does not compile). A document downcasts only its own `D`, so an edit
  queued for a Material texture is dropped, not applied, once a Light texture tab is active.
- **Kit additions.** `texture_kit` has Bevy-free `load` / `save` / `problems` (the texture rules, shared by Material
  and Light with the same order and texts) and opt-in `TextureDomain` constants: `NEW_TEMPLATE` (a "New Texture..."
  template without an image), `PREVIEW_FIRST_LAYER` (preview a cube map's first face) and `SETTINGS_NOTE`.
- **Missing modules.** Opening a project runs `generate` of every real domain with a crate contribution whose
  `crates/quillshade/src/<id>/mod.rs` is missing (disabled domains too: the umbrella names every domain's module), so
  saving any domain never leaves the umbrella crate uncompilable.

### The Light domain (design §9.2)

- **Crate `light_domain`**, following the kit pattern: the Bevy-free `backend` module and the `editor` feature
  (default on; `LightDomain`); the workspace dependency has `default-features = false`, the root package enables
  `editor` and forwards `dynamic_linking` (not a default). It depends on `project_core`, `texture_kit` and, with
  `editor`, `editor_core`; never on `material_domain` or a `shader_*` crate.
- **Model.** `quillshade/light/**`: `*.environment.ron` (`Environment { format_version: 1, cubemap: Option<String>,
  skybox_brightness: 1000.0, env_intensity: 1.0 }`, the cube map a Light Texture asset id) and `*.texture.ron` through
  `texture_kit` with `LIGHT_TEXTURES = TexturePolicy { extensions: ["ktx2"], shape: CubeMap, internal: None, presets:
  false }`.
- **Codes** (decision 0043, `L` prefix): `LE001` unknown cube map texture, `LW101` no cube map, `LE009` file does not
  load, and the texture kit's rules under Light's codes — `LE011` one file shared, `LE012` path, `LE013` sampler,
  `LE014` not a cube map, `LW109` file missing.
- **Codegen.** `LightBackend` writes one file, `crates/quillshade/src/light/mod.rs`: `ENVIRONMENTS` (the asset paths
  of the environments without errors, sorted) and an empty `QuillshadeLightInternalPlugin`. It owns
  `assets/quillshade/light/` for the orphan scan but writes nothing there yet.
- **Editor.** Assets menu New Environment... / New Texture...; an environment form tab (`"light.environment"`: cube
  map picker over the Light Texture assets, two number fields, undo / redo, live diagnostics); the texture kit's
  Texture tab without presets, whose preview shows a cube map's first face. Renaming a Light texture rewrites the
  environments that reference it (open tabs follow: a clean one reloads, a dirty one keeps its edits); deleting it
  clears them with a warning. Icon: `icon_rs::THEME_LIGHT` (icon-rs has no plain sun).
- **Sample.** `cargo run -p light_domain --example make_sample -- examples/sample_game` writes
  `assets/textures/sky_cube.ktx2` (a real 4×4 RGBA8 sRGB KTX2 cube map, six faces, read back by Bevy's own loader in
  a test) and `quillshade/light/{sky.texture.ron, day.environment.ron}`; `regen` then lists `day`. No fallback was
  needed. The example does not regenerate itself: a Light-only run would drop Material from the umbrella crate.

### Still deferred

- Typed per-domain settings in Project Settings (design §4.1 `settings`, §6.6).
- A tool's own persistent data (presets, last folder) in the user config dir: no API yet.
- The texture converter / HDR→KTX2 cube-map baker tools.
- The domain actions slot of the secondbar (no registrar API).
- The game side of Light: `QuillshadeLightInternalPlugin` does nothing yet.

## Consequences

A new domain is one `EditorDomain` and one line in `src/main.rs`; with the asset-kind defaults it gets its Project
tree, tab keys and file rename / delete without code, and its kit tabs cannot receive another domain's edits. The
shell shows one domain at a time, while project tools work over any of them. Decisions written before this one keep
the old area names (menu_bar, tabs bar) as history.
