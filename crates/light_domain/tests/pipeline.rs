//! The Light backend (design §9.2) on temporary projects: environment files, cube-map Texture assets through the
//! texture kit's rules, the L-prefixed diagnostics, the generated module, the cube maps cleared when their texture
//! is deleted and the references kept valid through a rename (ADR 0047 §13).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use curve_kit::Curve;
use light_domain::backend::{
    Environment, LIGHT, LightAssets, LightBackend, clear_texture_refs, curve_ref, environment_ref, load_curve,
    load_environment, save_curve, save_environment, texture_ref, texture_ref_names, to_pascal_case, validate,
};
use project_core::asset_index::IndexedRef;
use project_core::regen::{DomainBackend, run};
use project_core::{AssetIndex, AssetUuid, Diagnostic, DomainId, Level, Project};
use texture_kit::{SamplerSettings, TextureAsset};

/// A game crate whose workspace has `crates/quillshade`, with an empty `assets/`.
fn project() -> (tempfile::TempDir, Project) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Cargo.toml"), "[workspace]\nmembers = [\"crates/*\"]\n").unwrap();
    std::fs::create_dir(dir.path().join("assets")).unwrap();
    let p = Project::open(dir.path()).unwrap();
    (dir, p)
}

/// A fake KTX2 file: the 48-byte header with the given depth, layer and face counts (as in the texture kit's tests).
fn ktx2(depth: u32, layers: u32, faces: u32) -> Vec<u8> {
    let mut h = vec![0xAB, b'K', b'T', b'X', b' ', b'2', b'0', 0xBB, b'\r', b'\n', 0x1A, b'\n'];
    for v in [37, 1, 64, 64, depth, layers, faces, 1, 0] {
        h.extend(u32::to_le_bytes(v));
    }
    h
}

/// Writes the game file `assets/<path>`.
fn game_file(p: &Project, path: &str, bytes: &[u8]) {
    let file = p.assets_dir().join(path);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, bytes).unwrap();
}

fn file(p: &Project, rel: String) -> PathBuf {
    p.root.join(rel)
}

/// Light Texture asset `path` on the game file `image`, with an id (a fresh one unless `t` has one), as every
/// editor writer saves it.
fn texture(p: &Project, path: &str, t: &TextureAsset) {
    let mut t = t.clone();
    t.ensure_id();
    texture_kit::save(&file(p, texture_ref(path).source_rel()), &t).unwrap();
}

/// Environment `path` showing the cube map `cubemap`.
fn environment(p: &Project, path: &str, cubemap: Option<&str>) {
    let mut env = Environment { cubemap: cubemap.map(str::to_string), ..Environment::default() };
    save_environment(&file(p, environment_ref(path).source_rel()), &mut env, &AssetIndex::default()).unwrap();
}

/// A valid cube map: `textures/sky` on `assets/textures/sky.ktx2` (6 faces).
fn sky(p: &Project) {
    game_file(p, "textures/sky.ktx2", &ktx2(0, 0, 6));
    texture(p, "textures/sky", &TextureAsset::new("textures/sky.ktx2"));
}

fn diagnostics(p: &Project) -> Vec<Diagnostic> {
    validate(p, &LightAssets::load(p))
}

/// The diagnostics with `code`.
fn with_code<'a>(all: &'a [Diagnostic], code: &str) -> Vec<&'a Diagnostic> {
    all.iter().filter(|d| d.code == code).collect()
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn environment_round_trips() {
    let (_d, p) = project();
    let mut env = Environment {
        cubemap: Some("textures/sky".into()),
        skybox_brightness: 500.0,
        env_intensity: 2.5,
        ..Environment::default()
    };
    let path = file(&p, environment_ref("outdoor/day").source_rel());
    assert!(path.ends_with("quillshade/light/outdoor/day.environment.ron"), "{}", path.display());
    save_environment(&path, &mut env, &AssetIndex::default()).unwrap();
    let text = read(&path);
    assert!(text.starts_with(&format!("Environment(\n  format_version: 2,\n  id: \"{}\",\n", env.id)), "{text}");
    assert!(text.contains("\n  cubemap: Some(\"textures/sky\"),\n"), "{text}");
    assert!(text.ends_with(")\n") && !text.contains('\r'), "{text}");
    assert_eq!(load_environment(&path).unwrap(), env);
    // Missing fields take their defaults; a newer format is refused.
    let defaults = Environment::from_ron("Environment(format_version: 2)").unwrap();
    assert_eq!(defaults, Environment::default());
    assert_eq!((defaults.cubemap, defaults.skybox_brightness, defaults.env_intensity), (None, 2500.0, 2500.0));
    assert_eq!(defaults.lights.len(), 3);
    assert_eq!((defaults.lights[0].name.as_str(), defaults.lights[0].illuminance), ("Sun", 15_000.0));
    assert_eq!((defaults.lights[1].name.as_str(), defaults.lights[1].illuminance), ("Sky", 3_000.0));
    assert_eq!((defaults.lights[2].name.as_str(), defaults.lights[2].illuminance), ("Back", 2_500.0));
    assert!(Environment::from_ron("Environment(format_version: 9)").unwrap_err().contains("newer"));
    let assets = LightAssets::load(&p);
    assert_eq!(assets.environments.get("outdoor/day"), Some(&env));
    assert!(assets.textures.is_empty() && assets.curves.is_empty() && assets.load_errors.is_empty());
}

