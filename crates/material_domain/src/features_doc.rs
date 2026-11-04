//! Editing state and undo/redo history for a `.features.ron` Game Features table.

use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use editor_core::history::{History, MergeKey};
use project_core::{AssetRef, AssetUuid};
use shader_graph_core::features::{FeatureEnum, FeatureTable, FeatureVariant};
use shader_project::AssetId;

#[derive(Clone)]
struct DirtyState {
    saved_stamp: (usize, usize),
    saved_table: FeatureTable,
    last_check: Option<(u64, bool)>,
}

struct DirtyCache(Mutex<DirtyState>);

impl DirtyCache {
    fn new(saved: &str, saved_table: FeatureTable, revision: u64) -> Self {
        DirtyCache(Mutex::new(DirtyState {
            saved_stamp: (saved.as_ptr() as usize, saved.len()),
            saved_table,
            last_check: Some((revision, false)),
        }))
    }

    fn get(&self) -> std::sync::MutexGuard<'_, DirtyState> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

pub struct FeaturesDoc {
    pub aref: AssetRef,
    pub id: AssetId,
    pub table: FeatureTable,
    pub saved: String,
    pub revision: u64,
    /// Incremented on structural changes (adding/removing/reordering enums or variants, undo/redo, reload) so the
    /// view knows when to rebuild its widget tree.
    pub layout: u64,
    pub draft_enum_name: String,
    pub draft_variant_names: BTreeMap<AssetUuid, String>,
    pub form_error: Option<String>,
    history: History<FeatureTable>,
    dirty_cache: DirtyCache,
}

impl FeaturesDoc {
    pub fn new(id: AssetId, mut table: FeatureTable) -> Self {
        table.ensure_id();
        let saved = table.to_ron();
        let dirty_cache = DirtyCache::new(&saved, table.clone(), 0);
        Self {
            aref: id.to_ref(),
            id,
            table,
            saved,
            revision: 0,
            layout: 0,
            draft_enum_name: String::new(),
            draft_variant_names: BTreeMap::new(),
            form_error: None,
            history: History::default(),
            dirty_cache,
        }
    }

    pub fn reload(&mut self, mut table: FeatureTable) {
        table.ensure_id();
        self.table = table;
        self.history.clear();
        self.form_error = None;
        self.revision += 1;
        self.layout += 1;
        self.mark_saved();
    }

    pub fn mark_saved(&mut self) {
        self.table.ensure_id();
        self.saved = self.table.to_ron();
        let mut state = self.dirty_cache.get();
        state.saved_stamp = (self.saved.as_ptr() as usize, self.saved.len());
        state.saved_table = self.table.clone();
        state.last_check = Some((self.revision, false));
    }

    pub fn is_dirty(&self) -> bool {
        let stamp = (self.saved.as_ptr() as usize, self.saved.len());
        let mut state = self.dirty_cache.get();
        if stamp == state.saved_stamp {
            if let Some((rev, dirty)) = state.last_check
                && rev == self.revision
            {
                return dirty;
            }
            let dirty = self.table != state.saved_table;
            state.last_check = Some((self.revision, dirty));
            return dirty;
        }
        let dirty = self.table.to_ron() != self.saved;
        if !dirty {
            state.saved_stamp = stamp;
            state.saved_table = self.table.clone();
            state.last_check = Some((self.revision, false));
        } else if let Ok(parsed) = FeatureTable::from_ron(&self.saved) {
            state.saved_stamp = stamp;
            state.saved_table = parsed;
            state.last_check = Some((self.revision, true));
        }
        dirty
    }

    pub fn relayout(&mut self) {
        self.layout += 1;
    }

    pub fn edit_merge<R>(&mut self, key: Option<MergeKey>, f: impl FnOnce(&mut FeatureTable) -> R) -> R {
        let before = self.table.clone();
        let r = f(&mut self.table);
        let changed = self.table != before;
        self.history.record(changed.then_some(before), key);
        if changed {
            self.revision += 1;
        }
        r
    }

