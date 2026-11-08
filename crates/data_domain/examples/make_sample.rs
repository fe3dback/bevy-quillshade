//! Writes the sample game's Data assets (`examples/sample_game/quillshade/data/`):
//! - `lore/sector_coords.dto.ron` (`SectorCoords` schema — level 3 nested DTO)
//! - `lore/planet.dto.ron` (`Planet` schema — level 2 nested DTO embedding `SectorCoords`)
//! - `library/author.dto.ron` (`Author` schema — level 1 nested DTO embedding `Planet`)
//! - `library/book_tag.enum.ron` (`BookTag` plain unit enum)
//! - `library/enchantment.enum.ron` (`Enchantment` tagged union with unit + struct variants)
//! - `library/book.dto.ron` (`Book` schema — root DTO embedding `Author`, `BookTag`, `Enchantment`, `Option`, `Array`, `Set`)
//! - `library/books_core.collection.ron` (5 sample books with nested DTOs, enums, optionals, and containers)
//! - `library/library_rules.dto.ron` (`LibraryRules` schema with `Map<BookTag, u32>`)
//! - `library/library_rules.collection.ron` (1 singleton rules row)
//!
//! Then regenerate with the CLI, which knows every domain:
//!
//! ```sh
//! cargo run -p data_domain --example make_sample -- examples/sample_game
//! cargo run -- regen examples/sample_game
//! ```

use std::collections::BTreeMap;
use std::path::Path;

use data_domain::backend::{
    CollectionAsset, DTO_FORMAT_VERSION, DataBackend, DataValue, DtoAsset, ENUM_FORMAT_VERSION, EnumAsset, EnumVariant,
    Field, FieldSettings, FieldType, GameAssetKind, Row, collection_ref, dto_ref, enum_ref,
    save_collection_with_schemas_and_index, save_dto_with_schemas_and_index, save_enum_with_index,
};
use project_core::asset_index::{DomainIndex, IndexedAsset, SubItem};
use project_core::regen::DomainBackend;
use project_core::{AssetIndex, AssetRef, AssetUuid, DomainId, Project};

const SECTOR_COORDS_DTO: &str = "lore/sector_coords";
const PLANET_DTO: &str = "lore/planet";
const AUTHOR_DTO: &str = "library/author";
const BOOK_TAG_ENUM: &str = "library/book_tag";
const ENCHANTMENT_ENUM: &str = "library/enchantment";
const BOOK_DTO: &str = "library/book";
const BOOKS_CORE_COLLECTION: &str = "library/books_core";
const LIBRARY_RULES_DTO: &str = "library/library_rules";
const LIBRARY_RULES_COLLECTION: &str = "library/library_rules";

const MAT_CURVE_PULSE: &str = "quillshade/material/common/pulse.curve.ron";
const MAT_CURVE_STEPS: &str = "quillshade/material/common/steps.curve.ron";
const LIGHT_CURVE_SMOOTH: &str = "quillshade/light/smooth.curve.ron";
const MAT_GRAD_TERRAIN: &str = "quillshade/material/common/terrain.gradients.ron";
const MAT_ROCK_WET: &str = "quillshade/material/env/rock_wet.material.ron";
const MAT_EXP42: &str = "quillshade/material/env/exp42.material.ron";
const MAT_HOLOGRAM: &str = "quillshade/material/fx/hologram.material.ron";
const MAT_CURVE_BANDS: &str = "quillshade/material/fx/curve_bands.material.ron";
const MAT_GLOBAL_TINT: &str = "quillshade/material/fx/global_tint.material.ron";
const MAT_TEX_ATLAS: &str = "quillshade/material/textures/atlas.texture.ron";
const MAT_TEX_ROCK_ALBEDO: &str = "quillshade/material/textures/rock_albedo.texture.ron";
const MAT_TEX_SPRITE: &str = "quillshade/material/textures/sprite.texture.ron";
const LIGHT_ENV_DAY: &str = "quillshade/light/day.environment.ron";
const LIGHT_ENV_NIGHT: &str = "quillshade/light/night.environment.ron";

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| "examples/sample_game".into());
    let project = Project::open(&dir).unwrap_or_else(|e| panic!("{e}"));
    write_sources(&project.root).expect("write the Data sources");
    let dtos = BTreeMap::new();
    let enums = BTreeMap::new();
    let mut index = sample_asset_index(&dtos, &enums, None, None);
    index.replace_domain(DataBackend.index(&project));
    let plan = DataBackend.plan(&project, &index, false);
    for d in &plan.diagnostics {
        eprintln!("{d}");
    }
    println!("Data assets written ({} diagnostic(s)); now run `cargo run -- regen {dir}`", plan.diagnostics.len());
}

