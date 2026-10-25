# 0004. Stdlib storage

**Decision.** Each node's description (pins, parameters, type inference, emit) is Rust code in `shader_stdlib` written with a compact builder (`def(id, name, cat).kw().param().build(sig, emit)`). Reusable WESL functions (`sg_remap*`, noise, TBN, blending, blackbody…) are files `crates/shader_stdlib/wesl/*.wesl`, embedded with `include_str!` and always written to `assets/shaders/autogen/stdlib/` (materials import only the functions they need). User nodes are data-driven RON in `shaders/.nodes/*.ron` with the same `NodeDef` trait.

**Consequences.** No custom DSL is needed; every node is covered by a snapshot test (`crates/shader_project/tests/snapshots.rs`) that compiles the result against Bevy.
