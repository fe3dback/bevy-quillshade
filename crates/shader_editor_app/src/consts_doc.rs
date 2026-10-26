//! Open global constant table (decision 0017): the table (source of truth), undo/redo history, the add form.
//!
//! Pure data, no Bevy: the view rebuilds from [`ConstsDoc::table`] when [`ConstsDoc::layout`] changes.

use std::sync::atomic::{AtomicU64, Ordering};

use shader_graph_core::Value;
use shader_graph_core::consts::{ConstType, GlobalConst, GlobalConsts};
use shader_project::{AssetId, AssetSet, Project, assets, ops};

use crate::doc_kind::{TabDoc, asset_title, io_error};
use crate::history::History;
use crate::tab::ViewKind;
use crate::ui::icons::IconId;

/// A fresh value for [`ConstsDoc::layout`]: unique across documents, so a closed and reopened table never looks
/// like the view it had before.
fn next_layout() -> u64 {
    static LAYOUT: AtomicU64 = AtomicU64::new(1);
    LAYOUT.fetch_add(1, Ordering::Relaxed)
}

#[derive(Debug, Clone)]
pub struct ConstsDoc {
    pub id: AssetId,
    pub table: GlobalConsts,
    /// RON text of the last saved state (dirty check).
    pub saved: String,
    history: History<GlobalConsts>,
    /// Name typed into the add form (editor-only, not undoable).
    pub draft_name: String,
    /// Type picked in the add form.
    pub draft_ty: ConstType,
    /// Why the last Add was refused.
    pub form_error: Option<String>,
    /// Bumped on every change of the table.
    pub revision: u64,
    /// Changes when the view must be rebuilt: rows added/removed, a type changed, undo/redo, reload, the form
    /// reset ([`ConstsDoc::relayout`]). Value edits keep it (a rebuild would despawn the slider or field being
    /// dragged or typed into).
    pub layout: u64,
}

impl ConstsDoc {
    pub fn new(id: AssetId, table: GlobalConsts) -> Self {
        let saved = table.to_ron();
        ConstsDoc {
            id,
            table,
            saved,
            history: History::default(),
            draft_name: String::new(),
            draft_ty: ConstType::default(),
            form_error: None,
            revision: 1,
            layout: next_layout(),
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.table.to_ron() != self.saved
    }

    pub fn mark_saved(&mut self) {
        self.saved = self.table.to_ron();
    }

    /// Applies `f` as one undoable step; consecutive edits with the same `key` share one step (a slider drag).
    fn edit(&mut self, key: Option<&str>, relayout: bool, f: impl FnOnce(&mut GlobalConsts)) {
        let before = self.table.clone();
        f(&mut self.table);
        self.table.normalize();
        let changed = self.table != before;
        self.history.record(changed.then_some(before), key);
        if changed {
            self.revision += 1;
            if relayout {
                self.relayout();
            }
        }
    }

    /// Appends a constant with the value 1.0 in every component (`true` for flags): zero is a poor default in
    /// shaders (`log(0)`, `x / 0` fail to compile).
    pub fn add(&mut self, name: &str, ty: ConstType) {
        let c = GlobalConst::new(name, ty, Value::Float(1.0));
        self.edit(None, true, |t| t.consts.push(c));
    }

    pub fn remove(&mut self, name: &str) {
        self.edit(None, true, |t| t.consts.retain(|c| c.name != name));
    }

    /// Changes a constant's type; the value is converted (`vec3` → `f32` keeps `x`, numbers → bool: not zero).
    pub fn set_type(&mut self, name: &str, ty: ConstType) {
        self.edit(None, true, |t| {
            if let Some(c) = t.get_mut(name) {
                c.value = ty.normalize(&c.value);
                c.ty = ty;
            }
        });
    }

    /// Sets a constant's value (normalized to its type); `merge` edits of one constant are one undo step.
    pub fn set_value(&mut self, name: &str, value: Value, merge: bool) {
        let key = format!("value:{name}");
        self.edit(merge.then_some(key.as_str()), false, |t| {
            if let Some(c) = t.get_mut(name) {
                c.value = c.ty.normalize(&value);
            }
        });
    }

    /// Clears the add form after a successful Add (rebuilds the view, which empties the name field).
    pub fn reset_form(&mut self) {
        self.draft_name.clear();
        self.form_error = None;
        self.relayout();
    }

    /// Makes the view rebuild from the model.
    pub fn relayout(&mut self) {
        self.layout = next_layout();
    }

    /// The table an undo would restore.
    pub fn peek_undo(&self) -> Option<&GlobalConsts> {
        self.history.peek_undo()
    }

    /// The table a redo would restore.
    pub fn peek_redo(&self) -> Option<&GlobalConsts> {
        self.history.peek_redo()
    }

    pub fn end_merge(&mut self) {
        self.history.end_merge();
    }

    pub fn undo(&mut self) -> bool {
        let done = self.history.undo(&mut self.table);
        if done {
            self.after_history();
        }
        done
    }

    pub fn redo(&mut self) -> bool {
        let done = self.history.redo(&mut self.table);
        if done {
            self.after_history();
        }
        done
    }

    fn after_history(&mut self) {
        self.revision += 1;
        self.relayout();
    }

    /// Replaces the table (external reload) and clears history.
    pub fn reload(&mut self, table: GlobalConsts) {
        self.table = table;
        self.mark_saved();
        self.history.clear();
        self.after_history();
    }
}

impl TabDoc for ConstsDoc {
    fn asset(&self) -> Option<&AssetId> {
        Some(&self.id)
    }