fn sample_asset_index(
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
    book_dto: Option<&DtoAsset>,
    books_col: Option<&CollectionAsset>,
) -> AssetIndex {
    let mat = DomainId::new("material");
    let light = DomainId::new("light");
    let data = DomainId::new("data");

    let mut mat_idx = DomainIndex::empty(mat);
    for (path, suffix) in [
        ("common/pulse", "curve"),
        ("common/steps", "curve"),
        ("env/rock_wet", "material"),
        ("env/exp42", "material"),
        ("fx/hologram", "material"),
        ("fx/curve_bands", "material"),
        ("fx/global_tint", "material"),
        ("textures/atlas", "texture"),
        ("textures/rock_albedo", "texture"),
        ("textures/sprite", "texture"),
    ] {
        let aref = AssetRef { domain: mat, path: path.into(), suffix };
        let id = sample_id(&aref.source_rel());
        mat_idx.assets.push(IndexedAsset { asset: aref, id: Ok(id), sub_items: Vec::new(), refs: Vec::new() });
    }
    let grad_ref = AssetRef { domain: mat, path: "common/terrain".into(), suffix: "gradients" };
    mat_idx.assets.push(IndexedAsset {
        asset: grad_ref,
        id: Ok(sample_id(MAT_GRAD_TERRAIN)),
        sub_items: vec![SubItem::new(1007, "Terrain height"), SubItem::new(1014, "Water depth")],
        refs: Vec::new(),
    });

    let mut light_idx = DomainIndex::empty(light);
    for (path, suffix) in [("smooth", "curve"), ("day", "environment"), ("night", "environment")] {
        let aref = AssetRef { domain: light, path: path.into(), suffix };
        let id = sample_id(&aref.source_rel());
        light_idx.assets.push(IndexedAsset { asset: aref, id: Ok(id), sub_items: Vec::new(), refs: Vec::new() });
    }

    let mut data_idx = DomainIndex::empty(data);
    for (path, dto) in dtos {
        data_idx.assets.push(IndexedAsset {
            asset: dto_ref(path),
            id: Ok(dto.id),
            sub_items: Vec::new(),
            refs: Vec::new(),
        });
    }
    for (path, en) in enums {
        let mut sub_items = Vec::new();
        for v in &en.variants {
            sub_items.push(SubItem::with_uuid(v.id, v.name.clone()));
            for f in &v.fields {
                sub_items.push(SubItem::with_uuid(f.id, format!("{}.{}", v.name, f.name)));
            }
        }
        data_idx.assets.push(IndexedAsset { asset: enum_ref(path), id: Ok(en.id), sub_items, refs: Vec::new() });
    }
    if let (Some(book), Some(col)) = (book_dto, books_col) {
        let sub_items = col
            .rows
            .iter()
            .map(|r| SubItem::with_uuid(r.id, r.short_view_string_with_schemas(book, dtos, enums)))
            .collect();
        data_idx.assets.push(IndexedAsset {
            asset: collection_ref(BOOKS_CORE_COLLECTION),
            id: Ok(col.id),
            sub_items,
            refs: Vec::new(),
        });
    }

    AssetIndex::build(vec![mat_idx, light_idx, data_idx])
}

/// Writes the sample Data assets into `root/quillshade/data/`.
fn write_sources(root: &Path) -> std::io::Result<()> {
    let file = |r: &AssetRef| root.join(r.source_rel());

    let mut sector_coords = sector_coords_dto();
    let mut planet = planet_dto(&sector_coords);
    let mut author = author_dto(&planet, &sector_coords);
    let mut book_tag = book_tag_enum();
    let mut enchantment = enchantment_enum();
    let mut book = book_dto(&author, &planet, &sector_coords, &book_tag, &enchantment);
    let mut rules_dto = library_rules_dto(&book, &book_tag);
    let mut books = books_core_collection(&book, &author, &planet, &sector_coords, &book_tag, &enchantment);

    let dtos = BTreeMap::from([
        (SECTOR_COORDS_DTO.to_string(), sector_coords.clone()),
        (PLANET_DTO.to_string(), planet.clone()),
        (AUTHOR_DTO.to_string(), author.clone()),
        (BOOK_DTO.to_string(), book.clone()),
        (LIBRARY_RULES_DTO.to_string(), rules_dto.clone()),
    ]);
    let enums = BTreeMap::from([
        (BOOK_TAG_ENUM.to_string(), book_tag.clone()),
        (ENCHANTMENT_ENUM.to_string(), enchantment.clone()),
    ]);
    let index = sample_asset_index(&dtos, &enums, Some(&book), Some(&books));

    save_enum_with_index(&file(&enum_ref(BOOK_TAG_ENUM)), &mut book_tag, &dtos, &enums, Some(&index))?;
    save_enum_with_index(&file(&enum_ref(ENCHANTMENT_ENUM)), &mut enchantment, &dtos, &enums, Some(&index))?;
    save_dto_with_schemas_and_index(
        &file(&dto_ref(SECTOR_COORDS_DTO)),
        &mut sector_coords,
        &dtos,
        &enums,
        Some(&index),
    )?;
    save_dto_with_schemas_and_index(&file(&dto_ref(PLANET_DTO)), &mut planet, &dtos, &enums, Some(&index))?;
    save_dto_with_schemas_and_index(&file(&dto_ref(AUTHOR_DTO)), &mut author, &dtos, &enums, Some(&index))?;
    save_dto_with_schemas_and_index(&file(&dto_ref(BOOK_DTO)), &mut book, &dtos, &enums, Some(&index))?;

    let books_col_r = collection_ref(BOOKS_CORE_COLLECTION);
    save_collection_with_schemas_and_index(&file(&books_col_r), &mut books, Some(&book), &dtos, &enums, Some(&index))?;

    let rules_dto_r = dto_ref(LIBRARY_RULES_DTO);
    save_dto_with_schemas_and_index(&file(&rules_dto_r), &mut rules_dto, &dtos, &enums, Some(&index))?;

    let rules_col_r = collection_ref(LIBRARY_RULES_COLLECTION);
    let mut rules_col = library_rules_collection(&rules_dto, &book_tag);
    save_collection_with_schemas_and_index(
        &file(&rules_col_r),
        &mut rules_col,
        Some(&rules_dto),
        &dtos,
        &enums,
        Some(&index),
    )?;

    Ok(())
}

fn sample_id(key: &str) -> AssetUuid {
    AssetUuid::legacy(key)
}

