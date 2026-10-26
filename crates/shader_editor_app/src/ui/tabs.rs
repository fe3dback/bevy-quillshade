//! Open asset tabs with dirty markers (spec §9 "State"). MMB closes a tab, RMB opens its close menu. Tabs are
//! compact, labelled with the short file name ([`labels`]) and wrap onto more rows when they do not fit.

use bevy::prelude::*;
use bevy::ui_widgets::Button;

use super::dialog::open_icon_menu;
use super::icons::{IconId, icon};
use super::tooltip::Tooltip;
use super::{HoverRow, OnActivate, TEXT, TEXT_DIM, text};
use crate::state::{Action, Actions, CloseTabs, Editor, Unsaved};
use crate::tab::labels;

#[derive(Component, Default, Clone)]
pub struct TabsBar;

/// Tab index on a tab entity (middle click closes, right click opens the menu).
#[derive(Component, Default, Clone)]
pub struct TabIndexOf(pub usize);

pub struct TabsPlugin;

impl Plugin for TabsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, redraw).add_observer(tab_click);
    }
}

/// One compact tab: icon, label (the full path is its tooltip) and a close cross.
fn tab(i: usize, label: String, full: String, kind: IconId, active: bool) -> impl Scene {
    let base = if active { Color::srgb_u8(32, 34, 39) } else { Color::srgb_u8(48, 50, 57) };
    let (hover, act, idx, tip) = (HoverRow { base }, OnActivate(Action::ActivateTab(i)), TabIndexOf(i), Tooltip(full));
    let (close, close_hover) = (OnActivate(Action::CloseTab(i)), HoverRow { base: Color::NONE });
    let close_tip = Tooltip("Close (Ctrl+W, middle click)".into());
    let color = if active { TEXT } else { TEXT_DIM };
    bsn! {
        Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            padding: UiRect::new(px(6), px(2), px(2), px(2)),
            column_gap: px(4),
            border_radius: BorderRadius::top(px(4)),
        }
        BackgroundColor(base)
        Button
        hover
        act
        idx
        tip
        Children [
            @icon(kind, 14.0, color)
            --
            @text(label, color, 12.0)
            --
            Node { padding: UiRect::all(px(2)), border_radius: BorderRadius::all(px(3)) }
            BackgroundColor(Color::NONE)
            Button
            close
            close_hover
            close_tip
            Children [ @icon(IconId::Close, 12.0, color) ]
        ]
    }
}

type TabKey = (String, String, IconId, bool, bool);

fn redraw(
    editor: Res<Editor>,
    bar: Query<Entity, With<TabsBar>>,
    mut last: Local<Option<Vec<TabKey>>>,
    mut commands: Commands,
) {
    let labels = labels(&editor.tabs);
    let key: Vec<TabKey> = editor
        .tabs
        .iter()
        .zip(labels)
        .enumerate()
        .map(|(i, (t, label))| (label, t.title(), t.doc().icon(), t.doc().is_dirty(), editor.active == Some(i)))
        .collect();
    if last.as_ref() == Some(&key) {
        return;
    }
    let Ok(bar) = bar.single() else { return };
    *last = Some(key.clone());
    commands.entity(bar).despawn_children();
    for (i, (label, full, kind, dirty, active)) in key.into_iter().enumerate() {
        let label = if dirty { format!("{label} *") } else { label };
        commands.spawn_scene(bsn! { @tab(i, label, full, kind, active) ChildOf(bar) });
    }
}

fn tab_click(ev: On<PointerClick>, tabs: Query<&TabIndexOf>, mut actions: ResMut<Actions>, mut commands: Commands) {
    let Ok(&TabIndexOf(i)) = tabs.get(ev.event_target()) else { return };
    match ev.button {
        PointerButton::Middle => actions.push(Action::CloseTab(i)),
        PointerButton::Secondary => open_icon_menu(&mut commands, ev.pointer.position, tab_menu(i)),
        PointerButton::Primary => {}
    }
}

/// The close menu of tab `i`.
pub fn tab_menu(i: usize) -> Vec<(Option<IconId>, String, Action)> {
    let close = |which| Action::CloseTabs { which, at: i, unsaved: Unsaved::Ask };
    vec![
        (Some(IconId::Close), "Close\tCtrl+W".into(), Action::CloseTab(i)),
        (None, "Close All".into(), close(CloseTabs::All)),
        (None, "Close Others".into(), close(CloseTabs::Others)),
        (None, "Close to the Left".into(), close(CloseTabs::Left)),
        (None, "Close to the Right".into(), close(CloseTabs::Right)),
        (None, "Close Unmodified".into(), close(CloseTabs::Unmodified)),
    ]
}
