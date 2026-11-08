//! Data domain state (`DataState`), domain steps (`DataStep`), asset creation/opening/renaming/deletion,
//! live validation, and incremental code generation.

use project_core::asset_uuid::AssetUuid;
use project_core::ident::check_path;
use project_core::regen::{self, DomainBackend};
use project_core::{AssetRef, Diagnostic, DomainId};

use editor_core::action::{Action, Handled};
use editor_core::domain::DomainState;
use editor_core::dynval::Payload;
use editor_core::state::editor::Editor;
use editor_core::tab::{Tab, TextDoc};

use crate::backend::bake::compute_schema_hash_with_enums;
use crate::backend::codegen::{collection_rust_rel_path, dto_rust_rel_path, enum_rust_rel_path};
use crate::backend::model::{
    COLLECTION_SUFFIX, CURVE_SUFFIX, CollectionAsset, DTO_SUFFIX, DtoAsset, ENUM_SUFFIX, EnumAsset, collection_ref,
    curve_ref, dto_ref, enum_ref,
};
use crate::backend::ron_io::{
    load_collection, load_dto, load_enum, save_collection, save_curve, save_dto, save_enum,
    set_asset_uuid_with_schemas as backend_set_asset_uuid,
};
use crate::backend::validate::DataAssets;
use crate::backend::{DATA, DataBackend};

use super::collection_doc::CollectionDoc;
use super::dto_doc::DtoDoc;
use super::enum_doc::EnumDoc;
use super::{DataCurve, DataCurveDoc};

pub const DTO_KIND: &str = "data.dto";
pub const ENUM_KIND: &str = "data.enum";
pub const COLLECTION_KIND: &str = "data.collection";
pub const CURVE_KIND: &str = "data.curve";

/// Domain-level steps dispatched via `Action::Domain(DATA, Payload::new(...))` or `DocCtx::defer`.
#[derive(Debug, Clone, PartialEq)]
pub enum DataStep {
    /// Opens (or activates) the read-only generated Rust preview tab for `AssetRef`.
    OpenRustPreview(AssetRef),
    /// Opens (or activates) the `.dto.ron` schema tab with the given UUID.
    OpenDtoSchema(AssetUuid),
    /// Opens (or activates) the `.enum.ron` schema tab with the given UUID.
    OpenEnumSchema(AssetUuid),
    /// Opens (or activates) `collection` and selects `row` in [`CollectionDoc`].
    OpenCollectionRow { collection: AssetRef, row: AssetUuid },
    /// Copies text to the system clipboard via `Editor::copy_text`.
    CopyText(String),
    /// Updates the in-memory `DtoAsset` in `DataState`, refreshes diagnostics and `current_schema_hash`,
    /// and updates any open `CollectionDoc` bound to this DTO.
    SyncDtoInMemory { path: String, dto: DtoAsset },
    /// Updates the in-memory `EnumAsset` in `DataState`, refreshes diagnostics and `current_schema_hash`,
    /// and updates any open `DtoDoc`, `EnumDoc`, or `CollectionDoc` referencing this enum.
    SyncEnumInMemory { path: String, en: EnumAsset },
    /// Updates the in-memory `CollectionAsset` in `DataState` and refreshes diagnostics.
    SyncCollectionInMemory { path: String, col: CollectionAsset },
}

impl From<DataStep> for Action {
    fn from(step: DataStep) -> Self {
        Action::Domain(DATA, Payload::new(step))
    }
}

/// Editor state for the Data domain (`quillshade/data/`).
#[derive(Default)]
pub struct DataState {
    pub assets: DataAssets,
    pub diagnostics: Vec<Diagnostic>,
    pub startup_schema_hash: Option<u64>,
    pub current_schema_hash: u64,
    pub project_gen: Option<u64>,
}

impl DataState {
    /// Returns `true` when the current DTO/Enum schemas differ from the startup schema hash (`rebuild game required`).
    pub fn schema_changed(&self) -> bool {
        self.startup_schema_hash.is_some_and(|startup| startup != self.current_schema_hash)
    }

