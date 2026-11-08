//! Data domain editor integration (`DataDomain`): registers `.dto.ron` (`DtoEditor`), `.enum.ron` (`EnumEditor`),
//! and `.collection.ron` (`CollectionEditor`), `DataState`, domain icons (`DATA_ICONS`), and generated Rust preview tabs.

pub mod collection_doc;
pub mod collection_view;
pub mod dto_doc;
pub mod dto_view;
pub mod enum_doc;
pub mod enum_view;
pub mod quick_start;
pub mod state;
pub mod widgets;

use std::io;
use std::sync::Arc;

use project_core::regen::DomainBackend;
use project_core::{AssetRef, DomainId, Project};

use editor_core::action::Action;
use editor_core::domain::{
    AssetKindDef, AssetTemplate, DomainInfo, DomainList, DomainRegistrar, DomainState, EditorDomain,
};
use editor_core::menu::MenuEntry;
use editor_core::state::editor::Editor;
use editor_core::tab::{Tab, TextDoc, ViewKindId};
use editor_core::ui::icons::{IconId, IconRef};

pub use crate::backend::model::{
    COLLECTION_SUFFIX, COLLECTION_SUFFIX as COLLECTION_REF_SUFFIX, CURVE_SUFFIX, CURVE_SUFFIX as CURVE_REF_SUFFIX,
    CollectionAsset, DTO_SUFFIX, DTO_SUFFIX as DTO_REF_SUFFIX, DtoAsset, ENUM_SUFFIX, ENUM_SUFFIX as ENUM_REF_SUFFIX,
    EnumAsset, EnumVariant, collection_ref, curve_ref, dto_ref, enum_ref,
};
use crate::backend::ron_io::{load_collection, load_dto, load_enum, save_collection, save_dto, save_enum};
pub use crate::backend::{DATA, DataBackend, DataMigrator};

pub use collection_doc::{COLLECTION_VIEW, CollectionAction, CollectionDoc, CollectionProp};
pub use collection_view::{
    CollectionDetailBody, CollectionDtoSummaryText, CollectionEditor, CollectionListBody, CollectionResetIcon,
    CollectionSidebar, CollectionTopbar, collection_view,
};
pub use dto_doc::{ConstraintField, DTO_VIEW, DtoAction, DtoDoc, DtoProp};
pub use dto_view::{
    DtoDefaultResetIcon, DtoDefaultSummaryText, DtoEditor, DtoSidebar, DtoTopbar, DtoViewBody, dto_view,
};
pub use enum_doc::{ENUM_VIEW, EnumAction, EnumDoc, EnumProp};
pub use enum_view::{
    EnumDefaultResetIcon, EnumDefaultSummaryText, EnumEditor, EnumSidebar, EnumTopbar, EnumViewBody, enum_view,
};
pub use state::{
    COLLECTION_KIND, CURVE_KIND, DTO_KIND, DataState, DataStep, ENUM_KIND, create_asset, handle_step, open_asset,
    open_rust_preview, reopen_tab, rust_preview_path,
};
pub use widgets::{
    AssetPickerCatalog, DataColorSwatch, DataCurveMiniPreview, DataGradientMiniPreview, DataJumpAssetButton,
    SoftRefRowEntry, apply_set_component, apply_set_value, curve_mini_preview, data_value_editor,
    data_value_editor_with_catalog, data_value_to_prop, enum_variant_select, euler_deg_to_quat, gradient_mini_preview,
    jump_asset_button, jump_row_button, quat_to_euler_deg, switch_enum_variant_with_drafts,
};

/// The Data domain's curve editor view kind (`quillshade/data/*.curve.ron`).
pub const DATA_CURVE_VIEW: ViewKindId = ViewKindId("data.curve");

/// The Data domain's [`curve_kit::editor::CurveDomain`] implementation.
pub struct DataCurve;

impl curve_kit::editor::CurveDomain for DataCurve {
    const DOMAIN: DomainId = DATA;
    const VIEW: ViewKindId = DATA_CURVE_VIEW;
}

pub type DataCurveDoc = curve_kit::editor::CurveDoc<DataCurve>;
pub type DataCurveEdit = curve_kit::editor::CurveEdit<DataCurve>;
pub type DataCurveEditor = curve_kit::editor::CurveEditor<DataCurve>;

