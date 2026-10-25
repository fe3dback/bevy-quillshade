# 0001. wesl 0.6 instead of 0.4

**Context.** The spec (sections 4, 15) names wesl 0.4.x. Bevy 0.20 itself depends on `wesl 0.6` (via `bevy_shader`), and 0.4 is incompatible with the conditional compilation syntax and the `package::` paths that Bevy 0.20 uses.

**Decision.** Tests and the CLI `--compile` check use `wesl = "0.6"` (feature `naga-ext`) and `naga = "30"` — the same versions as Bevy 0.20. The check resolver (`shader_codegen_wesl::check`) loads the **real** WESL sources of `bevy_pbr`, `bevy_render`, `bevy_core_pipeline`, `bevy_sprite_render` 0.20 from the cargo registry instead of stubs.

**Consequences.** The check is stricter than the "stubs of bevy_pbr modules" from spec §15: it catches mismatches with Bevy's real signatures. If the Bevy sources are not in the registry (a machine without `cargo fetch`), the compile check is skipped with a warning; the rest of the validation works.
