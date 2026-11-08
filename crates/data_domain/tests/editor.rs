//! Headless GUI and document tests for `data_domain`'s `editor` feature (`S1.3`).

#![cfg(feature = "editor")]

use bevy::prelude::*;
use project_core::Project;

use editor_core::EditorPlugins;
use editor_core::action::{Action, TabAction};
use editor_core::dynval::{PropKey, PropValue};
use editor_core::menu::MainMenu;
use editor_core::state::editor::Editor;
use editor_core::tab::TextDoc;
use editor_core::ui::areas::{AreasPlugin, editor_topbar, right_sidebar};
use editor_core::ui::menu::menu_entries;
use editor_core::ui::shell::DomainWindowPlugin;

use data_domain::backend::model::{DataValue, FieldSettings, FieldType, GameAssetKind};
use data_domain::backend::ron_io::{load_collection, load_dto, load_enum};
use data_domain::editor::{
    AssetPickerCatalog, COLLECTION_KIND, CURVE_KIND, CollectionAction, CollectionDetailBody, CollectionDoc,
    CollectionDtoSummaryText, CollectionListBody, CollectionProp, CollectionSidebar, CollectionTopbar, ConstraintField,
    DATA, DATA_CURVE_VIEW, DTO_KIND, DataColorSwatch, DataCurveDoc, DataCurveEdit, DataCurveMiniPreview,
    DataGradientMiniPreview, DataJumpAssetButton, DataState, DataStep, DtoAction, DtoDefaultSummaryText, DtoDoc,
    DtoProp, DtoSidebar, DtoTopbar, DtoViewBody, ENUM_KIND, ENUM_VIEW, EnumAction, EnumDoc, EnumProp, EnumSidebar,
    EnumTopbar, EnumViewBody, apply_set_component, apply_set_value, collection_ref, collection_view, create_asset,
    curve_ref, data_value_to_prop, dto_ref, dto_view, enum_ref, enum_view, euler_deg_to_quat, quat_to_euler_deg,
    test_editor,
};
use project_core::DomainId;
use project_core::asset_index::{DomainIndex, IndexedAsset, SubItem};
use project_core::asset_ref::AssetRef;
use project_core::asset_uuid::AssetUuid;

fn open_temp_project() -> (tempfile::TempDir, Editor) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("assets")).unwrap();
    std::fs::write(dir.path().join("Cargo.toml"), "[workspace]\nmembers = [\"crates/*\"]\n").unwrap();
    let _p = Project::open(dir.path()).unwrap();
    let mut e = test_editor();
    e.open_project(dir.path()).unwrap();
    (dir, e)
}

#[test]
fn data_assets_menu_and_templates() {
    let (_dir, mut e) = open_temp_project();
    e.apply(Action::SwitchDomain(DATA));
    let entries = menu_entries(MainMenu::Assets, &e);
    let labels: Vec<&str> = entries.iter().map(|m| m.label.as_str()).collect();
    assert_eq!(
        labels,
        [
            "New DTO Schema...",
            "New Enum / Union...",
            "New Collection...",
            "New Curve...",
            "New folder...",
            "",
            "Generate"
        ]
    );

    // Create DTO via menu template
    e.apply(entries[0].action.clone());
    e.apply(Action::Submit("gameplay/enemy".into()));
    let dto_r = dto_ref("gameplay/enemy");
    assert!(e.project.as_ref().unwrap().root.join(dto_r.source_rel()).is_file());
    assert_eq!(e.tab().and_then(|t| t.doc().asset()), Some(&dto_r));

    // Create Enum / Union via menu template
    e.apply(entries[1].action.clone());
    e.apply(Action::Submit("gameplay/rarity".into()));
    let enum_r = enum_ref("gameplay/rarity");
    assert!(e.project.as_ref().unwrap().root.join(enum_r.source_rel()).is_file());
    assert_eq!(e.tab().and_then(|t| t.doc().asset()), Some(&enum_r));
    assert_eq!(e.tab().map(|t| t.doc().view()), Some(ENUM_VIEW));

    // Create Collection via menu template
    e.apply(entries[2].action.clone());
    e.apply(Action::Submit("gameplay/enemies".into()));
    let col_r = collection_ref("gameplay/enemies");
    assert!(e.project.as_ref().unwrap().root.join(col_r.source_rel()).is_file());
    assert_eq!(e.tab().and_then(|t| t.doc().asset()), Some(&col_r));

    // Create Curve via menu template
    e.apply(entries[3].action.clone());
    e.apply(Action::Submit("gameplay/xp_curve".into()));
    let crv_r = curve_ref("gameplay/xp_curve");
    assert!(e.project.as_ref().unwrap().root.join(crv_r.source_rel()).is_file());
    assert_eq!(e.tab().and_then(|t| t.doc().asset()), Some(&crv_r));
    assert_eq!(e.tab().map(|t| t.doc().view()), Some(DATA_CURVE_VIEW));
}

#[test]
fn dto_doc_add_rename_retype_reorder_delete_undo_redo_and_save() {
    let (dir, mut e) = open_temp_project();
    assert!(!e.domains.get::<DataState>().unwrap().schema_changed(), "no schema change initially");
    let dto_r = create_asset(&mut e, DTO_KIND, "gameplay/enemy").unwrap();
    assert_eq!(e.tab().and_then(|t| t.doc().asset()), Some(&dto_r));

    // Draft and add field 1: `name: String` (is_name = true)
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftName),
        value: PropValue::Str("name".into()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftType),
        value: PropValue::Str("String".into()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftCategory),
        value: PropValue::Str("Identity".into()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftIsName),
        value: PropValue::Bool(true),
        merge: false,
    }));
    e.apply(Action::from(DtoAction::AddField));

    // Draft and add field 2: `hp: I32` (is_name = true)
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftName),
        value: PropValue::Str("hp".into()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftType),
        value: PropValue::Str("I32".into()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftCategory),
        value: PropValue::Str("Combat".into()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftIsName),
        value: PropValue::Bool(true),
        merge: false,
    }));
    e.apply(Action::from(DtoAction::AddField));

    assert!(e.domains.get::<DataState>().unwrap().schema_changed(), "adding fields changes the schema hash");

    let (name_id, hp_id) = {
        let doc = e.active_doc::<DtoDoc>().unwrap();
        assert_eq!(doc.dto.fields.len(), 2);
        assert_eq!(doc.dto.fields[0].name, "name");
        assert_eq!(doc.dto.fields[1].name, "hp");
        (doc.dto.fields[0].id, doc.dto.fields[1].id)
    };

    // Expand inline Constraints for `hp`
    e.apply(Action::from(DtoAction::ToggleConstraints(hp_id)));
    assert_eq!(e.active_doc::<DtoDoc>().unwrap().expanded_constraints, Some(hp_id));

    // Edit `hp` default value and constraints (`min = 0`, `max = 500`, `slider = true`, `doc = "Hit points"`)
    e.apply(Action::Tab(TabAction::SetComponent {
        key: PropKey::new(DtoProp::FieldDefault(hp_id)),
        comp: 0,
        value: 100.0,
        is_final: true,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::FieldDoc(hp_id)),
        value: PropValue::Str("Hit points".into()),
        merge: false,
    }));
    e.apply(Action::from(DtoAction::SetConstraintOption { id: hp_id, field: ConstraintField::Min, some: true }));
    e.apply(Action::from(DtoAction::SetConstraintOption { id: hp_id, field: ConstraintField::Max, some: true }));
    e.apply(Action::Tab(TabAction::SetComponent {
        key: PropKey::new(DtoProp::FieldMax(hp_id)),
        comp: 0,
        value: 500.0,
        is_final: true,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::FieldSlider(hp_id)),
        value: PropValue::Bool(true),
        merge: false,
    }));

    // Setting `min = 200` clamps `hp.default` from 100 up to 200 immediately!
    e.apply(Action::Tab(TabAction::SetComponent {
        key: PropKey::new(DtoProp::FieldMin(hp_id)),
        comp: 0,
        value: 200.0,
        is_final: true,
    }));
    assert_eq!(e.active_doc::<DtoDoc>().unwrap().dto.fields[1].default, DataValue::I32(200));

    // Resetting `min` constraint back to `None`, then Undo reset, then set `min = 0.0` and `default = 100`
    e.apply(Action::from(DtoAction::ResetConstraint { id: hp_id, field: ConstraintField::Min }));
    assert_eq!(e.active_doc::<DtoDoc>().unwrap().dto.fields[1].settings.min, None);
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(e.active_doc::<DtoDoc>().unwrap().dto.fields[1].settings.min, Some(200.0));
    e.apply(Action::Tab(TabAction::SetComponent {
        key: PropKey::new(DtoProp::FieldMin(hp_id)),
        comp: 0,
        value: 0.0,
        is_final: true,
    }));
    e.apply(Action::Tab(TabAction::SetComponent {
        key: PropKey::new(DtoProp::FieldDefault(hp_id)),
        comp: 0,
        value: 100.0,
        is_final: true,
    }));

    {
        let doc = e.active_doc::<DtoDoc>().unwrap();
        let hp = doc.dto.fields.iter().find(|f| f.id == hp_id).unwrap();
        assert_eq!(hp.default, DataValue::I32(100));
        assert_eq!(hp.settings.doc, "Hit points");
        assert_eq!(hp.settings.min, Some(0.0));
        assert_eq!(hp.settings.max, Some(500.0));
        assert!(hp.settings.slider);
    }

    // Move `hp` up before `name`
    e.apply(Action::from(DtoAction::MoveField { id: hp_id, delta: -1 }));
    assert_eq!(e.active_doc::<DtoDoc>().unwrap().dto.fields[0].id, hp_id);

    // Undo move -> `name` is first again
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(e.active_doc::<DtoDoc>().unwrap().dto.fields[0].id, name_id);

    // Redo move -> `hp` is first again
    e.apply(Action::Tab(TabAction::Redo));
    assert_eq!(e.active_doc::<DtoDoc>().unwrap().dto.fields[0].id, hp_id);

    // Delete `name` field, then Undo delete
    e.apply(Action::from(DtoAction::DeleteField(name_id)));
    assert_eq!(e.active_doc::<DtoDoc>().unwrap().dto.fields.len(), 1);
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(e.active_doc::<DtoDoc>().unwrap().dto.fields.len(), 2);

    // Save -> writes `.dto.ron` and generates `crates/quillshade/src/data/dtos/gameplay/enemy.rs`
    e.apply(Action::Save);
    assert!(!e.is_dirty());
    let disk_dto = load_dto(&dir.path().join(dto_r.source_rel())).unwrap();
    assert_eq!(disk_dto.fields.len(), 2);
    let gen_rs = dir.path().join("crates/quillshade/src/data/dtos/gameplay/enemy.rs");
    assert!(gen_rs.is_file());
    let rs_text = std::fs::read_to_string(&gen_rs).unwrap();
    assert!(rs_text.contains("pub struct GameplayEnemy"));
    assert!(rs_text.contains("/// Hit points"));
    assert!(rs_text.contains("pub hp: i32"));
}

