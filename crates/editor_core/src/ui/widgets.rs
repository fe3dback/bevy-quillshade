//! Shared widgets built from Feathers 0.20 (design §3): panes and subpanes (the containers of the left / right
//! sidebars and the console/logs) and the scrollable column inside them, joined button groups (a mode, a preset, a
//! channel, a step) and captioned checkboxes (a click on the label toggles them). Use these instead of hand-built
//! look-alikes.

use bevy::feathers::containers::{flex_spacer, pane, pane_body, pane_header, subpane, subpane_body, subpane_header};
use bevy::feathers::controls::{ButtonVariant, FeathersButton};
use bevy::feathers::display::caption;
use bevy::feathers::rounded_corners::RoundedCorners;
use bevy::prelude::*;
use bevy::ui::InteractionDisabled;
use bevy::ui_widgets::ScrollArea;

use super::OnActivate;
use super::icons::{IconRef, icon};
pub use super::props::checkbox;
use crate::action::Action;
use crate::theme::TEXT;
use crate::tooltip::Tooltip;

/// A Feathers pane: `title` (a text scene, so the caller can tag it), a spacer and `header_tools` in the
/// `pane_header`, `body` in the `pane_body`. The body grows to fill the pane, so a pane given a height or a
/// `flex_grow` passes it on (a scroll column inside then scrolls).
pub fn pane_box(title: impl Scene, header_tools: Vec<Box<dyn Scene>>, body: impl Scene) -> impl Scene {
    bsn! {
        @pane()
        Children [
            @pane_header()
            Children [ @{title} -- @flex_spacer() -- {header_tools} ]
            --
            @pane_body()
            Node { flex_grow: 1.0, min_height: px(0) }
            Children [ @{body} ]
        ]
    }
}

/// A pane's title text (the Feathers caption, themed by the `pane_header`).
pub fn pane_title(title: impl Into<String>) -> impl Scene {
    caption(title.into())
}

/// A Feathers subpane: `title` in the `subpane_header`, `rows` in the `subpane_body`. One per kind of content in the
/// right sidebar (design §3).
pub fn subpane_box(title: &str, rows: Vec<Box<dyn Scene>>) -> Box<dyn Scene> {
    let title = title.to_string();
    Box::new(bsn! {
        @subpane()
        Children [
            @subpane_header()
            Children [ @caption(title) ]
            --
            @subpane_body()
            Children [ {rows} ]
        ]
    })
}

/// Scrollable column used by the panels; `marker` tags the entity the panel fills. Rows wider than the column (a
/// sidebar narrowed by a small window) are clipped, not drawn over the neighbouring area.
pub fn scroll_column() -> impl Scene {
    bsn! {
        Node {
            flex_direction: FlexDirection::Column,
            flex_grow: 1.0,
            min_height: px(0),
            overflow: Overflow { x: OverflowAxis::Clip, y: OverflowAxis::Scroll },
            padding: UiRect::all(px(4)),
            row_gap: px(1),
        }
        ScrollArea
    }
}

/// What a [`GroupButton`] shows: one of our icons (16 px, per `AGENTS.md` → Icons) or a text.
#[derive(Clone, Debug)]
pub enum Caption {
    Icon(IconRef),
    Text(String),
}

/// One button of a [`button_group`].
#[derive(Clone)]
pub struct GroupButton {
    pub caption: Caption,
    /// Pushed when the button is activated (not while `disabled`).
    pub action: Action,
    /// The on state: the primary variant.
    pub selected: bool,
    /// `InteractionDisabled`: greyed (an icon caption dimmed, less so when `selected`), no `Activate`, so no action.
    pub disabled: bool,
    /// Hover hint naming what the button does (and why it is disabled).
    pub tip: String,
}

/// The corners of button `i` of `n` joined ones: the outer corners of the group are rounded.
fn corners(i: usize, n: usize) -> RoundedCorners {
    match (i, n) {
        (_, 1) => RoundedCorners::All,
        (0, _) => RoundedCorners::Left,
        (i, n) if i + 1 == n => RoundedCorners::Right,
        _ => RoundedCorners::None,
    }
}

/// The alpha of a disabled button's icon: Feathers dims a disabled caption text, our icons carry their own tint.
pub const DISABLED_ICON_ALPHA: f32 = 0.3;
/// The alpha of a disabled but selected button's icon: a disabled group still shows its stored choice (design §5.4).
const DISABLED_SELECTED_ICON_ALPHA: f32 = 0.9;

/// The tint of a group button's icon caption.
fn icon_tint(disabled: bool, selected: bool) -> Color {
    match (disabled, selected) {
        (false, _) => TEXT,
        (true, false) => TEXT.with_alpha(DISABLED_ICON_ALPHA),
        (true, true) => TEXT.with_alpha(DISABLED_SELECTED_ICON_ALPHA),
    }
}

fn group_button(b: GroupButton, corners: RoundedCorners) -> Box<dyn Scene> {
    let tint = icon_tint(b.disabled, b.selected);
    let caption: Box<dyn SceneList> = match b.caption {
        Caption::Icon(id) => Box::new(bsn_list! { @icon(id, 16.0, tint) }),
        Caption::Text(s) => Box::new(bsn_list! { @caption(s) }),
    };
    let (variant, act, tip) = (ButtonVariant::selected(b.selected), OnActivate(b.action), Tooltip(b.tip));
    if b.disabled {
        Box::new(bsn! {
            @FeathersButton { @caption: caption, @variant: variant, @corners: corners }
            Node { flex_shrink: 0.0 }
            InteractionDisabled
            act
            tip
        })
    } else {
        Box::new(bsn! {
            @FeathersButton { @caption: caption, @variant: variant, @corners: corners }
            Node { flex_shrink: 0.0 }
            act
            tip
        })
    }
}

/// Joined `FeathersButton`s (the Feathers gallery's button group): 1 px apart, only the group's outer corners
/// rounded, the selected ones in the primary variant. The caller decides which are selected; each pushes its action.
/// Neither the group nor its buttons shrink: a row too narrow for it wraps or clips the whole group.
pub fn button_group(items: Vec<GroupButton>) -> impl Scene {
    let n = items.len();
    let buttons: Vec<Box<dyn Scene>> =
        items.into_iter().enumerate().map(|(i, b)| group_button(b, corners(i, n))).collect();
    bsn! {
        Node { flex_direction: FlexDirection::Row, align_items: AlignItems::Center, column_gap: px(1), flex_shrink: 0.0 }
        Children [ {buttons} ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_corners_round_only_the_outer_ends() {
        assert!(matches!(corners(0, 1), RoundedCorners::All));
        let three: Vec<_> = (0..3).map(|i| corners(i, 3)).collect();
        assert!(matches!(three[..], [RoundedCorners::Left, RoundedCorners::None, RoundedCorners::Right]));
        assert!(matches!([corners(0, 2), corners(1, 2)], [RoundedCorners::Left, RoundedCorners::Right]));
    }

    /// A disabled group still shows its stored selection (design §5.4): the selected icon stays clearly visible.
    #[test]
    fn disabled_selected_icon_stays_brighter_than_the_unselected_ones() {
        let alpha = |disabled, selected| icon_tint(disabled, selected).alpha();
        assert_eq!(alpha(false, false), 1.0);
        assert_eq!(alpha(false, true), 1.0);
        assert!(alpha(true, false) < 0.5);
        assert!(alpha(true, true) >= 0.6, "the stored mode must read clearly");
    }
}
