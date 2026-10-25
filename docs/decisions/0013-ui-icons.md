# 0013. UI icons

**Context.** The asset browser showed folders as `> name/` / `v name/` and the asset kind as a text tag
(`fn`/`pbr`/`unlit`/`2d`); tabs had no kind marker. We want an IDE-like file tree. Bevy 0.20 has no SVG loader,
Feathers 0.20 only ships three embedded PNG icons (`feathers::display::icon` is an `ImageNode` with an asset path).
[icon-rs](https://crates.io/crates/icon-rs) ships consistent 24x24 SVG icons as Rust constants (`Icon { svg, kind, .. }`
with `recolor`/`tinted`), but no rasterizer; its `bevy_color` feature targets bevy_color 0.19 and is not used.

**Decision.**
- Dependencies: `icon-rs = "=0.4.5"` (pinned: renaming an icon there is a breaking change; we refer to the constants,
  so it fails at compile time) and `resvg 0.48` with `default-features = false` (no text/fonts/raster images).
- `ui::icons`: `IconId` maps editor icons to icon-rs constants (`FILE`, `FOLDER`, `CARET_*_FILLED`, `VARIABLES` for
  functions, `MATERIAL_LIBRARY` for Pbr3d, `IMAGE` for Sprite2d). A `UiIcon { id, size }` component gets its image in
  `PostUpdate` from a cache keyed by (icon, layer, physical px): rasterized at `size × scale_factor × UiScale`, and
  re-resolved when that scale changes, so icons are sharp on HiDPI screens instead of a scaled bitmap.
- Layers (after cadrs, the icon-rs author's Bevy app): glyph/line icons are rendered white and tinted via
  `ImageNode::color`; their accent detail is a child image in `icons::ACCENT`, so tinting doesn't darken the accent.
  Solid/sketch icons are rendered in `Palette::DARK` (= icon-rs's `svg_dark`) and only faded by the tint's alpha.
  tiny-skia's premultiplied pixels are demultiplied (Bevy expects straight alpha).
- Icons icon-rs lacks (`folder-open`, `material-unlit`: Pbr3d's sphere-on-plate but flat grey — "no lighting") are
  drawn by hand in `crates/shader_editor_app/icons/*.svg` following icon-rs's rules (24x24, its root attributes and
  stroke widths, `Palette::LIGHT` colours only, no ids) and wrapped as `icon_rs::Icon` values, so recolouring works
  the same. We don't fork icon-rs or reuse its generator (`xtask` is unpublished and has no extension point).
- Browser rows: caret (or equal spacing) + folder/asset icon + name; the text kind tag is gone. Tabs: asset icon
  before the name. `+…` buttons use `ui::icon_tool_button`; context menus take optional icons
  (`dialog::open_icon_menu`; items without one keep its space so labels stay aligned). In the toolbar the frequent
  actions (Save, Save all, Undo, Redo) are icon-only (`ui::icon_button`) with a hover `tooltip::Tooltip` naming
  the action and hotkey; Feathers 0.20 has no tooltip widget, so ours watches `Hovered` (which covers descendants)
  and shows a popup under the widget after 0.5 s. `save`, `save-all`, `fit` and `auto-layout` are hand-drawn.

**Consequences.** ~13 small pure-Rust crates added (usvg, roxmltree, svgtypes, kurbo, …). Each (icon, layer, size) is
rasterized once and cached; a scale change re-resolves the visible icons.
