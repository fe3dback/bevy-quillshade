//! The editor's window (design §6, §8): the shell ([`shell`]), the per-kind areas ([`areas`]), the secondbar
//! ([`tabs`]), the sidebar trees ([`browser`], [`tree`]), the console/logs ([`log`]), dialogs and menus ([`dialog`],
//! [`menu`]), the welcome screen ([`welcome`]), and the building blocks: icons, property widgets, Feathers widgets.

pub mod areas;
pub mod browser;
pub mod dialog;
pub mod icons;
pub mod log;
pub mod menu;
pub mod props;
pub mod shell;
pub mod tabs;
pub mod text_view;
pub mod tree;
pub mod welcome;
pub mod widgets;

use crate::action::Action;
use crate::theme::ROW_HOVER;
use crate::theme::TEXT;
use crate::tooltip::Tooltip;
use crate::ui::icons::{IconRef, icon};
use bevy::feathers::controls::{ButtonVariant, FeathersButton, FeathersToolButton};
use bevy::feathers::display::caption;
use bevy::prelude::*;

/// Pushes its action when the widget is activated (buttons, menu items).
#[derive(Component, Default, Clone)]
pub struct OnActivate(pub Action);

/// Rows that highlight under the pointer ([`row_hover_on`] / [`row_hover_off`]).
#[derive(Component, Default, Clone)]
pub struct HoverRow {
    pub base: Color,
}

pub fn row_hover_on(ev: On<PointerOver>, mut q: Query<(&HoverRow, &mut BackgroundColor)>) {
    if let Ok((r, mut bg)) = q.get_mut(ev.event_target())
        && r.base.alpha() < 0.2
    {
        bg.0 = ROW_HOVER;
    }
}

pub fn row_hover_off(ev: On<PointerOut>, mut q: Query<(&HoverRow, &mut BackgroundColor)>) {
    if let Ok((r, mut bg)) = q.get_mut(ev.event_target()) {
        bg.0 = r.base;
    }
}

/// An open context menu: its barrier, which holds the anchor at the cursor and the popup. While one exists the
/// keyboard is held back from the hotkeys ([`crate::main_input::Typing`]).
#[derive(Component, Default, Clone)]
pub struct MenuRoot;

/// Small text button that pushes `action`. Buttons never shrink (`flex_shrink: 0`): a caption is not broken inside
/// its button, a row that runs out of room wraps or clips whole buttons instead.
pub fn tool_button(label: impl Into<String>, action: impl Into<Action>) -> impl Scene {
    let label = label.into();
    let act = OnActivate(action.into());
    bsn! {
        @FeathersToolButton { @variant: ButtonVariant::Plain, @caption: bsn! { @caption(label) } }
        Node { flex_shrink: 0.0 }
        act
    }
}

/// [`tool_button`] with an icon before the label and a hover tooltip.
pub fn icon_tool_button(
    id: impl Into<IconRef>,
    label: impl Into<String>,
    action: impl Into<Action>,
    tip: impl Into<String>,
) -> impl Scene {
    let (label, tip) = (label.into(), Tooltip(tip.into()));
    let act = OnActivate(action.into());
    bsn! {
        @FeathersToolButton {
            @variant: ButtonVariant::Plain,
            @caption: bsn_list! { @icon(id, 16.0, TEXT) Node { margin: UiRect::right(px(4)) } -- @caption(label) },
        }
        Node { flex_shrink: 0.0 }
        act
        tip
    }
}

/// Icon-only tool button; `tip` names the action (and its hotkey).
pub fn icon_button(id: impl Into<IconRef>, action: impl Into<Action>, tip: impl Into<String>) -> impl Scene {
    let (act, tip) = (OnActivate(action.into()), Tooltip(tip.into()));
    bsn! {
        @FeathersToolButton { @variant: ButtonVariant::Plain, @caption: bsn! { @icon(id, 16.0, TEXT) } }
        Node { flex_shrink: 0.0 }
        act
        tip
    }
}

pub fn button(label: impl Into<String>, action: impl Into<Action>, primary: bool) -> impl Scene {
    let label = label.into();
    let act = OnActivate(action.into());
    let variant = if primary { ButtonVariant::Primary } else { ButtonVariant::Normal };
    bsn! {
        @FeathersButton { @variant: variant, @caption: bsn! { @caption(label) } }
        Node { flex_shrink: 0.0 }
        act
    }
}
