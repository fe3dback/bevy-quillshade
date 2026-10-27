# 0031. Editor kinds own their window areas

**Context.** The editor grew one content kind at a time (graph, text, curve, consts, image, texture; decisions 0014,
0015, 0017, 0026). What a kind shares about its *document* was already the `TabDoc` trait, but its *UI* was spread
over `match tab` arms: kind buttons in the global toolbar (`view_tools`), the watermark label, the status text, and
the Properties panel (`ui/props.rs`, one `redraw` with a branch per kind). A new kind or a new button had no obvious
home (design `docs/superpowers/specs/2026-10-26-editor-kinds-qa-design.md` §1, §2.1).

**Decision.**

- **Area names.** The window areas are named as in `CLAUDE.md` → "Editor": menu_bar, left sidebar (project tree,
  assets tree), tabs bar, editor topbar, editor content, editor statusbar, console/logs, right sidebar. The menu_bar,
  the left sidebar, the tabs bar and console/logs are global. The editor topbar, editor content, editor statusbar and
  right sidebar belong to the active tab's *editor kind*. Code comments, docs and commit messages use these names.
- **One module per kind.** `crates/shader_editor_app/src/editors/<kind>.rs` holds every trait impl of the kind and
  the builders of its areas. The document types stay in their own files (`doc.rs`, `curve_doc.rs`, `tab.rs`, …).
  `editors/mod.rs` holds the shared traits and data.
- **Several small traits** instead of one big one:
  - `doc_kind::TabDoc` (on the document type) covers the document: asset, view, title, icon, watermark, dirty and
    read-only state, revision, save, external-change check and reload, rename, config key and `override_into`.
    `status(&StatusCtx) -> Option<StatusInfo>` gives the editor statusbar content (design §2.3). The editor fills
    `StatusCtx` with this asset's diagnostics and its generated WESL line count (`Editor::status_info`).
  - `editors::EditorAreas` (on a zero-sized `GraphEditor`, `TextEditor`, … per kind) covers the window areas:
    - `VIEW` (its `ViewKind`)
    - `SIDEBAR_TITLE` (the right sidebar header)
    - `topbar()` and `sidebar()`: `Option<impl Scene>`. `None` means the kind has no such area.
    - `build(app)`: registers the systems that fill those areas.
  - `editors::for_each_kind` with a `KindVisitor` is the one list of kinds. The editor topbar and the right sidebar
    are built by visiting it, and `AreasPlugin` calls each kind's `build` through it. No second list of the six types
    exists.
- **Areas are persistent scenes.** Each one is spawned once at startup inside its strip, in a container tagged
  `AreaOf(ViewKind)`.
  - `ui::areas::show_active_areas` sets `Node::display` to `Flex` on the active kind's containers and to `None` on the
    others. It uses `Display`, not `Visibility`, because hidden flow children must not take space, and it writes only
    on a change. (*superseded by 0035:* it also sets `Visibility::Hidden` on the hidden containers, so Tab navigation
    skips them.)
  - A strip with nothing for the active kind is not displayed at all: no empty strip and no empty panel. The one
    exception is the right sidebar with no tab open, which shows the header "Properties" and "Nothing open."
  - The right sidebar header shows the active kind's `SIDEBAR_TITLE`: Properties, Generated File, Curve, Constants,
    Image or Texture.
- **Content is rebuilt only when it changes.**
  - Static areas, such as the graph topbar buttons, are plain scenes.
  - Dynamic ones are a body with a kind-specific marker (`TextViewBar`, `GraphSidebarBody`, …), filled by the kind's
    redraw system. That system is keyed on what it shows and is a no-op while another kind is active.
  - The right sidebar bodies are keyed on `(Editor::ui.props, Editor::active)`, the old Properties key, through
    `editors::SidebarKey`. A body forgets its key while its kind is inactive, so it is rebuilt when the kind comes
    back.
  - The editor statusbar is one strip filled from `Editor::status_info()`. It is keyed on the log revision, the active
    tab, its config key, its revision and its dirty state.
- **What stays global.**
  - `ui/props.rs` keeps the property widgets (number / vector / select / text fields, `section`, `labeled`,
    `hotkey_row`, …) and their `ValueChange` / `TextEditChange` observers, shared by every kind and by inline editors
    on nodes.
  - The editor content views stay in `ui::DocArea`, tagged `ViewOf(kind)` (decision 0014). They are never despawned,
    so the graph canvas keeps its camera.
- **Layout.** The UI root has `min_width: 0`. Bevy places a root in an implicit grid cell that would otherwise grow to
  the root's min-content width, because the topbar buttons do not wrap, and push the right sidebar out of the window.

**Recipe for a new editor kind** (supersedes the list in 0014):

1. Add the document type, a `tab::Tab` variant and a `ViewKind` variant, plus their arms in `Tab::doc()` /
   `doc_mut()`. Add a `tab::TabKey` form in `parse_config_key` so the tab is restored on the next launch.
2. In `editors/<kind>.rs`:
   - Implement `TabDoc` for the document, with `status` if the kind has an editor statusbar.
   - Add a zero-sized `<Kind>Editor` implementing `EditorAreas`, with markers and redraw systems for its topbar and
     sidebar bodies.
   - Register it in `for_each_kind`.
3. Spawn its editor content view in `ui::DocArea`, tagged `ViewOf(kind)`.
4. If its edits change diagnostics, add it to `Editor::revisions`.

**Consequences.**
- Kind-specific UI no longer needs a `match tab`. The remaining matches are the `Tab` enum's own accessors and
  editing logic that belongs to one kind (`Editor::doc()` for graph commands).
- Every kind's right sidebar is a separate scroll column, so each kind keeps its own scroll offset.
- The text kind's sidebar shows only file information. Its actions (Copy / Source / Open externally) live in its
  editor topbar.