fn sector_coords_dto() -> DtoAsset {
    let base = "quillshade/data/lore/sector_coords.dto.ron";
    DtoAsset {
        format_version: DTO_FORMAT_VERSION,
        id: sample_id(base),
        doc: "Galactic sector and 3D parsec coordinates for a celestial world.".into(),
        fields: vec![
            Field {
                id: sample_id(&format!("{base}#sector_name")),
                name: "sector_name".into(),
                ty: FieldType::String,
                default: DataValue::String("Coreward".into()),
                is_name: true,
                category: "Astrogation".into(),
                settings: FieldSettings {
                    required: true,
                    max_len: Some(48),
                    doc: "Named region of the galactic archive map.".into(),
                    ..FieldSettings::default()
                },
            },
            Field {
                id: sample_id(&format!("{base}#galactic_xyz")),
                name: "galactic_xyz".into(),
                ty: FieldType::Vec3,
                default: DataValue::Vec3([0.0, 0.0, 0.0]),
                is_name: false,
                category: "Astrogation".into(),
                settings: FieldSettings {
                    min: Some(-1000.0),
                    max: Some(1000.0),
                    step: Some(0.5),
                    doc: "Parsec offset from the Grand Archive meridian.".into(),
                    ..FieldSettings::default()
                },
            },
        ],
    }
}

fn make_sector_coords_value(sector_coords: &DtoAsset, sector_name: &str, xyz: [f32; 3]) -> DataValue {
    let f_name = sector_coords.field_by_name("sector_name").unwrap().id;
    let f_xyz = sector_coords.field_by_name("galactic_xyz").unwrap().id;
    DataValue::Dto(BTreeMap::from([(f_name, DataValue::String(sector_name.into())), (f_xyz, DataValue::Vec3(xyz))]))
}

fn planet_dto(sector_coords: &DtoAsset) -> DtoAsset {
    let base = "quillshade/data/lore/planet.dto.ron";
    DtoAsset {
        format_version: DTO_FORMAT_VERSION,
        id: sample_id(base),
        doc: "Homeworld metadata and astrogation coordinates.".into(),
        fields: vec![
            Field {
                id: sample_id(&format!("{base}#planet_name")),
                name: "planet_name".into(),
                ty: FieldType::String,
                default: DataValue::String("Aethelgard".into()),
                is_name: true,
                category: "World".into(),
                settings: FieldSettings {
                    required: true,
                    max_len: Some(48),
                    doc: "Canonical planetary name.".into(),
                    ..FieldSettings::default()
                },
            },
            Field {
                id: sample_id(&format!("{base}#gravity_g")),
                name: "gravity_g".into(),
                ty: FieldType::F32,
                default: DataValue::F32(1.0),
                is_name: false,
                category: "World".into(),
                settings: FieldSettings {
                    min: Some(0.1),
                    max: Some(10.0),
                    step: Some(0.05),
                    doc: "Surface gravity in standard G.".into(),
                    ..FieldSettings::default()
                },
            },
            Field {
                id: sample_id(&format!("{base}#coordinates")),
                name: "coordinates".into(),
                ty: FieldType::Dto(sector_coords.id),
                default: make_sector_coords_value(sector_coords, "Coreward", [0.0, 0.0, 0.0]),
                is_name: true,
                category: "Astrogation".into(),
                settings: FieldSettings {
                    doc: "Galactic sector coordinates of this world.".into(),
                    ..FieldSettings::default()
                },
            },
        ],
    }
}

fn make_planet_value(
    planet: &DtoAsset,
    sector_coords: &DtoAsset,
    planet_name: &str,
    gravity_g: f32,
    sector_name: &str,
    xyz: [f32; 3],
) -> DataValue {
    let f_name = planet.field_by_name("planet_name").unwrap().id;
    let f_grav = planet.field_by_name("gravity_g").unwrap().id;
    let f_coords = planet.field_by_name("coordinates").unwrap().id;
    DataValue::Dto(BTreeMap::from([
        (f_name, DataValue::String(planet_name.into())),
        (f_grav, DataValue::F32(gravity_g)),
        (f_coords, make_sector_coords_value(sector_coords, sector_name, xyz)),
    ]))
}

fn author_dto(planet: &DtoAsset, sector_coords: &DtoAsset) -> DtoAsset {
    let base = "quillshade/data/library/author.dto.ron";
    DtoAsset {
        format_version: DTO_FORMAT_VERSION,
        id: sample_id(base),
        doc: "Attributed scribe or scholar in the Grand Archive registry.".into(),
        fields: vec![
            Field {
                id: sample_id(&format!("{base}#pen_name")),
                name: "pen_name".into(),
                ty: FieldType::String,
                default: DataValue::String("Anonymous".into()),
                is_name: true,
                category: "Identity".into(),
                settings: FieldSettings {
                    required: true,
                    max_len: Some(48),
                    doc: "Attributed author or scribe pen name.".into(),
                    ..FieldSettings::default()
                },
            },
            Field {
                id: sample_id(&format!("{base}#guild_rank")),
                name: "guild_rank".into(),
                ty: FieldType::U8,
                default: DataValue::U8(1),
                is_name: false,
                category: "Identity".into(),
                settings: FieldSettings {
                    min: Some(1.0),
                    max: Some(10.0),
                    step: Some(1.0),
                    doc: "Archivist guild tier (1..=10).".into(),
                    ..FieldSettings::default()
                },
            },
            Field {
                id: sample_id(&format!("{base}#homeworld")),
                name: "homeworld".into(),
                ty: FieldType::Dto(planet.id),
                default: make_planet_value(planet, sector_coords, "Aethelgard", 1.0, "Coreward", [0.0, 0.0, 0.0]),
                is_name: true,
                category: "Origin".into(),
                settings: FieldSettings {
                    doc: "Birthworld and galactic coordinates of the author.".into(),
                    ..FieldSettings::default()
                },
            },
        ],
    }
}

#[allow(clippy::too_many_arguments)]
fn make_author_value(
    author: &DtoAsset,
    planet: &DtoAsset,
    sector_coords: &DtoAsset,
    pen_name: &str,
    guild_rank: u8,
    planet_name: &str,
    gravity_g: f32,
    sector_name: &str,
    xyz: [f32; 3],
) -> DataValue {
    let f_pen = author.field_by_name("pen_name").unwrap().id;
    let f_rank = author.field_by_name("guild_rank").unwrap().id;
    let f_home = author.field_by_name("homeworld").unwrap().id;
    DataValue::Dto(BTreeMap::from([
        (f_pen, DataValue::String(pen_name.into())),
        (f_rank, DataValue::U8(guild_rank)),
        (f_home, make_planet_value(planet, sector_coords, planet_name, gravity_g, sector_name, xyz)),
    ]))
}