    /// Finds a loaded DTO by its `AssetUuid`, returning `(rel_path, &DtoAsset)`.
    pub fn dto_by_id(&self, id: AssetUuid) -> Option<(&str, &DtoAsset)> {
        if id.is_nil() {
            return None;
        }
        self.assets.dtos.iter().find(|(_, d)| d.id == id).map(|(p, d)| (p.as_str(), d))
    }

    /// Sorted `(AssetUuid, rel_path)` list of all loaded DTOs for schema dropdowns.
    pub fn dto_options(&self) -> Vec<(AssetUuid, String)> {
        self.assets.dtos.iter().filter(|(_, d)| !d.id.is_nil()).map(|(p, d)| (d.id, p.clone())).collect()
    }

    /// Finds a loaded Enum by its `AssetUuid`, returning `(rel_path, &EnumAsset)`.
    pub fn enum_by_id(&self, id: AssetUuid) -> Option<(&str, &EnumAsset)> {
        if id.is_nil() {
            return None;
        }
        self.assets.enums.iter().find(|(_, e)| e.id == id).map(|(p, e)| (p.as_str(), e))
    }

    /// Sorted `(AssetUuid, rel_path)` list of all loaded Enums for schema dropdowns.
    pub fn enum_options(&self) -> Vec<(AssetUuid, String)> {
        self.assets.enums.iter().filter(|(_, e)| !e.id.is_nil()).map(|(p, e)| (e.id, p.clone())).collect()
    }

    /// Recomputes `current_schema_hash` and validates all in-memory Data assets against `index`.
    pub fn refresh_diagnostics(&mut self, index: &project_core::AssetIndex) {
        self.refresh_diagnostics_with_project(None, index);
    }

    /// Recomputes `current_schema_hash` and validates all in-memory Data assets against `project` and `index`
    /// (checking `GameAsset(kind)` paths under `<project.root>/assets/` when `project` is `Some`).
    pub fn refresh_diagnostics_with_project(
        &mut self,
        project: Option<&project_core::Project>,
        index: &project_core::AssetIndex,
    ) {
        self.current_schema_hash = compute_schema_hash_with_enums(&self.assets.dtos, &self.assets.enums);
        if self.startup_schema_hash.is_none() {
            self.startup_schema_hash = Some(self.current_schema_hash);
        }
        self.diagnostics = self.assets.validate_with_project(project, index).diagnostics;
    }
}

impl DomainState for DataState {
    fn id(&self) -> DomainId {
        DATA
    }

