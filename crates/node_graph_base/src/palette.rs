//! Node palette popup (spec §9.3): right click / Space on the canvas or a wire dropped on empty space. The shell
//! (open, filter, keys, rows) is the base's; its entries and what picking one does come from
//! [`GraphDomain::palette`].

use std::marker::PhantomData;

use bevy::feathers::controls::{FeathersTextInput, FeathersTextInputContainer};
use bevy::input_focus::tab_navigation::TabGroup;
use bevy::input_focus::{AutoFocus, InputFocus};
use bevy::prelude::*;
use bevy::text::EditableText;
use bevy::ui_widgets::ScrollArea;
use editor_core::action::Actions;
use editor_core::search::search_indices_leading;
use editor_core::state::editor::Editor;
use editor_core::theme::{TEXT, TEXT_DIM, text};
use editor_core::ui::HoverRow;
use editor_core::ui::shell::ROW_ACTIVE;

use crate::domain::{GraphCtxRef, GraphDomain, active};
use crate::edit::PinRef;
use crate::view::PaletteEntry;

/// Open palette of `G`: where it is and what it may connect to.
#[derive(Resource)]
pub struct Palette<G> {
    pub open: bool,
    /// Window position (logical px).
    pub screen: Vec2,
    /// Graph position for the new node.
    pub graph: Vec2,
    /// The wire end it was opened from (the new node connects to it).
    pub link: Option<PinRef>,
    entries: Vec<PaletteEntry>,
    /// Indices into `entries`.
    results: Vec<usize>,
    query: Option<String>,
    highlight: usize,
    /// The query the rows were built for and the row highlighted in them.
    rows_for: Option<(Option<String>, usize)>,
    graph_domain: PhantomData<fn() -> G>,
}

impl<G> Default for Palette<G> {
    fn default() -> Self {
        Palette {
            open: false,
            screen: Vec2::ZERO,
            graph: Vec2::ZERO,
            link: None,
            entries: Vec::new(),
            results: Vec::new(),
            query: None,
            highlight: 0,
            rows_for: None,
            graph_domain: PhantomData,
        }
    }
}

