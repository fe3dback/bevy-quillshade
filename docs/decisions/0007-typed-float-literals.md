# 0007. Scalar literals with the `f` suffix

**Context.** The snapshot suite showed that naga 30 rejects `let x: f32 = mix(0.0, 1.0, 0.5);` outside a constant context ("Abstract types may only appear in constant expressions") when all arguments of a builtin function are abstract literals. This happens with any node whose inputs are unconnected (Lerp, SmoothStep…).

**Decision.** Scalar default values and constants are emitted as typed literals `0.5f` (`value::wesl_scalar`). Vector constructors `vec3<f32>(…)` do not need the suffix. The default value of Log/Log2 is 1.0: naga folds constants and rejects `log(0.0)`.
