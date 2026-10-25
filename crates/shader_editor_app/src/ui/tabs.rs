//! Open asset tabs with dirty markers (spec §9 "State").

use bevy::prelude::*;
use bevy::ui_widgets::Button;

use super::icons::{IconId, icon};
use super::{HoverRow, OnActivate, TEXT, TEXT_DIM, text, tool_button};
use crate::state::{Action, Actions, Editor};

#[derive(Component, Default, Clone)]
pub struct TabsBar;

/// Tab index on a tab entity (middle click closes).
#[derive(Component, Default, Clone)]
pub struct TabIndexOf(pub usize);

pub struct TabsPlugin;

impl Plugin for TabsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, redraw).add_observer(middle_click_closes);
    }
}

fn tab(i: usize, label: String, kind: IconId, active: bool) -> impl Scene {
    let base = if active { Color::srgb_u8(32, 34, 39) } else { Color::srgb_u8(48, 50, 57) };
    let (hover, act, idx) = (HoverRow { base }, OnActivate(Action::ActivateTab(i)), TabIndexOf(i));
    let color = if active { TEXT } else { TEXT_DIM };
    bsn! {
        Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            padding: UiRect::new(px(8), px(2), px(4), px(4)),
            column_gap: px(5),
            border_radius: BorderRadius::top(px(5)),
        }
        BackgroundColor(base)
        Button
        hover
        act
        idx
        Children [
            @icon(kind, 16.0, color)
            --
            @text(label, color, 13.0)
            --
            @tool_button("×", Action::CloseTab(i))
        ]
    }
}

fn redraw(
    editor: Res<Editor>,
    bar: Query<Entity, With<TabsBar>>,
    mut last: Local<Option<Vec<(String, IconId, bool, bool)>>>,
    mut commands: Commands,
) {
    let key: Vec<(String, IconId, bool, bool)> = editor
        .docs
        .iter()
        .enumerate()
        .map(|(i, d)| {
            (d.id.source_rel(), IconId::for_asset(Some(d.asset.kind)), d.is_dirty(), editor.active == Some(i))
        })
        .collect();
    if last.as_ref() == Some(&key) {
        return;
    }
    let Ok(bar) = bar.single() else { return };
    *last = Some(key.clone());
    commands.entity(bar).despawn_children();
    for (i, (rel, kind, dirty, active)) in key.into_iter().enumerate() {
        let name = rel.trim_end_matches(".ron").to_string();
        let label = if dirty { format!("{name} *") } else { name };
        commands.spawn_scene(bsn! { @tab(i, label, kind, active) ChildOf(bar) });
    }
}

fn middle_click_closes(ev: On<PointerClick>, tabs: Query<&TabIndexOf>, mut actions: ResMut<Actions>) {
    if ev.button == PointerButton::Middle
        && let Ok(t) = tabs.get(ev.event_target())
    {
        actions.push(Action::CloseTab(t.0));
    }
}
