//! The game's own files under `assets/` (design §4.2, §4.3): the images Material's Texture assets may point at and
//! the Texture asset that stands for one of them. Game files are never copied or modified. Creating a Texture asset
//! for a game file is the texture editor's (`texture_kit::editor::create_for_file`).

use shader_graph_core::texture::MATERIAL_TEXTURES;

use crate::assets::{AssetId, AssetSet};

/// Whether `path` names an image a Texture asset may point at (extension in [`MATERIAL_TEXTURES`], any case).
pub fn is_image(path: &str) -> bool {
    let file = path.rsplit('/').next().unwrap_or(path);
    file.rsplit_once('.').is_some_and(|(stem, ext)| {
        !stem.is_empty() && MATERIAL_TEXTURES.extensions.iter().any(|e| e.eq_ignore_ascii_case(ext))
    })
}

/// The Texture asset whose image is `path` (relative to `assets/`). There is at most one (E011 otherwise); with
/// several, the first in path order.
pub fn texture_for_file(set: &AssetSet, path: &str) -> Option<AssetId> {
    set.textures.iter().find(|(_, t)| t.as_ref().is_ok_and(|t| t.path == path)).map(|(id, _)| id.clone())
}
