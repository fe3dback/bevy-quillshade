//! The curve tab's edits (decision 0015, design §6.2): the body of [`TabDoc::apply`] / [`TabDoc::current_value`] of
//! [`CurveDoc`], its kind-specific actions ([`CurveAction`], sent as [`CurveEdit`]), property keys ([`CurveProp`],
//! keyed as `KitProp<D, CurveProp>`) and the curve clipboard. Both carry the domain (design §14.4).
//!
//! [`TabDoc::apply`]: editor_core::doc_kind::TabDoc::apply
//! [`TabDoc::current_value`]: editor_core::doc_kind::TabDoc::current_value

use std::collections::BTreeSet;
use std::marker::PhantomData;

use editor_core::action::{Action, Handled, TabAction};
use editor_core::doc_kind::DocCtx;
use editor_core::dynval::{KitProp, PropKey, PropValue};
use editor_core::history::MergeKey;
use editor_core::message::{Level, Message};

use super::CurveDoc;
use super::doc::moved;
use crate::Interp;

/// How far in `t` pasted / duplicated curve points land from the copied ones.
pub const PASTE_STEP: f32 = 0.05;

/// What a property widget of a curve tab edits (decision 0015); widgets key it as `KitProp<D, CurveProp>`
/// ([`curve_key`]), so only `D`'s curve tab takes it (design §14.4).
#[derive(Debug, Clone, PartialEq)]
pub enum CurveProp {
    /// `(t, value)` of a point of the active curve, as a `Float2`: `t` normalized, the value in real units (the
    /// curve's output range).
    Point(usize),
    /// Output range `(min, max)` of the active curve, as a `Float2` (decision 0038).
    Range,
    /// Interpolation the active curve view draws (editor-only).
    Preview,
}

/// Edits of the active curve tab (decision 0015).
#[derive(Debug, Clone, PartialEq)]
pub enum CurveAction {
    Select(BTreeSet<usize>),
    /// Adds a point and selects it.
    Add((f32, f32)),
    /// Replaces all points and the selection; `merge`d edits (a drag in progress) are one undo step until
    /// [`TabAction::CommitInline`].
    SetPoints {
        points: Vec<(f32, f32)>,
        selection: BTreeSet<usize>,
        merge: bool,
    },
    /// Moves the selected points by `(dt, dvalue)` (arrow keys).
    Nudge((f32, f32)),
}

/// A [`CurveAction`] for `D`'s curve tab (design §14.4): only a [`CurveDoc`] of `D` applies it, so an edit queued
/// for another domain's curve tab is dropped when it reaches this one. Every sender in the kit builds this form.
pub struct CurveEdit<D>(pub CurveAction, PhantomData<fn() -> D>);

impl<D> CurveEdit<D> {
    pub fn new(action: CurveAction) -> CurveEdit<D> {
        CurveEdit(action, PhantomData)
    }
}

impl<D> Clone for CurveEdit<D> {
    fn clone(&self) -> Self {
        CurveEdit::new(self.0.clone())
    }
}

impl<D> PartialEq for CurveEdit<D> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<D> std::fmt::Debug for CurveEdit<D> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?} for {}", self.0, std::any::type_name::<D>())
    }
}

/// A curve edit reaches the active tab as [`TabAction::Doc`] (design §6.2).
impl<D: 'static> From<CurveEdit<D>> for TabAction {
    fn from(a: CurveEdit<D>) -> TabAction {
        TabAction::doc(a)
    }
}

impl<D: 'static> From<CurveEdit<D>> for Action {
    fn from(a: CurveEdit<D>) -> Action {
        Action::Tab(TabAction::doc(a))
    }
}

/// The property key of `target` on `D`'s curve tab (design §14.4).
pub fn curve_key<D: 'static>(target: CurveProp) -> PropKey {
    PropKey::new(KitProp::<D, _>::new(target))
}

/// The curve clipboard, in the editor's in-app clipboards (`Editor::clipboards`): the copied points, shared by the
/// curve tabs of every domain (the points are the same model everywhere).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CurveClipboard {
    pub points: Vec<(f32, f32)>,
    /// Pastes since the copy: each lands one [`PASTE_STEP`] further.
    pub pastes: u32,
}

