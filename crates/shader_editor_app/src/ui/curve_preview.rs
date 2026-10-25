//! Read-only curve preview inside a Sample Curve node on the canvas (decision 0015): the sampled curve as a small
//! plot, a vertical line at `T` while that input is unconnected, "n/a" when no loadable curve is selected.
//!
//! Redrawn from the model every time its data changes (curve points — of the open curve tab if there is one —,
//! interpolation type, `T`), so it follows a curve edited in another tab and the inline `T` slider live.

use std::collections::HashMap;

use bevy::prelude::*;
use resvg::tiny_skia;
use shader_graph_core::Value;
use shader_graph_core::curve::{Curve, Interp};
use shader_graph_core::model::NodeId;
use shader_graph_core::registry::ParamKind;
use shader_project::{AssetId, AssetType};

use super::TEXT_DIM;
use super::curve_view::{CURVE_COLOR, opaque_image, skia_color};
use crate::doc::node_signature;
use crate::state::Editor;

/// Height of the preview in logical px (it spans the node's width).
const HEIGHT: f32 = 32.0;
/// Inset of the unit square from the preview's edges (logical px).
const PAD: f32 = 3.0;
/// Input whose unconnected value is marked on the plot.
const T_PIN: &str = "t";

/// The preview of model node `.0`.
#[derive(Component, Default, Clone)]
pub struct CurveNodePreview(pub NodeId);

/// What a preview entity last drew, and its image.
struct Shown {
    key: (Option<Preview>, UVec2),
    image: Option<Handle<Image>>,
}

/// "n/a" label of a preview.
#[derive(Component, Default, Clone)]
pub struct CurvePreviewNa;

#[derive(Clone, PartialEq)]
struct Preview {
    curve: Curve,
    interp: Interp,
    /// Value of the unconnected `T` input.
    t: Option<f32>,
}

pub struct CurvePreviewPlugin;

impl Plugin for CurvePreviewPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostUpdate, update_previews.before(bevy::ui::UiSystems::Prepare));
    }
}

/// Whether nodes of this kind get a preview: they sample a curve asset.
pub fn has_preview(editor: &Editor, kind: &str) -> bool {
    editor.registry.get(kind).is_some_and(|d| d.params().iter().any(|p| p.kind == ParamKind::Curve))
}

/// The preview row of model node `node`.
pub fn node_preview(node: &str) -> impl Scene {
    let tag = CurveNodePreview(node.into());
    bsn! {
        tag
        Node {
            width: percent(100),
            height: px(HEIGHT),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            border_radius: BorderRadius::all(px(3)),
        }
        // Transparent until the first image is drawn.
        ImageNode { color: Color::NONE }
        Pickable::IGNORE
        Children [
            CurvePreviewNa
            Text("n/a")
            TextFont { font_size: FontSize::Px(12.0) }
            TextColor(TEXT_DIM)
            Visibility::Hidden
            Pickable::IGNORE
        ]
    }
}

/// What the preview of `node` shows; `None` when it has no loadable curve.
fn preview(editor: &Editor, node: &str) -> Option<Preview> {
    let d = editor.doc()?;
    let n = d.asset.nodes.get(node)?;
    let def = editor.registry.get(&n.kind)?;
    let params = editor.registry.resolved_params(&n.kind, &n.params);
    let curve_param = def.params().into_iter().find(|p| p.kind == ParamKind::Curve)?;
    let path = params.get(&curve_param.id).map(Value::as_str).filter(|p| !p.is_empty())?;
    let id = AssetId::new(AssetType::Curve, path);
    // Unsaved edits of an open curve tab show right away.
    let curve = match editor.curve_index(&id) {
        Some(i) => &editor.tabs[i].curve()?.curve,
        None => editor.disk.curves.get(&id)?.as_ref().ok()?,
    };
    let interp = params.get("type").and_then(|v| Interp::from_label(v.as_str())).unwrap_or_default();
    let t = match d.asset.edge_into(node, T_PIN) {
        Some(_) => None,
        None => {
            let sig = node_signature(&d.asset, node, &editor.registry, &editor.functions, &d.id.path)?;
            let default = sig.input(T_PIN).map_or(0.0, |p| p.default.as_f32());
            Some(n.inputs.get(T_PIN).map_or(default, Value::as_f32))
        }
    };
    Some(Preview { curve: curve.clone(), interp, t })
}

fn update_previews(
    editor: Res<Editor>,
    mut previews: Query<(Entity, &CurveNodePreview, &ComputedNode, &mut ImageNode, &Children)>,
    mut labels: Query<&mut Visibility, With<CurvePreviewNa>>,
    mut images: ResMut<Assets<Image>>,
    mut shown: Local<HashMap<Entity, Shown>>,
) {
    // Canvas rebuilds respawn the nodes: forget (and free the images of) the previews that are gone.
    shown.retain(|e, _| previews.contains(*e));
    for (e, p, node, mut image_node, children) in &mut previews {
        let size = node.size().as_uvec2();
        if size.x == 0 || size.y == 0 {
            continue;
        }
        let key = (preview(&editor, &p.0), size);
        let entry = shown.entry(e).or_insert(Shown { key: (None, UVec2::ZERO), image: None });
        if entry.key == key {
            continue;
        }
        let available = key.0.is_some();
        for c in children {
            if let Ok(mut v) = labels.get_mut(*c) {
                v.set_if_neq(if available { Visibility::Hidden } else { Visibility::Inherited });
            }
        }
        if let Some(pv) = &key.0 {
            let pixmap = rasterize(pv, size, 1.0 / node.inverse_scale_factor());
            let handle = entry.image.get_or_insert_with(|| images.reserve_handle()).clone();
            let _ = images.insert(&handle, opaque_image(pixmap));
            image_node.image = handle;
            image_node.color = Color::WHITE;
        } else {
            image_node.color = Color::NONE;
        }
        entry.key = key;
    }
}

