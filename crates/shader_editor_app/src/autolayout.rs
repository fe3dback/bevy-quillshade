//! Layered auto-layout of a node subset: columns by distance to the sinks, rows by barycenter.

use std::collections::{BTreeMap, BTreeSet};

use shader_graph_core::model::{Asset, NodeId};

pub const COLUMN: f32 = 280.0;
pub const GAP: f32 = 30.0;

/// New positions for `ids` (all nodes when empty). `height` estimates a node's height.
pub fn layout(asset: &Asset, ids: &BTreeSet<NodeId>, height: impl Fn(&str) -> f32) -> BTreeMap<NodeId, (f32, f32)> {
    let set: BTreeSet<&str> =
        asset.nodes.keys().filter(|k| ids.is_empty() || ids.contains(*k)).map(String::as_str).collect();
    if set.is_empty() {
        return BTreeMap::new();
    }
    let mut down: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for e in &asset.edges {
        let (a, b) = (e.from.0.as_str(), e.to.0.as_str());
        if set.contains(a) && set.contains(b) && a != b {
            down.entry(a).or_default().push(b);
        }
    }
    // Column = longest path to a sink inside the subset (memoized DFS; cycles are cut).
    fn depth<'a>(
        n: &'a str,
        down: &BTreeMap<&'a str, Vec<&'a str>>,
        memo: &mut BTreeMap<&'a str, usize>,
        stack: &mut BTreeSet<&'a str>,
    ) -> usize {
        if let Some(d) = memo.get(n) {
            return *d;
        }
        if !stack.insert(n) {
            return 0;
        }
        let d = down.get(n).map_or(0, |v| v.iter().map(|m| depth(m, down, memo, stack) + 1).max().unwrap_or(0));
        stack.remove(n);
        memo.insert(n, d);
        d
    }
    let mut memo = BTreeMap::new();
    for n in &set {
        depth(n, &down, &mut memo, &mut BTreeSet::new());
    }
    let max_col = memo.values().copied().max().unwrap_or(0);
    // Anchor: keep the subset's right-most / top-most corner where it was.
    let right = set.iter().map(|n| asset.nodes[*n].pos.0).fold(f32::MIN, f32::max);
    let top = set.iter().map(|n| asset.nodes[*n].pos.1).fold(f32::MAX, f32::min);

    let mut out: BTreeMap<NodeId, (f32, f32)> = BTreeMap::new();
    for col in 0..=max_col {
        let mut nodes: Vec<&str> = set.iter().copied().filter(|n| memo[n] == col).collect();
        let key = |n: &str| -> f32 {
            let ys: Vec<f32> = down.get(n).into_iter().flatten().filter_map(|m| out.get(*m)).map(|p| p.1).collect();
            if ys.is_empty() { asset.nodes[n].pos.1 } else { ys.iter().sum::<f32>() / ys.len() as f32 }
        };
        nodes.sort_by(|a, b| key(a).total_cmp(&key(b)).then(a.cmp(b)));
        let x = right - col as f32 * COLUMN;
        let mut y = top;
        for n in nodes {
            out.insert(n.to_string(), (x, y));
            y += height(n) + GAP;
        }
    }
    out
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
