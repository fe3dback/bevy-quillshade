//! Material's tab documents that have no file of their own (generated text; game images are `texture_kit`'s), the
//! Material side of the open tabs and the user config's tab keys. The tab container is core (`editor_core::tab`, design §6.2); each
//! kind's trait impls are in [`crate::editors`]. See decision 0014.

pub use editor_core::tab::{Tab, TextDoc, ViewKindId, labels};
use shader_graph_core::texture::INTERNAL_PREFIX;
use shader_project::layout::MATERIAL;
use shader_project::regen::{rust_file, wesl_file};
use shader_project::{AssetId, AssetSet, Project, assets};

use crate::consts_doc::ConstsDoc;
use crate::curve::CurveDoc;
use crate::doc::Doc;
use crate::doc_kind::TabDoc;
use crate::gradients_doc::GradientsDoc;
use crate::texture::{TextureDoc, TextureDocId};

pub use crate::texture::{GAME_KEY_PREFIX, ImageDoc};

/// The language of generated WESL in a text tab (the `#wesl` of its config key, the highlighter registered for it).
pub const WESL: &str = "wesl";
/// The language of generated Rust in a text tab.
pub const RUST: &str = "rust";

/// The text tab of the file generated from `id` in `lang` ([`WESL`] / [`RUST`]), read now. `None` when the asset has
/// no such file (functions have no Rust module, constant tables nothing at all).
pub fn generated_doc(project: &Project, id: &AssetId, lang: &'static str) -> Option<TextDoc> {
    let path = generated_path(project, id, lang)?;
    Some(TextDoc::open(project, id.to_ref(), lang, path).with_reload_hook(note_wesl_lines))
}

/// The file generated from `id` in `lang` ([`WESL`] / [`RUST`]); `None` when the asset has no such file.
pub fn generated_path(project: &Project, id: &AssetId, lang: &str) -> Option<std::path::PathBuf> {
    match lang {
        WESL => wesl_file(project, id),
        RUST => rust_file(project, id),
        _ => None,
    }
}

/// A reloaded WESL file's line count goes to the Material state (the graph's editor statusbar).
fn note_wesl_lines(state: &mut dyn std::any::Any, t: &TextDoc) {
    if t.lang == WESL
        && let Some(text) = &t.text
        && let Some(state) = state.downcast_mut::<crate::material_state::MaterialState>()
        && let Some(id) = AssetId::from_ref(&t.source)
    {
        state.wesl_lines.insert(id, text.lines().count());
    }
}

/// A Material tab with document `doc`.
pub fn material_tab(doc: impl TabDoc) -> Tab {
    Tab::new(MATERIAL, doc)
}

/// Puts a Material tab's unsaved content into `set` in place of the file's (what live validation and reference
/// lookups see); nothing for read-only documents and other domains' tabs.
pub fn override_into(tab: &Tab, set: &mut AssetSet) {
    if let Some(d) = tab.get::<Doc>() {
        set.assets.insert(d.id.clone(), Ok(d.asset.clone()));
    } else if let Some((c, id)) = tab.get::<CurveDoc>().and_then(|c| Some((c, AssetId::from_ref(&c.aref)?))) {
        set.curves.insert(id, Ok(c.curve.clone()));
    } else if let Some(c) = tab.get::<ConstsDoc>() {
        set.consts.insert(c.id.clone(), Ok(c.table.clone()));
    } else if let Some(t) = tab.get::<TextureDoc>().filter(|t| !t.is_internal()) {
        set.textures.insert(t.id(), Ok(t.asset.clone()));
    } else if let Some(g) = tab.get::<GradientsDoc>() {
        set.gradients.insert(g.id.clone(), Ok(g.collection.clone()));
    }
}

/// What a Material tab's [`TabDoc::config_key`] names (the `key` of its `editor_core::tab::TabKey`).
#[derive(Debug, Clone, PartialEq)]
pub enum MaterialKey {
    /// An asset's own document (`quillshade/material/env/rock.material.ron`).
    Asset(AssetId),
    /// A file generated from the asset (`…ron#wesl` / `#rust`).
    Generated(AssetId, &'static str),
    /// A game image (`@game/<path under assets/>`).
    GameImage(String),
    /// A built-in texture's read-only Texture tab (`@internal/default`, design §2).
    Internal(String),
}

/// The key [`TabDoc::config_key`] gives for this tab.
impl std::fmt::Display for MaterialKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MaterialKey::Asset(id) => f.write_str(&id.source_rel()),
            MaterialKey::Generated(id, lang) => write!(f, "{}#{}", id.source_rel(), lang),
            MaterialKey::GameImage(path) => write!(f, "{GAME_KEY_PREFIX}{path}"),
            MaterialKey::Internal(id) => f.write_str(id),
        }
    }
}

/// The core tab key of a Material tab key.
pub fn tab_key(key: &MaterialKey) -> editor_core::tab::TabKey {
    editor_core::tab::TabKey::new(MATERIAL, key.to_string())
}

/// Parses [`TabDoc::config_key`].
pub fn parse_config_key(key: &str) -> Option<MaterialKey> {
    if let Some(path) = key.strip_prefix(GAME_KEY_PREFIX) {
        return (!path.is_empty()).then(|| MaterialKey::GameImage(path.to_string()));
    }
    if key.starts_with(INTERNAL_PREFIX) {
        return Some(MaterialKey::Internal(key.to_string()));
    }
    match key.split_once('#') {
        Some((rel, lang)) => {
            let lang = [WESL, RUST].into_iter().find(|l| *l == lang)?;
            Some(MaterialKey::Generated(assets::id_from_source_rel(rel)?, lang))
        }
        None => Some(MaterialKey::Asset(assets::id_from_source_rel(key)?)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shader_project::{AssetType, fixtures};

    fn project() -> (tempfile::TempDir, Project) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"g\"\n").unwrap();
        std::fs::create_dir_all(dir.path().join("assets")).unwrap();
        let p = Project::open(dir.path()).unwrap();
        fixtures::save_all(&p).unwrap();
        (dir, p)
    }

    /// The text view caches on `(rel, revision)`: a tab closed and reopened after its file changed (a preview
    /// toggle regenerates the WESL, decision 0022) must not reuse the revision of the closed one.
    #[test]
    fn reopened_text_doc_gets_new_revision() {
        let (_dir, p) = project();
        let id = AssetId::new(AssetType::Material, "env/rock_wet");
        let first = generated_doc(&p, &id, WESL).unwrap();
        std::fs::create_dir_all(first.path.parent().unwrap()).unwrap();
        std::fs::write(&first.path, "// preview variant\n").unwrap();
        let second = generated_doc(&p, &id, WESL).unwrap();
        assert_eq!(second.rel, first.rel);
        assert_eq!(second.text.as_deref(), Some("// preview variant\n"));
        assert_ne!(second.revision, first.revision);
    }
}
