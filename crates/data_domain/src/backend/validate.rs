//! Scanning and validation of `.dto.ron`, `.enum.ron`, and `.collection.ron` assets (ADR 0049 §5, techdoc §7.6):
//! diagnostic codes `DE001`–`DE006`, `DE008`–`DE009`, and `DW101`–`DW104`, including project-wide `Row.id` uniqueness
//! tie-breaking (D5) and by-value nested DTO/Enum cycle (`DE003`) and depth (`DE009`) checks.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use project_core::asset_index::{DomainIndex, IndexedAsset, IndexedRef, SubItem};
use project_core::ident::{check_ident, check_path};
use project_core::layout::QUILLSHADE_DIR;
use project_core::{AssetIndex, AssetRef, AssetUuid, Diagnostic, DomainId, Project};

use super::DATA;
use super::model::{
    COLLECTION_SUFFIX, CURVE_SUFFIX, CollectionAsset, DTO_SUFFIX, DataValue, DtoAsset, ENUM_SUFFIX, EnumAsset, Field,
    FieldType, MAX_NESTING_DEPTH, collection_ref, curve_ref, dto_ref, enum_ref, to_pascal_case,
};
use super::ron_io::{load_collection, load_curve, load_dto, load_enum};

/// Rust strict and reserved keywords forbidden as DTO/Enum field or variant names (`DE003`).
const RUST_KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern", "false", "fn",
    "for", "gen", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "self",
    "Self", "static", "struct", "super", "trait", "true", "type", "unsafe", "use", "where", "while", "abstract",
    "become", "box", "do", "final", "macro", "override", "priv", "try", "typeof", "unsized", "virtual", "yield",
];

/// Whether `ident` is a reserved Rust keyword.
pub fn is_rust_keyword(ident: &str) -> bool {
    RUST_KEYWORDS.contains(&ident)
}

/// Scans `quillshade/data/**` for `.dto.ron`, `.enum.ron`, `.collection.ron`, and `.curve.ron` files in path order,
/// skipping hidden files and folders (those whose name starts with `.`).
pub fn scan(project: &Project) -> Vec<AssetRef> {
    scan_root(&project.root)
}

/// [`scan`] rooted at `root` (works on any project version, used by [`super::migrate::DataMigrator`]).
pub fn scan_root(root: &Path) -> Vec<AssetRef> {
    let dir = root.join(QUILLSHADE_DIR).join(DATA.as_str());
    let mut out: Vec<AssetRef> = project_core::files::walk_files(&dir)
        .into_iter()
        .filter(|f| f.strip_prefix(&dir).is_ok_and(|rel| !rel.iter().any(|c| c.to_string_lossy().starts_with('.'))))
        .filter_map(|f| {
            let rel = f.strip_prefix(root).ok()?.to_string_lossy().replace('\\', "/");
            project_core::asset_ref::parse(&rel, DATA, &[DTO_SUFFIX, ENUM_SUFFIX, COLLECTION_SUFFIX, CURVE_SUFFIX])
        })
        .collect();
    out.sort();
    out
}

/// All scanned Data domain assets of a project (`quillshade/data/**`), keyed by domain-relative path without suffix.
#[derive(Debug, Clone, Default)]
pub struct DataAssets {
    pub dtos: BTreeMap<String, DtoAsset>,
    pub enums: BTreeMap<String, EnumAsset>,
    pub collections: BTreeMap<String, CollectionAsset>,
    pub curves: BTreeMap<String, curve_kit::Curve>,
    pub load_diagnostics: Vec<Diagnostic>,
}

/// Result of [`DataAssets::validate`].
#[derive(Debug, Clone, Default)]
pub struct ValidationReport {
    pub diagnostics: Vec<Diagnostic>,
    /// `(collection_path, row_index)` pairs excluded from `.qsd` baking due to row-level errors (`DE001`, `DE002`,
    /// `DE005`, `DE006`).
    pub skipped_rows: BTreeSet<(String, usize)>,
}

impl ValidationReport {
    /// Whether any diagnostic has [`project_core::Level::Error`].
    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(Diagnostic::is_error)
    }

    /// Returns the set of row indices in `collection_path` that must be skipped during `.qsd` baking.
    pub fn skipped_rows_for(&self, collection_path: &str) -> BTreeSet<usize> {
        self.skipped_rows.iter().filter_map(|(path, idx)| (path == collection_path).then_some(*idx)).collect()
    }
}

impl DataAssets {
    /// Scans and loads all `.dto.ron`, `.enum.ron`, `.collection.ron`, and `.curve.ron` files in `project`.
    pub fn scan(project: &Project) -> Self {
        let mut assets = DataAssets::default();
        for r in scan(project) {
            let file = project.root.join(r.source_rel());
            if let Err(reason) = check_path(&r.path) {
                assets.load_diagnostics.push(
                    Diagnostic::error("DE003", format!("invalid asset path `{}`: {reason}", r.path))
                        .in_domain(DATA)
                        .in_asset(r.source_rel()),
                );
            }
            match r.suffix {
                DTO_SUFFIX => match load_dto(&file) {
                    Ok(dto) => {
                        assets.dtos.insert(r.path.clone(), dto);
                    }
                    Err(e) => {
                        assets.load_diagnostics.push(
                            Diagnostic::error("DE001", format!("cannot load: {e}"))
                                .in_domain(DATA)
                                .in_asset(r.source_rel()),
                        );
                    }
                },
                ENUM_SUFFIX => match load_enum(&file) {
                    Ok(en) => {
                        assets.enums.insert(r.path.clone(), en);
                    }
                    Err(e) => {
                        assets.load_diagnostics.push(
                            Diagnostic::error("DE001", format!("cannot load: {e}"))
                                .in_domain(DATA)
                                .in_asset(r.source_rel()),
                        );
                    }
                },
                COLLECTION_SUFFIX => match load_collection(&file) {
                    Ok(col) => {
                        assets.collections.insert(r.path.clone(), col);
                    }
                    Err(e) => {
                        assets.load_diagnostics.push(
                            Diagnostic::error("DE001", format!("cannot load: {e}"))
                                .in_domain(DATA)
                                .in_asset(r.source_rel()),
                        );
                    }
                },
                CURVE_SUFFIX => match load_curve(&file) {
                    Ok(mut curve) => {
                        if let Some(stem) = r.path.rsplit('/').next() {
                            curve.name = stem.to_string();
                        }
                        assets.curves.insert(r.path.clone(), curve);
                    }
                    Err(e) => {
                        assets.load_diagnostics.push(
                            Diagnostic::error("DE001", format!("cannot load: {e}"))
                                .in_domain(DATA)
                                .in_asset(r.source_rel()),
                        );
                    }
                },
                _ => {}
            }
        }
        assets
    }

    /// Alias of [`DataAssets::scan`].
    pub fn load(project: &Project) -> Self {
        Self::scan(project)
    }

    /// Builds the Data domain's [`DomainIndex`] contribution for [`AssetIndex`] (ADR 0049 §4–§5):
    /// registers `SubItem::with_uuid` for every DTO field, every Enum variant and variant field, and every collection
    /// row, plus `IndexedRef`s from each DTO/Enum's `FieldType::Dto` / `FieldType::Enum` / `FieldType::SoftRef` fields,
    /// from each collection to its target DTO UUID, and from any direct cross-domain asset reference values (`Curve`,
    /// `Gradient` with slot sub-item, `Material`, `Texture`, `LightEnvironment`) in defaults and collection row values.
    pub fn index_part(&self) -> DomainIndex {
        let mut assets = Vec::new();
        for (path, dto) in &self.dtos {
            let sub_items = dto
                .fields
                .iter()
                .filter(|f| !f.id.is_nil())
                .map(|f| SubItem::with_uuid(f.id, f.name.clone()))
                .collect();
            let mut refs = Vec::new();
            for f in &dto.fields {
                collect_type_refs(&f.ty, &f.name, &mut refs);
                collect_value_refs(&f.default, &f.name, &mut refs);
            }
            assets.push(IndexedAsset { asset: dto_ref(path), id: Ok(dto.id), sub_items, refs });
        }
        for (path, en) in &self.enums {
            let mut sub_items = Vec::new();
            let mut refs = Vec::new();
            for variant in &en.variants {
                if !variant.id.is_nil() {
                    sub_items.push(SubItem::with_uuid(variant.id, variant.name.clone()));
                }
                for f in &variant.fields {
                    let loc = format!("{}.{}", variant.name, f.name);
                    if !f.id.is_nil() {
                        sub_items.push(SubItem::with_uuid(f.id, loc.clone()));
                    }
                    collect_type_refs(&f.ty, &loc, &mut refs);
                    collect_value_refs(&f.default, &loc, &mut refs);
                }
            }
            assets.push(IndexedAsset { asset: enum_ref(path), id: Ok(en.id), sub_items, refs });
        }
        for (path, col) in &self.collections {
            let dto = self.dtos.values().find(|d| !d.id.is_nil() && d.id == col.dto);
            let sub_items = col
                .rows
                .iter()
                .enumerate()
                .filter(|(_, r)| !r.id.is_nil())
                .map(|(i, r)| {
                    let label = dto
                        .map(|d| r.short_view_string_with_schemas(d, &self.dtos, &self.enums))
                        .filter(|s| !s.is_empty() && s != "—" && s != "row")
                        .unwrap_or_else(|| format!("row#{i}"));
                    SubItem::with_uuid(r.id, label)
                })
                .collect();
            let mut refs = Vec::new();
            if !col.dto.is_nil() {
                refs.push(IndexedRef { to: col.dto, sub: None, location: Some("dto".into()) });
            }
            for row in &col.rows {
                let row_loc = format!("row:{}", row.id);
                for val in row.values.values() {
                    collect_value_refs(val, &row_loc, &mut refs);
                }
            }
            assets.push(IndexedAsset { asset: collection_ref(path), id: Ok(col.id), sub_items, refs });
        }
        for (path, curve) in &self.curves {
            assets.push(IndexedAsset {
                asset: curve_ref(path),
                id: Ok(curve.id),
                sub_items: Vec::new(),
                refs: Vec::new(),
            });
        }
        assets.sort_by(|a, b| a.asset.cmp(&b.asset));
        DomainIndex { domain: DATA, assets }
    }

