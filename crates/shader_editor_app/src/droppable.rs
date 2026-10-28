//! Game files that can be dropped onto a graph (design §4.3): a [`DroppableFile`] says which files it takes (by
//! extension), which node a drop creates and how the node refers to the file. Images are the only kind for now:
//! dropping one adds a `stdlib::TextureSample` referencing the image's Texture asset, which is created when no Texture
//! asset points at the file yet. A Texture asset of the Project tree (or a built-in `@internal/…` texture) dropped the
//! same way only skips the lookup. Function and Curve assets of the Project tree are droppable too (design §4.11): a
//! Function adds its call node, a Curve sets a node's curve param or adds a `stdlib::SampleCurve`; a gradient collection
//! likewise sets a node's collection or adds a `stdlib::SampleGradient` with no gradient picked (gradient design §6).
//! The drop itself is [`crate::state::Action::Drop`].

use shader_graph_core::registry::ParamKind;
use shader_project::game_assets;

use crate::state::Editor;
use crate::ui::tree::{TreeItem, TreeRow};
use shader_project::AssetType;

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
    /// What dragging `row` onto a graph drops: a droppable game file, a Texture / Function / Curve / gradient
    /// collection asset or a built-in texture.
    pub fn from_row(row: &TreeRow) -> Option<Dropped> {
        match &row.item {
            TreeItem::GameFile(path) if droppable_for(path).is_some() => Some(Dropped::GameFile(path.clone())),
            TreeItem::Asset(id) => match id.ty {
                AssetType::Texture => Some(Dropped::Texture(id.path.clone())),
                AssetType::Function => Some(Dropped::Function(id.path.clone())),
                AssetType::Curve => Some(Dropped::Curve(id.path.clone())),
                AssetType::Gradients => Some(Dropped::Gradients(id.path.clone())),
                _ => None,
            },
            TreeItem::Internal(id) => Some(Dropped::Texture(id.clone())),
            _ => None,
        }
    }
}
