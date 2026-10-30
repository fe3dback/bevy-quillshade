//! Graph geometry shared by every graph domain: the grid (design §4.1), bounds, comment frame enclosure (design §4.5)
//! and the layered auto-layout.

use std::collections::{BTreeMap, BTreeSet};

/// Size of a comment frame added around no node, in graph units (design §6.1).
pub const EMPTY_FRAME_SIZE: (f32, f32) = (400.0, 300.0);

/// Minimum width and height of a comment frame when resized, in graph units.
pub const MIN_FRAME_SIZE: (f32, f32) = (120.0, 80.0);

/// Node positions snap to multiples of this many graph units when touched (design §4.1); positions loaded from a
/// file are kept as they are.
pub const GRID_SNAP: f32 = 10.0;

/// `p` rounded to the nearest grid point; ties round away from zero (`f32::round`), `-0.0` becomes `0.0`.
pub fn snap(p: (f32, f32)) -> (f32, f32) {
    (grid_point(p.0), grid_point(p.1))
}

/// The grid coordinate nearest to `v` (ties away from zero, never `-0.0`).
pub fn grid_point(v: f32) -> f32 {
    (v / GRID_SNAP).round() * GRID_SNAP + 0.0
}

/// `p` with float noise removed: a coordinate within `1e-3` of a grid point becomes that point. Moving a node by the
/// delta that put another one on the grid leaves such noise (`3.3 + 31.7 = 34.999996`); real off-grid positions are
/// kept.
pub fn tidy(p: (f32, f32)) -> (f32, f32) {
    let t = |v: f32| {
        let g = grid_point(v);
        if (v - g).abs() < 1e-3 { g } else { v }
    };
    (t(p.0), t(p.1))
}

/// Bounding box `(min x, min y, max x, max y)` of positions (top-left corners); `None` for none.
pub fn bounds(points: impl IntoIterator<Item = (f32, f32)>) -> Option<(f32, f32, f32, f32)> {
    let mut it = points.into_iter();
    let first = it.next()?;
    Some(it.fold((first.0, first.1, first.0, first.1), |b, p| (b.0.min(p.0), b.1.min(p.1), b.2.max(p.0), b.3.max(p.1))))
}

/// The frames of `frames` (`(id, nodes it was made around)`) among `marked` that may be deleted together with the
/// node selection `nodes`: there is a node selection and every node of the frame is in it (design §4.5).
pub fn enclosed<'a>(
    frames: impl IntoIterator<Item = (&'a str, &'a [String])>,
    marked: &BTreeSet<String>,
    nodes: &BTreeSet<String>,
) -> BTreeSet<String> {
    if nodes.is_empty() {
        return BTreeSet::new();
    }
    let frames = frames.into_iter().filter(|(id, _)| marked.contains(*id));
    frames.filter(|(_, inside)| inside.iter().all(|n| nodes.contains(n))).map(|(id, _)| id.to_string()).collect()
}

/// Horizontal distance between the auto-layout's columns.
pub const COLUMN: f32 = 280.0;
/// Vertical gap between nodes of a column.
pub const GAP: f32 = 30.0;

/// Layered auto-layout of a node subset: columns by distance to the sinks, rows by barycenter. New positions for
/// `ids` (all `nodes` when empty); `edges` are `(from node, to node)` wires in model order; `height` estimates a
/// node's height.
pub fn auto_layout(
    nodes: &BTreeMap<String, (f32, f32)>,
    edges: &[(String, String)],
    ids: &BTreeSet<String>,
    height: impl Fn(&str) -> f32,
) -> BTreeMap<String, (f32, f32)> {
    let set: BTreeSet<&str> = nodes.keys().filter(|k| ids.is_empty() || ids.contains(*k)).map(String::as_str).collect();
    if set.is_empty() {
        return BTreeMap::new();
    }
    let mut down: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (a, b) in edges {
        let (a, b) = (a.as_str(), b.as_str());
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
    let pos = |n: &str| nodes.get(n).copied().unwrap_or_default();
    // Anchor: keep the subset's right-most / top-most corner where it was.
    let right = set.iter().map(|n| pos(n).0).fold(f32::MIN, f32::max);
    let top = set.iter().map(|n| pos(n).1).fold(f32::MAX, f32::min);

    let mut out: BTreeMap<String, (f32, f32)> = BTreeMap::new();
    for col in 0..=max_col {
        let mut column: Vec<&str> = set.iter().copied().filter(|n| memo[n] == col).collect();
        let key = |n: &str| -> f32 {
            let ys: Vec<f32> = down.get(n).into_iter().flatten().filter_map(|m| out.get(*m)).map(|p| p.1).collect();
            if ys.is_empty() { pos(n).1 } else { ys.iter().sum::<f32>() / ys.len() as f32 }
        };
        column.sort_by(|a, b| key(a).total_cmp(&key(b)).then(a.cmp(b)));
        let x = right - col as f32 * COLUMN;
        let mut y = top;
        for n in column {
            out.insert(n.to_string(), (x, y));
            y += height(n) + GAP;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chain_goes_right_to_left() {
        let nodes: BTreeMap<String, (f32, f32)> =
            [("a", 0.0), ("b", 10.0), ("c", 500.0), ("d", 20.0)].map(|(id, x)| (id.to_string(), (x, 0.0))).into();
        let edges = [("a", "b"), ("b", "c"), ("d", "c")].map(|(a, b)| (a.to_string(), b.to_string()));
        let p = auto_layout(&nodes, &edges, &BTreeSet::new(), |_| 100.0);
        assert_eq!(p["c"].0, 500.0);
        assert_eq!(p["b"].0, 500.0 - COLUMN);
        assert_eq!(p["d"].0, 500.0 - COLUMN);
        assert_eq!(p["a"].0, 500.0 - 2.0 * COLUMN);
        assert_ne!(p["b"].1, p["d"].1);
    }

    #[test]
    fn grid_snap_and_tidy() {
        assert_eq!(snap((14.9, -15.0)), (10.0, -20.0));
        assert_eq!(snap((-0.4, 0.0)).0.to_bits(), 0.0f32.to_bits(), "never -0.0");
        assert_eq!(tidy((39.999996, 3.3)), (40.0, 3.3));
    }

    #[test]
    fn enclosure_needs_every_node_selected() {
        let a = vec!["n1".to_string(), "n2".to_string()];
        let frames = [("f", a.as_slice())];
        let marked: BTreeSet<String> = ["f".to_string()].into();
        let some: BTreeSet<String> = ["n1".to_string()].into();
        let all: BTreeSet<String> = ["n1".to_string(), "n2".to_string()].into();
        assert!(enclosed(frames, &marked, &some).is_empty());
        assert_eq!(enclosed(frames, &marked, &all), marked);
        assert!(enclosed(frames, &marked, &BTreeSet::new()).is_empty(), "no node selection");
        assert_eq!(bounds([(1.0, 5.0), (-2.0, 3.0)]), Some((-2.0, 3.0, 1.0, 5.0)));
        assert_eq!(bounds([]), None);
    }
}
