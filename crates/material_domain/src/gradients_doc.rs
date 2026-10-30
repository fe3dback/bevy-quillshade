//! Open gradient collection (gradient design §8): the collection (source of truth), undo/redo history, the selected
//! slot and the selected stop or midpoint.
//!
//! Pure data, no Bevy: the gradients view redraws from [`GradientsDoc::collection`] when [`GradientsDoc::revision`]
//! changes.

use shader_graph_core::gradient::{GradientCollection, MAX_STOPS, Slot, Stop};
use shader_project::AssetId;

use crate::history::{History, MergeKey};

/// Midpoint of a new stop's segment (the colors mix 50/50 halfway).
const NEW_MID: f32 = 0.5;

/// Color of the first stop of an empty slot.
const NEW_COLOR: [f32; 4] = [1.0; 4];

/// What is selected in the selected slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GradientSel {
    /// A color stop.
    Stop(usize),
    /// The midpoint of the segment from stop `i` to stop `i + 1` (stored on stop `i`).
    Mid(usize),
}

impl GradientSel {
    /// The stop the selection lives on.
    fn stop(self) -> usize {
        match self {
            GradientSel::Stop(i) | GradientSel::Mid(i) => i,
        }
    }

    /// The same kind of selection on stop `i`.
    fn on(self, i: usize) -> GradientSel {
        match self {
            GradientSel::Stop(_) => GradientSel::Stop(i),
            GradientSel::Mid(_) => GradientSel::Mid(i),
        }
    }

    /// Whether it names an existing stop (or segment) of a slot with `stops` stops.
    fn fits(self, stops: usize) -> bool {
        match self {
            GradientSel::Stop(i) => i < stops,
            GradientSel::Mid(i) => i + 1 < stops,
        }
    }
}

#[derive(Debug, Clone)]
pub struct GradientsDoc {
    pub id: AssetId,
    /// [`GradientsDoc::id`] as the core names it ([`crate::doc_kind::TabDoc::asset`]); follows renames.
    pub(crate) aref: project_core::AssetRef,
    pub collection: GradientCollection,
    /// RON text of the last saved state (dirty check).
    pub saved: String,
    history: History<GradientCollection>,
    /// Position of the selected slot (editor-only).
    pub slot: usize,
    /// Selected stop or midpoint of the selected slot (editor-only).
    pub selection: Option<GradientSel>,
    /// Bumped on every change the view shows (collection, slot, selection).
    pub revision: u64,
}