impl<G> Palette<G> {
    pub fn show(&mut self, screen: Vec2, graph: Vec2, link: Option<PinRef>) {
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

/// The palette of `G`.
pub struct PalettePlugin<G>(PhantomData<fn() -> G>);

impl<G> Default for PalettePlugin<G> {
    fn default() -> Self {
        PalettePlugin(PhantomData)
    }
}

impl<G: GraphDomain> Plugin for PalettePlugin<G> {
    fn build(&self, app: &mut App) {
        app.init_resource::<Palette<G>>()
            .add_systems(
                Update,
                (spawn_or_close::<G>, filter::<G>, keys::<G>, rows::<G>, follow_highlight::<G>).chain(),
            )
            .add_observer(click_row::<G>)
            .add_observer(click_outside::<G>);
    }
}

fn spawn_or_close<G: GraphDomain>(
    mut palette: ResMut<Palette<G>>,
    editor: Res<Editor>,
    roots: Query<Entity, With<PaletteRoot>>,
    window: Query<&Window>,
    mut commands: Commands,
) {
    let exists = !roots.is_empty();
    let Some(d) = active::<G>(&editor).filter(|_| palette.open) else {
        palette.open = false;
        for r in &roots {
            commands.entity(r).despawn();
        }
        return;
    };
    if exists {
        return;
    }
    palette.entries = G::palette(&GraphCtxRef { editor: &editor }, d, palette.link.as_ref());
    palette.query = None;
    let size = window.single().map(|w| Vec2::new(w.width(), w.height())).unwrap_or(Vec2::new(1600.0, 900.0));
    let x = palette.screen.x.min(size.x - 340.0).max(0.0);
    let y = palette.screen.y.min(size.y - 440.0).max(0.0);
    let hint = match &palette.link {
        Some(_) => "Compatible nodes (auto-connect)",
        None => "Add node",
    };
    // A modal tab group: Tab cycles inside the palette (Bevy's tab navigation; Feathers dialogs and menu popups are
    // modal groups too), so it never focuses a button behind it that Enter / Space would press.
    commands.spawn_scene(bsn! {
        PaletteRoot
        Node { position_type: PositionType::Absolute, left: px(0), top: px(0), width: percent(100), height: percent(100) }
        GlobalZIndex(90)
        TabGroup { modal: true }
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

/// The palette's rows for `query`: indices into `entries`, best first; entries that [`PaletteEntry::lead`] win ties
/// on the score.
pub fn results(entries: &[PaletteEntry], query: &str) -> Vec<usize> {
    search_indices_leading(entries, query, |e| e.lead)
}

fn filter<G: GraphDomain>(mut palette: ResMut<Palette<G>>, inputs: Query<&EditableText, With<PaletteInput>>) {
    if !palette.open {
        return;
    }
    let q = inputs.iter().next().map(|t| t.value().to_string()).unwrap_or_default();
    if palette.query.as_deref() == Some(q.as_str()) {
        return;
    }
    palette.results = results(&palette.entries, &q);
    palette.query = Some(q);
    palette.highlight = 0;
}

fn pick<G>(palette: &mut Palette<G>, actions: &mut Actions, i: usize) {
    if let Some(e) = palette.results.get(i).and_then(|r| palette.entries.get(*r)) {
        actions.push((e.pick)(palette.graph));
    }
    palette.open = false;
}

fn keys<G: GraphDomain>(
    keys: Res<ButtonInput<KeyCode>>,
    mut palette: ResMut<Palette<G>>,
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

/// Rebuilds the rows when the results change (every node, no cap: an empty query lists all categories); moving the
/// highlight only restyles two rows.
fn rows<G: GraphDomain>(
    mut palette: ResMut<Palette<G>>,
    mut list: Query<(Entity, &mut ScrollPosition), With<PaletteList>>,
    mut styled: Query<(&PaletteRow, &mut HoverRow, &mut BackgroundColor)>,
    mut commands: Commands,
) {
    if !palette.open {
        palette.rows_for = None;
        return;
    }
    let Ok((list, mut scroll)) = list.single_mut() else { return };
    let key = (palette.query.clone(), palette.highlight);
    if palette.rows_for.as_ref() == Some(&key) {
        return;
    }
    if let Some((query, _)) = &palette.rows_for
        && *query == key.0
    {
        for (row, mut hover, mut bg) in &mut styled {
            let base = if row.0 == key.1 { ROW_ACTIVE } else { Color::NONE };
            if hover.base != base {
                hover.base = base;
                bg.0 = base;
            }
        }
        palette.rows_for = Some(key);
        return;
    }
    palette.rows_for = Some(key);
    scroll.0 = Vec2::ZERO;
    commands.entity(list).despawn_children();
    if palette.results.is_empty() {
        commands.spawn_scene(bsn! { @text("No matching nodes", TEXT_DIM, 12.0) ChildOf(list) });
    }
    for (i, r) in palette.results.iter().enumerate() {
        let Some(e) = palette.entries.get(*r) else { continue };
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

/// Scrolls the list so the highlighted row is visible (arrows past the visible rows). Reads the previous frame's
/// layout: only the highlight moved since, the rows did not.
fn follow_highlight<G: GraphDomain>(
    palette: Res<Palette<G>>,
    mut list: Query<(&ComputedNode, &UiGlobalTransform, &mut ScrollPosition), With<PaletteList>>,
    rows: Query<(&PaletteRow, &ComputedNode, &UiGlobalTransform)>,
    mut last: Local<Option<(Option<String>, usize)>>,
) {
    if !palette.open {
        *last = None;
        return;
    }
    let key = (palette.query.clone(), palette.highlight);
    if last.as_ref() == Some(&key) {
        return;
    }
    let query_changed = last.as_ref().is_none_or(|(q, _)| *q != key.0);
    *last = Some(key);
    if query_changed {
        return;
    }
    let Ok((list_node, list_at, mut scroll)) = list.single_mut() else { return };
    let Some((_, row_node, row_at)) = rows.iter().find(|(r, ..)| r.0 == palette.highlight) else { return };
    let s = list_node.inverse_scale_factor;
    let top = (row_at.translation.y - row_node.size.y / 2.0 - (list_at.translation.y - list_node.size.y / 2.0)) * s;
    let bottom = top + row_node.size.y * s;
    let view = list_node.size.y * s;
    if top < 0.0 {
        scroll.0.y += top;
    } else if bottom > view {
        scroll.0.y += bottom - view;
    }
}

fn click_row<G: GraphDomain>(
    ev: On<PointerClick>,
    rows: Query<&PaletteRow>,
    mut palette: ResMut<Palette<G>>,
    mut actions: ResMut<Actions>,
) {
    if let Ok(r) = rows.get(ev.event_target()) {
        pick(&mut palette, &mut actions, r.0);
    }
}

fn click_outside<G: GraphDomain>(
    ev: On<PointerPress>,
    roots: Query<(), With<PaletteRoot>>,
    mut palette: ResMut<Palette<G>>,
) {
    if roots.contains(ev.event_target()) && ev.original_event_target() == ev.event_target() {
        palette.open = false;
    }
}