    fn rescan(editor: &mut Editor) {
        let Some(project) = editor.project.clone() else {
            if let Some(st) = editor.domains.get_mut::<DataState>() {
                *st = DataState::default();
            }
            return;
        };
        let dom_index = DataBackend.index(&project);
        editor.asset_index.replace_domain(dom_index);

        let assets = DataAssets::scan(&project);
        let open_dtos: Vec<(String, DtoAsset)> =
            editor.tabs.iter().filter_map(|t| t.get::<DtoDoc>().map(|d| (d.path.clone(), d.dto.clone()))).collect();
        let open_enums: Vec<(String, EnumAsset)> =
            editor.tabs.iter().filter_map(|t| t.get::<EnumDoc>().map(|e| (e.path.clone(), e.en.clone()))).collect();
        let open_cols: Vec<(String, CollectionAsset)> = editor
            .tabs
            .iter()
            .filter_map(|t| t.get::<CollectionDoc>().map(|c| (c.path.clone(), c.col.clone())))
            .collect();
        let open_curves: Vec<(String, curve_kit::Curve)> = editor
            .tabs
            .iter()
            .filter_map(|t| t.get::<DataCurveDoc>().map(|c| (c.aref.path.clone(), c.curve.clone())))
            .collect();

        let index_clone = editor.asset_index.clone();
        if let Some(st) = editor.domains.get_mut::<DataState>() {
            st.assets = assets;
            for (p, dto) in open_dtos {
                if st.assets.dtos.contains_key(&p) {
                    st.assets.dtos.insert(p, dto);
                }
            }
            for (p, en) in open_enums {
                if st.assets.enums.contains_key(&p) {
                    st.assets.enums.insert(p, en);
                }
            }
            for (p, col) in open_cols {
                if st.assets.collections.contains_key(&p) {
                    st.assets.collections.insert(p, col);
                }
            }
            for (p, curve) in open_curves {
                if st.assets.curves.contains_key(&p) {
                    st.assets.curves.insert(p, curve);
                }
            }
            st.refresh_diagnostics_with_project(Some(&project), &index_clone);
        }

        // Refresh cached_dtos / cached_enums / cached_dto on any open DtoDoc, EnumDoc, and CollectionDoc tabs.
        let dto_map = editor.domains.get::<DataState>().map(|st| st.assets.dtos.clone()).unwrap_or_default();
        let enum_map = editor.domains.get::<DataState>().map(|st| st.assets.enums.clone()).unwrap_or_default();
        for tab in &mut editor.tabs {
            if let Some(dto_doc) = tab.get_mut::<DtoDoc>() {
                dto_doc.cached_dtos = dto_map.clone();
                dto_doc.cached_enums = enum_map.clone();
                dto_doc.relayout();
            }
            if let Some(enum_doc) = tab.get_mut::<EnumDoc>() {
                enum_doc.cached_dtos = dto_map.clone();
                enum_doc.cached_enums = enum_map.clone();
                enum_doc.relayout();
            }
            if let Some(col_doc) = tab.get_mut::<CollectionDoc>() {
                col_doc.cached_dtos = dto_map.clone();
                col_doc.cached_enums = enum_map.clone();
                col_doc.cached_dto = dto_map.values().find(|d| !d.id.is_nil() && d.id == col_doc.col.dto).cloned();
                col_doc.relayout();
            }
        }
        editor.ui.browser += 1;
        editor.ui.log += 1;
    }

    fn after_apply(editor: &mut Editor) {
        let open_curves: Vec<(String, curve_kit::Curve)> = editor
            .tabs
            .iter()
            .filter_map(|t| t.get::<DataCurveDoc>().map(|c| (c.aref.path.clone(), c.curve.clone())))
            .collect();
        if let Some(st) = editor.domains.get_mut::<DataState>() {
            for (p, curve) in open_curves {
                if st.assets.curves.contains_key(&p) {
                    st.assets.curves.insert(p, curve);
                }
            }
            let dom_index = st.assets.index_part();
            editor.asset_index.replace_domain(dom_index);
        }
        let project = editor.project.clone();
        let index_clone = editor.asset_index.clone();
        if let Some(st) = editor.domains.get_mut::<DataState>() {
            st.refresh_diagnostics_with_project(project.as_ref(), &index_clone);
        }
        editor.ui.log += 1;
    }

    fn after_save(editor: &mut Editor, tabs: &[usize]) {
        if tabs.is_empty() {
            return;
        }
        Self::rescan(editor);
        Self::generate(editor);
        for tab in &mut editor.tabs {
            if let Some(td) = tab.get_mut::<TextDoc>()
                && td.source.domain == DATA
            {
                td.reload();
            }
        }
    }

    fn diagnostics(&self) -> Vec<Diagnostic> {
        self.diagnostics.clone()
    }

    fn generate(editor: &mut Editor) {
        let Some(project) = editor.project.clone() else { return };
        if !project.domain_enabled(DATA) {
            return;
        }
        let dom_index = DataBackend.index(&project);
        editor.asset_index.replace_domain(dom_index);
        let plan = DataBackend.plan(&project, &editor.asset_index, false);
        let contribs = editor.domain_list.crate_contributions();
        if let Err(e) = regen::commit(&project, &[plan.output], contribs, false) {
            editor.io_error("Data codegen failed", e.to_string());
        }
        let index_clone = editor.asset_index.clone();
        if let Some(st) = editor.domains.get_mut::<DataState>() {
            st.refresh_diagnostics_with_project(Some(&project), &index_clone);
        }
        editor.ui.log += 1;
    }

