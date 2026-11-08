//! Integration tests for `quillshade::data` inside `examples/sample_game`.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::time::{Duration, Instant};

use bevy::prelude::*;
use quillshade::data::{
    Author, Book, BookTag, Data, DataDomainPlugin, DataStore, Enchantment, LibraryRules, LocalizedString, Planet,
    SectorCoords, ToShortViewString, collections,
};

#[test]
fn book_and_library_rules_defaults_and_short_view_string() {
    fn assert_unit_enum_traits<T: Copy + Eq + Ord + std::hash::Hash + Default>() {}
    assert_unit_enum_traits::<BookTag>();
    assert_eq!(BookTag::default(), BookTag::Alchemy);
    assert_eq!(BookTag::StarLore.to_short_view_string(), "StarLore");
    let mut hash_set = HashSet::new();
    hash_set.insert(BookTag::Alchemy);
    hash_set.insert(BookTag::Forbidden);
    assert_eq!(hash_set.len(), 2);

    assert_eq!(Enchantment::default(), Enchantment::Unenchanted);
    assert_eq!(Enchantment::Unenchanted.to_short_view_string(), "Unenchanted");
    let ward = Enchantment::Ward { strength: 0.75, school: LocalizedString("Abjuration".into()) };
    assert_eq!(ward.to_short_view_string(), "Ward(0.75 | Abjuration)");
    let chrono = Enchantment::ChronoLock { seal_duration: Duration::from_secs(30), paradox_safe: true };
    assert_eq!(chrono.to_short_view_string(), "ChronoLock(30s)");

    let book = Book::default();
    assert_eq!(book.title, "Untitled Tome");
    assert_eq!(book.author, Author::default());
    assert_eq!(book.author.pen_name, "Anonymous");
    assert_eq!(book.author.guild_rank, 1);
    assert_eq!(book.author.homeworld, Planet::default());
    assert_eq!(book.author.homeworld.planet_name, "Aethelgard");
    assert!((book.author.homeworld.gravity_g - 1.0).abs() < 1e-6);
    assert_eq!(book.author.homeworld.coordinates, SectorCoords::default());
    assert_eq!(book.author.homeworld.coordinates.sector_name, "Coreward");
    assert_eq!(book.author.homeworld.coordinates.galactic_xyz, Vec3::new(0.0, 0.0, 0.0));
    assert_eq!(book.pages, 120);
    assert!((book.weight_kg - 0.8).abs() < 1e-6);
    assert_eq!(book.dimensions_cm, Vec3::new(15.0, 22.0, 3.0));
    assert!(!book.is_rare_edition);
    assert_eq!(book.wear_curve.points.len(), 5);
    assert!((book.wear_curve.sample(0.25) - 1.0).abs() < 1e-5);
    assert_eq!(book.spine_gradient.stops.len(), 4);
    assert_ne!(book.spine_gradient.sample(0.5), Color::BLACK);
    assert!(book.cover_material.is_some());
    assert!(book.seal_texture.is_some());
    assert_eq!(book.reading_ambience, Some(quillshade::light::Preset::Day));
    assert!(book.ex_libris_stamp.is_some());
    assert!(book.companion_volume.is_none());
    assert!(book.tags.is_empty());
    assert_eq!(book.enchantment, Enchantment::Unenchanted);
    assert!(book.chapters.is_empty());
    assert_eq!(book.errata_note, None);
    assert_eq!(book.to_short_view_string(), "Untitled Tome | Anonymous | Aethelgard | Coreward | 120");

    let rules = LibraryRules::default();
    assert_eq!(rules.shelf_label, "Grand Archive - Shelf A");
    assert_eq!(rules.max_loan_days, 14);
    assert!((rules.late_fee_coins_per_day - 2.5).abs() < 1e-6);
    assert!((rules.shelf_spacing_m - 0.28).abs() < 1e-6);
    assert!((rules.book_scale - 3.2).abs() < 1e-6);
    assert!((rules.rare_glow_boost - 1.5).abs() < 1e-6);
    assert!(rules.featured_book.is_some());
    assert!(rules.shelf_quotas.is_empty());
    assert_eq!(rules.to_short_view_string(), "Grand Archive - Shelf A | 14");
}