#[test]
fn missing_cubemap_is_le001() {
    let (_d, p) = project();
    sky(&p);
    environment(&p, "ok", Some("textures/sky"));
    environment(&p, "bad", Some("textures/nope"));
    let all = diagnostics(&p);
    let le001 = with_code(&all, "LE001");
    assert_eq!(le001.len(), 1, "{all:?}");
    let d = le001[0];
    assert_eq!(d.level, Level::Error);
    assert_eq!(d.domain, Some(LIGHT));
    assert_eq!(d.asset.as_deref(), Some("quillshade/light/bad.environment.ron"));
    assert_eq!(d.location.as_deref(), Some("cubemap"));
    assert!(d.message.contains("textures/nope"), "{}", d.message);
    assert_eq!(all.len(), 1, "the valid environment and texture have none: {all:?}");
}

#[test]
fn no_cubemap_is_lw101() {
    let (_d, p) = project();
    environment(&p, "empty", None);
    let all = diagnostics(&p);
    assert_eq!(all.len(), 1, "{all:?}");
    let d = &all[0];
    assert_eq!((d.code.as_str(), d.level, d.domain), ("LW101", Level::Warning, Some(LIGHT)));
    assert_eq!(d.asset.as_deref(), Some("quillshade/light/empty.environment.ron"));
}

#[test]
fn plain_2d_ktx2_is_le014() {
    let (_d, p) = project();
    game_file(&p, "textures/flat.ktx2", &ktx2(0, 0, 1));
    texture(&p, "textures/flat", &TextureAsset::new("textures/flat.ktx2"));
    environment(&p, "env", Some("textures/flat"));
    let all = diagnostics(&p);
    let le014 = with_code(&all, "LE014");
    assert_eq!(le014.len(), 1, "{all:?}");
    assert_eq!(le014[0].asset.as_deref(), Some("quillshade/light/textures/flat.texture.ron"));
    assert_eq!((le014[0].level, le014[0].domain), (Level::Error, Some(LIGHT)));
    assert!(le014[0].message.contains("not a cube map"), "{}", le014[0].message);
    assert!(with_code(&all, "LE001").is_empty(), "the texture exists: {all:?}");
}

#[test]
fn non_power_of_two_cube_map_is_lw104() {
    let (_d, p) = project();
    let mut bytes = ktx2(0, 0, 6);
    bytes[20..24].copy_from_slice(&48u32.to_le_bytes());
    bytes[24..28].copy_from_slice(&48u32.to_le_bytes());
    game_file(&p, "textures/odd.ktx2", &bytes);
    texture(&p, "textures/odd", &TextureAsset::new("textures/odd.ktx2"));
    sky(&p);
    let all = diagnostics(&p);
    let lw104 = with_code(&all, "LW104");
    assert_eq!(lw104.len(), 1, "only the 48x48 cube map, not the 64x64 one: {all:?}");
    assert_eq!(lw104[0].asset.as_deref(), Some("quillshade/light/textures/odd.texture.ron"));
    assert_eq!((lw104[0].level, lw104[0].domain), (Level::Warning, Some(LIGHT)));
    assert!(lw104[0].message.contains("48x48"), "{}", lw104[0].message);
}