    fn regenerate_affected(editor: &mut Editor, affected: &[AssetRef]) {
        if affected.iter().any(|r| r.domain == DATA) {
            Self::rescan(editor);
            Self::generate(editor);
        }
    }

    fn delete_message(&self, editor: &Editor, r: &AssetRef) -> Option<String> {
        if r.suffix == DTO_SUFFIX
            && let Some(dto) = self.assets.dtos.get(&r.path)
            && !dto.id.is_nil()
        {
            let names: Vec<String> = editor.asset_index.refs_to(dto.id).map(|u| u.source_rel()).collect();
            if !names.is_empty() {
                return Some(format!("DTO '{}' is referenced by: {}. Delete anyway?", r.path, names.join(", ")));
            }
        }
        if r.suffix == ENUM_SUFFIX
            && let Some(en) = self.assets.enums.get(&r.path)
            && !en.id.is_nil()
        {
            let names: Vec<String> = editor.asset_index.refs_to(en.id).map(|u| u.source_rel()).collect();
            if !names.is_empty() {
                return Some(format!("Enum '{}' is referenced by: {}. Delete anyway?", r.path, names.join(", ")));
            }
        }
        if r.suffix == CURVE_SUFFIX
            && let Some(curve) = self.assets.curves.get(&r.path)
            && !curve.id.is_nil()
        {
            let names: Vec<String> = editor.asset_index.refs_to(curve.id).map(|u| u.source_rel()).collect();
            if !names.is_empty() {
                return Some(format!("Curve '{}' is referenced by: {}. Delete anyway?", r.path, names.join(", ")));
            }
        }
        None
    }

    fn renamed(editor: &mut Editor, from: &AssetRef, to: &AssetRef) {
        if from.domain != DATA && to.domain != DATA {
            return;
        }
        Self::rescan(editor);
        Self::generate(editor);
        if let Some(project) = editor.project.clone() {
            for tab in &mut editor.tabs {
                if let Some(td) = tab.get_mut::<TextDoc>()
                    && &td.source == from
                {
                    let new_path = rust_preview_path(&project, to);
                    td.retarget(&project, to.clone(), new_path);
                }
            }
        }
    }

    fn deleted(editor: &mut Editor, r: &AssetRef) {
        if r.domain != DATA {
            return;
        }
        Self::rescan(editor);
        Self::generate(editor);
    }

    fn set_asset_uuid(editor: &mut Editor, r: &AssetRef, id: AssetUuid) -> Result<(), String> {
        let Some(project) = editor.project.clone() else {
            return Err("No open project".into());
        };
        let dtos = editor.domains.get::<DataState>().map(|st| st.assets.dtos.clone()).unwrap_or_default();
        let enums = editor.domains.get::<DataState>().map(|st| st.assets.enums.clone()).unwrap_or_default();
        backend_set_asset_uuid(&project, r, id, &editor.asset_index, &dtos, &enums)?;
        let file = project.root.join(r.source_rel());
        for tab in &mut editor.tabs {
            if let Some(doc) = tab.get_mut::<DtoDoc>()
                && &doc.aref == r
            {
                if let Ok(reloaded) = load_dto(&file) {
                    doc.dto = reloaded.clone();
                    doc.saved = reloaded;
                } else {
                    doc.dto.id = id;
                    doc.saved.id = id;
                }
                doc.bump(true);
            }
            if let Some(doc) = tab.get_mut::<EnumDoc>()
                && &doc.aref == r
            {
                if let Ok(reloaded) = load_enum(&file) {
                    doc.en = reloaded.clone();
                    doc.saved = reloaded;
                } else {
                    doc.en.id = id;
                    doc.saved.id = id;
                }
                doc.bump(true);
            }
            if let Some(doc) = tab.get_mut::<CollectionDoc>()
                && &doc.aref == r
            {
                if let Ok(reloaded) = load_collection(&file) {
                    doc.col = reloaded.clone();
                    doc.saved = reloaded;
                } else {
                    doc.col.id = id;
                    doc.saved.id = id;
                }
                doc.bump(true);
            }
            if let Some(doc) = tab.get_mut::<DataCurveDoc>()
                && &doc.aref == r
            {
                doc.curve.id = id;
                doc.saved.id = id;
                doc.revision += 1;
            }
        }
        Self::rescan(editor);
        Self::generate(editor);
        Ok(())
    }

