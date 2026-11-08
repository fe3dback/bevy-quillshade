//! `.collection.ron` document (`CollectionDoc`): in-memory `CollectionAsset`, undo/redo [`History`], selected row,
//! search filter, and [`TabDoc`] implementation.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};

use project_core::asset_uuid::AssetUuid;
use project_core::{AssetRef, Project};

use editor_core::action::{Action, Handled, TabAction};
use editor_core::doc_kind::{DocCtx, StatusCtx, StatusInfo, TabDoc};
use editor_core::dynval::{PropKey, PropTarget, PropValue};
use editor_core::history::{History, MergeKey};
use editor_core::tab::ViewKindId;
use editor_core::ui::icons::IconRef;

use crate::backend::model::{
    CollectionAsset, DataValue, DtoAsset, EnumAsset, FieldSettings, FieldType, MAX_NESTING_DEPTH, Row, collection_ref,
    ensure_nested_value_with_schemas_mut,
};
use crate::backend::ron_io::{load_collection, save_collection_with_schemas_and_index};

use super::COLLECTION_ICON;
use super::state::{DataState, DataStep};
use super::widgets::{
    apply_container_add, apply_container_move, apply_container_remove, apply_set_component, apply_set_option_some,
    apply_set_value, data_value_to_prop, switch_enum_variant_with_drafts,
};

pub const COLLECTION_VIEW: ViewKindId = ViewKindId("data.collection");

fn next_layout() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// Property keys for widgets inside the `.collection.ron` editor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CollectionProp {
    Filter,
    DtoSchema,
    RowValue { row: AssetUuid, path: Vec<AssetUuid> },
}

impl PropTarget for CollectionProp {}

/// Document-level actions for [`CollectionDoc`] (`TabAction::Doc`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CollectionAction {
    SelectRow(Option<AssetUuid>),
    AddRow,
    DuplicateRow(Option<AssetUuid>),
    DeleteRow(Option<AssetUuid>),
    MoveRow { id: AssetUuid, delta: isize },
    RegenerateRowId(AssetUuid),
    CopyRowId(AssetUuid),
    ClearFilter,
    ToggleDtoField { path: Vec<AssetUuid> },
    ResetCell { row: AssetUuid, path: Vec<AssetUuid> },
    SetOptionSome { row: AssetUuid, path: Vec<AssetUuid>, some: bool },
    ContainerAdd { row: AssetUuid, path: Vec<AssetUuid> },
    ContainerRemove { row: AssetUuid, path: Vec<AssetUuid>, index: usize },
    ContainerMove { row: AssetUuid, path: Vec<AssetUuid>, index: usize, delta: isize },
    FillDefaults,
    PruneUnknownFields,
}

impl From<CollectionAction> for Action {
    fn from(a: CollectionAction) -> Self {
        Action::Tab(TabAction::doc(a))
    }
}

/// Open `.collection.ron` document tab.
#[derive(Debug, Clone)]
pub struct CollectionDoc {
    pub path: String,
    pub aref: AssetRef,
    pub col: CollectionAsset,
    pub saved: CollectionAsset,
    pub history: History<CollectionAsset>,
    pub selected_row: Option<AssetUuid>,
    pub filter: String,
    pub expanded_dtos: BTreeSet<Vec<AssetUuid>>,
    pub cached_dto: Option<DtoAsset>,
    pub cached_dtos: BTreeMap<String, DtoAsset>,
    pub cached_enums: BTreeMap<String, EnumAsset>,
    /// In-memory per-variant payload drafts keyed by `(row_uuid, field_path)` -> `variant_uuid` -> `payload_field_map`
    /// so switching variants and back within a session restores previously edited payload values.
    pub variant_drafts: BTreeMap<(AssetUuid, Vec<AssetUuid>), super::widgets::VariantPayloadDrafts>,
    pub revision: u64,
    pub layout: u64,
    pub list_layout: u64,
}

impl CollectionDoc {
    pub fn new(path: impl Into<String>, mut col: CollectionAsset, cached_dto: Option<DtoAsset>) -> Self {
        col.ensure_id();
        let path = path.into();
        let selected_row = col.rows.first().map(|r| r.id);
        let saved = col.clone();
        let mut cached_dtos = BTreeMap::new();
        if let Some(d) = &cached_dto {
            cached_dtos.insert("cached".to_string(), d.clone());
        }
        Self {
            aref: collection_ref(&path),
            path,
            col,
            saved,
            history: History::default(),
            selected_row,
            filter: String::new(),
            expanded_dtos: BTreeSet::new(),
            cached_dto,
            cached_dtos,
            cached_enums: BTreeMap::new(),
            variant_drafts: BTreeMap::new(),
            revision: 1,
            layout: next_layout(),
            list_layout: next_layout(),
        }
    }