/// `env_rotation` was merged into `skybox_rotation` (the IBL turns with the skybox): older files still load.
#[test]
fn environment_with_legacy_env_rotation_loads() {
    let (_d, p) = project();
    let path = file(&p, environment_ref("old").source_rel());
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "Environment(format_version: 2, skybox_rotation: 90.0, env_rotation: 180.0)").unwrap();
    let env = load_environment(&path).unwrap();
    assert_eq!(env.skybox_rotation, 90.0);
}

#[test]
fn png_is_le012() {
    let (_d, p) = project();
    game_file(&p, "textures/sky.png", b"\x89PNG\r\n\x1a\n");
    texture(&p, "textures/sky", &TextureAsset::new("textures/sky.png"));
    let all = diagnostics(&p);
    let le012 = with_code(&all, "LE012");
    assert_eq!(le012.len(), 1, "{all:?}");
    assert_eq!(le012[0].asset.as_deref(), Some("quillshade/light/textures/sky.texture.ron"));
    assert!(le012[0].message.contains("ktx2"), "{}", le012[0].message);
    assert_eq!(all.len(), 1, "{all:?}");
}

/// The other texture-kit rules and the load error, each under its Light code.
#[test]
fn texture_rules_map_to_light_codes() {
    let (_d, p) = project();
    sky(&p);
    // One game file, two Texture assets: both are LE011.
    texture(&p, "textures/sky_copy", &TextureAsset::new("textures/sky.ktx2"));
    // Anisotropy with Nearest filters: LE013.
    let mut aniso = TextureAsset::new("textures/aniso.ktx2");
    aniso.settings.sampler = Some(SamplerSettings { anisotropy_clamp: 8, ..SamplerSettings::default() });
    game_file(&p, "textures/aniso.ktx2", &ktx2(0, 0, 6));
    texture(&p, "textures/aniso", &aniso);
    // No game file: LW109.
    texture(&p, "textures/gone", &TextureAsset::new("textures/gone.ktx2"));
    // Not RON: LE009; an environment showing a texture that does not load gets no LE001.
    let broken = file(&p, environment_ref("broken").source_rel());
    std::fs::write(&broken, "Environment(format_version: ").unwrap();
    std::fs::write(file(&p, texture_ref("textures/broken").source_rel()), "TextureAsset(").unwrap();
    environment(&p, "on_broken", Some("textures/broken"));
    let all = diagnostics(&p);
    assert!(all.iter().all(|d| d.domain == Some(LIGHT)), "{all:?}");
    let assets = |code: &str| -> Vec<String> {
        with_code(&all, code).iter().map(|d| d.asset.clone().unwrap_or_default()).collect()
    };
    assert_eq!(
        assets("LE011"),
        ["quillshade/light/textures/sky.texture.ron", "quillshade/light/textures/sky_copy.texture.ron"],
        "{all:?}"
    );
    assert_eq!(assets("LE013"), ["quillshade/light/textures/aniso.texture.ron"], "{all:?}");
    assert_eq!(assets("LW109"), ["quillshade/light/textures/gone.texture.ron"], "{all:?}");
    assert_eq!(
        assets("LE009"),
        ["quillshade/light/broken.environment.ron", "quillshade/light/textures/broken.texture.ron"],
        "{all:?}"
    );
    assert_eq!(with_code(&all, "LW109")[0].level, Level::Warning);
    assert_eq!(all.len(), 6, "{all:?}");
}