    fn focus_diagnostic(editor: &mut Editor, diag: &Diagnostic) {
        let Some(asset_str) = diag.asset.as_deref() else { return };
        let Some(aref) = project_core::asset_ref::parse(
            asset_str,
            DATA,
            &[DTO_SUFFIX, ENUM_SUFFIX, COLLECTION_SUFFIX, CURVE_SUFFIX],
        ) else {
            return;
        };
        let _ = open_asset(editor, &aref);
        let Some(loc) = diag.location.as_deref() else { return };
        if let Some(dto_doc) = editor.active_doc_mut::<DtoDoc>() {
            if let Some(hex) = loc.strip_prefix("field:")
                && let Some(fid) = AssetUuid::parse(hex)
            {
                dto_doc.selected_field = Some(fid);
                dto_doc.relayout();
                editor.ui.props += 1;
            }
        } else if let Some(enum_doc) = editor.active_doc_mut::<EnumDoc>() {
            if let Some(rest) = loc.strip_prefix("variant:") {
                let mut parts = rest.split(':');
                if let Some(v_hex) = parts.next()
                    && let Some(vid) = AssetUuid::parse(v_hex)
                {
                    enum_doc.selected_variant = Some(vid);
                    if parts.next() == Some("field")
                        && let Some(f_hex) = parts.next()
                        && let Some(fid) = AssetUuid::parse(f_hex)
                    {
                        enum_doc.selected_field = Some(fid);
                    }
                    enum_doc.relayout();
                    editor.ui.props += 1;
                }
            } else if let Some(hex) = loc.strip_prefix("field:")
                && let Some(fid) = AssetUuid::parse(hex)
            {
                if let Some(v) = enum_doc.en.variants.iter().find(|v| v.fields.iter().any(|f| f.id == fid)) {
                    enum_doc.selected_variant = Some(v.id);
                }
                enum_doc.selected_field = Some(fid);
                enum_doc.relayout();
                editor.ui.props += 1;
            }
        } else if let Some(col_doc) = editor.active_doc_mut::<CollectionDoc>()
            && let Some(rest) = loc.strip_prefix("row:")
        {
            let row_hex = rest.split(':').next().unwrap_or(rest);
            if let Some(rid) = AssetUuid::parse(row_hex) {
                col_doc.selected_row = Some(rid);
                col_doc.filter = rid.to_string();
                col_doc.relayout();
                editor.ui.props += 1;
            }
        }
    }
}

pub fn rust_preview_path(project: &project_core::Project, source: &AssetRef) -> std::path::PathBuf {
    let mod_dir = project.module_dir(DATA);
    if source.suffix == DTO_SUFFIX {
        mod_dir.join(dto_rust_rel_path(&source.path))
    } else if source.suffix == ENUM_SUFFIX {
        mod_dir.join(enum_rust_rel_path(&source.path))
    } else if source.suffix == COLLECTION_SUFFIX {
        mod_dir.join(collection_rust_rel_path(&source.path))
    } else {
        mod_dir.join("mod.rs")
    }
}