/// The Data domain's icon in the mainbar.
pub const DATA_ICON: IconRef = IconRef::new(&icon_rs::CUSTOM_TABLE);
/// The `.dto.ron` schema asset icon.
pub const DTO_ICON: IconRef = IconRef::new(&icon_rs::PROPERTIES);
/// The `.enum.ron` schema asset icon.
pub const ENUM_ICON: IconRef = IconRef::new(&icon_rs::LAYERS);
/// The `.collection.ron` asset icon.
pub const COLLECTION_ICON: IconRef = IconRef::new(&icon_rs::LIST_DETAILS);

/// All icons declared by `data_domain`, verified by `data_icons_rasterize`.
pub const DATA_ICONS: &[IconRef] = &[DATA_ICON, DTO_ICON, ENUM_ICON, COLLECTION_ICON];

/// The Data domain (`quillshade/data/`).
pub struct DataDomain;

impl EditorDomain for DataDomain {
    fn id(&self) -> DomainId {
        DATA
    }

    fn info(&self) -> DomainInfo {
        DomainInfo {
            title: "Data",
            icon: DATA_ICON,
            summary: "Data - strongly-typed DTO schemas (.dto.ron), enums & tagged unions (.enum.ron), and hot-reloadable collections (.collection.ron) in quillshade/data/.",
        }
    }

    fn backend(&self) -> Option<Box<dyn DomainBackend>> {
        Some(Box::new(DataBackend))
    }

    fn migrator(&self) -> Option<Box<dyn project_migrate::DomainMigrator>> {
        Some(Box::new(DataMigrator))
    }

    fn register(&self, r: &mut DomainRegistrar) {
        r.state(|| Box::new(DataState::default()))
            .quick_start(quick_start::QUICK_START)
            .steps(state::handle_step)
            .parse_tab(parse_rust_tab)
            .tab_exists(rust_tab_exists)
            .asset_kind(AssetKindDef {
                suffix: DTO_SUFFIX,
                title: "DTO Schema",
                icon: DTO_ICON,
                templates: vec![AssetTemplate {
                    label: "DTO Schema",
                    base_name: "new_dto",
                    icon: DTO_ICON,
                    create: create_dto_file,
                }],
                open: open_dto_tab,
                row_menu: data_row_menu,
            })
            .asset_kind(AssetKindDef {
                suffix: ENUM_SUFFIX,
                title: "Enum / Union",
                icon: ENUM_ICON,
                templates: vec![AssetTemplate {
                    label: "Enum / Union",
                    base_name: "new_enum",
                    icon: ENUM_ICON,
                    create: create_enum_file,
                }],
                open: open_enum_tab,
                row_menu: data_row_menu,
            })
            .asset_kind(AssetKindDef {
                suffix: COLLECTION_SUFFIX,
                title: "Collection",
                icon: COLLECTION_ICON,
                templates: vec![AssetTemplate {
                    label: "Collection",
                    base_name: "new_collection",
                    icon: COLLECTION_ICON,
                    create: create_collection_file,
                }],
                open: open_collection_tab,
                row_menu: data_row_menu,
            })
            .editor_kind::<DtoEditor>()
            .doc_view(|| Box::new(dto_view()))
            .editor_kind::<EnumEditor>()
            .doc_view(|| Box::new(enum_view()))
            .editor_kind::<CollectionEditor>()
            .doc_view(|| Box::new(collection_view()))
            .bevy(setup_data_window);
        curve_kit::editor::register::<DataCurve>(r);
    }
}

fn setup_data_window(app: &mut bevy::prelude::App) {
    use bevy::prelude::*;
    if std::env::var_os("SHADER_EDITOR_SCREENSHOT").is_some() {
        app.add_systems(Update, data_debug_screenshot.after(editor_core::apply_actions));
    }
}

type DataScrollFilter = bevy::prelude::Or<(
    bevy::prelude::With<DtoViewBody>,
    bevy::prelude::With<EnumViewBody>,
    bevy::prelude::With<CollectionDetailBody>,
)>;