fn book_tag_enum() -> EnumAsset {
    let base = "quillshade/data/library/book_tag.enum.ron";
    EnumAsset {
        format_version: ENUM_FORMAT_VERSION,
        id: sample_id(base),
        doc: "Thematic subject tags assigned to tomes in the Grand Archive.".into(),
        variants: vec![
            EnumVariant {
                id: sample_id(&format!("{base}#alchemy")),
                name: "alchemy".into(),
                doc: "Alchemical transmutation and elixirs.".into(),
                fields: Vec::new(),
            },
            EnumVariant {
                id: sample_id(&format!("{base}#forbidden")),
                name: "forbidden".into(),
                doc: "Restricted or warded knowledge.".into(),
                fields: Vec::new(),
            },
            EnumVariant {
                id: sample_id(&format!("{base}#history")),
                name: "history".into(),
                doc: "Chronicles and imperial records.".into(),
                fields: Vec::new(),
            },
            EnumVariant {
                id: sample_id(&format!("{base}#star_lore")),
                name: "star_lore".into(),
                doc: "Celestial navigation and astral geometry.".into(),
                fields: Vec::new(),
            },
            EnumVariant {
                id: sample_id(&format!("{base}#treatise")),
                name: "treatise".into(),
                doc: "Academic or botanical monograph.".into(),
                fields: Vec::new(),
            },
        ],
    }
}

fn enchantment_enum() -> EnumAsset {
    let base = "quillshade/data/library/enchantment.enum.ron";
    EnumAsset {
        format_version: ENUM_FORMAT_VERSION,
        id: sample_id(base),
        doc: "Protective arcane seal or temporal ward bound to a tome.".into(),
        variants: vec![
            EnumVariant {
                id: sample_id(&format!("{base}#unenchanted")),
                name: "unenchanted".into(),
                doc: "Mundane binding with no active spell.".into(),
                fields: Vec::new(),
            },
            EnumVariant {
                id: sample_id(&format!("{base}#ward")),
                name: "ward".into(),
                doc: "Protective abjuration barrier woven into the cover.".into(),
                fields: vec![
                    Field {
                        id: sample_id(&format!("{base}#ward.strength")),
                        name: "strength".into(),
                        ty: FieldType::F32,
                        default: DataValue::F32(0.75),
                        is_name: true,
                        category: String::new(),
                        settings: FieldSettings {
                            min: Some(0.0),
                            max: Some(1.0),
                            step: Some(0.05),
                            slider: true,
                            doc: "Barrier attenuation factor (0..=1).".into(),
                            ..FieldSettings::default()
                        },
                    },
                    Field {
                        id: sample_id(&format!("{base}#ward.school")),
                        name: "school".into(),
                        ty: FieldType::LocalizedString,
                        default: DataValue::LocalizedString("Abjuration".into()),
                        is_name: true,
                        category: String::new(),
                        settings: FieldSettings {
                            required: true,
                            max_len: Some(32),
                            doc: "Arcane school of the protective ward.".into(),
                            ..FieldSettings::default()
                        },
                    },
                ],
            },
            EnumVariant {
                id: sample_id(&format!("{base}#chrono_lock")),
                name: "chrono_lock".into(),
                doc: "Temporal stasis seal preserving fragile pages.".into(),
                fields: vec![
                    Field {
                        id: sample_id(&format!("{base}#chrono_lock.seal_duration")),
                        name: "seal_duration".into(),
                        ty: FieldType::Duration,
                        default: DataValue::Duration(30.0),
                        is_name: true,
                        category: String::new(),
                        settings: FieldSettings {
                            min: Some(1.0),
                            max: Some(3600.0),
                            step: Some(5.0),
                            doc: "Stasis cycle duration in seconds.".into(),
                            ..FieldSettings::default()
                        },
                    },
                    Field {
                        id: sample_id(&format!("{base}#chrono_lock.paradox_safe")),
                        name: "paradox_safe".into(),
                        ty: FieldType::Bool,
                        default: DataValue::Bool(true),
                        is_name: false,
                        category: String::new(),
                        settings: FieldSettings {
                            doc: "Whether temporal readers are shielded from causal loops.".into(),
                            ..FieldSettings::default()
                        },
                    },
                ],
            },
        ],
    }
}

fn make_enchantment_unenchanted(enchantment: &EnumAsset) -> DataValue {
    let vid = enchantment.variant_by_name("unenchanted").unwrap().id;
    DataValue::Enum { variant: vid, fields: BTreeMap::new() }
}

fn make_enchantment_ward(enchantment: &EnumAsset, strength: f32, school: &str) -> DataValue {
    let v = enchantment.variant_by_name("ward").unwrap();
    let f_strength = v.fields.iter().find(|f| f.name == "strength").unwrap().id;
    let f_school = v.fields.iter().find(|f| f.name == "school").unwrap().id;
    DataValue::Enum {
        variant: v.id,
        fields: BTreeMap::from([
            (f_strength, DataValue::F32(strength)),
            (f_school, DataValue::LocalizedString(school.into())),
        ]),
    }
}

fn make_enchantment_chrono_lock(enchantment: &EnumAsset, seal_duration: f32, paradox_safe: bool) -> DataValue {
    let v = enchantment.variant_by_name("chrono_lock").unwrap();
    let f_dur = v.fields.iter().find(|f| f.name == "seal_duration").unwrap().id;
    let f_safe = v.fields.iter().find(|f| f.name == "paradox_safe").unwrap().id;
    DataValue::Enum {
        variant: v.id,
        fields: BTreeMap::from([(f_dur, DataValue::Duration(seal_duration)), (f_safe, DataValue::Bool(paradox_safe))]),
    }
}

