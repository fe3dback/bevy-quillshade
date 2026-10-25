# 0010. v1 limitations and additional codes

- **E009** — unknown node kind / broken id / file load error; **E010** — generated WESL failed wesl+naga (with the pipeline variant named). All other codes follow spec §13 exactly.
- A function output of type `Texture2D` is not supported (E003): WESL does not allow returning textures from functions. Textures are passed into functions as inputs.
- User nodes (`shaders/.nodes`) have a single output.
- Static Switch / permutation parameters are out of v1; the model does not preclude them (node parameters are arbitrary `Value`s, `specialize` is already generated).
- A Sprite2d material is a `MaterialExtension2d` over `SpriteMaterial`; in the game the `SpriteMaterial(handle)` component is used next to `Sprite`.
- Uniform parameter struct: fields are sorted vec4 → vec3 (+ explicit pad) → vec2 → scalars, the size is a multiple of 16; the layout is computed once in the IR and checked by the generated `min_size` test. Binding 100 is the uniform, textures/samplers are 101/102, 103/104, …
- The Opacity Mask threshold is 0.3333 (as in UE).
- The render integration test (spec §15) with PSNR comparison is not automated: sample_game has a `SAMPLE_SCREENSHOT=out` mode that takes two screenshots and exits; CI without a GPU needs software Vulkan (lavapipe).
- Custom-node and user-node input names cannot be Rust/WGSL keywords (they are used verbatim in user WGSL; decision 0020).
