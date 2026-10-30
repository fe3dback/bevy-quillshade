//! Tree sources of the left sidebar (design §8.1, §14.4): the active domain's Project tree (`quillshade/<domain>/`,
//! built by core from the domain's folders, its [`DomainState::tree_items`](crate::domain::DomainState::tree_items)
//! or core's default rows of its asset kinds, and its virtual `tree_extras` folders) and the read-only Game Assets tree (the game files the
//! active domain's extensions accept); with the project domain active, its tools in their place ([`left_panes`]). Each tree lists its
//! rows as `(depth, TreeItem)`; the browser draws them and handles clicks keyed by [`TreeRow`] through
//! [`click_actions`], [`row_menu`] and [`drop_action`]. Collapse state is kept per tree in `Editor::collapsed`.

use std::collections::BTreeSet;

use crate::action::Action;
pub use crate::action::TreeId;
use crate::menu::MenuEntry;
use crate::ui::icons::{IconId, IconRef};
use bevy::prelude::*;
use project_core::{AssetRef, DomainId};

use crate::domain::{GameAssetsDef, TreeAsset, TreeFolder};
use crate::state::editor::Editor;
use crate::ui::menu::new_asset_items;

/// What a tree row stands for.
#[derive(Clone, PartialEq, Debug)]
pub enum TreeItem {
    /// A folder path relative to the tree root (`""` is the root itself).
    Folder(String),
    /// An editor asset (Project tree).
    Asset(AssetRef),
    /// A game file, path relative to `assets/` (Game Assets tree).
    GameFile(String),
    /// A domain's virtual read-only folder ([`TreeFolder`]): its key (starting with `@`) and label.
    VirtualFolder(String, &'static str),
    /// An item of a virtual folder: opens like an asset; never renamed, moved or deleted.
    Virtual(AssetRef),
}

/// A drawn row of a sidebar tree.
#[derive(Component, Clone, Debug)]
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

/// The source of `tree`.
pub fn source(tree: TreeId) -> Box<dyn TreeSource> {
    match tree {
        TreeId::Project(id) => Box::new(ProjectTree(id)),
        TreeId::GameAssets => Box::new(GameAssetsTree),
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

/// A domain's `quillshade/<domain>/` tree: its folders (directories plus the parents of every asset), its assets,
/// then its virtual folders.
pub struct ProjectTree(pub DomainId);

impl ProjectTree {
    /// The domain's asset rows ([`DomainState::tree_items`](crate::domain::DomainState::tree_items)), else core's
    /// default: the files of its registered asset kinds (design §14.4,
    /// [`default_tree_items`](crate::state::kinds::default_tree_items)).
    pub fn items(&self, editor: &Editor) -> Vec<TreeAsset> {
        let own = editor.domains.get_dyn(self.0).and_then(|s| s.tree_items(editor));
        own.unwrap_or_else(|| crate::state::kinds::default_tree_items(editor, self.0))
    }

    /// The domain's virtual folders.
    pub fn extras(&self, editor: &Editor) -> Vec<TreeFolder> {
        let extras = editor.domain_list.get(self.0).map(|d| d.tree_extras.clone()).unwrap_or_default();
        extras.into_iter().flat_map(|f| f(editor)).collect()
    }
}

impl TreeSource for ProjectTree {
    fn id(&self) -> TreeId {
        TreeId::Project(self.0)
    }

    fn rows(&self, editor: &Editor) -> Vec<(usize, TreeItem)> {
        // In the domain's order inside each folder.
        let leaves = self.items(editor).into_iter().map(|i| (i.asset.path.clone(), TreeItem::Asset(i.asset))).collect();
        let folders = editor.folders.get(&self.0).into_iter().flatten().cloned().collect();
        let mut rows = nest(editor, self.id(), folders, leaves);
        for folder in self.extras(editor) {
            rows.push((1, TreeItem::VirtualFolder(folder.key.to_string(), folder.label)));
            if !editor.is_collapsed(self.id(), folder.key) {
                rows.extend(folder.items.into_iter().map(|i| (2, TreeItem::Virtual(i.asset))));
            }
        }
        rows
    }

    fn read_only(&self) -> bool {
        false
    }
}

/// The game files the active domain shows under `assets/` and the folders holding them (`assets/quillshade/`
/// hidden); read-only. Empty while the active domain shows none (its pane is hidden, [`left_panes`]).
pub struct GameAssetsTree;

impl TreeSource for GameAssetsTree {
    fn id(&self) -> TreeId {
        TreeId::GameAssets
    }

    fn rows(&self, editor: &Editor) -> Vec<(usize, TreeItem)> {
        let Some(def) = active_game_assets(editor) else { return Vec::new() };
        let files = editor.game_files.iter().filter(|f| def.accepts(f));
        let leaves = files.map(|f| (f.clone(), TreeItem::GameFile(f.clone()))).collect();
        nest(editor, self.id(), BTreeSet::new(), leaves)
    }

    fn read_only(&self) -> bool {
        true
    }
}

/// Whether the row can be renamed / moved / deleted / created into: a folder or asset of an editable tree, not a
/// virtual folder or its items.
pub fn editable(row: &TreeRow) -> bool {
    let real = matches!(row.item, TreeItem::Folder(_) | TreeItem::Asset(_));
    real && !source(row.tree).read_only()
}

/// A row's label: the last path component; roots by name (a Project tree's by its domain id), virtual folders by
/// their label.
pub fn row_label(tree: TreeId, item: &TreeItem) -> String {
    let last = |p: &str| p.rsplit('/').next().unwrap_or(p).to_string();
    match item {
        TreeItem::Folder(f) if f.is_empty() => match tree {
            TreeId::Project(id) => id.to_string(),
            TreeId::GameAssets => "assets".into(),
        },
        TreeItem::VirtualFolder(_, label) => label.to_string(),
        TreeItem::Folder(f) | TreeItem::GameFile(f) => last(f),
        TreeItem::Asset(r) | TreeItem::Virtual(r) => last(&r.path),
    }
}

/// What a primary click (`count` = 2 for a double click) on a row does. A folder toggles (only the editable tree's
/// folders become the target of "New ..."); a double click opens an asset or a virtual item, or a game file through
/// the active domain.
pub fn click_actions(row: &TreeRow, count: u8) -> Vec<Action> {
    let mut actions = Vec::new();
    match &row.item {
        TreeItem::Folder(f) | TreeItem::VirtualFolder(f, _) => {
            if !f.is_empty() && count < 2 {
                actions.push(Action::ToggleFolder(row.tree, f.clone()));
            }
            if let (true, TreeId::Project(domain)) = (editable(row), row.tree) {
                actions.push(Action::SelectFolder(domain, f.clone()));
            }
        }
        TreeItem::Asset(r) | TreeItem::Virtual(r) if count >= 2 => actions.push(Action::OpenAsset(r.clone())),
        TreeItem::GameFile(path) if count >= 2 => actions.push(Action::OpenGameImage(path.clone())),
        TreeItem::Asset(_) | TreeItem::GameFile(_) | TreeItem::Virtual(_) => {}
    }
    actions
}

/// The "New ..." items of a Project tree's folder menu: the domain's templates and New folder, created in the
/// selected folder.
pub fn create_items(editor: &Editor, domain: DomainId) -> Vec<(Option<IconRef>, String, Action)> {
    let mut items = new_asset_items(editor, Some(domain));
    items.push((Some(IconId::FolderNew.into()), "New folder...".into(), Action::NewFolderDialog(Some(domain))));
    items
}

/// The icon of a Project tree asset row (its kind's when the domain does not list it).
pub fn asset_icon(editor: &Editor, r: &AssetRef) -> IconRef {
    let listed = ProjectTree(r.domain).items(editor).into_iter().find(|i| i.asset == *r).map(|i| i.icon);
    let kind = || editor.domain_list.asset_kind(r.domain, r.suffix).map(|k| k.icon);
    listed.or_else(kind).unwrap_or(IconId::File.into())
}

/// The RMB menu of a row (empty: no menu). Read-only rows get no create / rename / move / delete; an asset adds its
/// kind's items, a game file the active domain's items (design §14.4: two texture domains never both offer "Create
/// Texture Asset").
pub fn row_menu(editor: &Editor, row: &TreeRow) -> Vec<MenuEntry> {
    let entries = |items: Vec<(Option<IconRef>, String, Action)>| items.into_iter().map(MenuEntry::from).collect();
    match (&row.item, row.tree) {
        (TreeItem::Folder(f), TreeId::Project(domain)) if editable(row) => {
            let mut items = create_items(editor, domain);
            if !f.is_empty() {
                items.push((
                    Some(IconId::Delete.into()),
                    "Delete folder...".into(),
                    Action::DeleteFolderDialog(domain, f.clone()),
                ));
            }
            entries(items)
        }
        (TreeItem::Asset(r), _) if editable(row) => {
            let mut menu: Vec<MenuEntry> = entries(vec![
                (Some(asset_icon(editor, r)), "Open".to_string(), Action::OpenAsset(r.clone())),
                (Some(IconId::Edit.into()), "Rename...".into(), Action::RenameDialog(r.clone())),
                (Some(IconId::MoveToFolder.into()), "Move to...".into(), Action::MoveDialog(r.clone())),
                (Some(IconId::Delete.into()), "Delete...".into(), Action::DeleteDialog(r.clone())),
            ]);
            if let Some(kind) = editor.domain_list.asset_kind(r.domain, r.suffix) {
                menu.extend((kind.row_menu)(editor, r));
            }
            menu
        }
        (TreeItem::GameFile(path), _) => match active_game_assets(editor).filter(|g| g.accepts(path)) {
            Some(g) => (g.menu)(editor, path),
            None => Vec::new(),
        },
        _ => Vec::new(),
    }
}

/// Dragging row `dragged` onto row `target`: an asset dropped on a folder of the same editable tree moves there.
pub fn drop_action(target: &TreeRow, dragged: &TreeRow) -> Option<Action> {
    if !editable(target) || !editable(dragged) || target.tree != dragged.tree {
        return None;
    }
    match (&target.item, &dragged.item) {
        (TreeItem::Folder(folder), TreeItem::Asset(r)) => {
            let current = r.path.rsplit_once('/').map_or("", |(d, _)| d);
            (current != folder).then(|| Action::MoveTo(r.clone(), folder.clone()))
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

/// What the left sidebar shows (design §8.1, §14.4).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LeftPanes {
    /// A real domain is active: its Project tree and, if it shows game files, the Game Assets pane.
    Domain { tree: DomainId, game_assets: bool },
    /// The project domain is active: its tools ([`project_tools`]) in place of both panes.
    Tools,
    /// No active domain (no project open).
    Empty,
}

/// The left sidebar of the active domain.
pub fn left_panes(editor: &Editor) -> LeftPanes {
    match editor.active_domain {
        Some(DomainId::PROJECT) => LeftPanes::Tools,
        Some(tree) => LeftPanes::Domain { tree, game_assets: editor.domain_list.game_assets_of(tree).is_some() },
        None => LeftPanes::Empty,
    }
}

/// The Game Assets definition of the active domain: the files the Game Assets tree lists, its rows' menu and double
/// click (design §14.4).
pub fn active_game_assets(editor: &Editor) -> Option<&GameAssetsDef> {
    editor.active_domain.and_then(|d| editor.domain_list.game_assets_of(d))
}

/// The rows of the project domain's left sidebar (design §14.2): each project tool's title and the action opening it,
/// in registration order.
pub fn project_tools(editor: &Editor) -> Vec<(String, Action)> {
    let tools = editor.domain_list.tools(DomainId::PROJECT).iter();
    tools.map(|t| (t.title.to_string(), Action::OpenTool { domain: DomainId::PROJECT, id: t.id, arg: None })).collect()
}