fn make_tag_val(book_tag: &EnumAsset, name: &str) -> DataValue {
    let vid = book_tag.variant_by_name(name).unwrap().id;
    DataValue::Enum { variant: vid, fields: BTreeMap::new() }
}

fn book_dto(
    author: &DtoAsset,
    planet: &DtoAsset,
    sector_coords: &DtoAsset,
    book_tag: &EnumAsset,
    enchantment: &EnumAsset,
) -> DtoAsset {
    let base = "quillshade/data/library/book.dto.ron";
    DtoAsset {
        format_version: DTO_FORMAT_VERSION,
        id: sample_id(base),
        doc: "Catalog entry for a physical book displayed on the 3D library shelf.".into(),
        fields: vec![
            Field {
                id: sample_id(&format!("{base}#title")),
                name: "title".into(),
                ty: FieldType::String,
                default: DataValue::String("Untitled Tome".into()),
                is_name: true,
                category: "Identity".into(),
                settings: FieldSettings {
                    required: true,
                    max_len: Some(64),
                    doc: "Display title of the tome.".into(),
                    ..FieldSettings::default()
                },
            },
            Field {
                id: sample_id(&format!("{base}#author")),
                name: "author".into(),
                ty: FieldType::Dto(author.id),
                default: make_author_value(
                    author,
                    planet,
                    sector_coords,
                    "Anonymous",
                    1,
                    "Aethelgard",
                    1.0,
                    "Coreward",
                    [0.0, 0.0, 0.0],
                ),
                is_name: true,
                category: "Identity".into(),
                settings: FieldSettings {
                    doc: "Attributed author and origin metadata.".into(),
                    ..FieldSettings::default()
                },
            },
            Field {
                id: sample_id(&format!("{base}#pages")),
                name: "pages".into(),
                ty: FieldType::U32,
                default: DataValue::U32(120),
                is_name: true,
                category: "Specs".into(),
                settings: FieldSettings {
                    min: Some(1.0),
                    max: Some(2000.0),
                    step: Some(10.0),
                    doc: "Total leaf count.".into(),
                    ..FieldSettings::default()
                },
            },
            Field {
                id: sample_id(&format!("{base}#weight_kg")),
                name: "weight_kg".into(),
                ty: FieldType::F32,
                default: DataValue::F32(0.8),
                is_name: false,
                category: "Specs".into(),
                settings: FieldSettings {
                    min: Some(0.05),
                    max: Some(25.0),
                    step: Some(0.05),
                    doc: "Physical weight in kilograms.".into(),
                    ..FieldSettings::default()
                },
            },
            Field {
                id: sample_id(&format!("{base}#cover_tint")),
                name: "cover_tint".into(),
                ty: FieldType::Color,
                default: DataValue::Color([0.45, 0.25, 0.15, 1.0]),
                is_name: false,
                category: "Appearance".into(),
                settings: FieldSettings::default(),
            },
            Field {
                id: sample_id(&format!("{base}#dimensions_cm")),
                name: "dimensions_cm".into(),
                ty: FieldType::Vec3,
                default: DataValue::Vec3([15.0, 22.0, 3.0]),
                is_name: false,
                category: "Appearance".into(),
                settings: FieldSettings {
                    min: Some(1.0),
                    max: Some(100.0),
                    step: Some(0.5),
                    doc: "Book width (X), height (Y), and spine thickness (Z) in centimeters.".into(),
                    ..FieldSettings::default()
                },
            },
            Field {
                id: sample_id(&format!("{base}#is_rare_edition")),
                name: "is_rare_edition".into(),
                ty: FieldType::Bool,
                default: DataValue::Bool(false),
                is_name: false,
                category: "Catalog".into(),
                settings: FieldSettings::default(),
            },
            Field {
                id: sample_id(&format!("{base}#wear_curve")),
                name: "wear_curve".into(),
                ty: FieldType::Curve,
                default: DataValue::Curve(sample_id(MAT_CURVE_PULSE)),
                is_name: false,
                category: "Visuals".into(),
                settings: FieldSettings {
                    doc: "Aging and cover-sheen transfer curve.".into(),
                    ..FieldSettings::default()
                },
            },
            Field {
                id: sample_id(&format!("{base}#spine_gradient")),
                name: "spine_gradient".into(),
                ty: FieldType::Gradient,
                default: DataValue::Gradient(sample_id(MAT_GRAD_TERRAIN), 1007),
                is_name: false,
                category: "Visuals".into(),
                settings: FieldSettings {
                    doc: "OKLab color ramp across the book spine.".into(),
                    ..FieldSettings::default()
                },
            },
            Field {
                id: sample_id(&format!("{base}#cover_material")),
                name: "cover_material".into(),
                ty: FieldType::Material,
                default: DataValue::Material(sample_id(MAT_ROCK_WET)),
                is_name: false,
                category: "Visuals".into(),
                settings: FieldSettings {
                    doc: "Quillshade material asset assigned to the book binding.".into(),
                    ..FieldSettings::default()
                },
            },
            Field {
                id: sample_id(&format!("{base}#seal_texture")),
                name: "seal_texture".into(),
                ty: FieldType::Texture,
                default: DataValue::Texture(sample_id(MAT_TEX_ATLAS)),
                is_name: false,
                category: "Visuals".into(),
                settings: FieldSettings {
                    doc: "Quillshade texture policy for the guild wax seal.".into(),
                    ..FieldSettings::default()
                },
            },
            Field {
                id: sample_id(&format!("{base}#reading_ambience")),
                name: "reading_ambience".into(),
                ty: FieldType::LightEnvironment,
                default: DataValue::LightEnvironment(sample_id(LIGHT_ENV_DAY)),
                is_name: false,
                category: "Visuals".into(),
                settings: FieldSettings {
                    doc: "Recommended reading lighting preset (`LightPreset`).".into(),
                    ..FieldSettings::default()
                },
            },
            Field {
                id: sample_id(&format!("{base}#ex_libris_stamp")),
                name: "ex_libris_stamp".into(),
                ty: FieldType::GameAsset(GameAssetKind::Image),
                default: DataValue::GameAsset("textures/atlas.png".into()),
                is_name: false,
                category: "Visuals".into(),
                settings: FieldSettings {
                    doc: "Raw game image path (`assets/<path>`) for the bookplate stamp.".into(),
                    ..FieldSettings::default()
                },
            },
            Field {
                id: sample_id(&format!("{base}#companion_volume")),
                name: "companion_volume".into(),
                ty: FieldType::SoftRef(sample_id(base)),
                default: DataValue::SoftRef(AssetUuid::NIL),
                is_name: false,
                category: "Catalog".into(),
                settings: FieldSettings {
                    doc: "Optional companion or sequel tome in the archive (`SoftRef<Book>`).".into(),
                    ..FieldSettings::default()
                },
            },
            Field {
                id: sample_id(&format!("{base}#tags")),
                name: "tags".into(),
                ty: FieldType::Set(Box::new(FieldType::Enum(book_tag.id))),
                default: DataValue::Set(Vec::new()),
                is_name: false,
                category: "Classification & Lore".into(),
                settings: FieldSettings {
                    doc: "Subject classification tags (`BTreeSet<BookTag>`).".into(),
                    ..FieldSettings::default()
                },
            },
            Field {
                id: sample_id(&format!("{base}#enchantment")),
                name: "enchantment".into(),
                ty: FieldType::Enum(enchantment.id),
                default: make_enchantment_unenchanted(enchantment),
                is_name: false,
                category: "Arcane & Effects".into(),
                settings: FieldSettings {
                    doc: "Protective arcane enchantment (`Enchantment` tagged union).".into(),
                    ..FieldSettings::default()
                },
            },
            Field {
                id: sample_id(&format!("{base}#chapters")),
                name: "chapters".into(),
                ty: FieldType::Array(Box::new(FieldType::LocalizedString)),
                default: DataValue::Array(Vec::new()),
                is_name: false,
                category: "Content & Structure".into(),
                settings: FieldSettings {
                    doc: "Ordered chapter headings (`Vec<LocalizedString>`).".into(),
                    ..FieldSettings::default()
                },
            },
            Field {
                id: sample_id(&format!("{base}#errata_note")),
                name: "errata_note".into(),
                ty: FieldType::Option(Box::new(FieldType::LocalizedString)),
                default: DataValue::Option(None),
                is_name: false,
                category: "Content & Structure".into(),
                settings: FieldSettings {
                    doc: "Optional archivist errata or margin note (`Option<LocalizedString>`).".into(),
                    ..FieldSettings::default()
                },
            },
        ],
    }
}

