//! Material's Texture assets and game images (decision 0024, design §7.2): `texture_kit`'s texture editor for
//! `quillshade/material/**/*.texture.ron` ([`MaterialTexture`]) with Material's built-in textures
//! (`shader_stdlib::internal`, `@internal/default`) through the policy, plus what only Material has: its assets and
//! diagnostics as kept by its state, regeneration after "Create Texture Asset", and the texture references of its
//! graph nodes ([`MaterialTextures`]: the node thumbnails' images, the texture pickers' choices). Codegen and the
//! reference follow-ups (`texture_refs`) stay in Material's backend and ops.

use editor_core::menu::MenuEntry;
use editor_core::state::editor::Editor;
use editor_core::tab::{Tab, ViewKindId};
use project_core::{AssetRef, Diagnostic, DomainId};
use shader_graph_core::texture::{INTERNAL_PREFIX, MATERIAL_TEXTURES, TexturePolicy, TextureSettings};
use shader_project::layout::MATERIAL;
use shader_project::{AssetId, AssetType};
use texture_kit::editor::TextureDomain;

use crate::material_state::MaterialStates;
use crate::ops::MaterialRegen;

pub use texture_kit::editor::{
    Channel, CreateTextureForFile, FileStamp, FilterPreset, GAME_KEY_PREFIX, INTERNAL_FOLDER, PreviewDoc,
    TextureAction, TextureField, TextureProp, fits_easy,
};

/// Material's texture domain (`texture_kit::editor::register::<MaterialTexture>` in [`crate::MaterialDomain`]).
pub struct MaterialTexture;

/// Material's Texture editor kind's view: settings form + preview (design §4.4).
pub const TEXTURE: ViewKindId = ViewKindId("material.texture");
/// Material's image editor kind's view: a read-only game image (design §4.4).
pub const IMAGE: ViewKindId = ViewKindId("material.image");

/// An edit of Material's Texture tab (`texture_kit`'s, tagged with [`MaterialTexture`], design §14.4).
pub type TextureEdit = texture_kit::editor::TextureEdit<MaterialTexture>;
/// The preview channel of Material's image or Texture tab (tagged with [`MaterialTexture`]).
pub type PreviewChannel = texture_kit::editor::PreviewChannel<MaterialTexture>;
/// A property target of Material's Texture or image tab (`texture_kit`'s [`TextureProp`], tagged with
/// [`MaterialTexture`]).
pub type TextureKey = editor_core::dynval::KitProp<MaterialTexture, TextureProp>;

/// A Material Texture tab's document.
pub type TextureDoc = texture_kit::editor::TextureDoc<MaterialTexture>;
/// A Material game image tab's document.
pub type ImageDoc = texture_kit::editor::ImageDoc<MaterialTexture>;
/// Material's Texture editor kind.
pub type TextureEditor = texture_kit::editor::TextureEditor<MaterialTexture>;
/// Material's image editor kind.
pub type ImageEditor = texture_kit::editor::ImageEditor<MaterialTexture>;
/// Material's decoded game images (the tabs' previews and the texture nodes' thumbnails).
pub type GameImageCache = texture_kit::editor::GameImageCache<MaterialTexture>;
pub type GameImagePlugin = texture_kit::editor::GameImagePlugin<MaterialTexture>;
pub type ImagePreviewPlugin = texture_kit::editor::ImagePreviewPlugin<MaterialTexture>;
pub type TextureViewPlugin = texture_kit::editor::TextureViewPlugin<MaterialTexture>;
pub type ImageViewPlugin = texture_kit::editor::ImageViewPlugin<MaterialTexture>;
pub type PreviewArea = texture_kit::editor::PreviewArea<MaterialTexture>;
pub type PreviewZoomText = texture_kit::editor::PreviewZoomText<MaterialTexture>;

/// Material's Texture view in the editor content.
pub fn texture_view() -> impl bevy::prelude::Scene {
    texture_kit::editor::texture_view::<MaterialTexture>()
}

/// Material's image view in the editor content.
pub fn image_view() -> impl bevy::prelude::Scene {
    texture_kit::editor::image_view::<MaterialTexture>()
}

