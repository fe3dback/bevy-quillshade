# Migrating from 0.3 to 0.4

## Replace `BakedMaterialsLoader` with `QuillshadeLoader`

### Before (0.3)

```rust
use quillshade::material::{BakedMaterialsLoader, BakedMaterialsProgress, BakedMaterialsReady};
```

### After (0.4)

```rust
use quillshade::{QuillshadeLoader, QuillshadeProgress, QuillshadeReady};
```

## Shared Curves & Gradients (`quillshade::common`)

-  `CurveData`, `Interp`, `GradientData`, and `GradientStop` now lives in `quillshade::common`: