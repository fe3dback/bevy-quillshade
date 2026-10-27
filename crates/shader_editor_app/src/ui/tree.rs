//! Tree sources of the sidebar (design §4.2): each tree (the editable Project tree, the read-only Game Assets tree)
//! lists its rows as `(depth, TreeItem)`; `browser.rs` draws them and handles clicks keyed by [`TreeRow`] through
//! [`click_actions`], [`row_menu`] and [`drop_action`]. Collapse state is kept per tree in [`Editor::collapsed`].

use std::collections::BTreeSet;

use bevy::prelude::*;
use shader_project::{AssetId, AssetType};
use shader_stdlib::internal::INTERNAL_TEXTURES;

use super::browser::create_items;
use super::dialog::MenuEntry;
use super::icons::IconId;
use crate::state::{Action, Editor, GenFile};
use crate::tab::Lang;

/// Key of the Project tree's virtual read-only `Internal` folder of built-in textures (design §2): the parent of
/// their `@internal/…` ids. `@` is not allowed in asset or folder names, so no real folder made by the editor has
/// this path.
pub const INTERNAL_FOLDER: &str = "@internal";

/// Which sidebar tree a row belongs to.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub enum TreeId {
    /// The `shaders/` tree of editor assets.
    Project,
    /// Droppable files under the game's `assets/` dir (read-only).
    GameAssets,
}

/// What a tree row stands for.
#[derive(Clone, PartialEq, Debug)]
pub enum TreeItem {
    /// A folder path relative to the tree root (`""` is the root itself).
    Folder(String),
    /// An editor asset (Project tree).
    Asset(AssetId),
    /// A game file, path relative to `assets/` (Game Assets tree).
    GameFile(String),
    /// A built-in item of the virtual `Internal` folder, an `@internal/…` id (Project tree).
    Internal(String),
}

/// A drawn row of a sidebar tree.
#[derive(Component, Clone)]
pub struct TreeRow {
    pub tree: TreeId,
    pub item: TreeItem,
}

/// One sidebar tree: its rows in display order (depth-first, collapsed folders already skipped).
pub trait TreeSource {
    fn id(&self) -> TreeId;
    /// `(depth, item)` rows; the root folder (`TreeItem::Folder("")`) comes first at depth 0.
    fn rows(&self, editor: &Editor) -> Vec<(usize, TreeItem)>;
    /// No create / rename / move / delete.
    fn read_only(&self) -> bool;
}

/// The source of `tree` (`None` while that tree has none yet).
pub fn source(tree: TreeId) -> Option<&'static dyn TreeSource> {
    match tree {
        TreeId::Project => Some(&ProjectTree),
        TreeId::GameAssets => Some(&GameAssetsTree),
    }
}

/// Rows of a tree of `leaves` (`(path, item)`, sorted): the root, then depth-first each folder (the `folders` given
/// plus the parents of every leaf) followed by its leaves; leaves at the root last. Collapsed folders hide their
/// contents.
fn nest(
    editor: &Editor,
    tree: TreeId,
    mut folders: BTreeSet<String>,
    leaves: Vec<(String, TreeItem)>,
) -> Vec<(usize, TreeItem)> {
    for (path, _) in &leaves {
        let mut p = path.as_str();
        while let Some((dir, _)) = p.rsplit_once('/') {
            folders.insert(dir.to_string());
            p = dir;
        }
    }
    let hidden = |path: &str| editor.hidden_in(tree, path);
    let mut rows = vec![(0, TreeItem::Folder(String::new()))];
    let emit_leaves = |dir: &str, depth: usize, rows: &mut Vec<(usize, TreeItem)>| {
        for (path, item) in &leaves {
            if path.rsplit_once('/').map_or("", |(d, _)| d) == dir && !hidden(path) {
                rows.push((depth, item.clone()));
            }
        }
    };
    // By path components, so `a/c` follows `a` even though `-`, `.` or a space (`a-b`) sort before `/`.
    let mut folders: Vec<String> = folders.into_iter().collect();
    folders.sort_by(|a, b| a.split('/').cmp(b.split('/')));
    for f in &folders {
        if hidden(f) {
            continue;
        }
        let depth = f.matches('/').count() + 1;
        rows.push((depth, TreeItem::Folder(f.clone())));
        if !editor.is_collapsed(tree, f) {
            emit_leaves(f, depth + 1, &mut rows);
        }
    }
    emit_leaves("", 1, &mut rows);
    rows
}