impl<D: 'static> CurveDoc<D> {
    pub(super) fn apply_edit(&mut self, ctx: &mut DocCtx, action: &TabAction) -> Handled {
        let redraw = match action {
            TabAction::Undo => self.undo(),
            TabAction::Redo => self.redo(),
            TabAction::Copy => {
                self.copy_to(ctx);
                false
            }
            TabAction::Cut => {
                self.copy_to(ctx);
                self.delete_selected();
                true
            }
            TabAction::Paste(_) => {
                let Some(clip) = ctx.clipboards.get_mut::<CurveClipboard>() else { return Handled::Yes };
                // Shifted in t: a copy on top of the original would be invisible but change the slopes.
                clip.pastes += 1;
                let dt = PASTE_STEP * clip.pastes as f32;
                let points: Vec<(f32, f32)> = clip.points.iter().map(|(t, v)| (t + dt, *v)).collect();
                self.insert_points(&points);
                true
            }
            TabAction::Duplicate => {
                let copies: Vec<(f32, f32)> =
                    self.selected_points().into_iter().map(|(t, v)| (t + PASTE_STEP, v)).collect();
                if !copies.is_empty() {
                    self.insert_points(&copies);
                }
                true
            }
            TabAction::DeleteSelection => {
                self.delete_selected();
                true
            }
            TabAction::SelectAll => {
                self.select((0..self.curve.points.len()).collect());
                true
            }
            TabAction::CommitInline => {
                self.end_merge();
                true
            }
            TabAction::SetValue { key, value, merge } => {
                let Some(target) = key.downcast::<KitProp<D, CurveProp>>().map(|k| &k.0) else { return Handled::No };
                self.set_prop(ctx, key, target, value, *merge)
            }
            TabAction::SetComponent { key, comp, value, is_final } => {
                let Some(target) = key.downcast::<KitProp<D, CurveProp>>().map(|k| &k.0) else { return Handled::No };
                // `None`: a field sent its shown (rounded) text back on focus loss, which is not an edit.
                let Some(new) = self.prop_of(target).and_then(|cur| cur.with_component(*comp, *value)) else {
                    return Handled::Yes;
                };
                let redraw = self.set_prop(ctx, key, target, &new, true);
                if *is_final {
                    self.end_merge();
                }
                redraw
            }
            TabAction::Doc(p) => {
                let Some(CurveEdit(a, _)) = p.downcast::<CurveEdit<D>>() else { return Handled::No };
                self.curve_action(a)
            }
        };
        ctx.redraw_props |= redraw;
        Handled::Yes
    }

    /// The curve clipboard gets the selected points (nothing when none is selected).
    fn copy_to(&self, ctx: &mut DocCtx) {
        let points = self.selected_points();
        if !points.is_empty() {
            ctx.clipboards.set(CurveClipboard { points, pastes: 0 });
        }
    }

    /// A [`CurveAction`]; returns whether the right sidebar must redraw.
    fn curve_action(&mut self, action: &CurveAction) -> bool {
        match action {
            CurveAction::Select(sel) => self.select(sel.clone()),
            CurveAction::Add(p) => self.add_point(*p),
            CurveAction::SetPoints { points, selection, merge } => {
                self.set_points(points.clone(), selection.clone(), merge.then(|| "drag".into()));
                // Properties catch up on CommitInline.
                return !merge;
            }
            CurveAction::Nudge(delta) => {
                let (points, selection) = moved(&self.curve.points, &self.selection, *delta);
                self.set_points(points, selection, Some("nudge".into()));
            }
        }
        true
    }

    /// Value a property widget shows.
    pub fn prop_of(&self, target: &CurveProp) -> Option<PropValue> {
        match target {
            CurveProp::Point(i) => {
                let (t, v) = *self.curve.points.get(*i)?;
                Some(PropValue::Float2([t, self.curve.to_real(v)]))
            }
            CurveProp::Range => Some(PropValue::Float2([self.curve.min, self.curve.max])),
            CurveProp::Preview => Some(PropValue::Str(self.preview.label().into())),
        }
    }

    /// Sets a property (merged into the last undo step of the same `key` while `merge`); returns whether the right
    /// sidebar must redraw.
    fn set_prop(
        &mut self,
        ctx: &mut DocCtx,
        key: &PropKey,
        target: &CurveProp,
        value: &PropValue,
        merge: bool,
    ) -> bool {
        if self.prop_of(target).as_ref() == Some(value) {
            return false;
        }
        let mkey = merge.then(|| MergeKey::Prop(key.clone()));
        match target {
            CurveProp::Point(i) => {
                let mut points = self.curve.points.clone();
                let Some(p) = points.get_mut(*i) else { return false };
                let f = value.floats();
                // The value field is in real units; an unchanged one keeps the stored value exactly.
                let v = match f.get(1) {
                    Some(x) if *x != self.curve.to_real(p.1) => self.curve.to_normalized(*x),
                    _ => p.1,
                };
                *p = (f[0], v);
                // The point stays selected wherever the re-sort puts it.
                let (points, selection) = moved(&points, &[*i].into(), (0.0, 0.0));
                let moved = selection != BTreeSet::from([*i]);
                self.set_points(points, selection, mkey);
                if moved {
                    // The fields are bound to the old index: rebind them, and the next edit is a new step.
                    self.end_merge();
                    return true;
                }
                !merge
            }
            CurveProp::Range => {
                let f = value.floats();
                let min = f.first().copied().unwrap_or(self.curve.min);
                let max = f.get(1).copied().unwrap_or(self.curve.max);
                if let Err(e) = self.set_range(min, max, mkey) {
                    ctx.messages.push(Message::new(Level::Warning, format!("Range not changed: {e}")));
                    // The field shows the stored value again.
                    return true;
                }
                !merge
            }
            CurveProp::Preview => {
                if let Some(interp) = Interp::from_label(value.as_str()) {
                    self.set_preview(interp);
                }
                true
            }
        }
    }
}