impl GradientsDoc {
    pub fn new(id: AssetId, collection: GradientCollection) -> Self {
        let saved = collection.to_ron();
        GradientsDoc {
            aref: id.to_ref(),
            id,
            collection,
            saved,
            history: History::default(),
            slot: 0,
            selection: None,
            revision: 1,
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.collection.to_ron() != self.saved
    }

    pub fn mark_saved(&mut self) {
        self.saved = self.collection.to_ron();
    }

    /// The selected slot (`None` only for a collection without slots, which never loads).
    pub fn current(&self) -> Option<&Slot> {
        self.collection.slots.get(self.slot)
    }

    fn stop_count(&self) -> usize {
        self.current().map_or(0, |s| s.stops.len())
    }

    /// Selects the slot at position `slot` and clears the stop selection; ends the current merge group.
    pub fn select_slot(&mut self, slot: usize) {
        if slot >= self.collection.slots.len() || slot == self.slot {
            return;
        }
        self.slot = slot;
        self.selection = None;
        self.history.end_merge();
        self.revision += 1;
    }

    /// Selects a stop or a midpoint of the selected slot (one that does not exist selects nothing); ends the current
    /// merge group.
    pub fn select(&mut self, selection: Option<GradientSel>) {
        if self.set_selection(selection) {
            self.history.end_merge();
        }
    }

    fn set_selection(&mut self, selection: Option<GradientSel>) -> bool {
        let selection = selection.filter(|s| s.fits(self.stop_count()));
        let changed = selection != self.selection;
        if changed {
            self.selection = selection;
            self.revision += 1;
        }
        changed
    }

    /// One edit of the collection: normalized, recorded as an undo step (merged with the previous one of the same
    /// `key`); returns whether anything changed.
    fn edit(&mut self, key: Option<MergeKey>, f: impl FnOnce(&mut GradientCollection)) -> bool {
        let before = self.collection.clone();
        f(&mut self.collection);
        self.collection.normalize();
        let changed = self.collection != before;
        self.history.record(changed.then_some(before), key);
        if changed {
            self.revision += 1;
        }
        changed
    }

    /// Edits the selected slot.
    fn edit_slot(&mut self, key: Option<MergeKey>, f: impl FnOnce(&mut Slot)) -> bool {
        let slot = self.slot;
        self.edit(key, |c| {
            if let Some(s) = c.slots.get_mut(slot) {
                f(s);
            }
        })
    }

    /// Replaces the stops of the selected slot (clamped and sorted by `t`) and the selection as one undoable step;
    /// consecutive edits with the same `key` share one step (a drag). The selection follows its stop through the
    /// re-sort.
    pub fn set_stops(&mut self, stops: Vec<Stop>, selection: Option<GradientSel>, key: Option<MergeKey>) {
        let (stops, selection) = sorted(stops, selection);
        self.edit_slot(key, |s| s.stops = stops);
        // Not `select`: a drag re-sorting the stops must stay one undo step.
        self.set_selection(selection);
    }

    /// Adds a stop at `t` with the color the gradient has there (opaque white in an empty slot) and selects it;
    /// `false` (nothing changes) when the slot already has [`MAX_STOPS`].
    pub fn add_stop(&mut self, t: f32) -> bool {
        let Some(slot) = self.current() else { return false };
        if slot.stops.len() >= MAX_STOPS {
            return false;
        }
        let color = slot.sample(t).unwrap_or(NEW_COLOR);
        let mut stops = slot.stops.clone();
        // Pushed last: the stable sort puts it after existing stops with the same `t`.
        stops.push(Stop { t, color, mid: NEW_MID });
        let at = stops.len() - 1;
        self.set_stops(stops, Some(GradientSel::Stop(at)), None);
        true
    }

    /// Deletes the selected stop. A selected midpoint is not deleted (a segment always has one).
    pub fn delete_selected(&mut self) {
        let Some(GradientSel::Stop(i)) = self.selection else { return };
        let mut stops = self.current().map(|s| s.stops.clone()).unwrap_or_default();
        if i < stops.len() {
            stops.remove(i);
            self.set_stops(stops, None, None);
        }
    }

    /// Sets the midpoint of segment `seg` (`seg → seg + 1`), clamped to `MID_MIN..=MID_MAX`.
    pub fn set_mid(&mut self, seg: usize, mid: f32, key: Option<MergeKey>) {
        if seg + 1 >= self.stop_count() {
            return;
        }
        self.edit_slot(key, |s| s.stops[seg].mid = mid);
    }

    /// Moves stop `i` to `t` (clamped to `0..1`) and selects it, wherever the re-sort puts it.
    pub fn set_stop_t(&mut self, i: usize, t: f32, key: Option<MergeKey>) {
        let mut stops = self.current().map(|s| s.stops.clone()).unwrap_or_default();
        let Some(stop) = stops.get_mut(i) else { return };
        if !t.is_finite() {
            return;
        }
        stop.t = t;
        self.set_stops(stops, Some(GradientSel::Stop(i)), key);
    }

    /// Sets the linear RGBA color of stop `i` (clamped to `0..1`).
    pub fn set_stop_color(&mut self, i: usize, color: [f32; 4], key: Option<MergeKey>) {
        if i >= self.stop_count() {
            return;
        }
        self.edit_slot(key, |s| s.stops[i].color = color);
    }

    /// Sets the selected slot's editor-only name; consecutive edits with the same `key` (typing) share one step.
    pub fn set_name(&mut self, name: String, key: Option<MergeKey>) {
        self.edit_slot(key, |s| s.name = name);
    }

    /// Moves the slot at `from` to position `to` (the slots between shift by one); the moved slot keeps its id and
    /// stays selected if it was.
    pub fn move_slot(&mut self, from: usize, to: usize) {
        let n = self.collection.slots.len();
        if from >= n || to >= n || from == to {
            return;
        }
        self.edit(None, |c| {
            let slot = c.slots.remove(from);
            c.slots.insert(to, slot);
        });
        if self.slot == from {
            self.slot = to;
        } else if from < self.slot && self.slot <= to {
            self.slot -= 1;
        } else if to <= self.slot && self.slot < from {
            self.slot += 1;
        }
    }

    /// Empties the selected slot (stops and name); its id stays.
    pub fn clear_slot(&mut self) {
        self.edit_slot(None, |s| {
            s.stops.clear();
            s.name.clear();
        });
        self.set_selection(None);
    }

    /// Copies the selected slot's stops and name into the next empty slot after it (wrapping around) and selects
    /// the copy; `false` when no slot is empty.
    pub fn duplicate_slot(&mut self) -> bool {
        let n = self.collection.slots.len();
        let Some(source) = self.current().cloned() else { return false };
        let Some(target) = (1..n).map(|k| (self.slot + k) % n).find(|i| self.collection.slots[*i].is_empty()) else {
            return false;
        };
        self.edit(None, |c| {
            c.slots[target].stops = source.stops;
            c.slots[target].name = source.name;
        });
        self.select_slot(target);
        true
    }

    /// The selected slot, for the clipboard.
    pub fn copied_slot(&self) -> Slot {
        self.current().cloned().unwrap_or(Slot { id: 0, name: String::new(), stops: Vec::new() })
    }

    /// Replaces the selected slot's stops and name with `slot`'s; the target keeps its own id.
    pub fn paste_slot(&mut self, slot: &Slot) {
        let (stops, name) = (slot.stops.clone(), slot.name.clone());
        self.edit_slot(None, |s| {
            s.stops = stops;
            s.name = name;
        });
        self.set_selection(None);
    }

    pub fn end_merge(&mut self) {
        self.history.end_merge();
    }

    pub fn undo(&mut self) -> bool {
        let before = self.collection.clone();
        let done = self.history.undo(&mut self.collection);
        if done {
            self.after_history(&before);
        }
        done
    }

    pub fn redo(&mut self) -> bool {
        let before = self.collection.clone();
        let done = self.history.redo(&mut self.collection);
        if done {
            self.after_history(&before);
        }
        done
    }

    /// After undo / redo: when the selected slot did not change but another did, that one is selected, so the step
    /// shows; a selection that no longer fits is dropped.
    fn after_history(&mut self, before: &GradientCollection) {
        self.revision += 1;
        let changed = |i: usize| before.slots.get(i) != self.collection.slots.get(i);
        if !changed(self.slot)
            && let Some(i) = (0..self.collection.slots.len()).find(|i| changed(*i))
        {
            self.slot = i;
            self.selection = None;
        }
        let stops = self.stop_count();
        self.selection = self.selection.filter(|s| s.fits(stops));
    }

    /// Replaces the collection (external reload) and clears history; the slot position is kept.
    pub fn reload(&mut self, collection: GradientCollection) {
        self.collection = collection;
        self.mark_saved();
        self.history.clear();
        self.slot = self.slot.min(self.collection.slots.len().saturating_sub(1));
        self.revision += 1;
        let stops = self.stop_count();
        self.selection = self.selection.filter(|s| s.fits(stops));
    }
}

/// Drops stops with a non-finite `t`, clamps `t` and stable-sorts by it like [`GradientCollection::normalize`],
/// returning the selection moved onto the new index of its stop (dropped when it no longer fits).
fn sorted(stops: Vec<Stop>, selection: Option<GradientSel>) -> (Vec<Stop>, Option<GradientSel>) {
    let tagged = selection.map(GradientSel::stop);
    let mut stops: Vec<(Stop, bool)> = stops
        .into_iter()
        .enumerate()
        .filter(|(_, s)| s.t.is_finite())
        .map(|(i, mut s)| {
            s.t = s.t.clamp(0.0, 1.0);
            (s, tagged == Some(i))
        })
        .collect();
    stops.sort_by(|a, b| a.0.t.total_cmp(&b.0.t));
    let at = stops.iter().position(|(_, tagged)| *tagged);
    let selection = selection.zip(at).map(|(s, i)| s.on(i)).filter(|s| s.fits(stops.len()));
    (stops.into_iter().map(|(s, _)| s).collect(), selection)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shader_project::AssetType;

    fn doc() -> GradientsDoc {
        let mut next = 0;
        let c = GradientCollection::new(|| {
            next += 1;
            next
        });
        GradientsDoc::new(AssetId::new(AssetType::Gradients, "g"), c)
    }

    #[test]
    fn new_stop_samples_the_gradient() {
        let mut d = doc();
        assert!(d.add_stop(0.0));
        d.set_stop_color(0, [0.0, 0.0, 0.0, 1.0], None);
        assert!(d.add_stop(1.0));
        d.set_stop_color(1, [1.0, 1.0, 1.0, 1.0], None);
        assert!(d.add_stop(0.5));
        let c = d.current().unwrap().stops[1].color;
        assert_eq!(Some(c), d.current().unwrap().sample(0.5));
        assert!(c[0] > 0.0 && c[0] < 1.0, "{c:?}");
    }

    #[test]
    fn mid_selection_follows_its_stop_and_undo_shows_the_changed_slot() {
        let mut d = doc();
        for t in [0.2, 0.4, 0.8] {
            d.add_stop(t);
        }
        d.select(Some(GradientSel::Mid(0)));
        let mut stops = d.current().unwrap().stops.clone();
        stops[0].t = 0.6;
        d.set_stops(stops, d.selection, None);
        assert_eq!(d.selection, Some(GradientSel::Mid(1)));
        d.select_slot(5);
        assert!(d.undo());
        assert_eq!(d.slot, 0, "undo selects the slot it changed");
    }

    #[test]
    fn move_slot_keeps_the_selected_slot_in_view() {
        let mut d = doc();
        d.select_slot(3);
        d.move_slot(1, 6);
        assert_eq!(d.slot, 2);
        d.move_slot(6, 0);
        assert_eq!(d.slot, 3);
    }
}