    pub fn with_dtos(mut self, mut dtos: BTreeMap<String, DtoAsset>) -> Self {
        if let Some(d) = &self.cached_dto
            && !dtos.values().any(|x| x.id == d.id)
        {
            dtos.insert("cached".to_string(), d.clone());
        }
        if self.cached_dto.is_none() {
            self.cached_dto = dtos.values().find(|d| d.id == self.col.dto).cloned();
        }
        self.cached_dtos = dtos;
        self
    }

    pub fn with_enums(mut self, enums: BTreeMap<String, EnumAsset>) -> Self {
        self.cached_enums = enums;
        self
    }

    pub fn with_schemas(self, dtos: BTreeMap<String, DtoAsset>, enums: BTreeMap<String, EnumAsset>) -> Self {
        self.with_dtos(dtos).with_enums(enums)
    }

    pub fn relayout(&mut self) {
        self.layout = next_layout();
        self.list_layout = next_layout();
    }

    pub fn relayout_list(&mut self) {
        self.list_layout = next_layout();
    }

    pub fn bump(&mut self, relayout: bool) {
        self.revision = self.revision.wrapping_add(1);
        if relayout {
            self.relayout();
        }
    }

    fn resolve_dto<'a>(&'a self, state: Option<&'a DataState>) -> Option<&'a DtoAsset> {
        if let Some(st) = state
            && let Some((_, dto)) = st.dto_by_id(self.col.dto)
        {
            return Some(dto);
        }
        self.cached_dto
            .as_ref()
            .filter(|d| d.id == self.col.dto)
            .or_else(|| self.cached_dtos.values().find(|d| d.id == self.col.dto))
    }

    pub fn resolve_dtos<'a>(&'a self, state: Option<&'a DataState>) -> &'a BTreeMap<String, DtoAsset> {
        if let Some(st) = state { &st.assets.dtos } else { &self.cached_dtos }
    }

    pub fn resolve_enums<'a>(&'a self, state: Option<&'a DataState>) -> &'a BTreeMap<String, EnumAsset> {
        if let Some(st) = state { &st.assets.enums } else { &self.cached_enums }
    }

    fn sync_cached_dto(&mut self, ctx: &mut DocCtx) {
        if let Some(st) = ctx.domain::<DataState>() {
            self.cached_dtos = st.assets.dtos.clone();
            self.cached_enums = st.assets.enums.clone();
            self.cached_dto = st.dto_by_id(self.col.dto).map(|(_, d)| d.clone());
        } else {
            if let Some(d) = &self.cached_dto
                && !self.cached_dtos.values().any(|x| x.id == d.id)
            {
                self.cached_dtos.insert("cached".to_string(), d.clone());
            }
            if self.cached_dto.as_ref().map(|d| d.id) != Some(self.col.dto) {
                self.cached_dto = self.cached_dtos.values().find(|d| d.id == self.col.dto).cloned();
            }
        }
    }

    pub fn field_meta_at_path(
        &self,
        path: &[AssetUuid],
        dtos: &BTreeMap<String, DtoAsset>,
    ) -> Option<(FieldType, FieldSettings, DataValue, bool)> {
        self.field_meta_at_path_with_schemas(path, dtos, &self.cached_enums)
    }

    pub fn field_meta_at_path_with_schemas(
        &self,
        path: &[AssetUuid],
        dtos: &BTreeMap<String, DtoAsset>,
        enums: &BTreeMap<String, EnumAsset>,
    ) -> Option<(FieldType, FieldSettings, DataValue, bool)> {
        let dto = self
            .cached_dto
            .as_ref()
            .filter(|d| d.id == self.col.dto)
            .or_else(|| dtos.values().find(|d| d.id == self.col.dto))?;
        dto.field_meta_at_path_with_schemas(path, dtos, enums, false)
    }

    pub fn get_value_at_path(
        &self,
        row_id: AssetUuid,
        path: &[AssetUuid],
        dtos: &BTreeMap<String, DtoAsset>,
    ) -> Option<DataValue> {
        self.get_value_at_path_with_schemas(row_id, path, dtos, &self.cached_enums)
    }