fn data_debug_screenshot(
    mut frame: bevy::prelude::Local<u32>,
    mut editor: bevy::prelude::ResMut<Editor>,
    mut scroll_q: bevy::prelude::Query<&mut bevy::ui::ScrollPosition, DataScrollFilter>,
) {
    *frame += 1;
    if *frame == 4
        && let Ok(open) = std::env::var("SHADER_EDITOR_OPEN")
    {
        let (base, want_rust) = match open.strip_suffix("#rust") {
            Some(stripped) => (stripped, true),
            None => (open.as_str(), false),
        };
        let rel = base.strip_prefix("quillshade/data/").unwrap_or(base);
        let full = format!("quillshade/data/{rel}");
        if let Some(r) = parse_source_ref(&full) {
            editor.apply(Action::OpenAsset(r.clone()));
            if want_rust {
                editor.apply(Action::from(DataStep::OpenRustPreview(r)));
            }
        }
    }
    if *frame == 12
        && std::env::var("SHADER_EDITOR_DEMO").as_deref() == Ok("data_dto_constraints")
        && let Some(dto_doc) = editor.active_doc::<DtoDoc>()
    {
        let target_id = dto_doc
            .dto
            .fields
            .iter()
            .find(|f| f.ty.is_numeric() && !f.settings.is_default())
            .or_else(|| dto_doc.dto.fields.iter().find(|f| !f.settings.is_default()))
            .or_else(|| dto_doc.dto.fields.first())
            .map(|f| f.id);
        if let Some(fid) = target_id {
            editor.apply(Action::from(DtoAction::ToggleConstraints(fid)));
        }
    }
    if *frame == 12
        && std::env::var("SHADER_EDITOR_DEMO").as_deref() == Ok("data_asset_refs")
        && let Some(dto_doc) = editor.active_doc::<DtoDoc>()
        && let Some(fid) = dto_doc.dto.fields.iter().find(|f| f.ty.is_asset_ref()).map(|f| f.id)
    {
        editor.apply(Action::from(DtoAction::SelectField(Some(fid))));
    }
    if *frame == 14 && std::env::var("SHADER_EDITOR_DEMO").as_deref() == Ok("data_asset_refs") {
        for mut sp in &mut scroll_q {
            sp.y = 260.0;
        }
    }
    if *frame == 12
        && std::env::var("SHADER_EDITOR_DEMO").as_deref() == Ok("data_soft_ref")
        && let Some(dto_doc) = editor.active_doc::<DtoDoc>()
        && let Some(fid) = dto_doc.dto.fields.iter().find(|f| f.ty.is_soft_ref()).map(|f| f.id)
    {
        editor.apply(Action::from(DtoAction::SelectField(Some(fid))));
    }
    if *frame == 14 && std::env::var("SHADER_EDITOR_DEMO").as_deref() == Ok("data_soft_ref") {
        for mut sp in &mut scroll_q {
            sp.y = 380.0;
        }
    }
    if *frame == 12
        && std::env::var("SHADER_EDITOR_DEMO").as_deref() == Ok("data_enum_editor")
        && let Some(enum_doc) = editor.active_doc::<EnumDoc>()
        && let Some(v) = enum_doc.en.variants.iter().find(|v| !v.fields.is_empty())
    {
        let vid = v.id;
        let fid = v.fields.first().map(|f| f.id);
        editor.apply(Action::from(EnumAction::SelectVariant(Some(vid))));
        if let Some(fid) = fid {
            editor.apply(Action::from(EnumAction::SelectField(Some(fid))));
        }
    }
    if *frame == 12
        && std::env::var("SHADER_EDITOR_DEMO").as_deref() == Ok("data_dto_containers")
        && let Some(dto_doc) = editor.active_doc::<DtoDoc>()
        && let Some(fid) = dto_doc.dto.fields.iter().find(|f| f.ty.is_container() || f.ty.is_enum()).map(|f| f.id)
    {
        editor.apply(Action::from(DtoAction::SelectField(Some(fid))));
    }
    if *frame == 14 && std::env::var("SHADER_EDITOR_DEMO").as_deref() == Ok("data_dto_containers") {
        for mut sp in &mut scroll_q {
            sp.y = 480.0;
        }
    }
    if *frame == 12
        && std::env::var("SHADER_EDITOR_DEMO").as_deref() == Ok("data_collection_containers")
        && let Some(col_doc) = editor.active_doc::<CollectionDoc>()
        && let Some(dto) = col_doc.cached_dto.clone()
    {
        let expand_ids: Vec<_> =
            dto.fields.iter().filter(|f| f.ty.is_container() || f.ty.is_enum()).map(|f| vec![f.id]).collect();
        for path in expand_ids {
            editor.apply(Action::from(CollectionAction::ToggleDtoField { path }));
        }
    }
    if *frame == 15 && std::env::var("SHADER_EDITOR_DEMO").as_deref() == Ok("data_collection_containers") {
        for mut sp in &mut scroll_q {
            sp.y = 640.0;
        }
    }
    if *frame == 12 && std::env::var("SHADER_EDITOR_DEMO").as_deref() == Ok("data_nested_dto") {
        if let Some(dto_doc) = editor.active_doc::<DtoDoc>() {
            let paths = collect_nested_dto_paths(&dto_doc.dto, &dto_doc.cached_dtos);
            for path in paths {
                editor.apply(Action::from(DtoAction::ToggleDtoDefault(path)));
            }
        } else if let Some(col_doc) = editor.active_doc::<CollectionDoc>()
            && let Some(dto) = col_doc.cached_dto.clone()
        {
            let paths = collect_nested_dto_paths(&dto, &col_doc.cached_dtos);
            for path in paths {
                editor.apply(Action::from(CollectionAction::ToggleDtoField { path }));
            }
        }
    }
    if *frame == 12
        && std::env::var("SHADER_EDITOR_DEMO").as_deref() == Ok("data_duplicate_row_uuid")
        && let Some(col_doc) = editor.active_doc_mut::<CollectionDoc>()
        && col_doc.col.rows.len() >= 2
    {
        col_doc.col.rows[1].id = col_doc.col.rows[0].id;
        col_doc.bump(true);
        let path = col_doc.path.clone();
        let col = col_doc.col.clone();
        editor.apply(Action::from(DataStep::SyncCollectionInMemory { path, col }));
    }
    if *frame == 16
        && std::env::var("SHADER_EDITOR_DEMO").as_deref() == Ok("data_duplicate_row_uuid")
        && let Some(diag) =
            editor.domains.get::<DataState>().and_then(|st| st.diagnostics.iter().find(|d| d.code == "DE002").cloned())
    {
        editor.apply(Action::FocusDiagnostic(diag));
    }
}