pub fn open_rust_preview(editor: &mut Editor, source: AssetRef) {
    let Some(project) = editor.project.clone() else { return };
    if let Some(idx) =
        editor.tabs.iter().position(|t| t.get::<TextDoc>().is_some_and(|td| td.source == source && td.lang == "rust"))
    {
        editor.activate(idx);
        if let Some(td) = editor.tabs[idx].get_mut::<TextDoc>() {
            td.reload();
        }
        return;
    }
    let target_path = rust_preview_path(&project, &source);
    if !target_path.exists() {
        DataState::generate(editor);
    }
    let doc = TextDoc::open(&project, source, "rust", target_path);
    editor.push_tab(Tab::new(DATA, doc));
}

pub fn handle_step(editor: &mut Editor, payload: &Payload) -> Handled {
    let Some(step) = payload.downcast::<DataStep>() else {
        return Handled::No;
    };
    match step {
        DataStep::OpenRustPreview(source) => {
            open_rust_preview(editor, source.clone());
            Handled::Yes
        }
        DataStep::OpenDtoSchema(dto_id) => {
            let dto_path =
                editor.domains.get::<DataState>().and_then(|st| st.dto_by_id(*dto_id).map(|(p, _)| p.to_string()));
            if let Some(path) = dto_path {
                let _ = open_asset(editor, &dto_ref(&path));
            }
            Handled::Yes
        }
        DataStep::OpenEnumSchema(enum_id) => {
            let enum_path =
                editor.domains.get::<DataState>().and_then(|st| st.enum_by_id(*enum_id).map(|(p, _)| p.to_string()));
            if let Some(path) = enum_path {
                let _ = open_asset(editor, &enum_ref(&path));
            }
            Handled::Yes
        }
        DataStep::OpenCollectionRow { collection, row } => {
            let _ = open_asset(editor, collection);
            if let Some(col_doc) = editor.active_doc_mut::<CollectionDoc>() {
                col_doc.selected_row = Some(*row);
                col_doc.bump(true);
            }
            editor.ui.props += 1;
            Handled::Yes
        }
        DataStep::CopyText(text) => {
            editor.copy_text = Some(text.clone());
            Handled::Yes
        }
        DataStep::SyncDtoInMemory { path, dto } => {
            if let Some(st) = editor.domains.get_mut::<DataState>() {
                st.assets.dtos.insert(path.clone(), dto.clone());
                let dom_index = st.assets.index_part();
                editor.asset_index.replace_domain(dom_index);
            }
            let project = editor.project.clone();
            let index_clone = editor.asset_index.clone();
            if let Some(st) = editor.domains.get_mut::<DataState>() {
                st.refresh_diagnostics_with_project(project.as_ref(), &index_clone);
            }
            let dto_map = editor.domains.get::<DataState>().map(|st| st.assets.dtos.clone()).unwrap_or_default();
            for tab in &mut editor.tabs {
                if let Some(dto_doc) = tab.get_mut::<DtoDoc>() {
                    dto_doc.cached_dtos = dto_map.clone();
                    if dto_doc.path != *path {
                        dto_doc.relayout();
                    }
                }
                if let Some(enum_doc) = tab.get_mut::<EnumDoc>() {
                    enum_doc.cached_dtos = dto_map.clone();
                    enum_doc.relayout();
                }
                if let Some(col_doc) = tab.get_mut::<CollectionDoc>() {
                    col_doc.cached_dtos = dto_map.clone();
                    if !dto.id.is_nil() && col_doc.col.dto == dto.id {
                        col_doc.cached_dto = Some(dto.clone());
                    }
                    col_doc.relayout();
                }
            }
            editor.ui.log += 1;
            editor.ui.props += 1;
            Handled::Yes
        }
        DataStep::SyncEnumInMemory { path, en } => {
            if let Some(st) = editor.domains.get_mut::<DataState>() {
                st.assets.enums.insert(path.clone(), en.clone());
                let dom_index = st.assets.index_part();
                editor.asset_index.replace_domain(dom_index);
            }
            let project = editor.project.clone();
            let index_clone = editor.asset_index.clone();
            if let Some(st) = editor.domains.get_mut::<DataState>() {
                st.refresh_diagnostics_with_project(project.as_ref(), &index_clone);
            }
            let enum_map = editor.domains.get::<DataState>().map(|st| st.assets.enums.clone()).unwrap_or_default();
            for tab in &mut editor.tabs {
                if let Some(dto_doc) = tab.get_mut::<DtoDoc>() {
                    dto_doc.cached_enums = enum_map.clone();
                    dto_doc.relayout();
                }
                if let Some(enum_doc) = tab.get_mut::<EnumDoc>() {
                    enum_doc.cached_enums = enum_map.clone();
                    if enum_doc.path != *path {
                        enum_doc.relayout();
                    }
                }
                if let Some(col_doc) = tab.get_mut::<CollectionDoc>() {
                    col_doc.cached_enums = enum_map.clone();
                    col_doc.relayout();
                }
            }
            editor.ui.log += 1;
            editor.ui.props += 1;
            Handled::Yes
        }
        DataStep::SyncCollectionInMemory { path, col } => {
            if let Some(st) = editor.domains.get_mut::<DataState>() {
                st.assets.collections.insert(path.clone(), col.clone());
                let dom_index = st.assets.index_part();
                editor.asset_index.replace_domain(dom_index);
            }
            let project = editor.project.clone();
            let index_clone = editor.asset_index.clone();
            if let Some(st) = editor.domains.get_mut::<DataState>() {
                st.refresh_diagnostics_with_project(project.as_ref(), &index_clone);
            }
            for tab in &mut editor.tabs {
                if let Some(dto_doc) = tab.get_mut::<DtoDoc>() {
                    dto_doc.relayout();
                }
                if let Some(enum_doc) = tab.get_mut::<EnumDoc>() {
                    enum_doc.relayout();
                }
                if let Some(col_doc) = tab.get_mut::<CollectionDoc>()
                    && col_doc.path != *path
                {
                    col_doc.relayout();
                }
            }
            editor.ui.log += 1;
            Handled::Yes
        }
    }
}

