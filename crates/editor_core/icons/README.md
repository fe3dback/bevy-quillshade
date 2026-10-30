# Editor icons

Icons the editor needs but [icon-rs](https://crates.io/crates/icon-rs) does not have. They are drawn by hand in
icon-rs's house style, so they recolour and tint like its own icons (decision 0013):

- 24x24 `viewBox` (solid icons may use icon-rs's fitted view box, e.g. `1.8 2.45 20.4 20.4` like
  `material-library`), drawing kept inside 1.5..22.5.
- Root attributes as icon-rs writes them: `fill="none" stroke="#262626" stroke-linejoin="round"
  stroke-linecap="round"`, `stroke-width` 1.75 for line icons, 1.1 for solid ones.
- Only palette colours of `Palette::LIGHT`: ink `#262626`, faces `#ffffff #d4d4d4 #a4a4a4 #747474`, accent
  `#2563eb`. Line icons use ink (+ a little accent) only. No `id` attributes.

Core (UI chrome) icons live here and register in `src/ui/icons.rs` (`custom(..)` + an `IconId` variant); a domain keeps
its own SVGs next to its code (Material: `crates/material_domain/icons/`) and declares them as `IconRef` constants.
The tests (`core_icons_rasterize`, `assert_icons_rasterize` / `assert_only_palette_colours` for a domain's list) check
that they parse, are drawn and use only palette colours; names must be unique across all sets.
