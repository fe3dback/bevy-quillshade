# 0020. Prefixed identifiers in generated WESL and Rust; shape-only user names

**Context.** QA feedback: a parameter named `type`, a material named `loop` or a `mesh/` directory were rejected
with E005 (Rust/WGSL keywords and names used by generated code), and a few other checks existed only because user
names went into generated code verbatim: a parameter named like a called function, a material `env.material.ron`
next to an `env/` directory (Rust module tree), materials in top-level `shaders/stdlib/`, `functions/`, `curves/`
(they would land in the generated `autogen/stdlib/`, … directories). Spec §4 of the QA design asks to always
prefix user names in generated code instead.

**Decision.**
- Every user name is emitted with a static prefix that depends on its role (`shader_graph_core::ident`; nothing
  else spells the prefixes):

  | What | Prefix | WESL | Rust |
  | --- | --- | --- | --- |
  | material parameter (uniform field, texture + sampler) | `inp_` | `params.inp_wetness`, `inp_albedo`, `inp_albedo_sampler` | field `inp_wetness`, argument `new(inp_albedo, ..)`, `set_inp_wetness` |
  | function | `fun_` | `fn fun_wet_mask(..)`, `import ..::fun_wet_mask::{WetMaskOut, fun_wet_mask}` | — |
  | function input | `inp_` | `inp_strength` (texture: `inp_t`, `inp_t_sampler`) | — |
  | function output (result struct field) | `out_` | `WetMaskOut { out_mask: f32 }`, `.out_mask` | — |
  | directory | `dir_` | `autogen/dir_env/…`, `super::dir_common::…` | `pub mod dir_env;` |
  | material file / module | `mat_` | `autogen/dir_env/mat_rock_wet.wesl` | `src/materials/dir_env/mat_rock_wet.rs` |
  | curve file / module | `crv_` | `autogen/curves/dir_common/crv_pulse.wesl` | — |

  Path components come from `shader_codegen_wesl::paths` (`material_components`, `material_module`,
  `function_module`, `curve_module`); the WESL asset path (`SHADER_PATH`), the Rust module tree, the generated-file
  tabs and orphan detection all follow from it.
- Prefixes are **static** (always applied, not only on a clash): output is predictable, nothing changes when a
  name becomes a keyword in a later Bevy/WGSL/Rust version, and no name can collide with what the generator itself
  introduces (`n_{id}_{pin}`, `b_*` builtins, `sg_*` helpers, `params`, `pbr_input`, `_padN`, `stdlib/`, `functions/`,
  `curves/`), none of which starts with a user prefix.
- Kept unprefixed: Rust type names (`RockWetExt`, `RockWetMaterial`, `RockWetParams`, function result structs
  `WetMaskOut`) — PascalCase with a suffix, never a keyword — and the root re-exports
  (`pub use dir_env::mat_rock_wet::{RockWetExt, RockWetMaterial, RockWetParams};`), so game code mostly sees no
  module prefixes. The model and IR keep raw names: pin ids, `MaterialParam::name`, `FnPort::name`,
  `FunctionSig::name`, `IrStage::outputs` keys; only emitted text is prefixed (`UniformField::name` is emitted text).
- Names are shape-checked only (`ident::check_ident`: `[a-z][a-z0-9_]*`, no `__`, no trailing `_`) for assets,
  directories, material parameters, function ports and global constants (inlined as literals, never emitted as
  identifiers).
- **Exception:** input names of `stdlib::Custom` nodes and user nodes (`shaders/.nodes/*.ron`) and user node ids are
  referenced by bare name in user-written WGSL (`return a * b;`), so they stay unprefixed and keep the old rules
  (`ident::check_code_ident`: shape + reserved words + reserved prefixes `n_`, `b_`, `sg_`, `_`). They are parameters
  of the generated helper fn (`sg_custom_<node>`, `sg_user_<id>`); a name like `inp_x` only shadows a module-level
  texture inside that helper, which WGSL allows.
- Removed checks: reserved words and reserved generated prefixes for graph names; "parameter clashes with a
  function name" (`inp_x` vs `fun_x`); a material next to a directory of the same name (`mat_env` vs `dir_env`);
  reserved top-level `shaders/stdlib/`, `functions/`, `curves/` (now `autogen/dir_curves/…`). Kept: duplicate
  parameter names, a texture `x` next to a parameter `x_sampler` (`inp_x_sampler` twice), and two called functions
  with the same name in different directories (E006: two `fun_x` imports would clash).

**Consequences.**
- Breaking change of the generated crate API: fields, `new` argument names and setters gain `inp_` (positional
  `new(..)` calls and type names are unaffected; `examples/sample_game/src` needed no change). Module paths below
  `shaders::materials` change; code using the root re-exports is unaffected.
- Every generated file moves (`autogen/env/rock_wet.wesl` → `autogen/dir_env/mat_rock_wet.wesl`). The next full
  regeneration writes the new files and deletes the old ones as orphans (they carry the autogen marker); empty old
  directories may be left behind. On that first regeneration a material that has errors loses its old unprefixed
  `.wesl` and `.rs` as orphans too (its kept output is looked up at the new paths), so game code naming its type stops
  compiling until the graph is fixed; this happens once and is reported by the E-code on that asset. Games referencing `SHADER_PATH` get the new path automatically.
- A generated-file tab is now named `mat_rock_wet`, so it no longer shares its short label with the graph tab
  `rock_wet` (decision 0019's example); the WESL and Rust tabs of one material still do.
- Fixtures `kw/fn` (function `fn` with input `in`, outputs `out` and `let`) and `kw/loop/type` (parameters `self`
  and `mesh`) cover the keyword path in the snapshot suite, the WESL compile check and the sample game's clippy run.

*Update (decisions 0024, 0025):* fixed textures add the generator prefix `ft_` (`ft_<node>`, `ft_<function path with
/ → _>_<node>` for a function's hidden texture input; a clash of two such names is E006). The game-facing fields of
`BakedMaterials` / `BakedTextures` (`baked.env.rock_wet`) are **not** statically prefixed: they keep the plain name
and get `dir_` / `mat_` / the new `tex_` only for a Rust keyword or a clash inside one struct (their module paths
below `baked::` still use `dir_`).