pub fn open_asset(editor: &mut Editor, r: &AssetRef) -> Handled {
    if r.domain != DATA {
        return Handled::No;
    }
    if let Some(idx) = editor.tabs.iter().position(|t| t.doc().asset() == Some(r)) {
        editor.activate(idx);
        return Handled::Yes;
    }
    let Some(project) = editor.project.clone() else {
        return Handled::No;
    };
    let file = project.root.join(r.source_rel());
    let dto_map = editor.domains.get::<DataState>().map(|st| st.assets.dtos.clone()).unwrap_or_default();
    let enum_map = editor.domains.get::<DataState>().map(|st| st.assets.enums.clone()).unwrap_or_default();
    if r.suffix == DTO_SUFFIX {
        match load_dto(&file) {
            Ok(dto) => {
                editor.push_tab(Tab::new(DATA, DtoDoc::with_schemas(r.path.clone(), dto, dto_map, enum_map)));
                Handled::Yes
            }
            Err(e) => {
                editor.io_error(&format!("Failed to open {}", r.path), e.to_string());
                Handled::Yes
            }
        }
    } else if r.suffix == ENUM_SUFFIX {
        match load_enum(&file) {
            Ok(en) => {
                editor.push_tab(Tab::new(DATA, EnumDoc::with_schemas(r.path.clone(), en, dto_map, enum_map)));
                Handled::Yes
            }
            Err(e) => {
                editor.io_error(&format!("Failed to open {}", r.path), e.to_string());
                Handled::Yes
            }
        }
    } else if r.suffix == COLLECTION_SUFFIX {
        match load_collection(&file) {
            Ok(col) => {
                let cached_dto =
                    editor.domains.get::<DataState>().and_then(|st| st.dto_by_id(col.dto).map(|(_, d)| d.clone()));
                editor.push_tab(Tab::new(
                    DATA,
                    CollectionDoc::new(r.path.clone(), col, cached_dto).with_schemas(dto_map, enum_map),
                ));
                Handled::Yes
            }
            Err(e) => {
                editor.io_error(&format!("Failed to open {}", r.path), e.to_string());
                Handled::Yes
            }
        }
    } else if r.suffix == CURVE_SUFFIX {
        if let Some(tab) = curve_kit::editor::open::<DataCurve>(editor, r) {
            editor.push_tab(tab);
        }
        Handled::Yes
    } else {
        Handled::No
    }
}