impl TextureDomain for MaterialTexture {
    const DOMAIN: DomainId = MATERIAL;
    const VIEW: ViewKindId = TEXTURE;
    const IMAGE_VIEW: ViewKindId = IMAGE;
    /// One policy with the backend's ([`MATERIAL_TEXTURES`]), plus the editor's built-in textures.
    const POLICY: TexturePolicy =
        TexturePolicy { internal: Some(shader_stdlib::internal::internal_textures), ..MATERIAL_TEXTURES };

    fn row_menu(editor: &Editor, r: &AssetRef) -> Vec<MenuEntry> {
        crate::domain::asset_row_menu(editor, r)
    }

    /// From Material's last scan (what validation sees too).
    fn disk_owners(editor: &Editor, file: &str) -> Vec<AssetRef> {
        let textures = &editor.domains.material().disk.textures;
        textures.iter().filter(|(_, t)| t.as_ref().is_ok_and(|t| t.path == file)).map(|(id, _)| id.to_ref()).collect()
    }

    /// Material's live diagnostics of the asset (validation's, not the regeneration-only ones).
    fn problems(editor: &Editor, r: &AssetRef) -> Vec<Diagnostic> {
        let Some(id) = AssetId::from_ref(r) else { return Vec::new() };
        let diags = editor.domains.material().diags.get(&id);
        diags.into_iter().flatten().map(|d| d.to_generic(MATERIAL)).collect()
    }

    /// The new asset is generated (the baked manifest lists it).
    fn created(editor: &mut Editor, _r: &AssetRef) {
        editor.regenerate();
    }
}

/// The Material id of a Texture tab's asset (`Texture` + its `@internal/…` id for a built-in texture).
pub trait TextureDocId {
    fn id(&self) -> AssetId;
}

impl TextureDocId for TextureDoc {
    fn id(&self) -> AssetId {
        AssetId::new(AssetType::Texture, self.aref.path.clone())
    }
}

/// Texture assets and game images on Material's `Editor` (decision 0024): what its graph and its tests ask of the
/// texture kit, by Material id.
pub trait MaterialTextures {
    /// The Texture asset claiming the game file `path` (on disk or in an open tab): "Create Texture Asset" is
    /// offered only when there is none.
    fn texture_for_file(&self, path: &str) -> Option<AssetId>;

    /// Creates a Texture asset with default settings for the game image `path` (relative to `assets/`) at
    /// `quillshade/material/textures/<folders>/<stem>.texture.ron` (`texture_kit::new_texture_path_for_file`, design
    /// §4.3) and regenerates. Refused (an error message) when a Texture asset already points at the file, on disk or
    /// in an open tab. Not undoable: asset creation is not part of any document's history.
    fn create_texture_for_file(&mut self, path: &str) -> Option<AssetId>;

    /// [`Self::create_texture_for_file`] without the error dialog: the error text is returned instead (a drop onto
    /// a graph reports it itself).
    fn try_create_texture_for_file(&mut self, path: &str) -> Result<AssetId, String>;

    /// Opens (or activates) the read-only tab of the game image `assets/<path>` (design §4.4).
    fn open_game_image(&mut self, path: &str);

    /// What a texture reference (a node's `texture` value: a Texture asset path or an `@internal/…` id) shows: the
    /// key [`GameImageCache::get`] decodes and the settings. Built-in textures first, then open Texture tabs (unsaved
    /// edits), then the asset as saved; `None` for an unknown or broken reference.
    fn texture_image(&self, key: &str) -> Option<(String, TextureSettings)>;

    /// Choices of a `ParamKind::Texture` picker as `(stored value, label)`: the project's Texture assets, then the
    /// built-in textures (`Internal/default`).
    fn texture_choices(&self) -> Vec<(String, String)>;
}

impl MaterialTextures for Editor {
    fn texture_for_file(&self, path: &str) -> Option<AssetId> {
        texture_kit::editor::texture_for_file::<MaterialTexture>(self, path).as_ref().and_then(AssetId::from_ref)
    }

    fn create_texture_for_file(&mut self, path: &str) -> Option<AssetId> {
        texture_kit::editor::create_for_file::<MaterialTexture>(self, path).as_ref().and_then(AssetId::from_ref)
    }