    /// Resolves a target DTO UUID (`col.dto` or `FieldType::Dto`) to `(dto_path, &DtoAsset)` using `index` first
    /// (resolve seam, ADR 0047 §4) and falling back to `self.dtos` when `index` is empty in standalone unit tests.
    pub fn resolve_dto<'a>(&'a self, dto_uuid: AssetUuid, index: &'a AssetIndex) -> Option<(&'a str, &'a DtoAsset)> {
        if !dto_uuid.is_v7() {
            return None;
        }
        if let Some(entry) = index.get(dto_uuid)
            && entry.asset.domain == DATA
            && entry.asset.suffix == DTO_SUFFIX
            && let Some((k, v)) = self.dtos.get_key_value(entry.asset.path.as_str())
        {
            return Some((k.as_str(), v));
        }
        self.dtos.iter().find(|(_, d)| d.id == dto_uuid).map(|(k, v)| (k.as_str(), v))
    }

    /// Resolves a target Enum UUID (`FieldType::Enum`) to `(enum_path, &EnumAsset)` using `index` first
    /// (resolve seam, ADR 0047 §4) and falling back to `self.enums` when `index` is empty in standalone unit tests.
    pub fn resolve_enum<'a>(&'a self, enum_uuid: AssetUuid, index: &'a AssetIndex) -> Option<(&'a str, &'a EnumAsset)> {
        if !enum_uuid.is_v7() {
            return None;
        }
        if let Some(entry) = index.get(enum_uuid)
            && entry.asset.domain == DATA
            && entry.asset.suffix == ENUM_SUFFIX
            && let Some((k, v)) = self.enums.get_key_value(entry.asset.path.as_str())
        {
            return Some((k.as_str(), v));
        }
        self.enums.iter().find(|(_, e)| e.id == enum_uuid).map(|(k, v)| (k.as_str(), v))
    }

    /// Validates all loaded DTOs, Enums, and collections against `index` (ADR 0049 §5).
    pub fn validate(&self, index: &AssetIndex) -> ValidationReport {
        self.validate_with_project(None, index)
    }

    /// Validates all loaded DTOs, Enums, and collections against `index` and optional `project` (for enabled-domain and
    /// `assets/<path>` file-existence checks on `DE008`).
    pub fn validate_with_project(&self, project: Option<&Project>, index: &AssetIndex) -> ValidationReport {
        let mut out = self.load_diagnostics.clone();
        let mut skipped_rows: BTreeSet<(String, usize)> = BTreeSet::new();

        // Check paths for in-memory `DataAssets` if not already reported by `scan`.
        let mut reported_path_errors: BTreeSet<String> =
            out.iter().filter(|d| d.code == "DE003").filter_map(|d| d.asset.clone()).collect();

        for path in self.dtos.keys() {
            let rel = dto_ref(path).source_rel();
            if !reported_path_errors.contains(&rel)
                && let Err(reason) = check_path(path)
            {
                out.push(
                    Diagnostic::error("DE003", format!("invalid DTO path `{path}`: {reason}"))
                        .in_domain(DATA)
                        .in_asset(rel.clone()),
                );
                reported_path_errors.insert(rel);
            }
        }
        for path in self.enums.keys() {
            let rel = enum_ref(path).source_rel();
            if !reported_path_errors.contains(&rel)
                && let Err(reason) = check_path(path)
            {
                out.push(
                    Diagnostic::error("DE003", format!("invalid enum path `{path}`: {reason}"))
                        .in_domain(DATA)
                        .in_asset(rel.clone()),
                );
                reported_path_errors.insert(rel);
            }
        }
        for path in self.collections.keys() {
            let rel = collection_ref(path).source_rel();
            if !reported_path_errors.contains(&rel)
                && let Err(reason) = check_path(path)
            {
                out.push(
                    Diagnostic::error("DE003", format!("invalid collection path `{path}`: {reason}"))
                        .in_domain(DATA)
                        .in_asset(rel.clone()),
                );
                reported_path_errors.insert(rel);
            }
        }
        for (path, curve) in &self.curves {
            let rel = curve_ref(path).source_rel();
            if !reported_path_errors.contains(&rel)
                && let Err(reason) = check_path(path)
            {
                out.push(
                    Diagnostic::error("DE003", format!("invalid curve path `{path}`: {reason}"))
                        .in_domain(DATA)
                        .in_asset(rel.clone()),
                );
                reported_path_errors.insert(rel.clone());
            }
            if !curve.id.is_v7() {
                out.push(
                    Diagnostic::error("DE001", format!("curve `{path}` has missing or non-UUIDv7 `id` `{}`", curve.id))
                        .in_domain(DATA)
                        .in_asset(rel.clone()),
                );
            }
            if let Some(msg) = curve_kit::range_error(curve.min, curve.max) {
                out.push(Diagnostic::error("DE006", format!("curve `{path}`: {msg}")).in_domain(DATA).in_asset(rel));
            }
        }

        // Check PascalCase type name collisions across DTOs and Enums (`DE003`).
        let mut schema_by_pascal: BTreeMap<String, Vec<&str>> = BTreeMap::new();
        for path in self.dtos.keys() {
            schema_by_pascal.entry(to_pascal_case(path)).or_default().push(path.as_str());
        }
        for path in self.enums.keys() {
            schema_by_pascal.entry(to_pascal_case(path)).or_default().push(path.as_str());
        }

        // Memoized acyclic subgraph height per DTO/Enum UUID for O(V + E) diamond-safe cycle & depth checking.
        let mut acyclic_height_memo: BTreeMap<AssetUuid, usize> = BTreeMap::new();
        let mut cyclic_schemas: BTreeSet<AssetUuid> = BTreeSet::new();

        // 1. Validate DTO schemas.
        for (dto_path, dto) in &self.dtos {
            let source_rel = dto_ref(dto_path).source_rel();
            let at = |d: Diagnostic| d.in_domain(DATA).in_asset(source_rel.clone());

            if !dto.id.is_v7() {
                out.push(at(Diagnostic::error(
                    "DE001",
                    format!("DTO `{dto_path}` has missing or non-UUIDv7 `id` `{}`", dto.id),
                )));
            }

            let pascal = to_pascal_case(dto_path);
            if let Some(group) = schema_by_pascal.get(&pascal)
                && group.len() > 1
            {
                let others: Vec<&str> = group.iter().copied().filter(|&o| o != dto_path.as_str()).collect();
                out.push(at(Diagnostic::error(
                    "DE003",
                    format!("DTO `{dto_path}` collides in PascalCase (`{pascal}`) with {}", others.join(", ")),
                )));
            }

            if dto.fields.is_empty() {
                out.push(at(Diagnostic::warning("DW103", format!("DTO `{dto_path}` has 0 fields"))));
            }

            let mut seen_field_ids: BTreeMap<AssetUuid, &str> = BTreeMap::new();
            let mut seen_field_names: BTreeSet<&str> = BTreeSet::new();

            for field in &dto.fields {
                let loc = format!("field:{}", field.id);
                let at_field = |d: Diagnostic| at(d).at(loc.clone());

                if !field.id.is_v7() {
                    out.push(at_field(Diagnostic::error(
                        "DE001",
                        format!("field `{}` has missing or non-UUIDv7 `id` `{}`", field.name, field.id),
                    )));
                } else if let Some(prev_name) = seen_field_ids.insert(field.id, field.name.as_str()) {
                    out.push(at_field(Diagnostic::error(
                        "DE002",
                        format!(
                            "field `{}` has duplicate UUID `{}` (already used by field `{prev_name}`)",
                            field.name, field.id
                        ),
                    )));
                }

                if let Err(reason) = check_ident(&field.name) {
                    out.push(at_field(Diagnostic::error(
                        "DE003",
                        format!("invalid field name `{}`: {reason}", field.name),
                    )));
                } else if is_rust_keyword(&field.name) {
                    out.push(at_field(Diagnostic::error(
                        "DE003",
                        format!("field name `{}` is a reserved Rust keyword", field.name),
                    )));
                }

                if !seen_field_names.insert(field.name.as_str()) {
                    out.push(at_field(Diagnostic::error(
                        "DE003",
                        format!("duplicate field name `{}` in DTO `{dto_path}`", field.name),
                    )));
                }

                self.validate_schema_field_body(
                    dto.id,
                    dto_path,
                    &source_rel,
                    field,
                    &loc,
                    &field.name,
                    project,
                    index,
                    &mut acyclic_height_memo,
                    &mut cyclic_schemas,
                    &mut out,
                    &mut skipped_rows,
                );
            }
        }

        // 1b. Validate Enum schemas.
        for (enum_path, en) in &self.enums {
            let source_rel = enum_ref(enum_path).source_rel();
            let at = |d: Diagnostic| d.in_domain(DATA).in_asset(source_rel.clone());

            if !en.id.is_v7() {
                out.push(at(Diagnostic::error(
                    "DE001",
                    format!("enum `{enum_path}` has missing or non-UUIDv7 `id` `{}`", en.id),
                )));
            }

            let pascal = to_pascal_case(enum_path);
            if let Some(group) = schema_by_pascal.get(&pascal)
                && group.len() > 1
            {
                let others: Vec<&str> = group.iter().copied().filter(|&o| o != enum_path.as_str()).collect();
                out.push(at(Diagnostic::error(
                    "DE003",
                    format!("enum `{enum_path}` collides in PascalCase (`{pascal}`) with {}", others.join(", ")),
                )));
            }

            if en.variants.is_empty() {
                out.push(at(Diagnostic::warning("DW103", format!("enum `{enum_path}` has 0 variants"))));
            }

            let mut seen_ids: BTreeMap<AssetUuid, String> = BTreeMap::new();
            let mut seen_variant_names: BTreeSet<&str> = BTreeSet::new();

            for variant in &en.variants {
                let var_loc = format!("variant:{}", variant.id);
                let at_var = |d: Diagnostic| at(d).at(var_loc.clone());

                if !variant.id.is_v7() {
                    out.push(at_var(Diagnostic::error(
                        "DE001",
                        format!("variant `{}` has missing or non-UUIDv7 `id` `{}`", variant.name, variant.id),
                    )));
                } else if let Some(prev_desc) = seen_ids.insert(variant.id, format!("variant `{}`", variant.name)) {
                    out.push(at_var(Diagnostic::error(
                        "DE002",
                        format!(
                            "variant `{}` has duplicate UUID `{}` (already used by {prev_desc})",
                            variant.name, variant.id
                        ),
                    )));
                }

                if let Err(reason) = check_ident(&variant.name) {
                    out.push(at_var(Diagnostic::error(
                        "DE003",
                        format!("invalid variant name `{}`: {reason}", variant.name),
                    )));
                } else if is_rust_keyword(&variant.name) {
                    out.push(at_var(Diagnostic::error(
                        "DE003",
                        format!("variant name `{}` is a reserved Rust keyword", variant.name),
                    )));
                }

                if !seen_variant_names.insert(variant.name.as_str()) {
                    out.push(at_var(Diagnostic::error(
                        "DE003",
                        format!("duplicate variant name `{}` in enum `{enum_path}`", variant.name),
                    )));
                }

                let mut seen_field_names: BTreeSet<&str> = BTreeSet::new();
                for field in &variant.fields {
                    let field_loc = format!("variant:{}:field:{}", variant.id, field.id);
                    let at_field = |d: Diagnostic| at(d).at(field_loc.clone());
                    let qual_name = format!("{}.{}", variant.name, field.name);

                    if !field.id.is_v7() {
                        out.push(at_field(Diagnostic::error(
                            "DE001",
                            format!("field `{qual_name}` has missing or non-UUIDv7 `id` `{}`", field.id),
                        )));
                    } else if let Some(prev_desc) = seen_ids.insert(field.id, format!("field `{qual_name}`")) {
                        out.push(at_field(Diagnostic::error(
                            "DE002",
                            format!(
                                "field `{qual_name}` has duplicate UUID `{}` (already used by {prev_desc})",
                                field.id
                            ),
                        )));
                    }

                    if let Err(reason) = check_ident(&field.name) {
                        out.push(at_field(Diagnostic::error(
                            "DE003",
                            format!("invalid field name `{}` in variant `{}`: {reason}", field.name, variant.name),
                        )));
                    } else if is_rust_keyword(&field.name) {
                        out.push(at_field(Diagnostic::error(
                            "DE003",
                            format!(
                                "field name `{}` in variant `{}` is a reserved Rust keyword",
                                field.name, variant.name
                            ),
                        )));
                    }

                    if !seen_field_names.insert(field.name.as_str()) {
                        out.push(at_field(Diagnostic::error(
                            "DE003",
                            format!(
                                "duplicate field name `{}` in variant `{}` of enum `{enum_path}`",
                                field.name, variant.name
                            ),
                        )));
                    }

                    self.validate_schema_field_body(
                        en.id,
                        enum_path,
                        &source_rel,
                        field,
                        &field_loc,
                        &qual_name,
                        project,
                        index,
                        &mut acyclic_height_memo,
                        &mut cyclic_schemas,
                        &mut out,
                        &mut skipped_rows,
                    );
                }
            }
        }

        // 2. Check which DTOs and Enums are referenced (`DW104`).
        let mut referenced_dtos: BTreeSet<&str> = BTreeSet::new();
        let mut referenced_enums: BTreeSet<&str> = BTreeSet::new();
        for col in self.collections.values() {
            if let Some((dto_path, _)) = self.resolve_dto(col.dto, index) {
                referenced_dtos.insert(dto_path);
            }
        }
        for dto in self.dtos.values() {
            for field in &dto.fields {
                self.mark_referenced_schemas(&field.ty, index, &mut referenced_dtos, &mut referenced_enums);
            }
        }
        for en in self.enums.values() {
            for variant in &en.variants {
                for field in &variant.fields {
                    self.mark_referenced_schemas(&field.ty, index, &mut referenced_dtos, &mut referenced_enums);
                }
            }
        }
        for dto_path in self.dtos.keys() {
            if !referenced_dtos.contains(dto_path.as_str()) {
                out.push(
                    Diagnostic::warning(
                        "DW104",
                        format!("DTO `{dto_path}` is not referenced by any collection in the project"),
                    )
                    .in_domain(DATA)
                    .in_asset(dto_ref(dto_path).source_rel()),
                );
            }
        }
        for enum_path in self.enums.keys() {
            if !referenced_enums.contains(enum_path.as_str()) {
                out.push(
                    Diagnostic::warning(
                        "DW104",
                        format!("enum `{enum_path}` is not referenced by any DTO or enum in the project"),
                    )
                    .in_domain(DATA)
                    .in_asset(enum_ref(enum_path).source_rel()),
                );
            }
        }

        // 3. Validate collections and project-wide Row.id uniqueness (`D5`: winner = smallest `(collection_path, row_idx)`).
        let mut seen_row_ids: BTreeMap<AssetUuid, (&str, usize)> = BTreeMap::new();

        for (col_path, col) in &self.collections {
            let source_rel = collection_ref(col_path).source_rel();
            let at = |d: Diagnostic| d.in_domain(DATA).in_asset(source_rel.clone());

            if !col.id.is_v7() {
                out.push(at(Diagnostic::error(
                    "DE001",
                    format!("collection `{col_path}` has missing or non-UUIDv7 `id` `{}`", col.id),
                )));
            }

            if col.rows.is_empty() {
                out.push(at(Diagnostic::warning("DW103", format!("collection `{col_path}` is empty (0 rows)"))));
            }

            let resolved_dto = self.resolve_dto(col.dto, index);
            if resolved_dto.is_none() {
                out.push(
                    at(Diagnostic::error(
                        "DE004",
                        format!("collection `{col_path}` references unknown DTO UUID `{}`", col.dto),
                    ))
                    .at("dto"),
                );
            }

            let fields_by_id: BTreeMap<AssetUuid, &Field> = resolved_dto
                .map(|(_, dto)| dto.fields.iter().filter(|f| f.id.is_v7()).map(|f| (f.id, f)).collect())
                .unwrap_or_default();

            let mut unique_seen: BTreeMap<AssetUuid, Vec<(usize, &DataValue)>> = BTreeMap::new();

            for (row_idx, row) in col.rows.iter().enumerate() {
                let row_loc = format!("row:{}", row.id);
                if !row.id.is_v7() {
                    out.push(
                        at(Diagnostic::error(
                            "DE001",
                            format!("row #{row_idx} has missing or non-UUIDv7 `id` `{}`", row.id),
                        ))
                        .at(row_loc),
                    );
                    skipped_rows.insert((col_path.clone(), row_idx));
                } else if let Some(&(winner_col, winner_idx)) = seen_row_ids.get(&row.id) {
                    out.push(
                        at(Diagnostic::error(
                            "DE002",
                            format!(
                                "row #{row_idx} has duplicate UUID `{}` (already used in `{winner_col}` row #{winner_idx}); losing row is skipped",
                                row.id
                            ),
                        ))
                        .at(row_loc),
                    );
                    skipped_rows.insert((col_path.clone(), row_idx));
                } else {
                    seen_row_ids.insert(row.id, (col_path.as_str(), row_idx));
                }

                let Some((_, dto)) = resolved_dto else { continue };

                // Check for stale/unknown field UUIDs (`DW101`).
                for field_id in row.values.keys() {
                    if !fields_by_id.contains_key(field_id) {
                        out.push(
                            at(Diagnostic::warning(
                                "DW101",
                                format!(
                                    "row #{row_idx} contains value for unknown field UUID `{field_id}` (stale field value; save collection to prune)"
                                ),
                            ))
                            .at(format!("row:{}:field:{field_id}", row.id)),
                        );
                    }
                }

                // Validate each declared DTO field on this row (skipping duplicate field UUIDs in the DTO).
                let mut seen_row_fields: BTreeSet<AssetUuid> = BTreeSet::new();
                for field in &dto.fields {
                    if !field.id.is_v7() || !seen_row_fields.insert(field.id) {
                        continue;
                    }
                    let cell_loc = format!("row:{}:field:{}", row.id, field.id);
                    let at_cell = |d: Diagnostic| at(d).at(cell_loc.clone());
                    let explicit_val = row.values.get(&field.id);
                    let val = explicit_val.unwrap_or(&field.default);
                    // If the row uses the schema default and the schema default itself is invalid, the error is
                    // already reported on the DTO schema.
                    if explicit_val.is_none()
                        && (!field.ty.matches(&field.default)
                            || check_value_intrinsic_constraints(&field.name, &field.default).is_some()
                            || check_asset_ref_value(project, index, &field.name, &field.ty, &field.default).is_some()
                            || self.check_soft_ref_value(index, &field.name, &field.ty, &field.default).is_some()
                            || self.has_duplicate_container_keys(&field.ty, &field.default))
                    {
                        continue;
                    }

                    if !field.ty.matches(val) {
                        out.push(at_cell(Diagnostic::error(
                            "DE005",
                            format!(
                                "row #{row_idx} field `{}` value type `{}` does not match declared type `{}`",
                                field.name,
                                val.field_type().label(),
                                field.ty.label()
                            ),
                        )));
                        skipped_rows.insert((col_path.clone(), row_idx));
                        continue;
                    }

                    if value_nesting_depth(val) > MAX_NESTING_DEPTH {
                        out.push(at_cell(Diagnostic::error(
                            "DE009",
                            format!(
                                "row #{row_idx} field `{}` value nesting depth exceeds maximum of {MAX_NESTING_DEPTH}",
                                field.name
                            ),
                        )));
                        skipped_rows.insert((col_path.clone(), row_idx));
                        continue;
                    }

                    if let (FieldType::Dto(target_uuid), Some(DataValue::Dto(map))) = (&field.ty, explicit_val)
                        && !cyclic_schemas.contains(target_uuid)
                    {
                        let mut visited = BTreeSet::new();
                        if !dto.id.is_nil() {
                            visited.insert(dto.id);
                        }
                        self.validate_nested_dto_map(
                            map,
                            *target_uuid,
                            project,
                            index,
                            &source_rel,
                            &format!("row:{}:field:{}", row.id, field.name),
                            &field.name,
                            Some((col_path.as_str(), row_idx)),
                            &mut visited,
                            1,
                            &mut out,
                            &mut skipped_rows,
                        );
                    }

                    if let (FieldType::Enum(target_uuid), Some(DataValue::Enum { variant, fields })) =
                        (&field.ty, explicit_val)
                        && !cyclic_schemas.contains(target_uuid)
                    {
                        let mut visited = BTreeSet::new();
                        if !dto.id.is_nil() {
                            visited.insert(dto.id);
                        }
                        self.validate_nested_enum_value(
                            *variant,
                            fields,
                            *target_uuid,
                            project,
                            index,
                            &source_rel,
                            &format!("row:{}:field:{}", row.id, field.name),
                            &field.name,
                            Some((col_path.as_str(), row_idx)),
                            &mut visited,
                            1,
                            &mut out,
                            &mut skipped_rows,
                        );
                    }

                    if field.ty.is_container() {
                        let mut visited = BTreeSet::new();
                        self.validate_container_value(
                            &field.ty,
                            val,
                            project,
                            index,
                            &source_rel,
                            &cell_loc,
                            &field.name,
                            Some((col_path.as_str(), row_idx)),
                            &mut visited,
                            1,
                            &mut out,
                            &mut skipped_rows,
                        );
                    }

                    if field.settings.required
                        && let Some(s) = val.as_str()
                        && s.trim().is_empty()
                    {
                        out.push(at_cell(Diagnostic::error(
                            "DE006",
                            format!("row #{row_idx} required field `{}` must not be empty", field.name),
                        )));
                        skipped_rows.insert((col_path.clone(), row_idx));
                    }

                    if field.settings.required
                        && let (FieldType::SoftRef(_), DataValue::SoftRef(row_uuid)) = (&field.ty, val)
                        && row_uuid.is_nil()
                    {
                        out.push(at_cell(Diagnostic::error(
                            "DE006",
                            format!("row #{row_idx} required SoftRef field `{}` is unset (None)", field.name),
                        )));
                        skipped_rows.insert((col_path.clone(), row_idx));
                    }

                    if field.settings.required
                        && let Some(msg) = check_required_container(&field.name, &field.ty, val)
                    {
                        out.push(at_cell(Diagnostic::error("DE006", format!("row #{row_idx} {msg}"))));
                        skipped_rows.insert((col_path.clone(), row_idx));
                    }

                    if field.settings.required
                        && let Some(msg) = check_required_asset_ref(&field.name, &field.ty, val)
                    {
                        out.push(at_cell(Diagnostic::error("DE005", format!("row #{row_idx} {msg}"))));
                        skipped_rows.insert((col_path.clone(), row_idx));
                    }

                    if let Some(msg) = check_asset_ref_value(project, index, &field.name, &field.ty, val) {
                        out.push(at_cell(Diagnostic::error("DE008", format!("row #{row_idx} {msg}"))));
                        skipped_rows.insert((col_path.clone(), row_idx));
                    }

                    if let Some(msg) = self.check_soft_ref_value(index, &field.name, &field.ty, val) {
                        out.push(at_cell(Diagnostic::error("DE008", format!("row #{row_idx} {msg}"))));
                        skipped_rows.insert((col_path.clone(), row_idx));
                    }

                    if let Some(max_len) = field.settings.max_len
                        && let Some(s) = val.as_str()
                    {
                        let len = s.chars().count();
                        if len > max_len {
                            out.push(at_cell(Diagnostic::error(
                                "DE006",
                                format!(
                                    "row #{row_idx} field `{}` length ({len} chars) exceeds max_len ({max_len})",
                                    field.name
                                ),
                            )));
                            skipped_rows.insert((col_path.clone(), row_idx));
                        }
                    }

                    if let Some(msg) = check_value_intrinsic_constraints(&field.name, val) {
                        out.push(at_cell(Diagnostic::error("DE006", format!("row #{row_idx} {msg}"))));
                        skipped_rows.insert((col_path.clone(), row_idx));
                    }

                    if field.settings.unique_in_collection {
                        let entries = unique_seen.entry(field.id).or_default();
                        if let Some(&(prev_idx, _)) = entries.iter().find(|&&(_, prev_val)| prev_val == val) {
                            out.push(at_cell(Diagnostic::error(
                                "DE006",
                                format!(
                                    "row #{row_idx} field `{}` violates unique_in_collection (duplicate value with row #{prev_idx})",
                                    field.name
                                ),
                            )));
                            skipped_rows.insert((col_path.clone(), row_idx));
                        } else {
                            entries.push((row_idx, val));
                        }
                    }

                    let bounds_valid = !matches!(
                        (field.settings.min, field.settings.max),
                        (Some(min), Some(max)) if min > max || !min.is_finite() || !max.is_finite()
                    );
                    if bounds_valid && let Some(msg) = check_soft_numeric_bounds(&field.name, field, val) {
                        out.push(at_cell(Diagnostic::warning("DW102", format!("row #{row_idx} {msg}"))));
                    }
                }
            }
        }

        ValidationReport { diagnostics: out, skipped_rows }
    }

    #[allow(clippy::too_many_arguments)]
    fn validate_schema_field_body<'a>(
        &'a self,
        owner_id: AssetUuid,
        owner_path: &'a str,
        source_rel: &str,
        field: &Field,
        loc: &str,
        qual_field_name: &str,
        project: Option<&Project>,
        index: &'a AssetIndex,
        acyclic_height_memo: &mut BTreeMap<AssetUuid, usize>,
        cyclic_schemas: &mut BTreeSet<AssetUuid>,
        out: &mut Vec<Diagnostic>,
        skipped_rows: &mut BTreeSet<(String, usize)>,
    ) {
        let at = |d: Diagnostic| d.in_domain(DATA).in_asset(source_rel.to_string());
        let at_field = |d: Diagnostic| at(d).at(loc.to_string());

        if type_nesting_depth(&field.ty) > MAX_NESTING_DEPTH {
            out.push(
                at(Diagnostic::error(
                    "DE009",
                    format!("field `{qual_field_name}` type nesting depth exceeds maximum of {MAX_NESTING_DEPTH}"),
                ))
                .at(format!("{qual_field_name}.ty")),
            );
            return;
        }

        // Check `FieldType::Dto` / `FieldType::Enum` target resolution, cycle (`DE003`), and depth (`DE009`).
        let mut field_is_cyclic_or_dangling = false;
        match &field.ty {
            FieldType::Dto(target_uuid) => {
                if let Some((target_path, _)) = self.resolve_dto(*target_uuid, index) {
                    let mut active = vec![(owner_path, owner_id)];
                    match self.check_by_value_subgraph(
                        target_path,
                        *target_uuid,
                        index,
                        &mut active,
                        acyclic_height_memo,
                    ) {
                        Ok(sub_height) => {
                            let edge_depth = 1 + sub_height;
                            if edge_depth > MAX_NESTING_DEPTH {
                                out.push(
                                    at(Diagnostic::error(
                                        "DE009",
                                        format!(
                                            "by-value DTO nesting depth ({edge_depth}) exceeds maximum of {MAX_NESTING_DEPTH} on field `{qual_field_name}`"
                                        ),
                                    ))
                                    .at(qual_field_name.to_string()),
                                );
                                field_is_cyclic_or_dangling = true;
                            }
                        }
                        Err((cycle_start_idx, chain)) => {
                            field_is_cyclic_or_dangling = true;
                            cyclic_schemas.insert(owner_id);
                            cyclic_schemas.insert(*target_uuid);
                            if cycle_start_idx == 0 {
                                out.push(
                                    at(Diagnostic::error(
                                        "DE003",
                                        format!("by-value DTO cycle detected: {}", chain.join(" -> ")),
                                    ))
                                    .at(qual_field_name.to_string()),
                                );
                            }
                        }
                    }
                } else {
                    field_is_cyclic_or_dangling = true;
                    out.push(
                        at(Diagnostic::error(
                            "DE004",
                            format!("field `{qual_field_name}` references unknown DTO UUID `{target_uuid}`"),
                        ))
                        .at(format!("{qual_field_name}.ty")),
                    );
                }
            }
            FieldType::Enum(target_uuid) => {
                if let Some((target_path, _)) = self.resolve_enum(*target_uuid, index) {
                    let mut active = vec![(owner_path, owner_id)];
                    match self.check_by_value_subgraph(
                        target_path,
                        *target_uuid,
                        index,
                        &mut active,
                        acyclic_height_memo,
                    ) {
                        Ok(sub_height) => {
                            let edge_depth = 1 + sub_height;
                            if edge_depth > MAX_NESTING_DEPTH {
                                out.push(
                                    at(Diagnostic::error(
                                        "DE009",
                                        format!(
                                            "by-value enum nesting depth ({edge_depth}) exceeds maximum of {MAX_NESTING_DEPTH} on field `{qual_field_name}`"
                                        ),
                                    ))
                                    .at(qual_field_name.to_string()),
                                );
                                field_is_cyclic_or_dangling = true;
                            }
                        }
                        Err((cycle_start_idx, chain)) => {
                            field_is_cyclic_or_dangling = true;
                            cyclic_schemas.insert(owner_id);
                            cyclic_schemas.insert(*target_uuid);
                            if cycle_start_idx == 0 {
                                out.push(
                                    at(Diagnostic::error(
                                        "DE003",
                                        format!("by-value enum cycle detected: {}", chain.join(" -> ")),
                                    ))
                                    .at(qual_field_name.to_string()),
                                );
                            }
                        }
                    }
                } else {
                    field_is_cyclic_or_dangling = true;
                    out.push(
                        at(Diagnostic::error(
                            "DE004",
                            format!("field `{qual_field_name}` references unknown enum UUID `{target_uuid}`"),
                        ))
                        .at(format!("{qual_field_name}.ty")),
                    );
                }
            }
            FieldType::SoftRef(target_uuid) if self.resolve_dto(*target_uuid, index).is_none() => {
                out.push(
                    at(Diagnostic::error(
                        "DE004",
                        format!("field `{qual_field_name}` references unknown DTO UUID `{target_uuid}`"),
                    ))
                    .at(format!("{qual_field_name}.ty")),
                );
            }
            FieldType::Option(_) | FieldType::Array(_) | FieldType::Set(_) | FieldType::Map(_, _) => {
                self.validate_container_subtypes(
                    &field.ty,
                    source_rel,
                    qual_field_name,
                    project,
                    index,
                    &mut field_is_cyclic_or_dangling,
                    out,
                );
            }
            _ => {}
        }

        let bounds_inverted = matches!(
            (field.settings.min, field.settings.max),
            (Some(min), Some(max)) if min > max || !min.is_finite() || !max.is_finite()
        );
        if bounds_inverted {
            out.push(at_field(Diagnostic::error(
                "DE006",
                format!(
                    "field `{qual_field_name}` settings have invalid bounds min={:?} > max={:?}",
                    field.settings.min, field.settings.max
                ),
            )));
        }

        if field.ty == FieldType::LightEnvironment && project.is_some_and(|p| !p.domain_enabled(DomainId::new("light")))
        {
            out.push(at_field(Diagnostic::error(
                "DE008",
                format!(
                    "field `{qual_field_name}` has type `LightEnvironment`, but the `light` domain is disabled in `project.ron`"
                ),
            )));
        }

        if !field.ty.matches(&field.default) {
            out.push(at_field(Diagnostic::error(
                "DE005",
                format!(
                    "field `{qual_field_name}` default value type `{}` does not match declared type `{}`",
                    field.default.field_type().label(),
                    field.ty.label()
                ),
            )));
            return;
        }

        if value_nesting_depth(&field.default) > MAX_NESTING_DEPTH {
            out.push(
                at(Diagnostic::error(
                    "DE009",
                    format!(
                        "field `{qual_field_name}` default value nesting depth exceeds maximum of {MAX_NESTING_DEPTH}"
                    ),
                ))
                .at(qual_field_name.to_string()),
            );
            return;
        }

        if !field_is_cyclic_or_dangling {
            match (&field.ty, &field.default) {
                (FieldType::Dto(target_uuid), DataValue::Dto(map)) => {
                    let mut visited = BTreeSet::new();
                    if !owner_id.is_nil() {
                        visited.insert(owner_id);
                    }
                    self.validate_nested_dto_map(
                        map,
                        *target_uuid,
                        project,
                        index,
                        source_rel,
                        &format!("field:{qual_field_name}"),
                        qual_field_name,
                        None,
                        &mut visited,
                        1,
                        out,
                        skipped_rows,
                    );
                }
                (FieldType::Enum(target_uuid), DataValue::Enum { variant, fields }) => {
                    let mut visited = BTreeSet::new();
                    if !owner_id.is_nil() {
                        visited.insert(owner_id);
                    }
                    self.validate_nested_enum_value(
                        *variant,
                        fields,
                        *target_uuid,
                        project,
                        index,
                        source_rel,
                        &format!("field:{qual_field_name}"),
                        qual_field_name,
                        None,
                        &mut visited,
                        1,
                        out,
                        skipped_rows,
                    );
                }
                (FieldType::Option(_) | FieldType::Array(_) | FieldType::Set(_) | FieldType::Map(_, _), _) => {
                    let mut visited = BTreeSet::new();
                    self.validate_container_value(
                        &field.ty,
                        &field.default,
                        project,
                        index,
                        source_rel,
                        loc,
                        qual_field_name,
                        None,
                        &mut visited,
                        1,
                        out,
                        skipped_rows,
                    );
                }
                _ => {}
            }
        }

        if let Some(msg) = check_asset_ref_value(project, index, qual_field_name, &field.ty, &field.default) {
            out.push(at_field(Diagnostic::error("DE008", msg)));
        }

        if let Some(msg) = self.check_soft_ref_value(index, qual_field_name, &field.ty, &field.default) {
            out.push(at_field(Diagnostic::error("DE008", msg)));
        }

        if let Some(msg) = check_value_intrinsic_constraints(qual_field_name, &field.default) {
            out.push(at_field(Diagnostic::error("DE006", msg)));
        }

        if !bounds_inverted && let Some(msg) = check_soft_numeric_bounds(qual_field_name, field, &field.default) {
            out.push(at_field(Diagnostic::warning("DW102", msg)));
        }
    }

    /// Walks the by-value (`FieldType::Dto` and `FieldType::Enum`) dependency graph from `(node_path, node_id)` using
    /// `active` for cycle detection and `memo` for proven-acyclic subgraph heights (diamond-safe).
    fn check_by_value_subgraph<'a>(
        &'a self,
        node_path: &'a str,
        node_id: AssetUuid,
        index: &'a AssetIndex,
        active: &mut Vec<(&'a str, AssetUuid)>,
        memo: &mut BTreeMap<AssetUuid, usize>,
    ) -> Result<usize, (usize, Vec<String>)> {
        if let Some(pos) = active.iter().position(|&(_, id)| !id.is_nil() && id == node_id) {
            let mut chain: Vec<String> = active[pos..].iter().map(|&(p, _)| p.to_string()).collect();
            chain.push(node_path.to_string());
            return Err((pos, chain));
        }
        if !node_id.is_nil()
            && let Some(&cached_height) = memo.get(&node_id)
        {
            return Ok(cached_height);
        }

        active.push((node_path, node_id));
        let mut max_child_height = 0usize;

        let child_fields: Vec<&Field> = if let Some((_, dto)) = self.resolve_dto(node_id, index) {
            dto.fields.iter().collect()
        } else if let Some((_, en)) = self.resolve_enum(node_id, index) {
            en.variants.iter().flat_map(|v| v.fields.iter()).collect()
        } else {
            Vec::new()
        };

        for field in child_fields {
            let child_target = match field.ty {
                FieldType::Dto(target_uuid) => self.resolve_dto(target_uuid, index).map(|(p, d)| (p, d.id)),
                FieldType::Enum(target_uuid) => self.resolve_enum(target_uuid, index).map(|(p, e)| (p, e.id)),
                _ => None,
            };
            if let Some((target_path, target_id)) = child_target {
                match self.check_by_value_subgraph(target_path, target_id, index, active, memo) {
                    Ok(sub_h) => {
                        max_child_height = max_child_height.max(1 + sub_h);
                    }
                    Err(cycle) => {
                        active.pop();
                        return Err(cycle);
                    }
                }
            }
        }
        active.pop();

        if !node_id.is_nil() {
            memo.insert(node_id, max_child_height);
        }
        Ok(max_child_height)
    }

    #[allow(clippy::too_many_arguments)]
    fn validate_nested_dto_map(
        &self,
        map: &BTreeMap<AssetUuid, DataValue>,
        target_uuid: AssetUuid,
        project: Option<&Project>,
        index: &AssetIndex,
        source_rel: &str,
        loc_prefix: &str,
        field_path: &str,
        in_row: Option<(&str, usize)>,
        visited: &mut BTreeSet<AssetUuid>,
        depth: usize,
        out: &mut Vec<Diagnostic>,
        skipped_rows: &mut BTreeSet<(String, usize)>,
    ) {
        let row_prefix = match in_row {
            Some((_, row_idx)) => format!("row #{row_idx} "),
            None => String::new(),
        };
        let at_loc = |d: Diagnostic, loc: String| d.in_domain(DATA).in_asset(source_rel.to_string()).at(loc);

        if depth > MAX_NESTING_DEPTH {
            out.push(at_loc(
                Diagnostic::error(
                    "DE009",
                    format!(
                        "{row_prefix}field `{field_path}` nested DTO value exceeds maximum depth of {MAX_NESTING_DEPTH}"
                    ),
                ),
                loc_prefix.to_string(),
            ));
            if let Some((col_path, row_idx)) = in_row {
                skipped_rows.insert((col_path.to_string(), row_idx));
            }
            return;
        }

        let Some((_, target_dto)) = self.resolve_dto(target_uuid, index) else {
            return;
        };
        if !visited.insert(target_uuid) {
            return;
        }

        let target_fields_by_id: BTreeMap<AssetUuid, &Field> =
            target_dto.fields.iter().filter(|f| f.id.is_v7()).map(|f| (f.id, f)).collect();

        // Unknown field UUIDs inside `map` (`DW101`).
        for unknown_id in map.keys() {
            if !target_fields_by_id.contains_key(unknown_id) {
                out.push(at_loc(
                    Diagnostic::warning(
                        "DW101",
                        format!(
                            "{row_prefix}field `{field_path}` contains value for unknown nested field UUID `{unknown_id}`"
                        ),
                    ),
                    format!("{loc_prefix}.{unknown_id}"),
                ));
            }
        }

        let mut seen_target_fields: BTreeSet<AssetUuid> = BTreeSet::new();
        for t_field in &target_dto.fields {
            if !t_field.id.is_v7() || !seen_target_fields.insert(t_field.id) {
                continue;
            }
            let sub_loc = format!("{loc_prefix}.{}", t_field.name);
            let sub_path = format!("{field_path}.{}", t_field.name);

            let Some(sub_val) = map.get(&t_field.id) else {
                // Report `DW102` when a row's nested DTO map (or a partially-populated default map) omits a field.
                if in_row.is_some() || !map.is_empty() {
                    out.push(at_loc(
                        Diagnostic::warning(
                            "DW102",
                            format!("{row_prefix}field `{sub_path}` is missing from nested DTO map (using default)"),
                        ),
                        sub_loc,
                    ));
                }
                continue;
            };

            self.validate_nested_field_value(
                t_field,
                sub_val,
                project,
                index,
                source_rel,
                &sub_loc,
                &sub_path,
                in_row,
                visited,
                depth,
                out,
                skipped_rows,
            );
        }

        visited.remove(&target_uuid);
    }

    #[allow(clippy::too_many_arguments)]
    fn validate_nested_enum_value(
        &self,
        variant_id: AssetUuid,
        fields: &BTreeMap<AssetUuid, DataValue>,
        target_uuid: AssetUuid,
        project: Option<&Project>,
        index: &AssetIndex,
        source_rel: &str,
        loc_prefix: &str,
        field_path: &str,
        in_row: Option<(&str, usize)>,
        visited: &mut BTreeSet<AssetUuid>,
        depth: usize,
        out: &mut Vec<Diagnostic>,
        skipped_rows: &mut BTreeSet<(String, usize)>,
    ) {
        let row_prefix = match in_row {
            Some((_, row_idx)) => format!("row #{row_idx} "),
            None => String::new(),
        };
        let at_loc = |d: Diagnostic, loc: String| d.in_domain(DATA).in_asset(source_rel.to_string()).at(loc);

        if depth > MAX_NESTING_DEPTH {
            out.push(at_loc(
                Diagnostic::error(
                    "DE009",
                    format!(
                        "{row_prefix}field `{field_path}` nested enum value exceeds maximum depth of {MAX_NESTING_DEPTH}"
                    ),
                ),
                loc_prefix.to_string(),
            ));
            if let Some((col_path, row_idx)) = in_row {
                skipped_rows.insert((col_path.to_string(), row_idx));
            }
            return;
        }

        let Some((target_enum_path, target_enum)) = self.resolve_enum(target_uuid, index) else {
            return;
        };
        if !visited.insert(target_uuid) {
            return;
        }

        // Unpopulated schema default (`variant == NIL && fields.is_empty()`) falls back to the first variant.
        if in_row.is_none() && variant_id.is_nil() && fields.is_empty() {
            visited.remove(&target_uuid);
            return;
        }

        if target_enum.variants.is_empty() {
            if !variant_id.is_nil() || !fields.is_empty() {
                out.push(at_loc(
                    Diagnostic::error(
                        "DE005",
                        format!(
                            "{row_prefix}field `{field_path}` references unknown enum variant UUID `{variant_id}` in empty enum `{target_enum_path}`"
                        ),
                    ),
                    loc_prefix.to_string(),
                ));
                if let Some((col_path, row_idx)) = in_row {
                    skipped_rows.insert((col_path.to_string(), row_idx));
                }
            }
            visited.remove(&target_uuid);
            return;
        }

        let Some(target_variant) = target_enum.variant_by_id(variant_id) else {
            out.push(at_loc(
                Diagnostic::error(
                    "DE005",
                    format!(
                        "{row_prefix}field `{field_path}` references unknown enum variant UUID `{variant_id}` in enum `{target_enum_path}`"
                    ),
                ),
                loc_prefix.to_string(),
            ));
            if let Some((col_path, row_idx)) = in_row {
                skipped_rows.insert((col_path.to_string(), row_idx));
            }
            visited.remove(&target_uuid);
            return;
        };

        let target_fields_by_id: BTreeMap<AssetUuid, &Field> =
            target_variant.fields.iter().filter(|f| f.id.is_v7()).map(|f| (f.id, f)).collect();

        // Unknown field UUIDs inside `fields` (`DW101`).
        for unknown_id in fields.keys() {
            if !target_fields_by_id.contains_key(unknown_id) {
                out.push(at_loc(
                    Diagnostic::warning(
                        "DW101",
                        format!(
                            "{row_prefix}field `{field_path}` variant `{}` contains value for unknown field UUID `{unknown_id}`",
                            target_variant.name
                        ),
                    ),
                    format!("{loc_prefix}.{unknown_id}"),
                ));
            }
        }

        let mut seen_target_fields: BTreeSet<AssetUuid> = BTreeSet::new();
        for t_field in &target_variant.fields {
            if !t_field.id.is_v7() || !seen_target_fields.insert(t_field.id) {
                continue;
            }
            let sub_loc = format!("{loc_prefix}.{}", t_field.name);
            let sub_path = format!("{field_path}.{}", t_field.name);

            let Some(sub_val) = fields.get(&t_field.id) else {
                out.push(at_loc(
                    Diagnostic::error(
                        "DE005",
                        format!(
                            "{row_prefix}field `{sub_path}` is missing required payload field `{}` for enum variant `{}`",
                            t_field.name, target_variant.name
                        ),
                    ),
                    sub_loc,
                ));
                if let Some((col_path, row_idx)) = in_row {
                    skipped_rows.insert((col_path.to_string(), row_idx));
                }
                continue;
            };

            self.validate_nested_field_value(
                t_field,
                sub_val,
                project,
                index,
                source_rel,
                &sub_loc,
                &sub_path,
                in_row,
                visited,
                depth,
                out,
                skipped_rows,
            );
        }

        visited.remove(&target_uuid);
    }

    #[allow(clippy::too_many_arguments)]
    fn validate_nested_field_value(
        &self,
        t_field: &Field,
        sub_val: &DataValue,
        project: Option<&Project>,
        index: &AssetIndex,
        source_rel: &str,
        sub_loc: &str,
        sub_path: &str,
        in_row: Option<(&str, usize)>,
        visited: &mut BTreeSet<AssetUuid>,
        depth: usize,
        out: &mut Vec<Diagnostic>,
        skipped_rows: &mut BTreeSet<(String, usize)>,
    ) {
        let row_prefix = match in_row {
            Some((_, row_idx)) => format!("row #{row_idx} "),
            None => String::new(),
        };
        let at_loc = |d: Diagnostic, loc: String| d.in_domain(DATA).in_asset(source_rel.to_string()).at(loc);

        if !t_field.ty.matches(sub_val) {
            out.push(at_loc(
                Diagnostic::error(
                    "DE005",
                    format!(
                        "{row_prefix}field `{sub_path}` value type `{}` does not match declared type `{}`",
                        sub_val.field_type().label(),
                        t_field.ty.label()
                    ),
                ),
                sub_loc.to_string(),
            ));
            if let Some((col_path, row_idx)) = in_row {
                skipped_rows.insert((col_path.to_string(), row_idx));
            }
            return;
        }

        if in_row.is_some() {
            if t_field.settings.required
                && let Some(s) = sub_val.as_str()
                && s.trim().is_empty()
            {
                out.push(at_loc(
                    Diagnostic::error("DE006", format!("{row_prefix}required field `{sub_path}` must not be empty")),
                    sub_loc.to_string(),
                ));
                if let Some((col_path, row_idx)) = in_row {
                    skipped_rows.insert((col_path.to_string(), row_idx));
                }
            }

            if t_field.settings.required
                && let (FieldType::SoftRef(_), DataValue::SoftRef(row_uuid)) = (&t_field.ty, sub_val)
                && row_uuid.is_nil()
            {
                out.push(at_loc(
                    Diagnostic::error(
                        "DE006",
                        format!("{row_prefix}required SoftRef field `{sub_path}` is unset (None)"),
                    ),
                    sub_loc.to_string(),
                ));
                if let Some((col_path, row_idx)) = in_row {
                    skipped_rows.insert((col_path.to_string(), row_idx));
                }
            }

            if t_field.settings.required
                && let Some(msg) = check_required_container(sub_path, &t_field.ty, sub_val)
            {
                out.push(at_loc(Diagnostic::error("DE006", format!("{row_prefix}{msg}")), sub_loc.to_string()));
                if let Some((col_path, row_idx)) = in_row {
                    skipped_rows.insert((col_path.to_string(), row_idx));
                }
            }

            if t_field.settings.required
                && let Some(msg) = check_required_asset_ref(sub_path, &t_field.ty, sub_val)
            {
                out.push(at_loc(Diagnostic::error("DE005", format!("{row_prefix}{msg}")), sub_loc.to_string()));
                if let Some((col_path, row_idx)) = in_row {
                    skipped_rows.insert((col_path.to_string(), row_idx));
                }
            }

            if let Some(max_len) = t_field.settings.max_len
                && let Some(s) = sub_val.as_str()
            {
                let len = s.chars().count();
                if len > max_len {
                    out.push(at_loc(
                        Diagnostic::error(
                            "DE006",
                            format!("{row_prefix}field `{sub_path}` length ({len} chars) exceeds max_len ({max_len})"),
                        ),
                        sub_loc.to_string(),
                    ));
                    if let Some((col_path, row_idx)) = in_row {
                        skipped_rows.insert((col_path.to_string(), row_idx));
                    }
                }
            }
        }

        if let Some(msg) = check_asset_ref_value(project, index, sub_path, &t_field.ty, sub_val) {
            out.push(at_loc(Diagnostic::error("DE008", format!("{row_prefix}{msg}")), sub_loc.to_string()));
            if let Some((col_path, row_idx)) = in_row {
                skipped_rows.insert((col_path.to_string(), row_idx));
            }
        }

        if let Some(msg) = self.check_soft_ref_value(index, sub_path, &t_field.ty, sub_val) {
            out.push(at_loc(Diagnostic::error("DE008", format!("{row_prefix}{msg}")), sub_loc.to_string()));
            if let Some((col_path, row_idx)) = in_row {
                skipped_rows.insert((col_path.to_string(), row_idx));
            }
        }

        if let Some(msg) = check_value_intrinsic_constraints(sub_path, sub_val) {
            out.push(at_loc(Diagnostic::error("DE006", format!("{row_prefix}{msg}")), sub_loc.to_string()));
            if let Some((col_path, row_idx)) = in_row {
                skipped_rows.insert((col_path.to_string(), row_idx));
            }
        }

        let bounds_valid = !matches!(
            (t_field.settings.min, t_field.settings.max),
            (Some(min), Some(max)) if min > max || !min.is_finite() || !max.is_finite()
        );
        if bounds_valid && let Some(msg) = check_soft_numeric_bounds(sub_path, t_field, sub_val) {
            out.push(at_loc(Diagnostic::warning("DW102", format!("{row_prefix}{msg}")), sub_loc.to_string()));
        }

        match (&t_field.ty, sub_val) {
            (FieldType::Dto(sub_target_uuid), DataValue::Dto(sub_map)) => {
                self.validate_nested_dto_map(
                    sub_map,
                    *sub_target_uuid,
                    project,
                    index,
                    source_rel,
                    sub_loc,
                    sub_path,
                    in_row,
                    visited,
                    depth + 1,
                    out,
                    skipped_rows,
                );
            }
            (FieldType::Enum(sub_target_uuid), DataValue::Enum { variant, fields }) => {
                self.validate_nested_enum_value(
                    *variant,
                    fields,
                    *sub_target_uuid,
                    project,
                    index,
                    source_rel,
                    sub_loc,
                    sub_path,
                    in_row,
                    visited,
                    depth + 1,
                    out,
                    skipped_rows,
                );
            }
            (FieldType::Option(_) | FieldType::Array(_) | FieldType::Set(_) | FieldType::Map(_, _), _) => {
                let mut container_visited = BTreeSet::new();
                self.validate_container_value(
                    &t_field.ty,
                    sub_val,
                    project,
                    index,
                    source_rel,
                    sub_loc,
                    sub_path,
                    in_row,
                    &mut container_visited,
                    depth + 1,
                    out,
                    skipped_rows,
                );
            }
            _ => {}
        }
    }

    fn mark_referenced_schemas<'a>(
        &'a self,
        ty: &FieldType,
        index: &'a AssetIndex,
        referenced_dtos: &mut BTreeSet<&'a str>,
        referenced_enums: &mut BTreeSet<&'a str>,
    ) {
        match ty {
            FieldType::Dto(target_uuid) | FieldType::SoftRef(target_uuid) => {
                if let Some((target_path, _)) = self.resolve_dto(*target_uuid, index) {
                    referenced_dtos.insert(target_path);
                }
            }
            FieldType::Enum(target_uuid) => {
                if let Some((target_path, _)) = self.resolve_enum(*target_uuid, index) {
                    referenced_enums.insert(target_path);
                }
            }
            FieldType::Option(inner) | FieldType::Array(inner) | FieldType::Set(inner) => {
                self.mark_referenced_schemas(inner, index, referenced_dtos, referenced_enums);
            }
            FieldType::Map(k, v) => {
                self.mark_referenced_schemas(k, index, referenced_dtos, referenced_enums);
                self.mark_referenced_schemas(v, index, referenced_dtos, referenced_enums);
            }
            _ => {}
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn validate_container_subtypes(
        &self,
        ty: &FieldType,
        source_rel: &str,
        qual_field_name: &str,
        project: Option<&Project>,
        index: &AssetIndex,
        field_is_cyclic_or_dangling: &mut bool,
        out: &mut Vec<Diagnostic>,
    ) {
        let at_ty =
            |d: Diagnostic| d.in_domain(DATA).in_asset(source_rel.to_string()).at(format!("{qual_field_name}.ty"));
        match ty {
            FieldType::Option(inner) | FieldType::Array(inner) => {
                self.validate_container_subtypes(
                    inner,
                    source_rel,
                    qual_field_name,
                    project,
                    index,
                    field_is_cyclic_or_dangling,
                    out,
                );
            }
            FieldType::Set(key_ty) => {
                if !key_ty.is_valid_key_type(&self.enums) {
                    *field_is_cyclic_or_dangling = true;
                    out.push(at_ty(Diagnostic::error(
                        "DE010",
                        format!(
                            "field `{qual_field_name}` uses invalid key type `{}` in `Set`; only Bool, I8..I64, U8..U64, String, LocalizedString, SoftRef, or unit-only Enum are allowed",
                            key_ty.describe(&self.dtos, &self.enums)
                        ),
                    )));
                }
                self.validate_container_subtypes(
                    key_ty,
                    source_rel,
                    qual_field_name,
                    project,
                    index,
                    field_is_cyclic_or_dangling,
                    out,
                );
            }
            FieldType::Map(key_ty, val_ty) => {
                if !key_ty.is_valid_key_type(&self.enums) {
                    *field_is_cyclic_or_dangling = true;
                    out.push(at_ty(Diagnostic::error(
                        "DE010",
                        format!(
                            "field `{qual_field_name}` uses invalid key type `{}` in `Map`; only Bool, I8..I64, U8..U64, String, LocalizedString, SoftRef, or unit-only Enum are allowed",
                            key_ty.describe(&self.dtos, &self.enums)
                        ),
                    )));
                }
                self.validate_container_subtypes(
                    key_ty,
                    source_rel,
                    qual_field_name,
                    project,
                    index,
                    field_is_cyclic_or_dangling,
                    out,
                );
                self.validate_container_subtypes(
                    val_ty,
                    source_rel,
                    qual_field_name,
                    project,
                    index,
                    field_is_cyclic_or_dangling,
                    out,
                );
            }
            FieldType::Dto(target_uuid) | FieldType::SoftRef(target_uuid) => {
                if self.resolve_dto(*target_uuid, index).is_none() {
                    *field_is_cyclic_or_dangling = true;
                    out.push(at_ty(Diagnostic::error(
                        "DE004",
                        format!("field `{qual_field_name}` references unknown DTO UUID `{target_uuid}`"),
                    )));
                }
            }
            FieldType::Enum(target_uuid) => {
                if self.resolve_enum(*target_uuid, index).is_none() {
                    *field_is_cyclic_or_dangling = true;
                    out.push(at_ty(Diagnostic::error(
                        "DE004",
                        format!("field `{qual_field_name}` references unknown enum UUID `{target_uuid}`"),
                    )));
                }
            }
            FieldType::LightEnvironment if project.is_some_and(|p| !p.domain_enabled(DomainId::new("light"))) => {
                out.push(at_ty(Diagnostic::error(
                    "DE008",
                    format!(
                        "field `{qual_field_name}` uses `LightEnvironment`, but the `light` domain is disabled in `project.ron`"
                    ),
                )));
            }
            _ => {}
        }
    }

    fn has_duplicate_container_keys(&self, ty: &FieldType, val: &DataValue) -> bool {
        match (ty, val) {
            (FieldType::Option(inner_ty), DataValue::Option(Some(inner_val))) => {
                self.has_duplicate_container_keys(inner_ty, inner_val)
            }
            (FieldType::Array(inner_ty), DataValue::Array(items)) => {
                items.iter().any(|item| self.has_duplicate_container_keys(inner_ty, item))
            }
            (FieldType::Set(key_ty), DataValue::Set(items)) => {
                if !key_ty.is_valid_key_type(&self.enums) {
                    return false;
                }
                items.iter().enumerate().any(|(i, item)| {
                    items[..i]
                        .iter()
                        .any(|prev| DataValue::cmp_key(prev, item, &self.enums, key_ty.enum_target()).is_eq())
                })
            }
            (FieldType::Map(key_ty, val_ty), DataValue::Map(entries)) => {
                if !key_ty.is_valid_key_type(&self.enums) {
                    return false;
                }
                let dup_key = entries.iter().enumerate().any(|(i, (k, _))| {
                    entries[..i]
                        .iter()
                        .any(|(prev_k, _)| DataValue::cmp_key(prev_k, k, &self.enums, key_ty.enum_target()).is_eq())
                });
                dup_key || entries.iter().any(|(_, v)| self.has_duplicate_container_keys(val_ty, v))
            }
            _ => false,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn validate_container_value(
        &self,
        ty: &FieldType,
        val: &DataValue,
        project: Option<&Project>,
        index: &AssetIndex,
        source_rel: &str,
        loc_prefix: &str,
        field_path: &str,
        in_row: Option<(&str, usize)>,
        visited: &mut BTreeSet<AssetUuid>,
        depth: usize,
        out: &mut Vec<Diagnostic>,
        skipped_rows: &mut BTreeSet<(String, usize)>,
    ) {
        let row_prefix = match in_row {
            Some((_, row_idx)) => format!("row #{row_idx} "),
            None => String::new(),
        };
        let at_loc = |d: Diagnostic, loc: String| d.in_domain(DATA).in_asset(source_rel.to_string()).at(loc);

        if depth > MAX_NESTING_DEPTH {
            out.push(at_loc(
                Diagnostic::error(
                    "DE009",
                    format!(
                        "{row_prefix}field `{field_path}` nested container value exceeds maximum depth of {MAX_NESTING_DEPTH}"
                    ),
                ),
                loc_prefix.to_string(),
            ));
            if let Some((col_path, row_idx)) = in_row {
                skipped_rows.insert((col_path.to_string(), row_idx));
            }
            return;
        }

        match (ty, val) {
            (FieldType::Option(_), DataValue::Option(None)) => {}
            (FieldType::Option(inner_ty), DataValue::Option(Some(inner_val))) => {
                let mut inner_visited = BTreeSet::new();
                self.validate_container_value(
                    inner_ty,
                    inner_val,
                    project,
                    index,
                    source_rel,
                    loc_prefix,
                    field_path,
                    in_row,
                    &mut inner_visited,
                    depth + 1,
                    out,
                    skipped_rows,
                );
            }
            (FieldType::Array(inner_ty), DataValue::Array(items)) => {
                for (i, item) in items.iter().enumerate() {
                    let item_loc = format!("{loc_prefix}[{i}]");
                    let item_path = format!("{field_path}[{i}]");
                    let mut inner_visited = BTreeSet::new();
                    self.validate_container_value(
                        inner_ty,
                        item,
                        project,
                        index,
                        source_rel,
                        &item_loc,
                        &item_path,
                        in_row,
                        &mut inner_visited,
                        depth + 1,
                        out,
                        skipped_rows,
                    );
                }
            }
            (FieldType::Set(key_ty), DataValue::Set(items)) => {
                if key_ty.is_valid_key_type(&self.enums) {
                    for (i, item) in items.iter().enumerate() {
                        if items[..i]
                            .iter()
                            .any(|prev| DataValue::cmp_key(prev, item, &self.enums, key_ty.enum_target()).is_eq())
                        {
                            out.push(at_loc(
                                Diagnostic::error(
                                    "DE010",
                                    format!(
                                        "{row_prefix}field `{field_path}` contains duplicate Set key at index #{i}"
                                    ),
                                ),
                                format!("{loc_prefix}[{i}]"),
                            ));
                            if let Some((col_path, row_idx)) = in_row {
                                skipped_rows.insert((col_path.to_string(), row_idx));
                            }
                        }
                    }
                }
                for (i, item) in items.iter().enumerate() {
                    let item_loc = format!("{loc_prefix}[{i}]");
                    let item_path = format!("{field_path}[{i}]");
                    let mut inner_visited = BTreeSet::new();
                    self.validate_container_value(
                        key_ty,
                        item,
                        project,
                        index,
                        source_rel,
                        &item_loc,
                        &item_path,
                        in_row,
                        &mut inner_visited,
                        depth + 1,
                        out,
                        skipped_rows,
                    );
                }
            }
            (FieldType::Map(key_ty, val_ty), DataValue::Map(entries)) => {
                if key_ty.is_valid_key_type(&self.enums) {
                    for (i, (k, _)) in entries.iter().enumerate() {
                        if entries[..i]
                            .iter()
                            .any(|(prev_k, _)| DataValue::cmp_key(prev_k, k, &self.enums, key_ty.enum_target()).is_eq())
                        {
                            out.push(at_loc(
                                Diagnostic::error(
                                    "DE010",
                                    format!(
                                        "{row_prefix}field `{field_path}` contains duplicate Map key at entry #{i}"
                                    ),
                                ),
                                format!("{loc_prefix}[{i}].key"),
                            ));
                            if let Some((col_path, row_idx)) = in_row {
                                skipped_rows.insert((col_path.to_string(), row_idx));
                            }
                        }
                    }
                }
                for (i, (k, v)) in entries.iter().enumerate() {
                    let key_loc = format!("{loc_prefix}[{i}].key");
                    let key_path = format!("{field_path}[{i}].key");
                    let mut key_visited = BTreeSet::new();
                    self.validate_container_value(
                        key_ty,
                        k,
                        project,
                        index,
                        source_rel,
                        &key_loc,
                        &key_path,
                        in_row,
                        &mut key_visited,
                        depth + 1,
                        out,
                        skipped_rows,
                    );
                    let val_loc = format!("{loc_prefix}[{i}].val");
                    let val_path = format!("{field_path}[{i}].val");
                    let mut val_visited = BTreeSet::new();
                    self.validate_container_value(
                        val_ty,
                        v,
                        project,
                        index,
                        source_rel,
                        &val_loc,
                        &val_path,
                        in_row,
                        &mut val_visited,
                        depth + 1,
                        out,
                        skipped_rows,
                    );
                }
            }
            (FieldType::Dto(target_uuid), DataValue::Dto(map)) => {
                self.validate_nested_dto_map(
                    map,
                    *target_uuid,
                    project,
                    index,
                    source_rel,
                    loc_prefix,
                    field_path,
                    in_row,
                    visited,
                    depth,
                    out,
                    skipped_rows,
                );
            }
            (FieldType::Enum(target_uuid), DataValue::Enum { variant, fields }) => {
                self.validate_nested_enum_value(
                    *variant,
                    fields,
                    *target_uuid,
                    project,
                    index,
                    source_rel,
                    loc_prefix,
                    field_path,
                    in_row,
                    visited,
                    depth,
                    out,
                    skipped_rows,
                );
            }
            _ => {
                if let Some(msg) = check_asset_ref_value(project, index, field_path, ty, val) {
                    out.push(at_loc(Diagnostic::error("DE008", format!("{row_prefix}{msg}")), loc_prefix.to_string()));
                    if let Some((col_path, row_idx)) = in_row {
                        skipped_rows.insert((col_path.to_string(), row_idx));
                    }
                }
                if let Some(msg) = self.check_soft_ref_value(index, field_path, ty, val) {
                    out.push(at_loc(Diagnostic::error("DE008", format!("{row_prefix}{msg}")), loc_prefix.to_string()));
                    if let Some((col_path, row_idx)) = in_row {
                        skipped_rows.insert((col_path.to_string(), row_idx));
                    }
                }
                if let Some(msg) = check_value_intrinsic_constraints(field_path, val) {
                    out.push(at_loc(Diagnostic::error("DE006", format!("{row_prefix}{msg}")), loc_prefix.to_string()));
                    if let Some((col_path, row_idx)) = in_row {
                        skipped_rows.insert((col_path.to_string(), row_idx));
                    }
                }
            }
        }
    }

    fn check_soft_ref_value(
        &self,
        index: &AssetIndex,
        field_name: &str,
        ty: &FieldType,
        val: &DataValue,
    ) -> Option<String> {
        let (FieldType::SoftRef(target_dto_uuid), DataValue::SoftRef(row_uuid)) = (ty, val) else {
            return None;
        };
        if row_uuid.is_nil() {
            return None;
        }
        let (target_dto_name, _) = self.resolve_dto(*target_dto_uuid, index)?;
        for (col_path, col) in &self.collections {
            if col.rows.iter().any(|r| !r.id.is_nil() && r.id == *row_uuid) {
                if col.dto != *target_dto_uuid {
                    let actual_dto_name = self
                        .resolve_dto(col.dto, index)
                        .map(|(p, _)| p.to_string())
                        .unwrap_or_else(|| col.dto.to_string());
                    return Some(format!(
                        "field `{field_name}` SoftRef<{target_dto_name}> references row {row_uuid} in collection '{col_path}' of type '{actual_dto_name}', expected '{target_dto_name}'"
                    ));
                }
                return None;
            }
        }
        Some(format!("field `{field_name}` SoftRef references unknown row UUID `{row_uuid}`"))
    }
}

fn collect_type_refs(ty: &FieldType, loc: &str, out: &mut Vec<IndexedRef>) {
    match ty {
        FieldType::Dto(target_uuid) | FieldType::Enum(target_uuid) | FieldType::SoftRef(target_uuid) => {
            if !target_uuid.is_nil() {
                out.push(IndexedRef { to: *target_uuid, sub: None, location: Some(loc.to_string()) });
            }
        }
        FieldType::Option(inner) | FieldType::Array(inner) | FieldType::Set(inner) => {
            collect_type_refs(inner, loc, out);
        }
        FieldType::Map(k, v) => {
            collect_type_refs(k, loc, out);
            collect_type_refs(v, loc, out);
        }
        _ => {}
    }
}

fn collect_value_refs(val: &DataValue, loc: &str, out: &mut Vec<IndexedRef>) {
    match val {
        DataValue::Curve(id)
        | DataValue::Material(id)
        | DataValue::Texture(id)
        | DataValue::LightEnvironment(id)
        | DataValue::SoftRef(id) => {
            if !id.is_nil() {
                out.push(IndexedRef { to: *id, sub: None, location: Some(loc.to_string()) });
            }
        }
        DataValue::Gradient(id, slot) => {
            if !id.is_nil() {
                out.push(IndexedRef {
                    to: *id,
                    sub: (*slot != 0).then_some(u64::from(*slot)),
                    location: Some(loc.to_string()),
                });
            }
        }
        DataValue::Dto(map) => {
            for (k, v) in map {
                collect_value_refs(v, &format!("{loc}.{k}"), out);
            }
        }
        DataValue::Enum { fields, .. } => {
            for (k, v) in fields {
                collect_value_refs(v, &format!("{loc}.{k}"), out);
            }
        }
        DataValue::Option(Some(inner)) => {
            collect_value_refs(inner, loc, out);
        }
        DataValue::Array(items) | DataValue::Set(items) => {
            for (i, item) in items.iter().enumerate() {
                collect_value_refs(item, &format!("{loc}[{i}]"), out);
            }
        }
        DataValue::Map(entries) => {
            for (i, (k, v)) in entries.iter().enumerate() {
                collect_value_refs(k, &format!("{loc}[{i}].key"), out);
                collect_value_refs(v, &format!("{loc}[{i}].val"), out);
            }
        }
        _ => {}
    }
}

/// Checks whether `path` is a safe relative path under `<game_root>/assets/` outside `assets/quillshade/`.
pub fn is_safe_game_asset_path(path: &str) -> bool {
    if path.is_empty()
        || path.starts_with('/')
        || path.starts_with('\\')
        || path.ends_with('/')
        || path.contains('\\')
        || path == "quillshade"
        || path.starts_with("quillshade/")
    {
        return false;
    }
    path.split('/').all(|seg| !seg.is_empty() && seg != "." && seg != "..")
}

fn check_required_container(field_name: &str, ty: &FieldType, val: &DataValue) -> Option<String> {
    match (ty, val) {
        (FieldType::Option(_), DataValue::Option(None)) => {
            Some(format!("required Option field `{field_name}` must not be None"))
        }
        (FieldType::Array(_), DataValue::Array(items)) if items.is_empty() => {
            Some(format!("required Array field `{field_name}` must not be empty"))
        }
        (FieldType::Set(_), DataValue::Set(items)) if items.is_empty() => {
            Some(format!("required Set field `{field_name}` must not be empty"))
        }
        (FieldType::Map(_, _), DataValue::Map(entries)) if entries.is_empty() => {
            Some(format!("required Map field `{field_name}` must not be empty"))
        }
        _ => None,
    }
}

fn check_required_asset_ref(field_name: &str, ty: &FieldType, val: &DataValue) -> Option<String> {
    match (ty, val) {
        (FieldType::Curve, DataValue::Curve(id))
        | (FieldType::Material, DataValue::Material(id))
        | (FieldType::Texture, DataValue::Texture(id))
        | (FieldType::LightEnvironment, DataValue::LightEnvironment(id))
            if id.is_nil() =>
        {
            Some(format!("required field `{field_name}` ({}) must not be None", ty.label()))
        }
        (FieldType::Gradient, DataValue::Gradient(id, slot)) if id.is_nil() || *slot == 0 => {
            Some(format!("required field `{field_name}` (Gradient) must reference a gradient and non-zero slot"))
        }
        (FieldType::GameAsset(_), DataValue::GameAsset(path)) if path.trim().is_empty() => {
            Some(format!("required field `{field_name}` (GameAsset) must not be empty"))
        }
        _ => None,
    }
}

fn check_asset_ref_value(
    project: Option<&Project>,
    index: &AssetIndex,
    field_name: &str,
    ty: &FieldType,
    val: &DataValue,
) -> Option<String> {
    match (ty, val) {
        (FieldType::GameAsset(kind), DataValue::GameAsset(path)) => {
            if path.is_empty() {
                return None;
            }
            if !is_safe_game_asset_path(path) {
                return Some(format!(
                    "field `{field_name}` game asset path `{path}` must be a safe relative path under `assets/` outside `quillshade/`"
                ));
            }
            if !kind.matches_path(path) {
                return Some(format!(
                    "field `{field_name}` game asset path `{path}` does not match `GameAssetKind::{}` allowed extensions ({})",
                    kind.label(),
                    kind.extensions().join(", ")
                ));
            }
            if let Some(proj) = project
                && !proj.root.join("assets").join(path).is_file()
            {
                return Some(format!("field `{field_name}` game asset `assets/{path}` does not exist on disk"));
            }
            None
        }
        (FieldType::Gradient, DataValue::Gradient(id, slot)) => {
            if id.is_nil() && *slot == 0 {
                return None;
            }
            if id.is_nil() && *slot != 0 {
                return Some(format!("field `{field_name}` gradient reference has nil UUID with non-zero slot {slot}"));
            }
            if !id.is_nil() && *slot == 0 {
                return Some(format!(
                    "field `{field_name}` gradient reference `{id}` must specify a non-zero slot (got 0)"
                ));
            }
            check_cross_domain_uuid_ref(project, index, field_name, ty, *id, Some(*slot))
        }
        (FieldType::Curve, DataValue::Curve(id))
        | (FieldType::Material, DataValue::Material(id))
        | (FieldType::Texture, DataValue::Texture(id))
        | (FieldType::LightEnvironment, DataValue::LightEnvironment(id)) => {
            if id.is_nil() {
                return None;
            }
            check_cross_domain_uuid_ref(project, index, field_name, ty, *id, None)
        }
        _ => None,
    }
}

fn check_cross_domain_uuid_ref(
    project: Option<&Project>,
    index: &AssetIndex,
    field_name: &str,
    ty: &FieldType,
    uuid: AssetUuid,
    gradient_slot: Option<u32>,
) -> Option<String> {
    if !uuid.is_v7() {
        return Some(format!("field `{field_name}` {} reference `{uuid}` is not a valid UUIDv7", ty.label()));
    }
    let Some(entry) = index.get(uuid) else {
        return Some(format!("field `{field_name}` references unknown {} asset UUID `{uuid}`", ty.label()));
    };
    let d = entry.asset.domain.as_str();
    let s = entry.asset.suffix;
    let kind_ok = match ty {
        FieldType::Curve => (d == "data" || d == "material" || d == "light") && (s == "curve" || s == ".curve.ron"),
        FieldType::Gradient => d == "material" && (s == "gradients" || s == ".gradients.ron"),
        FieldType::Material => d == "material" && (s == "material" || s == ".material.ron"),
        FieldType::Texture => (d == "material" || d == "light") && (s == "texture" || s == ".texture.ron"),
        FieldType::LightEnvironment => d == "light" && (s == "environment" || s == ".environment.ron"),
        _ => true,
    };
    if !kind_ok {
        return Some(format!(
            "field `{field_name}` expects a `{}` asset, but `{uuid}` is `{}` (`{}:{}`)",
            ty.label(),
            entry.kind_id(),
            entry.asset.domain,
            entry.asset.path
        ));
    }
    if project.is_some_and(|p| !p.domain_enabled(entry.asset.domain)) {
        return Some(format!(
            "field `{field_name}` references `{}:{}` in disabled domain `{}`",
            entry.asset.domain, entry.asset.path, entry.asset.domain
        ));
    }
    if let Some(slot) = gradient_slot
        && slot != 0
        && !entry.sub_items.iter().any(|sub| sub.id == u64::from(slot))
    {
        return Some(format!(
            "field `{field_name}` gradient `{}:{}` has no slot {slot}",
            entry.asset.domain, entry.asset.path
        ));
    }
    None
}

/// Computes the nesting depth of container (`Option`, `Array`, `Set`, `Map`) type wrappers.
fn type_nesting_depth(ty: &FieldType) -> usize {
    match ty {
        FieldType::Option(inner) | FieldType::Array(inner) | FieldType::Set(inner) => 1 + type_nesting_depth(inner),
        FieldType::Map(k, v) => 1 + type_nesting_depth(k).max(type_nesting_depth(v)),
        _ => 0,
    }
}

/// Computes the nesting depth of `DataValue::Dto`, `DataValue::Enum`, and container values (`0` for scalar/ref values).
fn value_nesting_depth(val: &DataValue) -> usize {
    match val {
        DataValue::Dto(map) => 1 + map.values().map(value_nesting_depth).max().unwrap_or(0),
        DataValue::Enum { fields, .. } => 1 + fields.values().map(value_nesting_depth).max().unwrap_or(0),
        DataValue::Option(Some(inner)) => 1 + value_nesting_depth(inner),
        DataValue::Array(items) | DataValue::Set(items) => 1 + items.iter().map(value_nesting_depth).max().unwrap_or(0),
        DataValue::Map(entries) => {
            1 + entries.iter().map(|(k, v)| value_nesting_depth(k).max(value_nesting_depth(v))).max().unwrap_or(0)
        }
        _ => 0,
    }
}

fn check_value_intrinsic_constraints(field_name: &str, val: &DataValue) -> Option<String> {
    match val {
        DataValue::RangeF32([min, max]) if !min.is_finite() || !max.is_finite() || min > max => {
            Some(format!("field `{field_name}` range min ({min}) must be <= max ({max}) and finite"))
        }
        DataValue::RangeI32([min, max]) if min > max => {
            Some(format!("field `{field_name}` range min ({min}) must be <= max ({max})"))
        }
        DataValue::Duration(secs) if !secs.is_finite() || *secs < 0.0 => {
            Some(format!("field `{field_name}` duration ({secs}) must be finite and >= 0"))
        }
        _ => None,
    }
}

fn check_soft_numeric_bounds(field_name: &str, field: &Field, val: &DataValue) -> Option<String> {
    let v = val.as_f64()?;
    if let Some(min) = field.settings.min
        && v < min
    {
        return Some(format!("field `{field_name}` value {v} is below soft minimum {min}"));
    }
    if let Some(max) = field.settings.max
        && v > max
    {
        return Some(format!("field `{field_name}` value {v} exceeds soft maximum {max}"));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::model::{COLLECTION_FORMAT_VERSION, DTO_FORMAT_VERSION, FieldSettings, FieldType, Row};

    fn test_uuid(n: u128) -> AssetUuid {
        AssetUuid::from_u128(0x0196_3a10_0000_7000_8000_0000_0000_0000 | n)
    }

    fn valid_book_dto() -> DtoAsset {
        DtoAsset {
            format_version: DTO_FORMAT_VERSION,
            id: test_uuid(1),
            doc: "Book".into(),
            fields: vec![
                Field {
                    id: test_uuid(101),
                    name: "title".into(),
                    ty: FieldType::LocalizedString,
                    category: "Identity".into(),
                    is_name: true,
                    default: DataValue::LocalizedString("Book".into()),
                    settings: FieldSettings {
                        required: true,
                        max_len: Some(20),
                        unique_in_collection: true,
                        ..FieldSettings::default()
                    },
                },
                Field {
                    id: test_uuid(102),
                    name: "pages".into(),
                    ty: FieldType::U32,
                    category: "Physical".into(),
                    is_name: true,
                    default: DataValue::U32(100),
                    settings: FieldSettings { min: Some(10.0), max: Some(500.0), ..FieldSettings::default() },
                },
            ],
        }
    }

    #[test]
    fn validates_clean_assets_and_reports_dw103_dw104() {
        let mut assets = DataAssets::default();
        assets.dtos.insert("library/book".into(), valid_book_dto());
        assets.dtos.insert(
            "library/empty_unused".into(),
            DtoAsset { format_version: DTO_FORMAT_VERSION, id: test_uuid(2), doc: String::new(), fields: Vec::new() },
        );
        assets.collections.insert(
            "library/books_core".into(),
            CollectionAsset {
                format_version: COLLECTION_FORMAT_VERSION,
                id: test_uuid(10),
                dto: test_uuid(1),
                rows: Vec::new(),
            },
        );

        let report = assets.validate(&AssetIndex::default());
        assert!(!report.has_errors(), "{:?}", report.diagnostics);
        let codes: Vec<_> = report.diagnostics.iter().map(|d| (d.code.as_str(), d.asset.as_deref().unwrap())).collect();
        assert_eq!(
            codes,
            vec![
                ("DW103", "quillshade/data/library/empty_unused.dto.ron"),
                ("DW104", "quillshade/data/library/empty_unused.dto.ron"),
                ("DW103", "quillshade/data/library/books_core.collection.ron"),
            ]
        );
    }

    #[test]
    fn validates_de001_de002_de003_de004_de005_de006_dw101_dw102() {
        let mut dto = valid_book_dto();
        // Add duplicate field UUID (`DE002`), invalid field name / keyword (`DE003`), duplicate field name (`DE003`),
        // mismatched default (`DE005`), and inverted min > max (`DE006`).
        dto.fields.push(Field {
            id: test_uuid(101),    // duplicate of `title`
            name: "struct".into(), // Rust keyword -> DE003
            ty: FieldType::I32,
            category: String::new(),
            is_name: false,
            default: DataValue::Bool(true), // wrong type -> DE005
            settings: FieldSettings { min: Some(50.0), max: Some(10.0), ..FieldSettings::default() }, // min > max -> DE006
        });
        dto.fields.push(Field {
            id: test_uuid(103),
            name: "title".into(), // duplicate name -> DE003
            ty: FieldType::RangeF32,
            category: String::new(),
            is_name: false,
            default: DataValue::RangeF32([5.0, 1.0]), // min > max -> DE006
            settings: FieldSettings::default(),
        });

        let mut assets = DataAssets::default();
        assets.dtos.insert("library/book".into(), dto);

        // Collection 1 (`library/a_books`) and Collection 2 (`library/b_books`) share a duplicate Row.id (`test_uuid(500)`).
        // Per D5, `library/a_books` row #0 wins; `library/b_books` row #0 gets `DE002` and is added to `skipped_rows`.
        let mut row_a0 = Row { id: test_uuid(500), values: BTreeMap::new() };
        row_a0.values.insert(test_uuid(101), DataValue::LocalizedString("Dune".into()));
        row_a0.values.insert(test_uuid(102), DataValue::U32(900)); // > 500 soft max -> DW102
        row_a0.values.insert(test_uuid(999), DataValue::Bool(false)); // unknown field -> DW101

        // Row a1 violates `unique_in_collection` ("Dune") and `max_len` / `required` -> DE006.
        let mut row_a1 = Row { id: test_uuid(501), values: BTreeMap::new() };
        row_a1.values.insert(test_uuid(101), DataValue::LocalizedString("Dune".into()));
        row_a1.values.insert(test_uuid(102), DataValue::String("wrong_type".into())); // DE005

        let mut row_a2 = Row { id: test_uuid(502), values: BTreeMap::new() };
        row_a2.values.insert(test_uuid(101), DataValue::LocalizedString("".into())); // required empty -> DE006
        row_a2.values.insert(test_uuid(102), DataValue::U32(100));

        let mut row_a3 = Row { id: test_uuid(503), values: BTreeMap::new() };
        row_a3.values.insert(
            test_uuid(101),
            DataValue::LocalizedString("This title is definitely longer than twenty chars".into()),
        ); // max_len -> DE006
        row_a3.values.insert(test_uuid(102), DataValue::U32(100));

        assets.collections.insert(
            "library/a_books".into(),
            CollectionAsset {
                format_version: COLLECTION_FORMAT_VERSION,
                id: test_uuid(10),
                dto: test_uuid(1),
                rows: vec![row_a0, row_a1, row_a2, row_a3],
            },
        );

        let mut row_b0 = Row { id: test_uuid(500), values: BTreeMap::new() }; // duplicate of row_a0 -> DE002
        row_b0.values.insert(test_uuid(101), DataValue::LocalizedString("Neuromancer".into()));
        row_b0.values.insert(test_uuid(102), DataValue::U32(200));

        assets.collections.insert(
            "library/b_books".into(),
            CollectionAsset {
                format_version: COLLECTION_FORMAT_VERSION,
                id: test_uuid(11),
                dto: test_uuid(1),
                rows: vec![row_b0],
            },
        );

        // Collection 3 (`library/orphan`) references unknown DTO -> DE004, and has nil `id` -> DE001.
        assets.collections.insert(
            "library/orphan".into(),
            CollectionAsset {
                format_version: COLLECTION_FORMAT_VERSION,
                id: AssetUuid::NIL,
                dto: test_uuid(888),
                rows: vec![Row { id: test_uuid(600), values: BTreeMap::new() }],
            },
        );

        let report = assets.validate(&AssetIndex::default());
        assert!(report.has_errors());
        let codes: BTreeSet<&str> = report.diagnostics.iter().map(|d| d.code.as_str()).collect();
        for expected in ["DE001", "DE002", "DE003", "DE004", "DE005", "DE006", "DW101", "DW102"] {
            assert!(codes.contains(expected), "missing {expected} in {:?}", report.diagnostics);
        }

        // Verify D5 tie-breaker: `library/a_books` row #0 is NOT skipped (it only had DW101/DW102 warnings),
        // while `library/a_books` rows #1, #2, #3 (DE005/DE006) and `library/b_books` row #0 (DE002 loser) ARE skipped!
        assert!(!report.skipped_rows.contains(&("library/a_books".to_string(), 0)));
        assert!(report.skipped_rows.contains(&("library/a_books".to_string(), 1)));
        assert!(report.skipped_rows.contains(&("library/a_books".to_string(), 2)));
        assert!(report.skipped_rows.contains(&("library/a_books".to_string(), 3)));
        assert!(report.skipped_rows.contains(&("library/b_books".to_string(), 0)));
    }

    fn simple_dto(id: u128, field_id: u128, name: &str, ty: FieldType) -> DtoAsset {
        let default = ty.default_value();
        DtoAsset {
            format_version: DTO_FORMAT_VERSION,
            id: test_uuid(id),
            doc: String::new(),
            fields: vec![Field {
                id: test_uuid(field_id),
                name: name.into(),
                ty,
                category: String::new(),
                is_name: true,
                default,
                settings: FieldSettings::default(),
            }],
        }
    }

    #[test]
    fn detects_direct_and_indirect_dto_cycles_and_allows_diamond_dag() {
        // Direct cycle: A -> A
        let mut direct = DataAssets::default();
        direct.dtos.insert("a".into(), simple_dto(1, 11, "self_ref", FieldType::Dto(test_uuid(1))));
        let report = direct.validate(&AssetIndex::default());
        assert!(
            report.diagnostics.iter().any(|d| d.code == "DE003" && d.message.contains("a -> a")),
            "{:?}",
            report.diagnostics
        );

        // Indirect 3-node cycle: A -> B -> C -> A
        let mut indirect = DataAssets::default();
        indirect.dtos.insert("a".into(), simple_dto(1, 11, "b", FieldType::Dto(test_uuid(2))));
        indirect.dtos.insert("b".into(), simple_dto(2, 21, "c", FieldType::Dto(test_uuid(3))));
        indirect.dtos.insert("c".into(), simple_dto(3, 31, "a", FieldType::Dto(test_uuid(1))));
        let report = indirect.validate(&AssetIndex::default());
        assert!(
            report.diagnostics.iter().any(|d| d.code == "DE003" && d.message.contains("a -> b -> c -> a")),
            "{:?}",
            report.diagnostics
        );

        // Diamond DAG reuse: A -> (B, C), B -> D, C -> D (valid, no DE003, and D/B/C get no DW104 when A has collection)
        let mut diamond = DataAssets::default();
        diamond.dtos.insert("d".into(), simple_dto(4, 41, "val", FieldType::I32));
        diamond.dtos.insert("b".into(), simple_dto(2, 21, "d", FieldType::Dto(test_uuid(4))));
        diamond.dtos.insert("c".into(), simple_dto(3, 31, "d", FieldType::Dto(test_uuid(4))));
        diamond.dtos.insert(
            "a".into(),
            DtoAsset {
                format_version: DTO_FORMAT_VERSION,
                id: test_uuid(1),
                doc: String::new(),
                fields: vec![
                    Field {
                        id: test_uuid(11),
                        name: "left".into(),
                        ty: FieldType::Dto(test_uuid(2)),
                        category: String::new(),
                        is_name: true,
                        default: DataValue::Dto(BTreeMap::new()),
                        settings: FieldSettings::default(),
                    },
                    Field {
                        id: test_uuid(12),
                        name: "right".into(),
                        ty: FieldType::Dto(test_uuid(3)),
                        category: String::new(),
                        is_name: false,
                        default: DataValue::Dto(BTreeMap::new()),
                        settings: FieldSettings::default(),
                    },
                ],
            },
        );
        diamond.collections.insert(
            "items".into(),
            CollectionAsset {
                format_version: COLLECTION_FORMAT_VERSION,
                id: test_uuid(100),
                dto: test_uuid(1),
                rows: Vec::new(),
            },
        );
        let report = diamond.validate(&AssetIndex::default());
        assert!(!report.has_errors(), "{:?}", report.diagnostics);
        assert!(!report.diagnostics.iter().any(|d| d.code == "DW104"), "{:?}", report.diagnostics);
    }

    #[test]
    fn detects_de009_nesting_depth_over_64_and_de004_dangling_dto() {
        let mut assets = DataAssets::default();
        // Build a 66-DTO chain d0 -> d1 -> ... -> d65 (65 edges from d0, exceeding MAX_NESTING_DEPTH = 64).
        let leaf_idx = (MAX_NESTING_DEPTH + 1) as u128;
        assets.dtos.insert(format!("d{leaf_idx}"), simple_dto(1000 + leaf_idx, 2000 + leaf_idx, "v", FieldType::I32));
        for i in (0..leaf_idx).rev() {
            let target_id = test_uuid(1000 + i + 1);
            assets.dtos.insert(format!("d{i}"), simple_dto(1000 + i, 2000 + i, "next", FieldType::Dto(target_id)));
        }
        // Also add a DTO with a dangling FieldType::Dto reference -> DE004.
        assets.dtos.insert("dangling".into(), simple_dto(9000, 9001, "missing", FieldType::Dto(test_uuid(9999))));

        let report = assets.validate(&AssetIndex::default());
        assert!(
            report
                .diagnostics
                .iter()
                .any(|d| d.code == "DE009" && d.asset.as_deref() == Some("quillshade/data/d0.dto.ron")),
            "{:?}",
            report.diagnostics
        );
        assert!(
            !report
                .diagnostics
                .iter()
                .any(|d| d.code == "DE009" && d.asset.as_deref() == Some("quillshade/data/d1.dto.ron")),
            "d1 has depth 64 and should not trigger DE009: {:?}",
            report.diagnostics
        );
        assert!(
            report.diagnostics.iter().any(|d| d.code == "DE004" && d.location.as_deref() == Some("missing.ty")),
            "{:?}",
            report.diagnostics
        );
    }

    #[test]
    fn validates_nested_dto_values_in_collection_rows() {
        let author_dto = DtoAsset {
            format_version: DTO_FORMAT_VERSION,
            id: test_uuid(2),
            doc: String::new(),
            fields: vec![
                Field {
                    id: test_uuid(201),
                    name: "name".into(),
                    ty: FieldType::String,
                    category: String::new(),
                    is_name: true,
                    default: DataValue::String("Anon".into()),
                    settings: FieldSettings { required: true, max_len: Some(10), ..FieldSettings::default() },
                },
                Field {
                    id: test_uuid(202),
                    name: "score".into(),
                    ty: FieldType::I32,
                    category: String::new(),
                    is_name: false,
                    default: DataValue::I32(5),
                    settings: FieldSettings { min: Some(0.0), max: Some(10.0), ..FieldSettings::default() },
                },
            ],
        };
        let book_dto = DtoAsset {
            format_version: DTO_FORMAT_VERSION,
            id: test_uuid(1),
            doc: String::new(),
            fields: vec![Field {
                id: test_uuid(101),
                name: "author".into(),
                ty: FieldType::Dto(test_uuid(2)),
                category: String::new(),
                is_name: true,
                default: DataValue::Dto(BTreeMap::new()),
                settings: FieldSettings::default(),
            }],
        };

        let mut assets = DataAssets::default();
        assets.dtos.insert("author".into(), author_dto);
        assets.dtos.insert("book".into(), book_dto);

        // Row 0: unknown nested key (DW101), missing nested `score` (DW102), valid `name` -> warnings only, NOT skipped.
        let mut map0 = BTreeMap::new();
        map0.insert(test_uuid(201), DataValue::String("Vance".into()));
        map0.insert(test_uuid(999), DataValue::Bool(true)); // DW101
        let mut row0 = Row { id: test_uuid(500), values: BTreeMap::new() };
        row0.values.insert(test_uuid(101), DataValue::Dto(map0));

        // Row 1: wrong nested type for `score` (DE005) and empty required `name` (DE006) -> skipped!
        let mut map1 = BTreeMap::new();
        map1.insert(test_uuid(201), DataValue::String("".into())); // DE006
        map1.insert(test_uuid(202), DataValue::Bool(false)); // DE005
        let mut row1 = Row { id: test_uuid(501), values: BTreeMap::new() };
        row1.values.insert(test_uuid(101), DataValue::Dto(map1));

        assets.collections.insert(
            "books".into(),
            CollectionAsset {
                format_version: COLLECTION_FORMAT_VERSION,
                id: test_uuid(10),
                dto: test_uuid(1),
                rows: vec![row0, row1],
            },
        );

        let report = assets.validate(&AssetIndex::default());
        let codes: BTreeSet<&str> = report.diagnostics.iter().map(|d| d.code.as_str()).collect();
        for expected in ["DE005", "DE006", "DW101", "DW102"] {
            assert!(codes.contains(expected), "missing {expected} in {:?}", report.diagnostics);
        }
        assert!(!report.skipped_rows.contains(&("books".to_string(), 0)));
        assert!(report.skipped_rows.contains(&("books".to_string(), 1)));
    }

    #[test]
    fn index_part_and_de008_validate_direct_asset_refs() {
        use crate::backend::model::GameAssetKind;
        use project_core::layout::{ProjectSettings, save_settings};

        let curve_id = test_uuid(10);
        let grad_id = test_uuid(11);
        let mat_id = test_uuid(12);
        let tex_id = test_uuid(13);
        let env_id = test_uuid(14);
        let missing_id = test_uuid(99);

        let mat_domain = DomainId::new("material");
        let light_domain = DomainId::new("light");

        let sub_dto = DtoAsset {
            format_version: DTO_FORMAT_VERSION,
            id: test_uuid(2),
            doc: String::new(),
            fields: vec![Field {
                id: test_uuid(201),
                name: "nested_curve".into(),
                ty: FieldType::Curve,
                category: String::new(),
                is_name: false,
                default: DataValue::Curve(curve_id),
                settings: FieldSettings::default(),
            }],
        };

        let mut def_nested = BTreeMap::new();
        def_nested.insert(test_uuid(201), DataValue::Curve(curve_id));

        let vfx_dto = DtoAsset {
            format_version: DTO_FORMAT_VERSION,
            id: test_uuid(1),
            doc: String::new(),
            fields: vec![
                Field {
                    id: test_uuid(101),
                    name: "falloff".into(),
                    ty: FieldType::Curve,
                    category: String::new(),
                    is_name: false,
                    default: DataValue::Curve(curve_id),
                    settings: FieldSettings { required: true, ..FieldSettings::default() },
                },
                Field {
                    id: test_uuid(102),
                    name: "ramp".into(),
                    ty: FieldType::Gradient,
                    category: String::new(),
                    is_name: false,
                    default: DataValue::Gradient(grad_id, 1),
                    settings: FieldSettings::default(),
                },
                Field {
                    id: test_uuid(103),
                    name: "surface".into(),
                    ty: FieldType::Material,
                    category: String::new(),
                    is_name: false,
                    default: DataValue::Material(mat_id),
                    settings: FieldSettings::default(),
                },
                Field {
                    id: test_uuid(104),
                    name: "icon".into(),
                    ty: FieldType::Texture,
                    category: String::new(),
                    is_name: false,
                    default: DataValue::Texture(tex_id),
                    settings: FieldSettings::default(),
                },
                Field {
                    id: test_uuid(105),
                    name: "lighting".into(),
                    ty: FieldType::LightEnvironment,
                    category: String::new(),
                    is_name: false,
                    default: DataValue::LightEnvironment(env_id),
                    settings: FieldSettings::default(),
                },
                Field {
                    id: test_uuid(106),
                    name: "sprite".into(),
                    ty: FieldType::GameAsset(GameAssetKind::Image),
                    category: String::new(),
                    is_name: false,
                    default: DataValue::GameAsset("textures/hero.png".into()),
                    settings: FieldSettings::default(),
                },
                Field {
                    id: test_uuid(107),
                    name: "nested".into(),
                    ty: FieldType::Dto(test_uuid(2)),
                    category: String::new(),
                    is_name: false,
                    default: DataValue::Dto(def_nested.clone()),
                    settings: FieldSettings::default(),
                },
            ],
        };

        let mut assets = DataAssets::default();
        assets.dtos.insert("sub".into(), sub_dto);
        assets.dtos.insert("vfx".into(), vfx_dto);

        // Row 0: valid references.
        let mut r0 = BTreeMap::new();
        r0.insert(test_uuid(101), DataValue::Curve(curve_id));
        r0.insert(test_uuid(102), DataValue::Gradient(grad_id, 1));
        r0.insert(test_uuid(103), DataValue::Material(mat_id));
        r0.insert(test_uuid(104), DataValue::Texture(tex_id));
        r0.insert(test_uuid(105), DataValue::LightEnvironment(env_id));
        r0.insert(test_uuid(106), DataValue::GameAsset("textures/hero.png".into()));
        r0.insert(test_uuid(107), DataValue::Dto(def_nested));

        // Row 1: dangling UUID, wrong asset kind (curve_id in Material), missing gradient slot 99, unsafe GameAsset path.
        let mut r1 = BTreeMap::new();
        r1.insert(test_uuid(101), DataValue::Curve(missing_id)); // DE008 dangling
        r1.insert(test_uuid(102), DataValue::Gradient(grad_id, 99)); // DE008 missing slot
        r1.insert(test_uuid(103), DataValue::Material(curve_id)); // DE008 wrong asset kind
        r1.insert(test_uuid(104), DataValue::Texture(AssetUuid::NIL));
        r1.insert(test_uuid(105), DataValue::LightEnvironment(AssetUuid::NIL));
        r1.insert(test_uuid(106), DataValue::GameAsset("../outside.png".into())); // DE008 unsafe path

        // Row 2: required Curve is NIL (DE005), Gradient has nil UUID with slot 1 (DE008), GameAsset wrong extension (DE008).
        let mut r2 = BTreeMap::new();
        r2.insert(test_uuid(101), DataValue::Curve(AssetUuid::NIL)); // DE005 required
        r2.insert(test_uuid(102), DataValue::Gradient(AssetUuid::NIL, 1)); // DE008 nil + slot != 0
        r2.insert(test_uuid(106), DataValue::GameAsset("audio/sfx.ogg".into())); // DE008 extension mismatch

        assets.collections.insert(
            "effects".into(),
            CollectionAsset {
                format_version: COLLECTION_FORMAT_VERSION,
                id: test_uuid(50),
                dto: test_uuid(1),
                rows: vec![
                    Row { id: test_uuid(500), values: r0 },
                    Row { id: test_uuid(501), values: r1 },
                    Row { id: test_uuid(502), values: r2 },
                ],
            },
        );

        let data_domain_index = assets.index_part();
        // Verify index_part emitted cross-domain refs (including Gradient sub-item and nested DTO curve ref).
        let col_indexed = data_domain_index.assets.iter().find(|a| a.asset.path == "effects").unwrap();
        assert!(col_indexed.refs.iter().any(|r| r.to == curve_id && r.sub.is_none()));
        assert!(col_indexed.refs.iter().any(|r| r.to == grad_id && r.sub == Some(1)));
        assert!(col_indexed.refs.iter().any(|r| r.to == mat_id));
        assert!(col_indexed.refs.iter().any(|r| r.to == tex_id));
        assert!(col_indexed.refs.iter().any(|r| r.to == env_id));

        let index = AssetIndex::build(vec![
            data_domain_index,
            DomainIndex {
                domain: mat_domain,
                assets: vec![
                    IndexedAsset {
                        asset: AssetRef { domain: mat_domain, path: "curves/ease".into(), suffix: "curve" },
                        id: Ok(curve_id),
                        sub_items: Vec::new(),
                        refs: Vec::new(),
                    },
                    IndexedAsset {
                        asset: AssetRef { domain: mat_domain, path: "palettes/fire".into(), suffix: "gradients" },
                        id: Ok(grad_id),
                        sub_items: vec![SubItem::new(1, "ember")],
                        refs: Vec::new(),
                    },
                    IndexedAsset {
                        asset: AssetRef { domain: mat_domain, path: "env/rock".into(), suffix: "material" },
                        id: Ok(mat_id),
                        sub_items: Vec::new(),
                        refs: Vec::new(),
                    },
                    IndexedAsset {
                        asset: AssetRef { domain: mat_domain, path: "albedo/rock".into(), suffix: "texture" },
                        id: Ok(tex_id),
                        sub_items: Vec::new(),
                        refs: Vec::new(),
                    },
                ],
            },
            DomainIndex {
                domain: light_domain,
                assets: vec![IndexedAsset {
                    asset: AssetRef { domain: light_domain, path: "caves/crystal".into(), suffix: "environment" },
                    id: Ok(env_id),
                    sub_items: Vec::new(),
                    refs: Vec::new(),
                }],
            },
        ]);

        // Create temporary project with `assets/textures/hero.png` on disk.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[workspace]\nmembers = [\"crates/*\"]\n").unwrap();
        std::fs::create_dir_all(dir.path().join("assets/textures")).unwrap();
        std::fs::write(dir.path().join("assets/textures/hero.png"), b"png").unwrap();
        let project = Project::create(dir.path()).unwrap();
        let mut settings = ProjectSettings::default();
        settings.domains.insert(mat_domain.to_string(), project_core::layout::DomainEntry { enabled: true });
        settings.domains.insert(light_domain.to_string(), project_core::layout::DomainEntry { enabled: true });
        settings.domains.insert(DATA.to_string(), project_core::layout::DomainEntry { enabled: true });
        save_settings(&project, &settings).unwrap();
        let project = Project::open(dir.path()).unwrap();

        let report = assets.validate_with_project(Some(&project), &index);
        // Row 0 is valid; rows 1 and 2 have DE008 / DE005 and are skipped.
        assert!(!report.skipped_rows.contains(&("effects".to_string(), 0)), "{:?}", report.diagnostics);
        assert!(report.skipped_rows.contains(&("effects".to_string(), 1)));
        assert!(report.skipped_rows.contains(&("effects".to_string(), 2)));
        let de008_count = report.diagnostics.iter().filter(|d| d.code == "DE008").count();
        assert_eq!(de008_count, 6, "expected 6 DE008 errors on rows 1 & 2, got: {:?}", report.diagnostics);

        // Disabling `light` domain triggers DE008 on the `LightEnvironment` field and reference.
        let mut settings_no_light = settings;
        settings_no_light
            .domains
            .insert(light_domain.to_string(), project_core::layout::DomainEntry { enabled: false });
        save_settings(&project, &settings_no_light).unwrap();
        let project_no_light = Project::open(dir.path()).unwrap();
        let report_no_light = assets.validate_with_project(Some(&project_no_light), &index);
        assert!(
            report_no_light.diagnostics.iter().any(|d| {
                d.code == "DE008"
                    && d.asset.as_deref() == Some("quillshade/data/vfx.dto.ron")
                    && d.message.contains("LightEnvironment")
            }),
            "{:?}",
            report_no_light.diagnostics
        );
    }

    #[test]
    fn validate_soft_ref_target_dto_and_row_values() {
        let book_dto_id = test_uuid(60);
        let author_dto_id = test_uuid(61);
        let unknown_dto_id = test_uuid(69);

        let f_book_title = test_uuid(62);
        let f_book_sequel = test_uuid(63);
        let f_book_author = test_uuid(64);
        let f_author_name = test_uuid(65);
        let f_author_fav_book = test_uuid(66);
        let f_bad_soft_ref = test_uuid(67);

        let book_row_0 = test_uuid(70);
        let book_row_1 = test_uuid(71);
        let author_row_0 = test_uuid(72);
        let unknown_row = test_uuid(79);

        let book_dto = DtoAsset {
            format_version: DTO_FORMAT_VERSION,
            id: book_dto_id,
            doc: String::new(),
            fields: vec![
                Field {
                    id: f_book_title,
                    name: "title".into(),
                    ty: FieldType::String,
                    category: String::new(),
                    is_name: true,
                    default: DataValue::String("Untitled".into()),
                    settings: FieldSettings::default(),
                },
                // Self-referential SoftRef<Book>: must NOT trigger DE003 cycle error!
                Field {
                    id: f_book_sequel,
                    name: "sequel".into(),
                    ty: FieldType::SoftRef(book_dto_id),
                    category: String::new(),
                    is_name: false,
                    default: DataValue::SoftRef(AssetUuid::NIL),
                    settings: FieldSettings::default(),
                },
                // Mutual SoftRef<Author> (Author also has SoftRef<Book>): must NOT trigger DE003!
                Field {
                    id: f_book_author,
                    name: "author".into(),
                    ty: FieldType::SoftRef(author_dto_id),
                    category: String::new(),
                    is_name: false,
                    default: DataValue::SoftRef(AssetUuid::NIL),
                    settings: FieldSettings { required: true, ..FieldSettings::default() },
                },
            ],
        };

        let author_dto = DtoAsset {
            format_version: DTO_FORMAT_VERSION,
            id: author_dto_id,
            doc: String::new(),
            fields: vec![
                Field {
                    id: f_author_name,
                    name: "name".into(),
                    ty: FieldType::String,
                    category: String::new(),
                    is_name: true,
                    default: DataValue::String("Anon".into()),
                    settings: FieldSettings::default(),
                },
                Field {
                    id: f_author_fav_book,
                    name: "favorite_book".into(),
                    ty: FieldType::SoftRef(book_dto_id),
                    category: String::new(),
                    is_name: false,
                    default: DataValue::SoftRef(AssetUuid::NIL),
                    settings: FieldSettings::default(),
                },
            ],
        };

        let broken_dto = DtoAsset {
            format_version: DTO_FORMAT_VERSION,
            id: test_uuid(68),
            doc: String::new(),
            fields: vec![Field {
                id: f_bad_soft_ref,
                name: "broken_ref".into(),
                ty: FieldType::SoftRef(unknown_dto_id),
                category: String::new(),
                is_name: false,
                default: DataValue::SoftRef(AssetUuid::NIL),
                settings: FieldSettings::default(),
            }],
        };

        let books_col = CollectionAsset {
            format_version: COLLECTION_FORMAT_VERSION,
            id: test_uuid(80),
            dto: book_dto_id,
            rows: vec![
                // Row 0: valid (sequel -> book_row_1, author -> author_row_0)
                Row {
                    id: book_row_0,
                    values: BTreeMap::from([
                        (f_book_title, DataValue::String("Volume I".into())),
                        (f_book_sequel, DataValue::SoftRef(book_row_1)),
                        (f_book_author, DataValue::SoftRef(author_row_0)),
                    ]),
                },
                // Row 1: valid (sequel -> None, author -> author_row_0)
                Row {
                    id: book_row_1,
                    values: BTreeMap::from([
                        (f_book_title, DataValue::String("Volume II".into())),
                        (f_book_sequel, DataValue::SoftRef(AssetUuid::NIL)),
                        (f_book_author, DataValue::SoftRef(author_row_0)),
                    ]),
                },
                // Row 2: required `author` is nil -> DE006, skipped
                Row {
                    id: test_uuid(73),
                    values: BTreeMap::from([
                        (f_book_title, DataValue::String("Orphan Book".into())),
                        (f_book_author, DataValue::SoftRef(AssetUuid::NIL)),
                    ]),
                },
                // Row 3: `sequel` references unknown row UUID -> DE008, skipped
                Row {
                    id: test_uuid(74),
                    values: BTreeMap::from([
                        (f_book_title, DataValue::String("Dangling Sequel".into())),
                        (f_book_sequel, DataValue::SoftRef(unknown_row)),
                        (f_book_author, DataValue::SoftRef(author_row_0)),
                    ]),
                },
                // Row 4: `sequel` (SoftRef<book>) references `author_row_0` (in collection of type `author`) -> DE008, skipped
                Row {
                    id: test_uuid(75),
                    values: BTreeMap::from([
                        (f_book_title, DataValue::String("Wrong Target Type".into())),
                        (f_book_sequel, DataValue::SoftRef(author_row_0)),
                        (f_book_author, DataValue::SoftRef(author_row_0)),
                    ]),
                },
            ],
        };

        let authors_col = CollectionAsset {
            format_version: COLLECTION_FORMAT_VERSION,
            id: test_uuid(81),
            dto: author_dto_id,
            rows: vec![Row {
                id: author_row_0,
                values: BTreeMap::from([
                    (f_author_name, DataValue::String("Ada".into())),
                    (f_author_fav_book, DataValue::SoftRef(book_row_0)),
                ]),
            }],
        };

        let mut assets = DataAssets::default();
        assets.dtos.insert("book".into(), book_dto);
        assets.dtos.insert("author".into(), author_dto);
        assets.dtos.insert("broken".into(), broken_dto);
        assets.collections.insert("books".into(), books_col);
        assets.collections.insert("authors".into(), authors_col);

        let index = AssetIndex::build(vec![assets.index_part()]);
        let report = assets.validate(&index);

        // No DE003 cycle errors on self-referential or mutually-referential SoftRef.
        assert!(!report.diagnostics.iter().any(|d| d.code == "DE003"), "{:?}", report.diagnostics);

        // DE004 on `broken.dto.ron` for unknown target DTO UUID.
        assert!(
            report.diagnostics.iter().any(|d| {
                d.code == "DE004"
                    && d.asset.as_deref() == Some("quillshade/data/broken.dto.ron")
                    && d.location.as_deref() == Some("broken_ref.ty")
            }),
            "{:?}",
            report.diagnostics
        );

        // Rows 0 and 1 of `books` and row 0 of `authors` are valid.
        assert!(!report.skipped_rows.contains(&("books".to_string(), 0)), "{:?}", report.diagnostics);
        assert!(!report.skipped_rows.contains(&("books".to_string(), 1)), "{:?}", report.diagnostics);
        assert!(!report.skipped_rows.contains(&("authors".to_string(), 0)), "{:?}", report.diagnostics);

        // Row 2 has DE006 for required unset SoftRef.
        assert!(report.skipped_rows.contains(&("books".to_string(), 2)));
        assert!(
            report.diagnostics.iter().any(|d| {
                d.code == "DE006" && d.message.contains("row #2 required SoftRef field `author` is unset (None)")
            }),
            "{:?}",
            report.diagnostics
        );

        // Row 3 has DE008 for unknown row UUID.
        assert!(report.skipped_rows.contains(&("books".to_string(), 3)));
        assert!(
            report.diagnostics.iter().any(|d| {
                d.code == "DE008"
                    && d.message
                        .contains(&format!("row #3 field `sequel` SoftRef references unknown row UUID `{unknown_row}`"))
            }),
            "{:?}",
            report.diagnostics
        );

        // Row 4 has DE008 for wrong collection DTO type.
        assert!(report.skipped_rows.contains(&("books".to_string(), 4)));
        assert!(
            report.diagnostics.iter().any(|d| {
                d.code == "DE008"
                    && d.message.contains(&format!(
                        "row #4 field `sequel` SoftRef<book> references row {author_row_0} in collection 'authors' of type 'author', expected 'book'"
                    ))
            }),
            "{:?}",
            report.diagnostics
        );
    }

    #[test]
    fn validates_enum_schemas_cycles_collisions_and_row_values() {
        use crate::backend::model::{ENUM_FORMAT_VERSION, EnumAsset, EnumVariant};

        let rarity_enum_id = test_uuid(300);
        let var_common = test_uuid(301);
        let var_rare = test_uuid(302);

        let effect_enum_id = test_uuid(310);
        let var_none = test_uuid(311);
        let var_burn = test_uuid(312);
        let f_burn_dps = test_uuid(313);
        let f_burn_duration = test_uuid(314);

        let item_dto_id = test_uuid(320);
        let f_item_name = test_uuid(321);
        let f_item_rarity = test_uuid(322);
        let f_item_effect = test_uuid(323);

        let rarity_enum = EnumAsset {
            format_version: ENUM_FORMAT_VERSION,
            id: rarity_enum_id,
            doc: "Rarity".into(),
            variants: vec![
                EnumVariant { id: var_common, name: "common".into(), doc: String::new(), fields: Vec::new() },
                EnumVariant { id: var_rare, name: "rare".into(), doc: String::new(), fields: Vec::new() },
            ],
        };

        let effect_enum = EnumAsset {
            format_version: ENUM_FORMAT_VERSION,
            id: effect_enum_id,
            doc: "Effect".into(),
            variants: vec![
                EnumVariant { id: var_none, name: "none".into(), doc: String::new(), fields: Vec::new() },
                EnumVariant {
                    id: var_burn,
                    name: "burn".into(),
                    doc: String::new(),
                    fields: vec![
                        Field {
                            id: f_burn_dps,
                            name: "dps".into(),
                            ty: FieldType::F32,
                            category: String::new(),
                            is_name: true,
                            default: DataValue::F32(10.0),
                            settings: FieldSettings { min: Some(0.0), max: Some(100.0), ..FieldSettings::default() },
                        },
                        Field {
                            id: f_burn_duration,
                            name: "duration".into(),
                            ty: FieldType::Duration,
                            category: String::new(),
                            is_name: false,
                            default: DataValue::Duration(3.0),
                            settings: FieldSettings::default(),
                        },
                    ],
                },
            ],
        };

        let item_dto = DtoAsset {
            format_version: DTO_FORMAT_VERSION,
            id: item_dto_id,
            doc: String::new(),
            fields: vec![
                Field {
                    id: f_item_name,
                    name: "name".into(),
                    ty: FieldType::String,
                    category: String::new(),
                    is_name: true,
                    default: DataValue::String("Sword".into()),
                    settings: FieldSettings::default(),
                },
                Field {
                    id: f_item_rarity,
                    name: "rarity".into(),
                    ty: FieldType::Enum(rarity_enum_id),
                    category: String::new(),
                    is_name: false,
                    default: DataValue::Enum { variant: var_common, fields: BTreeMap::new() },
                    settings: FieldSettings::default(),
                },
                Field {
                    id: f_item_effect,
                    name: "effect".into(),
                    ty: FieldType::Enum(effect_enum_id),
                    category: String::new(),
                    is_name: false,
                    default: DataValue::Enum {
                        variant: var_burn,
                        fields: BTreeMap::from([
                            (f_burn_dps, DataValue::F32(10.0)),
                            (f_burn_duration, DataValue::Duration(3.0)),
                        ]),
                    },
                    settings: FieldSettings::default(),
                },
            ],
        };

        // Row 0: valid unit + payload enum values
        let row0 = Row {
            id: test_uuid(400),
            values: BTreeMap::from([
                (f_item_name, DataValue::String("Firebrand".into())),
                (f_item_rarity, DataValue::Enum { variant: var_rare, fields: BTreeMap::new() }),
                (
                    f_item_effect,
                    DataValue::Enum {
                        variant: var_burn,
                        fields: BTreeMap::from([
                            (f_burn_dps, DataValue::F32(25.0)),
                            (f_burn_duration, DataValue::Duration(5.0)),
                        ]),
                    },
                ),
            ]),
        };

        // Row 1: unknown variant UUID -> DE005, skipped
        let row1 = Row {
            id: test_uuid(401),
            values: BTreeMap::from([
                (f_item_name, DataValue::String("Broken Variant".into())),
                (f_item_rarity, DataValue::Enum { variant: test_uuid(999), fields: BTreeMap::new() }),
                (f_item_effect, DataValue::Enum { variant: var_none, fields: BTreeMap::new() }),
            ]),
        };

        // Row 2: unknown payload field key -> DW101, soft max exceeded -> DW102 (warnings only, not skipped)
        let row2 = Row {
            id: test_uuid(402),
            values: BTreeMap::from([
                (f_item_name, DataValue::String("Overpowered".into())),
                (f_item_rarity, DataValue::Enum { variant: var_common, fields: BTreeMap::new() }),
                (
                    f_item_effect,
                    DataValue::Enum {
                        variant: var_burn,
                        fields: BTreeMap::from([
                            (f_burn_dps, DataValue::F32(250.0)), // > 100.0 -> DW102
                            (f_burn_duration, DataValue::Duration(5.0)),
                            (test_uuid(888), DataValue::Bool(true)), // unknown payload field -> DW101
                        ]),
                    },
                ),
            ]),
        };

        let mut assets = DataAssets::default();
        assets.enums.insert("rarity".into(), rarity_enum);
        assets.enums.insert("effect".into(), effect_enum);
        // Empty unused enum -> DW103 + DW104
        assets.enums.insert(
            "empty_unused".into(),
            EnumAsset {
                format_version: ENUM_FORMAT_VERSION,
                id: test_uuid(350),
                doc: String::new(),
                variants: Vec::new(),
            },
        );
        assets.dtos.insert("item".into(), item_dto);
        assets.collections.insert(
            "items".into(),
            CollectionAsset {
                format_version: COLLECTION_FORMAT_VERSION,
                id: test_uuid(390),
                dto: item_dto_id,
                rows: vec![row0, row1, row2],
            },
        );

        let index = AssetIndex::build(vec![assets.index_part()]);
        let report = assets.validate(&index);
        assert!(!report.skipped_rows.contains(&("items".to_string(), 0)), "{:?}", report.diagnostics);
        assert!(report.skipped_rows.contains(&("items".to_string(), 1)), "{:?}", report.diagnostics);
        assert!(!report.skipped_rows.contains(&("items".to_string(), 2)), "{:?}", report.diagnostics);

        let codes: BTreeSet<&str> = report.diagnostics.iter().map(|d| d.code.as_str()).collect();
        for expected in ["DE005", "DW101", "DW102", "DW103", "DW104"] {
            assert!(codes.contains(expected), "missing {expected} in {:?}", report.diagnostics);
        }

        // Cross DTO/Enum cycle detection: DTO `node_a` -> Enum `node_b` -> DTO `node_a` -> DE003
        let mut cycle_assets = DataAssets::default();
        let dto_a_id = test_uuid(700);
        let enum_b_id = test_uuid(701);
        let var_b0 = test_uuid(702);
        cycle_assets.dtos.insert("node_a".into(), simple_dto(700, 710, "b", FieldType::Enum(enum_b_id)));
        cycle_assets.enums.insert(
            "node_b".into(),
            EnumAsset {
                format_version: ENUM_FORMAT_VERSION,
                id: enum_b_id,
                doc: String::new(),
                variants: vec![EnumVariant {
                    id: var_b0,
                    name: "wrap".into(),
                    doc: String::new(),
                    fields: vec![Field {
                        id: test_uuid(711),
                        name: "a".into(),
                        ty: FieldType::Dto(dto_a_id),
                        category: String::new(),
                        is_name: false,
                        default: DataValue::Dto(BTreeMap::new()),
                        settings: FieldSettings::default(),
                    }],
                }],
            },
        );
        // Also test PascalCase collision between DTO `foo_item` and Enum `foo/item` -> DE003
        cycle_assets.dtos.insert("foo_item".into(), simple_dto(720, 721, "x", FieldType::I32));
        cycle_assets.enums.insert(
            "foo/item".into(),
            EnumAsset {
                format_version: ENUM_FORMAT_VERSION,
                id: test_uuid(722),
                doc: String::new(),
                variants: vec![EnumVariant {
                    id: test_uuid(723),
                    name: "v".into(),
                    doc: String::new(),
                    fields: Vec::new(),
                }],
            },
        );
        let cycle_report = cycle_assets.validate(&AssetIndex::default());
        assert!(
            cycle_report
                .diagnostics
                .iter()
                .any(|d| d.code == "DE003" && d.message.contains("node_a -> node_b -> node_a")),
            "{:?}",
            cycle_report.diagnostics
        );
        assert!(
            cycle_report
                .diagnostics
                .iter()
                .any(|d| d.code == "DE003" && d.message.contains("collides in PascalCase (`FooItem`)")),
            "{:?}",
            cycle_report.diagnostics
        );
    }

    #[test]
    fn validates_option_and_containers_de010_de008_de006_de009_and_recursive_option() {
        use crate::backend::model::{ENUM_FORMAT_VERSION, EnumVariant};

        let unit_enum_id = test_uuid(800);
        let var_a = test_uuid(801);
        let var_b = test_uuid(802);
        let tagged_enum_id = test_uuid(810);
        let var_payload = test_uuid(811);
        let f_payload_val = test_uuid(812);

        let unit_enum = EnumAsset {
            format_version: ENUM_FORMAT_VERSION,
            id: unit_enum_id,
            doc: String::new(),
            variants: vec![
                EnumVariant { id: var_a, name: "alpha".into(), doc: String::new(), fields: Vec::new() },
                EnumVariant { id: var_b, name: "beta".into(), doc: String::new(), fields: Vec::new() },
            ],
        };
        let tagged_enum = EnumAsset {
            format_version: ENUM_FORMAT_VERSION,
            id: tagged_enum_id,
            doc: String::new(),
            variants: vec![EnumVariant {
                id: var_payload,
                name: "payload".into(),
                doc: String::new(),
                fields: vec![Field {
                    id: f_payload_val,
                    name: "v".into(),
                    ty: FieldType::I32,
                    category: String::new(),
                    is_name: false,
                    default: DataValue::I32(1),
                    settings: FieldSettings::default(),
                }],
            }],
        };

        // 1. Recursive `Option<Self>` is valid (broken by `Option<Box<Self>>` indirection, no DE003).
        let tree_id = test_uuid(820);
        let f_tree_label = test_uuid(821);
        let f_tree_child = test_uuid(822);
        let tree_dto = DtoAsset {
            format_version: DTO_FORMAT_VERSION,
            id: tree_id,
            doc: String::new(),
            fields: vec![
                Field {
                    id: f_tree_label,
                    name: "label".into(),
                    ty: FieldType::String,
                    category: String::new(),
                    is_name: true,
                    default: DataValue::String("root".into()),
                    settings: FieldSettings::default(),
                },
                Field {
                    id: f_tree_child,
                    name: "next".into(),
                    ty: FieldType::Option(Box::new(FieldType::Dto(tree_id))),
                    category: String::new(),
                    is_name: false,
                    default: DataValue::Option(None),
                    settings: FieldSettings::default(),
                },
            ],
        };

        // 2. DTO with invalid Set/Map keys (`DE010`), duplicate Set default key (`DE008`), and valid container fields.
        let bag_id = test_uuid(830);
        let f_bad_set_float = test_uuid(831);
        let f_bad_map_tagged = test_uuid(832);
        let f_dup_default_set = test_uuid(833);
        let f_req_tags = test_uuid(834);
        let f_enum_map = test_uuid(835);
        let bag_dto = DtoAsset {
            format_version: DTO_FORMAT_VERSION,
            id: bag_id,
            doc: String::new(),
            fields: vec![
                Field {
                    id: f_bad_set_float,
                    name: "bad_float_set".into(),
                    ty: FieldType::Set(Box::new(FieldType::F32)),
                    category: String::new(),
                    is_name: false,
                    default: DataValue::Set(Vec::new()),
                    settings: FieldSettings::default(),
                },
                Field {
                    id: f_bad_map_tagged,
                    name: "bad_tagged_map".into(),
                    ty: FieldType::Map(Box::new(FieldType::Enum(tagged_enum_id)), Box::new(FieldType::I32)),
                    category: String::new(),
                    is_name: false,
                    default: DataValue::Map(Vec::new()),
                    settings: FieldSettings::default(),
                },
                Field {
                    id: f_dup_default_set,
                    name: "dup_default_set".into(),
                    ty: FieldType::Set(Box::new(FieldType::String)),
                    category: String::new(),
                    is_name: false,
                    default: DataValue::Set(vec![DataValue::String("fire".into()), DataValue::String("fire".into())]),
                    settings: FieldSettings::default(),
                },
                Field {
                    id: f_req_tags,
                    name: "req_tags".into(),
                    ty: FieldType::Array(Box::new(FieldType::String)),
                    category: String::new(),
                    is_name: false,
                    default: DataValue::Array(vec![DataValue::String("default_tag".into())]),
                    settings: FieldSettings { required: true, ..FieldSettings::default() },
                },
                Field {
                    id: f_enum_map,
                    name: "weights".into(),
                    ty: FieldType::Map(Box::new(FieldType::Enum(unit_enum_id)), Box::new(FieldType::I32)),
                    category: String::new(),
                    is_name: false,
                    default: DataValue::Map(Vec::new()),
                    settings: FieldSettings::default(),
                },
            ],
        };

        // Row 0: valid non-empty required array and unique map keys.
        let row0 = Row {
            id: test_uuid(840),
            values: BTreeMap::from([
                (f_bad_set_float, DataValue::Set(Vec::new())),
                (f_bad_map_tagged, DataValue::Map(Vec::new())),
                (f_dup_default_set, DataValue::Set(vec![DataValue::String("ok".into())])),
                (f_req_tags, DataValue::Array(vec![DataValue::String("quest".into())])),
                (
                    f_enum_map,
                    DataValue::Map(vec![
                        (DataValue::Enum { variant: var_a, fields: BTreeMap::new() }, DataValue::I32(10)),
                        (DataValue::Enum { variant: var_b, fields: BTreeMap::new() }, DataValue::I32(20)),
                    ]),
                ),
            ]),
        };

        // Row 1: empty required array (`DE006`) + duplicate map key (`DE008`) -> skipped.
        let row1 = Row {
            id: test_uuid(841),
            values: BTreeMap::from([
                (f_bad_set_float, DataValue::Set(Vec::new())),
                (f_bad_map_tagged, DataValue::Map(Vec::new())),
                (f_dup_default_set, DataValue::Set(Vec::new())),
                (f_req_tags, DataValue::Array(Vec::new())),
                (
                    f_enum_map,
                    DataValue::Map(vec![
                        (DataValue::Enum { variant: var_a, fields: BTreeMap::new() }, DataValue::I32(10)),
                        (DataValue::Enum { variant: var_a, fields: BTreeMap::new() }, DataValue::I32(99)),
                    ]),
                ),
            ]),
        };

        let mut assets = DataAssets::default();
        assets.enums.insert("unit_enum".into(), unit_enum);
        assets.enums.insert("tagged_enum".into(), tagged_enum);
        assets.dtos.insert("tree".into(), tree_dto);
        assets.dtos.insert("bag".into(), bag_dto);
        assets.collections.insert(
            "bags".into(),
            CollectionAsset {
                format_version: COLLECTION_FORMAT_VERSION,
                id: test_uuid(850),
                dto: bag_id,
                rows: vec![row0, row1],
            },
        );
        assets.collections.insert(
            "trees".into(),
            CollectionAsset {
                format_version: COLLECTION_FORMAT_VERSION,
                id: test_uuid(851),
                dto: tree_id,
                rows: Vec::new(),
            },
        );

        let index = AssetIndex::build(vec![assets.index_part()]);
        let report = assets.validate(&index);

        // `tree` with `Option<Dto(tree)>` must NOT emit DE003.
        assert!(
            !report.diagnostics.iter().any(|d| d.code == "DE003"),
            "Option<Self> should not report DE003 cycle: {:?}",
            report.diagnostics
        );
        assert!(
            report.diagnostics.iter().any(|d| d.code == "DE010" && d.message.contains("bad_float_set")),
            "{:?}",
            report.diagnostics
        );
        assert!(
            report.diagnostics.iter().any(|d| d.code == "DE010" && d.message.contains("bad_tagged_map")),
            "{:?}",
            report.diagnostics
        );
        assert!(
            report
                .diagnostics
                .iter()
                .any(|d| d.code == "DE010" && d.message.contains("dup_default_set") && d.message.contains("duplicate")),
            "{:?}",
            report.diagnostics
        );
        assert!(
            report.diagnostics.iter().any(|d| d.code == "DE006" && d.message.contains("req_tags")),
            "{:?}",
            report.diagnostics
        );
        assert!(
            report
                .diagnostics
                .iter()
                .any(|d| d.code == "DE010" && d.message.contains("weights") && d.message.contains("duplicate")),
            "{:?}",
            report.diagnostics
        );
        assert!(!report.skipped_rows.contains(&("bags".to_string(), 0)));
        assert!(report.skipped_rows.contains(&("bags".to_string(), 1)));
    }
}
