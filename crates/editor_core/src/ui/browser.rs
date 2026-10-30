//! Asset browser (spec §9.1), the left sidebar's panes (design §8.1, §14.4), all of the active domain: its
//! `quillshade/<domain>/` tree in the "Project" pane — open (double click), create (RMB menus), rename, move (drag
//! onto a folder), delete; it ends with the domain's virtual read-only folders (Material: Internal, design §2). The
//! asset of the active tab is highlighted; unsaved ones get an accent dot after the name. Below it the read-only
//! "Game Assets" pane lists the game files the domain shows, hidden for a domain without any. With the project domain
//! active, a "Tools" pane of its tools replaces both ([`left_panes`]). Rows come from a [`TreeSource`] and carry a
//! [`TreeRow`]; what clicks, menus and drops do is decided in [`tree`].

use std::collections::BTreeMap;

use bevy::feathers::controls::{ButtonVariant, FeathersButton};
use bevy::prelude::*;
use project_core::{AssetRef, DomainId};

use crate::action::{Action, Actions};
use crate::tab::Tab;
use crate::theme::{ACCENT, TEXT, TEXT_DIM, text};
use crate::ui::HoverRow;
use crate::ui::icons::{IconId, IconRef, icon};

use super::OnActivate;
use super::dialog::{open_icon_menu, open_menu_entries};
use super::shell::ROW_ACTIVE;
use super::tree::{
    self, GameAssetsTree, LeftPanes, ProjectTree, TreeId, TreeItem, TreeRow, TreeSource, left_panes, project_tools,
};
use crate::state::editor::Editor;

/// The Project panel's scroll body.
#[derive(Component, Default, Clone)]
pub struct BrowserBody;

/// The Game Assets panel's scroll body.
#[derive(Component, Default, Clone)]
pub struct GameAssetsBody;

/// The project domain's Tools pane body: one button per project tool (design §14.2).
#[derive(Component, Default, Clone)]
pub struct ProjectToolsBody;

/// Which of the left sidebar's panes a pane is; [`show_panes`] shows the ones of the active domain.
#[derive(Component, Default, Clone, Copy, PartialEq, Eq, Debug)]
pub enum LeftPane {
    #[default]
    Project,
    GameAssets,
    Tools,
}

impl LeftPane {
    /// Whether the pane is shown for `panes`.
    pub fn shown(self, panes: LeftPanes) -> bool {
        match (self, panes) {
            (LeftPane::Project, LeftPanes::Domain { .. } | LeftPanes::Empty) => true,
            (LeftPane::GameAssets, LeftPanes::Domain { game_assets, .. }) => game_assets,
            (LeftPane::GameAssets, LeftPanes::Empty) => true,
            (LeftPane::Tools, LeftPanes::Tools) => true,
            _ => false,
        }
    }
}

pub struct BrowserPlugin;

impl Plugin for BrowserPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (redraw, show_panes, redraw_tools))
            .add_observer(click_row)
            .add_observer(click_background)
            .add_observer(drop_on_folder);
    }
}

const ICON: f32 = 16.0;

/// A row's leading icons: the folder caret (or as much space, to keep icons aligned) and the item's icon.
fn row_icons(caret: Option<IconRef>, item: impl Into<IconRef>) -> Vec<Box<dyn Scene>> {
    let caret: Box<dyn Scene> = match caret {
        Some(c) => Box::new(icon(c, ICON, TEXT_DIM)),
        None => Box::new(bsn! { Node { width: px(ICON), flex_shrink: 0.0 } }),
    };
    vec![caret, Box::new(icon(item, ICON, TEXT))]
}

/// What a row shows besides its label.
#[derive(Clone, Copy, Default)]
struct RowState {
    active: bool,
    error: bool,
    dirty: bool,
}

fn row(label: String, icons: Vec<Box<dyn Scene>>, depth: usize, row: TreeRow, state: RowState) -> impl Scene {
    let RowState { active, error, dirty } = state;
    let base = if active { ROW_ACTIVE } else { Color::NONE };
    let hover = HoverRow { base };
    let pad = px(6.0 + depth as f32 * 14.0);
    let color = if error { Color::srgb(0.95, 0.45, 0.45) } else { TEXT };
    let dot: Vec<Box<dyn Scene>> = if dirty {
        vec![Box::new(bsn! {
            Node { width: px(7), height: px(7), margin: UiRect::left(px(2)), border_radius: BorderRadius::MAX }
            BackgroundColor(ACCENT)
            Pickable::IGNORE
        })]
    } else {
        Vec::new()
    };
    bsn! {
        Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            min_height: px(22),
            padding: UiRect::new(pad, px(6), px(1), px(1)),
            column_gap: px(4),
            border_radius: BorderRadius::all(px(3)),
        }
        BackgroundColor(base)
        hover
        row
        Children [
            {icons}
            --
            @text(label, color, 13.0)
            --
            {dot}
        ]
    }
}

