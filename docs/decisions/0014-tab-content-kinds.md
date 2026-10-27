# 0014. Tab content kinds; read-only text tabs for generated files

**Context.** Every tab was a node graph: `Editor.docs: Vec<Doc>`, the tab bar, the canvas and the properties
panel all assumed it. Planned content needs other views: read-only text (generated WESL/Rust), a curve editor,
gradients / gradient collections, a form + table editor for global shader parameters. Spec §9 has "Show generated
WESL/Rust" open the file in an external program; for a quick look that leaves the editor and, on Linux, depends on
the desktop's handler for `.wesl` (see `external.rs`).

What Bevy 0.20 / Feathers 0.20 offer for text (researched in the registry sources):
- `Text` + `TextSpan` children, each span with its own `TextColor`/`TextFont` (parley layout) — enough for syntax
  colouring; no selection.
- `EditableText` (`bevy_text::editing`) with `TextReadWriteMode::ReadOnly`: cursor, selection, Ctrl+C, scrolling,
  but one style for the whole buffer (`parley::PlainEditor` with a single default style) — no colouring. Feathers'
  `text_input` is a single-line field built on it.
- Fonts: Feathers embeds `fonts::MONO` (FiraMono Medium); Bevy's default font is a FiraMono subset.
- `bevy::clipboard::Clipboard` (arboard, part of `DefaultPlugins`) can set the system clipboard.
- Highlighters: `syntect` has Rust but no WGSL/WESL grammar and pulls in regex engines plus large dumps;
  tree-sitter needs C grammars compiled per platform. Both are heavy for ~100-line generated files.

**Decision.**
- `tab::Tab` is an enum of content kinds (`Graph(Doc)`, `Text(TextDoc)`); `Editor.tabs: Vec<Tab>` replaces `docs`.
  `Editor::doc()` / `doc_mut()` return the active tab's graph and are `None` on other tabs, so every graph command
  (palette, hotkeys, undo, paste, view requests) is a no-op there without extra checks; `docs()` iterates the
  open graphs (validation, dirty state, rename). Each kind maps to a `tab::ViewKind`.
- Views live in `ui::DocArea` (below the tab bar). Each is a full-size child tagged `ui::ViewOf(kind)`, with the
  dark `VIEW_BG` background; `show_active_view` makes the active tab's view visible (the graph view also when no tab
  is open). The `DocKindText` watermark and the minimap are spawned after the views, so they stay on top of any of
  them; the watermark names the asset type for graphs and the language (`wesl`/`rust`) for text. The graph canvas
  is not despawned: with a text tab active `doc()` is `None`, the canvas rebuilds empty and stores the graph's
  camera, which is restored when its tab comes back.
- *Superseded in part by 0031* (the recipe for a new kind and the per-kind toolbar / properties panel).
  Adding a content kind = a `Tab` variant + `ViewKind` variant, a view scene spawned in `DocArea`, its redraw
  system keyed on (tab identity, revision), a branch in the properties panel and the watermark label, and a
  `Tab::config_key` form so it is restored on the next launch.
  *Update:* what every kind shares (asset, view, title, icon, watermark label, dirty/read-only state, revision,
  save, external-change check and reload, rename, config key) is the `doc_kind::TabDoc` trait, reached through
  `Tab::doc()` / `doc_mut()`; a new kind implements it and adds its arm to those two matches, and code that only
  needs one of these operations no longer matches on the kind. Still per kind: the revisions that trigger validation
  (`revisions`), `parse_config_key`, the view toolbar and the properties panel. (Decision 0026 records the trait's
  final shape: `keep_mine`, `kind_label`, `override_into`, `rename` taking the project.)
- "Show generated WESL/Rust" (browser RMB) opens a `TextDoc` tab (`Action::ShowGenerated`); the same file is
  activated rather than opened twice. Files are re-read on activation and after every regeneration (the revision
  only changes when the text did, so views don't rebuild needlessly). A rename moves them with the asset, a delete
  closes them. They are stored in the config as `shaders/<path>.ron#wesl` / `#rust`.
  *Update:* game images (design §4.4) open in a read-only `ImageDoc` tab (`ViewKind::Image`, double click in Game
  Assets, `Action::OpenGameImage`), stored as `@game/<path under assets/>`; `parse_config_key` returns a
  `tab::TabKey` (`Asset` / `Generated` / `GameImage`). The tab only holds the path and a file stamp (mtime + size);
  the view decodes the bytes with `Image::from_buffer` through `game_image::GameImageCache` (the editor's
  `AssetServer` serves the editor's own assets), shared with the Texture tab and node thumbnails.
  *Update:* Texture assets open in a `TextureDoc` tab (`ViewKind::Texture`, config key `shaders/….texture.ron`;
  built-in textures read-only, key `@internal/<name>` → `TabKey::Internal`). The unsaved content used for validation
  now goes through `TabDoc::override_into` (each kind puts its content into the `AssetSet`), so `overridden_set` no
  longer matches on the kind; `revisions` still lists the kinds whose edits change diagnostics (graphs, constant
  tables, Texture assets).
- Highlighting is our own small lexer (`highlight.rs`): comments (nested block comments), strings/chars, numbers,
  keywords, built-in and CamelCase types, function calls, `@attributes` (WESL), `#[attributes]`, macros and
  lifetimes (Rust). The text view is a line-number `Text` next to one `Text` whose `TextSpan`s carry the colours
  (same font and size, so lines match), no wrapping, scrolled by the wheel (Shift+wheel horizontally).
- Since spans can't be selected, Ctrl+C on a text tab (and the Copy button) copies the whole file; the header and
  the properties panel offer "Open in default application" (`Action::ShowFile`, the old behaviour) and the source
  graph. "Show source (.ron)" still opens externally.

**Consequences.** Deviation from spec §9 (generated files open inside the editor; the external program is one
click away). The toolbar is still graph-oriented (Auto-layout, Comment, Fit do nothing on a text tab); a per-view
toolbar is left for when the next view arrives. Partial selection in highlighted text would need either an
`EditableText` overlay with transparent glyphs or a highlighter for `EditableText` in Bevy itself.
