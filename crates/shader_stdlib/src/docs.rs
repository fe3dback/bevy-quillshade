//! Help texts of the stdlib nodes, shown in the properties pane of a selected node (`NodeInfo::description`).
//!
//! Markup of `editor_core::ui::rich_text`: `# Heading`, `- item`, `! important note`, `> example`, `**bold**`,
//! `` `code` ``; indentation is ignored. Convention: a one-line summary, then the inputs and outputs with their type
//! (`` `Float` ``) and expected range (**bold**), notes on pitfalls, and a short example where the use is not obvious.
//! ASCII only: the editor's UI font is an ASCII subset. Nodes whose behavior is obvious from the name (Add, Min,
//! MakeFloat3, ...) have no entry.

use shader_graph_core::registry::FUNCTION_CALL_KIND;

/// The help text of node `kind`, `""` when it has none.
pub fn doc(kind: &str) -> &'static str {
    DOCS.iter().find(|(k, _)| *k == kind).map_or("", |(_, d)| d)
}

/// `(kind, help text)` of every documented stdlib node.
pub const DOCS: &[(&str, &str)] = &[
    // --- Math --------------------------------------------------------------------------------------------------
    ("stdlib::Divide", "`A / B`, per component.\n! **B = 0** gives inf / NaN (black or flickering pixels)."),
    (
        "stdlib::Power",
        r#"`Base` raised to `Exp`, per component.
        ! A **negative Base** gives NaN (black or flickering pixels). Use `Abs` first or keep Base >= 0."#,
    ),
    (
        "stdlib::Fmod",
        r#"Remainder of `A / B` (WGSL `%`), per component.
        ! Keeps the **sign of A**: `-0.3 % 1 = -0.3`. For a wrap that is always **0..B** use `Frac(A / B) * B`."#,
    ),
    (
        "stdlib::Step",
        r#"Hard threshold: **1** where `X >= Y`, else **0**.
        # Inputs
        - **Y** `Float`: the edge (threshold)
        - **X** `Float`: the value tested
        # Output: same type as the inputs, **0 or 1**
        > e.g. Y 0.5, X = noise: a black and white mask."#,
    ),
    (
        "stdlib::ArcTan2",
        r#"Angle of the point (X, Y) from the +X axis.
        # Output: `Float` radians **-Pi..Pi**
        > e.g. `ArcTan2(uv.y - 0.5, uv.x - 0.5) / TwoPi + 0.5`: a 0..1 radial sweep."#,
    ),
    ("stdlib::Sqrt", "Square root.\n! A **negative** input gives NaN."),
    (
        "stdlib::Frac",
        r#"Fractional part `x - floor(x)`, always **0..1** (to just below 1).
        ! For negative input it counts up from the floor: `Frac(-0.25) = 0.75`.
        > e.g. `Frac(Time)`: a 0..1 saw wave, once per second."#,
    ),
    ("stdlib::Sign", r#"**-1**, **0** or **1** by the sign of each component."#),
    ("stdlib::Saturate", r#"Clamps to **0..1**, per component."#),
    ("stdlib::OneMinus", r#"`1 - x`: inverts a **0..1** mask or value."#),
    ("stdlib::Exp", r#"`e^x`. Grows fast: x = 10 is already ~22000."#),
    ("stdlib::Exp2", r#"`2^x`. +1 doubles the result, -1 halves it."#),
    ("stdlib::Log", "Natural logarithm.\n! Input must be **> 0**; 0 or below gives -inf / NaN."),
    ("stdlib::Log2", "Base-2 logarithm.\n! Input must be **> 0**; 0 or below gives -inf / NaN."),
    (
        "stdlib::Sin",
        r#"Sine of an angle in **radians**.
        # Output: **-1..1**
        > e.g. `Sin(Time * TwoPi) * 0.5 + 0.5`: 0..1, once per second."#,
    ),
    (
        "stdlib::Cos",
        r#"Cosine of an angle in **radians**.
        # Output: **-1..1**"#,
    ),
    ("stdlib::Tan", "Tangent of an angle in **radians**.\n! Infinite near **+-Pi/2**."),
    (
        "stdlib::ArcSin",
        r#"Inverse sine. Input **-1..1** (NaN outside).
        # Output: radians **-Pi/2..Pi/2**"#,
    ),
    (
        "stdlib::ArcCos",
        r#"Inverse cosine. Input **-1..1** (NaN outside).
        # Output: radians **0..Pi**
        > e.g. `ArcCos(DotProduct(a, b))` of unit vectors: the angle between them."#,
    ),
    (
        "stdlib::ArcTan",
        r#"Inverse tangent.
        # Output: radians **-Pi/2..Pi/2**
        For a full-circle angle use `ArcTan2`."#,
    ),
    (
        "stdlib::DDX",
        r#"How much the input changes to the **next pixel on screen** in X (WGSL `dpdx`).
        ! **Fragment stage only**: not in World Position Offset.
        > e.g. `Length(DDX(uv))`: UV change per pixel, for custom anti-aliasing."#,
    ),
    (
        "stdlib::DDY",
        r#"How much the input changes to the **next pixel on screen** in Y (WGSL `dpdy`).
        ! **Fragment stage only**: not in World Position Offset."#,
    ),
    (
        "stdlib::Round",
        r#"Rounds to the nearest whole number; halves go to the **even** neighbor (`2.5 -> 2`, `3.5 -> 4`).
        # Outputs
        - **Out**: same type as the input
        - **Int** `Int`: the **first component** only"#,
    ),
    (
        "stdlib::Truncate",
        r#"Drops the fraction, rounding toward **zero** (`-1.7 -> -1`; Floor gives -2).
        # Outputs
        - **Out**: same type as the input
        - **Int** `Int`: the **first component** only"#,
    ),
    (
        "stdlib::Lerp",
        r#"Linear blend from A to B: `A + (B - A) * Alpha`.
        # Inputs
        - **A**, **B**: any float type, or **Material Attributes** (blends every field)
        - **Alpha** `Float` (or A's type): **0** = A, **1** = B
        ! Alpha is **not clamped**: outside 0..1 it extrapolates. Saturate it if unsure."#,
    ),
    (
        "stdlib::SmoothStep",
        r#"Smooth 0 -> 1 ramp of Value between Min and Max (Hermite S-curve).
        # Inputs
        - **Min**, **Max**: the edges; Value <= Min gives 0, Value >= Max gives 1
        - **Value**: the value tested
        # Output: **0..1**
        ! Keep **Min < Max**; equal edges give undefined results.
        > e.g. Min 0.4, Max 0.6 on a noise: a soft-edged mask."#,
    ),
    (
        "stdlib::Remap",
        r#"Maps Value from **In Min..In Max** to **Out Min..Out Max** (linear).
        ! **Not clamped**: values outside the input range map outside the output range. In Min = In Max divides by zero.
        > e.g. In -1..1 -> Out 0..1 turns a `Sin` into a 0..1 wave."#,
    ),
    // --- Vector ------------------------------------------------------------------------------------------------
    (
        "stdlib::Normalize",
        r#"The vector scaled to **length 1**.
        ! A **zero vector** gives NaN. On a `Float` it returns the sign (-1, 0 or 1)."#,
    ),
    (
        "stdlib::NormalStrength",
        r#"Flattens or strengthens a **tangent-space** normal (from a normal map).
        # Inputs
        - **Normal** `Float3`: tangent space, **-1..1** (Z up)
        - **Strength** `Float`: **0** = flat, **1** = unchanged, **> 1** = stronger
        # Output: `Float3` unit tangent-space normal"#,
    ),
    (
        "stdlib::DotProduct",
        r#"Sum of the products of the components.
        For **unit vectors**: **1** = same direction, **0** = perpendicular, **-1** = opposite.
        > e.g. `DotProduct(VertexNormalWS, (0,1,0))`: 1 on floors, 0 on walls."#,
    ),
    (
        "stdlib::Reflect",
        r#"Mirrors Vector about the plane of Normal (WGSL `reflect`).
        # Inputs
        - **Vector** `Float3`: the incoming direction (toward the surface)
        - **Normal** `Float3`: must be **unit length**"#,
    ),
    (
        "stdlib::Refract",
        r#"Bends Vector through a surface (WGSL `refract`).
        # Inputs
        - **Vector** `Float3`: incoming direction, **unit length**
        - **Normal** `Float3`: **unit length**
        - **Eta** `Float`: ratio of indices of refraction, outside / inside (air -> glass ~**0.66**)
        # Output: `Float3`; **zero** on total internal reflection"#,
    ),
    (
        "stdlib::AppendVector",
        r#"Joins two values into a longer vector: `A`'s components, then `B`'s.
        > e.g. Float2 + Float -> Float3. At most 4 components in total."#,
    ),
    (
        "stdlib::ComponentMask",
        r#"Keeps the checked channels (R/G/B/A = X/Y/Z/W), in order.
        # Output: `Float` .. `Float4`, one component per checked channel
        ! Checking a channel the input does not have is an error."#,
    ),
    (
        "stdlib::Swizzle",
        r#"Reorders or repeats components by a pattern of 1-4 letters from **xyzw** or **rgba**.
        > e.g. `zyx` reverses a Float3, `xxx` spreads X, `ba` takes B then A."#,
    ),
    (
        "stdlib::TransformVector",
        r#"Converts a **direction** between spaces: Tangent, Local (object), World, View.
        ! Directions only (no translation). Tangent space needs mesh **tangents**.
        > e.g. Tangent -> World: a normal-map normal in world space."#,
    ),
    // --- Coordinates -------------------------------------------------------------------------------------------
    (
        "stdlib::TextureCoordinate",
        r#"Mesh UVs (channel 0 or 1) times Tiling.
        # Output: `Float2`, usually **0..1** across the mesh, **(0,0) = top-left** of a texture
        ! A mesh without that UV channel reads **(0,0)**. Sprite materials have one channel."#,
    ),
    (
        "stdlib::WorldPosition",
        r#"Position of the pixel (or of the vertex in World Position Offset).
        # Output: `Float3` in **world units** (meters), Y up"#,
    ),
    (
        "stdlib::VertexNormalWS",
        r#"Surface normal of the mesh in world space (before the material's Normal output).
        # Output: `Float3` unit vector, components **-1..1**"#,
    ),
    (
        "stdlib::CameraPositionWS",
        r#"Position of the camera.
        # Output: `Float3` in **world units**"#,
    ),
    (
        "stdlib::VertexTangentWS",
        r#"Mesh tangent in world space (the U direction of the UVs).
        # Output: `Float3`, components **-1..1**
        ! A mesh without tangents reads **(1,0,0)**."#,
    ),
    (
        "stdlib::VertexColor",
        r#"Per-vertex color of the mesh, interpolated.
        # Output: linear RGBA, usually **0..1**
        ! A mesh without vertex colors reads **white (1,1,1,1)**."#,
    ),
    (
        "stdlib::ObjectPosition",
        r#"Pivot (origin) of the object in the world.
        # Output: `Float3` in **world units**
        > e.g. per-object random: `Noise(ObjectPosition * 10)`."#,
    ),
    (
        "stdlib::ObjectScale",
        r#"Scale of the object along its local X, Y, Z axes.
        # Output: `Float3`, **1** = unscaled"#,
    ),
    (
        "stdlib::CameraVectorWS",
        r#"Direction **from the pixel toward the camera**.
        # Output: `Float3` unit vector
        > e.g. `DotProduct(normal, CameraVectorWS)`: 1 facing the camera, 0 at grazing angles."#,
    ),
    (
        "stdlib::ScreenPosition",
        r#"Where the pixel is on screen.
        # Outputs
        - **ViewportUV** `Float2`: **0..1**, (0,0) = top-left
        - **PixelPosition** `Float2`: in **pixels**
        ! **Fragment stage only**."#,
    ),
    (
        "stdlib::PixelDepth",
        r#"Distance of the pixel from the camera along the view direction.
        # Output: `Float` in **world units**, positive in front of the camera"#,
    ),
    (
        "stdlib::Panner",
        r#"Scrolls UVs over time: `Coordinate + Speed * Time`.
        # Inputs
        - **Coordinate** `Float2`: unconnected = UV0
        - **Time** `Float`: seconds; unconnected = Time
        - **Speed** `Float2`: UV units per second
        > e.g. Speed (0.1, 0): one full texture width every 10 s."#,
    ),
    (
        "stdlib::Rotator",
        r#"Rotates UVs around Center over time.
        # Inputs
        - **Coordinate** `Float2`: unconnected = UV0
        - **Time** `Float`: seconds; unconnected = Time
        - **Center** `Float2`: pivot in UV space (**0.5** = middle)
        - **Speed** `Float`: **radians** per second (TwoPi = 1 turn/s)
        > e.g. a constant 1 in Time: a fixed rotation of Speed radians."#,
    ),
    (
        "stdlib::ScaleUVsByCenter",
        r#"Scales UVs around the middle (0.5, 0.5).
        ! **> 1 zooms out** (the texture looks smaller and repeats), **< 1 zooms in**."#,
    ),
    // --- Constants ---------------------------------------------------------------------------------------------
    (
        "stdlib::Time",
        r#"Time since the app started.
        # Output: `Float` in **seconds**
        ! **Wraps to 0 after 1 hour** (Bevy globals): long animations jump once an hour."#,
    ),
    ("stdlib::DeltaTime", "Duration of the last frame.\n# Output: `Float` in **seconds** (~0.016 at 60 fps)"),
    (
        "stdlib::ViewSize",
        r#"Size of the camera's viewport.
        # Output: `Float2` in **pixels**"#,
    ),
    (
        "stdlib::Toggle",
        r#"A checkbox as a number.
        # Output: `Float` **0.0** (unchecked) or **1.0** (checked)
        For branchless math (`Multiply`, `Lerp` alpha). For an `If` branch use `CompileBool`; for `RuntimeIf` use `RuntimeBool`."#,
    ),
    (
        "stdlib::RuntimeBool",
        r#"A constant `true` / `false` for GPU logic.
        # Output: `RuntimeBool` (`RuntimeIf`, `And`, `Or`, `Not`)
        For a 0 / 1 number use `Toggle`; to remove a branch from the shader use `CompileBool` with `If`."#,
    ),
    (
        "stdlib::CompileBool",
        r#"A flag decided when the shader is **generated**, not on the GPU.
        # Output: `CompileBool` (`If`, `CompileAnd`, `CompileOr`, `CompileNot`)
        The `If` branch it disables is **left out of the shader** entirely.
        To expose the flag on the material, use `CompileBoolParameter`."#,
    ),
    (
        "stdlib::GlobalConstF32",
        r#"An `f32` constant of a project constant table (`.consts.ron`), inlined into the shader.
        Pick the Table and the Constant in Parameters.
        ! Nothing selected: outputs **1.0**."#,
    ),
    (
        "stdlib::GlobalConstVec2",
        r#"A `vec2` constant of a project constant table, inlined into the shader.
        ! Nothing selected: outputs **(1, 1)**."#,
    ),
    (
        "stdlib::GlobalConstVec3",
        r#"A `vec3` constant (often a color) of a project constant table, inlined into the shader.
        ! Nothing selected: outputs **(1, 1, 1)**."#,
    ),
    (
        "stdlib::GlobalConstVec4",
        r#"A `vec4` constant of a project constant table, inlined into the shader.
        ! Nothing selected: outputs **(1, 1, 1, 1)**."#,
    ),
    (
        "stdlib::GlobalToggle",
        r#"A `toggle` constant of a project constant table.
        # Output: `Float` **0.0** or **1.0**, for branchless math (`Multiply`, `Lerp`)
        To remove a branch use `GlobalCompileBool`; for `RuntimeIf` use `GlobalRuntimeBool`.
        ! Nothing selected: outputs **1.0**."#,
    ),
    (
        "stdlib::GlobalRuntimeBool",
        r#"A `runtimeBool` constant of a project constant table.
        # Output: `RuntimeBool` (`RuntimeIf`, `And`, `Or`, `Not`)
        ! Nothing selected: outputs **true**."#,
    ),
    (
        "stdlib::GlobalCompileBool",
        r#"A `compileBool` flag of a project constant table, decided when the shader is generated.
        # Output: `CompileBool` for `If`: the disabled branch is **left out of the shader**
        ! Nothing selected: outputs **true**."#,
    ),
    // --- Parameters --------------------------------------------------------------------------------------------
    (
        "param::Scalar",
        r#"A number the game can change at runtime (field of the generated material struct).
        # Output: `Float`
        Name: the Rust field name. Slider Min / Max only limit the editor slider.
        ! Renaming, adding or removing a parameter changes the material layout: the game must be **recompiled**."#,
    ),
    (
        "param::Vector",
        r#"A vector the game can change at runtime (field of the generated material struct).
        # Output: `Float2`, `Float3` or `Float4` (Type)
        ! For colors prefer `ColorParameter`: it gets a color picker and linear color handling."#,
    ),
    (
        "param::Color",
        r#"A color the game can change at runtime (`LinearRgba` field of the generated material struct).
        # Outputs
        - **RGB** `Float3`, **R**/**G**/**B**/**A** `Float`, **RGBA** `Color`
        Values are **linear**, usually **0..1** (above 1 for HDR / emissive)."#,
    ),
    (
        "param::RuntimeBool",
        r#"A switch the game can flip at runtime without recompiling (`bool` field; `u32` on the GPU).
        # Output: `RuntimeBool` for `RuntimeIf`, `And`, `Or`, `Not`
        ! **Both branches** of a `RuntimeIf` still run on the GPU. For a feature switch that removes code, use `CompileBoolParameter`."#,
    ),
    (
        "param::CompileBool",
        r#"A named flag decided when the shader is **generated**; uses no GPU memory.
        # Output: `CompileBool` for `If`: the disabled branch is **left out of the shader**
        ! The game **cannot** flip it at runtime. For that use `RuntimeBoolParameter` with `RuntimeIf`."#,
    ),
    (
        "param::TextureObject",
        r#"A texture the game sets (`Handle<Image>` field), not sampled yet.
        # Output: `Texture2D` for `TextureSample`, `TextureSize`, function inputs, ...
        Default Texture: used until the game sets one."#,
    ),
    (
        "param::TextureSample2D",
        r#"A texture the game sets (`Handle<Image>` field), sampled at UVs.
        # Inputs
        - **UVs** `Float2`: unconnected = UV0
        - **Mip** `Float`: only with MipValueMode Bias / Level
        # Outputs: **RGB**, **R**, **G**, **B**, **A**, **RGBA**
        Colors are **linear** for sRGB textures, raw values otherwise. Sampler Type **Normal** remaps to **-1..1**."#,
    ),
    // --- Texture -----------------------------------------------------------------------------------------------
    (
        "stdlib::TextureSample",
        r#"Reads a texture at UVs.
        # Inputs
        - **Tex** `Texture2D`: optional; when connected it **replaces** the Texture property
        - **UVs** `Float2`: unconnected = UV0; outside 0..1 follows the sampler's address mode
        - **Mip** `Float`: only with MipValueMode Bias / Level
        # Outputs: **RGB**, **R**, **G**, **B**, **A**, **RGBA**
        Color (sRGB) textures read as **linear** color **0..1**; data textures (`is_srgb: false`) as stored.
        ! For normal maps set Sampler Type to **Normal**: the result is remapped from 0..1 to **-1..1**.
        No texture selected: the default texture (warning W110)."#,
    ),
    (
        "stdlib::TextureObject",
        r#"A Texture asset of the project, not sampled yet.
        # Output: `Texture2D` for `TextureSample` (Tex), `TextureSize`, `FlipBook`, function inputs, ..."#,
    ),
    (
        "stdlib::TextureSize",
        r#"Width and height of a texture.
        # Output: `Float2` in **pixels** (mip 0)"#,
    ),
    (
        "stdlib::TexelSize",
        r#"Size of one pixel of a texture in UV units: `1 / TextureSize`.
        > e.g. UV + (TexelSize.x, 0): the neighboring pixel, for edge detection or blur."#,
    ),
    (
        "stdlib::FlipBook",
        r#"Plays an atlas of animation frames (sprite sheet).
        # Inputs
        - **Texture** `Texture2D`: the atlas
        - **Rows**, **Columns** `Float`: grid of frames; frames go left to right, top to bottom
        - **Phase** `Float`: **0..1** = first..last frame, wraps; unconnected = `Frac(Time)` (one loop per second)
        - **UVs** `Float2`: UVs of one frame (0..1); unconnected = UV0
        # Outputs: the sample (**RGB** ... **RGBA**) and **UV**, the atlas UV used
        > e.g. 16 frames at 8 fps: Phase = `Frac(Time * 8 / 16)`."#,
    ),
    (
        "stdlib::NormalFromHeightmap",
        r#"Builds a normal from a height texture (its **red** channel) by comparing neighboring pixels.
        # Inputs
        - **Heightmap** `Texture2D`: height in **R**, 0..1
        - **UVs** `Float2`: unconnected = UV0
        - **Height Scale** `Float`: bump strength (1 = subtle, 10+ = strong)
        # Output: `Float3` unit **tangent-space** normal, ready for the material Normal"#,
    ),
    (
        "stdlib::SpriteTexture",
        r#"The sprite's own image at UVs (flip, slicing and atlas already applied), **without** its tint color.
        # Output: **RGB** ... **RGBA**, linear
        ! **2D (Sprite) materials only**."#,
    ),
    // --- Scene -------------------------------------------------------------------------------------------------
    (
        "stdlib::SceneDepth",
        r#"Depth of the opaque scene behind this pixel (or at a viewport UV).
        # Inputs
        - **UV** `Float2`: viewport UV **0..1**; unconnected = this pixel
        # Outputs
        - **Depth** `Float`: distance along the view direction in **world units** (like PixelDepth)
        - **Raw** `Float`: depth buffer value, reverse Z: **1** = near, **0** = far
        ! Needs **DepthPrepass** on the camera; without it reads as infinitely far (Raw 0).
        > e.g. `SceneDepth.Depth - PixelDepth`: thickness of water in front of the ground."#,
    ),
    (
        "stdlib::DepthFade",
        r#"Fades a surface out where it meets geometry behind it (soft particles, shorelines).
        # Inputs
        - **Opacity** `Float`: the opacity to fade
        - **Fade Distance** `Float`: in **world units**
        # Output: `Float` **0** at the intersection, rising to **Opacity** at Fade Distance
        ! Use in a **translucent** material, into Opacity. Needs **DepthPrepass** on the camera; without it outputs Opacity."#,
    ),
    // --- Color -------------------------------------------------------------------------------------------------
    (
        "stdlib::MakeHSV",
        r#"Color from hue, saturation and value (like a color picker).
        # Inputs: `Float` each
        - **Hue**: **0..1** turns (not degrees), wraps; 0 = red, 1/3 = green, 2/3 = blue
        - **Saturation**: **0..1**, 0 = gray
        - **Value**: **0..1**, 0 = black
        # Output: `Float3` **linear** RGB"#,
    ),
    (
        "stdlib::BreakHSV",
        r#"Hue, saturation and value of a color (like a color picker).
        # Input: `Float3` **linear** RGB
        # Outputs: `Float` each
        - **Hue**: **0..1** turns (0 = red)
        - **Saturation**: **0..1**
        - **Value**: **0..1** (above 1 for HDR colors)"#,
    ),
    (
        "stdlib::MakeHSL",
        r#"Color from hue, saturation and lightness.
        # Inputs: `Float` each
        - **Hue**: **0..1** turns (not degrees), wraps; 0 = red
        - **Saturation**: **0..1**
        - **Lightness**: **0..1**; 0 = black, **0.5 = pure color**, 1 = white
        # Output: `Float3` **linear** RGB"#,
    ),
    (
        "stdlib::BreakHSL",
        r#"Hue, saturation and lightness of a color.
        # Input: `Float3` **linear** RGB
        # Outputs: **Hue** (turns), **Saturation**, **Lightness**, `Float` **0..1** each"#,
    ),
    (
        "stdlib::MakeOKLab",
        r#"Color from OKLab: a perceptual space where equal steps look equally different.
        # Inputs: `Float` each
        - **L**: perceived lightness **0..1**
        - **A**: green (-) .. red (+), about **-0.4..0.4**
        - **B**: blue (-) .. yellow (+), about **-0.4..0.4**
        # Output: `Float3` **linear** RGB; out-of-gamut channels clamp at 0"#,
    ),
    (
        "stdlib::BreakOKLab",
        r#"OKLab components of a color (perceptual: equal steps look equally different).
        # Input: `Float3` **linear** RGB
        # Outputs: **L** **0..1**, **A** and **B** about **-0.4..0.4**"#,
    ),
    (
        "stdlib::MakeOKLCh",
        r#"Color from OKLCh, the polar form of OKLab: hue changes keep the perceived brightness.
        # Inputs: `Float` each
        - **L**: perceived lightness **0..1**
        - **Chroma**: colorfulness **0..~0.37**, 0 = gray
        - **Hue**: **0..1** turns (not degrees), wraps
        # Output: `Float3` **linear** RGB; out-of-gamut channels clamp at 0"#,
    ),
    (
        "stdlib::BreakOKLCh",
        r#"OKLCh components of a color.
        # Input: `Float3` **linear** RGB
        # Outputs: **L** **0..1**, **Chroma** **0..~0.37**, **Hue** **0..1** turns"#,
    ),
    (
        "stdlib::LinearToSRGB",
        r#"Encodes a **linear** color with the sRGB curve (the values stored in a PNG / shown by a picker).
        # Output: `Float3` **sRGB-encoded**
        ! Material outputs expect **linear** color: use this only for math on encoded values."#,
    ),
    (
        "stdlib::SRGBToLinear",
        r#"Decodes **sRGB-encoded** values (from a color picker or a non-sRGB texture) to linear color.
        # Output: `Float3` **linear**, ready for material outputs"#,
    ),
    (
        "stdlib::HueShift",
        r#"Rotates the hue, keeping lightness and colorfulness (OKLCh).
        - **Shift** `Float`: **turns**: 0.5 = opposite hue, 1 = unchanged (wraps)"#,
    ),
    (
        "stdlib::Saturation",
        r#"Scales colorfulness (OKLCh chroma), keeping hue and lightness.
        - **Saturation** `Float`: **0** = gray, **1** = unchanged, **2** = double"#,
    ),
    (
        "stdlib::Lightness",
        r#"Changes perceived lightness: `L * Scale + Offset` (OKLab L, 0..1), keeping hue and colorfulness.
        - **Scale**: **1** = unchanged
        - **Offset**: **-1..1**, added after scaling"#,
    ),
    (
        "stdlib::HueSaturationLightness",
        r#"Hue shift, saturation and lightness in one step (OKLCh).
        - **Hue Shift**: **turns**, 0.5 = opposite
        - **Saturation**: **1** = unchanged, 0 = gray
        - **Lightness**: scale, **1** = unchanged"#,
    ),
    (
        "stdlib::Colorize",
        r#"Tints a color: takes hue and colorfulness from Tint, keeps the original lightness (OKLab).
        - **Amount** `Float`: **0** = original, **1** = fully tinted"#,
    ),
    (
        "stdlib::Contrast",
        r#"Pushes values away from (or toward) Pivot, on sRGB-encoded values like an image editor.
        - **Amount**: **1** = unchanged, 0 = flat gray at Pivot, 2 = strong
        - **Pivot**: **0..1**, the value that stays put (0.5 = mid gray)"#,
    ),
    (
        "stdlib::Gamma",
        r#"Brightens or darkens the midtones, keeping black and white (on sRGB-encoded values).
        - **Gamma**: **1** = unchanged, **> 1 brighter**, < 1 darker"#,
    ),
    (
        "stdlib::Levels",
        r#"Image-editor Levels on sRGB-encoded values.
        - **In Black** / **In White**: input range **0..1**; values outside are clipped
        - **Gamma**: midtones, **> 1 brighter**
        - **Out Black** / **Out White**: output range **0..1**
        ! The result is **clamped to 0..1**: HDR values above 1 are lost."#,
    ),
    (
        "stdlib::Exposure",
        r#"Brightens or darkens in photographic stops: `Color * 2^Stops`.
        - **Stops**: **+1** = twice as bright, **-1** = half, 0 = unchanged
        Not clamped (HDR stays HDR)."#,
    ),
    ("stdlib::Grayscale", "Gray of the same **perceived** lightness (OKLab L).\n# Output: `Float3` **linear** gray"),
    (
        "stdlib::Desaturation",
        r#"Blends toward gray, weighting R, G, B by Luminance Factors (on sRGB-encoded values).
        - **Fraction**: **0** = original, **1** = fully gray
        For gray by perceived lightness use `Grayscale`."#,
    ),
    ("stdlib::Sepia", r#"Old-photo brown tone. **Amount** **0..1**: 0 = original, 1 = full sepia."#),
    (
        "stdlib::Invert",
        r#"Photo negative: `1 - color` on sRGB-encoded values.
        ! HDR values above 1 become **black**."#,
    ),
    (
        "stdlib::Posterize",
        r#"Reduces each channel to a few flat levels (toon / retro look).
        - **Steps**: levels per channel, **at least 2**
        ! Clamps to **0..1** first: HDR values above 1 are lost."#,
    ),
    (
        "stdlib::Blend",
        r#"Image-editor blend mode of **Blend** over **Base** (computed on sRGB-encoded values, like an image editor).
        - **Base**, **Blend** `Float3`: **linear** colors **0..1**
        - **Opacity** `Float`: **0** = Base, **1** = full effect
        # Output: `Float3` **linear**"#,
    ),
    (
        "stdlib::SampleGradient",
        r#"Color of a gradient of a gradient collection (`.gradients.ron`) at T.
        # Inputs
        - **T** `Float`: position **0..1** along the gradient (clamped)
        # Outputs: **RGB** `Float3` **linear**, **A** `Float`, **RGBA** `Float4`
        ! No gradient selected: outputs **white** (warning W111).
        > e.g. T = a noise mask: noise colored by a palette."#,
    ),
    (
        "stdlib::BlackBody",
        r#"Color of a glowing hot object at a temperature.
        - **Temp** `Float`: **Kelvin**, clamped to **1000..40000**; ~1900 = candle, 6500 = white, 10000+ = blue
        # Output: `Float3` **linear 0..1** (hue only; multiply for brightness)"#,
    ),
    // --- Material Attributes -----------------------------------------------------------------------------------
    (
        "stdlib::MakeMaterialAttributesPbr",
        r#"Packs the PBR inputs of a layer into one **Material Attributes** value, to blend layers with `Lerp` or `If`.
        Unconnected fields use the pin defaults (Base Color white, Roughness 0.5, Normal (0,0,1), ...).
        # Output: `MaterialAttributes (PBR)` for a material output in **Wrapped** mode"#,
    ),
    (
        "stdlib::BreakMaterialAttributesPbr",
        r#"Splits a **Material Attributes** value into its PBR fields (Base Color, Metallic, ..., Normal)."#,
    ),
    // --- Output ------------------------------------------------------------------------------------------------
    (
        "output::Pbr3d",
        r#"The result of a lit 3D material (`StandardMaterial` extension).
        # Inputs
        - **Base Color** `Float3`: **linear** color 0..1
        - **Metallic**, **Roughness** `Float`: **0..1**
        - **Specular** `Float`: reflectance **0..1**, 0.5 = 4% (most non-metals)
        - **Emissive Color** `Float3`: **linear**, above 1 for glow (bloom)
        - **Opacity** / **Opacity Mask** `Float`: **0..1**, by Blend Mode (Mask cuts below **0.333**)
        - **Normal** `Float3`: **tangent space** -1..1 (from a normal map with Sampler Type Normal)
        - **World Position Offset** `Float3`: **world units**, moves vertices
        - **World Normal** `Float3`: replaces the vertex normal (vertex stage)
        - **Ambient Occlusion** `Float`: **0..1**, 1 = no occlusion
        Unconnected pins keep the `StandardMaterial` values. Greyed pins are unused with the current settings."#,
    ),
    (
        "output::Unlit3d",
        r#"The result of an unlit 3D material (no lighting).
        # Inputs
        - **Color** `Float3`: **linear**, shown as is (above 1 for bloom)
        - **Opacity** `Float`: **0..1**, translucent blend modes
        - **Opacity Mask** `Float`: Mask mode, pixels below **0.333** are cut
        - **World Position Offset** `Float3`: **world units**"#,
    ),
    (
        "output::Sprite2d",
        r#"The result of a 2D sprite material.
        # Inputs
        - **Color** `Float4`: **linear** RGBA; unconnected = the sprite's own color (image * tint)
        - **UV override** `Float2`: UVs for the sprite image; unconnected = the sprite's UVs"#,
    ),
    (
        "output::FunctionInput",
        r#"An input pin of this function, as seen by the nodes calling it.
        Name and Type define the pin; Default is used when the caller leaves it unconnected (unless **Required**).
        Sort Priority orders the pins; Slider Min / Max set the caller's slider range (Float only)."#,
    ),
    (
        "output::FunctionOutput",
        r#"An output pin of this function. Name and Type define the pin; Sort Priority orders the pins."#,
    ),
    // --- Utility -----------------------------------------------------------------------------------------------
    (
        "stdlib::If",
        r#"Chooses True or False when the shader is **generated**: the other branch is **left out** of the shader (with its textures and parameters).
        # Inputs
        - **Condition** `CompileBool`: `CompileBool`, `CompileBoolParameter`, `GlobalCompileBool`, ...
        - **True**, **False**: any type, also **Material Attributes**
        ! Free on the GPU, but cannot change at runtime. For a runtime switch use `RuntimeIf`."#,
    ),
    (
        "stdlib::RuntimeIf",
        r#"Chooses True or False **on the GPU**, per pixel (WGSL `select`).
        # Inputs
        - **Condition** `RuntimeBool`: `Compare`, `RuntimeBoolParameter`, `And`, ...
        - **True**, **False**: any float type, also **Material Attributes**
        ! **Both branches are always computed**: it saves nothing. For a fixed choice use `If`."#,
    ),
    (
        "stdlib::RuntimeCompareIf",
        r#"Compares A and B on the GPU and outputs one of three values.
        - **A > B**, **A == B**, **A < B**: the outputs per case
        Equal means `|A - B| <= Equals Threshold` (parameter).
        ! **All three inputs are always computed**."#,
    ),
    (
        "stdlib::CompileAnd",
        r#"`A && B` of two `CompileBool` flags, decided when the shader is generated (for `If`).
        For `RuntimeBool` values use `And`."#,
    ),
    (
        "stdlib::CompileOr",
        r#"`A || B` of two `CompileBool` flags, decided when the shader is generated (for `If`).
        For `RuntimeBool` values use `Or`."#,
    ),
    (
        "stdlib::CompileNot",
        r#"`!A` of a `CompileBool` flag, decided when the shader is generated (for `If`).
        For a `RuntimeBool` use `Not`."#,
    ),
    (
        "stdlib::And",
        r#"`A && B` of two `RuntimeBool` values, on the GPU (for `RuntimeIf`).
        ! Cannot drive `If`; for `CompileBool` flags use `CompileAnd`."#,
    ),
    (
        "stdlib::Or",
        r#"`A || B` of two `RuntimeBool` values, on the GPU (for `RuntimeIf`).
        ! Cannot drive `If`; for `CompileBool` flags use `CompileOr`."#,
    ),
    (
        "stdlib::Not",
        r#"`!A` of a `RuntimeBool` value, on the GPU (for `RuntimeIf`).
        ! Cannot drive `If`; for a `CompileBool` flag use `CompileNot`."#,
    ),
    (
        "stdlib::Compare",
        r#"Compares two numbers on the GPU (Operation: `>`, `>=`, `<`, `<=`, `==`, `!=`).
        # Output: `RuntimeBool` for `RuntimeIf`, `And`, `Or`, `Not`
        ! `==` on computed floats is rarely exact; prefer `RuntimeCompareIf` with a threshold."#,
    ),
    (
        "stdlib::Fresnel",
        r#"Rim factor: bright where the surface is seen at a **grazing angle**, dark where it faces the camera.
        # Inputs
        - **Exponent** `Float`: rim width; higher = thinner rim (5 = physically based)
        - **Base Reflect Fraction** `Float`: **0..1**, the value when facing the camera
        - **Normal** `Float3`: world space, unit; unconnected = vertex normal
        # Output: `Float` **Base..1**
        > e.g. `Fresnel * color` into Emissive: a glowing rim."#,
    ),
    (
        "stdlib::WorldAlignedBlend",
        r#"Mask of surfaces facing **Up** (snow on top, moss on ledges).
        # Inputs
        - **Blend Sharpness** `Float`: higher = harder edge
        - **Blend Bias** `Float`: **-1..1**; lower = only flatter surfaces
        - **Normal** `Float3`: world space; unconnected = vertex normal
        - **Up** `Float3`: the direction to compare with (default +Y)
        # Outputs
        - **Mask** `Float`: **1** facing Up .. **0** steep
        - **Angle** `Float`: slope in **degrees**, 0 = facing Up, 90 = wall"#,
    ),
    (
        "stdlib::Noise",
        r#"Smooth 3D noise at `Position * Scale`.
        # Inputs
        - **Position** `Float3`: world units; unconnected = world position
        - **Scale** `Float`: frequency; 1 = one cell per unit
        # Output: `Float` **0..1**
        ! Centered on **0.5**, not 0. For signed noise use `x * 2 - 1` (**-1..1**).
        Function: **Value** = blurry cells, **Perlin** = smoother, rarely reaches 0 or 1.
        > e.g. Scale 0.1: blobs about 10 units wide."#,
    ),
    (
        "stdlib::FractalNoise",
        r#"Several layers (octaves) of noise added together: large shapes plus finer detail.
        # Inputs
        - **Position** `Float3`: world units; unconnected = world position
        - **Scale** `Float`: frequency of the first octave
        - **Octaves** `Int`: **1..12** layers; more = finer detail, slower
        - **Lacunarity** `Float`: frequency step per octave (**2** = each twice as fine)
        - **Gain** `Float`: **0..1**, strength step per octave (0.5 = each half as strong)
        - **Seed** `Float`: any number; each value gives a different pattern
        # Output: `Float` **0..1**
        Type: **FBM** = clouds (centered on ~0.5), **Turbulence** = billows, **Ridged** = sharp ridges (mountains)."#,
    ),
    (
        "stdlib::Voronoi",
        r#"Cellular (Worley) noise: random cells, like stones, scales or cracks.
        # Inputs
        - **Position** `Float3`: world units; unconnected = world position
        - **Scale** `Float`: cells per unit
        - **Randomness** `Float`: **0** = regular grid, **1** = fully random
        - **Seed** `Float`: any number; each value gives a different pattern
        # Outputs: `Float`, in **cell units** (1 = one cell)
        - **F1**: distance to the nearest cell center, **0..~1**
        - **F2**: distance to the second nearest
        - **Border**: distance to the cell edge, **0** on the edge
        - **Cell**: a random value per cell, **0..1**
        > e.g. `Step(0.05, Border)`: black cell outlines; Cell into `SampleGradient`: colored tiles."#,
    ),
    (
        "stdlib::DomainWarp",
        r#"Wobbles a position with noise, to make other noise look swirly or organic.
        # Inputs
        - **Position** `Float3`: unconnected = world position
        - **Scale** `Float`: frequency of the warping noise
        - **Strength** `Float`: how far points move, in **Position units**
        - **Octaves** `Int`: **1..12**, detail of the warp
        - **Seed** `Float`: different warp per value
        # Output: `Float3` the moved position
        > e.g. DomainWarp -> Position of `Noise` or `Voronoi`."#,
    ),
    (
        "stdlib::Checkerboard",
        r#"Checkerboard of Size squares (cubes in Triplanar mapping).
        # Inputs
        - **Position** / **Normal**: Triplanar mapping (world space); unconnected = the mesh's
        - **UVs** `Float2`: UV mapping; unconnected = UV0
        - **Size** `Float`: square size in world units (or UV units)
        - **Color A** / **Color B** `Float3`
        # Outputs
        - **Mask** `Float` **0 or 1** (blended on triplanar seams)
        - **Color** `Float3`: Color A where Mask is 0, B where 1"#,
    ),
    (
        "stdlib::Grid",
        r#"Anti-aliased grid lines that stay crisp at any distance.
        # Inputs
        - **Position** / **Normal**: Triplanar mapping (world space); **UVs**: UV mapping
        - **Cell Size** `Float`: distance between lines
        - **Line Width** `Float2`: fraction of a cell, per axis (**0.02** = thin)
        # Output: `Float` **1 on the lines**, 0 between
        ! **Fragment stage only**."#,
    ),
    (
        "stdlib::BlockoutGrid",
        r#"Ready-made prototype (greybox) surface: floors and walls in two colors with a checker and grid lines.
        # Inputs
        - **Cell Size** `Float`: grid spacing in **world units**
        - **Line Width** `Float2`: fraction of a cell
        - **Floor Color** (surfaces facing up / down), **Wall Color**, **Line Color** `Float3`
        # Output: `Float3` **linear** color, for Base Color
        ! **Fragment stage only**."#,
    ),
    (
        FUNCTION_CALL_KIND,
        r#"Calls a function of the project (`.function.ron`); its pins are the function's inputs and outputs."#,
    ),
    (
        "stdlib::Custom",
        r#"Your own WESL code as a node.
        - **Inputs**: `name: Type` pairs, comma separated (`a: Float3, t: Float, tex: Texture2D`)
        - **Output Type**: the result's type
        - **Code**: a function body with `return ...;`, or a single expression
        > e.g. Inputs `a: Float3, b: Float`, Code `return a * b;`"#,
    ),
    (
        "stdlib::NamedRoute",
        r#"Names its input so **Named Route Usage** nodes can output it anywhere in this graph, without a long wire."#,
    ),
    ("stdlib::NamedRouteUsage", r#"Outputs the input of the chosen **Named Route** (Route parameter)."#),
    (
        "stdlib::SampleCurve",
        r#"Value of a Curve asset (`.curve.ron`) at T.
        # Inputs
        - **T** `Float`: position along the curve, usually **0..1** (outside: the end values)
        # Output: `Float` the curve's value (its own range, see the curve editor)
        Type: **Simple** = straight lines between points, **Catmull-Rom** = smooth.
        > e.g. T = a mask, to reshape its falloff."#,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn docs_name_registered_nodes_once() {
        let r = crate::registry();
        let mut seen = BTreeSet::new();
        for (kind, d) in DOCS {
            assert!(r.get(kind).is_some(), "documented node `{kind}` is not registered");
            assert!(seen.insert(*kind), "`{kind}` is documented twice");
            assert!(!d.trim().is_empty(), "`{kind}` has an empty help text");
        }
    }

    /// The editor's UI font is an ASCII subset (`editor_core::ui::rich_text`).
    #[test]
    fn docs_are_ascii() {
        for (kind, d) in DOCS {
            assert!(d.is_ascii(), "help text of `{kind}` has non-ASCII characters");
        }
    }

    #[test]
    fn registered_nodes_get_their_docs() {
        let r = crate::registry();
        assert_eq!(r.get("stdlib::Noise").unwrap().info().description, doc("stdlib::Noise"));
    }
}
