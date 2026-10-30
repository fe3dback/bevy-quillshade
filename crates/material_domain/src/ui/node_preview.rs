//! Material's in-node preview kinds (curve plot of a Sample Curve node, value of a `Global*` node's constant,
//! thumbnail of a texture node's Texture asset, strip of a Sample Gradient's gradient): the mechanism is
//! `node_graph_base::preview`'s ([`NodePreview`]); a Material kind takes the node definition ([`NodeDef`]) and the
//! graph document ([`Doc`]).
//!
//! A new preview kind: implement [`NodePreview`] (`type Doc = Doc; type Def = dyn NodeDef;`) and add it to `kinds()`.

use bevy::prelude::*;
use node_graph_base::PreviewRow;
use shader_graph_core::model::NodeId;
use shader_graph_core::registry::NodeDef;

use super::const_preview::ConstPreview;
use super::curve_preview::CurvePreview;
use super::gradient_preview::GradientPreview;
use super::texture_preview::TexturePreview;
use crate::texture::GameImageCache;

pub use node_graph_base::preview::{
    NodePreview, NodePreviewPlugin, PreviewNa, PreviewRoot, PreviewTarget, preview, preview_row,
};

/// Every preview kind: [`NodePreviewPlugin`] for each.
pub struct NodePreviewsPlugin;

impl Plugin for NodePreviewsPlugin {
    fn build(&self, app: &mut App) {
        // Texture thumbnails decode through Material's image cache (`texture_kit`'s `GameImagePlugin` frees its unused
        // entries).
        app.init_resource::<GameImageCache>();
        for k in kinds() {
            (k.plugin)(app);
        }
    }
}

/// One registered preview kind, type-erased.
struct Kind {
    row: fn(&(dyn NodeDef + 'static), &NodeId) -> Option<PreviewRow>,
    plugin: fn(&mut App),
}

impl Kind {
    fn of<P: NodePreview<Def = dyn NodeDef>>() -> Self {
        Kind { row: preview::<P>, plugin: |app| _ = app.add_plugins(NodePreviewPlugin::<P>::default()) }
    }
}

/// Every preview kind, in row order.
fn kinds() -> [Kind; 4] {
    [
        Kind::of::<CurvePreview>(),
        Kind::of::<ConstPreview>(),
        Kind::of::<TexturePreview>(),
        Kind::of::<GradientPreview>(),
    ]
}

/// The preview rows of model node `node` of definition `def` (used by the canvas when it builds the node), in
/// row order.
pub fn previews(def: &(dyn NodeDef + 'static), node: &NodeId) -> Vec<PreviewRow> {
    kinds().iter().filter_map(|k| (k.row)(def, node)).collect()
}
