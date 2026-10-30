//! Geometry of UI nodes.

use bevy::prelude::*;
use bevy::ui::ui_transform::UiGlobalTransform;

pub fn logical_size(node: &ComputedNode) -> Vec2 {
    node.size() * node.inverse_scale_factor()
}

/// Canvas-local logical position of a window position, if it is inside the canvas.
pub fn local_pos(node: &ComputedNode, t: &UiGlobalTransform, window_pos: Vec2) -> Option<Vec2> {
    let n = node.normalize_point(*t, window_pos / node.inverse_scale_factor())?;
    (n.x.abs() <= 0.5 && n.y.abs() <= 0.5).then(|| (n + 0.5) * logical_size(node))
}
