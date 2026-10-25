//! Node palette popup (spec §9.3): right click / Tab on the canvas or a wire dropped on empty space.

use bevy::feathers::controls::{FeathersTextInput, FeathersTextInputContainer};
use bevy::input_focus::{AutoFocus, InputFocus};
use bevy::prelude::*;
use bevy::text::EditableText;
use bevy::ui_widgets::ScrollArea;

use super::{HoverRow, ROW_ACTIVE, TEXT, TEXT_DIM, text};
use crate::search::{self, Entry, WireEnd};
use crate::state::{Action, Actions, Editor, Link};

const MAX_ROWS: usize = 60;

/// Open palette: where it is and what it may connect to.
#[derive(Resource, Default)]
pub struct Palette {
    pub open: bool,
    /// Window position (logical px).
    pub screen: Vec2,
    /// Graph position for the new node.
    pub graph: Vec2,
    pub link: Option<Link>,
    entries: Vec<Entry>,
    results: Vec<String>,
    query: Option<String>,
    highlight: usize,
    rows_for: Option<(Option<String>, usize)>,
}

impl Palette {
    pub fn show(&mut self, screen: Vec2, graph: Vec2, link: Option<Link>) {
        *self = Palette { open: true, screen, graph, link, ..Default::default() };
    }
}

#[derive(Component, Default, Clone)]
pub struct PaletteRoot;

#[derive(Component, Default, Clone)]
pub struct PaletteInput;

#[derive(Component, Default, Clone)]
pub struct PaletteList;

#[derive(Component, Default, Clone)]
pub struct PaletteRow(pub usize);

pub struct PalettePlugin;

impl Plugin for PalettePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Palette>()
            .add_systems(Update, (spawn_or_close, filter, keys, rows).chain())
            .add_observer(click_row)
            .add_observer(click_outside);
    }
}

fn spawn_or_close(
    mut palette: ResMut<Palette>,
    editor: Res<Editor>,
    roots: Query<Entity, With<PaletteRoot>>,
    window: Query<&Window>,
    mut commands: Commands,
) {
    let exists = !roots.is_empty();
    if !palette.open || editor.doc().is_none() {
        palette.open = false;
        for r in &roots {
            commands.entity(r).despawn();
        }
        return;
    }
    if exists {
        return;
    }
    // Entries for this asset, filtered by the wire's type.
    let d = editor.doc().expect("checked");
    let mut entries = search::entries(&editor.registry, &editor.functions, d.asset.kind, &d.id.path);
    if let Some(link) = &palette.link
        && let Some(t) = editor.pin_type(&link.node, &link.pin, link.from_output)
    {
        let end = if link.from_output { WireEnd::FromOutput(t) } else { WireEnd::FromInput(t) };
        entries.retain(|e| {
            search::compatible_pin(&editor.registry, &editor.functions, d.asset.kind, &e.kind, end).is_some()
        });
    }
    palette.entries = entries;
    palette.query = None;
    let size = window.single().map(|w| Vec2::new(w.width(), w.height())).unwrap_or(Vec2::new(1600.0, 900.0));
    let x = palette.screen.x.min(size.x - 340.0).max(0.0);
    let y = palette.screen.y.min(size.y - 440.0).max(0.0);
    let hint = match &palette.link {
        Some(_) => "Compatible nodes (auto-connect)",
        None => "Add node",
    };
    commands.spawn_scene(bsn! {
        PaletteRoot
        Node { position_type: PositionType::Absolute, left: px(0), top: px(0), width: percent(100), height: percent(100) }
        GlobalZIndex(90)
        Children [
            Node {
                position_type: PositionType::Absolute,
                left: px(x),
                top: px(y),
                width: px(330),
                max_height: px(430),
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(px(6)),
                row_gap: px(4),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(5)),
            }
            BackgroundColor(Color::srgb_u8(34, 36, 42))
            BorderColor::all(Color::srgb_u8(80, 84, 95))
            BoxShadow::new(Color::srgba(0.0, 0.0, 0.0, 0.6), px(0), px(4), px(0), px(14))
            Children [
                @text(hint, TEXT_DIM, 11.0)
                --
                @FeathersTextInputContainer Node { flex_grow: 0.0, flex_shrink: 0.0 } Children [ @FeathersTextInput PaletteInput AutoFocus ]
                --
                PaletteList
                Node { flex_direction: FlexDirection::Column, overflow: Overflow::scroll_y(), min_height: px(0), flex_shrink: 1.0 }
                ScrollArea
            ]
        ]
    });
}

