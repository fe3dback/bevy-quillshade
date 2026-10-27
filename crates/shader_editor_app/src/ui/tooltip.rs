//! Hover tooltips: a widget with [`Tooltip`] shows its text under itself after a short hover (Feathers 0.20 has no
//! tooltip widget).

use bevy::picking::hover::Hovered;
use bevy::prelude::*;

use super::{BORDER, TEXT, text};

/// Seconds the pointer rests on a widget before its tooltip shows.
const DELAY: f32 = 0.5;

/// Hover text of a widget.
#[derive(Component, Clone, Default)]
#[require(Hovered)]
pub struct Tooltip(pub String);

#[derive(Component, Default, Clone)]
struct TooltipPopup;

pub struct TooltipPlugin;

impl Plugin for TooltipPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, show_tooltips);
    }
}

#[derive(Default)]
struct TooltipState {
    target: Option<Entity>,
    since: f32,
    popup: Option<Entity>,
    /// A press hides the tooltip until the pointer moves to another widget.
    pressed: bool,
}

fn show_tooltips(
    time: Res<Time>,
    mouse: Res<ButtonInput<MouseButton>>,
    q: Query<(Entity, &Tooltip, &Hovered, &ComputedNode, &UiGlobalTransform)>,
    parents: Query<&ChildOf>,
    mut state: Local<TooltipState>,
    mut commands: Commands,
) {
    let now = time.elapsed_secs();
    // `Hovered` holds for a widget and every ancestor of it: the innermost one is under the pointer (a tab's close
    // button inside the tab).
    let hovered =
        q.iter().filter(|(_, _, h, ..)| h.get()).map(|(e, ..)| e).max_by_key(|e| parents.iter_ancestors(*e).count());
    let hide = |state: &mut TooltipState, commands: &mut Commands| {
        if let Some(p) = state.popup.take() {
            commands.entity(p).try_despawn();
        }
    };
    if hovered != state.target {
        hide(&mut state, &mut commands);
        *state = TooltipState { target: hovered, since: now, popup: None, pressed: false };
    }
    if mouse.get_just_pressed().next().is_some() {
        hide(&mut state, &mut commands);
        state.pressed = true;
    }
    let Some(target) = state.target else { return };
    if state.popup.is_some() || state.pressed || now - state.since < DELAY {
        return;
    }
    let Ok((_, tip, _, node, transform)) = q.get(target) else { return };
    // Logical px: the transform is the node's center in physical px.
    let half = node.size() * node.inverse_scale_factor() / 2.0;
    let center = transform.translation * node.inverse_scale_factor();
    let (left, top) = (px(center.x - half.x), px(center.y + half.y + 4.0));
    let label = tip.0.clone();
    let popup = commands
        .spawn_scene(bsn! {
            TooltipPopup
            Node {
                position_type: PositionType::Absolute,
                left: left,
                top: top,
                padding: UiRect::axes(px(8), px(4)),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(4)),
            }
            BackgroundColor(Color::srgb_u8(36, 38, 44))
            BorderColor::all(BORDER)
            GlobalZIndex(100)
            Pickable::IGNORE
            Children [ @text(label, TEXT, 12.0) ]
        })
        .id();
    state.popup = Some(popup);
}
