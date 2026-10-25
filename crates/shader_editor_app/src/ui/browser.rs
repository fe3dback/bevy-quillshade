//! Asset browser (spec §9.1): `shaders/` tree, open, create, rename, move (drag onto a folder), delete.

use std::collections::BTreeSet;

use bevy::prelude::*;
use shader_project::{AssetId, AssetType};

use super::dialog::open_icon_menu;
use super::icons::{IconId, icon};
use super::{HoverRow, ROW_ACTIVE, TEXT, TEXT_DIM, text};
use crate::state::{Action, Actions, Editor, GenFile, NewKind};

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
        app.add_systems(Update, redraw).add_observer(click_row).add_observer(drop_on_folder);
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

fn row(
    label: String,
    icons: Vec<Box<dyn Scene>>,
    depth: usize,
    row: BrowserRow,
    active: bool,
    error: bool,
) -> impl Scene {
    let base = if active { ROW_ACTIVE } else { Color::NONE };
    let hover = HoverRow { base };
    let pad = px(6.0 + depth as f32 * 14.0);
    let color = if error { Color::srgb(0.95, 0.45, 0.45) } else { TEXT };
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
        editor.doc().map(|d| d.id.clone()),
        editor.diags.values().map(|v| v.iter().filter(|d| d.is_error()).count()).sum(),
        editor.docs.iter().map(|d| d.is_dirty()).collect(),
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
    let active = editor.doc().map(|d| d.id.clone());
    let errors: BTreeSet<&AssetId> =
        editor.diags.iter().filter(|(_, v)| v.iter().any(|d| d.is_error())).map(|(k, _)| k).collect();

    // Folders: explicit directories plus the parents of every asset.
    let mut folders: BTreeSet<String> = editor.folders.iter().cloned().collect();
    for id in editor.disk.assets.keys() {
        let mut p = id.path.as_str();
        while let Some((dir, _)) = p.rsplit_once('/') {
            folders.insert(dir.to_string());
            p = dir;
        }
    }
    let hidden = |path: &str| editor.collapsed.iter().any(|c| path.starts_with(&format!("{c}/")));
    let root_row = BrowserRow { folder: Some(String::new()), asset: None };
    let root_active = editor.current_folder.is_empty();
    let root_icons = row_icons(None, IconId::FolderOpen);
    commands.spawn_scene(bsn! { @row("shaders".into(), root_icons, 0, root_row, root_active, false) ChildOf(body) });

    // Depth-first: each folder, then its assets; root assets last.
    let emit_assets = |dir: &str, depth: usize, commands: &mut Commands| {
        for (id, _) in
            editor.disk.assets.iter().filter(|(id, _)| id.path.rsplit_once('/').map_or("", |(d, _)| d) == dir)
        {
            if hidden(&id.path) {
                continue;
            }
            let icons = row_icons(None, IconId::for_asset(editor.asset_kind(id)));
            let dirty = editor.doc_index(id).is_some_and(|i| editor.docs[i].is_dirty());
            let label = format!("{}{}", id.name(), if dirty { " *" } else { "" });
            let r = BrowserRow { folder: None, asset: Some(id.clone()) };
            let (is_active, err) = (active.as_ref() == Some(id), errors.contains(id));
            commands.spawn_scene(bsn! { @row(label, icons, depth, r, is_active, err) ChildOf(body) });
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
        let is_current = editor.current_folder == *f;
        commands.spawn_scene(bsn! { @row(name.to_string(), icons, depth, r, is_current, false) ChildOf(body) });
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
        (PointerButton::Primary, _, Some(id)) => actions.push(Action::OpenAsset(id.clone())),
        (PointerButton::Secondary, Some(f), _) => {
            let mut items: Vec<(Option<IconId>, String, Action)> = NewKind::ALL
                .iter()
                .map(|k| (Some(IconId::for_new(*k)), format!("New {}...", k.label()), Action::NewAssetDialog(*k)))
                .collect();
            items.push((Some(IconId::FolderNew), "New folder...".into(), Action::NewFolderDialog));
            if !f.is_empty() {
                items.push((Some(IconId::Delete), "Delete folder...".into(), Action::DeleteFolderDialog(f.clone())));
            }
            actions.push(Action::SelectFolder(f.clone()));
            open_icon_menu(&mut commands, pos, items);
        }
        (PointerButton::Secondary, _, Some(id)) => {
            let kind = IconId::for_asset(editor.asset_kind(id));
            let mut items = vec![
                (Some(kind), "Open".to_string(), Action::OpenAsset(id.clone())),
                (Some(IconId::Edit), "Rename...".into(), Action::RenameDialog(id.clone())),
                (Some(IconId::MoveToFolder), "Move to...".into(), Action::MoveDialog(id.clone())),
                (Some(IconId::Delete), "Delete...".into(), Action::DeleteDialog(id.clone())),
                (Some(IconId::Code), "Show generated WESL".into(), Action::ShowFile(id.clone(), GenFile::Wesl)),
            ];
            if id.ty == AssetType::Material {
                items.push((
                    Some(IconId::Code),
                    "Show generated Rust".into(),
                    Action::ShowFile(id.clone(), GenFile::Rust),
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