#[test]
fn data_domain_plugin_loads_baked_qsd_collections_and_resolves_rows() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin { file_path: format!("{}/assets", env!("CARGO_MANIFEST_DIR")), ..default() },
        DataDomainPlugin,
    ));

    #[derive(Resource, Default)]
    struct Probe {
        ready: bool,
        books_len: usize,
        row0_title: String,
        row0_short: String,
        rules_short: String,
    }
    app.init_resource::<Probe>();

    app.add_systems(Update, |data: Data, mut probe: ResMut<Probe>| {
        if !data.is_ready() || data.collection_library_books_core.is_empty() {
            return;
        }
        let Some((row0_id, row0)) = data.collection_library_books_core.first_with_id() else {
            return;
        };
        let Some(rules0) = data.collection_library_rules.first() else {
            return;
        };
        // Verify O(1) lookup by RowUuid and by string UUID:
        assert_eq!(data.collection_library_books_core.get(row0_id), Some(row0));
        let row0_id_str = row0_id.to_string();
        assert_eq!(data.collection_library_books_core.get(row0_id_str.as_str()), Some(row0));

        // Verify SoftRef<Book> resolution via `.resolve(&data)` and `CollectionHandle::get(SoftRef<Book>)`:
        assert_eq!(rules0.featured_book.resolve(&data).map(|b| b.title.as_str()), Some("Codex of Emberlight"));
        assert_eq!(row0.companion_volume.resolve(&data).map(|b| b.title.as_str()), Some("Treatise on Astral Geometry"));
        assert_eq!(
            data.collection_library_books_core.get(row0.companion_volume).map(|b| b.title.as_str()),
            Some("Treatise on Astral Geometry")
        );

        // Verify direct iteration over `data.collection_library_books_core`:
        let mut count = 0;
        for (id, book) in data.collection_library_books_core {
            assert_eq!(data.collection_library_books_core.get(id), Some(book));
            count += 1;
        }
        assert_eq!(count, data.collection_library_books_core.len);

        probe.ready = true;
        probe.books_len = data.collection_library_books_core.len;
        probe.row0_title = row0.title.clone();
        probe.row0_short = row0.to_short_view_string();
        probe.rules_short = rules0.to_short_view_string();
    });

    let deadline = Instant::now() + Duration::from_secs(5);
    while !app.world().resource::<Probe>().ready && Instant::now() < deadline {
        app.update();
        std::thread::sleep(Duration::from_millis(5));
    }

    let probe = app.world().resource::<Probe>();
    assert!(probe.ready, "timed out waiting for .collection.qsd assets to load");
    assert_eq!(probe.books_len, 5);
    assert_eq!(probe.row0_title, "Codex of Emberlight");
    assert_eq!(probe.row0_short, "Codex of Emberlight | Aurelia Vance | Solis Prime | Helios Reach | 340");
    assert_eq!(probe.rules_short, "Grand Archive - Shelf A | 14");

    // Also verify `DataStore::collection` (returns `&Collection<Book>`, no Option) and 3-level nested DTO fields
    let store = app.world().resource::<DataStore>();
    let books = store.collection::<Book>(collections::BOOKS_CORE);
    assert_eq!(books.len(), 5);
    let first_book = books.first().unwrap();
    assert_eq!(first_book.author.pen_name, "Aurelia Vance");
    assert_eq!(first_book.author.guild_rank, 9);
    assert_eq!(first_book.author.homeworld.planet_name, "Solis Prime");
    assert!((first_book.author.homeworld.gravity_g - 1.15).abs() < 1e-5);
    assert_eq!(first_book.author.homeworld.coordinates.sector_name, "Helios Reach");
    assert_eq!(first_book.author.homeworld.coordinates.galactic_xyz, Vec3::new(12.5, 4.0, -3.2));
    assert_eq!(first_book.wear_curve.points.len(), 5);
    assert!((first_book.wear_curve.sample(0.25) - 1.0).abs() < 1e-5);
    assert_eq!(first_book.spine_gradient.stops.len(), 4);
    assert_ne!(first_book.spine_gradient.sample(0.5), Color::BLACK);
    assert!(first_book.cover_material.is_some());
    assert!(first_book.seal_texture.is_some());
    assert_eq!(
        first_book.seal_texture.as_ref().and_then(|h| h.path()).map(|p| p.to_string()),
        Some("textures/atlas.png".to_string())
    );
    assert_eq!(first_book.reading_ambience, Some(quillshade::light::Preset::Day));
    assert!(first_book.ex_libris_stamp.is_some());
    assert_eq!(first_book.tags, BTreeSet::from([BookTag::Alchemy, BookTag::Forbidden]));
    assert_eq!(
        first_book.enchantment,
        Enchantment::Ward { strength: 0.85, school: LocalizedString("Pyromancy".into()) }
    );
    assert_eq!(
        first_book.chapters,
        vec![
            LocalizedString("I. Ignition of the Core".into()),
            LocalizedString("II. Cinder Lattices".into()),
            LocalizedString("III. Rite of Ash".into()),
        ]
    );
    assert_eq!(first_book.errata_note, Some(LocalizedString("Margin warning: keep away from frost runes.".into())));

    for (_, b) in books.iter() {
        assert!(
            b.seal_texture.as_ref().and_then(|h| h.path()).is_some(),
            "every book in books_core must have a path-backed seal_texture handle, got {:?}",
            b.seal_texture
        );
    }

    let second_book = &books.iter().nth(1).unwrap().1;
    assert_eq!(second_book.title, "Atlas of Whispering Tides");
    assert_eq!(second_book.spine_gradient.stops.len(), 3);
    assert!(second_book.cover_material.is_some());
    assert!(second_book.seal_texture.is_some());
    assert_eq!(second_book.reading_ambience, Some(quillshade::light::Preset::Night));
    assert!(second_book.ex_libris_stamp.is_some());
    assert_eq!(second_book.tags, BTreeSet::from([BookTag::History, BookTag::StarLore]));
    assert_eq!(
        second_book.enchantment,
        Enchantment::ChronoLock { seal_duration: Duration::from_secs(120), paradox_safe: true }
    );
    assert_eq!(
        second_book.chapters,
        vec![LocalizedString("I. Pelagic Currents".into()), LocalizedString("II. Sunken Astrolabes".into()),]
    );
    assert_eq!(second_book.errata_note, None);
    assert_eq!(second_book.companion_volume.get(store).map(|b| b.title.as_str()), Some("Herbarium of Sunlit Glades"));
    assert_eq!(first_book.companion_volume.get(store).map(|b| b.title.as_str()), Some("Treatise on Astral Geometry"));

    let third_book = &books.iter().nth(2).unwrap().1;
    assert_eq!(third_book.title, "Herbarium of Sunlit Glades");
    assert_eq!(third_book.tags, BTreeSet::from([BookTag::Alchemy, BookTag::Treatise]));
    assert_eq!(third_book.enchantment, Enchantment::Unenchanted);
    assert_eq!(third_book.chapters.len(), 3);
    assert_eq!(third_book.errata_note, None);

    let fourth_book = &books.iter().nth(3).unwrap().1;
    assert_eq!(fourth_book.title, "Treatise on Astral Geometry");
    assert_eq!(fourth_book.tags, BTreeSet::from([BookTag::Forbidden, BookTag::StarLore, BookTag::Treatise]));
    assert_eq!(
        fourth_book.enchantment,
        Enchantment::Ward { strength: 0.95, school: LocalizedString("Abjuration".into()) }
    );
    assert_eq!(
        fourth_book.errata_note,
        Some(LocalizedString("Folio 42 recalibrated after the Solstice eclipse.".into()))
    );

    let rules_col = store.collection::<LibraryRules>(collections::LIBRARY_RULES);
    let rules0 = rules_col.first().unwrap();
    assert_eq!(rules0.featured_book.get(store).map(|b| b.title.as_str()), Some("Codex of Emberlight"));
    assert_eq!(
        rules0.shelf_quotas,
        BTreeMap::from([
            (BookTag::Alchemy, 12),
            (BookTag::Forbidden, 3),
            (BookTag::History, 10),
            (BookTag::StarLore, 8),
            (BookTag::Treatise, 15),
        ])
    );

    let fifth_book = &books.iter().nth(4).unwrap().1;
    assert_eq!(fifth_book.title, "Chronicles of the Iron Forge");
    assert!(fifth_book.companion_volume.is_none());
    assert!(fifth_book.companion_volume.get(store).is_none());
    assert_eq!(fifth_book.tags, BTreeSet::from([BookTag::History, BookTag::Treatise]));
    assert_eq!(
        fifth_book.enchantment,
        Enchantment::ChronoLock { seal_duration: Duration::from_secs(45), paradox_safe: false }
    );
    assert_eq!(fifth_book.errata_note, None);
}