fn books_core_collection(
    book_dto: &DtoAsset,
    author_dto: &DtoAsset,
    planet_dto: &DtoAsset,
    sector_coords_dto: &DtoAsset,
    book_tag: &EnumAsset,
    enchantment: &EnumAsset,
) -> CollectionAsset {
    let base = "quillshade/data/library/books_core.collection.ron";
    let field_id = |name: &str| -> AssetUuid {
        book_dto.fields.iter().find(|f| f.name == name).unwrap_or_else(|| panic!("missing field {name}")).id
    };
    let f_title = field_id("title");
    let f_author = field_id("author");
    let f_pages = field_id("pages");
    let f_weight = field_id("weight_kg");
    let f_tint = field_id("cover_tint");
    let f_dims = field_id("dimensions_cm");
    let f_rare = field_id("is_rare_edition");
    let f_curve = field_id("wear_curve");
    let f_grad = field_id("spine_gradient");
    let f_mat = field_id("cover_material");
    let f_tex = field_id("seal_texture");
    let f_env = field_id("reading_ambience");
    let f_stamp = field_id("ex_libris_stamp");
    let f_companion = field_id("companion_volume");
    let f_tags = field_id("tags");
    let f_enchantment = field_id("enchantment");
    let f_chapters = field_id("chapters");
    let f_errata = field_id("errata_note");

    let specs = [
        (
            "row0",
            "Codex of Emberlight",
            ("Aurelia Vance", 9u8, "Solis Prime", 1.15f32, "Helios Reach", [12.5, 4.0, -3.2]),
            340u32,
            1.45f32,
            [0.82, 0.24, 0.14, 1.0],
            [18.0, 26.0, 4.2],
            true,
            (MAT_CURVE_PULSE, 1007u32, MAT_ROCK_WET, MAT_TEX_ATLAS, LIGHT_ENV_DAY, "textures/atlas.png"),
            Some("row3"),
            vec!["alchemy", "forbidden"],
            make_enchantment_ward(enchantment, 0.85, "Pyromancy"),
            vec!["I. Ignition of the Core", "II. Cinder Lattices", "III. Rite of Ash"],
            Some("Margin warning: keep away from frost runes."),
        ),
        (
            "row1",
            "Atlas of Whispering Tides",
            ("Corvus Sterling", 7, "Thalassa", 0.88, "Pelagic Rim", [-42.0, 18.5, 9.0]),
            215,
            0.95,
            [0.16, 0.48, 0.76, 1.0],
            [16.0, 24.0, 2.8],
            false,
            (MAT_CURVE_STEPS, 1014, MAT_HOLOGRAM, MAT_TEX_SPRITE, LIGHT_ENV_NIGHT, "textures/sprite.png"),
            Some("row2"),
            vec!["history", "star_lore"],
            make_enchantment_chrono_lock(enchantment, 120.0, true),
            vec!["I. Pelagic Currents", "II. Sunken Astrolabes"],
            None,
        ),
        (
            "row2",
            "Herbarium of Sunlit Glades",
            ("Sylvia Moss", 6, "Verdania", 0.94, "Briar Expanse", [7.0, -14.0, 2.5]),
            180,
            0.72,
            [0.22, 0.64, 0.32, 1.0],
            [14.5, 21.0, 2.4],
            false,
            (LIGHT_CURVE_SMOOTH, 1007, MAT_CURVE_BANDS, MAT_TEX_ROCK_ALBEDO, LIGHT_ENV_DAY, "textures/rock_albedo.png"),
            Some("row1"),
            vec!["alchemy", "treatise"],
            make_enchantment_unenchanted(enchantment),
            vec!["I. Luminous Spores", "II. Canopy Distillation", "III. Root Elixirs"],
            None,
        ),
        (
            "row3",
            "Treatise on Astral Geometry",
            ("Orion Kael", 10, "Zenith Station", 0.42, "Orrery Core", [0.0, 120.0, 64.0]),
            480,
            1.85,
            [0.56, 0.28, 0.78, 1.0],
            [20.0, 28.0, 5.0],
            true,
            (MAT_CURVE_PULSE, 1014, MAT_EXP42, MAT_TEX_ATLAS, LIGHT_ENV_NIGHT, "textures/atlas.png"),
            Some("row0"),
            vec!["forbidden", "star_lore", "treatise"],
            make_enchantment_ward(enchantment, 0.95, "Abjuration"),
            vec!["I. Non-Euclidean Orbits", "II. Void Parallels", "III. Zenith Projections"],
            Some("Folio 42 recalibrated after the Solstice eclipse."),
        ),
        (
            "row4",
            "Chronicles of the Iron Forge",
            ("Bram Ironhand", 8, "Ferrum", 1.65, "Anvil Belt", [31.0, -8.5, -19.0]),
            290,
            1.20,
            [0.76, 0.58, 0.20, 1.0],
            [17.0, 23.5, 3.6],
            false,
            (MAT_CURVE_STEPS, 1007, MAT_GLOBAL_TINT, MAT_TEX_ROCK_ALBEDO, LIGHT_ENV_DAY, "textures/rock_normal.png"),
            None,
            vec!["history", "treatise"],
            make_enchantment_chrono_lock(enchantment, 45.0, false),
            vec!["I. Smelting the Star-Ore", "II. Anvil Covenants"],
            None,
        ),
    ];

    let rows = specs
        .into_iter()
        .map(
            |(
                tag,
                title,
                (pen_name, rank, planet_name, grav, sector_name, xyz),
                pages,
                weight,
                tint,
                dims,
                rare,
                (curve_key, grad_slot, mat_key, tex_key, env_key, stamp_path),
                companion_tag,
                tag_names,
                enchantment_val,
                chapter_titles,
                errata_opt,
            )| {
                let mut values = BTreeMap::new();
                values.insert(f_title, DataValue::String(title.into()));
                values.insert(
                    f_author,
                    make_author_value(
                        author_dto,
                        planet_dto,
                        sector_coords_dto,
                        pen_name,
                        rank,
                        planet_name,
                        grav,
                        sector_name,
                        xyz,
                    ),
                );
                values.insert(f_pages, DataValue::U32(pages));
                values.insert(f_weight, DataValue::F32(weight));
                values.insert(f_tint, DataValue::Color(tint));
                values.insert(f_dims, DataValue::Vec3(dims));
                values.insert(f_rare, DataValue::Bool(rare));
                values.insert(f_curve, DataValue::Curve(sample_id(curve_key)));
                values.insert(f_grad, DataValue::Gradient(sample_id(MAT_GRAD_TERRAIN), grad_slot));
                values.insert(f_mat, DataValue::Material(sample_id(mat_key)));
                values.insert(f_tex, DataValue::Texture(sample_id(tex_key)));
                values.insert(f_env, DataValue::LightEnvironment(sample_id(env_key)));
                values.insert(f_stamp, DataValue::GameAsset(stamp_path.into()));
                let companion_id = companion_tag.map_or(AssetUuid::NIL, |ctag| sample_id(&format!("{base}#{ctag}")));
                values.insert(f_companion, DataValue::SoftRef(companion_id));
                values
                    .insert(f_tags, DataValue::Set(tag_names.into_iter().map(|n| make_tag_val(book_tag, n)).collect()));
                values.insert(f_enchantment, enchantment_val);
                values.insert(
                    f_chapters,
                    DataValue::Array(
                        chapter_titles.into_iter().map(|c| DataValue::LocalizedString(c.into())).collect(),
                    ),
                );
                values.insert(
                    f_errata,
                    DataValue::Option(errata_opt.map(|s| Box::new(DataValue::LocalizedString(s.into())))),
                );
                Row { id: sample_id(&format!("{base}#{tag}")), values }
            },
        )
        .collect();

    CollectionAsset {
        format_version: data_domain::backend::COLLECTION_FORMAT_VERSION,
        id: sample_id(base),
        dto: book_dto.id,
        rows,
    }
}