    pub fn end_merge(&mut self) {
        self.history.end_merge();
    }

    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    pub fn peek_undo(&self) -> Option<&FeatureTable> {
        self.history.peek_undo()
    }

    pub fn peek_redo(&self) -> Option<&FeatureTable> {
        self.history.peek_redo()
    }

    pub fn undo(&mut self) -> bool {
        let done = self.history.undo_keeping(&mut self.table, |m| &mut m.id);
        if done {
            self.form_error = None;
            self.revision += 1;
            self.layout += 1;
        }
        done
    }

    pub fn redo(&mut self) -> bool {
        let done = self.history.redo_keeping(&mut self.table, |m| &mut m.id);
        if done {
            self.form_error = None;
            self.revision += 1;
            self.layout += 1;
        }
        done
    }

    /// Adds a new enum with default `low` and `high` variants; returns its UUID.
    pub fn add_enum(&mut self, name: &str) -> AssetUuid {
        let en = FeatureEnum::new(name, vec![FeatureVariant::new("low"), FeatureVariant::new("high")]);
        let enum_id = en.id;
        self.edit_merge(None, |t| {
            t.enums.push(en);
        });
        self.form_error = None;
        self.layout += 1;
        enum_id
    }

    pub fn reset_enum_form(&mut self) {
        self.draft_enum_name.clear();
        self.form_error = None;
        self.layout += 1;
    }

    pub fn set_enum_name(&mut self, enum_id: AssetUuid, name: &str, merge: bool) {
        let key = merge.then(|| MergeKey::Named(format!("enum_name_{enum_id}")));
        self.edit_merge(key, |t| {
            if let Some(e) = t.get_by_id_mut(enum_id) {
                e.name = name.to_string();
            }
        });
    }

    pub fn remove_enum(&mut self, enum_id: AssetUuid) {
        self.edit_merge(None, |t| {
            t.enums.retain(|e| e.id != enum_id);
        });
        self.draft_variant_names.remove(&enum_id);
        self.form_error = None;
        self.layout += 1;
    }

    pub fn add_variant(&mut self, enum_id: AssetUuid, name: &str) -> Option<AssetUuid> {
        let var = FeatureVariant::new(name);
        let variant_id = var.id;
        let mut added = false;
        self.edit_merge(None, |t| {
            if let Some(e) = t.get_by_id_mut(enum_id) {
                e.variants.push(var);
                added = true;
            }
        });
        if added {
            self.draft_variant_names.remove(&enum_id);
            self.form_error = None;
            self.layout += 1;
            Some(variant_id)
        } else {
            None
        }
    }

    pub fn set_variant_name(&mut self, enum_id: AssetUuid, variant_id: AssetUuid, name: &str, merge: bool) {
        let key = merge.then(|| MergeKey::Named(format!("variant_name_{enum_id}_{variant_id}")));
        self.edit_merge(key, |t| {
            if let Some(e) = t.get_by_id_mut(enum_id)
                && let Some(v) = e.variants.iter_mut().find(|v| v.id == variant_id)
            {
                v.name = name.to_string();
            }
        });
    }

    pub fn remove_variant(&mut self, enum_id: AssetUuid, variant_id: AssetUuid) {
        self.edit_merge(None, |t| {
            if let Some(e) = t.get_by_id_mut(enum_id) {
                e.variants.retain(|v| v.id != variant_id);
            }
        });
        self.form_error = None;
        self.layout += 1;
    }

    pub fn move_variant(&mut self, enum_id: AssetUuid, variant_id: AssetUuid, dir: i32) {
        self.edit_merge(None, |t| {
            let Some(e) = t.get_by_id_mut(enum_id) else { return };
            let Some(pos) = e.variants.iter().position(|v| v.id == variant_id) else { return };
            let next = pos as i32 + dir;
            if next >= 0 && (next as usize) < e.variants.len() {
                e.variants.swap(pos, next as usize);
            }
        });
        self.layout += 1;
    }
}
