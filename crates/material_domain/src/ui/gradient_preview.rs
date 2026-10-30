//! Read-only gradient preview inside a Sample Gradient node on the canvas (gradient design §6): the picked gradient as
//! a strip above the pins, "n/a" when no collection or no gradient is picked (or either is missing).
//!
//! The strip shows the slot's baked row (what the game samples; an empty slot is opaque white, as in the game), taken
//! from [`Editor::gradient_collection`]: an open gradients tab's unsaved state first, so an edit there shows on the
//! node live.

use crate::material_state::MaterialStates;
use bevy::ecs::system::SystemParamItem;
use bevy::prelude::*;
use shader_graph_core::Value;
use shader_graph_core::gradient::SAMPLES;
use shader_graph_core::model::NodeId;
use shader_graph_core::registry::{NodeDef, ParamKind};

use super::gradient_strip::strip_pixmap;
use super::node_preview::{NodePreview, PreviewTarget};
use crate::doc::Doc;
use crate::ops::Editor;
use crate::ops::prelude::*;
use crate::util::raster::opaque_image;

/// Height of the strip in logical px (it spans the node's width).
const HEIGHT: f32 = 18.0;

/// The strip image of a preview row (it owns the image: despawning the row frees it).
#[derive(Component, Default, Clone)]
pub struct GradientPreviewImage;

/// Preview of nodes sampling a gradient collection.
pub struct GradientPreview;

impl NodePreview for GradientPreview {
    type Doc = crate::doc::Doc;
    type Def = dyn shader_graph_core::registry::NodeDef;
    /// The picked slot's baked row.
    type Key = [[u8; 4]; SAMPLES];
    type Draw = (
        Query<'static, 'static, &'static Children>,
        Query<'static, 'static, &'static mut ImageNode, With<GradientPreviewImage>>,
        ResMut<'static, Assets<Image>>,
    );
    const SIZED: bool = true;

    fn applies(def: &dyn NodeDef) -> bool {
        def.params().iter().any(|p| p.kind == ParamKind::GradientSlot)
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
            GradientPreviewImage
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

    fn key(editor: &Editor, doc: &Doc, node: &NodeId) -> Option<Self::Key> {
        let n = doc.asset.nodes.get(node)?;
        let def = editor.domains.material().registry.get(&n.kind)?;
        let params = editor.domains.material().registry.resolved_params(&n.kind, &n.params);
        let value = |kind: ParamKind| {
            let p = def.params().into_iter().find(|p| p.kind == kind)?;
            params.get(&p.id).cloned()
        };
        let collection = value(ParamKind::GradientCollection)?;
        let collection = Some(collection.as_str()).filter(|c| !c.is_empty())?;
        let slot = value(ParamKind::GradientSlot).as_ref().map(Value::as_i32).filter(|s| *s > 0)?;
        let slot = editor.gradient_collection(collection)?.slots.iter().find(|s| i64::from(s.id) == i64::from(slot))?;
        Some(slot.bake_row())
    }

    fn draw(
        key: Option<&Self::Key>,
        at: PreviewTarget,
        (children, image_nodes, images): &mut SystemParamItem<Self::Draw>,
    ) -> bool {
        let Ok(children) = children.get(at.root) else { return false };
        let Some(strip) = children.iter().find(|c| image_nodes.contains(*c)) else { return false };
        let Ok(mut image_node) = image_nodes.get_mut(strip) else { return false };
        let Some(row) = key else {
            image_node.color = Color::NONE;
            return false;
        };
        // One asset per row, replaced in place: no new texture handle per redraw.
        if !image_node.image.is_strong() {
            image_node.image = images.reserve_handle();
        }
        let _ = images.insert(&image_node.image, opaque_image(strip_pixmap(row, at.size)));
        image_node.color = Color::WHITE;
        true
    }
}