fn library_rules_dto(book: &DtoAsset, book_tag: &EnumAsset) -> DtoAsset {
    let base = "quillshade/data/library/library_rules.dto.ron";
    DtoAsset {
        format_version: DTO_FORMAT_VERSION,
        id: sample_id(base),
        doc: "Global display and loan policy parameters for the Grand Archive shelf.".into(),
        fields: vec![
            Field {
                id: sample_id(&format!("{base}#shelf_label")),
                name: "shelf_label".into(),
                ty: FieldType::String,
                default: DataValue::String("Grand Archive - Shelf A".into()),
                is_name: true,
                category: "General".into(),
                settings: FieldSettings {
                    required: true,
                    max_len: Some(64),
                    doc: "Display banner for the library shelf.".into(),
                    ..FieldSettings::default()
                },
            },
            Field {
                id: sample_id(&format!("{base}#max_loan_days")),
                name: "max_loan_days".into(),
                ty: FieldType::U32,
                default: DataValue::U32(14),
                is_name: true,
                category: "Loan Policy".into(),
                settings: FieldSettings {
                    min: Some(1.0),
                    max: Some(90.0),
                    step: Some(1.0),
                    doc: "Maximum days a tome may be borrowed.".into(),
                    ..FieldSettings::default()
                },
            },
            Field {
                id: sample_id(&format!("{base}#late_fee_coins_per_day")),
                name: "late_fee_coins_per_day".into(),
                ty: FieldType::F32,
                default: DataValue::F32(2.5),
                is_name: false,
                category: "Loan Policy".into(),
                settings: FieldSettings {
                    min: Some(0.0),
                    max: Some(100.0),
                    step: Some(0.5),
                    doc: "Overdue fee per day in gold coins.".into(),
                    ..FieldSettings::default()
                },
            },
            Field {
                id: sample_id(&format!("{base}#shelf_spacing_m")),
                name: "shelf_spacing_m".into(),
                ty: FieldType::F32,
                default: DataValue::F32(0.28),
                is_name: false,
                category: "Display".into(),
                settings: FieldSettings {
                    min: Some(0.1),
                    max: Some(1.0),
                    step: Some(0.02),
                    doc: "Horizontal spacing between books on the 3D shelf in meters.".into(),
                    ..FieldSettings::default()
                },
            },
            Field {
                id: sample_id(&format!("{base}#book_scale")),
                name: "book_scale".into(),
                ty: FieldType::F32,
                default: DataValue::F32(3.2),
                is_name: false,
                category: "Display".into(),
                settings: FieldSettings {
                    min: Some(0.5),
                    max: Some(10.0),
                    step: Some(0.1),
                    doc: "Visual multiplier applied to book dimensions on the 3D shelf.".into(),
                    ..FieldSettings::default()
                },
            },
            Field {
                id: sample_id(&format!("{base}#rare_glow_boost")),
                name: "rare_glow_boost".into(),
                ty: FieldType::F32,
                default: DataValue::F32(1.5),
                is_name: false,
                category: "Display".into(),
                settings: FieldSettings {
                    min: Some(0.0),
                    max: Some(5.0),
                    step: Some(0.1),
                    doc: "Emissive multiplier applied to rare editions.".into(),
                    ..FieldSettings::default()
                },
            },
            Field {
                id: sample_id(&format!("{base}#featured_book")),
                name: "featured_book".into(),
                ty: FieldType::SoftRef(book.id),
                default: DataValue::SoftRef(sample_id("quillshade/data/library/books_core.collection.ron#row0")),
                is_name: false,
                category: "Display".into(),
                settings: FieldSettings {
                    doc: "Featured tome highlighted on the Grand Archive banner (`SoftRef<Book>`).".into(),
                    ..FieldSettings::default()
                },
            },
            Field {
                id: sample_id(&format!("{base}#shelf_quotas")),
                name: "shelf_quotas".into(),
                ty: FieldType::Map(Box::new(FieldType::Enum(book_tag.id)), Box::new(FieldType::U32)),
                default: DataValue::Map(Vec::new()),
                is_name: false,
                category: "Quotas".into(),
                settings: FieldSettings {
                    doc: "Maximum allowed volumes per subject tag (`BTreeMap<BookTag, u32>`).".into(),
                    ..FieldSettings::default()
                },
            },
        ],
    }
}