#[test]
fn rust_preview_tab_opens_and_reloads_on_save() {
    let (_dir, mut e) = open_temp_project();
    let dto_r = create_asset(&mut e, DTO_KIND, "gameplay/weapon").unwrap();
    let dto_id = e.active_doc::<DtoDoc>().unwrap().dto.id;
    e.apply(Action::from(DtoAction::AddField));
    e.apply(Action::Save);

    e.apply(Action::from(DataStep::OpenRustPreview(dto_r.clone())));
    let td = e.active_doc::<TextDoc>().expect("TextDoc preview tab should be active");
    assert_eq!(td.source, dto_r);
    assert_eq!(td.lang, "rust");
    assert!(td.text.as_deref().unwrap_or_default().contains("pub struct GameplayWeapon"));

    // Create a collection bound to `gameplay/weapon` and verify its dedicated Rust preview file
    let col_r = create_asset(&mut e, COLLECTION_KIND, "gameplay/weapons").unwrap();
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(CollectionProp::DtoSchema),
        value: PropValue::Str(dto_id.to_string()),
        merge: false,
    }));
    e.apply(Action::Save);
    e.apply(Action::from(DataStep::OpenRustPreview(col_r.clone())));
    let col_td = e.active_doc::<TextDoc>().expect("Collection TextDoc preview tab should be active");
    assert_eq!(col_td.source, col_r);
    assert!(col_td.path.ends_with("crates/quillshade/src/data/collections/gameplay/weapons.rs"));
    let col_preview = col_td.text.as_deref().unwrap_or_default();
    assert!(col_preview.contains("pub const GAMEPLAY_WEAPONS"));
    assert!(col_preview.contains("pub const LEN: usize = 0;"));
    assert!(!col_preview.contains("ROW_0"));
    assert_eq!(data_domain::editor::quick_start::QUICK_START_STEPS, [("Querying", false)]);
}

#[test]
fn collection_doc_row_crud_cell_edits_undo_redo_and_save() {
    let (dir, mut e) = open_temp_project();
    let dto_r = create_asset(&mut e, DTO_KIND, "gameplay/item").unwrap();
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftName),
        value: PropValue::Str("title".into()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftType),
        value: PropValue::Str("String".into()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftIsName),
        value: PropValue::Bool(true),
        merge: false,
    }));
    e.apply(Action::from(DtoAction::AddField));
    let title_field_id = e.active_doc::<DtoDoc>().unwrap().dto.fields[0].id;
    e.apply(Action::Save);

    let col_r = create_asset(&mut e, COLLECTION_KIND, "gameplay/items").unwrap();
    assert_eq!(e.tab().and_then(|t| t.doc().asset()), Some(&col_r));

    // Add 1st row
    e.apply(Action::from(CollectionAction::AddRow));
    let row0_id = e.active_doc::<CollectionDoc>().unwrap().col.rows[0].id;

    // Edit cell `title` on row 0
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(CollectionProp::RowValue { row: row0_id, path: vec![title_field_id] }),
        value: PropValue::Str("Iron Sword".into()),
        merge: false,
    }));
    assert_eq!(
        e.active_doc::<CollectionDoc>().unwrap().col.rows[0].values.get(&title_field_id),
        Some(&DataValue::String("Iron Sword".into()))
    );

    // Duplicate row 0 -> creates row 1 with `"Iron Sword"`
    e.apply(Action::from(CollectionAction::DuplicateRow(Some(row0_id))));
    assert_eq!(e.active_doc::<CollectionDoc>().unwrap().col.rows.len(), 2);
    let row1_id = e.active_doc::<CollectionDoc>().unwrap().col.rows[1].id;
    assert_ne!(row0_id, row1_id);

    // Reset cell on row 1 back to default `""`
    e.apply(Action::from(CollectionAction::ResetCell { row: row1_id, path: vec![title_field_id] }));
    assert_eq!(
        e.active_doc::<CollectionDoc>().unwrap().col.rows[1].values.get(&title_field_id),
        Some(&DataValue::String(String::new()))
    );

    // Undo reset -> back to `"Iron Sword"`
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(
        e.active_doc::<CollectionDoc>().unwrap().col.rows[1].values.get(&title_field_id),
        Some(&DataValue::String("Iron Sword".into()))
    );

    // Save collection -> writes `.collection.ron` and `assets/quillshade/data/autogen/gameplay/items.collection.qsd`
    e.apply(Action::Save);
    assert!(!e.is_dirty());
    let disk_col = load_collection(&dir.path().join(col_r.source_rel())).unwrap();
    assert_eq!(disk_col.rows.len(), 2);
    let qsd = dir.path().join("assets/quillshade/data/autogen/gameplay/items.collection.qsd");
    assert!(qsd.is_file());
    let _ = dto_r;
}

#[test]
fn all_25_slice1_field_types_widget_and_prop_roundtrip() {
    let settings = FieldSettings::default();
    for ty in FieldType::ALL_PRIMITIVES {
        let mut val = ty.default_value();
        let prop = data_value_to_prop(ty, &val).unwrap_or_else(|| panic!("missing PropValue conversion for {ty:?}"));
        match prop {
            PropValue::Bool(b) => {
                assert!(apply_set_value(ty, &mut val, &PropValue::Bool(!b), &settings));
            }
            PropValue::Str(_) => {
                assert!(apply_set_value(ty, &mut val, &PropValue::Str("hello".into()), &settings));
            }
            PropValue::Float(_) | PropValue::Float2(_) | PropValue::Float3(_) | PropValue::Float4(_) => {
                let comp_val = if *ty == FieldType::Color { 0.5 } else { 7.0 };
                assert!(apply_set_component(ty, &mut val, 0, comp_val, &settings));
            }
            PropValue::Color(_) | PropValue::Int(_) => unreachable!("unused PropValue variant"),
        }
    }

    // Quat Euler XYZ degree round-trip
    let deg = [30.0f32, -45.0, 60.0];
    let q = euler_deg_to_quat(deg);
    let back = quat_to_euler_deg(q);
    for (a, b) in deg.iter().zip(back.iter()) {
        assert!((a - b).abs() < 1e-2, "expected {deg:?}, got {back:?}");
    }
}

#[test]
fn headless_bevy_ecs_dto_and_collection_views_populate() {
    let (_dir, mut editor) = open_temp_project();
    let dto_r = create_asset(&mut editor, DTO_KIND, "gameplay/hero").unwrap();
    editor.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftName),
        value: PropValue::Str("tint".into()),
        merge: false,
    }));
    editor.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftType),
        value: PropValue::Str("Color".into()),
        merge: false,
    }));
    editor.apply(Action::from(DtoAction::AddField));
    editor.apply(Action::Save);

    let col_r = create_asset(&mut editor, COLLECTION_KIND, "gameplay/heroes").unwrap();
    editor.apply(Action::from(CollectionAction::AddRow));
    editor.apply(Action::Save);

    // Switch back to DTO tab first
    editor.apply(Action::OpenAsset(dto_r));

    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default(), bevy::scene::ScenePlugin, bevy::input::InputPlugin));
    app.init_asset::<Font>().init_asset::<Image>();
    app.insert_resource(editor);
    app.add_plugins((EditorPlugins, AreasPlugin, DomainWindowPlugin));

    app.world_mut().spawn_scene(editor_topbar()).unwrap();
    app.world_mut().spawn_scene(right_sidebar()).unwrap();
    app.world_mut().spawn_scene(dto_view()).unwrap();
    app.world_mut().spawn_scene(collection_view()).unwrap();
    app.update();
    app.update();

    {
        let w = app.world_mut();
        let topbar_e = w.query_filtered::<Entity, With<DtoTopbar>>().single(w).unwrap();
        let sidebar_e = w.query_filtered::<Entity, With<DtoSidebar>>().single(w).unwrap();
        let body_e = w.query_filtered::<Entity, With<DtoViewBody>>().single(w).unwrap();
        assert!(w.get::<Children>(topbar_e).is_some_and(|c| !c.is_empty()));
        assert!(w.get::<Children>(sidebar_e).is_some_and(|c| !c.is_empty()));
        assert!(w.get::<Children>(body_e).is_some_and(|c| !c.is_empty()));
        // Exactly 1 DataColorSwatch in DTO view for the `tint: Color` field
        let swatches = w.query_filtered::<Entity, With<DataColorSwatch>>().iter(w).count();
        assert_eq!(swatches, 1);
    }

    // Activate Collection tab and update
    app.world_mut().resource_mut::<Editor>().apply(Action::OpenAsset(col_r));
    app.update();
    app.update();

    {
        let w = app.world_mut();
        let topbar_e = w.query_filtered::<Entity, With<CollectionTopbar>>().single(w).unwrap();
        let sidebar_e = w.query_filtered::<Entity, With<CollectionSidebar>>().single(w).unwrap();
        let list_e = w.query_filtered::<Entity, With<CollectionListBody>>().single(w).unwrap();
        let detail_e = w.query_filtered::<Entity, With<CollectionDetailBody>>().single(w).unwrap();
        assert!(w.get::<Children>(topbar_e).is_some_and(|c| !c.is_empty()));
        assert!(w.get::<Children>(sidebar_e).is_some_and(|c| !c.is_empty()));
        assert!(w.get::<Children>(list_e).is_some_and(|c| !c.is_empty()));
        assert!(w.get::<Children>(detail_e).is_some_and(|c| !c.is_empty()));
    }
}

#[test]
fn duplicate_row_uuid_focus_filter_regenerate_and_copy() {
    let (_dir, mut e) = open_temp_project();
    let _dto_r = create_asset(&mut e, DTO_KIND, "library/book").unwrap();
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftName),
        value: PropValue::Str("title".into()),
        merge: false,
    }));
    e.apply(Action::from(DtoAction::AddField));
    e.apply(Action::Save);

    let _col_r = create_asset(&mut e, COLLECTION_KIND, "library/books_core").unwrap();
    e.apply(Action::from(CollectionAction::AddRow));
    e.apply(Action::from(CollectionAction::AddRow));
    let dup_id = e.active_doc::<CollectionDoc>().unwrap().col.rows[0].id;

    // Force a duplicate row UUID on row #1
    {
        let col_doc = e.active_doc_mut::<CollectionDoc>().unwrap();
        col_doc.col.rows[1].id = dup_id;
        let path = col_doc.path.clone();
        let col = col_doc.col.clone();
        e.apply(Action::from(DataStep::SyncCollectionInMemory { path, col }));
    }

    let de002 = e
        .domains
        .get::<DataState>()
        .unwrap()
        .diagnostics
        .iter()
        .find(|d| d.code == "DE002")
        .cloned()
        .expect("expected DE002 diagnostic");

    // Focus diagnostic -> filters row list to `dup_id` and selects `dup_id`
    e.apply(Action::FocusDiagnostic(de002));
    {
        let col_doc = e.active_doc::<CollectionDoc>().unwrap();
        assert_eq!(col_doc.selected_row, Some(dup_id));
        assert_eq!(col_doc.filter, dup_id.to_string());
    }

    // CopyRowId -> sets `editor.copy_text`
    e.apply(Action::from(CollectionAction::CopyRowId(dup_id)));
    assert_eq!(e.copy_text.as_deref(), Some(dup_id.to_string().as_str()));

    // RegenerateRowId -> mints a fresh UUIDv7 on the duplicate row, clears filter, and resolves DE002
    e.apply(Action::from(CollectionAction::RegenerateRowId(dup_id)));
    let new_row1_id = {
        let col_doc = e.active_doc::<CollectionDoc>().unwrap();
        assert_eq!(col_doc.col.rows[0].id, dup_id);
        assert_ne!(col_doc.col.rows[1].id, dup_id);
        assert_eq!(col_doc.selected_row, Some(col_doc.col.rows[1].id));
        assert!(col_doc.filter.is_empty());
        col_doc.col.rows[1].id
    };
    assert!(
        !e.domains.get::<DataState>().unwrap().diagnostics.iter().any(|d| d.code == "DE002"),
        "DE002 should be cleared after RegenerateRowId"
    );

    // ClearFilter action
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(CollectionProp::Filter),
        value: PropValue::Str("query".into()),
        merge: false,
    }));
    assert_eq!(e.active_doc::<CollectionDoc>().unwrap().filter, "query");
    e.apply(Action::from(CollectionAction::ClearFilter));
    assert!(e.active_doc::<CollectionDoc>().unwrap().filter.is_empty());
    let _ = new_row1_id;
}

