//! Baked materials runtime (design §3.3), headless: no window, no renderer.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bevy::asset::io::{
    AssetReader, AssetReaderError, AssetSource, AssetSourceBuilder, AssetSourceId, ErasedAssetReader, PathStream,
    Reader, VecReader,
};
use bevy::asset::{AssetLoadFailedEvent, LoadState};
use bevy::image::{CompressedImageFormats, ImageLoader, ImageSampler};
use bevy::prelude::*;
use quillshade::material::baked::{
    AddressModeDef, BakedManifest, FilterModeDef, MANIFEST_PATH, MaterialValues, ParamValue, SamplerDef, TextureDef,
};
use quillshade::material::{
    BakedMaterials, BakedMaterialsLoader, BakedMaterialsProgress, BakedMaterialsReady, BakedTextures, CoordsMaterial,
    FlagMaterial, HologramMaterial, RockWetMaterial, TexLayersMaterial, WaterMaterial,
};

/// Messages seen by the test app.
#[derive(Resource, Default)]
struct Seen {
    progress: Vec<BakedMaterialsProgress>,
    ready: u32,
    /// What `load()` returned (`Start::Load`).
    load_count: Option<u32>,
    /// `BakedTextures` as seen by a `Startup` system running right after the one calling `load()`.
    startup_textures: Option<BakedTextures>,
    /// `AssetLoadFailedEvent<BakedManifest>` count.
    manifest_failures: u32,
    /// `AssetEvent::Modified` count of `baked.env.rock_wet`.
    rock_modified: u32,
}

fn record(
    mut seen: ResMut<Seen>,
    mut progress: MessageReader<BakedMaterialsProgress>,
    mut ready: MessageReader<BakedMaterialsReady>,
    mut failed: MessageReader<AssetLoadFailedEvent<BakedManifest>>,
    mut rock: MessageReader<AssetEvent<RockWetMaterial>>,
    baked: Res<BakedMaterials>,
) {
    seen.progress.extend(progress.read().copied());
    seen.ready += ready.read().count() as u32;
    seen.manifest_failures += failed.read().count() as u32;
    let id = baked.env.rock_wet.id();
    seen.rock_modified += rock.read().filter(|e| e.is_modified(id)).count() as u32;
}

/// Text served instead of `baked.ron` while set (garbage / another version / edited manifests, through a real
/// asset load).
#[derive(Resource, Clone, Default)]
struct ManifestText(Arc<Mutex<Option<String>>>);

impl ManifestText {
    fn set(&self, text: &str) {
        *self.0.lock().unwrap() = Some(text.to_string());
    }
}

/// The game's `assets/` reader, except for `baked.ron` while [`ManifestText`] is set.
struct OverlayReader {
    files: Box<dyn ErasedAssetReader>,
    manifest: ManifestText,
}

impl AssetReader for OverlayReader {
    async fn read<'a>(&'a self, path: &'a Path) -> Result<Box<dyn Reader + 'a>, AssetReaderError> {
        let text = self.manifest.0.lock().unwrap().clone();
        if path == Path::new(MANIFEST_PATH)
            && let Some(text) = text
        {
            return Ok(Box::new(VecReader::new(text.into_bytes())));
        }
        self.files.read(path).await
    }

    async fn read_meta<'a>(&'a self, path: &'a Path) -> Result<Box<dyn Reader + 'a>, AssetReaderError> {
        self.files.read_meta(path).await
    }

    async fn read_directory<'a>(&'a self, path: &'a Path) -> Result<Box<PathStream>, AssetReaderError> {
        self.files.read_directory(path).await
    }

    async fn is_directory<'a>(&'a self, path: &'a Path) -> Result<bool, AssetReaderError> {
        self.files.is_directory(path).await
    }
}

/// How a test app starts loading.
enum Start {
    /// `load()` in `Startup`; its result goes to `Seen::load_count`.
    Load,
    /// `load_from_str(manifest)` in `Startup`.
    FromStr(String),
    /// No call: `BakedPlugin` starts in the first `PreUpdate`.
    Auto,
}