fn library_rules_collection(rules_dto: &DtoAsset, book_tag: &EnumAsset) -> CollectionAsset {
    let base = "quillshade/data/library/library_rules.collection.ron";
    let mut values = BTreeMap::new();
    for field in &rules_dto.fields {
        values.insert(field.id, field.default.clone());
    }
    let f_quotas = rules_dto.field_by_name("shelf_quotas").unwrap().id;
    values.insert(
        f_quotas,
        DataValue::Map(vec![
            (make_tag_val(book_tag, "alchemy"), DataValue::U32(12)),
            (make_tag_val(book_tag, "forbidden"), DataValue::U32(3)),
            (make_tag_val(book_tag, "history"), DataValue::U32(10)),
            (make_tag_val(book_tag, "star_lore"), DataValue::U32(8)),
            (make_tag_val(book_tag, "treatise"), DataValue::U32(15)),
        ]),
    );
    CollectionAsset {
        format_version: data_domain::backend::COLLECTION_FORMAT_VERSION,
        id: sample_id(base),
        dto: rules_dto.id,
        rows: vec![Row { id: sample_id(&format!("{base}#row0")), values }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_game() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/sample_game")
    }

    #[test]
    fn sample_files_are_up_to_date() {
        let p = Project::open(sample_game()).unwrap();
        let stale = "stale: rerun `cargo run -p data_domain --example make_sample`";
        let dir = tempfile::tempdir().unwrap();
        write_sources(dir.path()).unwrap();
        let refs = [
            enum_ref(BOOK_TAG_ENUM),
            enum_ref(ENCHANTMENT_ENUM),
            dto_ref(SECTOR_COORDS_DTO),
            dto_ref(PLANET_DTO),
            dto_ref(AUTHOR_DTO),
            dto_ref(BOOK_DTO),
            collection_ref(BOOKS_CORE_COLLECTION),
            dto_ref(LIBRARY_RULES_DTO),
            collection_ref(LIBRARY_RULES_COLLECTION),
        ];
        for r in refs {
            let read = |root: &Path| std::fs::read_to_string(root.join(r.source_rel())).unwrap();
            assert_eq!(read(&p.root), read(dir.path()), "{r} is {stale}");
        }
    }
}