/// The plot: dark background, the curve, the `T` marker; `scale` = physical px per logical px.
fn rasterize(p: &Preview, size: UVec2, scale: f32) -> tiny_skia::Pixmap {
    let mut pixmap = tiny_skia::Pixmap::new(size.x, size.y).expect("non-empty preview size");
    pixmap.fill(skia_color(Color::srgb_u8(24, 25, 29), 1.0));
    let pad = PAD * scale;
    let (w, h) = ((size.x as f32 - 2.0 * pad).max(1.0), (size.y as f32 - 2.0 * pad).max(1.0));
    let at = |t: f32, v: f32| (pad + t * w, pad + (1.0 - v) * h);
    let mut paint = tiny_skia::Paint { anti_alias: true, ..default() };
    let id = tiny_skia::Transform::identity();
    let stroke = |width: f32| tiny_skia::Stroke { width: width * scale, ..default() };

    if let Some(t) = p.t {
        let t = t.clamp(0.0, 1.0);
        let ((x, y0), (_, y1)) = (at(t, 0.0), at(t, 1.0));
        let mut pb = tiny_skia::PathBuilder::new();
        pb.move_to(x, y0 + pad);
        pb.line_to(x, y1 - pad);
        if let Some(path) = pb.finish() {
            paint.set_color(skia_color(Color::WHITE, 0.45));
            pixmap.stroke_path(&path, &paint, &stroke(1.0), id, None);
        }
    }

    let slopes = p.curve.slopes();
    let steps = (w / 2.0).max(16.0) as usize;
    let mut pb = tiny_skia::PathBuilder::new();
    for k in 0..=steps {
        let t = k as f32 / steps as f32;
        let (x, y) = at(t, p.curve.sample_with(t, p.interp, &slopes));
        if k == 0 { pb.move_to(x, y) } else { pb.line_to(x, y) }
    }
    if let Some(path) = pb.finish() {
        paint.set_color(skia_color(CURVE_COLOR, 1.0));
        pixmap.stroke_path(&path, &paint, &stroke(1.5), id, None);
    }

    if let Some(t) = p.t {
        let (x, y) = at(t.clamp(0.0, 1.0), p.curve.sample_with(t, p.interp, &slopes));
        if let Some(dot) = tiny_skia::PathBuilder::from_circle(x, y, 2.5 * scale) {
            paint.set_color(skia_color(Color::WHITE, 1.0));
            pixmap.fill_path(&dot, &paint, tiny_skia::FillRule::Winding, id, None);
        }
    }
    pixmap
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Action, CurveAction, PropTarget};
    use shader_project::{Project, fixtures};

    #[test]
    fn preview_follows_the_node_and_the_curve_tab() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"g\"\n").unwrap();
        std::fs::create_dir_all(dir.path().join("assets")).unwrap();
        fixtures::save_all(&Project::open(dir.path()).unwrap()).unwrap();
        let mut e = Editor::default();
        e.open_project(dir.path()).unwrap();
        e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "fx/curve_bands")));
        let d = e.doc().unwrap();
        let (node, _) = d
            .asset
            .nodes
            .iter()
            .find(|(_, n)| n.params.get("curve") == Some(&Value::Str("common/pulse".into())))
            .unwrap();
        let node = node.clone();

        let p = preview(&e, &node).unwrap();
        assert_eq!((p.interp, p.t), (Interp::CatmullRom, None), "T is wired: no marker");
        assert_eq!(p.curve.points.len(), 5);

        e.apply(Action::Disconnect { to: (node.clone(), T_PIN.into()) });
        assert_eq!(preview(&e, &node).unwrap().t, Some(0.0));
        let t = PropTarget::Input { node: node.clone(), pin: T_PIN.into() };
        e.apply(Action::SetValue { target: t, value: Value::Float(0.4), merge: false });
        assert_eq!(preview(&e, &node).unwrap().t, Some(0.4));

        // Unsaved edits of the open curve show on the node.
        let graph = e.active.unwrap();
        e.apply(Action::OpenAsset(AssetId::new(AssetType::Curve, "common/pulse")));
        e.apply(Action::Curve(CurveAction::Add((0.9, 0.9))));
        e.apply(Action::ActivateTab(graph));
        assert_eq!(preview(&e, &node).unwrap().curve.points.len(), 6);

        let curve = PropTarget::Param { node: node.clone(), key: "curve".into() };
        e.apply(Action::SetValue { target: curve, value: Value::Str(String::new()), merge: false });
        assert!(preview(&e, &node).is_none(), "n/a without a curve");
    }
}