fn filter(mut palette: ResMut<Palette>, inputs: Query<&EditableText, With<PaletteInput>>) {
    if !palette.open {
        return;
    }
    let q = inputs.iter().next().map(|t| t.value().to_string()).unwrap_or_default();
    if palette.query.as_deref() == Some(q.as_str()) {
        return;
    }
    let results: Vec<String> =
        search::search(&palette.entries, &q).into_iter().take(MAX_ROWS).map(|e| e.kind.clone()).collect();
    palette.results = results;
    palette.query = Some(q);
    palette.highlight = 0;
}

fn pick(palette: &mut Palette, actions: &mut Actions, i: usize) {
    if let Some(kind) = palette.results.get(i) {
        let pos = (palette.graph.x, palette.graph.y);
        actions.push(Action::AddNode { kind: kind.clone(), pos, link: palette.link.clone() });
    }
    palette.open = false;
}

fn keys(
    keys: Res<ButtonInput<KeyCode>>,
    mut palette: ResMut<Palette>,
    mut actions: ResMut<Actions>,
    mut focus: ResMut<InputFocus>,
) {
    if !palette.open {
        return;
    }
    let n = palette.results.len();
    if keys.just_pressed(KeyCode::Escape) {
        palette.open = false;
        focus.clear();
    } else if keys.just_pressed(KeyCode::ArrowDown) && n > 0 {
        palette.highlight = (palette.highlight + 1) % n;
    } else if keys.just_pressed(KeyCode::ArrowUp) && n > 0 {
        palette.highlight = (palette.highlight + n - 1) % n;
    } else if keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::NumpadEnter) {
        let i = palette.highlight;
        pick(&mut palette, &mut actions, i);
        focus.clear();
    }
}

fn rows(mut palette: ResMut<Palette>, list: Query<Entity, With<PaletteList>>, mut commands: Commands) {
    if !palette.open {
        palette.rows_for = None;
        return;
    }
    let Ok(list) = list.single() else { return };
    let key = (palette.query.clone(), palette.highlight);
    if palette.rows_for.as_ref() == Some(&key) {
        return;
    }
    palette.rows_for = Some(key);
    commands.entity(list).despawn_children();
    if palette.results.is_empty() {
        commands.spawn_scene(bsn! { @text("No matching nodes", TEXT_DIM, 12.0) ChildOf(list) });
    }
    for (i, kind) in palette.results.iter().enumerate() {
        let Some(e) = palette.entries.iter().find(|e| e.kind == *kind) else { continue };
        let base = if i == palette.highlight { ROW_ACTIVE } else { Color::NONE };
        let (name, cat, row, hover) = (e.name.clone(), e.category.clone(), PaletteRow(i), HoverRow { base });
        commands.spawn_scene(bsn! {
            Node { flex_direction: FlexDirection::Row, padding: UiRect::axes(px(8), px(3)), column_gap: px(8), border_radius: BorderRadius::all(px(3)) }
            BackgroundColor(base)
            row
            hover
            ChildOf(list)
            Children [ @text(name, TEXT, 13.0) -- Node { flex_grow: 1.0 } -- @text(cat, TEXT_DIM, 11.0) ]
        });
    }
}

fn click_row(
    ev: On<PointerClick>,
    rows: Query<&PaletteRow>,
    mut palette: ResMut<Palette>,
    mut actions: ResMut<Actions>,
) {
    if let Ok(r) = rows.get(ev.event_target()) {
        pick(&mut palette, &mut actions, r.0);
    }
}

fn click_outside(ev: On<PointerPress>, roots: Query<(), With<PaletteRoot>>, mut palette: ResMut<Palette>) {
    if roots.contains(ev.event_target()) && ev.original_event_target() == ev.event_target() {
        palette.open = false;
    }
}
