//! The game's own files under `assets/` (design §4.2, §4.3): the images the editor's Game Assets tree lists and the
//! Texture asset that stands for one of them. Game files are never copied or modified.

use shader_graph_core::texture::{TEXTURE_EXTENSIONS, TextureAsset, asset_name_from_file_stem};

use crate::assets::{AssetId, AssetSet, AssetType};
use crate::layout::Project;
use crate::ops::{self, OpError};

/// The directory under `assets/` the editor writes into (generated WESL, internal textures); hidden from the Game
/// Assets tree.
pub const OWN_DIR: &str = "shaders";

/// Folder under `shaders/` that Texture assets made for game files go to (design §4.3).
pub const TEXTURES_DIR: &str = "textures";

/// Whether `path` names an image a Texture asset may point at (extension in [`TEXTURE_EXTENSIONS`], any case).
pub fn is_image(path: &str) -> bool {
    let file = path.rsplit('/').next().unwrap_or(path);
    file.rsplit_once('.')
        .is_some_and(|(stem, ext)| !stem.is_empty() && TEXTURE_EXTENSIONS.iter().any(|e| e.eq_ignore_ascii_case(ext)))
}

/// Images under `assets/`: paths relative to it, `/`-separated, sorted. `assets/shaders/**` (the editor's output),
/// hidden files / directories and symlinked directories (a link loop would never end) are skipped; symlinked files
/// are listed.
pub fn scan_images(project: &Project) -> Vec<String> {
    let root = project.assets_dir();
    let mut out = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            let p = e.path();
            let Ok(rel) = p.strip_prefix(&root) else { continue };
            let rel = rel.to_string_lossy().replace('\\', "/");
            let Ok(ty) = e.file_type() else { continue };
            if ty.is_dir() {
                if rel != OWN_DIR {
                    stack.push(p);
                }
            } else if ty.is_symlink() && p.is_dir() {
                continue;
            } else if is_image(&rel) {
                out.push(rel);
            }
        }
    }
    out.sort();
    out
}

/// The Texture asset whose image is `path` (relative to `assets/`). There is at most one (E011 otherwise); with
/// several, the first in path order.
pub fn texture_for_file(set: &AssetSet, path: &str) -> Option<AssetId> {
    set.textures.iter().find(|(_, t)| t.as_ref().is_ok_and(|t| t.path == path)).map(|(id, _)| id.clone())
}

/// Id of a new Texture asset for the game file `path` (design §4.3): `textures/<folders>/<stem>`, every component
/// made a valid name with [`asset_name_from_file_stem`]. The folders are the file's folders under `assets/`, minus a
/// leading `textures` (so `assets/textures/env/Rock-Albedo.ktx2` → `textures/env/rock_albedo`). A name another asset
/// already has gets `_2`, `_3`, …
pub fn new_texture_id_for_file(set: &AssetSet, path: &str) -> AssetId {
    let mut parts: Vec<&str> = path.split('/').collect();
    let file = parts.pop().unwrap_or_default();
    if parts.first() == Some(&TEXTURES_DIR) {
        parts.remove(0);
    }
    let stem = file.rsplit_once('.').map_or(file, |(s, _)| s);
    let mut base: Vec<String> = vec![TEXTURES_DIR.to_string()];
    base.extend(parts.iter().map(|d| asset_name_from_file_stem(d)));
    base.push(asset_name_from_file_stem(stem));
    let base = base.join("/");
    let taken = |p: &str| set.ids().any(|id| id.path == p);
    let mut candidate = base.clone();
    let mut n = 2;
    while taken(&candidate) {
        candidate = format!("{base}_{n}");
        n += 1;
    }
    AssetId::new(AssetType::Texture, candidate)
}

/// Creates a Texture asset with default settings for the game file `path` at [`new_texture_id_for_file`] (the
/// browser's "Create Texture Asset" and a drop onto a graph). `set` is the project's current asset set. Refused when
/// a Texture asset already points at the file.
pub fn create_texture_for_file(project: &Project, set: &AssetSet, path: &str) -> Result<AssetId, OpError> {
    if let Some(other) = texture_for_file(set, path) {
        return Err(OpError::Invalid(format!("`{path}` already has a Texture asset: {other}")));
    }
    let id = new_texture_id_for_file(set, path);
    ops::create_texture(project, &id, &TextureAsset::new(path))?;
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> (tempfile::TempDir, Project) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"g\"\n").unwrap();
        std::fs::create_dir_all(dir.path().join("assets")).unwrap();
        let p = Project::open(dir.path()).unwrap();
        (dir, p)
    }

    fn touch(p: &Project, rel: &str) {
        let f = p.assets_dir().join(rel);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, b"x").unwrap();
    }

    #[test]
    fn scan_images_filters_and_skips_shaders_dir() {
        let (_dir, p) = project();
        for f in ["a.png", "b.KTX2", "c.ktx", "d.jpg", "shaders/internal/default.png"] {
            touch(&p, f);
        }
        assert_eq!(scan_images(&p), ["a.png", "b.KTX2"]);
    }

    #[test]
    fn scan_images_recurses_and_skips_hidden() {
        let (_dir, p) = project();
        for f in ["env/rock.png", "env/deep/x.ktx2", ".cache/y.png", "env/.z.png", "shaders/autogen/w.png"] {
            touch(&p, f);
        }
        assert_eq!(scan_images(&p), ["env/deep/x.ktx2", "env/rock.png"]);
    }

    /// A symlinked directory (here a loop back to `assets/`) is not followed; a symlinked image file is listed.
    #[cfg(unix)]
    #[test]
    fn scan_images_skips_symlinked_dirs() {
        let (_dir, p) = project();
        touch(&p, "env/rock.png");
        let assets = p.assets_dir();
        std::os::unix::fs::symlink(&assets, assets.join("env/loop")).unwrap();
        std::os::unix::fs::symlink(assets.join("env/rock.png"), assets.join("linked.png")).unwrap();
        assert_eq!(scan_images(&p), ["env/rock.png", "linked.png"]);
    }

    #[test]
    fn new_texture_ids_mirror_folders_and_avoid_clashes() {
        let (_dir, p) = project();
        let set = AssetSet::default();
        let path = |f: &str| new_texture_id_for_file(&set, f).path;
        assert_eq!(path("textures/env/Rock-Albedo.ktx2"), "textures/env/rock_albedo");
        assert_eq!(path("ui/Icons 2/1.png"), "textures/ui/icons_2/t_1");
        assert_eq!(path("a.png"), "textures/a");

        let id = create_texture_for_file(&p, &AssetSet::load_all(&p), "textures/a.png").unwrap();
        assert_eq!(id, AssetId::new(AssetType::Texture, "textures/a"));
        let set = AssetSet::load_all(&p);
        assert_eq!(texture_for_file(&set, "textures/a.png"), Some(id.clone()));
        assert_eq!(texture_for_file(&set, "textures/a.ktx2"), None);
        // Same stem, other file: a suffix. The same file again: refused.
        assert_eq!(new_texture_id_for_file(&set, "textures/a.ktx2").path, "textures/a_2");
        assert!(create_texture_for_file(&p, &set, "textures/a.png").is_err());
        let b = create_texture_for_file(&p, &set, "textures/a.ktx2").unwrap();
        assert_eq!(b.path, "textures/a_2");
        assert_eq!(new_texture_id_for_file(&AssetSet::load_all(&p), "a.ktx2").path, "textures/a_3");
    }
}
