//! Open curve asset (decision 0015): the points (source of truth), undo/redo history, point selection.
//!
//! Pure data, no Bevy: the curve view redraws from [`CurveDoc::curve`] when [`CurveDoc::revision`] changes.

use std::collections::BTreeSet;

use shader_graph_core::curve::{Curve, Interp};
use shader_project::AssetId;

use crate::history::History;

#[derive(Debug, Clone)]
pub struct CurveDoc {
    pub id: AssetId,
    pub curve: Curve,
    /// RON text of the last saved state (dirty check).
    pub saved: String,
    history: History<Curve>,
    /// Indices into `curve.points`.
    pub selection: BTreeSet<usize>,
    /// Interpolation the view draws; editor-only, not stored in the file.
    pub preview: Interp,
    /// Bumped on every change the view shows (points, selection, preview).
    pub revision: u64,
}

impl CurveDoc {
    pub fn new(id: AssetId, curve: Curve) -> Self {
        let saved = curve.to_ron();
        CurveDoc {
            id,
            curve,
            saved,
            history: History::default(),
            selection: BTreeSet::new(),
            preview: Interp::default(),
            revision: 1,
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.curve.to_ron() != self.saved
    }

    pub fn mark_saved(&mut self) {
        self.saved = self.curve.to_ron();
    }

    /// Replaces the points (normalized) and the selection as one undoable step; consecutive edits with the same
    /// `key` share one step (a drag).
    pub fn set_points(&mut self, points: Vec<(f32, f32)>, selection: BTreeSet<usize>, key: Option<&str>) {
        let before = self.curve.clone();
        self.curve.points = points;
        self.curve.normalize();
        let changed = self.curve != before;
        self.history.record(changed.then_some(before), key);
        // Not `select`: a drag re-sorting the points must stay one undo step.
        self.set_selection(selection);
        if changed {
            self.revision += 1;
        }
    }

    /// Adds a point and selects it.
    pub fn add_point(&mut self, p: (f32, f32)) {
        let mut points = self.curve.points.clone();
        points.push(p);
        // Sorted like `Curve::normalize`: a new point goes after existing ones with the same t.
        let at = points.iter().filter(|q| q.0 <= p.0.clamp(0.0, 1.0)).count() - 1;
        self.set_points(points, [at].into(), None);
    }

    /// Inserts copied points and selects them.
    pub fn insert_points(&mut self, new: &[(f32, f32)]) {
        let tagged: Vec<((f32, f32), bool)> =
            self.curve.points.iter().map(|p| (*p, false)).chain(new.iter().map(|p| (*p, true))).collect();
        let (points, selection) = sorted_with_selection(tagged);
        self.set_points(points, selection, None);
    }

    pub fn delete_selected(&mut self) {
        let points = self
            .curve
            .points
            .iter()
            .enumerate()
            .filter(|(i, _)| !self.selection.contains(i))
            .map(|(_, p)| *p)
            .collect();
        self.set_points(points, BTreeSet::new(), None);
    }

    /// Moves the selected points of `from` (the points when a drag started) by `delta`, clamped to the unit square.
    /// Points may pass each other; the selection follows them through the re-sort.
    pub fn moved(
        from: &[(f32, f32)],
        selected: &BTreeSet<usize>,
        delta: (f32, f32),
    ) -> (Vec<(f32, f32)>, BTreeSet<usize>) {
        let tagged = from
            .iter()
            .enumerate()
            .map(|(i, p)| {
                if selected.contains(&i) {
                    (((p.0 + delta.0).clamp(0.0, 1.0), (p.1 + delta.1).clamp(0.0, 1.0)), true)
                } else {
                    (*p, false)
                }
            })
            .collect();
        sorted_with_selection(tagged)
    }

    pub fn selected_points(&self) -> Vec<(f32, f32)> {
        self.selection.iter().filter_map(|i| self.curve.points.get(*i).copied()).collect()
    }

    /// Changes the selection; ends the current merge group (nudges of other points are a new undo step).
    pub fn select(&mut self, selection: BTreeSet<usize>) {
        if self.set_selection(selection) {
            self.history.end_merge();
        }
    }

    fn set_selection(&mut self, selection: BTreeSet<usize>) -> bool {
        let selection: BTreeSet<usize> = selection.into_iter().filter(|i| *i < self.curve.points.len()).collect();
        let changed = selection != self.selection;
        if changed {
            self.selection = selection;
            self.revision += 1;
        }
        changed
    }

    pub fn set_preview(&mut self, preview: Interp) {
        if preview != self.preview {
            self.preview = preview;
            self.revision += 1;
        }
    }

    pub fn end_merge(&mut self) {
        self.history.end_merge();
    }

    pub fn undo(&mut self) -> bool {
        let done = self.history.undo(&mut self.curve);
        if done {
            self.after_history();
        }
        done
    }

    pub fn redo(&mut self) -> bool {
        let done = self.history.redo(&mut self.curve);
        if done {
            self.after_history();
        }
        done
    }

    fn after_history(&mut self) {
        self.revision += 1;
        self.selection.retain(|i| *i < self.curve.points.len());
    }

    /// Replaces the curve (external reload) and clears history.
    pub fn reload(&mut self, curve: Curve) {
        self.curve = curve;
        self.mark_saved();
        self.history.clear();
        self.after_history();
    }
}

/// Clamps and sorts points like [`Curve::normalize`] (stable by `t`), returning the new indices of the tagged ones.
fn sorted_with_selection(mut tagged: Vec<((f32, f32), bool)>) -> (Vec<(f32, f32)>, BTreeSet<usize>) {
    for (p, _) in &mut tagged {
        *p = (p.0.clamp(0.0, 1.0), p.1.clamp(0.0, 1.0));
    }
    tagged.sort_by(|a, b| a.0.0.total_cmp(&b.0.0));
    let selection = tagged.iter().enumerate().filter(|(_, (_, s))| *s).map(|(i, _)| i).collect();
    (tagged.into_iter().map(|(p, _)| p).collect(), selection)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shader_project::AssetType;

    fn doc() -> CurveDoc {
        CurveDoc::new(AssetId::new(AssetType::Curve, "c"), Curve::ramp("c"))
    }

    #[test]
    fn edits_undo_and_dirty() {
        let mut d = doc();
        assert!(!d.is_dirty());
        d.add_point((0.5, 0.8));
        assert_eq!(d.curve.points, vec![(0.0, 0.0), (0.5, 0.8), (1.0, 1.0)]);
        assert_eq!(d.selection, [1].into());
        assert!(d.is_dirty());
        d.delete_selected();
        assert_eq!(d.curve.points.len(), 2);
        assert!(!d.is_dirty(), "back to the saved points");
        assert!(d.undo());
        assert_eq!(d.curve.points.len(), 3);
        assert!(d.redo());
        assert_eq!(d.curve.points.len(), 2);
    }

    #[test]
    fn drag_passes_neighbours_and_is_one_step() {
        let mut d = doc();
        d.add_point((0.5, 0.5));
        let (from, sel) = (d.curve.points.clone(), d.selection.clone());
        for dx in [0.2, 0.4, 0.7] {
            let (points, selection) = CurveDoc::moved(&from, &sel, (dx, 0.9));
            d.set_points(points, selection, Some("drag"));
        }
        d.end_merge();
        // (0.5, 0.5) moved onto (1, 1): clamped to the unit square, still selected (stable sort keeps it first).
        assert_eq!(d.curve.points, vec![(0.0, 0.0), (1.0, 1.0), (1.0, 1.0)]);
        assert_eq!(d.selection, [1].into());
        let (_, selection) = CurveDoc::moved(&from, &sel, (0.4, -0.4));
        assert_eq!(selection, [1].into());
        assert!(d.undo());
        assert_eq!(d.curve.points, vec![(0.0, 0.0), (0.5, 0.5), (1.0, 1.0)]);
    }

    #[test]
    fn paste_selects_the_new_points() {
        let mut d = doc();
        d.insert_points(&[(0.25, 0.75), (0.75, 0.25)]);
        assert_eq!(d.curve.points.len(), 4);
        assert_eq!(d.selected_points(), vec![(0.25, 0.75), (0.75, 0.25)]);
    }
}