#[test]
fn codegen_lists_environments() {
    let (_d, p) = project();
    sky(&p);
    environment(&p, "outdoor/day", Some("textures/sky"));
    environment(&p, "outdoor/night", Some("textures/missing"));
    let backend = LightBackend;
    assert_eq!(backend.id(), LIGHT);
    let report = run(&p, &[&backend], &[], false, false).unwrap();
    assert!(report.has_errors(), "LE001 of the night environment");
    let module = read(&p.module_dir(LIGHT).join("mod.rs"));
    assert!(module.starts_with("// AUTOGENERATED by shader-editor v1"), "{module}");
    assert!(module.contains("use bevy::prelude::*;"), "{module}");
    assert!(module.contains("pub struct QuillshadeLightInternalPlugin;"), "{module}");
    assert!(module.contains("impl Plugin for QuillshadeLightInternalPlugin"), "{module}");
    assert!(module.contains("pub const ENVIRONMENTS: &[&str] = &[\n    \"outdoor/day\",\n];"), "{module}");
    assert!(!module.contains("outdoor/night"), "{module}");
    let lib = read(&p.crate_dir().join("src/lib.rs"));
    assert!(lib.contains("pub mod light;"), "{lib}");
    assert!(lib.contains("app.add_plugins(light::QuillshadeLightInternalPlugin);"), "{lib}");
    // The module and baked manifest are the Light outputs.
    let light_files: Vec<_> = project_core::files::walk_files(&p.module_dir(LIGHT));
    assert_eq!(light_files, [p.module_dir(LIGHT).join("mod.rs")]);
    let out_files: Vec<_> = project_core::files::walk_files(&p.output_dir(LIGHT));
    assert_eq!(out_files, [p.output_dir(LIGHT).join("autogen/baked.ron")]);
    let manifest = read(&p.output_dir(LIGHT).join("autogen/baked.ron"));
    assert!(manifest.starts_with("// AUTOGENERATED by shader-editor v1"), "{manifest}");
    assert!(manifest.contains("\"outdoor/day\""), "{manifest}");
    assert!(!manifest.contains("\"outdoor/night\""), "{manifest}");
    // A second run changes nothing; an empty project lists no environment.
    assert!(run(&p, &[&backend], &[], false, false).unwrap().changes.is_empty());
    let (_e, empty) = project();
    run(&empty, &[&backend], &[], false, false).unwrap();
    let module = read(&empty.module_dir(LIGHT).join("mod.rs"));
    assert!(module.contains("pub const ENVIRONMENTS: &[&str] = &[];"), "{module}");
}

/// Deleting a texture clears the cube maps naming it — by its UUID, or by its path in a hand-written file — and no
/// other.
#[test]
fn delete_texture_clears_refs_by_uuid() {
    let (_d, p) = project();
    sky(&p);
    let index = AssetIndex::build(vec![LightBackend.index(&p)]);
    let uuid = index.id_of(&texture_ref("textures/sky")).unwrap().to_string();
    environment(&p, "a", Some(&uuid));
    environment(&p, "sub/b", Some("textures/sky"));
    environment(&p, "c", Some("textures/other"));
    environment(&p, "d", None);
    let cubemap = |path: &str| load_environment(&file(&p, environment_ref(path).source_rel())).unwrap().cubemap;
    let names = texture_ref_names("textures/sky", &index);
    assert_eq!(names, ["textures/sky".to_string(), uuid]);
    std::fs::remove_file(file(&p, texture_ref("textures/sky").source_rel())).unwrap();
    let cleared = clear_texture_refs(&p, &names).unwrap();
    assert_eq!(cleared, ["a", "sub/b"]);
    assert_eq!((cubemap("a"), cubemap("sub/b")), (None, None));
    assert_eq!(cubemap("c").as_deref(), Some("textures/other"));
    assert_eq!(cubemap("d"), None);
    assert!(clear_texture_refs(&p, &names).unwrap().is_empty());
}

/// Renaming or moving a Light texture outside the editor (`std::fs::rename`) keeps the environments valid: no
/// source changes, the project generates without errors, and the generated module names the new path.
#[test]
fn external_rename_regenerates_cleanly() {
    let (_d, p) = project();
    uuid_form(&p);
    let day = file(&p, environment_ref("day").source_rel());
    let before = read(&day);
    std::fs::create_dir_all(file(&p, texture_ref("moved/sky").source_rel()).parent().unwrap()).unwrap();
    std::fs::rename(
        file(&p, texture_ref("textures/sky").source_rel()),
        file(&p, texture_ref("moved/sky").source_rel()),
    )
    .unwrap();
    let report = run(&p, &[&LightBackend], &[], false, false).unwrap();
    assert!(!report.has_errors(), "{:?}", report.diagnostics);
    let index = AssetIndex::build(vec![LightBackend.index(&p)]);
    assert!(index.diagnostics(&p).is_empty(), "{:?}", index.diagnostics(&p));
    assert_eq!(read(&day), before, "the environment is not rewritten");
    let manifest = read(&p.root.join("assets/quillshade/light/autogen/baked.ron"));
    assert!(manifest.contains("cubemap: Some(\"moved/sky\")"), "{manifest}");
}

