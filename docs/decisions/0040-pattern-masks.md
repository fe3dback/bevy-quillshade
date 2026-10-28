# 0040. Pattern masks (checkerboard, grid, blockout grid)

**Context.** Level blockouts need a prototype material: a world-space grid every meter and a faint checkerboard that
shows scale and orientation without UVs, floors told apart from walls. The starting point was a hand-written Bevy
extension shader (triplanar checkerboards, Ben Golus' "pristine grid", its own copy of the OKLab conversions). The
stdlib should offer it as one node and its parts as standalone nodes, reusing the existing stdlib functions.

**Decision.**
- New stdlib module `wesl/pattern.wesl` (`autogen/stdlib/pattern.wesl`). It is the first stdlib module importing
  another one: `import super::color::sg_lightness;` instead of a third copy of the OKLab math. Stdlib modules are always
  written together, so the import always resolves.
- Functions: `sg_triplanar_weights` (`|n|` normalized to a sum of 1), `sg_checker` (2D, 0 / 1, parity of
  `floor(uv / size)`, correct for negative coordinates), `sg_checker3` (triplanar: YZ, XZ, XY planes), `sg_pristine_grid`
  (Ben Golus, https://bgolus.medium.com/the-best-darn-grid-shader-yet-727f9278b9d8, rewritten with `select`; line
  width above 0.5 inverts), `sg_grid3` (triplanar of cells of `cell` world units), `sg_blockout_grid`.
- Nodes (Utility): `Checkerboard` (Size, Color A / B → `Mask`, `Color`), `Grid` (Cell Size, Line Width `Float2` in
  cells → mask), both with a **Mapping** param `Triplanar` (Position = world position, Normal = vertex normal when
  unconnected) or `UV` (`uv0` when unconnected; a UV grid divides the UV by Cell Size); `BlockoutGrid` (triplanar only:
  Position, Normal, Cell Size, Line Width, Floor / Wall / Line Color → `Float3` linear color).
- `sg_blockout_grid` keeps the source's look: floor color where `|n.y| >= 0.99`, wall color elsewhere; checkerboards
  of `cell / 10` and `cell` lifted to `0.1..1` and averaged into `k`, OKLab lightness offset `k * 0.2 - 0.2`
  (`sg_lightness`, so at most 0.18 darker); then `mix(shaded, line_color, grid)`. The source called
  `mix(grid, line_color, color)` — arguments swapped, the color used as the factor; the intent (lines over the
  checkered color) is implemented. The source's 1 m grid became Cell Size (default 1); its unused `cell_multiplier`
  is dropped. Result is clamped to non-negative RGB by `sg_oklab_to_rgb`.
- `Grid` and `BlockoutGrid` use `dpdx` / `dpdy`: wired into the vertex stage they report E009 like `DDX` / `DDY`.