/// `MinimalPlugins + AssetPlugin + ImagePlugin + BakedPlugin`; `load()` (or `load_from_str(manifest)`) at startup.
fn app(manifest: Option<String>) -> App {
    app_with(manifest.map_or(Start::Load, Start::FromStr))
}

fn app_with(start: Start) -> App {
    let mut app = App::new();
    let text = ManifestText::default();
    let reader_text = text.clone();
    app.register_asset_source(
        AssetSourceId::Default,
        AssetSourceBuilder::new(move || {
            let files = AssetSource::get_default_reader("assets".into())();
            Box::new(OverlayReader { files, manifest: reader_text.clone() })
        }),
    );
    app.insert_resource(text);
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin { file_path: "assets".into(), watch_for_changes_override: Some(false), ..default() },
        ImagePlugin::default(),
        quillshade::material::BakedPlugin,
    ));
    // Without the render plugin nobody registers the image loader (it only preregisters it).
    app.register_asset_loader(ImageLoader::new(CompressedImageFormats::NONE));
    app.init_resource::<Seen>();
    let load = move |mut loader: BakedMaterialsLoader, mut seen: ResMut<Seen>| match &start {
        Start::Load => seen.load_count = Some(loader.load()),
        Start::FromStr(m) => loader.load_from_str(m),
        Start::Auto => {}
    };
    let capture = |tex: Res<BakedTextures>, mut seen: ResMut<Seen>| seen.startup_textures = Some(tex.clone());
    app.add_systems(Startup, (load, capture).chain());
    app.add_systems(Update, record);
    app.finish();
    app.cleanup();
    app
}

