//! Light's cube-map Texture assets (design §9.2, §7.2): `texture_kit`'s texture editor for
//! `quillshade/light/**/*.texture.ron` under [`LIGHT_TEXTURES`] (`.ktx2` cube maps, the plain settings form), with a
//! "New Texture..." template and a preview of the cube map's first face.

use editor_core::tab::ViewKindId;
use project_core::DomainId;
use texture_kit::TexturePolicy;
use texture_kit::editor::TextureDomain;

use crate::backend::{LIGHT, LIGHT_TEXTURES};

/// Light's Texture editor kind's view: settings form + preview.
pub const LIGHT_TEXTURE_VIEW: ViewKindId = ViewKindId("light.texture");
/// Light's image editor kind's view: a read-only `.ktx2` game image.
pub const LIGHT_IMAGE_VIEW: ViewKindId = ViewKindId("light.image");

/// Light's texture domain (`texture_kit::editor::register::<LightTexture>` in [`super::LightDomain`]). The kit's
/// defaults fit: owners read from disk, problems from [`super::LightState`]'s diagnostics, and "Create Texture Asset"
/// followed by the state's `after_save` (regeneration).
pub struct LightTexture;

impl TextureDomain for LightTexture {
    const DOMAIN: DomainId = LIGHT;
    const VIEW: ViewKindId = LIGHT_TEXTURE_VIEW;
    const IMAGE_VIEW: ViewKindId = LIGHT_IMAGE_VIEW;
    const POLICY: TexturePolicy = LIGHT_TEXTURES;
    /// The Assets menu has "New Texture..." (design §9.2): the form then picks the `.ktx2` file.
    const NEW_TEMPLATE: bool = true;
    /// The cube map's first face (design §7.2: a cross layout is future work).
    const PREVIEW_FIRST_LAYER: bool = true;
    /// Light's module lists the environments only (design §9.2): nothing applies these settings yet.
    const SETTINGS_NOTE: &'static str = "Loader and sampler settings of the cube map, saved with the asset; Light's \
                                         generated module does not apply them yet.";
}
