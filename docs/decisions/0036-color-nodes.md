# 0036. Color nodes (perceptual color spaces, adjustments, blend modes)

**Context.** A landscape material needs color variation of its tiling textures: shift the hue in some places
without changing the lightness, darken the mountains, mute the saturation, all driven by per-pixel masks (noise,
height, `WorldAlignedBlend`). Spec §8 only lists `Desaturation`, `Blend` (Overlay / Screen / Multiply / SoftLight)
and `BlackBody` under Utility, all working directly on linear values, where the same shift looks much stronger in
the darks than in the lights and a hue change also changes the perceived lightness.

**Decision.**
- **Category `Color`** (`crates/shader_stdlib/src/nodes/color.rs`, WESL in `wesl/color.wesl`). `Desaturation`,
  `Blend` and `BlackBody` move there from Utility (deviation from spec §8); their kinds stay the same, so saved
  graphs load unchanged.
- **Linear in, linear out.** Every color pin is `Float3` linear RGB (as a texture sample gives it); each node
  converts into its working space and back, so a graph never needs manual sRGB conversions. Every amount is an
  ordinary `Float` input, so a mask can drive it.
- **Working spaces** — perceptual, chosen by the user over raw linear math:
  - OKLab / OKLCh (Björn Ottosson, from linear sRGB) for hue, chroma and lightness: `HueShift` (rotation by
    `Shift` turns), `Saturation` (chroma scale), `Lightness` (`L * Scale + Offset`), `HueSaturationLightness`
    (the three in one conversion; lightness as a scale), `Colorize` (hue and chroma of `Tint`, lightness kept,
    blended by `Amount`), `Grayscale` (gray of the same L).
  - sRGB-encoded values (the IEC 61966-2-1 transfer function) for HSV / HSL — so numbers match color pickers —
    and for `Contrast` (around `Pivot`), `Gamma` (`pow(x, 1 / Gamma)`, above 1 brightens like the Levels midtone
    slider), `Levels`, `Desaturation` (its Luminance Factors now weight sRGB-encoded channels, as Rec. 601 luma
    weights are meant to), `Sepia`, `Invert`, `Posterize` and every `Blend` mode. `Exposure` multiplies the linear
    color by `2^Stops`.
- **Make / Break** for HSV, HSL, OKLab, OKLCh: three `Float` pins each (Break computes the triple once into a
  `n_<id>_v_` line and aliases its components, like a multi-output FunctionCall). Hues are in turns `0..1`
  (`frac` wraps them). No separate `Float3 → Float3` converters: AppendVector / BreakOutFloat3 cover that.
  `LinearToSRGB` / `SRGBToLinear` are there for data that arrives in the wrong space.
- **Blend** modes: Burn, Darken, Difference, Dodge, HardLight, Lighten, Multiply, Overlay, Screen, SoftLight
  (W3C formula; the old Pegtop one is gone). New `Opacity` input (default 1) mixes the result into the base, in
  sRGB-encoded values like Photoshop. The old linear behaviour of `Blend` and `Desaturation` is not kept (user
  decision): existing graphs regenerate with the new math.
- **Ranges.** Values above 1 pass through the conversions where the formula allows (sRGB transfer, OKLab);
  negatives clamp to 0, and so do out-of-gamut results of an OKLab → RGB conversion. `Levels` and `Posterize`
  clip to `0..1`; HSL is only meaningful in `0..1`.