#[test]
fn nested_dto_3_level_schema_and_collection_subforms_and_headless_ui() {
    let (dir, mut e) = open_temp_project();

    // 1. Level 3 DTO: `lore/sector_coords` (`sector_name: String`, `is_name: true`)
    let _coords_r = create_asset(&mut e, DTO_KIND, "lore/sector_coords").unwrap();
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftName),
        value: PropValue::Str("sector_name".into()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftType),
        value: PropValue::Str("String".into()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftIsName),
        value: PropValue::Bool(true),
        merge: false,
    }));
    e.apply(Action::from(DtoAction::AddField));
    let (coords_dto_id, f_sector_name_id) = {
        let doc = e.active_doc::<DtoDoc>().unwrap();
        (doc.dto.id, doc.dto.fields[0].id)
    };
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::FieldDefault(f_sector_name_id)),
        value: PropValue::Str("Coreward".into()),
        merge: false,
    }));
    e.apply(Action::Save);

    // 2. Level 2 DTO: `lore/planet` (`planet_name: String`, `coordinates: Dto(sector_coords)`)
    let _planet_r = create_asset(&mut e, DTO_KIND, "lore/planet").unwrap();
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftName),
        value: PropValue::Str("planet_name".into()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftType),
        value: PropValue::Str("String".into()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftIsName),
        value: PropValue::Bool(true),
        merge: false,
    }));
    e.apply(Action::from(DtoAction::AddField));

    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftName),
        value: PropValue::Str("coordinates".into()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftType),
        value: PropValue::Str("Dto".into()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftDtoTarget),
        value: PropValue::Str(coords_dto_id.to_string()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftIsName),
        value: PropValue::Bool(true),
        merge: false,
    }));
    e.apply(Action::from(DtoAction::AddField));
    let (planet_dto_id, f_planet_name_id, f_coords_id) = {
        let doc = e.active_doc::<DtoDoc>().unwrap();
        (doc.dto.id, doc.dto.fields[0].id, doc.dto.fields[1].id)
    };
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::FieldDefault(f_planet_name_id)),
        value: PropValue::Str("Aethelgard".into()),
        merge: false,
    }));
    e.apply(Action::Save);

    // 3. Level 1 DTO: `library/author` (`pen_name: String`, `homeworld: Dto(planet)`)
    let _author_r = create_asset(&mut e, DTO_KIND, "library/author").unwrap();
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftName),
        value: PropValue::Str("pen_name".into()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftType),
        value: PropValue::Str("String".into()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftIsName),
        value: PropValue::Bool(true),
        merge: false,
    }));
    e.apply(Action::from(DtoAction::AddField));

    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftName),
        value: PropValue::Str("homeworld".into()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftType),
        value: PropValue::Str("Dto".into()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftDtoTarget),
        value: PropValue::Str(planet_dto_id.to_string()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftIsName),
        value: PropValue::Bool(true),
        merge: false,
    }));
    e.apply(Action::from(DtoAction::AddField));
    let (author_dto_id, f_pen_name_id, f_homeworld_id) = {
        let doc = e.active_doc::<DtoDoc>().unwrap();
        (doc.dto.id, doc.dto.fields[0].id, doc.dto.fields[1].id)
    };
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::FieldDefault(f_pen_name_id)),
        value: PropValue::Str("Anonymous".into()),
        merge: false,
    }));
    e.apply(Action::Save);

    // 4. Root DTO: `library/book` (`title: String`, `author: Dto(author)`)
    let book_r = create_asset(&mut e, DTO_KIND, "library/book").unwrap();
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftName),
        value: PropValue::Str("title".into()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftType),
        value: PropValue::Str("String".into()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftIsName),
        value: PropValue::Bool(true),
        merge: false,
    }));
    e.apply(Action::from(DtoAction::AddField));

    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftName),
        value: PropValue::Str("author".into()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftType),
        value: PropValue::Str("Dto".into()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftDtoTarget),
        value: PropValue::Str(author_dto_id.to_string()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftIsName),
        value: PropValue::Bool(true),
        merge: false,
    }));
    e.apply(Action::from(DtoAction::AddField));
    let (book_dto_id, f_title_id, f_author_id) = {
        let doc = e.active_doc::<DtoDoc>().unwrap();
        (doc.dto.id, doc.dto.fields[0].id, doc.dto.fields[1].id)
    };

    // Expand all 3 nested DTO default sub-forms in `book.dto.ron` and override the deepest leaf default
    let p1 = vec![f_author_id];
    let p2 = vec![f_author_id, f_homeworld_id];
    let p3 = vec![f_author_id, f_homeworld_id, f_coords_id];
    let p_leaf = vec![f_author_id, f_homeworld_id, f_coords_id, f_sector_name_id];
    e.apply(Action::from(DtoAction::ToggleDtoDefault(p1.clone())));
    e.apply(Action::from(DtoAction::ToggleDtoDefault(p2.clone())));
    e.apply(Action::from(DtoAction::ToggleDtoDefault(p3.clone())));
    assert!(e.active_doc::<DtoDoc>().unwrap().expanded_dtos.contains(&p3));

    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::NestedFieldDefault(p_leaf.clone())),
        value: PropValue::Str("Outer Rim".into()),
        merge: false,
    }));
    {
        let doc = e.active_doc::<DtoDoc>().unwrap();
        let fallback = DataValue::String(String::new());
        assert_eq!(doc.get_default_at_path(&p_leaf, &fallback), &DataValue::String("Outer Rim".into()));
    }
    // ResetNestedDefault resets `sector_name` back to `sector_coords`'s schema default (`"Coreward"`), then Undo
    e.apply(Action::from(DtoAction::ResetNestedDefault(p_leaf.clone())));
    {
        let doc = e.active_doc::<DtoDoc>().unwrap();
        let fallback = DataValue::String(String::new());
        assert_eq!(doc.get_default_at_path(&p_leaf, &fallback), &DataValue::String("Coreward".into()));
    }
    e.apply(Action::Tab(TabAction::Undo));
    e.apply(Action::Save);

    let book_ron = std::fs::read_to_string(dir.path().join(book_r.source_rel())).unwrap();
    assert!(book_ron.contains("// library/author"), "expected D27 comment on Dto field: {book_ron}");

    // 5. Create `library/books_core` collection bound to `library/book`
    let col_r = create_asset(&mut e, COLLECTION_KIND, "library/books_core").unwrap();
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(CollectionProp::DtoSchema),
        value: PropValue::Str(book_dto_id.to_string()),
        merge: false,
    }));
    e.apply(Action::from(CollectionAction::AddRow));
    let row0_id = e.active_doc::<CollectionDoc>().unwrap().col.rows[0].id;

    // Expand 3 levels of nested DTO fields in the collection inspector
    e.apply(Action::from(CollectionAction::ToggleDtoField { path: p1.clone() }));
    e.apply(Action::from(CollectionAction::ToggleDtoField { path: p2.clone() }));
    e.apply(Action::from(CollectionAction::ToggleDtoField { path: p3.clone() }));
    assert!(e.active_doc::<CollectionDoc>().unwrap().expanded_dtos.contains(&p3));

    // Edit `title` and 3-level nested `author.homeworld.coordinates.sector_name`
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(CollectionProp::RowValue { row: row0_id, path: vec![f_title_id] }),
        value: PropValue::Str("Codex of Emberlight".into()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(CollectionProp::RowValue { row: row0_id, path: p_leaf.clone() }),
        value: PropValue::Str("Helios Reach".into()),
        merge: false,
    }));
    {
        let col_doc = e.active_doc::<CollectionDoc>().unwrap();
        let dtos = &col_doc.cached_dtos;
        assert_eq!(col_doc.get_value_at_path(row0_id, &p_leaf, dtos), Some(DataValue::String("Helios Reach".into())));
        let summary = col_doc.cached_dto.as_ref().unwrap().short_view_string_indexed_with_dtos(
            &col_doc.col.rows[0],
            Some(0),
            dtos,
        );
        assert_eq!(summary, "Codex of Emberlight | Anonymous | Aethelgard | Helios Reach");
    }

    // Reset nested cell at depth 3 -> returns to `book`'s field default (`"Outer Rim"`), then Undo
    e.apply(Action::from(CollectionAction::ResetCell { row: row0_id, path: p_leaf.clone() }));
    {
        let col_doc = e.active_doc::<CollectionDoc>().unwrap();
        assert_eq!(
            col_doc.get_value_at_path(row0_id, &p_leaf, &col_doc.cached_dtos),
            Some(DataValue::String("Outer Rim".into()))
        );
    }
    e.apply(Action::Tab(TabAction::Undo));
    e.apply(Action::Save);

    // 6. Headless Bevy ECS verification of expanded nested DTO sub-forms in both `dto_view` and `collection_view`
    e.apply(Action::OpenAsset(book_r));
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default(), bevy::scene::ScenePlugin, bevy::input::InputPlugin));
    app.init_asset::<Font>().init_asset::<Image>();
    app.insert_resource(e);
    app.add_plugins((EditorPlugins, AreasPlugin, DomainWindowPlugin));

    app.world_mut().spawn_scene(editor_topbar()).unwrap();
    app.world_mut().spawn_scene(right_sidebar()).unwrap();
    app.world_mut().spawn_scene(dto_view()).unwrap();
    app.world_mut().spawn_scene(collection_view()).unwrap();
    app.update();
    app.update();

    {
        let w = app.world_mut();
        // 3 levels of nested DTO toggle buttons (`author`, `homeworld`, `coordinates`) in `DtoEditor`
        let dto_summaries: Vec<String> =
            w.query_filtered::<&Text, With<DtoDefaultSummaryText>>().iter(w).map(|t| t.0.clone()).collect();
        assert_eq!(dto_summaries.len(), 3);
        assert!(dto_summaries.iter().all(|s| !s.contains('▾') && !s.contains('▸')));
    }

    app.world_mut().resource_mut::<Editor>().apply(Action::OpenAsset(col_r));
    app.update();
    app.update();

    {
        let w = app.world_mut();
        // 3 levels of nested DTO toggle buttons (`author`, `homeworld`, `coordinates`) in `CollectionEditor`
        let col_summaries: Vec<String> =
            w.query_filtered::<&Text, With<CollectionDtoSummaryText>>().iter(w).map(|t| t.0.clone()).collect();
        assert_eq!(col_summaries.len(), 3);
        assert!(col_summaries.iter().all(|s| !s.contains('▾') && !s.contains('▸')));
    }
}

#[test]
fn all_6_slice3_asset_ref_field_types_prop_roundtrip() {
    let settings = FieldSettings::default();
    let u = AssetUuid::legacy("material/ease_in.curve.ron");
    let u_str = u.to_string();

    for ty in [FieldType::Curve, FieldType::Material, FieldType::Texture, FieldType::LightEnvironment] {
        let mut val = ty.default_value();
        assert_eq!(data_value_to_prop(&ty, &val), Some(PropValue::Str(String::new())));
        assert!(apply_set_value(&ty, &mut val, &PropValue::Str(u_str.clone()), &settings));
        assert_eq!(data_value_to_prop(&ty, &val), Some(PropValue::Str(u_str.clone())));
        assert!(apply_set_value(&ty, &mut val, &PropValue::Str(String::new()), &settings));
        assert_eq!(val, ty.default_value());
    }

    // Gradient: `("<uuid>", <slot>)`
    let mut grad_val = FieldType::Gradient.default_value();
    assert_eq!(data_value_to_prop(FieldType::Gradient, &grad_val), Some(PropValue::Str(String::new())));
    assert!(apply_set_value(FieldType::Gradient, &mut grad_val, &PropValue::Str(format!("{u_str}:3")), &settings));
    assert_eq!(grad_val, DataValue::Gradient(u, 3));
    assert_eq!(data_value_to_prop(FieldType::Gradient, &grad_val), Some(PropValue::Str(format!("{u_str}:3"))));
    assert!(apply_set_value(FieldType::Gradient, &mut grad_val, &PropValue::Str(String::new()), &settings));
    assert_eq!(grad_val, DataValue::Gradient(AssetUuid::NIL, 0));

    // GameAsset: relative path under `assets/`
    let ga_ty = FieldType::GameAsset(GameAssetKind::Image);
    let mut ga_val = ga_ty.default_value();
    assert_eq!(data_value_to_prop(&ga_ty, &ga_val), Some(PropValue::Str(String::new())));
    assert!(apply_set_value(&ga_ty, &mut ga_val, &PropValue::Str("textures/stamp.png".into()), &settings));
    assert_eq!(ga_val, DataValue::GameAsset("textures/stamp.png".into()));
    assert!(apply_set_value(&ga_ty, &mut ga_val, &PropValue::Str("   ".into()), &settings));
    assert_eq!(ga_val, DataValue::GameAsset(String::new()));
}