fn collect_nested_dto_paths(
    dto: &DtoAsset,
    dtos: &std::collections::BTreeMap<String, DtoAsset>,
) -> Vec<Vec<project_core::asset_uuid::AssetUuid>> {
    fn walk(
        dto: &DtoAsset,
        dtos: &std::collections::BTreeMap<String, DtoAsset>,
        prefix: Vec<project_core::asset_uuid::AssetUuid>,
        visited: &mut std::collections::BTreeSet<project_core::asset_uuid::AssetUuid>,
        out: &mut Vec<Vec<project_core::asset_uuid::AssetUuid>>,
    ) {
        for f in &dto.fields {
            if let crate::backend::model::FieldType::Dto(target_id) = f.ty {
                let mut path = prefix.clone();
                path.push(f.id);
                out.push(path.clone());
                if visited.insert(target_id) {
                    if let Some((_, sub)) = dtos.iter().find(|(_, d)| d.id == target_id) {
                        walk(sub, dtos, path, visited, out);
                    }
                    visited.remove(&target_id);
                }
            }
        }
    }
    let mut visited = std::collections::BTreeSet::new();
    visited.insert(dto.id);
    let mut out = Vec::new();
    walk(dto, dtos, Vec::new(), &mut visited, &mut out);
    out
}

/// Helper returning a [`DomainList`] with `DataDomain` registered for headless tests.
pub fn data_only() -> Arc<DomainList> {
    Arc::new(DomainList::new(vec![Box::new(DataDomain)]))
}

/// Helper returning an [`Editor`] with `DataDomain` registered for headless tests.
pub fn test_editor() -> Editor {
    Editor::new(data_only())
}

fn create_dto_file(project: &Project, r: &AssetRef) -> io::Result<()> {
    project_core::ident::check_path(&r.path).map_err(io::Error::other)?;
    let file = project.root.join(r.source_rel());
    if file.exists() {
        return Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("`{}` already exists", r.source_rel())));
    }
    project.ensure_layout()?;
    let mut dto = DtoAsset::new();
    save_dto(&file, &mut dto)
}

fn create_enum_file(project: &Project, r: &AssetRef) -> io::Result<()> {
    project_core::ident::check_path(&r.path).map_err(io::Error::other)?;
    let file = project.root.join(r.source_rel());
    if file.exists() {
        return Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("`{}` already exists", r.source_rel())));
    }
    project.ensure_layout()?;
    let mut en = EnumAsset::new();
    save_enum(&file, &mut en)
}

fn create_collection_file(project: &Project, r: &AssetRef) -> io::Result<()> {
    project_core::ident::check_path(&r.path).map_err(io::Error::other)?;
    let file = project.root.join(r.source_rel());
    if file.exists() {
        return Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("`{}` already exists", r.source_rel())));
    }
    project.ensure_layout()?;
    let mut col = CollectionAsset::new(project_core::asset_uuid::AssetUuid::NIL);
    save_collection(&file, &mut col, None, None)
}

