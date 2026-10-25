//! Asset browser (spec §9.1): `shaders/` tree, open (double click), create (RMB menus), rename, move (drag onto a
//! folder), delete. The asset of the active tab is highlighted; unsaved ones get an accent dot after the name.

use std::collections::BTreeSet;

use bevy::prelude::*;
use shader_project::{AssetId, AssetType};

use super::dialog::open_icon_menu;
use super::icons::{IconId, icon};
use super::{ACCENT, HoverRow, ROW_ACTIVE, TEXT, TEXT_DIM, text};
use crate::state::{Action, Actions, Editor, GenFile, NewKind};
use crate::tab::{Lang, Tab};

#[derive(Component, Default, Clone)]
pub struct BrowserBody;

/// What a browser row stands for.
#[derive(Component, Default, Clone)]
pub struct BrowserRow {
    pub folder: Option<String>,
    pub asset: Option<AssetId>,
}

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

fn row(label: String, icons: Vec<Box<dyn Scene>>, depth: usize, row: BrowserRow, state: RowState) -> impl Scene {
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
    mut last: Local<Option<(u64, Option<AssetId>, usize, Vec<bool>)>>,
    mut commands: Commands,
) {
    let key = (
        editor.ui.browser,
        editor.tab().map(|t| t.asset().clone()),
        editor.diags.values().map(|v| v.iter().filter(|d| d.is_error()).count()).sum(),
        editor.tabs.iter().map(Tab::is_dirty).collect(),
    );
    if last.as_ref() == Some(&key) {
        return;
    }
    let Ok(body) = body.single() else { return };
    *last = Some(key);
    commands.entity(body).despawn_children();
    if editor.project.is_none() {
        commands.spawn_scene(bsn! { @text("No project open", TEXT_DIM, 13.0) ChildOf(body) });
        return;
    }
    let active = editor.tab().map(|t| t.asset().clone());
    let errors: BTreeSet<&AssetId> =
        editor.diags.iter().filter(|(_, v)| v.iter().any(|d| d.is_error())).map(|(k, _)| k).collect();

    // Folders: explicit directories plus the parents of every asset.
    let mut folders: BTreeSet<String> = editor.folders.iter().cloned().collect();
    for id in editor.disk.ids() {
        let mut p = id.path.as_str();
        while let Some((dir, _)) = p.rsplit_once('/') {
            folders.insert(dir.to_string());
            p = dir;
        }
    }
    let hidden = |path: &str| editor.collapsed.iter().any(|c| path.starts_with(&format!("{c}/")));
    let root_row = BrowserRow { folder: Some(String::new()), asset: None };
    let root_icons = row_icons(None, IconId::FolderOpen);
    commands.spawn_scene(bsn! { @row("shaders".into(), root_icons, 0, root_row, RowState::default()) ChildOf(body) });

    // Depth-first: each folder, then its assets; root assets last.
    let emit_assets = |dir: &str, depth: usize, commands: &mut Commands| {
        let mut ids: Vec<&AssetId> =
            editor.disk.ids().filter(|id| id.path.rsplit_once('/').map_or("", |(d, _)| d) == dir).collect();
        ids.sort();
        for id in ids {
            if hidden(&id.path) {
                continue;
            }
            let icons = row_icons(None, IconId::for_id(id, editor.asset_kind(id)));
            let state = RowState {
                active: active.as_ref() == Some(id),
                error: errors.contains(id),
                dirty: editor.tabs.iter().any(|t| t.asset() == id && t.is_dirty()),
            };
            let r = BrowserRow { folder: None, asset: Some(id.clone()) };
            commands.spawn_scene(bsn! { @row(id.name().to_string(), icons, depth, r, state) ChildOf(body) });
        }
    };
    for f in &folders {
        if hidden(f) {
            continue;
        }
        let depth = f.matches('/').count() + 1;
        let name = f.rsplit('/').next().unwrap_or(f);
        let collapsed = editor.collapsed.contains(f);
        let icons = if collapsed {
            row_icons(Some(IconId::CaretRight), IconId::Folder)
        } else {
            row_icons(Some(IconId::CaretDown), IconId::FolderOpen)
        };
        let r = BrowserRow { folder: Some(f.clone()), asset: None };
        commands.spawn_scene(bsn! { @row(name.to_string(), icons, depth, r, RowState::default()) ChildOf(body) });
        if !collapsed {
            emit_assets(f, depth + 1, &mut commands);
        }
    }
    emit_assets("", 1, &mut commands);
}

fn click_row(
    ev: On<PointerClick>,
    rows: Query<&BrowserRow>,
    editor: Res<Editor>,
    mut actions: ResMut<Actions>,
    mut commands: Commands,
) {
    let Ok(row) = rows.get(ev.event_target()) else { return };
    let pos = ev.pointer.position;
    match (ev.button, &row.folder, &row.asset) {
        (PointerButton::Primary, Some(f), _) => {
            if !f.is_empty() && ev.count < 2 {
                actions.push(Action::ToggleFolder(f.clone()));
            }
            actions.push(Action::SelectFolder(f.clone()));
        }
        (PointerButton::Primary, _, Some(id)) if ev.count >= 2 => actions.push(Action::OpenAsset(id.clone())),
        (PointerButton::Secondary, Some(f), _) => {
            let mut items = create_items();
            if !f.is_empty() {
                items.push((Some(IconId::Delete), "Delete folder...".into(), Action::DeleteFolderDialog(f.clone())));
            }
            actions.push(Action::SelectFolder(f.clone()));
            open_icon_menu(&mut commands, pos, items);
        }
        (PointerButton::Secondary, _, Some(id)) => {
            let kind = IconId::for_id(id, editor.asset_kind(id));
            let mut items = vec![
                (Some(kind), "Open".to_string(), Action::OpenAsset(id.clone())),
                (Some(IconId::Edit), "Rename...".into(), Action::RenameDialog(id.clone())),
                (Some(IconId::MoveToFolder), "Move to...".into(), Action::MoveDialog(id.clone())),
                (Some(IconId::Delete), "Delete...".into(), Action::DeleteDialog(id.clone())),
                (Some(IconId::Code), "Show generated WESL".into(), Action::ShowGenerated(id.clone(), Lang::Wesl)),
            ];
            if id.ty == AssetType::Material {
                items.push((
                    Some(IconId::Code),
                    "Show generated Rust".into(),
                    Action::ShowGenerated(id.clone(), Lang::Rust),
                ));
            }
            items.push((
                Some(IconId::File),
                "Show source (.ron)".into(),
                Action::ShowFile(id.clone(), GenFile::Source),
            ));
            open_icon_menu(&mut commands, pos, items);
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

/// Dragging an asset row onto a folder row moves the asset there.
fn drop_on_folder(ev: On<PointerDragDrop>, rows: Query<&BrowserRow>, mut actions: ResMut<Actions>) {
    let (Ok(target), Ok(dragged)) = (rows.get(ev.event_target()), rows.get(ev.dropped)) else { return };
    if let (Some(folder), Some(id)) = (&target.folder, &dragged.asset) {
        let current = id.path.rsplit_once('/').map_or("", |(d, _)| d);
        if current != folder {
            actions.push(Action::MoveTo(id.clone(), folder.clone()));
        }
    }
}
