# 0042. Texture Easy mode: Normal map preset, presets set sRGB; the node palette lists every node

**Context.** QA: the Texture tab's Easy mode (decision 0028) had no choice for normal maps, which need linear data
(`is_srgb: false`) — a user had to go to Advanced for it. Presets replaced the sampler only and Easy hid the sRGB flag,
so a linear texture under any preset showed in Easy with nothing telling it was not color data. Separately, the node
palette (spec §9.3) opened from a right click with an empty query showed only the first categories (color, consts,
coordinates, functions): its results were capped at 60 rows, while a search found every node.

**Decision.**
- A preset is the sampler **and the sRGB flag** (`FilterPreset::is_srgb`). New **Normal map** preset (after 3D): 3D's
  sampler (trilinear, Repeat) with sRGB off; every other preset, Default included, sets sRGB on. Expand grayscale
  still stays. `FilterPreset::of` matches both, so the file still stores the preset only through what it writes:
  3D's sampler on linear data reads back as Normal map.
- `fits_easy` follows: linear data under no sampler or under a sampler other than Normal map's matches no preset, so
  such a texture opens in Advanced (Easy disabled) instead of hiding its sRGB flag — e.g. the sample game's
  `rock_normal` (no sampler, sRGB off); `water_normal` (Linear ×3, Repeat, sRGB off) is Normal map.
- The palette lists all results (no row cap). Its rows are rebuilt only when the results change; moving the
  highlight with the arrows restyles the rows instead of respawning the whole list.

**Consequences.** Nothing in saved files changes; picking 3D, UI, Pixel art or Default on a linear texture now turns
sRGB on (one undo step). Decision 0028's "a preset keeps sRGB" no longer holds.