#[test]
fn direct_asset_ref_pickers_previews_jump_and_diagnostics() {
    let (dir, mut e) = open_temp_project();

    // Write sample curve, gradient, texture RON files and game assets on disk for mini-previews, bake & DE008
    let curve_uuid = AssetUuid::legacy("material/ease_in.curve.ron");
    let grad_uuid = AssetUuid::legacy("material/palettes.gradients.ron");
    let mat_uuid = AssetUuid::legacy("material/book_cover.material.ron");
    let tex_uuid = AssetUuid::legacy("material/noise.texture.ron");
    let env_uuid = AssetUuid::legacy("light/day.environment.ron");

    let mat_dir = dir.path().join("quillshade/material");
    let light_dir = dir.path().join("quillshade/light");
    std::fs::create_dir_all(&mat_dir).unwrap();
    std::fs::create_dir_all(&light_dir).unwrap();
    std::fs::create_dir_all(dir.path().join("assets/textures")).unwrap();
    std::fs::create_dir_all(dir.path().join("assets/audio")).unwrap();
    std::fs::write(dir.path().join("assets/textures/stamp.png"), b"PNG").unwrap();
    std::fs::write(dir.path().join("assets/audio/chime.ogg"), b"OGG").unwrap();

    std::fs::write(
        mat_dir.join("ease_in.curve.ron"),
        format!(
            "Curve(\n    format_version: 2,\n    id: \"{curve_uuid}\",\n    interp: CatmullRom,\n    points: [(0.0, 0.0), (0.5, 0.25), (1.0, 1.0)],\n)\n"
        ),
    )
    .unwrap();
    std::fs::write(
        mat_dir.join("palettes.gradients.ron"),
        format!(
            "GradientCollection(\n    format_version: 2,\n    id: \"{grad_uuid}\",\n    slots: [\n        Slot(\n            id: 1,\n            name: \"heat\",\n            stops: [\n                Stop(pos: 0.0, color: (0.1, 0.0, 0.0, 1.0)),\n                Stop(pos: 1.0, color: (1.0, 0.8, 0.2, 1.0)),\n            ],\n        ),\n    ],\n)\n"
        ),
    )
    .unwrap();
    std::fs::write(
        mat_dir.join("noise.texture.ron"),
        format!(
            "TextureAsset(\n    format_version: 2,\n    id: \"{tex_uuid}\",\n    source: \"textures/stamp.png\",\n)\n"
        ),
    )
    .unwrap();

    let insert_external_indices = |ed: &mut Editor| {
        let mat_dom = DomainId::new("material");
        let mut mat_idx = DomainIndex::empty(mat_dom);
        mat_idx.assets.push(IndexedAsset {
            asset: AssetRef { domain: mat_dom, path: "ease_in".into(), suffix: "curve" },
            id: Ok(curve_uuid),
            sub_items: Vec::new(),
            refs: Vec::new(),
        });
        mat_idx.assets.push(IndexedAsset {
            asset: AssetRef { domain: mat_dom, path: "palettes".into(), suffix: "gradients" },
            id: Ok(grad_uuid),
            sub_items: vec![SubItem { id: 1, uuid: None, label: "heat".into() }],
            refs: Vec::new(),
        });
        mat_idx.assets.push(IndexedAsset {
            asset: AssetRef { domain: mat_dom, path: "book_cover".into(), suffix: "material" },
            id: Ok(mat_uuid),
            sub_items: Vec::new(),
            refs: Vec::new(),
        });
        mat_idx.assets.push(IndexedAsset {
            asset: AssetRef { domain: mat_dom, path: "noise".into(), suffix: "texture" },
            id: Ok(tex_uuid),
            sub_items: Vec::new(),
            refs: Vec::new(),
        });
        ed.asset_index.replace_domain(mat_idx);

        let light_dom = DomainId::new("light");
        let mut light_idx = DomainIndex::empty(light_dom);
        light_idx.assets.push(IndexedAsset {
            asset: AssetRef { domain: light_dom, path: "day".into(), suffix: "environment" },
            id: Ok(env_uuid),
            sub_items: Vec::new(),
            refs: Vec::new(),
        });
        ed.asset_index.replace_domain(light_idx);
    };
    insert_external_indices(&mut e);

    // Create DTO with all 6 Slice 3 asset reference fields
    let dto_r = create_asset(&mut e, DTO_KIND, "library/book").unwrap();
    insert_external_indices(&mut e);

    for (name, ty_label, ga_kind) in [
        ("wear_curve", "Curve", None),
        ("spine_gradient", "Gradient", None),
        ("cover_material", "Material", None),
        ("seal_texture", "Texture", None),
        ("reading_ambience", "LightEnvironment", None),
        ("ex_libris_stamp", "GameAsset", Some("Audio")),
    ] {
        e.apply(Action::Tab(TabAction::SetValue {
            key: PropKey::new(DtoProp::DraftName),
            value: PropValue::Str(name.into()),
            merge: false,
        }));
        e.apply(Action::Tab(TabAction::SetValue {
            key: PropKey::new(DtoProp::DraftType),
            value: PropValue::Str(ty_label.into()),
            merge: false,
        }));
        if let Some(kind_str) = ga_kind {
            e.apply(Action::Tab(TabAction::SetValue {
                key: PropKey::new(DtoProp::DraftGameAssetKind),
                value: PropValue::Str(kind_str.into()),
                merge: false,
            }));
        }
        e.apply(Action::from(DtoAction::AddField));
    }

    let (dto_id, f_curve, f_grad, f_mat, f_tex, f_env, f_ga) = {
        let doc = e.active_doc::<DtoDoc>().unwrap();
        assert_eq!(doc.dto.fields.len(), 6);
        assert_eq!(doc.dto.fields[5].ty, FieldType::GameAsset(GameAssetKind::Audio));
        (
            doc.dto.id,
            doc.dto.fields[0].id,
            doc.dto.fields[1].id,
            doc.dto.fields[2].id,
            doc.dto.fields[3].id,
            doc.dto.fields[4].id,
            doc.dto.fields[5].id,
        )
    };

    // Change `ex_libris_stamp` sub-kind from `AudioSource` to `Image` via `FieldGameAssetKind`
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::FieldGameAssetKind(f_ga)),
        value: PropValue::Str("Image".into()),
        merge: false,
    }));
    assert_eq!(e.active_doc::<DtoDoc>().unwrap().dto.fields[5].ty, FieldType::GameAsset(GameAssetKind::Image));

    // Populate default values for all 6 asset reference fields
    for (fid, val) in [
        (f_curve, curve_uuid.to_string()),
        (f_grad, format!("{grad_uuid}:1")),
        (f_mat, mat_uuid.to_string()),
        (f_tex, tex_uuid.to_string()),
        (f_env, env_uuid.to_string()),
        (f_ga, "textures/stamp.png".to_string()),
    ] {
        e.apply(Action::Tab(TabAction::SetValue {
            key: PropKey::new(DtoProp::FieldDefault(fid)),
            value: PropValue::Str(val),
            merge: false,
        }));
    }

    insert_external_indices(&mut e);
    e.apply(Action::Save);
    insert_external_indices(&mut e);

    // Verify D27 trailing reference comments in saved `.dto.ron`
    let dto_ron = std::fs::read_to_string(dir.path().join(dto_r.source_rel())).unwrap();
    assert!(dto_ron.contains("// material:ease_in"), "expected curve D27 comment in {dto_ron}");
    assert!(dto_ron.contains("// material:palettes.heat"), "expected gradient D27 comment in {dto_ron}");
    assert!(dto_ron.contains("// material:book_cover"), "expected material D27 comment in {dto_ron}");
    assert!(dto_ron.contains("// material:noise"), "expected texture D27 comment in {dto_ron}");
    assert!(dto_ron.contains("// light:day"), "expected light env D27 comment in {dto_ron}");

    // Create Collection bound to `library/book`, add row, and test live `DE008` + D27 save
    let col_r = create_asset(&mut e, COLLECTION_KIND, "library/books_core").unwrap();
    insert_external_indices(&mut e);
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(CollectionProp::DtoSchema),
        value: PropValue::Str(dto_id.to_string()),
        merge: false,
    }));
    e.apply(Action::from(CollectionAction::AddRow));
    let row0_id = e.active_doc::<CollectionDoc>().unwrap().col.rows[0].id;

    // Setting `ex_libris_stamp` (`GameAsset(Image)`) to an `.ogg` file triggers live `DE008` in `DataState`
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(CollectionProp::RowValue { row: row0_id, path: vec![f_ga] }),
        value: PropValue::Str("audio/chime.ogg".into()),
        merge: false,
    }));
    assert!(
        e.domains.get::<DataState>().unwrap().diagnostics.iter().any(|d| d.code == "DE008"),
        "expected live DE008 diagnostic when GameAsset(Image) points to audio/chime.ogg"
    );

    // Fixing `ex_libris_stamp` to `textures/stamp.png` clears `DE008`
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(CollectionProp::RowValue { row: row0_id, path: vec![f_ga] }),
        value: PropValue::Str("textures/stamp.png".into()),
        merge: false,
    }));
    let de008s: Vec<_> =
        e.domains.get::<DataState>().unwrap().diagnostics.iter().filter(|d| d.code == "DE008").cloned().collect();
    assert!(
        de008s.is_empty(),
        "DE008 should clear once GameAsset(Image) points to textures/stamp.png, got: {de008s:#?}"
    );

    insert_external_indices(&mut e);
    e.apply(Action::Save);
    insert_external_indices(&mut e);

    let col_ron = std::fs::read_to_string(dir.path().join(col_r.source_rel())).unwrap();
    assert!(col_ron.contains("// wear_curve, material:ease_in"), "expected curve D27 comment in {col_ron}");
    assert!(
        col_ron.contains("// spine_gradient, material:palettes.heat"),
        "expected gradient D27 comment in {col_ron}"
    );
    assert!(col_ron.contains("// reading_ambience, light:day"), "expected light env D27 comment in {col_ron}");

    // Verify headless Bevy ECS spawns `DataCurveMiniPreview`, `DataGradientMiniPreview`, and `DataJumpAssetButton`
    e.apply(Action::OpenAsset(dto_r));
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default(), bevy::scene::ScenePlugin, bevy::input::InputPlugin));
    app.init_asset::<Font>().init_asset::<Image>();
    app.insert_resource(e);
    app.add_plugins((EditorPlugins, AreasPlugin, DomainWindowPlugin));

    app.world_mut().spawn_scene(editor_topbar()).unwrap();
    app.world_mut().spawn_scene(right_sidebar()).unwrap();
    app.world_mut().spawn_scene(dto_view()).unwrap();
    app.world_mut().spawn_scene(collection_view()).unwrap();
    app.update();
    app.update();

    {
        let w = app.world_mut();
        let curves = w.query_filtered::<Entity, With<DataCurveMiniPreview>>().iter(w).count();
        let grads = w.query_filtered::<Entity, With<DataGradientMiniPreview>>().iter(w).count();
        let jumps = w.query_filtered::<&DataJumpAssetButton, ()>().iter(w).count();
        assert_eq!(curves, 1, "expected 1 curve mini-preview in DtoEditor");
        assert_eq!(grads, 1, "expected 1 gradient mini-preview in DtoEditor");
        assert_eq!(jumps, 5, "expected 5 jump buttons for the 5 editor-owned asset refs in DtoEditor");
    }

    app.world_mut().resource_mut::<Editor>().apply(Action::OpenAsset(col_r));
    app.update();
    app.update();

    {
        let w = app.world_mut();
        let curves = w.query_filtered::<Entity, With<DataCurveMiniPreview>>().iter(w).count();
        let grads = w.query_filtered::<Entity, With<DataGradientMiniPreview>>().iter(w).count();
        let jumps = w.query_filtered::<&DataJumpAssetButton, ()>().iter(w).count();
        assert_eq!(curves, 2, "expected 1 curve mini-preview in DtoEditor + 1 in CollectionEditor");
        assert_eq!(grads, 2, "expected 1 gradient mini-preview in DtoEditor + 1 in CollectionEditor");
        assert_eq!(jumps, 10, "expected 5 jump buttons in DtoEditor + 5 in CollectionEditor");
    }
}