/// The project's `shaders/` tree: folders (explicit directories plus the parents of every asset) and assets.
pub struct ProjectTree;

impl TreeSource for ProjectTree {
    fn id(&self) -> TreeId {
        TreeId::Project
    }

    /// Folders and assets of `shaders/`, then the virtual `Internal` folder.
    fn rows(&self, editor: &Editor) -> Vec<(usize, TreeItem)> {
        let mut ids: Vec<&AssetId> = editor.disk.ids().collect();
        ids.sort();
        let leaves = ids.into_iter().map(|id| (id.path.clone(), TreeItem::Asset(id.clone()))).collect();
        let mut rows = nest(editor, self.id(), editor.folders.iter().cloned().collect(), leaves);
        rows.push((1, TreeItem::Folder(INTERNAL_FOLDER.to_string())));
        if !editor.is_collapsed(self.id(), INTERNAL_FOLDER) {
            rows.extend(INTERNAL_TEXTURES.iter().map(|t| (2, TreeItem::Internal(t.id.to_string()))));
        }
        rows
    }

    fn read_only(&self) -> bool {
        false
    }
}

/// The game's images under `assets/` and the folders holding them (`assets/shaders/` hidden); read-only.
pub struct GameAssetsTree;

impl TreeSource for GameAssetsTree {
    fn id(&self) -> TreeId {
        TreeId::GameAssets
    }

    fn rows(&self, editor: &Editor) -> Vec<(usize, TreeItem)> {
        let leaves = editor.game_files.iter().map(|f| (f.clone(), TreeItem::GameFile(f.clone()))).collect();
        nest(editor, self.id(), BTreeSet::new(), leaves)
    }

    fn read_only(&self) -> bool {
        true
    }
}

/// Whether the row can be renamed / moved / deleted / created into: an editable tree, not the `Internal` folder.
pub fn editable(row: &TreeRow) -> bool {
    let virtual_item = match &row.item {
        TreeItem::Internal(_) => true,
        TreeItem::Folder(f) => f == INTERNAL_FOLDER,
        TreeItem::Asset(_) | TreeItem::GameFile(_) => false,
    };
    !virtual_item && source(row.tree).is_some_and(|s| !s.read_only())
}

/// A row's label: the last path component; roots and the `Internal` folder by name.
pub fn row_label(tree: TreeId, item: &TreeItem) -> String {
    let last = |p: &str| p.rsplit('/').next().unwrap_or(p).to_string();
    match item {
        TreeItem::Folder(f) if f.is_empty() => match tree {
            TreeId::Project => "shaders".into(),
            TreeId::GameAssets => "assets".into(),
        },
        TreeItem::Folder(f) if f == INTERNAL_FOLDER => "Internal".into(),
        TreeItem::Folder(f) | TreeItem::GameFile(f) | TreeItem::Internal(f) => last(f),
        TreeItem::Asset(id) => id.name().to_string(),
    }
}

/// What a primary click (`count` = 2 for a double click) on a row does. A folder toggles (only the editable tree's
/// folders become the target of "New ..."); a double click opens an asset, a game file's read-only image tab or a
/// built-in texture's read-only Texture tab.
pub fn click_actions(row: &TreeRow, count: u8) -> Vec<Action> {
    let mut actions = Vec::new();
    match &row.item {
        TreeItem::Folder(f) => {
            if !f.is_empty() && count < 2 {
                actions.push(Action::ToggleFolder(row.tree, f.clone()));
            }
            if editable(row) {
                actions.push(Action::SelectFolder(f.clone()));
            }
        }
        TreeItem::Asset(id) if count >= 2 => actions.push(Action::OpenAsset(id.clone())),
        TreeItem::GameFile(path) if count >= 2 => actions.push(Action::OpenGameImage(path.clone())),
        TreeItem::Internal(id) if count >= 2 => actions.push(Action::OpenAsset(AssetId::new(AssetType::Texture, id))),
        TreeItem::Asset(_) | TreeItem::GameFile(_) | TreeItem::Internal(_) => {}
    }
    actions
}