/// ADR 0047 §11, techdoc §10 Phase 1 Done-when 3, Light half: a copied environment has its original's UUID — AE001
/// at the copy (the later path) only, and the project still generates both. (A copied Texture asset is LE011 as well:
/// a game file has one Texture asset.)
#[test]
fn copied_file_is_reported_as_duplicate() {
    let (_d, p) = project();
    uuid_form(&p);
    let (day, copy) = (environment_ref("day"), environment_ref("day_copy"));
    std::fs::copy(file(&p, day.source_rel()), file(&p, copy.source_rel())).unwrap();
    let index = AssetIndex::build(vec![LightBackend.index(&p)]);
    let diags = index.diagnostics(&p);
    let at: Vec<_> = diags.iter().map(|d| (d.code.as_str(), d.asset.clone())).collect();
    assert_eq!(at, [("AE001", Some(copy.source_rel()))], "{diags:?}");
    assert!(diags[0].message.contains(&format!("as `{}`", day.source_rel())), "{}", diags[0].message);
    let report = run(&p, &[&LightBackend], &[], false, false).unwrap();
    let errors: Vec<_> = report.diagnostics.iter().filter(|d| d.level == Level::Error).collect();
    assert!(errors.iter().all(|d| d.code == "AE001"), "AE001 blocks nothing: {errors:?}");
    let manifest = read(&p.root.join("assets/quillshade/light/autogen/baked.ron"));
    assert_eq!(manifest.matches("cubemap: Some(\"textures/sky\")").count(), 2, "{manifest}");
}

/// The sample game's Light assets (`day` and `night` on the `sky` cube map, plus the `smooth` curve) plan without
/// diagnostics and list `day`, `night`, and `smooth`.
#[test]
fn sample_game_light_is_clean() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/sample_game");
    let p = Project::open(&dir).unwrap();
    let plan = LightBackend.plan(&p, &project_core::AssetIndex::build(vec![LightBackend.index(&p)]), false);
    assert!(plan.diagnostics.is_empty(), "{:?}", plan.diagnostics);
    let module = String::from_utf8(plan.output.files[0].bytes.clone()).unwrap();
    assert!(module.contains("pub const ENVIRONMENTS: &[&str] = &[\n    \"day\",\n    \"night\",\n];"), "{module}");
    assert!(module.contains("pub const CURVES: &[&str] = &[\n    \"smooth\",\n];"), "{module}");
}

#[test]
fn pascal_case_collisions_and_reserved_names_are_le002() {
    assert_eq!(to_pascal_case("day"), "Day");
    assert_eq!(to_pascal_case("outdoor/clear_day"), "OutdoorClearDay");
    assert_eq!(to_pascal_case("ease_in_out"), "EaseInOut");

    let (_d, p) = project();
    sky(&p);
    environment(&p, "a_b/c", Some("textures/sky"));
    environment(&p, "a/b_c", Some("textures/sky"));
    environment(&p, "none", Some("textures/sky"));
    environment(&p, "transitioning", Some("textures/sky"));

    save_curve(&file(&p, curve_ref("linear").source_rel()), &mut Curve::ramp("linear")).unwrap();
    save_curve(&file(&p, curve_ref("ease_in/out").source_rel()), &mut Curve::ramp("out")).unwrap();
    save_curve(&file(&p, curve_ref("ease/in_out").source_rel()), &mut Curve::ramp("in_out")).unwrap();

    let all = diagnostics(&p);
    let le002: Vec<_> = with_code(&all, "LE002").into_iter().map(|d| d.asset.as_deref().unwrap_or("")).collect();
    assert_eq!(
        le002,
        [
            "quillshade/light/a/b_c.environment.ron",
            "quillshade/light/a_b/c.environment.ron",
            "quillshade/light/none.environment.ron",
            "quillshade/light/transitioning.environment.ron",
            "quillshade/light/ease/in_out.curve.ron",
            "quillshade/light/ease_in/out.curve.ron",
            "quillshade/light/linear.curve.ron",
        ],
        "{all:?}"
    );
}

#[test]
fn curve_validation_reports_le003_and_lw103() {
    let (_d, p) = project();
    let ok_path = file(&p, curve_ref("smooth").source_rel());
    save_curve(&ok_path, &mut Curve::new("smooth", vec![(0.0, 0.0), (0.5, 0.7), (1.0, 1.0)])).unwrap();
    assert_eq!(load_curve(&ok_path).unwrap().name, "smooth");

    let jump_path = file(&p, curve_ref("jump").source_rel());
    save_curve(&jump_path, &mut Curve::new("jump", vec![(0.0, 0.2), (1.0, 1.0)])).unwrap();

    let scaled_path = file(&p, curve_ref("scaled").source_rel());
    let mut scaled = Curve::ramp("scaled");
    scaled.min = 0.0;
    scaled.max = 2.0;
    save_curve(&scaled_path, &mut scaled).unwrap();

    let mut assets = LightAssets::load(&p);
    let mut bad = Curve::ramp("bad");
    bad.min = 5.0;
    bad.max = 1.0;
    assets.curves.insert("bad".into(), bad);

    let all = validate(&p, &assets);
    let le003: Vec<_> = with_code(&all, "LE003").into_iter().map(|d| d.asset.as_deref().unwrap_or("")).collect();
    assert_eq!(le003, ["quillshade/light/bad.curve.ron"], "{all:?}");
    let lw103: Vec<_> = with_code(&all, "LW103").into_iter().map(|d| d.asset.as_deref().unwrap_or("")).collect();
    assert_eq!(lw103, ["quillshade/light/jump.curve.ron", "quillshade/light/scaled.curve.ron"], "{all:?}");
}