/// The domain whose Project tree the left sidebar shows.
fn shown_tree(editor: &Editor) -> Option<DomainId> {
    match left_panes(editor) {
        LeftPanes::Domain { tree, .. } => Some(tree),
        LeftPanes::Tools | LeftPanes::Empty => None,
    }
}

/// The icons and error marks of the shown Project tree's rows (assets and virtual items), by asset.
fn asset_rows(editor: &Editor) -> BTreeMap<AssetRef, (IconRef, bool)> {
    let mut rows = BTreeMap::new();
    if let Some(id) = shown_tree(editor) {
        let tree = ProjectTree(id);
        let extras = tree.extras(editor).into_iter().flat_map(|f| f.items);
        for item in tree.items(editor).into_iter().chain(extras) {
            rows.insert(item.asset, (item.icon, item.error));
        }
    }
    rows
}

fn redraw(
    editor: Res<Editor>,
    body: Query<Entity, With<BrowserBody>>,
    game_body: Query<Entity, With<GameAssetsBody>>,
    mut last: Local<Option<(u64, Option<DomainId>, Option<AssetRef>, Vec<AssetRef>, Vec<bool>)>>,
    mut errors: Local<Option<(u64, Option<DomainId>, Vec<AssetRef>)>>,
    mut commands: Commands,
) {
    // The rows in error only change with the diagnostics, which bump the log revision, and with the shown tree.
    let tree = shown_tree(&editor);
    if errors.as_ref().is_none_or(|(rev, shown, _)| *rev != editor.ui.log || *shown != tree) {
        let list = asset_rows(&editor).into_iter().filter(|(_, (_, error))| *error).map(|(r, _)| r).collect();
        *errors = Some((editor.ui.log, tree, list));
    }
    let key = (
        editor.ui.browser,
        tree,
        editor.tab().and_then(Tab::owner).cloned(),
        errors.as_ref().map(|(_, _, e)| e.clone()).unwrap_or_default(),
        editor.tabs.iter().map(|t| t.doc().is_dirty()).collect(),
    );
    if last.as_ref() == Some(&key) {
        return;
    }
    let (Ok(body), Ok(game_body)) = (body.single(), game_body.single()) else { return };
    *last = Some(key);
    commands.entity(body).despawn_children();
    commands.entity(game_body).despawn_children();
    if editor.project.is_none() {
        commands.spawn_scene(bsn! { @text("No project open", TEXT_DIM, 13.0) ChildOf(body) });
        return;
    }
    let assets = asset_rows(&editor);
    if let Some(id) = tree {
        draw_tree(&ProjectTree(id), &editor, &assets, body, &mut commands);
        draw_tree(&GameAssetsTree, &editor, &assets, game_body, &mut commands);
    }
}

/// Shows the panes of the active domain ([`left_panes`]): its Project tree and Game Assets (hidden when it shows no
/// game files), or the project domain's Tools. Runs when that changed or panes were spawned.
fn show_panes(
    editor: Res<Editor>,
    mut last: Local<Option<LeftPanes>>,
    spawned: Query<(), Added<LeftPane>>,
    mut panes: Query<(&LeftPane, &mut Node, &mut Visibility)>,
) {
    let shown = left_panes(&editor);
    if *last == Some(shown) && spawned.is_empty() {
        return;
    }
    *last = Some(shown);
    for (pane, mut node, mut visibility) in &mut panes {
        super::areas::display(&mut node, &mut visibility, pane.shown(shown));
    }
}

/// Fills a new Tools pane with a plain Feathers button per project tool (a click opens it), or "No tools"; the tools
/// are registered once, before the app runs.
fn redraw_tools(editor: Res<Editor>, bodies: Query<Entity, Added<ProjectToolsBody>>, mut commands: Commands) {
    let tools = project_tools(&editor);
    for body in &bodies {
        if tools.is_empty() {
            commands.spawn_scene(bsn! { @text("No tools", TEXT_DIM, 13.0) ChildOf(body) });
        }
        for (title, action) in tools.iter().cloned() {
            commands.spawn_scene(bsn! { @tool_row(title, action) ChildOf(body) });
        }
    }
}

