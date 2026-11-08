//! The Data domain's headless backend (ADR 0049, techdoc §5–§7): Bevy-free models ([`model`]), RON I/O with D27
//! reference comments ([`ron_io`]), validation (`DE001`–`DE006`, `DE008`, `DW101`–`DW104`, [`validate`]), `.qsd` binary
//! baking and `DATA_SCHEMA_HASH` ([`bake`]), Rust codegen ([`codegen`]), [`DataBackend`] (`DomainBackend`), and
//! [`DataMigrator`] (`DomainMigrator`).

use std::collections::{BTreeMap, BTreeSet};

use project_core::files::{OWNED_DIR_MARKER, owned_binary_dir};
use project_core::marker::{civil_today, header, insert_date};
use project_core::regen::{DomainBackend, DomainOutput, DomainPlan, OutFile, OutRoot};
use project_core::umbrella::CrateContribution;
use project_core::{AssetIndex, Diagnostic, DomainId, DomainIndex, Project};

pub mod bake;
pub mod codegen;
pub mod migrate;
pub mod model;
pub mod ron_io;
pub mod validate;

pub use bake::{
    DataBakeResolver, DecodedRow, QSD_HEADER_LEN, QSD_MAGIC, QSD_VERSION, QsdError, QsdHeader, bake_collection,
    bake_collection_with_context, bake_collection_with_dtos, bake_collection_with_schemas,
    bake_collection_with_schemas_and_context, compute_dto_schema_hash_with_schemas, compute_schema_hash,
    compute_schema_hash_with_dtos, compute_schema_hash_with_enums, decode_collection_rows,
    decode_collection_rows_with_dtos, decode_collection_rows_with_schemas, parse_qsd_header, verify_qsd_header,
};
pub use codegen::{
    collection_module_ident, collection_rust_rel_path, dto_module_ident, dto_rust_rel_path, emit_collection_file,
    emit_collections_mod, emit_collections_mod_with_skipped, emit_data_mod, emit_data_mod_with_context,
    emit_data_mod_with_schemas_and_context, emit_data_mod_with_skipped, emit_dto_file, emit_dto_file_with_context,
    emit_dto_file_with_dtos, emit_dto_file_with_schemas, emit_dto_file_with_schemas_and_context, emit_dtos_mod,
    emit_enum_file, emit_enum_file_with_schemas, emit_enum_file_with_schemas_and_context, emit_enums_mod,
    enum_module_ident, enum_rust_rel_path, option_needs_box, rust_field_type, rust_field_type_with_context,
    rust_field_type_with_dtos, rust_field_type_with_schemas, rust_field_type_with_schemas_and_context, wire_field_type,
    wire_field_type_with_dtos, wire_field_type_with_schemas,
};
pub use migrate::DataMigrator;
pub use model::{
    COLLECTION_FORMAT_VERSION, COLLECTION_SUFFIX, CURVE_SUFFIX, CollectionAsset, DTO_FORMAT_VERSION, DTO_SUFFIX,
    DataValue, DtoAsset, ENUM_FORMAT_VERSION, ENUM_SUFFIX, EnumAsset, EnumVariant, Field, FieldSettings, FieldType,
    GameAssetKind, HasFieldValues, MAX_NESTING_DEPTH, PathStep, Row, ToShortViewString, collection_ref, curve_ref,
    decode_path_step, dto_ref, ensure_nested_value_mut, ensure_nested_value_with_schemas_mut, enum_ref,
    format_color_hex, format_compact_f64, linear_channel_to_srgb_u8, path_step_item, path_step_map_key,
    path_step_map_val, path_step_option_some, to_pascal_case, to_upper_snake_case,
};
pub use ron_io::{
    collection_text, collection_text_with_dtos, collection_text_with_index, collection_text_with_schemas,
    collection_text_with_schemas_and_index, dto_text, dto_text_with_dtos, dto_text_with_index, dto_text_with_schemas,
    dto_text_with_schemas_and_index, enum_text, enum_text_with_index, enum_text_with_schemas, load_collection,
    load_curve, load_dto, load_enum, parse_collection, parse_dto, parse_enum, save_collection,
    save_collection_with_dtos, save_collection_with_index, save_collection_with_schemas,
    save_collection_with_schemas_and_index, save_curve, save_dto, save_dto_with_dtos, save_dto_with_index,
    save_dto_with_schemas, save_dto_with_schemas_and_index, save_enum, save_enum_with_index, save_enum_with_schemas,
    set_asset_uuid,
};
pub use validate::{DataAssets, ValidationReport, is_rust_keyword, scan, scan_root};