#[test]
fn data_curve_authoring_picker_and_qsd_bake() {
    let (dir, mut e) = open_temp_project();
    let crv_r = create_asset(&mut e, CURVE_KIND, "gameplay/xp_curve").unwrap();
    assert_eq!(e.tab().map(|t| t.doc().view()), Some(DATA_CURVE_VIEW));

    // Edit the curve by adding a midpoint at (0.5, 0.8) and save.
    e.apply(Action::from(DataCurveEdit::new(curve_kit::editor::CurveAction::Add((0.5, 0.8)))));
    assert!(e.is_dirty());
    assert_eq!(e.active_doc::<DataCurveDoc>().unwrap().curve.points.len(), 3);
    e.apply(Action::Save);
    assert!(!e.is_dirty());

    let crv_id = e.asset_index.id_of(&crv_r).expect("data curve indexed with UUIDv7");
    assert!(crv_id.is_v7());

    // Verify AssetPickerCatalog includes `data:gameplay/xp_curve` with 3 points and Jump target.
    let cat = AssetPickerCatalog::from_editor(&e);
    assert_eq!(cat.curves.len(), 1);
    assert_eq!(cat.curves[0].0, crv_id);
    assert_eq!(cat.curves[0].1, "data:gameplay/xp_curve");
    assert_eq!(cat.curves[0].2.points, vec![(0.0, 0.0), (0.5, 0.8), (1.0, 1.0)]);
    assert_eq!(cat.refs_by_id.get(&crv_id), Some(&crv_r));

    // Create a DTO and Collection referencing the data curve.
    let dto_r = create_asset(&mut e, DTO_KIND, "gameplay/level_scaling").unwrap();
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftName),
        value: PropValue::Str("xp_growth".into()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftType),
        value: PropValue::Str("Curve".into()),
        merge: false,
    }));
    e.apply(Action::from(DtoAction::AddField));
    let f_crv = e.active_doc::<DtoDoc>().unwrap().dto.fields[0].id;
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::FieldDefault(f_crv)),
        value: PropValue::Str(crv_id.to_string()),
        merge: false,
    }));
    e.apply(Action::Save);

    let col_r = create_asset(&mut e, COLLECTION_KIND, "gameplay/levels").unwrap();
    e.apply(Action::from(CollectionAction::AddRow));
    let row0_id = e.active_doc::<CollectionDoc>().unwrap().col.rows[0].id;
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(CollectionProp::RowValue { row: row0_id, path: vec![f_crv] }),
        value: PropValue::Str(crv_id.to_string()),
        merge: false,
    }));
    e.apply(Action::Save);

    assert!(e.domains.get::<DataState>().unwrap().diagnostics.is_empty());

    let dto_ron = std::fs::read_to_string(dir.path().join(dto_r.source_rel())).unwrap();
    assert!(dto_ron.contains("// gameplay/xp_curve"), "expected data curve D27 comment in {dto_ron}");
    let col_ron = std::fs::read_to_string(dir.path().join(col_r.source_rel())).unwrap();
    assert!(col_ron.contains("// xp_growth, gameplay/xp_curve"), "expected data curve D27 comment in {col_ron}");

    // Verify baked `.collection.qsd` contains the 3-point curve!
    let qsd_bytes =
        std::fs::read(dir.path().join("assets/quillshade/data/autogen/gameplay/levels.collection.qsd")).unwrap();
    let dto = load_dto(&dir.path().join(dto_r.source_rel())).unwrap();
    let (_, payload) = data_domain::backend::verify_qsd_header(
        &qsd_bytes,
        data_domain::backend::compute_schema_hash(&std::collections::BTreeMap::from([(
            "gameplay/level_scaling".to_string(),
            dto,
        )])),
        None,
    )
    .unwrap();
    let decoded: Vec<(u128, (project_core::common::curve::CurveData,))> = postcard::from_bytes(payload).unwrap();
    assert_eq!(decoded.len(), 1);
    assert_eq!(decoded[0].0, row0_id.as_u128());
    assert_eq!(decoded[0].1.0.points, vec![(0.0, 0.0), (0.5, 0.8), (1.0, 1.0)]);

    // Duplicate the curve asset via `Action::DuplicateAsset` and verify fresh UUID and zero AE001 diagnostics.
    e.apply(Action::DuplicateAsset(crv_r.clone()));
    let copy_r = curve_ref("gameplay/xp_curve_copy");
    assert!(dir.path().join(copy_r.source_rel()).is_file());
    let copy_id = e.asset_index.id_of(&copy_r).expect("duplicated curve indexed");
    assert_ne!(copy_id, crv_id);
    assert!(e.index_diagnostics.is_empty(), "no AE001 duplicate UUID after DuplicateAsset: {:?}", e.index_diagnostics);
}

#[test]
fn soft_ref_authoring_picker_jump_and_diagnostics() {
    let (dir, mut e) = open_temp_project();

    // 1. Prop roundtrip for FieldType::SoftRef
    let target_dto_id = AssetUuid::legacy("library/book.dto.ron");
    let sr_ty = FieldType::SoftRef(target_dto_id);
    let mut sr_val = sr_ty.default_value();
    let settings = FieldSettings::default();
    let row_u = AssetUuid::legacy("library/books_core.collection.ron#row0");
    assert_eq!(data_value_to_prop(&sr_ty, &sr_val), Some(PropValue::Str(String::new())));
    assert!(apply_set_value(&sr_ty, &mut sr_val, &PropValue::Str(row_u.to_string()), &settings));
    assert_eq!(sr_val, DataValue::SoftRef(row_u));
    assert_eq!(data_value_to_prop(&sr_ty, &sr_val), Some(PropValue::Str(row_u.to_string())));
    assert!(apply_set_value(&sr_ty, &mut sr_val, &PropValue::Str(String::new()), &settings));
    assert_eq!(sr_val, DataValue::SoftRef(AssetUuid::NIL));

    // 2. Create `library/book` DTO with `title: String` (is_name = true) and self-referential `companion_volume: SoftRef<Book>`
    let book_dto_r = create_asset(&mut e, DTO_KIND, "library/book").unwrap();
    let book_dto_id = e.active_doc::<DtoDoc>().unwrap().dto.id;

    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftName),
        value: PropValue::Str("title".into()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftType),
        value: PropValue::Str("String".into()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftIsName),
        value: PropValue::Bool(true),
        merge: false,
    }));
    e.apply(Action::from(DtoAction::AddField));

    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftName),
        value: PropValue::Str("companion_volume".into()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftType),
        value: PropValue::Str("SoftRef".into()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftDtoTarget),
        value: PropValue::Str(book_dto_id.to_string()),
        merge: false,
    }));
    e.apply(Action::from(DtoAction::AddField));

    let (f_title, f_companion) = {
        let doc = e.active_doc::<DtoDoc>().unwrap();
        assert_eq!(doc.dto.fields.len(), 2);
        assert_eq!(doc.dto.fields[1].ty, FieldType::SoftRef(book_dto_id));
        assert_eq!(doc.dto.fields[1].default, DataValue::SoftRef(AssetUuid::NIL));
        (doc.dto.fields[0].id, doc.dto.fields[1].id)
    };

    // Toggle `Required` constraint on `SoftRef` field and then untoggle it
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::FieldRequired(f_companion)),
        value: PropValue::Bool(true),
        merge: false,
    }));
    assert!(e.active_doc::<DtoDoc>().unwrap().dto.fields[1].settings.required);
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::FieldRequired(f_companion)),
        value: PropValue::Bool(false),
        merge: false,
    }));
    assert!(!e.active_doc::<DtoDoc>().unwrap().dto.fields[1].settings.required);
    e.apply(Action::Save);

    // 3. Create `library/books_core` collection with 2 rows and cross-link them via `SoftRef<Book>`
    let books_col_r = create_asset(&mut e, COLLECTION_KIND, "library/books_core").unwrap();
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(CollectionProp::DtoSchema),
        value: PropValue::Str(book_dto_id.to_string()),
        merge: false,
    }));
    e.apply(Action::from(CollectionAction::AddRow));
    e.apply(Action::from(CollectionAction::AddRow));
    let (row0_id, row1_id) = {
        let doc = e.active_doc::<CollectionDoc>().unwrap();
        (doc.col.rows[0].id, doc.col.rows[1].id)
    };
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(CollectionProp::RowValue { row: row0_id, path: vec![f_title] }),
        value: PropValue::Str("Codex of Emberlight".into()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(CollectionProp::RowValue { row: row1_id, path: vec![f_title] }),
        value: PropValue::Str("Treatise on Astral Geometry".into()),
        merge: false,
    }));

    // Verify `AssetPickerCatalog` lists both rows for `book_dto_id`
    let cat = AssetPickerCatalog::from_editor(&e);
    let rows_for_book = cat.soft_ref_rows_for(book_dto_id);
    assert_eq!(rows_for_book.len(), 2);
    assert_eq!(rows_for_book[0].0, row0_id);
    assert!(rows_for_book[0].1.contains("Codex of Emberlight"), "unexpected label: {}", rows_for_book[0].1);
    assert_eq!(rows_for_book[0].2, books_col_r);
    assert_eq!(rows_for_book[1].0, row1_id);
    assert!(rows_for_book[1].1.contains("Treatise on Astral Geometry"), "unexpected label: {}", rows_for_book[1].1);

    // Setting `row0.companion_volume` to a non-existent row UUID triggers live `DE008`
    let missing_row_id = AssetUuid::legacy("library/books_core.collection.ron#missing_row");
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(CollectionProp::RowValue { row: row0_id, path: vec![f_companion] }),
        value: PropValue::Str(missing_row_id.to_string()),
        merge: false,
    }));
    assert!(
        e.domains.get::<DataState>().unwrap().diagnostics.iter().any(|d| d.code == "DE008"),
        "expected live DE008 when SoftRef<Book> points to a non-existent row UUID"
    );

    // Setting `row0.companion_volume` = `row1_id` and `row1.companion_volume` = `row0_id` clears `DE008`
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(CollectionProp::RowValue { row: row0_id, path: vec![f_companion] }),
        value: PropValue::Str(row1_id.to_string()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(CollectionProp::RowValue { row: row1_id, path: vec![f_companion] }),
        value: PropValue::Str(row0_id.to_string()),
        merge: false,
    }));
    let de008s: Vec<_> =
        e.domains.get::<DataState>().unwrap().diagnostics.iter().filter(|d| d.code == "DE008").cloned().collect();
    assert!(de008s.is_empty(), "DE008 should clear once SoftRef points to existing row, got: {de008s:#?}");

    e.apply(Action::Save);
    let col_ron = std::fs::read_to_string(dir.path().join(books_col_r.source_rel())).unwrap();
    assert!(
        col_ron.contains("// companion_volume, library/books_core#Treatise on Astral Geometry"),
        "expected D27 comment for row0 -> row1 in {col_ron}"
    );
    assert!(
        col_ron.contains("// companion_volume, library/books_core#Codex of Emberlight"),
        "expected D27 comment for row1 -> row0 in {col_ron}"
    );

    // 4. Verify Rust preview on `library/book` emits `SoftRef<...::LibraryBook>`
    e.apply(Action::from(DataStep::OpenRustPreview(book_dto_r.clone())));
    let rust_doc = e.active_doc::<TextDoc>().expect("Rust preview tab opened");
    let rust_text = rust_doc.text.as_deref().unwrap_or_default();
    assert!(
        rust_text.contains("pub companion_volume: super::SoftRef<super::LibraryBook>,"),
        "expected SoftRef<LibraryBook> in generated Rust preview:\n{rust_text}",
    );

    // 5. Verify `DataStep::OpenCollectionRow` opens `library/books_core` and selects `row1_id`
    e.apply(Action::OpenAsset(book_dto_r.clone()));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::FieldDefault(f_companion)),
        value: PropValue::Str(row1_id.to_string()),
        merge: false,
    }));
    e.apply(Action::Save);
    e.apply(Action::from(DataStep::OpenCollectionRow { collection: books_col_r.clone(), row: row1_id }));
    {
        let col_doc = e.active_doc::<CollectionDoc>().expect("CollectionEditor tab focused by OpenCollectionRow");
        assert_eq!(col_doc.path, "library/books_core");
        assert_eq!(col_doc.selected_row, Some(row1_id));
    }

    // 6. Verify headless Bevy ECS spawns `DataJumpAssetButton` for `SoftRef` in both `DtoEditor` and `CollectionEditor`
    e.apply(Action::OpenAsset(book_dto_r));
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default(), bevy::scene::ScenePlugin, bevy::input::InputPlugin));
    app.init_asset::<Font>().init_asset::<Image>();
    app.insert_resource(e);
    app.add_plugins((EditorPlugins, AreasPlugin, DomainWindowPlugin));

    app.world_mut().spawn_scene(editor_topbar()).unwrap();
    app.world_mut().spawn_scene(right_sidebar()).unwrap();
    app.world_mut().spawn_scene(dto_view()).unwrap();
    app.world_mut().spawn_scene(collection_view()).unwrap();
    app.update();
    app.update();

    {
        let w = app.world_mut();
        let jumps = w.query_filtered::<&DataJumpAssetButton, ()>().iter(w).count();
        assert_eq!(jumps, 1, "expected 1 jump button for SoftRef default in DtoEditor");
    }

    app.world_mut().resource_mut::<Editor>().apply(Action::OpenAsset(books_col_r));
    app.update();
    app.update();

    {
        let w = app.world_mut();
        let jumps = w.query_filtered::<&DataJumpAssetButton, ()>().iter(w).count();
        assert_eq!(jumps, 2, "expected 1 jump button in DtoEditor + 1 in CollectionEditor");
    }
}

