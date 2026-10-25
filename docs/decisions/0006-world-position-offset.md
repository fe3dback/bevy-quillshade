# 0006. World Position Offset in v1

**Decision.** WPO is implemented in v1. If the `world_position_offset` pin is connected, a custom vertex shader is generated — a copy of `bevy_pbr` `mesh.wesl` (morphing, skinning, instance index) with the offset added in world space. For Pbr3d it is hooked up as `MaterialExtension::vertex_shader`, for Unlit3d as `Material::vertex_shader`. Conditional `VertexOutput` fields are guarded by `@if(...)`, as in Bevy.

**Limitation.** The prepass (depth/normal), shadows and the deferred prepass use the stock vertex shader and do not see the offset: shadows and SSAO are built from the original geometry. For small offsets (flags, grass) this is acceptable.

**Risk.** The copy of `mesh.wesl` is tied to Bevy 0.20.x; the snapshot tests compile it against the real sources and will fail on an incompatible patch release.