#[test]
fn game_code_conflict_scanner_reports_lw102() {
    let (_d, p) = project();
    let src_dir = p.root.join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(
        src_dir.join("main.rs"),
        r##"//! Game entry point
// DirectionalLight in a line comment is ignored.
/* Block comment with Exposure
   /* nested DistanceFog */
   and Skybox */
fn setup() {
    let _s = "DirectionalLight and GlobalAmbientLight inside string \"Exposure\"";
    let _r = r#"Raw string with Tonemapping and ColorGrading"#;
    let _c = ClearColorConfig::None;
    let _light = DirectionalLight::default();
    let _fog = (DistanceFog::default(), Exposure::default());
    let _clear = ClearColor::default();
}
"##,
    )
    .unwrap();

    // Files inside `crates/quillshade`, `target/`, and `.hidden/` must be ignored.
    let qs_dir = p.crate_dir().join("src/light");
    std::fs::create_dir_all(&qs_dir).unwrap();
    std::fs::write(qs_dir.join("mod.rs"), "pub fn f() { let _ = DirectionalLight::default(); }\n").unwrap();
    let target_dir = p.root.join("target/debug");
    std::fs::create_dir_all(&target_dir).unwrap();
    std::fs::write(target_dir.join("gen.rs"), "pub fn f() { let _ = Skybox::default(); }\n").unwrap();
    let hidden_dir = p.root.join(".cache");
    std::fs::create_dir_all(&hidden_dir).unwrap();
    std::fs::write(hidden_dir.join("cached.rs"), "pub fn f() { let _ = AmbientLight::default(); }\n").unwrap();

    let all = diagnostics(&p);
    let lw102 = with_code(&all, "LW102");
    let found: Vec<(&str, &str, bool)> = lw102
        .iter()
        .map(|d| {
            (
                d.asset.as_deref().unwrap_or(""),
                d.location.as_deref().unwrap_or(""),
                d.level == Level::Warning && d.domain == Some(LIGHT),
            )
        })
        .collect();
    assert_eq!(
        found,
        [
            ("src/main.rs", "line 10", true),
            ("src/main.rs", "line 11", true),
            ("src/main.rs", "line 11", true),
            ("src/main.rs", "line 12", true),
        ],
        "{lw102:?}"
    );
    assert!(lw102[0].message.contains("`DirectionalLight`"));
    assert!(lw102[1].message.contains("`DistanceFog`"));
    assert!(lw102[2].message.contains("`Exposure`"));
    assert!(lw102[3].message.contains("`ClearColor`"));
}

/// The Light test project in the UUID form (`legacy` ids, the cube map stored as the texture's UUID with its D27
/// comment): `day` on `textures/sky`, `night` without a cube map, the `smooth` curve.
fn uuid_form(p: &Project) {
    let mut curve = Curve::ramp("smooth");
    curve.id = AssetUuid::legacy(&curve_ref("smooth").source_rel());
    save_curve(&file(p, curve_ref("smooth").source_rel()), &mut curve).unwrap();
    game_file(p, "textures/sky.ktx2", &ktx2(0, 0, 6));
    let sky = AssetUuid::legacy(&texture_ref("textures/sky").source_rel());
    let t = TextureAsset { id: sky, ..TextureAsset::new("textures/sky.ktx2") };
    texture(p, "textures/sky", &t);
    let index = AssetIndex::build(vec![LightBackend.index(p)]);
    for (path, cubemap) in [("day", Some(sky.to_string())), ("night", None)] {
        let mut env = Environment {
            id: AssetUuid::legacy(&environment_ref(path).source_rel()),
            cubemap,
            ..Environment::default()
        };
        save_environment(&file(p, environment_ref(path).source_rel()), &mut env, &index).unwrap();
    }
}