#[test]
fn enum_authoring_variant_switching_drafts_and_qsd_bake() {
    let (dir, mut e) = open_temp_project();

    // 1. Create a plain unit enum `library/rarity.enum.ron` (`common`, `rare`, `mythic`).
    let rarity_r = create_asset(&mut e, ENUM_KIND, "library/rarity").unwrap();
    assert_eq!(e.tab().and_then(|t| t.doc().asset()), Some(&rarity_r));
    assert_eq!(e.tab().map(|t| t.doc().view()), Some(ENUM_VIEW));

    for (v_name, v_doc) in [("common", "Standard volume"), ("rare", "Limited printing"), ("mythic", "Vault relic")] {
        e.apply(Action::Tab(TabAction::SetValue {
            key: PropKey::new(EnumProp::DraftVariantName),
            value: PropValue::Str(v_name.into()),
            merge: false,
        }));
        e.apply(Action::from(EnumAction::AddVariant));
        let vid = e.active_doc::<EnumDoc>().unwrap().en.variants.last().unwrap().id;
        e.apply(Action::Tab(TabAction::SetValue {
            key: PropKey::new(EnumProp::VariantDoc(vid)),
            value: PropValue::Str(v_doc.into()),
            merge: false,
        }));
    }

    let (rarity_id, _var_common, var_rare, var_mythic) = {
        let doc = e.active_doc::<EnumDoc>().unwrap();
        assert!(doc.en.is_unit_only());
        assert_eq!(doc.en.variants.len(), 3);
        (doc.en.id, doc.en.variants[0].id, doc.en.variants[1].id, doc.en.variants[2].id)
    };

    // Test variant move + undo + redo + save on `EnumDoc`.
    e.apply(Action::from(EnumAction::MoveVariant { id: var_mythic, delta: -1 }));
    assert_eq!(e.active_doc::<EnumDoc>().unwrap().en.variants[1].id, var_mythic);
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(e.active_doc::<EnumDoc>().unwrap().en.variants[2].id, var_mythic);
    e.apply(Action::Tab(TabAction::Redo));
    assert_eq!(e.active_doc::<EnumDoc>().unwrap().en.variants[1].id, var_mythic);
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(e.active_doc::<EnumDoc>().unwrap().en.variants[2].id, var_mythic);
    e.apply(Action::Save);
    assert!(!e.is_dirty());

    // 2. Create a tagged union `library/binding_effect.enum.ron`:
    //    - `none` (unit variant)
    //    - `ward { power: F32, label: String }`
    //    - `curse { drain_per_sec: F32 }`
    let effect_r = create_asset(&mut e, ENUM_KIND, "library/binding_effect").unwrap();
    for v_name in ["none", "ward", "curse"] {
        e.apply(Action::Tab(TabAction::SetValue {
            key: PropKey::new(EnumProp::DraftVariantName),
            value: PropValue::Str(v_name.into()),
            merge: false,
        }));
        e.apply(Action::from(EnumAction::AddVariant));
    }
    let (effect_id, var_none, var_ward, var_curse) = {
        let doc = e.active_doc::<EnumDoc>().unwrap();
        (doc.en.id, doc.en.variants[0].id, doc.en.variants[1].id, doc.en.variants[2].id)
    };

    // Select `ward` and add 2 payload fields: `power: F32` (min=0, max=100, default=25.0) and `label: String` (default="Aegis")
    e.apply(Action::from(EnumAction::SelectVariant(Some(var_ward))));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(EnumProp::DraftFieldName),
        value: PropValue::Str("power".into()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(EnumProp::DraftFieldType),
        value: PropValue::Str("F32".into()),
        merge: false,
    }));
    e.apply(Action::from(EnumAction::AddField));

    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(EnumProp::DraftFieldName),
        value: PropValue::Str("label".into()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(EnumProp::DraftFieldType),
        value: PropValue::Str("String".into()),
        merge: false,
    }));
    e.apply(Action::from(EnumAction::AddField));

    let (pf_ward_power, pf_ward_label) = {
        let doc = e.active_doc::<EnumDoc>().unwrap();
        let ward = doc.en.variant_by_id(var_ward).unwrap();
        assert_eq!(ward.fields.len(), 2);
        (ward.fields[0].id, ward.fields[1].id)
    };
    e.apply(Action::from(EnumAction::ToggleConstraints(pf_ward_power)));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(EnumProp::FieldMin(pf_ward_power)),
        value: PropValue::Float(0.0),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(EnumProp::FieldMax(pf_ward_power)),
        value: PropValue::Float(100.0),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(EnumProp::FieldDefault(pf_ward_power)),
        value: PropValue::Float(25.0),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(EnumProp::FieldDefault(pf_ward_label)),
        value: PropValue::Str("Aegis".into()),
        merge: false,
    }));

    // Select `curse` and add `drain_per_sec: F32` (default = 4.5)
    e.apply(Action::from(EnumAction::SelectVariant(Some(var_curse))));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(EnumProp::DraftFieldName),
        value: PropValue::Str("drain_per_sec".into()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(EnumProp::DraftFieldType),
        value: PropValue::Str("F32".into()),
        merge: false,
    }));
    e.apply(Action::from(EnumAction::AddField));
    let pf_curse_drain = {
        let doc = e.active_doc::<EnumDoc>().unwrap();
        assert!(!doc.en.is_unit_only());
        doc.en.variant_by_id(var_curse).unwrap().fields[0].id
    };
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(EnumProp::FieldDefault(pf_curse_drain)),
        value: PropValue::Float(4.5),
        merge: false,
    }));
    e.apply(Action::Save);
    assert!(!e.is_dirty());

    let loaded_effect = load_enum(&dir.path().join(effect_r.source_rel())).unwrap();
    assert_eq!(loaded_effect.variants.len(), 3);
    assert_eq!(loaded_effect.variants[1].fields.len(), 2);

    // 3. Create DTO `library/tome.dto.ron` with `title: String` (is_name = true),
    //    `rarity: Enum(library/rarity)`, and `effect: Enum(library/binding_effect)`.
    let tome_dto_r = create_asset(&mut e, DTO_KIND, "library/tome").unwrap();
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftName),
        value: PropValue::Str("title".into()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftType),
        value: PropValue::Str("String".into()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftIsName),
        value: PropValue::Bool(true),
        merge: false,
    }));
    e.apply(Action::from(DtoAction::AddField));

    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftName),
        value: PropValue::Str("rarity".into()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftType),
        value: PropValue::Str("Enum".into()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftEnumTarget),
        value: PropValue::Str(rarity_id.to_string()),
        merge: false,
    }));
    e.apply(Action::from(DtoAction::AddField));

    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftName),
        value: PropValue::Str("effect".into()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftType),
        value: PropValue::Str("Enum".into()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftEnumTarget),
        value: PropValue::Str(effect_id.to_string()),
        merge: false,
    }));
    e.apply(Action::from(DtoAction::AddField));

    let (tome_dto_id, f_title, f_rarity, f_effect) = {
        let doc = e.active_doc::<DtoDoc>().unwrap();
        assert_eq!(doc.dto.fields.len(), 3);
        assert_eq!(doc.dto.fields[1].ty, FieldType::Enum(rarity_id));
        assert_eq!(doc.dto.fields[2].ty, FieldType::Enum(effect_id));
        (doc.dto.id, doc.dto.fields[0].id, doc.dto.fields[1].id, doc.dto.fields[2].id)
    };

    // Set DTO default for `rarity` to `rare`, and for `effect` to `ward { power: 30.0, label: "Aegis" }`
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::FieldDefault(f_rarity)),
        value: PropValue::Str(var_rare.to_string()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::FieldDefault(f_effect)),
        value: PropValue::Str(var_ward.to_string()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::NestedFieldDefault(vec![f_effect, pf_ward_power])),
        value: PropValue::Float(30.0),
        merge: false,
    }));
    e.apply(Action::Save);

    let dto_ron = std::fs::read_to_string(dir.path().join(tome_dto_r.source_rel())).unwrap();
    assert!(dto_ron.contains("// library/rarity"), "expected enum target D27 comment in {dto_ron}");
    assert!(dto_ron.contains("// library/rarity::rare"), "expected enum variant D27 comment in {dto_ron}");
    assert!(dto_ron.contains("// library/binding_effect::ward"), "expected union variant D27 comment in {dto_ron}");
    assert!(dto_ron.contains("F32(30.0), // power"), "expected payload field D27 comment in {dto_ron}");

    // 4. Create Collection `library/tomes.collection.ron` bound to `library/tome`, add row, and test variant switcher
    //    with in-memory draft preservation!
    let tomes_col_r = create_asset(&mut e, COLLECTION_KIND, "library/tomes").unwrap();
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(CollectionProp::DtoSchema),
        value: PropValue::Str(tome_dto_id.to_string()),
        merge: false,
    }));
    e.apply(Action::from(CollectionAction::AddRow));
    let row0_id = e.active_doc::<CollectionDoc>().unwrap().col.rows[0].id;
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(CollectionProp::RowValue { row: row0_id, path: vec![f_title] }),
        value: PropValue::Str("Grimoire of Dawn".into()),
        merge: false,
    }));

    // Edit `ward.power` = 85.0 and `ward.label` = "Sunburst" on `row0`
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(CollectionProp::RowValue { row: row0_id, path: vec![f_effect, pf_ward_power] }),
        value: PropValue::Float(85.0),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(CollectionProp::RowValue { row: row0_id, path: vec![f_effect, pf_ward_label] }),
        value: PropValue::Str("Sunburst".into()),
        merge: false,
    }));

    // Switch `row0.effect` to `curse` -> initializes `curse { drain_per_sec: 4.5 }` and stashes `ward` draft
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(CollectionProp::RowValue { row: row0_id, path: vec![f_effect] }),
        value: PropValue::Str(var_curse.to_string()),
        merge: false,
    }));
    {
        let col_doc = e.active_doc::<CollectionDoc>().unwrap();
        let drain_val = col_doc.get_value_at_path(row0_id, &[f_effect, pf_curse_drain], &col_doc.cached_dtos);
        assert_eq!(drain_val, Some(DataValue::F32(4.5)));
    }
    // Edit `curse.drain_per_sec` = 12.0
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(CollectionProp::RowValue { row: row0_id, path: vec![f_effect, pf_curse_drain] }),
        value: PropValue::Float(12.0),
        merge: false,
    }));

    // Switch `row0.effect` to `none` (unit variant) and then back to `ward` -> restores `power: 85.0, label: "Sunburst"`!
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(CollectionProp::RowValue { row: row0_id, path: vec![f_effect] }),
        value: PropValue::Str(var_none.to_string()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(CollectionProp::RowValue { row: row0_id, path: vec![f_effect] }),
        value: PropValue::Str(var_ward.to_string()),
        merge: false,
    }));
    {
        let col_doc = e.active_doc::<CollectionDoc>().unwrap();
        assert_eq!(
            col_doc.get_value_at_path(row0_id, &[f_effect, pf_ward_power], &col_doc.cached_dtos),
            Some(DataValue::F32(85.0)),
            "switching back to `ward` must restore previously edited `power: 85.0` from session draft"
        );
        assert_eq!(
            col_doc.get_value_at_path(row0_id, &[f_effect, pf_ward_label], &col_doc.cached_dtos),
            Some(DataValue::String("Sunburst".into())),
            "switching back to `ward` must restore previously edited `label: Sunburst` from session draft"
        );
    }

    // Switch back to `curse` -> restores `drain_per_sec: 12.0`!
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(CollectionProp::RowValue { row: row0_id, path: vec![f_effect] }),
        value: PropValue::Str(var_curse.to_string()),
        merge: false,
    }));
    {
        let col_doc = e.active_doc::<CollectionDoc>().unwrap();
        assert_eq!(
            col_doc.get_value_at_path(row0_id, &[f_effect, pf_curse_drain], &col_doc.cached_dtos),
            Some(DataValue::F32(12.0)),
            "switching back to `curse` must restore previously edited `drain_per_sec: 12.0` from session draft"
        );
    }

    // Switch back to `ward` and save
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(CollectionProp::RowValue { row: row0_id, path: vec![f_effect] }),
        value: PropValue::Str(var_ward.to_string()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(CollectionProp::RowValue { row: row0_id, path: vec![f_rarity] }),
        value: PropValue::Str(var_mythic.to_string()),
        merge: false,
    }));
    e.apply(Action::Save);

    assert!(
        e.domains.get::<DataState>().unwrap().diagnostics.is_empty(),
        "unexpected diagnostics: {:?}",
        e.domains.get::<DataState>().unwrap().diagnostics
    );

    let col_ron = std::fs::read_to_string(dir.path().join(tomes_col_r.source_rel())).unwrap();
    assert!(col_ron.contains("// library/rarity::mythic"), "expected unit enum variant D27 comment in {col_ron}");
    assert!(col_ron.contains("// library/binding_effect::ward"), "expected union variant D27 comment in {col_ron}");
    assert!(col_ron.contains("F32(85.0), // power"), "expected payload field D27 comment in {col_ron}");

    // 5. Verify `.collection.qsd` baked binary decodes cleanly via `decode_collection_rows_with_schemas`.
    let qsd_bytes =
        std::fs::read(dir.path().join("assets/quillshade/data/autogen/library/tomes.collection.qsd")).unwrap();
    let st = e.domains.get::<DataState>().unwrap();
    let tome_dto = st.assets.dtos.get("library/tome").unwrap();
    let decoded_rows = data_domain::backend::decode_collection_rows_with_schemas(
        &qsd_bytes,
        tome_dto,
        &st.assets.dtos,
        &st.assets.enums,
        st.current_schema_hash,
    )
    .unwrap();
    assert_eq!(decoded_rows.len(), 1);
    assert_eq!(
        decoded_rows[0].1.get(&f_rarity),
        Some(&DataValue::Enum { variant: var_mythic, fields: std::collections::BTreeMap::new() })
    );
    assert_eq!(
        decoded_rows[0].1.get(&f_effect),
        Some(&DataValue::Enum {
            variant: var_ward,
            fields: std::collections::BTreeMap::from([
                (pf_ward_power, DataValue::F32(85.0)),
                (pf_ward_label, DataValue::String("Sunburst".into())),
            ]),
        })
    );

    // 6. Verify generated Rust preview for `library/rarity.enum.ron` and `library/binding_effect.enum.ron`.
    e.apply(Action::from(DataStep::OpenRustPreview(rarity_r.clone())));
    let rarity_rust = e.active_doc::<TextDoc>().unwrap().text.clone().unwrap();
    assert!(rarity_rust.contains("pub enum LibraryRarity {"), "expected LibraryRarity enum in:\n{rarity_rust}");
    assert!(rarity_rust.contains("#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]"));

    e.apply(Action::from(DataStep::OpenRustPreview(effect_r.clone())));
    let effect_rust = e.active_doc::<TextDoc>().unwrap().text.clone().unwrap();
    assert!(
        effect_rust.contains("pub enum LibraryBindingEffect {"),
        "expected LibraryBindingEffect enum in:\n{effect_rust}"
    );
    assert!(
        effect_rust.contains("Ward {\n        power: f32,\n        label: String,\n    },"),
        "expected Ward variant fields in:\n{effect_rust}"
    );

    // 7. Verify headless Bevy ECS renders `enum_view()`, `dto_view()`, and `collection_view()` with expanded payload sub-forms.
    e.apply(Action::OpenAsset(effect_r));
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default(), bevy::scene::ScenePlugin, bevy::input::InputPlugin));
    app.init_asset::<Font>().init_asset::<Image>();
    app.init_resource::<bevy::input_focus::InputFocus>().init_resource::<bevy::input_focus::InputFocusVisible>();
    app.init_resource::<bevy::feathers::controls::UnitsRegistry>();
    app.insert_resource(bevy::feathers::theme::UiTheme(bevy::feathers::dark_theme::create_dark_theme()));
    app.insert_resource(e);
    app.add_plugins((EditorPlugins, AreasPlugin, DomainWindowPlugin));

    app.world_mut().spawn_scene(editor_topbar()).unwrap();
    app.world_mut().spawn_scene(right_sidebar()).unwrap();
    app.world_mut().spawn_scene(enum_view()).unwrap();
    app.world_mut().spawn_scene(dto_view()).unwrap();
    app.world_mut().spawn_scene(collection_view()).unwrap();
    app.update();
    app.update();

    {
        let w = app.world_mut();
        assert_eq!(w.query_filtered::<Entity, With<EnumTopbar>>().iter(w).count(), 1);
        assert_eq!(w.query_filtered::<Entity, With<EnumSidebar>>().iter(w).count(), 1);
        assert_eq!(w.query_filtered::<Entity, With<EnumViewBody>>().iter(w).count(), 1);
    }

    app.world_mut().resource_mut::<Editor>().apply(Action::OpenAsset(tomes_col_r));
    app.world_mut()
        .resource_mut::<Editor>()
        .apply(Action::from(CollectionAction::ToggleDtoField { path: vec![f_effect] }));
    app.update();
    app.update();

    {
        let w = app.world_mut();
        let summaries = w.query_filtered::<&CollectionDtoSummaryText, ()>().iter(w).count();
        assert_eq!(
            summaries, 1,
            "expected 1 summary toggle button for tagged union field `effect` in CollectionEditor"
        );
    }
}

