//! An open curve asset (decision 0015): the points (source of truth), undo/redo history, point selection, and its
//! [`TabDoc`] side (saving, reloading, renames, the editor statusbar).
//!
//! Pure data: the curve view redraws from [`CurveDoc::curve`] when [`CurveDoc::revision`] changes.

use std::collections::BTreeSet;
use std::fmt;
use std::marker::PhantomData;

use editor_core::action::{Handled, TabAction};
use editor_core::doc_kind::{DocCtx, StatusCtx, StatusInfo, TabDoc};
use editor_core::dynval::{KitProp, PropKey, PropValue};
use editor_core::history::{History, MergeKey};
use editor_core::tab::ViewKindId;
use editor_core::ui::icons::IconRef;
use editor_core::util::text::plural;
use project_core::{AssetRef, Project};

use super::{CURVE_ICON, CurveDomain, load, name_of, save};
use crate::{Curve, Interp, range_error};

/// A curve tab of domain `D`.
pub struct CurveDoc<D> {
    /// The asset; follows renames.
    pub aref: AssetRef,
    pub curve: Curve,
    /// The curve last saved or loaded: dirty is `curve != saved`.
    pub saved: Curve,
    history: History<Curve>,
    /// Indices into `curve.points`.
    pub selection: BTreeSet<usize>,
    /// Interpolation the view draws; editor-only, not stored in the file.
    pub preview: Interp,
    /// Bumped on every change the view shows (points, selection, preview).
    pub revision: u64,
    domain: PhantomData<fn() -> D>,
}

impl<D> Clone for CurveDoc<D> {
    fn clone(&self) -> Self {
        CurveDoc {
            aref: self.aref.clone(),
            curve: self.curve.clone(),
            saved: self.saved.clone(),
            history: self.history.clone(),
            selection: self.selection.clone(),
            preview: self.preview,
            revision: self.revision,
            domain: PhantomData,
        }
    }
}

impl<D> fmt::Debug for CurveDoc<D> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CurveDoc")
            .field("aref", &self.aref)
            .field("curve", &self.curve)
            .field("selection", &self.selection)
            .field("preview", &self.preview)
            .field("revision", &self.revision)
            .finish_non_exhaustive()
    }
}