pub fn create_asset(editor: &mut Editor, kind: &str, path: &str) -> Result<AssetRef, String> {
    check_path(path).map_err(|e| e.to_string())?;
    let Some(project) = editor.project.clone() else {
        return Err("No open project".into());
    };
    if kind == DTO_KIND {
        let aref = dto_ref(path);
        let file = project.root.join(aref.source_rel());
        if file.exists() {
            return Err(format!("DTO '{path}' already exists"));
        }
        let mut dto = DtoAsset::new();
        save_dto(&file, &mut dto).map_err(|e| e.to_string())?;
        DataState::rescan(editor);
        DataState::generate(editor);
        let _ = open_asset(editor, &aref);
        Ok(aref)
    } else if kind == ENUM_KIND {
        let aref = enum_ref(path);
        let file = project.root.join(aref.source_rel());
        if file.exists() {
            return Err(format!("Enum '{path}' already exists"));
        }
        let mut en = EnumAsset::new();
        save_enum(&file, &mut en).map_err(|e| e.to_string())?;
        DataState::rescan(editor);
        DataState::generate(editor);
        let _ = open_asset(editor, &aref);
        Ok(aref)
    } else if kind == COLLECTION_KIND {
        let aref = collection_ref(path);
        let file = project.root.join(aref.source_rel());
        if file.exists() {
            return Err(format!("Collection '{path}' already exists"));
        }
        let default_dto = editor
            .domains
            .get::<DataState>()
            .and_then(|st| st.assets.dtos.iter().find(|(_, d)| !d.id.is_nil()).map(|(p, d)| (p.clone(), d.clone())));
        let dto_id = default_dto.as_ref().map(|(_, d)| d.id).unwrap_or(AssetUuid::NIL);
        let mut col = CollectionAsset::new(dto_id);
        save_collection(
            &file,
            &mut col,
            default_dto.as_ref().map(|(p, _)| p.as_str()),
            default_dto.as_ref().map(|(_, d)| d),
        )
        .map_err(|e| e.to_string())?;
        DataState::rescan(editor);
        DataState::generate(editor);
        let _ = open_asset(editor, &aref);
        Ok(aref)
    } else if kind == CURVE_KIND {
        let aref = curve_ref(path);
        let file = project.root.join(aref.source_rel());
        if file.exists() {
            return Err(format!("Curve '{path}' already exists"));
        }
        let stem = path.rsplit('/').next().unwrap_or(path);
        let mut curve = curve_kit::Curve::ramp(stem);
        save_curve(&file, &mut curve).map_err(|e| e.to_string())?;
        DataState::rescan(editor);
        DataState::generate(editor);
        let _ = open_asset(editor, &aref);
        Ok(aref)
    } else {
        Err(format!("Unknown Data asset kind '{kind}'"))
    }
}

pub fn reopen_tab(editor: &mut Editor, key: &str) -> bool {
    if let Some(base) = key.strip_suffix("#rust") {
        let Some(source) =
            project_core::asset_ref::parse(base, DATA, &[DTO_SUFFIX, ENUM_SUFFIX, COLLECTION_SUFFIX, CURVE_SUFFIX])
        else {
            return false;
        };
        open_rust_preview(editor, source);
        return true;
    }
    if let Some(aref) =
        project_core::asset_ref::parse(key, DATA, &[DTO_SUFFIX, ENUM_SUFFIX, COLLECTION_SUFFIX, CURVE_SUFFIX])
    {
        return open_asset(editor, &aref) == Handled::Yes;
    }
    false
}