#[test]
fn option_and_containers_authoring_in_dto_enum_and_collection_with_undo_redo_and_qsd_bake() {
    use data_domain::backend::{path_step_item, path_step_map_key, path_step_map_val, path_step_option_some};

    let (dir, mut e) = open_temp_project();

    // 1. Create unit enum `library/tag_kind.enum.ron` (`lore`, `quest`) for Set/Map keys.
    let _tag_enum_r = create_asset(&mut e, ENUM_KIND, "library/tag_kind").unwrap();
    for v_name in ["lore", "quest"] {
        e.apply(Action::Tab(TabAction::SetValue {
            key: PropKey::new(EnumProp::DraftVariantName),
            value: PropValue::Str(v_name.into()),
            merge: false,
        }));
        e.apply(Action::from(EnumAction::AddVariant));
    }
    let (tag_enum_id, var_lore, var_quest) = {
        let doc = e.active_doc::<EnumDoc>().unwrap();
        (doc.en.id, doc.en.variants[0].id, doc.en.variants[1].id)
    };
    e.apply(Action::Save);

    // 2. Create tagged union `library/chapter_note.enum.ron` with variant `annotated { tags: Array<String> }`
    //    to verify container payload fields in `EnumDoc`.
    let note_enum_r = create_asset(&mut e, ENUM_KIND, "library/chapter_note").unwrap();
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(EnumProp::DraftVariantName),
        value: PropValue::Str("annotated".into()),
        merge: false,
    }));
    e.apply(Action::from(EnumAction::AddVariant));
    let var_annotated = e.active_doc::<EnumDoc>().unwrap().en.variants[0].id;
    let note_enum_id = e.active_doc::<EnumDoc>().unwrap().en.id;
    e.apply(Action::from(EnumAction::SelectVariant(Some(var_annotated))));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(EnumProp::DraftFieldName),
        value: PropValue::Str("notes".into()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(EnumProp::DraftFieldType),
        value: PropValue::Str("Array".into()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(EnumProp::DraftFieldInnerType),
        value: PropValue::Str(FieldType::String.tag().to_string()),
        merge: false,
    }));
    e.apply(Action::from(EnumAction::AddField));
    let pf_notes = e.active_doc::<EnumDoc>().unwrap().en.variants[0].fields[0].id;
    assert_eq!(
        e.active_doc::<EnumDoc>().unwrap().en.variants[0].fields[0].ty,
        FieldType::Array(Box::new(FieldType::String))
    );
    e.apply(Action::from(EnumAction::ContainerAdd { path: vec![pf_notes] }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(EnumProp::NestedFieldDefault(vec![pf_notes, path_step_item(0)])),
        value: PropValue::Str("margin_note".into()),
        merge: false,
    }));
    assert_eq!(
        e.active_doc::<EnumDoc>().unwrap().en.variants[0].fields[0].default,
        DataValue::Array(vec![DataValue::String("margin_note".into())])
    );
    e.apply(Action::Save);

    // 3. Create DTO `library/shelf.dto.ron` with:
    //    - `subtitle: Option<String>`
    //    - `scores: Array<I32>`
    //    - `kinds: Set<Enum(library/tag_kind)>`
    //    - `counts: Map<String, I32>`
    let shelf_dto_r = create_asset(&mut e, DTO_KIND, "library/shelf").unwrap();

    // Add `subtitle: Option<String>`
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftName),
        value: PropValue::Str("subtitle".into()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftType),
        value: PropValue::Str("Option".into()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftInnerType),
        value: PropValue::Str(FieldType::String.tag().to_string()),
        merge: false,
    }));
    e.apply(Action::from(DtoAction::AddField));

    // Add `scores: Array<I32>`
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftName),
        value: PropValue::Str("scores".into()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftType),
        value: PropValue::Str("Array".into()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftInnerType),
        value: PropValue::Str(FieldType::I32.tag().to_string()),
        merge: false,
    }));
    e.apply(Action::from(DtoAction::AddField));

    // Add `kinds: Set<Enum(library/tag_kind)>`
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftName),
        value: PropValue::Str("kinds".into()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftType),
        value: PropValue::Str("Set".into()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftInnerType),
        value: PropValue::Str(FieldType::Enum(AssetUuid::NIL).tag().to_string()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftInnerTarget),
        value: PropValue::Str(tag_enum_id.to_string()),
        merge: false,
    }));
    e.apply(Action::from(DtoAction::AddField));

    // Add `counts: Map<String, I32>`
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftName),
        value: PropValue::Str("counts".into()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftType),
        value: PropValue::Str("Map".into()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftKeyType),
        value: PropValue::Str(FieldType::String.tag().to_string()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftInnerType),
        value: PropValue::Str(FieldType::I32.tag().to_string()),
        merge: false,
    }));
    e.apply(Action::from(DtoAction::AddField));

    // Add `note: Option<Enum(library/chapter_note)>` so `library/chapter_note` is reachable via `Option<Enum>` (no DW104)
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftName),
        value: PropValue::Str("note".into()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftType),
        value: PropValue::Str("Option".into()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftInnerType),
        value: PropValue::Str(FieldType::Enum(AssetUuid::NIL).tag().to_string()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::DraftInnerTarget),
        value: PropValue::Str(note_enum_id.to_string()),
        merge: false,
    }));
    e.apply(Action::from(DtoAction::AddField));

    let (shelf_dto_id, f_subtitle, f_scores, f_kinds, f_counts) = {
        let doc = e.active_doc::<DtoDoc>().unwrap();
        assert_eq!(doc.dto.fields.len(), 5);
        assert_eq!(doc.dto.fields[0].ty, FieldType::Option(Box::new(FieldType::String)));
        assert_eq!(doc.dto.fields[1].ty, FieldType::Array(Box::new(FieldType::I32)));
        assert_eq!(doc.dto.fields[2].ty, FieldType::Set(Box::new(FieldType::Enum(tag_enum_id))));
        assert_eq!(doc.dto.fields[3].ty, FieldType::Map(Box::new(FieldType::String), Box::new(FieldType::I32)));
        assert_eq!(doc.dto.fields[4].ty, FieldType::Option(Box::new(FieldType::Enum(note_enum_id))));
        (doc.dto.id, doc.dto.fields[0].id, doc.dto.fields[1].id, doc.dto.fields[2].id, doc.dto.fields[3].id)
    };

    // Test `DtoAction::SetOptionSome` and `ContainerAdd`/`ContainerMove`/`ContainerRemove` on `DtoDoc` defaults:
    e.apply(Action::from(DtoAction::SetOptionSome { path: vec![f_subtitle], some: true }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::NestedFieldDefault(vec![f_subtitle, path_step_option_some()])),
        value: PropValue::Str("East Wing".into()),
        merge: false,
    }));
    assert_eq!(
        e.active_doc::<DtoDoc>().unwrap().dto.fields[0].default,
        DataValue::Option(Some(Box::new(DataValue::String("East Wing".into()))))
    );

    e.apply(Action::from(DtoAction::ContainerAdd { path: vec![f_scores] }));
    e.apply(Action::from(DtoAction::ContainerAdd { path: vec![f_scores] }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::NestedFieldDefault(vec![f_scores, path_step_item(0)])),
        value: PropValue::Float(10.0),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(DtoProp::NestedFieldDefault(vec![f_scores, path_step_item(1)])),
        value: PropValue::Float(20.0),
        merge: false,
    }));
    e.apply(Action::from(DtoAction::ContainerMove { path: vec![f_scores], index: 1, delta: -1 }));
    assert_eq!(
        e.active_doc::<DtoDoc>().unwrap().dto.fields[1].default,
        DataValue::Array(vec![DataValue::I32(20), DataValue::I32(10)])
    );
    e.apply(Action::from(DtoAction::ContainerRemove { path: vec![f_scores], index: 0 }));
    assert_eq!(e.active_doc::<DtoDoc>().unwrap().dto.fields[1].default, DataValue::Array(vec![DataValue::I32(10)]));
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(
        e.active_doc::<DtoDoc>().unwrap().dto.fields[1].default,
        DataValue::Array(vec![DataValue::I32(20), DataValue::I32(10)])
    );
    e.apply(Action::Save);

    // 4. Create collection `library/shelves.collection.ron` bound to `library/shelf` and exercise row container ops + undo/redo:
    let shelves_col_r = create_asset(&mut e, COLLECTION_KIND, "library/shelves").unwrap();
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(CollectionProp::DtoSchema),
        value: PropValue::Str(shelf_dto_id.to_string()),
        merge: false,
    }));
    e.apply(Action::from(CollectionAction::AddRow));
    let row0_id = e.active_doc::<CollectionDoc>().unwrap().col.rows[0].id;

    // Verify `row0` starts with the DTO default `Some("East Wing")`, then toggle to `None`, then `Some` (`""`), and edit to `"Vault"`
    assert_eq!(
        e.active_doc::<CollectionDoc>().unwrap().col.rows[0].values.get(&f_subtitle),
        Some(&DataValue::Option(Some(Box::new(DataValue::String("East Wing".into())))))
    );
    e.apply(Action::from(CollectionAction::SetOptionSome { row: row0_id, path: vec![f_subtitle], some: false }));
    assert_eq!(
        e.active_doc::<CollectionDoc>().unwrap().col.rows[0].values.get(&f_subtitle),
        Some(&DataValue::Option(None))
    );
    e.apply(Action::from(CollectionAction::SetOptionSome { row: row0_id, path: vec![f_subtitle], some: true }));
    assert_eq!(
        e.active_doc::<CollectionDoc>().unwrap().col.rows[0].values.get(&f_subtitle),
        Some(&DataValue::Option(Some(Box::new(DataValue::String(String::new())))))
    );
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(CollectionProp::RowValue { row: row0_id, path: vec![f_subtitle, path_step_option_some()] }),
        value: PropValue::Str("Vault".into()),
        merge: false,
    }));

    // Add Set item (`var_quest`) and Map entry (`"tomes" -> 42`) on `row0`
    e.apply(Action::from(CollectionAction::ContainerAdd { row: row0_id, path: vec![f_kinds] }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(CollectionProp::RowValue { row: row0_id, path: vec![f_kinds, path_step_item(0)] }),
        value: PropValue::Str(var_quest.to_string()),
        merge: false,
    }));
    e.apply(Action::from(CollectionAction::ContainerAdd { row: row0_id, path: vec![f_kinds] }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(CollectionProp::RowValue { row: row0_id, path: vec![f_kinds, path_step_item(1)] }),
        value: PropValue::Str(var_lore.to_string()),
        merge: false,
    }));

    e.apply(Action::from(CollectionAction::ContainerAdd { row: row0_id, path: vec![f_counts] }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(CollectionProp::RowValue { row: row0_id, path: vec![f_counts, path_step_map_key(0)] }),
        value: PropValue::Str("tomes".into()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: PropKey::new(CollectionProp::RowValue { row: row0_id, path: vec![f_counts, path_step_map_val(0)] }),
        value: PropValue::Float(42.0),
        merge: false,
    }));

    // Undo / Redo on `CollectionDoc`
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(
        e.active_doc::<CollectionDoc>().unwrap().col.rows[0].values.get(&f_counts),
        Some(&DataValue::Map(vec![(DataValue::String("tomes".into()), DataValue::I32(0))]))
    );
    e.apply(Action::Tab(TabAction::Redo));
    assert_eq!(
        e.active_doc::<CollectionDoc>().unwrap().col.rows[0].values.get(&f_counts),
        Some(&DataValue::Map(vec![(DataValue::String("tomes".into()), DataValue::I32(42))]))
    );

    e.apply(Action::Save);
    assert!(
        e.domains.get::<DataState>().unwrap().diagnostics.is_empty(),
        "unexpected diagnostics: {:?}",
        e.domains.get::<DataState>().unwrap().diagnostics
    );

    // 5. Verify `.collection.qsd` baked binary decodes cleanly via `decode_collection_rows_with_schemas`.
    let qsd_bytes =
        std::fs::read(dir.path().join("assets/quillshade/data/autogen/library/shelves.collection.qsd")).unwrap();
    let st = e.domains.get::<DataState>().unwrap();
    let shelf_dto = st.assets.dtos.get("library/shelf").unwrap();
    let decoded_rows = data_domain::backend::decode_collection_rows_with_schemas(
        &qsd_bytes,
        shelf_dto,
        &st.assets.dtos,
        &st.assets.enums,
        st.current_schema_hash,
    )
    .unwrap();
    assert_eq!(decoded_rows.len(), 1);
    assert_eq!(
        decoded_rows[0].1.get(&f_subtitle),
        Some(&DataValue::Option(Some(Box::new(DataValue::String("Vault".into())))))
    );
    assert_eq!(decoded_rows[0].1.get(&f_scores), Some(&DataValue::Array(vec![DataValue::I32(20), DataValue::I32(10)])));
    assert_eq!(
        decoded_rows[0].1.get(&f_kinds),
        Some(&DataValue::Set(vec![
            DataValue::Enum { variant: var_lore, fields: std::collections::BTreeMap::new() },
            DataValue::Enum { variant: var_quest, fields: std::collections::BTreeMap::new() },
        ]))
    );
    assert_eq!(
        decoded_rows[0].1.get(&f_counts),
        Some(&DataValue::Map(vec![(DataValue::String("tomes".into()), DataValue::I32(42))]))
    );

    // 6. Verify headless Bevy ECS renders `dto_view()`, `enum_view()`, and `collection_view()` with expanded containers.
    e.apply(Action::OpenAsset(shelf_dto_r));
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default(), bevy::scene::ScenePlugin, bevy::input::InputPlugin));
    app.init_asset::<Font>().init_asset::<Image>();
    app.init_resource::<bevy::input_focus::InputFocus>().init_resource::<bevy::input_focus::InputFocusVisible>();
    app.init_resource::<bevy::feathers::controls::UnitsRegistry>();
    app.insert_resource(bevy::feathers::theme::UiTheme(bevy::feathers::dark_theme::create_dark_theme()));
    app.insert_resource(e);
    app.add_plugins((EditorPlugins, AreasPlugin, DomainWindowPlugin));

    app.world_mut().spawn_scene(editor_topbar()).unwrap();
    app.world_mut().spawn_scene(right_sidebar()).unwrap();
    app.world_mut().spawn_scene(dto_view()).unwrap();
    app.world_mut().spawn_scene(enum_view()).unwrap();
    app.world_mut().spawn_scene(collection_view()).unwrap();
    app.update();
    app.update();

    app.world_mut().resource_mut::<Editor>().apply(Action::OpenAsset(note_enum_r));
    app.update();
    app.update();

    app.world_mut().resource_mut::<Editor>().apply(Action::OpenAsset(shelves_col_r));
    app.update();
    app.update();
}