    fn try_create_texture_for_file(&mut self, path: &str) -> Result<AssetId, String> {
        let r = texture_kit::editor::try_create_for_file::<MaterialTexture>(self, path)?;
        AssetId::from_ref(&r).ok_or_else(|| format!("{} is not a Material asset", r.source_rel()))
    }

    fn open_game_image(&mut self, path: &str) {
        texture_kit::editor::open_image::<MaterialTexture>(self, path);
    }

    fn texture_image(&self, key: &str) -> Option<(String, TextureSettings)> {
        if let Some(t) = MaterialTexture::POLICY.internal_texture(key) {
            return Some((t.id.to_string(), (t.settings)()));
        }
        let id = AssetId::new(AssetType::Texture, key);
        if let Some(t) = self.tabs.iter().filter_map(Tab::get::<TextureDoc>).find(|t| t.id() == id) {
            return Some((t.asset.path.clone(), t.asset.settings.clone()));
        }
        let t = self.domains.material().disk.textures.get(&id)?.as_ref().ok()?;
        Some((t.path.clone(), t.settings.clone()))
    }

    fn texture_choices(&self) -> Vec<(String, String)> {
        let assets = self.domains.material().disk.textures.keys().map(|id| (id.path.clone(), id.path.clone()));
        let internal = MaterialTexture::POLICY.internal_textures().iter().map(|t| {
            let name = t.id.strip_prefix(INTERNAL_PREFIX).unwrap_or(t.id);
            (t.id.to_string(), format!("Internal/{name}"))
        });
        assets.chain(internal).collect()
    }
}

#[cfg(test)]
mod tests {
    use bevy::image::{ImageFilterMode, ImageSampler};
    use bevy::prelude::*;
    use shader_stdlib::internal::{DEFAULT_TEXTURE, INTERNAL_TEXTURES};

    use super::*;

    /// Material's built-in textures (moved from `game_image`): decoded from the embedded bytes (no file under
    /// `assets/` needed), cached per settings, shown at the path regeneration copies them to.
    #[test]
    fn internal_textures_decode_from_embedded_bytes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"g\"\n").unwrap();
        std::fs::create_dir_all(dir.path().join("assets")).unwrap();
        let p = shader_project::Project::open(dir.path()).unwrap();
        let mut images = Assets::<Image>::default();
        let mut cache = GameImageCache::default();
        let settings = (INTERNAL_TEXTURES[0].settings)();
        let h = cache.get(&p, DEFAULT_TEXTURE, &settings, &mut images).unwrap();
        assert_eq!(images.get(&h).unwrap().size(), UVec2::new(64, 64));
        assert!(
            matches!(&images.get(&h).unwrap().sampler, ImageSampler::Descriptor(d) if d.mag_filter == ImageFilterMode::Linear)
        );
        assert_eq!(cache.get(&p, DEFAULT_TEXTURE, &settings, &mut images), Ok(h));
        let nearest = cache.get(&p, DEFAULT_TEXTURE, &TextureSettings::default(), &mut images).unwrap();
        assert!(matches!(images.get(&nearest).unwrap().sampler, ImageSampler::Default));
        let unknown = cache.get(&p, "@internal/nope", &settings, &mut images).unwrap_err();
        assert!(unknown.contains("unknown built-in texture"), "{unknown}");
        let path = texture_kit::editor::internal_path::<MaterialTexture>(&INTERNAL_TEXTURES[0]);
        assert_eq!(path, "quillshade/material/internal/default.png");
        let internal_dir = shader_project::layout::INTERNAL_DIR;
        assert_eq!(format!("assets/{path}"), format!("{internal_dir}/{}", INTERNAL_TEXTURES[0].file), "regen's file");
    }

    /// One policy (design §7.2): the backend's, plus the editor's built-in textures.
    #[test]
    fn policy_is_the_backends_plus_internal_textures() {
        let p = MaterialTexture::POLICY;
        assert_eq!((p.extensions, p.shape, p.presets), (MATERIAL_TEXTURES.extensions, MATERIAL_TEXTURES.shape, true));
        assert!(MATERIAL_TEXTURES.internal.is_none());
        assert_eq!(p.internal_textures().len(), INTERNAL_TEXTURES.len());
        assert!(p.internal_texture(DEFAULT_TEXTURE).is_some());
    }
}