    fn view(&self) -> ViewKind {
        ViewKind::Consts
    }

    fn base_title(&self) -> String {
        asset_title(&self.id)
    }

    fn icon(&self) -> IconId {
        IconId::Consts
    }

    fn kind_label(&self) -> &'static str {
        "consts"
    }

    fn is_dirty(&self) -> bool {
        ConstsDoc::is_dirty(self)
    }

    fn is_read_only(&self) -> bool {
        false
    }

    fn revision(&self) -> u64 {
        self.revision
    }

    fn save(&mut self, project: &Project) -> std::io::Result<()> {
        ops::save_consts(project, &self.id, &self.table).map_err(io_error)?;
        self.mark_saved();
        Ok(())
    }

    fn changed_on_disk(&self, project: &Project) -> bool {
        assets::load_consts(project, &self.id).is_ok_and(|c| GlobalConsts::from_ron(&self.saved).ok() != Some(c))
    }

    fn reload_from_disk(&mut self, project: &Project) -> Result<(), String> {
        self.reload(assets::load_consts(project, &self.id)?);
        Ok(())
    }

    fn keep_mine(&mut self, project: &Project) {
        if let Ok(on_disk) = assets::load_consts(project, &self.id) {
            self.saved = on_disk.to_ron();
        }
    }

    fn rename(&mut self, _project: &Project, from: &AssetId, to: &AssetId) {
        if self.id != *from {
            return;
        }
        self.id = to.clone();
        self.table.name = to.name().to_string();
        self.mark_saved();
        self.revision += 1;
        self.relayout();
    }

    fn config_key(&self) -> Option<String> {
        Some(self.id.source_rel())
    }

    fn override_into(&self, set: &mut AssetSet) {
        set.consts.insert(self.id.clone(), Ok(self.table.clone()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shader_project::AssetType;

    fn doc() -> ConstsDoc {
        ConstsDoc::new(AssetId::new(AssetType::Consts, "t"), GlobalConsts::new("t", Vec::new()))
    }

    #[test]
    fn edits_undo_and_layout() {
        let mut d = doc();
        d.add("wind", ConstType::Vec3);
        assert_eq!(d.table.get("wind").unwrap().value, Value::Float3([1.0; 3]));
        assert!(d.is_dirty());
        let layout = d.layout;
        for x in [0.1, 0.2, 0.3] {
            d.set_value("wind", Value::Float3([x, 0.5, 0.5]), true);
        }
        d.end_merge();
        assert_eq!(d.layout, layout, "value edits keep the view");
        d.set_type("wind", ConstType::F32);
        assert_eq!(d.table.get("wind").unwrap().value, Value::Float(0.3));
        assert!(d.layout > layout);
        assert!(d.undo());
        assert_eq!(d.table.get("wind").unwrap().value, Value::Float3([0.3, 0.5, 0.5]));
        assert!(d.undo(), "the drag was one step");
        assert_eq!(d.table.get("wind").unwrap().value, Value::Float3([1.0; 3]));
        d.remove("wind");
        assert!(!d.is_dirty(), "back to the saved (empty) table");
        assert!(d.undo());
        assert!(d.table.get("wind").is_some());
    }
}