/// The Data domain (`quillshade/data/`, `assets/quillshade/data/`, `crates/quillshade/src/data/`).
pub const DATA: DomainId = DomainId::new("data");

/// Bevy features required by the generated `quillshade::data` runtime module.
pub const DATA_BEVY_FEATURES: &[&str] = &["std", "bevy_asset", "bevy_color", "bevy_image", "bevy_log"];

/// The Data module's contribution to the `crates/quillshade` umbrella crate (ADR 0049 §7).
pub const DATA_CRATE: CrateContribution = CrateContribution {
    domain: DATA,
    plugin: "DataDomainPlugin",
    loader: Some("BakedDataLoader"),
    bevy_features: DATA_BEVY_FEATURES,
    dependencies: &[
        ("serde", r#"{ version = "1", features = ["derive"] }"#),
        ("postcard", r#"{ version = "1", default-features = false, features = ["alloc"] }"#),
    ],
};

/// The Data domain as a [`DomainBackend`]: generates `crates/quillshade/src/data/**` and
/// `assets/quillshade/data/autogen/<path>.collection.qsd`.
#[derive(Debug, Clone, Copy, Default)]
pub struct DataBackend;

impl DomainBackend for DataBackend {
    fn id(&self) -> DomainId {
        DATA
    }

    fn crate_contribution(&self) -> CrateContribution {
        DATA_CRATE
    }

    fn index(&self, project: &Project) -> DomainIndex {
        DataAssets::scan(project).index_part()
    }

    fn plan(&self, project: &Project, index: &AssetIndex, _compile: bool) -> DomainPlan {
        let assets = DataAssets::scan(project);
        plan_assets(project, &assets, index)
    }
}

/// Validates `assets` against `index` and builds the Data domain's [`DomainPlan`].
pub fn plan_assets(project: &Project, assets: &DataAssets, index: &AssetIndex) -> DomainPlan {
    let report = assets.validate_with_project(Some(project), index);
    let mut diagnostics = report.diagnostics;

    // Collect asset `source_rel` paths that have any asset-level error (excluding row-level errors which only skip the
    // offending row per D5 / techdoc §5.2).
    let mut failed_assets: BTreeSet<String> = BTreeSet::new();
    for d in diagnostics.iter().filter(|d| d.is_error()) {
        let is_row_only = d.location.as_deref().is_some_and(|loc| loc.starts_with("row:"));
        if !is_row_only && let Some(asset) = &d.asset {
            failed_assets.insert(asset.clone());
        }
    }

    let mut valid_dtos: BTreeMap<String, DtoAsset> = assets
        .dtos
        .iter()
        .filter(|(p, _)| !failed_assets.contains(&dto_ref(p).source_rel()))
        .map(|(p, d)| (p.clone(), d.clone()))
        .collect();

    let mut valid_enums: BTreeMap<String, EnumAsset> = assets
        .enums
        .iter()
        .filter(|(p, _)| !failed_assets.contains(&enum_ref(p).source_rel()))
        .map(|(p, e)| (p.clone(), e.clone()))
        .collect();

    // Iteratively prune any DTO or Enum that references a missing or broken child DTO or Enum.
    loop {
        let before = valid_dtos.len() + valid_enums.len();
        let valid_dto_paths: BTreeSet<String> = valid_dtos.keys().cloned().collect();
        let valid_enum_paths: BTreeSet<String> = valid_enums.keys().cloned().collect();
        fn type_targets_valid(
            ty: &FieldType,
            assets: &DataAssets,
            index: &AssetIndex,
            valid_dto_paths: &BTreeSet<String>,
            valid_enum_paths: &BTreeSet<String>,
        ) -> bool {
            match ty {
                FieldType::Dto(target_uuid) | FieldType::SoftRef(target_uuid) => assets
                    .resolve_dto(*target_uuid, index)
                    .is_some_and(|(target_path, _)| valid_dto_paths.contains(target_path)),
                FieldType::Enum(target_uuid) => assets
                    .resolve_enum(*target_uuid, index)
                    .is_some_and(|(target_path, _)| valid_enum_paths.contains(target_path)),
                FieldType::Option(inner) | FieldType::Array(inner) | FieldType::Set(inner) => {
                    type_targets_valid(inner, assets, index, valid_dto_paths, valid_enum_paths)
                }
                FieldType::Map(k, v) => {
                    type_targets_valid(k, assets, index, valid_dto_paths, valid_enum_paths)
                        && type_targets_valid(v, assets, index, valid_dto_paths, valid_enum_paths)
                }
                _ => true,
            }
        }
        let field_targets_valid =
            |f: &Field| type_targets_valid(&f.ty, assets, index, &valid_dto_paths, &valid_enum_paths);
        valid_dtos.retain(|_, dto| dto.fields.iter().all(&field_targets_valid));
        valid_enums.retain(|_, en| en.variants.iter().all(|v| v.fields.iter().all(&field_targets_valid)));
        if valid_dtos.len() + valid_enums.len() == before {
            break;
        }
    }

    let valid_collections: BTreeMap<String, CollectionAsset> = assets
        .collections
        .iter()
        .filter(|(p, _)| !failed_assets.contains(&collection_ref(p).source_rel()))
        .filter(|(_, col)| {
            assets.resolve_dto(col.dto, index).is_some_and(|(dto_path, _)| valid_dtos.contains_key(dto_path))
        })
        .map(|(p, c)| (p.clone(), c.clone()))
        .collect();

    let schema_hash = compute_schema_hash_with_enums(&valid_dtos, &valid_enums);
    let date = project.settings.header_date.then(civil_today);

    let module_dir = project.module_dir(DATA);
    let autogen_dir = project.output_dir(DATA).join("autogen");

    let mut files = Vec::new();
    let mut mod_text = codegen::emit_data_mod_with_schemas_and_context(
        &valid_dtos,
        &valid_enums,
        &valid_collections,
        schema_hash,
        &report.skipped_rows,
        Some(project),
    );
    if let Some(d) = &date {
        insert_date(&mut mod_text, d);
    }
    files.push(OutFile { path: module_dir.join("mod.rs"), bytes: mod_text.into_bytes(), owner: None });

    let mut dtos_mod_text = codegen::emit_dtos_mod(&valid_dtos);
    if let Some(d) = &date {
        insert_date(&mut dtos_mod_text, d);
    }
    files.push(OutFile { path: module_dir.join("dtos/mod.rs"), bytes: dtos_mod_text.into_bytes(), owner: None });

    for (dto_path, dto) in &valid_dtos {
        let mut dto_text = codegen::emit_dto_file_with_schemas_and_context(
            dto_path,
            dto,
            &valid_dtos,
            &valid_enums,
            Some(project),
            Some(index),
        );
        if let Some(d) = &date {
            insert_date(&mut dto_text, d);
        }
        files.push(OutFile {
            path: module_dir.join(codegen::dto_rust_rel_path(dto_path)),
            bytes: dto_text.into_bytes(),
            owner: Some(dto_ref(dto_path).source_rel()),
        });
    }

    let mut enums_mod_text = codegen::emit_enums_mod(&valid_enums);
    if let Some(d) = &date {
        insert_date(&mut enums_mod_text, d);
    }
    files.push(OutFile { path: module_dir.join("enums/mod.rs"), bytes: enums_mod_text.into_bytes(), owner: None });

    for (enum_path, en) in &valid_enums {
        let mut enum_text = codegen::emit_enum_file_with_schemas_and_context(
            enum_path,
            en,
            &valid_dtos,
            &valid_enums,
            Some(project),
            Some(index),
        );
        if let Some(d) = &date {
            insert_date(&mut enum_text, d);
        }
        files.push(OutFile {
            path: module_dir.join(codegen::enum_rust_rel_path(enum_path)),
            bytes: enum_text.into_bytes(),
            owner: Some(enum_ref(enum_path).source_rel()),
        });
    }

    let mut collections_mod_text =
        codegen::emit_collections_mod_with_skipped(&valid_dtos, &valid_collections, &report.skipped_rows);
    if let Some(d) = &date {
        insert_date(&mut collections_mod_text, d);
    }
    files.push(OutFile {
        path: module_dir.join("collections/mod.rs"),
        bytes: collections_mod_text.into_bytes(),
        owner: None,
    });

    for (col_path, col) in &valid_collections {
        let Some((dto_path, _)) = assets.resolve_dto(col.dto, index) else { continue };
        let skipped = report.skipped_rows.iter().filter_map(|(p, idx)| (p == col_path).then_some(*idx)).collect();
        let mut col_text = codegen::emit_collection_file(col_path, col, dto_path, &skipped);
        if let Some(d) = &date {
            insert_date(&mut col_text, d);
        }
        files.push(OutFile {
            path: module_dir.join(codegen::collection_rust_rel_path(col_path)),
            bytes: col_text.into_bytes(),
            owner: Some(collection_ref(col_path).source_rel()),
        });
    }

    if !valid_collections.is_empty() {
        if !owned_binary_dir(&autogen_dir) {
            diagnostics.push(
                Diagnostic::error(
                    "E008",
                    format!(
                        "`{}` exists without `{OWNED_DIR_MARKER}`; not writing baked `.qsd` files",
                        project.rel(&autogen_dir)
                    ),
                )
                .in_domain(DATA),
            );
        } else {
            let mut marker_text = format!("{}\n", header("#", None));
            if let Some(d) = &date {
                insert_date(&mut marker_text, d);
            }
            files.push(OutFile {
                path: autogen_dir.join(OWNED_DIR_MARKER),
                bytes: marker_text.into_bytes(),
                owner: None,
            });
            for (col_path, col) in &valid_collections {
                let Some((dto_path, _)) = assets.resolve_dto(col.dto, index) else { continue };
                let Some(dto) = valid_dtos.get(dto_path) else { continue };
                let skipped =
                    report.skipped_rows.iter().filter_map(|(p, idx)| (p == col_path).then_some(*idx)).collect();
                match bake_collection_with_schemas_and_context(
                    col,
                    dto,
                    &valid_dtos,
                    &valid_enums,
                    Some(project),
                    Some(index),
                    schema_hash,
                    &skipped,
                ) {
                    Ok(bytes) => {
                        files.push(OutFile {
                            path: autogen_dir.join(format!("{col_path}.collection.qsd")),
                            bytes,
                            owner: Some(collection_ref(col_path).source_rel()),
                        });
                    }
                    Err(err) => {
                        diagnostics.push(
                            Diagnostic::error("DE005", err)
                                .in_domain(DATA)
                                .in_asset(collection_ref(col_path).source_rel()),
                        );
                    }
                }
            }
        }
    }

    let mut roots = vec![OutRoot { dir: module_dir, binary: false }];
    if owned_binary_dir(&autogen_dir) {
        roots.push(OutRoot { dir: autogen_dir, binary: true });
    }

    DomainPlan {
        diagnostics,
        output: DomainOutput { domain: DATA, files, keep: BTreeSet::new(), roots, orphan_scan: true },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::model::{Field, FieldSettings, FieldType, Row};
    use project_core::AssetUuid;

    fn test_uuid(n: u128) -> AssetUuid {
        AssetUuid::from_u128(0x0196_3a10_0000_7000_8000_0000_0000_0000 | n)
    }

    #[test]
    fn data_backend_index_registers_sub_items_and_collection_to_dto_refs() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[workspace]\nmembers = [\"crates/*\"]\n").unwrap();
        std::fs::create_dir_all(dir.path().join("assets")).unwrap();
        let project = Project::create(dir.path()).unwrap();

        let dto_id = test_uuid(1);
        let f_title_id = test_uuid(101);
        let mut dto = DtoAsset {
            format_version: DTO_FORMAT_VERSION,
            id: dto_id,
            doc: "Book".into(),
            fields: vec![Field {
                id: f_title_id,
                name: "title".into(),
                ty: FieldType::LocalizedString,
                category: "Identity".into(),
                is_name: true,
                default: DataValue::LocalizedString("Book".into()),
                settings: FieldSettings::default(),
            }],
        };
        let dto_path = project.domain_dir(DATA).join("library/book.dto.ron");
        save_dto(&dto_path, &mut dto).unwrap();

        let col_id = test_uuid(2);
        let row0_id = test_uuid(201);
        let mut col = CollectionAsset {
            format_version: COLLECTION_FORMAT_VERSION,
            id: col_id,
            dto: dto_id,
            rows: vec![Row {
                id: row0_id,
                values: BTreeMap::from([(f_title_id, DataValue::LocalizedString("The Shader Alchemist".into()))]),
            }],
        };
        let col_path = project.domain_dir(DATA).join("library/books_core.collection.ron");
        save_collection(&col_path, &mut col, Some("library/book"), Some(&dto)).unwrap();

        let part = DataBackend.index(&project);
        assert_eq!(part.domain, DATA);
        assert_eq!(part.assets.len(), 2);

        let index = AssetIndex::build(vec![part]);
        let dto_entry = index.get(dto_id).expect("dto in index");
        assert_eq!(dto_entry.asset, dto_ref("library/book"));
        assert_eq!(dto_entry.sub_items.len(), 1);
        assert_eq!(dto_entry.sub_items[0].uuid, Some(f_title_id));
        assert_eq!(dto_entry.sub_items[0].label, "title");

        let col_entry = index.get(col_id).expect("collection in index");
        assert_eq!(col_entry.asset, collection_ref("library/books_core"));
        assert_eq!(col_entry.sub_items.len(), 1);
        assert_eq!(col_entry.sub_items[0].uuid, Some(row0_id));
        assert_eq!(col_entry.sub_items[0].label, "The Shader Alchemist");

        let refs_to_dto: Vec<_> = index.refs_to(dto_id).collect();
        assert_eq!(refs_to_dto, vec![&collection_ref("library/books_core")]);

        // Plan and commit through `project_core::regen::run`.
        let report = project_core::regen::run(&project, &[&DataBackend], &[], false, false).unwrap();
        assert!(!report.has_errors(), "{:?}", report.diagnostics);
        let qsd_path = project.output_dir(DATA).join("autogen/library/books_core.collection.qsd");
        assert!(qsd_path.is_file());
        let qsd_bytes = std::fs::read(&qsd_path).unwrap();
        let decoded = decode_collection_rows(
            &qsd_bytes,
            &dto,
            compute_schema_hash(&BTreeMap::from([("library/book".into(), dto.clone())])),
        )
        .unwrap();
        assert_eq!(decoded.len(), 1);
        assert_eq!(decoded[0].0, row0_id);
    }
}
