# 0028. Texture tab: Easy / Advanced mode, use-case presets, channel view, the real default sampler

**Context.** QA of the Texture tab (decision 0026): the sampler form (decision 0024) exposes every
`ImageSamplerDescriptor` field, while most textures want one of a few setups — the user should only pick what the
texture is for. Two things also confused: turning "Default" off filled the fields with Bevy's
`ImageSamplerDescriptor::default()` (**Nearest** filters, Clamp to edge), which is *not* what a game gets for the
default sampler; and "Default" was expected to mean Repeat (some tutorials say so). In Bevy 0.20 `ImagePlugin::default()`
sets `default_sampler: ImageSamplerDescriptor::linear()` — Linear mag / min / mipmap filters, `ClampToEdge` on every
axis — and the PNG loader generates no mipmaps (only KTX2 / DDS / Basis files bring their own).

**Decision.**
- `SamplerSettings::image_plugin_default()` (core) is that `linear()` sampler. The editor reads a default sampler's
  fields as it (`texture_doc::effective_sampler`), and turning "Default" off starts from it, so the preview does not
  change — the "Tile 3x3" preview of a default texture shows Clamp to edge, as the game would.
- **Easy / Advanced** (`TextureDoc::easy`, editor-only, not undoable, not saved). Easy has **no form**: the header
  offers "Use for" presets and that is the whole choice. Advanced is the form (path, sRGB, Expand grayscale, sampler).
  A tab starts in Easy when nothing would be hidden from it (`fits_easy`: a default sampler or exactly a preset's);
  sRGB and the path are Advanced-only, and Properties shows the path.
- **Presets** (`FilterPreset`, each a whole sampler with default LOD clamps; after the filtering guide the user referred
  to, address modes chosen per use): Pixel art = Nearest ×3, Clamp to edge; UI = Linear / Linear / Nearest
  (bilinear), Clamp to edge; 3D = Linear ×3, Repeat; Ground = Linear ×3 + anisotropy 16, Repeat. A preset replaces the
  sampler only (sRGB and grayscale expansion stay) as one undo step (superseded by 0042: presets also set sRGB, new
  Normal map preset). A default sampler is no preset (none highlighted):
  3D differs from it by Repeat, which tiled 3D textures need.
- **Channel view** R | G | B | All (`TextureDoc::channel`, `Action::TextureChannel`, also on built-in textures): one
  channel shows as a grey image — `GameImageCache::get_channel` decodes the image again and copies the channel into
  R, G, B (opaque) before the pixels go to the render world; formats without per-pixel bytes (compressed) are tinted
  instead. Preview only.
- The header is one row: Easy | Advanced, the presets, R | G | B | All, Tile 3x3, the zoom. The image's
  `WxH | format | mips` moved to Properties, next to the path. Button groups use the shared `ui::chip` (also the Quick
  Start step tabs, decision 0027).

**Consequences.** Nothing in saved files changes; a texture with an explicit Nearest / Clamp descriptor (the old
"Default off") is Pixel art. A grey channel view costs one more decode of the image while it is shown.
