//! Layered auto-layout of a node subset of a graph asset: `node_graph_base::layout::auto_layout` over its nodes and
//! wires.

use std::collections::{BTreeMap, BTreeSet};

use shader_graph_core::model::{Asset, NodeId};

pub use node_graph_base::layout::{COLUMN, GAP};

/// New positions for `ids` (all nodes when empty). `height` estimates a node's height.
pub fn layout(asset: &Asset, ids: &BTreeSet<NodeId>, height: impl Fn(&str) -> f32) -> BTreeMap<NodeId, (f32, f32)> {
    let nodes: BTreeMap<NodeId, (f32, f32)> = asset.nodes.iter().map(|(id, n)| (id.clone(), n.pos)).collect();
    let edges: Vec<(NodeId, NodeId)> = asset.edges.iter().map(|e| (e.from.0.clone(), e.to.0.clone())).collect();
    node_graph_base::layout::auto_layout(&nodes, &edges, ids, height)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shader_graph_core::model::{AssetKind, Node};

    #[test]
    fn chain_goes_right_to_left() {
        let mut a = Asset::new("t", AssetKind::Function);
        for (id, x) in [("a", 0.0), ("b", 10.0), ("c", 500.0), ("d", 20.0)] {
            a.nodes.insert(id.into(), Node::new("stdlib::Add", (x, 0.0)));
        }
        a.connect(("a", "out"), ("b", "a"));
        a.connect(("b", "out"), ("c", "a"));
        a.connect(("d", "out"), ("c", "b"));
        let p = layout(&a, &BTreeSet::new(), |_| 100.0);
        assert_eq!(p["c"].0, 500.0);
        assert_eq!(p["b"].0, 500.0 - COLUMN);
        assert_eq!(p["d"].0, 500.0 - COLUMN);
        assert_eq!(p["a"].0, 500.0 - 2.0 * COLUMN);
        assert_ne!(p["b"].1, p["d"].1);
    }
}
