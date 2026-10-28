# 0039. Procedural masks (fractal noise, Voronoi, domain warp)

**Context.** A randomly generated landscape needs masks that do not exist as authored data, e.g. dirt paths between
patches of grass: there is no texture or spline to draw them from, the shader has to make them. Spec §8 lists only a
simple `Noise` (one octave of value / Perlin) under Utility, which is too smooth for dirt and cannot make cell edges.
The editor is not tied to one game, so the stdlib gets generic building blocks, not a "paths" node.

**Decision.**
- Three Utility nodes (`crates/shader_stdlib/src/nodes/utility.rs`, WESL in `wesl/noise.wesl`), all in 3D like
  `Noise`: Position defaults to the world position (seamless across meshes, no UVs), multiplied by Scale.
  - `FractalNoise`: params Function (Value / Perlin) and Type — `FBM` (sum of octaves), `Turbulence` (`|2n - 1|`
    octaves), `Ridged` (`(1 - |2n - 1|)²` octaves); inputs Octaves (`Int`, clamped to 1..12 in WESL so a wired value
    cannot make an unbounded loop), Lacunarity, Gain, Seed. Normalized by the sum of amplitudes, so `0..1`. Each
    octave is shifted by a constant so octaves do not line up at the origin.
  - `Voronoi`: inputs Randomness (jitter, saturated; 0 = regular grid) and Seed; outputs `F1`, `F2`, `Border`,
    `Cell`, in cell units. `sg_voronoi3` returns one `vec4` (`n_<id>_v_` line) whose components the outputs alias,
    like the Break color nodes (decision 0036). `Border` is the exact distance to the bisector planes (Inigo Quilez's
    second pass) instead of `F2 - F1`, so a threshold gives lines of equal width — what paths, cracks and tiles need.
    Both passes visit 3x3x3 cells (54 hashes per sample); the 5x5x5 second pass needed for exactness at full jitter
    would cost 125, the rare error it avoids is a slightly too large `Border` far from the nearest point.
  - `DomainWarp`: `p + (fbm3(p * Scale) * 2 - 1) * Strength`, three Perlin FBMs (Octaves, lacunarity 2, gain 0.5)
    with constant offsets per axis; output `Float3` for the Position of any noise node.
- Seed is a `Float` added as an offset of the lattice (`sg_seed_offset`): it needs no extra hash and keeps the
  pattern continuous in the seed; large seeds lose hash precision like large world positions do.
- `Noise` keeps its kind, pins and generated code (saved graphs and snapshots unchanged); it now shares only the
  Position fallback (`noise_position`).
- Not added: checker / brick / tile patterns and any game-specific mask node — they compose from these or come later.