fn open_dto_tab(editor: &mut Editor, r: &AssetRef) -> Option<Tab> {
    let project = editor.project.clone()?;
    let dto_map = editor.domains.get::<DataState>().map(|st| st.assets.dtos.clone()).unwrap_or_default();
    let enum_map = editor.domains.get::<DataState>().map(|st| st.assets.enums.clone()).unwrap_or_default();
    match load_dto(&project.root.join(r.source_rel())) {
        Ok(dto) => Some(Tab::new(DATA, DtoDoc::with_schemas(r.path.clone(), dto, dto_map, enum_map))),
        Err(err) => {
            editor.io_error(&format!("cannot open {}", r.source_rel()), err.to_string());
            None
        }
    }
}

fn open_enum_tab(editor: &mut Editor, r: &AssetRef) -> Option<Tab> {
    let project = editor.project.clone()?;
    let dto_map = editor.domains.get::<DataState>().map(|st| st.assets.dtos.clone()).unwrap_or_default();
    let enum_map = editor.domains.get::<DataState>().map(|st| st.assets.enums.clone()).unwrap_or_default();
    match load_enum(&project.root.join(r.source_rel())) {
        Ok(en) => Some(Tab::new(DATA, EnumDoc::with_schemas(r.path.clone(), en, dto_map, enum_map))),
        Err(err) => {
            editor.io_error(&format!("cannot open {}", r.source_rel()), err.to_string());
            None
        }
    }
}

fn open_collection_tab(editor: &mut Editor, r: &AssetRef) -> Option<Tab> {
    let project = editor.project.clone()?;
    let dto_map = editor.domains.get::<DataState>().map(|st| st.assets.dtos.clone()).unwrap_or_default();
    let enum_map = editor.domains.get::<DataState>().map(|st| st.assets.enums.clone()).unwrap_or_default();
    match load_collection(&project.root.join(r.source_rel())) {
        Ok(col) => {
            let cached_dto =
                editor.domains.get::<DataState>().and_then(|st| st.dto_by_id(col.dto).map(|(_, d)| d.clone()));
            Some(Tab::new(DATA, CollectionDoc::new(r.path.clone(), col, cached_dto).with_schemas(dto_map, enum_map)))
        }
        Err(err) => {
            editor.io_error(&format!("cannot open {}", r.source_rel()), err.to_string());
            None
        }
    }
}

fn data_row_menu(_editor: &Editor, r: &AssetRef) -> Vec<MenuEntry> {
    vec![MenuEntry::from((
        Some(IconRef::from(IconId::Code)),
        "Show generated Rust".to_string(),
        Action::from(DataStep::OpenRustPreview(r.clone())),
    ))]
}

fn parse_source_ref(source_rel: &str) -> Option<AssetRef> {
    project_core::asset_ref::parse(source_rel, DATA, &[DTO_SUFFIX, ENUM_SUFFIX, COLLECTION_SUFFIX, CURVE_SUFFIX])
}

fn parse_rust_tab(editor: &mut Editor, key: &str) -> Option<Tab> {
    let base = key.strip_suffix("#rust")?;
    let source = parse_source_ref(base)?;
    let project = editor.project.clone()?;
    let target = rust_preview_path(&project, &source);
    if !target.exists() {
        DataState::generate(editor);
    }
    Some(Tab::new(DATA, TextDoc::open(&project, source, "rust", target)))
}

fn rust_tab_exists(editor: &Editor, key: &str) -> bool {
    let Some(base) = key.strip_suffix("#rust") else {
        return false;
    };
    let Some(source) = parse_source_ref(base) else {
        return false;
    };
    editor.project.as_ref().is_some_and(|p| p.root.join(source.source_rel()).is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_icons_rasterize() {
        editor_core::ui::icons::assert_icons_rasterize(DATA_ICONS);
    }

    #[test]
    fn data_and_core_icon_names_are_disjoint() {
        for icon in DATA_ICONS {
            for id in IconId::ALL {
                assert_ne!(icon.name(), IconRef::from(id).name(), "{id:?} clashes with a Data icon");
            }
        }
    }

    #[test]
    fn domain_names_its_backend() {
        let d = DataDomain;
        assert_eq!(d.id(), DATA);
        assert_eq!(d.info().title, "Data");
        assert_eq!(d.backend().map(|b| b.id()), Some(DATA));
    }
}
