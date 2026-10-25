//! Asset browser (spec §9.1): `shaders/` tree, open, create, rename, move (drag onto a folder), delete.

use std::collections::BTreeSet;

use bevy::prelude::*;
use shader_graph_core::model::{AssetKind, MaterialKind};
use shader_project::{AssetId, AssetType};

use super::dialog::open_menu;
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

fn kind_tag(kind: Option<AssetKind>) -> &'static str {
    match kind {
        Some(AssetKind::Function) => "fn",
        Some(AssetKind::Material(MaterialKind::Pbr3d(_))) => "pbr",
        Some(AssetKind::Material(MaterialKind::Unlit3d(_))) => "unlit",
        Some(AssetKind::Material(MaterialKind::Sprite2d(_))) => "2d",
        None => "?",
    }
}

fn row(label: String, tag: String, depth: usize, row: BrowserRow, active: bool, error: bool) -> impl Scene {
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
            column_gap: px(6),
            border_radius: BorderRadius::all(px(3)),
        }
        BackgroundColor(base)
        hover
        row
        Children [
            @text(label, color, 13.0)
            --
            Node { flex_grow: 1.0 }
            --
            @text(tag, TEXT_DIM, 11.0)
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
    commands
        .spawn_scene(bsn! { @row("shaders/".into(), String::new(), 0, root_row, root_active, false) ChildOf(body) });

    // Depth-first: each folder, then its assets; root assets last.
    let emit_assets = |dir: &str, depth: usize, commands: &mut Commands| {
        for (id, _) in
            editor.disk.assets.iter().filter(|(id, _)| id.path.rsplit_once('/').map_or("", |(d, _)| d) == dir)
        {
            if hidden(&id.path) {
                continue;
            }
            let tag = kind_tag(editor.asset_kind(id)).to_string();
            let dirty = editor.doc_index(id).is_some_and(|i| editor.docs[i].is_dirty());
            let label = format!("{}{}", id.name(), if dirty { " *" } else { "" });
            let r = BrowserRow { folder: None, asset: Some(id.clone()) };
            let (is_active, err) = (active.as_ref() == Some(id), errors.contains(id));
            commands.spawn_scene(bsn! { @row(label, tag, depth, r, is_active, err) ChildOf(body) });
        }
    };
    for f in &folders {
        if hidden(f) {
            continue;
        }
        let depth = f.matches('/').count() + 1;
        let name = f.rsplit('/').next().unwrap_or(f);
        let arrow = if editor.collapsed.contains(f) { ">" } else { "v" };
        let r = BrowserRow { folder: Some(f.clone()), asset: None };
        let is_current = editor.current_folder == *f;
        commands.spawn_scene(
            bsn! { @row(format!("{arrow} {name}/"), String::new(), depth, r, is_current, false) ChildOf(body) },
        );
        if !editor.collapsed.contains(f) {
            emit_assets(f, depth + 1, &mut commands);
        }
    }
    emit_assets("", 1, &mut commands);
}

fn click_row(ev: On<PointerClick>, rows: Query<&BrowserRow>, mut actions: ResMut<Actions>, mut commands: Commands) {
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
            let mut items: Vec<(String, Action)> =
                NewKind::ALL.iter().map(|k| (format!("New {}...", k.label()), Action::NewAssetDialog(*k))).collect();
            items.push(("New folder...".into(), Action::NewFolderDialog));
            if !f.is_empty() {
                items.push(("Delete folder...".into(), Action::DeleteFolderDialog(f.clone())));
            }
            actions.push(Action::SelectFolder(f.clone()));
            open_menu(&mut commands, pos, items);
        }
        (PointerButton::Secondary, _, Some(id)) => {
            let mut items = vec![
                ("Open".to_string(), Action::OpenAsset(id.clone())),
                ("Rename...".into(), Action::RenameDialog(id.clone())),
                ("Move to...".into(), Action::MoveDialog(id.clone())),
                ("Delete...".into(), Action::DeleteDialog(id.clone())),
                ("Show generated WESL".into(), Action::ShowFile(id.clone(), GenFile::Wesl)),
            ];
            if id.ty == AssetType::Material {
                items.push(("Show generated Rust".into(), Action::ShowFile(id.clone(), GenFile::Rust)));
            }
            items.push(("Show source (.ron)".into(), Action::ShowFile(id.clone(), GenFile::Source)));
            open_menu(&mut commands, pos, items);
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
