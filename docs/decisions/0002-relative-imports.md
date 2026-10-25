# 0002. Imports between autogen modules (spike M0)

**Context.** Risk from spec §18: the Bevy 0.20 resolver might not support relative imports between files in `assets/`.

**Verified (M0).** The asset `assets/shaders/autogen/env/rock.wesl` gets the module path `package::shaders::autogen::env::rock`. `super::` drops the last component, so from `autogen/env/rock` the module `super::super::stdlib::math` points to `autogen/stdlib/math.wesl`. Bevy modules are imported as `bevy_pbr::…`; `constants::MATERIAL_BIND_GROUP` is a virtual module (3 for 3D, 2 for 2D). Imports are written only with the `import` statement (the resolver does not scan qualified paths inside expressions). Hot reload of imported files works with the `file_watcher` feature.

**Decision.** Stdlib and functions are separate modules; materials import them with relative paths (`paths::relative`), falling back to `package::` when that is impossible. No inline fallback is needed.

**Consequences.** Changing a function rewrites only its `.wesl`; Bevy recompiles the materials via the import graph.
