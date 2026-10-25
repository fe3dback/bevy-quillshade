# 0005. Builtin values in functions are hidden arguments

**Context.** A function may use UV, time, world position, etc., but it is called from materials of different kinds (Pbr3d/Unlit3d/Sprite2d) and from the vertex stage; the source of the value differs between them (`VertexOutput`, `globals`, prepass bindings).

**Decision.** Every builtin value a function needs (directly or through nested functions) is added as a trailing `b_*` parameter of its signature. The calling material substitutes the value from its own environment.

**Consequences.** Functions are portable across material kinds and stages; a function's WESL signature depends on the builtins it uses (visible in the snapshot tests).
