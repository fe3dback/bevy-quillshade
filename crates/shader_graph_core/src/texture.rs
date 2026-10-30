//! Texture assets of the Material domain (design §7.2, decision 0024): the model lives in `texture_kit` and is
//! re-exported here together with the Material [`TexturePolicy`].

pub use texture_kit::*;

/// What Material accepts: `ktx2` / `png` images that are plain 2D textures (Bevy 0.20 cannot read `.ktx` v1), with
/// the Texture tab's Easy mode. Its built-in textures (`@internal/default`) are `shader_stdlib::internal`'s; the
/// editor's policy adds them (`material_domain::texture::MaterialTexture`).
pub const MATERIAL_TEXTURES: TexturePolicy =
    TexturePolicy { extensions: &["ktx2", "png"], shape: TextureShape::Plain2d, internal: None, presets: true };