    pub fn get_value_at_path_with_schemas(
        &self,
        row_id: AssetUuid,
        path: &[AssetUuid],
        dtos: &BTreeMap<String, DtoAsset>,
        enums: &BTreeMap<String, EnumAsset>,
    ) -> Option<DataValue> {
        if path.is_empty() || path.len() > MAX_NESTING_DEPTH {
            return None;
        }
        let dto = self
            .cached_dto
            .as_ref()
            .filter(|d| d.id == self.col.dto)
            .or_else(|| dtos.values().find(|d| d.id == self.col.dto))?;
        let f0 = dto.fields.iter().find(|f| f.id == path[0])?;
        let row = self.col.rows.iter().rfind(|r| r.id == row_id)?;
        if path.len() == 1 {
            return Some(row.values.get(&f0.id).cloned().unwrap_or_else(|| f0.default_with_schemas(dtos, enums)));
        }
        if let Some(root_val) = row.values.get(&f0.id)
            && let Some(found) = root_val.get_at_path(&path[1..])
        {
            return Some(found.clone());
        }
        let (_, _, def, _) = dto.field_meta_at_path_with_schemas(path, dtos, enums, false)?;
        Some(def)
    }

    pub fn get_value_mut_at_path<'a>(
        col: &'a mut CollectionAsset,
        dto: &DtoAsset,
        row_idx: usize,
        path: &[AssetUuid],
        dtos: &BTreeMap<String, DtoAsset>,
    ) -> Option<&'a mut DataValue> {
        Self::get_value_mut_at_path_with_schemas(col, dto, row_idx, path, dtos, &BTreeMap::new())
    }

    pub fn get_value_mut_at_path_with_schemas<'a>(
        col: &'a mut CollectionAsset,
        dto: &DtoAsset,
        row_idx: usize,
        path: &[AssetUuid],
        dtos: &BTreeMap<String, DtoAsset>,
        enums: &BTreeMap<String, EnumAsset>,
    ) -> Option<&'a mut DataValue> {
        if path.is_empty() || path.len() > MAX_NESTING_DEPTH {
            return None;
        }
        let f0 = dto.fields.iter().find(|f| f.id == path[0])?;
        let row = col.rows.get_mut(row_idx)?;
        let root_def = f0.default_with_schemas(dtos, enums);
        let root_val = row.values.entry(f0.id).or_insert_with(|| root_def.clone());
        if !f0.ty.matches(root_val) {
            *root_val = root_def;
        }
        if path.len() == 1 {
            return Some(root_val);
        }
        ensure_nested_value_with_schemas_mut(root_val, f0.ty.clone(), dtos, enums, &path[1..])
    }

    /// Programmatically adds a new row populated with the bound DTO's defaults.
    pub fn add_row(&mut self, dto: Option<&DtoAsset>) -> AssetUuid {
        let before = self.col.clone();
        let row = match dto.or(self.cached_dto.as_ref()) {
            Some(d) => Row::from_dto_defaults_with_schemas(d, &self.cached_dtos, &self.cached_enums),
            None => Row { id: AssetUuid::new_v7(), values: Default::default() },
        };
        let id = row.id;
        self.col.rows.push(row);
        self.history.record(Some(before), None);
        self.selected_row = Some(id);
        self.bump(true);
        id
    }

    /// Duplicates `row_id` (or `selected_row` if `None`), inserting the clone right after the source row.
    pub fn duplicate_row(&mut self, row_id: Option<AssetUuid>) -> Option<AssetUuid> {
        let target_id = row_id.or(self.selected_row)?;
        let idx = self.col.rows.iter().position(|r| r.id == target_id)?;
        let before = self.col.clone();
        let mut copy = self.col.rows[idx].clone();
        let new_id = AssetUuid::new_v7();
        copy.id = new_id;
        self.col.rows.insert(idx + 1, copy);
        self.history.record(Some(before), None);
        self.selected_row = Some(new_id);
        self.bump(true);
        Some(new_id)
    }

    /// Deletes `row_id` (or `selected_row` if `None`).
    pub fn delete_row(&mut self, row_id: Option<AssetUuid>) -> bool {
        let Some(target_id) = row_id.or(self.selected_row) else {
            return false;
        };
        let Some(idx) = self.col.rows.iter().position(|r| r.id == target_id) else {
            return false;
        };
        let before = self.col.clone();
        self.col.rows.remove(idx);
        self.history.record(Some(before), None);
        if self.selected_row == Some(target_id) {
            self.selected_row = self.col.rows.get(idx).or_else(|| self.col.rows.last()).map(|r| r.id);
        }
        self.bump(true);
        true
    }

    /// Moves `id` by `delta` (`-1` up, `+1` down).
    pub fn move_row(&mut self, id: AssetUuid, delta: isize) -> bool {
        let Some(idx) = self.col.rows.iter().position(|r| r.id == id) else {
            return false;
        };
        let next = idx as isize + delta;
        if next < 0 || next as usize >= self.col.rows.len() {
            return false;
        }
        let before = self.col.clone();
        self.col.rows.swap(idx, next as usize);
        self.history.record(Some(before), None);
        self.bump(true);
        true
    }

    /// Mints a fresh UUIDv7 for the row matching `target_id` (updating the last matching row when duplicate rows share
    /// `target_id` so the D5 duplicate loser is resolved).
    pub fn regenerate_row_id(&mut self, target_id: AssetUuid) -> Option<AssetUuid> {
        let idx = self.col.rows.iter().rposition(|r| r.id == target_id)?;
        let before = self.col.clone();
        let new_id = AssetUuid::new_v7();
        self.col.rows[idx].id = new_id;
        self.history.record(Some(before), None);
        self.selected_row = Some(new_id);
        if self.filter.trim().eq_ignore_ascii_case(&target_id.to_string()) {
            self.filter.clear();
        }
        self.bump(true);
        Some(new_id)
    }

    fn apply_doc_action(&mut self, ctx: &mut DocCtx, action: &CollectionAction) -> Handled {
        self.sync_cached_dto(ctx);
        match action {
            CollectionAction::SelectRow(id) => {
                if self.selected_row != *id {
                    self.selected_row = *id;
                    self.relayout();
                    ctx.redraw_props = true;
                }
                Handled::Yes
            }
            CollectionAction::AddRow => {
                let dto = self.cached_dto.clone();
                self.add_row(dto.as_ref());
                ctx.redraw_props = true;
                ctx.defer(DataStep::SyncCollectionInMemory { path: self.path.clone(), col: self.col.clone() });
                Handled::Yes
            }
            CollectionAction::DuplicateRow(id) => {
                if self.duplicate_row(*id).is_some() {
                    ctx.redraw_props = true;
                    ctx.defer(DataStep::SyncCollectionInMemory { path: self.path.clone(), col: self.col.clone() });
                }
                Handled::Yes
            }
            CollectionAction::DeleteRow(id) => {
                if self.delete_row(*id) {
                    ctx.redraw_props = true;
                    ctx.defer(DataStep::SyncCollectionInMemory { path: self.path.clone(), col: self.col.clone() });
                }
                Handled::Yes
            }
            CollectionAction::MoveRow { id, delta } => {
                if self.move_row(*id, *delta) {
                    ctx.redraw_props = true;
                    ctx.defer(DataStep::SyncCollectionInMemory { path: self.path.clone(), col: self.col.clone() });
                }
                Handled::Yes
            }
            CollectionAction::RegenerateRowId(target_id) => {
                if self.regenerate_row_id(*target_id).is_some() {
                    ctx.redraw_props = true;
                    ctx.defer(DataStep::SyncCollectionInMemory { path: self.path.clone(), col: self.col.clone() });
                }
                Handled::Yes
            }
            CollectionAction::CopyRowId(row_id) => {
                ctx.defer(DataStep::CopyText(row_id.to_string()));
                Handled::Yes
            }
            CollectionAction::ClearFilter => {
                if !self.filter.is_empty() {
                    self.filter.clear();
                    self.relayout_list();
                }
                Handled::Yes
            }
            CollectionAction::ToggleDtoField { path } => {
                if !self.expanded_dtos.remove(path) {
                    self.expanded_dtos.insert(path.clone());
                }
                self.relayout();
                Handled::Yes
            }
            CollectionAction::ResetCell { row, path } => {
                let dtos = self.cached_dtos.clone();
                let enums = self.cached_enums.clone();
                let Some(dto) =
                    self.cached_dto.clone().or_else(|| dtos.values().find(|d| d.id == self.col.dto).cloned())
                else {
                    return Handled::No;
                };
                let Some((_, _, default_val, _)) = dto.field_meta_at_path_with_schemas(path, &dtos, &enums, false)
                else {
                    return Handled::No;
                };
                let Some(r_idx) = self.col.rows.iter().position(|r| r.id == *row) else {
                    return Handled::No;
                };
                let cur_val = self.get_value_at_path_with_schemas(*row, path, &dtos, &enums);
                if cur_val.as_ref() != Some(&default_val) {
                    let before = self.col.clone();
                    if let Some(slot) =
                        Self::get_value_mut_at_path_with_schemas(&mut self.col, &dto, r_idx, path, &dtos, &enums)
                    {
                        *slot = default_val;
                        self.history.record(Some(before), None);
                        self.bump(true);
                        ctx.redraw_props = true;
                        ctx.defer(DataStep::SyncCollectionInMemory { path: self.path.clone(), col: self.col.clone() });
                    }
                }
                Handled::Yes
            }
            CollectionAction::SetOptionSome { row, path, some } => {
                let dtos = self.cached_dtos.clone();
                let enums = self.cached_enums.clone();
                let Some(dto) =
                    self.cached_dto.clone().or_else(|| dtos.values().find(|d| d.id == self.col.dto).cloned())
                else {
                    return Handled::No;
                };
                let Some((ty, _, _, _)) = dto.field_meta_at_path_with_schemas(path, &dtos, &enums, false) else {
                    return Handled::No;
                };
                let Some(r_idx) = self.col.rows.iter().position(|r| r.id == *row) else {
                    return Handled::No;
                };
                let before = self.col.clone();
                if let Some(slot) =
                    Self::get_value_mut_at_path_with_schemas(&mut self.col, &dto, r_idx, path, &dtos, &enums)
                    && apply_set_option_some(&ty, slot, *some, &dtos, &enums)
                {
                    if !*some {
                        self.expanded_dtos.remove(path);
                    }
                    self.history.record(Some(before), None);
                    self.bump(true);
                    ctx.redraw_props = true;
                    ctx.defer(DataStep::SyncCollectionInMemory { path: self.path.clone(), col: self.col.clone() });
                }
                Handled::Yes
            }
            CollectionAction::ContainerAdd { row, path } => {
                let dtos = self.cached_dtos.clone();
                let enums = self.cached_enums.clone();
                let Some(dto) =
                    self.cached_dto.clone().or_else(|| dtos.values().find(|d| d.id == self.col.dto).cloned())
                else {
                    return Handled::No;
                };
                let Some((ty, _, _, _)) = dto.field_meta_at_path_with_schemas(path, &dtos, &enums, false) else {
                    return Handled::No;
                };
                let Some(r_idx) = self.col.rows.iter().position(|r| r.id == *row) else {
                    return Handled::No;
                };
                let before = self.col.clone();
                if let Some(slot) =
                    Self::get_value_mut_at_path_with_schemas(&mut self.col, &dto, r_idx, path, &dtos, &enums)
                    && apply_container_add(&ty, slot, &dtos, &enums)
                {
                    self.expanded_dtos.insert(path.clone());
                    self.history.record(Some(before), None);
                    self.bump(true);
                    ctx.redraw_props = true;
                    ctx.defer(DataStep::SyncCollectionInMemory { path: self.path.clone(), col: self.col.clone() });
                }
                Handled::Yes
            }
            CollectionAction::ContainerRemove { row, path, index } => {
                let dtos = self.cached_dtos.clone();
                let enums = self.cached_enums.clone();
                let Some(dto) =
                    self.cached_dto.clone().or_else(|| dtos.values().find(|d| d.id == self.col.dto).cloned())
                else {
                    return Handled::No;
                };
                let Some((ty, _, _, _)) = dto.field_meta_at_path_with_schemas(path, &dtos, &enums, false) else {
                    return Handled::No;
                };
                let Some(r_idx) = self.col.rows.iter().position(|r| r.id == *row) else {
                    return Handled::No;
                };
                let before = self.col.clone();
                if let Some(slot) =
                    Self::get_value_mut_at_path_with_schemas(&mut self.col, &dto, r_idx, path, &dtos, &enums)
                    && apply_container_remove(&ty, slot, *index)
                {
                    self.history.record(Some(before), None);
                    self.bump(true);
                    ctx.redraw_props = true;
                    ctx.defer(DataStep::SyncCollectionInMemory { path: self.path.clone(), col: self.col.clone() });
                }
                Handled::Yes
            }
            CollectionAction::ContainerMove { row, path, index, delta } => {
                let dtos = self.cached_dtos.clone();
                let enums = self.cached_enums.clone();
                let Some(dto) =
                    self.cached_dto.clone().or_else(|| dtos.values().find(|d| d.id == self.col.dto).cloned())
                else {
                    return Handled::No;
                };
                let Some((ty, _, _, _)) = dto.field_meta_at_path_with_schemas(path, &dtos, &enums, false) else {
                    return Handled::No;
                };
                let Some(r_idx) = self.col.rows.iter().position(|r| r.id == *row) else {
                    return Handled::No;
                };
                let before = self.col.clone();
                if let Some(slot) =
                    Self::get_value_mut_at_path_with_schemas(&mut self.col, &dto, r_idx, path, &dtos, &enums)
                    && apply_container_move(&ty, slot, *index, *delta)
                {
                    self.history.record(Some(before), None);
                    self.bump(true);
                    ctx.redraw_props = true;
                    ctx.defer(DataStep::SyncCollectionInMemory { path: self.path.clone(), col: self.col.clone() });
                }
                Handled::Yes
            }
            CollectionAction::FillDefaults => {
                if let Some(dto) = self.cached_dto.clone() {
                    let before = self.col.clone();
                    self.col.fill_defaults_with_schemas(&dto, &self.cached_dtos, &self.cached_enums);
                    if self.col != before {
                        self.history.record(Some(before), None);
                        self.bump(true);
                        ctx.redraw_props = true;
                        ctx.defer(DataStep::SyncCollectionInMemory { path: self.path.clone(), col: self.col.clone() });
                    }
                }
                Handled::Yes
            }
            CollectionAction::PruneUnknownFields => {
                if let Some(dto) = self.cached_dto.clone() {
                    let before = self.col.clone();
                    let removed =
                        self.col.prune_unknown_fields_with_schemas(&dto, &self.cached_dtos, &self.cached_enums);
                    if removed > 0 {
                        self.history.record(Some(before), None);
                        self.bump(true);
                        ctx.redraw_props = true;
                        ctx.defer(DataStep::SyncCollectionInMemory { path: self.path.clone(), col: self.col.clone() });
                    }
                }
                Handled::Yes
            }
        }
    }
}