/// A project tool in the Tools pane: a plain Feathers button, left-aligned, as wide as the pane.
fn tool_row(title: String, action: Action) -> impl Scene {
    let act = OnActivate(action);
    bsn! {
        @FeathersButton { @variant: ButtonVariant::Plain, @caption: bsn! { @text(title, TEXT, 13.0) } }
        Node { justify_content: JustifyContent::Start, padding: UiRect::axes(px(8), px(4)) }
        act
    }
}

/// Spawns the rows of `source` under `body`; `assets` gives the asset rows' icons and error marks.
fn draw_tree(
    source: &dyn TreeSource,
    editor: &Editor,
    assets: &BTreeMap<AssetRef, (IconRef, bool)>,
    body: Entity,
    commands: &mut Commands,
) {
    let tree = source.id();
    let active = editor.tab().and_then(Tab::owner);
    let file = IconRef::from(IconId::File);
    for (depth, item) in source.rows(editor) {
        let label = tree::row_label(tree, &item);
        let (icons, state) = match &item {
            TreeItem::Folder(f) if f.is_empty() => (row_icons(None, IconId::FolderOpen), RowState::default()),
            TreeItem::Folder(f) | TreeItem::VirtualFolder(f, _) => {
                let icons = if editor.is_collapsed(tree, f) {
                    row_icons(Some(IconId::CaretRight.into()), IconId::Folder)
                } else {
                    row_icons(Some(IconId::CaretDown.into()), IconId::FolderOpen)
                };
                (icons, RowState::default())
            }
            TreeItem::Asset(r) => {
                let (icon, error) = assets.get(r).copied().unwrap_or((file, false));
                let state = RowState {
                    active: active == Some(r),
                    error,
                    dirty: editor.tabs.iter().any(|t| t.owner() == Some(r) && t.doc().is_dirty()),
                };
                (row_icons(None, icon), state)
            }
            TreeItem::GameFile(_) => (row_icons(None, IconId::Image), RowState::default()),
            // A virtual item (a built-in texture) opens as a read-only tab.
            TreeItem::Virtual(r) => (row_icons(None, assets.get(r).map_or(file, |a| a.0)), RowState::default()),
        };
        let r = TreeRow { tree, item };
        commands.spawn_scene(bsn! { @row(label, icons, depth, r, state) ChildOf(body) });
    }
}

fn click_row(
    ev: On<PointerClick>,
    rows: Query<&TreeRow>,
    editor: Res<Editor>,
    mut actions: ResMut<Actions>,
    mut commands: Commands,
) {
    let Ok(row) = rows.get(ev.event_target()) else { return };
    match ev.button {
        PointerButton::Primary => {
            for a in tree::click_actions(row, ev.count) {
                actions.push(a);
            }
        }
        PointerButton::Secondary => {
            let items = tree::row_menu(&editor, row);
            if items.is_empty() {
                return;
            }
            if let (TreeItem::Folder(f), TreeId::Project(domain)) = (&row.item, row.tree) {
                actions.push(Action::SelectFolder(domain, f.clone()));
            }
            open_menu_entries(&mut commands, ev.pointer.position, items);
        }
        _ => {}
    }
}

/// RMB on the empty space below the tree: the root folder's menu.
fn click_background(
    ev: On<PointerClick>,
    body: Query<(), With<BrowserBody>>,
    editor: Res<Editor>,
    mut actions: ResMut<Actions>,
    mut commands: Commands,
) {
    let target = ev.event_target();
    if ev.button != PointerButton::Secondary
        || !body.contains(target)
        || ev.original_event_target() != target
        || editor.project.is_none()
    {
        return;
    }
    // The empty space lies below the shown Project tree: its root takes the menu.
    let Some(domain) = shown_tree(&editor) else { return };
    actions.push(Action::SelectFolder(domain, String::new()));
    open_icon_menu(&mut commands, ev.pointer.position, tree::create_items(&editor, domain));
}

/// Dragging an asset row onto a folder row of the same editable tree moves the asset there.
fn drop_on_folder(ev: On<PointerDragDrop>, rows: Query<&TreeRow>, mut actions: ResMut<Actions>) {
    let (Ok(target), Ok(dragged)) = (rows.get(ev.event_target()), rows.get(ev.dropped)) else { return };
    if let Some(a) = tree::drop_action(target, dragged) {
        actions.push(a);
    }
}