/// The RMB menu of a row (empty: no menu). Read-only rows get no create / rename / move / delete; a game file offers
/// "Create Texture Asset", disabled once a Texture asset points at it.
pub fn row_menu(editor: &Editor, row: &TreeRow) -> Vec<MenuEntry> {
    let entries = |items: Vec<(Option<IconId>, String, Action)>| items.into_iter().map(MenuEntry::from).collect();
    match &row.item {
        TreeItem::Folder(f) if editable(row) => {
            let mut items = create_items();
            if !f.is_empty() {
                items.push((Some(IconId::Delete), "Delete folder...".into(), Action::DeleteFolderDialog(f.clone())));
            }
            entries(items)
        }
        TreeItem::Asset(id) if editable(row) => entries(asset_menu(editor, id)),
        TreeItem::GameFile(path) => {
            let disabled = editor
                .texture_for_file(path)
                .map(|other| format!("{} already points at this file", other.source_rel()));
            vec![MenuEntry {
                icon: Some(IconId::Texture),
                label: "Create Texture Asset".into(),
                action: Action::CreateTextureForFile(path.clone()),
                disabled,
            }]
        }
        _ => Vec::new(),
    }
}

fn asset_menu(editor: &Editor, id: &AssetId) -> Vec<(Option<IconId>, String, Action)> {
    let kind = IconId::for_id(id, editor.asset_kind(id));
    let mut items = vec![
        (Some(kind), "Open".to_string(), Action::OpenAsset(id.clone())),
        (Some(IconId::Edit), "Rename...".into(), Action::RenameDialog(id.clone())),
        (Some(IconId::MoveToFolder), "Move to...".into(), Action::MoveDialog(id.clone())),
        (Some(IconId::Delete), "Delete...".into(), Action::DeleteDialog(id.clone())),
    ];
    if id.ty.has_wesl() {
        items.push((Some(IconId::Code), "Show generated WESL".into(), Action::ShowGenerated(id.clone(), Lang::Wesl)));
    }
    if id.ty == AssetType::Material {
        items.push((Some(IconId::Code), "Show generated Rust".into(), Action::ShowGenerated(id.clone(), Lang::Rust)));
    }
    items.push((Some(IconId::File), "Show source (.ron)".into(), Action::ShowFile(id.clone(), GenFile::Source)));
    items
}

/// Dragging row `dragged` onto row `target`: an asset dropped on a folder of the same editable tree moves there.
pub fn drop_action(target: &TreeRow, dragged: &TreeRow) -> Option<Action> {
    if !editable(target) || !editable(dragged) || target.tree != dragged.tree {
        return None;
    }
    match (&target.item, &dragged.item) {
        (TreeItem::Folder(folder), TreeItem::Asset(id)) => {
            let current = id.path.rsplit_once('/').map_or("", |(d, _)| d);
            (current != folder).then(|| Action::MoveTo(id.clone(), folder.clone()))
        }
        _ => None,
    }
}

impl Editor {
    /// Whether `folder` of `tree` is collapsed.
    pub fn is_collapsed(&self, tree: TreeId, folder: &str) -> bool {
        self.collapsed.get(&tree).is_some_and(|c| c.contains(folder))
    }

    /// Whether `path` of `tree` lies inside a collapsed folder.
    pub fn hidden_in(&self, tree: TreeId, path: &str) -> bool {
        self.collapsed.get(&tree).is_some_and(|c| c.iter().any(|f| path.starts_with(&format!("{f}/"))))
    }
}