impl TabDoc for CollectionDoc {
    fn asset(&self) -> Option<&AssetRef> {
        Some(&self.aref)
    }

    fn view(&self) -> ViewKindId {
        COLLECTION_VIEW
    }

    fn base_title(&self) -> String {
        format!("quillshade/data/{}.collection", self.path)
    }

    fn icon(&self) -> IconRef {
        COLLECTION_ICON
    }

    fn watermark(&self) -> Option<&'static str> {
        Some("COLLECTION")
    }

    fn is_dirty(&self) -> bool {
        self.col != self.saved
    }

    fn is_read_only(&self) -> bool {
        false
    }

    fn revision(&self) -> u64 {
        self.revision
    }

    fn save(&mut self, ctx: &mut DocCtx) -> io::Result<()> {
        let Some(project) = ctx.project else {
            return Err(io::Error::other("No open project"));
        };
        let file = project.root.join(self.aref.source_rel());
        let (dto_opt, dtos, enums) = if let Some(st) = ctx.domain::<DataState>() {
            let d = st.dto_by_id(self.col.dto).map(|(_, d)| d.clone()).or_else(|| self.cached_dto.clone());
            (d, st.assets.dtos.clone(), st.assets.enums.clone())
        } else {
            (self.cached_dto.clone(), self.cached_dtos.clone(), self.cached_enums.clone())
        };
        save_collection_with_schemas_and_index(&file, &mut self.col, dto_opt.as_ref(), &dtos, &enums, Some(ctx.index))?;
        self.saved = self.col.clone();
        self.bump(false);
        ctx.defer(DataStep::SyncCollectionInMemory { path: self.path.clone(), col: self.col.clone() });
        Ok(())
    }

    fn changed_on_disk(&self, project: &Project) -> bool {
        let file = project.root.join(self.aref.source_rel());
        load_collection(&file).is_ok_and(|disk| disk != self.saved)
    }

    fn reload_from_disk(&mut self, ctx: &mut DocCtx) -> Result<(), String> {
        let Some(project) = ctx.project else {
            return Err("No open project".into());
        };
        let file = project.root.join(self.aref.source_rel());
        let mut loaded = load_collection(&file).map_err(|e| e.to_string())?;
        loaded.ensure_id();
        self.col = loaded.clone();
        self.saved = loaded;
        self.history = History::default();
        self.sync_cached_dto(ctx);
        if self.selected_row.is_none_or(|id| !self.col.rows.iter().any(|r| r.id == id)) {
            self.selected_row = self.col.rows.first().map(|r| r.id);
        }
        self.bump(true);
        ctx.redraw_props = true;
        ctx.defer(DataStep::SyncCollectionInMemory { path: self.path.clone(), col: self.col.clone() });
        Ok(())
    }

    fn keep_mine(&mut self, project: &Project) {
        let file = project.root.join(self.aref.source_rel());
        if let Ok(disk) = load_collection(&file) {
            self.saved = disk;
            self.bump(false);
        }
    }

    fn rename(&mut self, _project: &Project, from: &AssetRef, to: &AssetRef) {
        if &self.aref == from {
            self.path = to.path.clone();
            self.aref = to.clone();
            self.bump(true);
        }
    }

    fn config_key(&self) -> Option<String> {
        Some(self.aref.source_rel())
    }

    fn status(&self, ctx: &StatusCtx) -> Option<StatusInfo> {
        let n_rows = self.col.rows.len();
        let rows_label = if n_rows == 1 { "1 row".to_string() } else { format!("{n_rows} rows") };
        let schema_label = if let Some(st) = ctx.domain::<DataState>()
            && let Some((dto_path, _)) = st.dto_by_id(self.col.dto)
        {
            format!("schema: {dto_path}")
        } else if self.col.dto.is_nil() {
            "schema: (none)".to_string()
        } else {
            format!("schema: {}", self.col.dto)
        };
        Some(StatusInfo::new(self.is_dirty(), ctx, vec![rows_label, schema_label]))
    }

    fn apply(&mut self, ctx: &mut DocCtx, action: &TabAction) -> Handled {
        self.sync_cached_dto(ctx);
        match action {
            TabAction::Undo => {
                if self.history.undo_keeping(&mut self.col, |m| &mut m.id) {
                    self.sync_cached_dto(ctx);
                    if self.selected_row.is_none_or(|id| !self.col.rows.iter().any(|r| r.id == id)) {
                        self.selected_row = self.col.rows.first().map(|r| r.id);
                    }
                    self.bump(true);
                    ctx.redraw_props = true;
                    ctx.defer(DataStep::SyncCollectionInMemory { path: self.path.clone(), col: self.col.clone() });
                }
                Handled::Yes
            }
            TabAction::Redo => {
                if self.history.redo_keeping(&mut self.col, |m| &mut m.id) {
                    self.sync_cached_dto(ctx);
                    if self.selected_row.is_none_or(|id| !self.col.rows.iter().any(|r| r.id == id)) {
                        self.selected_row = self.col.rows.first().map(|r| r.id);
                    }
                    self.bump(true);
                    ctx.redraw_props = true;
                    ctx.defer(DataStep::SyncCollectionInMemory { path: self.path.clone(), col: self.col.clone() });
                }
                Handled::Yes
            }
            TabAction::CommitInline => {
                self.history.end_merge();
                Handled::Yes
            }
            TabAction::DeleteSelection => self.apply_doc_action(ctx, &CollectionAction::DeleteRow(None)),
            TabAction::SetValue { key, value, merge } => {
                let Some(prop) = key.downcast::<CollectionProp>() else {
                    return Handled::No;
                };
                match prop {
                    CollectionProp::Filter => {
                        if let PropValue::Str(s) = value {
                            self.filter = s.clone();
                            self.relayout_list();
                            return Handled::Yes;
                        }
                        Handled::No
                    }
                    CollectionProp::DtoSchema => {
                        let PropValue::Str(s) = value else { return Handled::No };
                        let next_dto = if s.trim().is_empty() {
                            AssetUuid::NIL
                        } else if let Some(id) = AssetUuid::parse(s.trim()) {
                            id
                        } else {
                            return Handled::No;
                        };
                        if self.col.dto == next_dto {
                            return Handled::Yes;
                        }
                        let before = self.col.clone();
                        self.col.dto = next_dto;
                        self.sync_cached_dto(ctx);
                        if let Some(dto) = self.cached_dto.clone() {
                            self.col.fill_defaults_with_schemas(&dto, &self.cached_dtos, &self.cached_enums);
                        }
                        self.history.record(Some(before), None);
                        self.bump(true);
                        ctx.redraw_props = true;
                        ctx.defer(DataStep::SyncCollectionInMemory { path: self.path.clone(), col: self.col.clone() });
                        Handled::Yes
                    }
                    CollectionProp::RowValue { row, path } => {
                        let dtos = self.cached_dtos.clone();
                        let enums = self.cached_enums.clone();
                        let Some(dto) =
                            self.cached_dto.clone().or_else(|| dtos.values().find(|d| d.id == self.col.dto).cloned())
                        else {
                            return Handled::No;
                        };
                        let Some((ty, settings, _, _)) =
                            dto.field_meta_at_path_with_schemas(path, &dtos, &enums, false)
                        else {
                            return Handled::No;
                        };
                        let Some(r_idx) = self.col.rows.iter().position(|r| r.id == *row) else {
                            return Handled::No;
                        };
                        let before = self.col.clone();
                        let Some(entry) =
                            Self::get_value_mut_at_path_with_schemas(&mut self.col, &dto, r_idx, path, &dtos, &enums)
                        else {
                            return Handled::No;
                        };
                        if let FieldType::Enum(enum_id) = &ty {
                            if let PropValue::Str(s) = value
                                && let Some(next_var) = AssetUuid::parse(s.trim())
                            {
                                let en_opt = enums.values().find(|e| e.id == *enum_id);
                                let drafts = self.variant_drafts.entry((*row, path.clone())).or_default();
                                if switch_enum_variant_with_drafts(entry, en_opt, next_var, &dtos, &enums, drafts) {
                                    let mkey = merge.then(|| MergeKey::from(key.clone()));
                                    self.history.record(Some(before), mkey);
                                    self.bump(true);
                                    ctx.redraw_props = true;
                                    ctx.defer(DataStep::SyncCollectionInMemory {
                                        path: self.path.clone(),
                                        col: self.col.clone(),
                                    });
                                }
                            }
                            return Handled::Yes;
                        }
                        if apply_set_value(&ty, entry, value, &settings) {
                            let mkey = merge.then(|| MergeKey::from(key.clone()));
                            self.history.record(Some(before), mkey);
                            self.bump(ty.is_asset_ref() || ty.is_soft_ref());
                            ctx.defer(DataStep::SyncCollectionInMemory {
                                path: self.path.clone(),
                                col: self.col.clone(),
                            });
                        }
                        Handled::Yes
                    }
                }
            }
            TabAction::SetComponent { key, comp, value, is_final } => {
                let Some(CollectionProp::RowValue { row, path }) = key.downcast::<CollectionProp>() else {
                    return Handled::No;
                };
                let dtos = self.cached_dtos.clone();
                let enums = self.cached_enums.clone();
                let Some(dto) =
                    self.cached_dto.clone().or_else(|| dtos.values().find(|d| d.id == self.col.dto).cloned())
                else {
                    return Handled::No;
                };
                let Some((ty, settings, _, _)) = dto.field_meta_at_path_with_schemas(path, &dtos, &enums, false) else {
                    return Handled::No;
                };
                let Some(r_idx) = self.col.rows.iter().position(|r| r.id == *row) else {
                    return Handled::No;
                };
                let before = self.col.clone();
                let Some(entry) =
                    Self::get_value_mut_at_path_with_schemas(&mut self.col, &dto, r_idx, path, &dtos, &enums)
                else {
                    return Handled::No;
                };
                if apply_set_component(&ty, entry, *comp, *value, &settings) {
                    let mkey = MergeKey::from(key.clone());
                    self.history.record(Some(before), Some(mkey));
                    self.bump(false);
                    ctx.defer(DataStep::SyncCollectionInMemory { path: self.path.clone(), col: self.col.clone() });
                }
                if *is_final {
                    self.history.end_merge();
                }
                Handled::Yes
            }
            TabAction::Doc(payload) => {
                let Some(act) = payload.downcast::<CollectionAction>() else {
                    return Handled::No;
                };
                self.apply_doc_action(ctx, act)
            }
            _ => Handled::No,
        }
    }

    fn current_value(&self, key: &PropKey, ctx: &StatusCtx) -> Option<PropValue> {
        let prop = key.downcast::<CollectionProp>()?;
        match prop {
            CollectionProp::Filter => Some(PropValue::Str(self.filter.clone())),
            CollectionProp::DtoSchema => {
                Some(PropValue::Str(if self.col.dto.is_nil() { String::new() } else { self.col.dto.to_string() }))
            }
            CollectionProp::RowValue { row, path } => {
                let dtos = self.resolve_dtos(ctx.domain::<DataState>());
                let enums = self.resolve_enums(ctx.domain::<DataState>());
                let dto = self.resolve_dto(ctx.domain::<DataState>())?;
                let (ty, _, _, _) = dto.field_meta_at_path_with_schemas(path, dtos, enums, false)?;
                let val = self.get_value_at_path_with_schemas(*row, path, dtos, enums)?;
                data_value_to_prop(&ty, &val)
            }
        }
    }
}
