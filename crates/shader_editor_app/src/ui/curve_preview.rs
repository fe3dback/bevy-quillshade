//! Read-only curve preview inside a Sample Curve node on the canvas (decision 0015): the sampled curve as a small
//! plot, a vertical line at `T` while that input is unconnected, "n/a" when no loadable curve is selected.
//!
//! Redrawn from the model every time its data changes (curve points — of the open curve tab if there is one —,
//! interpolation type, `T`), so it follows a curve edited in another tab and the inline `T` field live.

use bevy::ecs::system::SystemParamItem;
use bevy::prelude::*;
use resvg::tiny_skia;
use shader_graph_core::Value;
use shader_graph_core::curve::{Curve, Interp};
use shader_graph_core::model::NodeId;
use shader_graph_core::registry::{NodeDef, ParamKind};
use shader_project::{AssetId, AssetType};

use super::curve_view::CURVE_COLOR;
use super::node_preview::{NodePreview, PreviewTarget, opaque_image, skia_color};
use crate::doc::{Doc, node_signature};
use crate::state::Editor;

/// Height of the preview in logical px (it spans the node's width).
const HEIGHT: f32 = 32.0;
/// Inset of the unit square from the preview's edges (logical px).
const PAD: f32 = 3.0;
/// Input whose unconnected value is marked on the plot.
const T_PIN: &str = "t";

/// The plot image of a preview row (it owns the image: despawning the row frees it).
#[derive(Component, Default, Clone)]
pub struct CurvePreviewImage;

/// What a curve preview shows.
#[derive(Clone, PartialEq)]
pub struct CurveKey {
    curve: Curve,
    interp: Interp,
    /// Value of the unconnected `T` input.
    t: Option<f32>,
}

/// Preview of nodes sampling a curve asset.
pub struct CurvePreview;

impl NodePreview for CurvePreview {
    type Key = CurveKey;
    type Draw = (
        Query<'static, 'static, &'static Children>,
        Query<'static, 'static, &'static mut ImageNode, With<CurvePreviewImage>>,
        ResMut<'static, Assets<Image>>,
    );
    const SIZED: bool = true;

    fn applies(def: &dyn NodeDef) -> bool {
        def.params().iter().any(|p| p.kind == ParamKind::Curve)
    }

    fn layout(_: &dyn NodeDef) -> Node {
        Node {
            width: percent(100),
            height: px(HEIGHT),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        }
    }

    fn content(_: &dyn NodeDef) -> Box<dyn Scene> {
        Box::new(bsn! {
            CurvePreviewImage
            Node {
                position_type: PositionType::Absolute,
                left: px(0),
                top: px(0),
                width: percent(100),
                height: percent(100),
                border_radius: BorderRadius::all(px(3)),
            }
            // Transparent until the first image is drawn.
            ImageNode { color: Color::NONE }
            Pickable::IGNORE
        })
    }

    fn key(editor: &Editor, doc: &Doc, node: &NodeId) -> Option<CurveKey> {
        let n = doc.asset.nodes.get(node)?;
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
        let t = match doc.asset.edge_into(node, T_PIN) {
            Some(_) => None,
            None => {
                let sig = node_signature(&doc.asset, node, &editor.registry, &editor.functions, &doc.id.path)?;
                let default = sig.input(T_PIN).map_or(0.0, |p| p.default.as_f32());
                Some(n.inputs.get(T_PIN).map_or(default, Value::as_f32))
            }
        };
        Some(CurveKey { curve: curve.clone(), interp, t })
    }

    fn draw(
        key: Option<&CurveKey>,
        at: PreviewTarget,
        (children, image_nodes, images): &mut SystemParamItem<Self::Draw>,
    ) -> bool {
        let Ok(children) = children.get(at.root) else { return false };
        let Some(plot) = children.iter().find(|c| image_nodes.contains(*c)) else { return false };
        let Ok(mut image_node) = image_nodes.get_mut(plot) else { return false };
        let Some(key) = key else {
            image_node.color = Color::NONE;
            return false;
        };
        // One asset per row, replaced in place: no new texture handle per redraw.
        if !image_node.image.is_strong() {
            image_node.image = images.reserve_handle();
        }
        let _ = images.insert(&image_node.image, opaque_image(rasterize(key, at.size, at.scale)));
        image_node.color = Color::WHITE;
        true
    }
}

/// The plot: dark background, the curve, the `T` marker; `scale` = physical px per logical px.
fn rasterize(p: &CurveKey, size: UVec2, scale: f32) -> tiny_skia::Pixmap {
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
        let (x, y) = at(t, p.curve.sample_normalized_with(t, p.interp, &slopes));
        if k == 0 { pb.move_to(x, y) } else { pb.line_to(x, y) }
    }
    if let Some(path) = pb.finish() {
        paint.set_color(skia_color(CURVE_COLOR, 1.0));
        pixmap.stroke_path(&path, &paint, &stroke(1.5), id, None);
    }

    if let Some(t) = p.t {
        let (x, y) = at(t.clamp(0.0, 1.0), p.curve.sample_normalized_with(t, p.interp, &slopes));
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

    fn key(e: &Editor, node: &NodeId) -> Option<CurveKey> {
        CurvePreview::key(e, e.doc()?, node)
    }

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

        let p = key(&e, &node).unwrap();
        assert_eq!((p.interp, p.t), (Interp::CatmullRom, None), "T is wired: no marker");
        assert_eq!(p.curve.points.len(), 5);

        e.apply(Action::Disconnect { to: (node.clone(), T_PIN.into()) });
        assert_eq!(key(&e, &node).unwrap().t, Some(0.0));
        let t = PropTarget::Input { node: node.clone(), pin: T_PIN.into() };
        e.apply(Action::SetValue { target: t, value: Value::Float(0.4), merge: false });
        assert_eq!(key(&e, &node).unwrap().t, Some(0.4));

        // Unsaved edits of the open curve show on the node.
        let graph = e.active.unwrap();
        e.apply(Action::OpenAsset(AssetId::new(AssetType::Curve, "common/pulse")));
        e.apply(Action::Curve(CurveAction::Add((0.9, 0.9))));
        e.apply(Action::ActivateTab(graph));
        assert_eq!(key(&e, &node).unwrap().curve.points.len(), 6);

        let curve = PropTarget::Param { node: node.clone(), key: "curve".into() };
        e.apply(Action::SetValue { target: curve, value: Value::Str(String::new()), merge: false });
        assert!(key(&e, &node).is_none(), "n/a without a curve");
    }
}