/// Updates until `BakedMaterialsReady` arrived (then 30 more frames, to catch a second one), at most 600 frames.
fn run(app: &mut App) {
    let mut after_ready = 0;
    for _ in 0..600 {
        app.update();
        if app.world().resource::<Seen>().ready > 0 {
            after_ready += 1;
            if after_ready > 30 {
                return;
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn progress_reaches_ready() {
    let mut app = app(None);
    run(&mut app);
    let world = app.world_mut();
    let seen = world.resource::<Seen>();
    assert_eq!(seen.ready, 1, "BakedMaterialsReady once; progress: {:?}", seen.progress);
    let last = *seen.progress.last().expect("progress messages");
    let compiled = BakedManifest::parse(quillshade::material::COMPILED_MANIFEST).unwrap();
    assert_eq!(last.total as usize, compiled.textures.len());
    assert_eq!(seen.load_count, Some(last.total), "load() returns the number of textures");
    assert_eq!(last, BakedMaterialsProgress { loaded: last.total, failed: 0, total: last.total });
    let baked = world.resource::<BakedMaterials>().clone();
    assert_eq!(baked.unresolved(world), Vec::<&str>::new(), "materials not in their Assets<M>");
    let state = world.run_system_cached(|l: BakedMaterialsLoader| (l.is_ready(), l.progress())).unwrap();
    assert_eq!(state, (true, last));
    // The manifest asset is loaded for hot reload.
    assert_eq!(world.resource::<Assets<BakedManifest>>().len(), 1);
}

/// Decision 0027: without a `load()` call loading starts in the first `PreUpdate` and completes the same way.
#[test]
fn loads_without_explicit_call() {
    let mut app = app_with(Start::Auto);
    run(&mut app);
    let world = app.world_mut();
    let seen = world.resource::<Seen>();
    assert_eq!(seen.ready, 1, "progress: {:?}", seen.progress);
    let startup = seen.startup_textures.clone().expect("captured in Startup");
    assert_eq!(startup.textures.sprite, Handle::default(), "nothing loaded before the first PreUpdate");
    assert_ne!(world.resource::<BakedTextures>().textures.sprite, Handle::default());
    let baked = world.resource::<BakedMaterials>().clone();
    assert_eq!(baked.unresolved(world), Vec::<&str>::new(), "materials not in their Assets<M>");
    // A later call returns the same count and changes nothing.
    let total = seen.progress.last().unwrap().total;
    assert_eq!(world.run_system_cached(|mut l: BakedMaterialsLoader| l.load()).unwrap(), total);
}

/// Design §3.1: `load()` fills `BakedTextures` at once, so a system after it in the same schedule gets real handles.
#[test]
fn load_fills_textures_immediately() {
    let mut app = app(None);
    app.update();
    let tex = app.world().resource::<Seen>().startup_textures.clone().expect("captured in Startup");
    let compiled = BakedManifest::parse(quillshade::material::COMPILED_MANIFEST).unwrap();
    assert!(!compiled.textures.is_empty());
    for id in compiled.textures.keys() {
        let h = tex.get(id).unwrap_or_else(|| panic!("no BakedTextures field for `{id}`"));
        assert_ne!(*h, Handle::default(), "`{id}` still has the default handle");
    }
    assert_ne!(tex.textures.sprite, Handle::default());
}

/// Design §3.1: a `BakedTextures` field whose Texture asset is not in the manifest (it had an error at regeneration,
/// e.g. E011) gets the default texture; it is not counted.
#[test]
fn texture_without_manifest_entry_gets_default() {
    let entry = "    \"textures/sprite\": (\n      path: \"textures/sprite.png\",\n    ),\n";
    let manifest = quillshade::material::COMPILED_MANIFEST.replace(entry, "");
    assert_ne!(manifest, quillshade::material::COMPILED_MANIFEST);
    let total = BakedManifest::parse(&manifest).unwrap().textures.len() as u32;
    let mut app = app(Some(manifest));
    run(&mut app);
    let world = app.world();
    let tex = world.resource::<BakedTextures>();
    assert_ne!(tex.internal.default, Handle::default());
    assert_eq!(tex.textures.sprite, tex.internal.default);
    let last = *world.resource::<Seen>().progress.last().expect("progress messages");
    assert_eq!(last, BakedMaterialsProgress { loaded: total, failed: 0, total });
}

/// Review Focus 4: a texture that fails to load is counted, its materials use `@internal/default`, Ready still comes.
#[test]
fn missing_texture_counts_failed() {
    let manifest =
        quillshade::material::COMPILED_MANIFEST.replace("\"textures/atlas.png\"", "\"textures/missing.png\"");
    assert_ne!(manifest, quillshade::material::COMPILED_MANIFEST);
    let mut app = app(Some(manifest));
    run(&mut app);
    let world = app.world();
    let seen = world.resource::<Seen>();
    assert_eq!(seen.ready, 1, "BakedMaterialsReady once; progress: {:?}", seen.progress);
    let last = *seen.progress.last().expect("progress messages");
    assert_eq!(last, BakedMaterialsProgress { loaded: last.total - 1, failed: 1, total: last.total });
    let baked = world.resource::<BakedMaterials>();
    let tex = world.resource::<BakedTextures>();
    let coords = world.resource::<Assets<CoordsMaterial>>().get(&baked.fx.coords).expect("coords material");
    assert_eq!(coords.inp_atlas, tex.internal.default);
    assert_ne!(tex.internal.default, Handle::default());
}

/// Updates while `cond` is false, at most `frames` times; whether it became true.
fn run_until(app: &mut App, frames: u32, cond: impl Fn(&mut World) -> bool) -> bool {
    for _ in 0..frames {
        if cond(app.world_mut()) {
            return true;
        }
        app.update();
        std::thread::sleep(Duration::from_millis(5));
    }
    cond(app.world_mut())
}

/// Handle of the `baked.ron` asset the loader started.
fn manifest_handle(world: &World) -> Handle<BakedManifest> {
    world.resource::<AssetServer>().get_handle(MANIFEST_PATH).expect("baked.ron is being loaded")
}

/// `load()`, every texture loaded, `baked.ron` loaded and applied.
fn loaded_app() -> App {
    let mut app = app(None);
    run(&mut app);
    let loaded = run_until(&mut app, 600, |w| {
        let h = manifest_handle(w);
        w.resource::<Assets<BakedManifest>>().contains(&h)
    });
    assert!(loaded, "baked.ron did not load");
    for _ in 0..5 {
        app.update();
    }
    app
}

/// Edits the loaded manifest asset in place (what a hot reload of an edited `baked.ron` does), then lets the app
/// apply it.
fn edit_manifest(app: &mut App, f: impl FnOnce(&mut BakedManifest)) {
    let h = manifest_handle(app.world());
    f(&mut app.world_mut().resource_mut::<Assets<BakedManifest>>().get_mut(&h).expect("manifest asset"));
    for _ in 0..5 {
        app.update();
    }
}

fn rock(world: &World) -> RockWetMaterial {
    let baked = world.resource::<BakedMaterials>();
    world.resource::<Assets<RockWetMaterial>>().get(&baked.env.rock_wet).expect("rock material").clone()
}

fn compiled() -> BakedManifest {
    BakedManifest::parse(quillshade::material::COMPILED_MANIFEST).unwrap()
}

fn param(m: &BakedManifest, material: &str, key: &str) -> ParamValue {
    m.materials[material].params[key]
}

/// Design §3.3 step 4: a parameter value edited in the editor reaches the material at its handle.
#[test]
fn hot_reload_updates_param() {
    let mut app = loaded_app();
    assert_eq!(param(&compiled(), "env/rock_wet", "inp_wetness"), ParamValue::Float(0.3));
    assert_eq!(rock(app.world()).extension.params.inp_wetness, 0.3);
    edit_manifest(&mut app, |m| {
        let rock = m.materials.get_mut("env/rock_wet").unwrap();
        rock.params.insert("inp_wetness".into(), ParamValue::Float(0.9));
    });
    let rock = rock(app.world());
    assert_eq!(rock.extension.params.inp_wetness, 0.9);
    assert_eq!(rock.extension.params.inp_speed, 1.0, "other values stay");
}

/// Design §3.3 step 4: a sampler change is set on the loaded image and the materials using it are touched.
#[test]
fn hot_reload_sampler_change() {
    let mut app = loaded_app();
    let image = app.world().resource::<BakedTextures>().textures.rock_albedo.clone();
    let before = app.world().resource::<Assets<Image>>().get(&image).expect("rock_albedo loaded").sampler.clone();
    assert_eq!(before, ImageSampler::Default);
    let sampler = SamplerDef {
        address_mode_u: AddressModeDef::Repeat,
        address_mode_v: AddressModeDef::MirrorRepeat,
        address_mode_w: AddressModeDef::Repeat,
        mag_filter: FilterModeDef::Linear,
        min_filter: FilterModeDef::Linear,
        mipmap_filter: FilterModeDef::Nearest,
        lod_min_clamp: 0.0,
        lod_max_clamp: 32.0,
        anisotropy_clamp: 1,
    };
    app.world_mut().resource_mut::<Seen>().rock_modified = 0;
    edit_manifest(&mut app, |m| {
        m.textures.get_mut("textures/rock_albedo").unwrap().settings.sampler = Some(sampler);
    });
    let world = app.world();
    let tex = world.resource::<BakedTextures>();
    assert_eq!(tex.textures.rock_albedo, image, "the same image, not a reload");
    let after = &world.resource::<Assets<Image>>().get(&image).unwrap().sampler;
    assert_eq!(*after, ImageSampler::Descriptor(sampler.descriptor()));
    assert!(world.resource::<Seen>().rock_modified > 0, "rock_wet (uses rock_albedo) was not touched");
}

/// Review Focus 3: a manifest out of sync with the compiled code (material removed, unknown material, parameter
/// added / removed / retyped, unknown binding, uncompiled texture) warns, never panics, and keeps compiled values;
/// the valid part of the same manifest still applies.
#[test]
fn hot_reload_tolerates_unknown_and_missing_keys() {
    let mut app = loaded_app();
    let compiled = compiled();
    edit_manifest(&mut app, |m| {
        m.materials.remove("env/rock_wet");
        let mut unknown = MaterialValues::default();
        unknown.params.insert("inp_x".into(), ParamValue::Float(1.0));
        unknown.textures.insert("inp_t".into(), "textures/atlas".into());
        m.materials.insert("env/unknown".into(), unknown);
        let flag = m.materials.get_mut("env/flag").unwrap();
        flag.params.remove("inp_strength");
        flag.params.insert("inp_new".into(), ParamValue::Float(5.0));
        flag.params.insert("inp_color".into(), ParamValue::Float(0.5));
        let coords = m.materials.get_mut("fx/coords").unwrap();
        coords.textures.insert("ft_new".into(), "textures/atlas".into());
        m.textures
            .insert("textures/unused".into(), TextureDef { path: "textures/sprite.png".into(), settings: default() });
        m.textures.remove("textures/water_normal");
        // Valid edit in the same manifest.
        let hologram = m.materials.get_mut("fx/hologram").unwrap();
        hologram.params.insert("inp_scan_speed".into(), ParamValue::Float(2.0));
    });
    let world = app.world();
    let baked = world.resource::<BakedMaterials>().clone();
    let tex = world.resource::<BakedTextures>().clone();
    let rock = rock(world);
    assert_eq!(rock.extension.params.inp_wetness, 0.3);
    assert_eq!(rock.extension.params.inp_speed, 1.0);
    assert_eq!(rock.extension.inp_albedo, tex.textures.rock_albedo);
    let flag = world.resource::<Assets<FlagMaterial>>().get(&baked.env.flag).unwrap();
    assert_eq!(param(&compiled, "env/flag", "inp_strength"), ParamValue::Float(0.2));
    assert_eq!(flag.extension.params.inp_strength, 0.2, "removed parameter keeps the compiled value");
    assert_eq!(flag.extension.params.inp_color, Vec4::new(0.8, 0.1, 0.1, 1.0), "retyped parameter keeps it too");
    let coords = world.resource::<Assets<CoordsMaterial>>().get(&baked.fx.coords).unwrap();
    assert_eq!(coords.inp_atlas, tex.textures.atlas);
    let water = world.resource::<Assets<WaterMaterial>>().get(&baked.env.water).unwrap();
    assert_eq!(water.extension.inp_normals, tex.textures.water_normal, "texture missing in the manifest stays");
    let hologram = world.resource::<Assets<HologramMaterial>>().get(&baked.fx.hologram).unwrap();
    assert_eq!(hologram.params.inp_scan_speed, 2.0, "the valid part applies");
}

/// Decision 0030: handles by asset id, for game data that names materials and textures.
#[test]
fn handles_by_path() {
    let app = loaded_app();
    let world = app.world();
    let baked = world.resource::<BakedMaterials>();
    let tex = world.resource::<BakedTextures>();
    assert_eq!(baked.get::<RockWetMaterial>("env/rock_wet"), Some(baked.env.rock_wet.clone()));
    assert_eq!(baked.get::<HologramMaterial>("fx/hologram"), Some(baked.fx.hologram.clone()));
    assert_eq!(baked.get::<HologramMaterial>("env/rock_wet"), None, "another material type");
    assert_eq!(baked.get::<RockWetMaterial>("env/nope"), None);
    assert_eq!(baked.by_path.len(), compiled().materials.len());
    assert_eq!(tex.get("textures/rock_albedo"), Some(&tex.textures.rock_albedo));
    assert_eq!(tex.get("@internal/default"), Some(&tex.internal.default));
    assert_eq!(tex.get("textures/nope"), None);
    assert_eq!(tex.by_path.len(), compiled().textures.len());
}

/// Decision 0029: another fixed texture node wired in (another `ft_*` name at the same binding, same layout) is
/// matched by position and hot-reloads; with another layout the compiled texture stays (the shader does not compile
/// until a rebuild anyway).
#[test]
fn hot_reload_maps_bindings_by_position() {
    let mut app = loaded_app();
    let layers = |app: &App| {
        let baked = app.world().resource::<BakedMaterials>().clone();
        let m = app.world().resource::<Assets<TexLayersMaterial>>().get(&baked.env.tex_layers).unwrap();
        m.extension.ft_common__tex_layer_n02.clone()
    };
    let tex = app.world().resource::<BakedTextures>().clone();
    assert_eq!(compiled().materials["env/tex_layers"].bindings, ["ft_common__tex_layer_n02"]);
    assert_eq!(layers(&app), tex.textures.rock_albedo);
    let rewire = |layout: String| {
        move |m: &mut BakedManifest| {
            let v = m.materials.get_mut("env/tex_layers").unwrap();
            v.textures = [("ft_n09".to_string(), "textures/atlas".to_string())].into();
            v.bindings = vec!["ft_n09".into()];
            v.layout = layout;
        }
    };
    edit_manifest(&mut app, rewire("SG_LAYOUT_00000000".into()));
    assert_eq!(layers(&app), tex.textures.rock_albedo, "another layout: the compiled texture stays");
    edit_manifest(&mut app, rewire(compiled().materials["env/tex_layers"].layout.clone()));
    assert_eq!(layers(&app), tex.textures.atlas);
}

/// Design §3.3 step 4: a Texture asset re-pointed to another file is loaded and replaces the image in
/// `BakedTextures` and in the materials; a texture unknown to the compiled code but referenced by a material is
/// loaded by path and used.
#[test]
fn hot_reload_repoints_texture() {
    let mut app = loaded_app();
    edit_manifest(&mut app, |m| {
        m.textures.get_mut("textures/atlas").unwrap().path = "textures/sprite.png".into();
        m.textures.insert(
            "textures/extra".into(),
            TextureDef { path: "textures/rock_normal.png".into(), settings: default() },
        );
        let water = m.materials.get_mut("env/water").unwrap();
        water.textures.insert("inp_normals".into(), "textures/extra".into());
    });
    let world = app.world();
    let server = world.resource::<AssetServer>().clone();
    let baked = world.resource::<BakedMaterials>().clone();
    let tex = world.resource::<BakedTextures>().clone();
    let path = |h: &Handle<Image>| server.get_path(h.id()).map(|p| p.to_string());
    assert_eq!(path(&tex.textures.atlas).as_deref(), Some("textures/sprite.png"));
    let coords = world.resource::<Assets<CoordsMaterial>>().get(&baked.fx.coords).unwrap();
    assert_eq!(coords.inp_atlas, tex.textures.atlas);
    let water = world.resource::<Assets<WaterMaterial>>().get(&baked.env.water).unwrap();
    let normals = water.extension.inp_normals.clone();
    assert_eq!(path(&normals).as_deref(), Some("textures/rock_normal.png"));
    assert_eq!(tex.get("textures/atlas"), Some(&tex.textures.atlas), "`by_path` follows a re-pointed texture");
    assert_eq!(tex.get("textures/extra"), Some(&normals), "a texture without a field is in `by_path`");
    let loaded = run_until(&mut app, 600, |w| {
        let server = w.resource::<AssetServer>();
        matches!(server.load_state(&normals), LoadState::Loaded)
            && matches!(server.load_state(&tex.textures.atlas), LoadState::Loaded)
    });
    assert!(loaded, "re-pointed textures did not load");
    assert_eq!(app.world().resource::<Seen>().ready, 1, "no second Ready after a hot reload");
    edit_manifest(&mut app, |m| {
        m.textures.remove("textures/extra");
        let water = m.materials.get_mut("env/water").unwrap();
        water.textures.insert("inp_normals".into(), "textures/water_normal".into());
    });
    assert_eq!(app.world().resource::<BakedTextures>().get("textures/extra"), None, "dropped from `by_path`");
}

/// Design §3.3 step 4: a Texture asset re-pointed to a file another asset already loaded (Bevy returns that image and
/// ignores the new load settings) still gets its own sampler on the image.
#[test]
fn hot_reload_repoint_to_loaded_file_applies_sampler() {
    let mut app = loaded_app();
    let sprite = app.world().resource::<BakedTextures>().textures.sprite.clone();
    let before = app.world().resource::<Assets<Image>>().get(&sprite).expect("sprite loaded").sampler.clone();
    assert_eq!(before, ImageSampler::Default);
    let sampler = SamplerDef {
        address_mode_u: AddressModeDef::MirrorRepeat,
        address_mode_v: AddressModeDef::Repeat,
        address_mode_w: AddressModeDef::Repeat,
        mag_filter: FilterModeDef::Nearest,
        min_filter: FilterModeDef::Nearest,
        mipmap_filter: FilterModeDef::Nearest,
        lod_min_clamp: 0.0,
        lod_max_clamp: 32.0,
        anisotropy_clamp: 1,
    };
    edit_manifest(&mut app, |m| {
        let atlas = m.textures.get_mut("textures/atlas").unwrap();
        atlas.path = "textures/sprite.png".into();
        atlas.settings.sampler = Some(sampler);
    });
    let want = ImageSampler::Descriptor(sampler.descriptor());
    let applied = run_until(&mut app, 600, |w| {
        let atlas = w.resource::<BakedTextures>().textures.atlas.clone();
        w.resource::<Assets<Image>>().get(&atlas).is_some_and(|i| i.sampler == want)
    });
    assert!(applied, "the re-pointed texture did not get its sampler");
    let world = app.world();
    let tex = world.resource::<BakedTextures>();
    assert_eq!(tex.textures.atlas.id(), sprite.id(), "the already loaded image is reused");
    let baked = world.resource::<BakedMaterials>();
    let coords = world.resource::<Assets<CoordsMaterial>>().get(&baked.fx.coords).unwrap();
    assert_eq!(coords.inp_atlas, tex.textures.atlas);
}

/// Review Focus 3: a `baked.ron` that does not parse (or has another version) fails to load with a warning and
/// leaves the materials as they are — at startup (compiled values) and after a valid reload (its values).
#[test]
fn hot_reload_garbage_ron_keeps_state() {
    let mut app = app(None);
    let text = app.world().resource::<ManifestText>().clone();
    text.set("this is not RON (");
    run(&mut app);
    assert!(run_until(&mut app, 600, |w| w.resource::<Seen>().manifest_failures == 1), "garbage did not fail");
    assert_eq!(rock(app.world()).extension.params.inp_wetness, 0.3, "compiled values");

    const WET_09: &str = "\"inp_wetness\": Float(0.9)";
    let edited = quillshade::material::COMPILED_MANIFEST.replace("\"inp_wetness\": Float(0.3)", WET_09);
    assert_ne!(edited, quillshade::material::COMPILED_MANIFEST);
    text.set(&edited);
    app.world().resource::<AssetServer>().reload(MANIFEST_PATH);
    let applied = run_until(&mut app, 600, |w| rock(w).extension.params.inp_wetness == 0.9);
    assert!(applied, "a valid reload after a failed one applies");

    for (n, bad) in [
        (2, "BakedManifest(version: 1, materials: {"),
        (3, &*edited.replace("version: 1", "version: 2").replace(WET_09, "\"inp_wetness\": Float(0.5)")),
    ] {
        text.set(bad);
        app.world().resource::<AssetServer>().reload(MANIFEST_PATH);
        assert!(run_until(&mut app, 600, |w| w.resource::<Seen>().manifest_failures == n), "load {n} did not fail");
        for _ in 0..5 {
            app.update();
        }
        assert_eq!(rock(app.world()).extension.params.inp_wetness, 0.9, "state kept after failure {n}");
    }
}