impl<D> CurveDoc<D> {
    pub fn new(aref: AssetRef, curve: Curve) -> Self {
        let saved = curve.clone();
        CurveDoc {
            aref,
            curve,
            saved,
            history: History::default(),
            selection: BTreeSet::new(),
            preview: Interp::default(),
            revision: 1,
            domain: PhantomData,
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.curve != self.saved
    }

    pub fn mark_saved(&mut self) {
        self.saved = self.curve.clone();
    }

    /// Replaces the points (normalized) and the selection as one undoable step; consecutive edits with the same
    /// `key` share one step (a drag).
    pub fn set_points(&mut self, points: Vec<(f32, f32)>, selection: BTreeSet<usize>, key: Option<MergeKey>) {
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

    /// Sets the output range (decision 0038) as one undoable step (merged with the previous one of the same `key`);
    /// an invalid range (`min >= max`, non-finite) changes nothing and is returned as an error.
    pub fn set_range(&mut self, min: f32, max: f32, key: Option<MergeKey>) -> Result<(), String> {
        if let Some(e) = range_error(min, max) {
            return Err(e);
        }
        let before = self.curve.clone();
        (self.curve.min, self.curve.max) = (min, max);
        let changed = self.curve != before;
        self.history.record(changed.then_some(before), key);
        if changed {
            self.revision += 1;
        }
        Ok(())
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

    /// [`moved`]: the selected points of `from` moved by `delta`.
    pub fn moved(
        from: &[(f32, f32)],
        selected: &BTreeSet<usize>,
        delta: (f32, f32),
    ) -> (Vec<(f32, f32)>, BTreeSet<usize>) {
        moved(from, selected, delta)
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

    /// Applies the built-in curve preset at `index` in [`super::CURVE_PRESETS`] as a single undo step, preserving the
    /// curve's output range (`min..max`), clearing the point selection, and switching the preview interpolation to the
    /// preset's [`Interp`]. Returns `true` if the points, selection, or preview interpolation changed.
    pub fn apply_preset(&mut self, index: usize) -> bool {
        let Some(preset) = super::CURVE_PRESETS.get(index) else { return false };
        let round4 = |x: f32| (x.clamp(0.0, 1.0) * 10_000.0).round() / 10_000.0;
        let points: Vec<(f32, f32)> = preset.points.iter().map(|&[t, v]| (round4(t), round4(v))).collect();
        let points_changed = self.curve.points != points;
        let preview_changed = self.preview != preset.interp;
        let sel_changed = !self.selection.is_empty();
        self.set_points(points, BTreeSet::new(), None);
        self.end_merge();
        self.set_preview(preset.interp);
        points_changed || preview_changed || sel_changed
    }

    pub fn end_merge(&mut self) {
        self.history.end_merge();
    }

    pub fn undo(&mut self) -> bool {
        let done = self.history.undo_keeping(&mut self.curve, |m| &mut m.id);
        if done {
            self.after_history();
        }
        done
    }

    pub fn redo(&mut self) -> bool {
        let done = self.history.redo_keeping(&mut self.curve, |m| &mut m.id);
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

/// Moves the selected points of `from` (the points when a drag started) by `delta`, clamped to the unit square.
/// Points may pass each other; the selection follows them through the re-sort.
pub fn moved(from: &[(f32, f32)], selected: &BTreeSet<usize>, delta: (f32, f32)) -> (Vec<(f32, f32)>, BTreeSet<usize>) {
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

/// Clamps and sorts points like [`Curve::normalize`] (stable by `t`), returning the new indices of the tagged ones.
fn sorted_with_selection(mut tagged: Vec<((f32, f32), bool)>) -> (Vec<(f32, f32)>, BTreeSet<usize>) {
    for (p, _) in &mut tagged {
        *p = (p.0.clamp(0.0, 1.0), p.1.clamp(0.0, 1.0));
    }
    tagged.sort_by(|a, b| a.0.0.total_cmp(&b.0.0));
    let selection = tagged.iter().enumerate().filter(|(_, (_, s))| *s).map(|(i, _)| i).collect();
    (tagged.into_iter().map(|(p, _)| p).collect(), selection)
}

impl<D: CurveDomain> TabDoc for CurveDoc<D> {
    fn apply(&mut self, ctx: &mut DocCtx, action: &TabAction) -> Handled {
        self.apply_edit(ctx, action)
    }

    fn current_value(&self, key: &PropKey, _ctx: &StatusCtx) -> Option<PropValue> {
        self.prop_of(&key.downcast::<KitProp<D, super::CurveProp>>()?.0)
    }

    fn asset(&self) -> Option<&AssetRef> {
        Some(&self.aref)
    }

    fn view(&self) -> ViewKindId {
        D::VIEW
    }

    /// The source path without `.ron`.
    fn base_title(&self) -> String {
        self.aref.source_rel().trim_end_matches(".ron").to_string()
    }

    fn icon(&self) -> IconRef {
        CURVE_ICON
    }

    fn watermark(&self) -> Option<&'static str> {
        Some(D::SUFFIX)
    }

    fn is_dirty(&self) -> bool {
        CurveDoc::is_dirty(self)
    }

    fn is_read_only(&self) -> bool {
        false
    }

    fn revision(&self) -> u64 {
        self.revision
    }

    fn save(&mut self, ctx: &mut DocCtx) -> std::io::Result<()> {
        let project = ctx.project.ok_or_else(|| std::io::Error::other("no project is open"))?;
        save(project, &self.aref, &mut self.curve)?;
        self.mark_saved();
        Ok(())
    }

    fn changed_on_disk(&self, project: &Project) -> bool {
        load(project, &self.aref).is_ok_and(|c| c != self.saved)
    }

    fn reload_from_disk(&mut self, ctx: &mut DocCtx) -> Result<(), String> {
        let project = ctx.project.ok_or("no project is open")?;
        self.reload(load(project, &self.aref)?);
        Ok(())
    }

    fn keep_mine(&mut self, project: &Project) {
        if let Ok(on_disk) = load(project, &self.aref) {
            self.saved = on_disk;
        }
    }

    fn rename(&mut self, _project: &Project, from: &AssetRef, to: &AssetRef) {
        if self.aref != *from {
            return;
        }
        self.aref = to.clone();
        self.curve.name = name_of(to).to_string();
        self.mark_saved();
        self.revision += 1;
    }

    fn config_key(&self) -> Option<String> {
        Some(self.aref.source_rel())
    }

    /// The point count; the problems are the domain's diagnostics of the asset plus [`CurveDomain::validate`].
    fn status(&self, ctx: &StatusCtx) -> Option<StatusInfo> {
        let counts = vec![plural(self.curve.points.len(), "point", "points")];
        let mut status = StatusInfo::new(CurveDoc::is_dirty(self), ctx, counts);
        let own = D::validate(&self.curve);
        status.errors += own.iter().filter(|d| d.is_error()).count();
        status.warnings += own.iter().filter(|d| d.is_warning()).count();
        Some(status)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use project_core::DomainId;

    /// A domain for the kit's unit tests.
    pub(crate) struct TestCurve;

    impl CurveDomain for TestCurve {
        const DOMAIN: DomainId = DomainId::new("test");
        const VIEW: ViewKindId = ViewKindId("test.curve");
    }

    fn doc() -> CurveDoc<TestCurve> {
        CurveDoc::new(AssetRef { domain: TestCurve::DOMAIN, path: "c".into(), suffix: "curve" }, Curve::ramp("c"))
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
            let (points, selection) = CurveDoc::<TestCurve>::moved(&from, &sel, (dx, 0.9));
            d.set_points(points, selection, Some("drag".into()));
        }
        d.end_merge();
        // (0.5, 0.5) moved onto (1, 1): clamped to the unit square, still selected (stable sort keeps it first).
        assert_eq!(d.curve.points, vec![(0.0, 0.0), (1.0, 1.0), (1.0, 1.0)]);
        assert_eq!(d.selection, [1].into());
        let (_, selection) = CurveDoc::<TestCurve>::moved(&from, &sel, (0.4, -0.4));
        assert_eq!(selection, [1].into());
        assert!(d.undo());
        assert_eq!(d.curve.points, vec![(0.0, 0.0), (0.5, 0.5), (1.0, 1.0)]);
    }

    #[test]
    fn range_edits_keep_the_points_and_refuse_empty_ranges() {
        let mut d = doc();
        assert!(d.set_range(-1.0, 3.0, None).is_ok());
        assert_eq!((d.curve.min, d.curve.max, d.curve.points.len()), (-1.0, 3.0, 2));
        assert!(d.is_dirty());
        assert!(d.set_range(3.0, 3.0, None).is_err());
        assert!(d.set_range(f32::NAN, 3.0, None).is_err());
        assert_eq!((d.curve.min, d.curve.max), (-1.0, 3.0));
        assert!(d.undo());
        assert_eq!((d.curve.min, d.curve.max), (0.0, 1.0));
        assert!(!d.undo(), "a refused edit is no step");
    }

    #[test]
    fn paste_selects_the_new_points() {
        let mut d = doc();
        d.insert_points(&[(0.25, 0.75), (0.75, 0.25)]);
        assert_eq!(d.curve.points.len(), 4);
        assert_eq!(d.selected_points(), vec![(0.25, 0.75), (0.75, 0.25)]);
    }

    #[test]
    fn apply_preset_scales_with_range_updates_preview_and_undoes_in_one_step() {
        use super::super::CURVE_PRESETS;

        let mut d = doc();
        d.set_range(-2.0, 10.0, None).unwrap();
        d.mark_saved();
        d.select([0].into());

        // Preset 1: Fade Out (`Simple`, `[(0.0, 1.0), (1.0, 0.0)]`).
        let rev_before = d.revision;
        assert!(d.apply_preset(1));
        assert!(d.revision > rev_before);
        assert!(d.is_dirty());
        assert!(d.selection.is_empty());
        assert_eq!(d.preview, Interp::Simple);
        assert_eq!(d.curve.points, vec![(0.0, 1.0), (1.0, 0.0)]);
        assert_eq!((d.curve.min, d.curve.max), (-2.0, 10.0), "output range is preserved");
        assert!((d.curve.sample(0.0, d.preview) - 10.0).abs() < 1e-5);
        assert!((d.curve.sample(0.5, d.preview) - 4.0).abs() < 1e-5);
        assert!((d.curve.sample(1.0, d.preview) - (-2.0)).abs() < 1e-5);

        // Preset 2: Ease In (Quad) (`CatmullRom`, 5 points).
        assert!(d.apply_preset(2));
        assert_eq!(d.preview, Interp::CatmullRom);
        assert_eq!(d.curve.points.len(), 5);
        assert!((d.curve.sample(0.5, d.preview) - 1.0).abs() < 1e-4, "-2 + 0.25 * 12 = 1.0");

        // Re-applying the same preset when nothing is selected is a no-op.
        let rev_same = d.revision;
        assert!(!d.apply_preset(2));
        assert_eq!(d.revision, rev_same);

        // Out-of-bounds preset index returns false without changing anything.
        assert!(!d.apply_preset(CURVE_PRESETS.len() + 5));

        // Undo steps back through the applied presets one step at a time.
        assert!(d.undo());
        assert_eq!(d.curve.points, vec![(0.0, 1.0), (1.0, 0.0)]);
        assert!(d.undo());
        assert_eq!(d.curve.points, vec![(0.0, 0.0), (1.0, 1.0)]);
        assert!(!d.is_dirty());
        assert!(d.redo());
        assert_eq!(d.curve.points, vec![(0.0, 1.0), (1.0, 0.0)]);
    }
}
