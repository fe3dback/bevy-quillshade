//! Light's cube-map Texture assets (design §9.2, §7.2): `texture_kit`'s texture editor for
//! `quillshade/light/**/*.texture.ron` under [`LIGHT_TEXTURES`] (`.ktx2` cube maps, the plain settings form), with a
//! "New Texture..." template and the shared cube-map preview (`CubeMapPreview`: 3D sphere, 3D skybox, and 6-face 2D
//! strip).

use editor_core::state::editor::Editor;
use editor_core::tab::ViewKindId;
use project_core::{AssetRef, DomainId};
use texture_kit::TexturePolicy;
use texture_kit::editor::TextureDomain;

use super::LightState;
use crate::backend::{LIGHT, LIGHT_TEXTURES, texture_ref};

/// Light's Texture editor kind's view: settings form + preview.
pub const LIGHT_TEXTURE_VIEW: ViewKindId = ViewKindId("light.texture");
/// Light's image editor kind's view: a read-only `.ktx2` game image.
pub const LIGHT_IMAGE_VIEW: ViewKindId = ViewKindId("light.image");

/// Light's texture domain (`texture_kit::editor::register::<LightTexture>` in [`super::LightDomain`]): owners and
/// problems come from [`LightState`]'s in-memory scan and diagnostics, and "Create Texture Asset" is followed by the
/// state's `after_save` (regeneration).
pub struct LightTexture;

impl TextureDomain for LightTexture {
    const DOMAIN: DomainId = LIGHT;
    const VIEW: ViewKindId = LIGHT_TEXTURE_VIEW;
    const IMAGE_VIEW: ViewKindId = LIGHT_IMAGE_VIEW;
    const POLICY: TexturePolicy = LIGHT_TEXTURES;
    /// The Assets menu has "New Texture..." (design §9.2): the form then picks the `.ktx2` file.
    const NEW_TEMPLATE: bool = true;
    /// Loader and sampler settings are baked into `assets/quillshade/light/autogen/baked.ron`.
    const SETTINGS_NOTE: &'static str = "Loader and sampler settings of the cube map, saved with the asset and baked \
                                         into assets/quillshade/light/autogen/baked.ron for the runtime loader.";

    /// From Light's last scan (what validation sees too).
    fn disk_owners(editor: &Editor, file: &str) -> Vec<AssetRef> {
        let Some(state) = editor.domains.get::<LightState>() else { return Vec::new() };
        state.assets.textures.iter().filter(|(_, t)| t.path == file).map(|(p, _)| texture_ref(p)).collect()
    }
}