/// The project of [`uuid_form`] with the cube map stored as the texture's path (as written by hand; the resolve seam
/// passes a path through) and the ids minted on save.
fn path_form(p: &Project) {
    save_curve(&file(p, curve_ref("smooth").source_rel()), &mut Curve::ramp("smooth")).unwrap();
    sky(p);
    environment(p, "day", Some("textures/sky"));
    environment(p, "night", None);
}

/// An environment gets an id on its first save and keeps it, and its cube map UUID carries the D27 path comment;
/// the annotated text reads back to the same environment. Older formats ask for the Migration Tool.
#[test]
fn uuid_environment_round_trips_with_path_comment() {
    let (_d, p) = project();
    let (sky, gone) = (AssetUuid::new_v7(), AssetUuid::new_v7());
    let names: BTreeMap<AssetUuid, (DomainId, String)> = [(sky, (LIGHT, "textures/sky".to_string()))].into();
    let path = file(&p, environment_ref("day").source_rel());
    let mut env = Environment { cubemap: Some(sky.to_string()), ..Environment::default() };
    assert!(env.id.is_nil());
    save_environment(&path, &mut env, &names).unwrap();
    let id = env.id;
    assert!(id.is_v7(), "the first save mints the id");
    let text = read(&path);
    assert!(text.starts_with(&format!("Environment(\n  format_version: 2,\n  id: \"{id}\",\n")), "{text}");
    assert!(text.contains(&format!("\n  cubemap: Some(\"{sky}\"), // textures/sky\n")), "{text}");
    assert_eq!(load_environment(&path).unwrap(), env);
    save_environment(&path, &mut env, &names).unwrap();
    assert_eq!(env.id, id, "a later save keeps it");
    env.cubemap = Some(gone.to_string());
    save_environment(&path, &mut env, &names).unwrap();
    assert!(read(&path).contains(&format!("\n  cubemap: Some(\"{gone}\"), // (missing)\n")));
    assert!(Environment::from_ron("Environment(format_version: 3)").unwrap_err().contains("newer"));
    let older = Environment::from_ron("Environment(format_version: 1)").unwrap_err();
    assert!(older.contains("older than this editor") && older.contains("shader-editor migrate"), "{older}");
}

/// The Light index part lists the environments, curves and textures with their ids and the cube map references
/// located at `cubemap` (a cube map written by hand as a path is no edge); a file without an id is AE004, a cube map
/// UUID that names nothing AE002.
#[test]
fn index_part_lists_uuid_assets_and_cube_map_refs() {
    let (_d, p) = project();
    uuid_form(&p);
    environment(&p, "by_path", Some("textures/sky"));
    let part = LightBackend.index(&p);
    assert_eq!(part.domain, LIGHT);
    let listed: Vec<String> = part.assets.iter().map(|a| a.asset.source_rel()).collect();
    assert_eq!(
        listed,
        [
            "quillshade/light/by_path.environment.ron",
            "quillshade/light/day.environment.ron",
            "quillshade/light/night.environment.ron",
            "quillshade/light/smooth.curve.ron",
            "quillshade/light/textures/sky.texture.ron",
        ],
        "in path order"
    );
    let by_path = part.assets.iter().find(|a| a.asset == environment_ref("by_path")).unwrap();
    assert!(by_path.refs.is_empty(), "{:?}", by_path.refs);
    let sky = AssetUuid::legacy(&texture_ref("textures/sky").source_rel());
    let day = part.assets.iter().find(|a| a.asset == environment_ref("day")).unwrap();
    assert_eq!(day.id, Ok(AssetUuid::legacy(&environment_ref("day").source_rel())));
    assert_eq!(day.refs, [IndexedRef { to: sky, sub: None, location: Some("cubemap".into()) }]);
    let index = AssetIndex::build(vec![part]);
    assert!(index.diagnostics(&p).is_empty(), "{:?}", index.diagnostics(&p));
    assert_eq!(index.refs_to(sky).collect::<Vec<_>>(), [&environment_ref("day")]);

    // An environment without an id (hand-written) and one on a UUID that names nothing.
    let gone = AssetUuid::new_v7();
    let write = |path: &str, text: String| std::fs::write(file(&p, environment_ref(path).source_rel()), text).unwrap();
    write("no_id", "Environment(format_version: 2)".into());
    let dusk = AssetUuid::legacy(&environment_ref("dusk").source_rel());
    write("dusk", format!("Environment(format_version: 2, id: \"{dusk}\", cubemap: Some(\"{gone}\"))"));
    let index = AssetIndex::build(vec![LightBackend.index(&p)]);
    let codes: Vec<(String, Option<String>)> = index.diagnostics(&p).into_iter().map(|d| (d.code, d.asset)).collect();
    assert_eq!(
        codes,
        [
            ("AE002".to_string(), Some("quillshade/light/dusk.environment.ron".to_string())),
            ("AE004".to_string(), Some("quillshade/light/no_id.environment.ron".to_string())),
        ]
    );
}

