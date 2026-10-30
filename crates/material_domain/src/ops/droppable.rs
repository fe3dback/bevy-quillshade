//! Game files that can be dropped onto a graph (design §4.3): a [`DroppableFile`] says which files it takes (by
//! extension), which node a drop creates and how the node refers to the file. Images are the only kind for now:
//! dropping one adds a `stdlib::TextureSample` referencing the image's Texture asset, which is created when no Texture
//! asset points at the file yet. A Texture asset of the Project tree (or a built-in `@internal/…` texture) dropped the
//! same way only skips the lookup. Function and Curve assets of the Project tree are droppable too (design §4.11): a
//! Function adds its call node, a Curve sets a node's curve param or adds a `stdlib::SampleCurve`; a gradient collection
//! likewise sets a node's collection or adds a `stdlib::SampleGradient` with no gradient picked (gradient design §6).
//! The drop itself is [`crate::ops::MaterialAction::Drop`].

use shader_graph_core::registry::ParamKind;
use shader_project::{AssetId, AssetType, game_assets};

use super::Editor;
use crate::ops::prelude::*;
use editor_core::ui::tree::{TreeItem, TreeRow};

/// A kind of game file that can be dropped onto a graph.
pub trait DroppableFile: Send + Sync {
    /// Whether `path` (relative to `assets/`) is of this kind; by extension.
    fn matches(&self, path: &str) -> bool;
    /// Node a drop on empty canvas adds, e.g. `"stdlib::TextureSample"`.
    fn node_kind(&self) -> &'static str;
    /// Param of [`Self::node_kind`] that gets the reference.
    fn param(&self) -> &'static str;
    /// Kind of param a drop onto an existing node fills (the node's first param of this kind).
    fn param_kind(&self) -> ParamKind;
    /// Asset to reference (created if needed); returns its id as the param value. Creating an asset is not part of
    /// any document's undo history (design §4.3).
    fn resolve(&self, editor: &mut Editor, path: &str) -> Result<String, String>;
}

/// `.png` / `.ktx2` images → the Texture asset pointing at them.
pub struct ImageFile;

impl DroppableFile for ImageFile {
    fn matches(&self, path: &str) -> bool {
        game_assets::is_image(path)
    }

    fn node_kind(&self) -> &'static str {
        "stdlib::TextureSample"
    }

    fn param(&self) -> &'static str {
        shader_stdlib::nodes::texture::TEXTURE_PARAM
    }

    fn param_kind(&self) -> ParamKind {
        ParamKind::Texture
    }

    fn resolve(&self, editor: &mut Editor, path: &str) -> Result<String, String> {
        // Claimed on disk or by an open (unsaved) Texture tab: reuse it.
        if let Some(id) = editor.texture_for_file(path) {
            return Ok(id.path);
        }
        editor.try_create_texture_for_file(path).map(|id| id.path)
    }
}

/// Every droppable file kind, tried in order.
pub fn droppables() -> &'static [&'static dyn DroppableFile] {
    &[&ImageFile]
}

/// The droppable kind of the game file `path`.
pub fn droppable_for(path: &str) -> Option<&'static dyn DroppableFile> {
    droppables().iter().copied().find(|d| d.matches(path))
}

/// What is dropped onto a graph.
#[derive(Debug, Clone, PartialEq)]
pub enum Dropped {
    /// A game file, path relative to `assets/` (Game Assets tree).
    GameFile(String),
    /// A texture reference as stored in a node: a Texture asset id of the Project tree or an `@internal/…` id.
    Texture(String),
    /// A Function asset of the Project tree, path without suffix: adds a `function::<path>` call node.
    Function(String),
    /// A Curve asset of the Project tree, path without suffix: the value of a `ParamKind::Curve` param.
    Curve(String),
    /// A gradient collection of the Project tree, path without suffix: the value of a `GradientCollection` param.
    Gradients(String),
}

impl Dropped {
    /// What dragging a sidebar tree row onto a graph drops: a droppable game file, or by the asset kind of a Project
    /// tree row (Texture, Function, Curve, gradient collection; a built-in `@internal/…` texture of the virtual
    /// Internal folder too). `None` for anything else (another domain's asset, a folder).
    pub fn from_row(row: &TreeRow) -> Option<Dropped> {
        match &row.item {
            TreeItem::GameFile(path) => droppable_for(path).map(|_| Dropped::GameFile(path.clone())),
            TreeItem::Asset(r) | TreeItem::Virtual(r) => {
                let id = AssetId::from_ref(r)?;
                match id.ty {
                    AssetType::Texture => Some(Dropped::Texture(id.path)),
                    AssetType::Function => Some(Dropped::Function(id.path)),
                    AssetType::Curve => Some(Dropped::Curve(id.path)),
                    AssetType::Gradients => Some(Dropped::Gradients(id.path)),
                    _ => None,
                }
            }
            TreeItem::Folder(_) | TreeItem::VirtualFolder(..) => None,
        }
    }
}
