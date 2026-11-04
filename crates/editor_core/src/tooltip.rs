//! Hover tooltips: a widget with [`Tooltip`] shows its text under itself after a short hover (Feathers 0.20 has no
//! tooltip widget).

use bevy::picking::hover::Hovered;
use bevy::prelude::*;

use crate::theme::{BORDER, TEXT, TEXT_DIM, text};

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
    windows: Query<&Window>,
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
    if tip.0.is_empty() {
        return;
    }
    // Logical px: the transform is the node's center in physical px.
    let half = node.size() * node.inverse_scale_factor() / 2.0;
    let center = transform.translation * node.inverse_scale_factor();
    let line_count = tip.0.lines().count().max(1);
    let estimated_height = 10.0 + line_count as f32 * 16.0;
    let top_y = if windows.iter().next().is_some_and(|w| center.y + half.y + 4.0 + estimated_height > w.height()) {
        (center.y - half.y - estimated_height - 4.0).max(4.0)
    } else {
        center.y + half.y + 4.0
    };
    let (left, top) = (px(center.x - half.x), px(top_y));
    let lines: Vec<Box<dyn Scene>> = tip
        .0
        .lines()
        .enumerate()
        .map(|(i, line)| -> Box<dyn Scene> {
            let (color, size) = if i == 0 { (TEXT, 12.0) } else { (TEXT_DIM, 11.0) };
            Box::new(text(line.to_string(), color, size))
        })
        .collect();
    let popup = commands
        .spawn_scene(bsn! {
            TooltipPopup
            Node {
                position_type: PositionType::Absolute,
                flex_direction: FlexDirection::Column,
                row_gap: px(2),
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
            Children [ {lines} ]
        })
        .id();
    state.popup = Some(popup);
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use bevy::math::Affine2;
    use bevy::window::WindowResolution;

    use super::*;

    #[test]
    fn multiline_tooltip_styles_lines_and_flips_near_window_bottom() {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            bevy::scene::ScenePlugin,
            bevy::input::InputPlugin,
            TooltipPlugin,
        ));
        app.init_asset::<Font>();
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(0.3)));
        app.world_mut().spawn(Window { resolution: WindowResolution::new(800, 600), ..default() });

        let target = app
            .world_mut()
            .spawn((
                Tooltip("assets/textures/rock.png\nSize: 64 B\nModified: 2026-11-04 18:47:14".into()),
                Hovered(true),
                ComputedNode { size: Vec2::new(120.0, 22.0), inverse_scale_factor: 1.0, ..default() },
                UiGlobalTransform::from(Affine2::from_translation(Vec2::new(100.0, 585.0))),
            ))
            .id();

        // First frame registers hover start; two 0.3s frames exceed DELAY (0.5s).
        app.update();
        app.update();
        app.update();

        let w = app.world_mut();
        let (popup, node) = w.query_filtered::<(Entity, &Node), With<TooltipPopup>>().single(w).unwrap();
        assert_eq!(node.flex_direction, FlexDirection::Column);
        assert_eq!(node.row_gap, px(2));
        // Near bottom of 600px window (y = 585), tooltip flips above the row (`top < 574`).
        let Val::Px(top_px) = node.top else { panic!("expected px top, got {:?}", node.top) };
        assert!(top_px < 574.0, "expected flipped top above widget, got {top_px}");

        let children: Vec<Entity> = w.get::<Children>(popup).unwrap().to_vec();
        assert_eq!(children.len(), 3);
        assert_eq!(w.get::<Text>(children[0]).unwrap().0, "assets/textures/rock.png");
        assert_eq!(w.get::<TextColor>(children[0]).unwrap().0, TEXT);
        assert_eq!(w.get::<Text>(children[1]).unwrap().0, "Size: 64 B");
        assert_eq!(w.get::<TextColor>(children[1]).unwrap().0, TEXT_DIM);
        assert_eq!(w.get::<Text>(children[2]).unwrap().0, "Modified: 2026-11-04 18:47:14");
        assert_eq!(w.get::<TextColor>(children[2]).unwrap().0, TEXT_DIM);

        let _ = target;
    }
}