/// The resolve seam: a cube map that is the UUID of a Light Texture asset becomes its path; a path, a dangling UUID
/// and the UUID of another kind (a curve) pass through unchanged.
#[test]
fn resolve_turns_texture_uuids_into_paths_and_passes_the_rest_through() {
    let (_d, p) = project();
    uuid_form(&p);
    let smooth = AssetUuid::legacy(&curve_ref("smooth").source_rel());
    let gone = AssetUuid::new_v7();
    let mut assets = LightAssets::load(&p);
    for (path, cubemap) in
        [("path", "textures/sky".to_string()), ("gone", gone.to_string()), ("curve", smooth.to_string())]
    {
        let env = Environment { cubemap: Some(cubemap), ..Environment::default() };
        assets.environments.insert(path.into(), env);
    }
    let index = AssetIndex::build(vec![assets.index_part()]);
    let mut resolved = assets.clone();
    resolved.resolve(&index);
    let cubemap = |path: &str| resolved.environments[path].cubemap.clone();
    assert_eq!(cubemap("day").as_deref(), Some("textures/sky"));
    assert_eq!(cubemap("night"), None);
    assert_eq!(cubemap("path").as_deref(), Some("textures/sky"));
    assert_eq!(cubemap("gone"), Some(gone.to_string()));
    assert_eq!(cubemap("curve"), Some(smooth.to_string()));
    let mut local = assets.clone();
    local.resolve_locally();
    assert_eq!(local.environments, resolved.environments);
    // Nothing else changes; resolving twice is resolving once.
    assert_eq!((resolved.textures.clone(), resolved.curves.clone()), (assets.textures, assets.curves));
    let once = resolved.environments.clone();
    resolved.resolve(&index);
    assert_eq!(resolved.environments, once);
    // An unresolved UUID is LE001, named by its UUID.
    let all = validate(&p, &resolved);
    let le001: Vec<&str> = with_code(&all, "LE001").iter().map(|d| d.message.as_str()).collect();
    assert_eq!(le001.len(), 2, "{all:?}");
    assert!(le001.iter().all(|m| m.contains("is not the UUID of a Light Texture asset")), "{le001:?}");
}

/// The UUID form generates exactly what the path form generates (module and manifest): the cube map resolves to the
/// path, and neither the source `format_version` nor the asset ids reach generated files (ADR 0047 §15).
#[test]
fn uuid_sources_generate_like_path_sources() {
    let generate = |write: fn(&Project)| {
        let (dir, p) = project();
        write(&p);
        let report = run(&p, &[&LightBackend], &[], false, false).unwrap();
        assert!(!report.has_errors(), "{:?}", report.diagnostics);
        let out: BTreeMap<String, Vec<u8>> =
            ["crates/quillshade/src/light/mod.rs", "assets/quillshade/light/autogen/baked.ron"]
                .into_iter()
                .map(|rel| (rel.to_string(), std::fs::read(p.root.join(rel)).unwrap()))
                .collect();
        (dir, out)
    };
    let (_a, paths) = generate(path_form);
    let (_b, uuids) = generate(uuid_form);
    assert_eq!(paths.keys().collect::<Vec<_>>(), uuids.keys().collect::<Vec<_>>());
    for (rel, bytes) in &paths {
        assert!(uuids[rel] == *bytes, "{rel} differs:\n{}", String::from_utf8_lossy(&uuids[rel]));
    }
    let manifest = String::from_utf8(uuids["assets/quillshade/light/autogen/baked.ron"].clone()).unwrap();
    assert!(manifest.contains("cubemap: Some(\"textures/sky\")") && !manifest.contains("id:"), "{manifest}");
    assert!(!manifest.contains("format_version: 2"), "{manifest}");
}
