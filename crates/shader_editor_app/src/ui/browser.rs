//! Asset browser (spec §9.1), the sidebar's "Project" panel: `shaders/` tree, open (double click), create (RMB menus),
//! rename, move (drag onto a folder), delete; it ends with the virtual read-only `Internal` folder (design §2). The
//! asset of the active tab is highlighted; unsaved ones get an accent dot after the name. Below it the read-only
//! "Game Assets" panel lists the game's images (design §4.2). Rows come from a [`TreeSource`] and carry a
//! [`TreeRow`]; what clicks, menus and drops do is decided in [`tree`].

use std::collections::BTreeSet;

use bevy::prelude::*;
use shader_project::AssetId;

use super::dialog::{open_icon_menu, open_menu_entries};
use super::icons::{IconId, icon};
use super::tree::{self, GameAssetsTree, ProjectTree, TreeItem, TreeRow, TreeSource};
use super::{ACCENT, HoverRow, ROW_ACTIVE, TEXT, TEXT_DIM, text};
use crate::state::{Action, Actions, Editor, NewKind};
use crate::tab::Tab;

/// The Project panel's scroll body.
#[derive(Component, Default, Clone)]
pub struct BrowserBody;

/// The Game Assets panel's scroll body.
#[derive(Component, Default, Clone)]
pub struct GameAssetsBody;

pub struct BrowserPlugin;

impl Plugin for BrowserPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, redraw)
            .add_observer(click_row)
            .add_observer(click_background)
            .add_observer(drop_on_folder);
    }
}

const ICON: f32 = 16.0;

/// A row's leading icons: the folder caret (or as much space, to keep icons aligned) and the item's icon.
fn row_icons(caret: Option<IconId>, item: IconId) -> Vec<Box<dyn Scene>> {
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

fn redraw(
    editor: Res<Editor>,
    body: Query<Entity, With<BrowserBody>>,
    game_body: Query<Entity, With<GameAssetsBody>>,
    mut last: Local<Option<(u64, Option<AssetId>, usize, Vec<bool>)>>,
    mut commands: Commands,
) {
    let key = (
        editor.ui.browser,
        editor.tab().and_then(Tab::owner).cloned(),
        editor.diags.values().map(|v| v.iter().filter(|d| d.is_error()).count()).sum(),
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
    draw_tree(&ProjectTree, &editor, body, &mut commands);
    draw_tree(&GameAssetsTree, &editor, game_body, &mut commands);
}

/// Spawns the rows of `source` under `body`.
fn draw_tree(source: &dyn TreeSource, editor: &Editor, body: Entity, commands: &mut Commands) {
    let tree = source.id();
    let active = editor.tab().and_then(Tab::owner).cloned();
    let errors: BTreeSet<&AssetId> =
        editor.diags.iter().filter(|(_, v)| v.iter().any(|d| d.is_error())).map(|(k, _)| k).collect();
    for (depth, item) in source.rows(editor) {
        let label = tree::row_label(tree, &item);
        let (icons, state) = match &item {
            TreeItem::Folder(f) if f.is_empty() => (row_icons(None, IconId::FolderOpen), RowState::default()),
            TreeItem::Folder(f) => {
                let icons = if editor.is_collapsed(tree, f) {
                    row_icons(Some(IconId::CaretRight), IconId::Folder)
                } else {
                    row_icons(Some(IconId::CaretDown), IconId::FolderOpen)
                };
                (icons, RowState::default())
            }
            TreeItem::Asset(id) => {
                let state = RowState {
                    active: active.as_ref() == Some(id),
                    error: errors.contains(id),
                    dirty: editor.tabs.iter().any(|t| t.owner() == Some(id) && t.doc().is_dirty()),
                };
                (row_icons(None, IconId::for_id(id, editor.asset_kind(id))), state)
            }
            TreeItem::GameFile(_) => (row_icons(None, IconId::Image), RowState::default()),
            // A built-in texture opens as a (read-only) Texture tab.
            TreeItem::Internal(_) => (row_icons(None, IconId::Texture), RowState::default()),
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
            if let TreeItem::Folder(f) = &row.item {
                actions.push(Action::SelectFolder(f.clone()));
            }
            open_menu_entries(&mut commands, ev.pointer.position, items);
        }
        _ => {}
    }
}

/// "New ..." items of the folder menus (and the main menu's Assets): created in the selected folder.
pub fn create_items() -> Vec<(Option<IconId>, String, Action)> {
    let mut items: Vec<(Option<IconId>, String, Action)> = NewKind::ALL
        .iter()
        .map(|k| (Some(IconId::for_new(*k)), format!("New {}...", k.label()), Action::NewAssetDialog(*k)))
        .collect();
    items.push((Some(IconId::FolderNew), "New folder...".into(), Action::NewFolderDialog));
    items
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
    actions.push(Action::SelectFolder(String::new()));
    open_icon_menu(&mut commands, ev.pointer.position, create_items());
}

/// Dragging an asset row onto a folder row of the same editable tree moves the asset there.
fn drop_on_folder(ev: On<PointerDragDrop>, rows: Query<&TreeRow>, mut actions: ResMut<Actions>) {
    let (Ok(target), Ok(dragged)) = (rows.get(ev.event_target()), rows.get(ev.dropped)) else { return };
    if let Some(a) = tree::drop_action(target, dragged) {
        actions.push(a);
    }
}
