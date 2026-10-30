//! The Light backend (design §9.2) on temporary projects: environment files, cube-map Texture assets through the
//! texture kit's rules, the L-prefixed diagnostics, the generated module and the reference rewrite of a renamed or
//! deleted texture.

use std::path::{Path, PathBuf};

use light_domain::backend::{
    Environment, LIGHT, LightAssets, LightBackend, environment_ref, load_environment, rewrite_texture_refs,
    save_environment, texture_ref, validate,
};
use project_core::regen::{DomainBackend, run};
use project_core::{Diagnostic, Level, Project};
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

/// Light Texture asset `path` on the game file `image`.
fn texture(p: &Project, path: &str, t: &TextureAsset) {
    texture_kit::save(&file(p, texture_ref(path).source_rel()), t).unwrap();
}

/// Environment `path` showing the cube map `cubemap`.
fn environment(p: &Project, path: &str, cubemap: Option<&str>) {
    let env = Environment { cubemap: cubemap.map(str::to_string), ..Environment::default() };
    save_environment(&file(p, environment_ref(path).source_rel()), &env).unwrap();
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
    let env = Environment {
        format_version: 1,
        cubemap: Some("textures/sky".into()),
        skybox_brightness: 500.0,
        env_intensity: 2.5,
    };
    let path = file(&p, environment_ref("outdoor/day").source_rel());
    assert!(path.ends_with("quillshade/light/outdoor/day.environment.ron"), "{}", path.display());
    save_environment(&path, &env).unwrap();
    let text = read(&path);
    assert!(text.starts_with("Environment(\n  format_version: 1,\n"), "{text}");
    assert!(text.contains("\n  cubemap: Some(\"textures/sky\"),\n"), "{text}");
    assert!(text.ends_with(")\n") && !text.contains('\r'), "{text}");
    assert_eq!(load_environment(&path).unwrap(), env);
    // Missing fields take their defaults; a newer format is refused.
    let defaults = Environment::from_ron("Environment(format_version: 1)").unwrap();
    assert_eq!(defaults, Environment::default());
    assert_eq!((defaults.cubemap, defaults.skybox_brightness, defaults.env_intensity), (None, 1000.0, 1.0));
    assert!(Environment::from_ron("Environment(format_version: 9)").unwrap_err().contains("newer"));
    let assets = LightAssets::load(&p);
    assert_eq!(assets.environments.get("outdoor/day"), Some(&env));
    assert!(assets.textures.is_empty() && assets.load_errors.is_empty());
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
    // The module is the only Light output.
    let light_files: Vec<_> = project_core::files::walk_files(&p.module_dir(LIGHT));
    assert_eq!(light_files, [p.module_dir(LIGHT).join("mod.rs")]);
    assert!(!p.output_dir(LIGHT).exists());
    // A second run changes nothing; an empty project lists no environment.
    assert!(run(&p, &[&backend], &[], false, false).unwrap().changes.is_empty());
    let (_e, empty) = project();
    run(&empty, &[&backend], &[], false, false).unwrap();
    let module = read(&empty.module_dir(LIGHT).join("mod.rs"));
    assert!(module.contains("pub const ENVIRONMENTS: &[&str] = &[];"), "{module}");
}

#[test]
fn rename_and_delete_texture_rewrite_refs() {
    let (_d, p) = project();
    sky(&p);
    environment(&p, "a", Some("textures/sky"));
    environment(&p, "sub/b", Some("textures/sky"));
    environment(&p, "c", Some("textures/other"));
    environment(&p, "d", None);
    let cubemap = |path: &str| load_environment(&file(&p, environment_ref(path).source_rel())).unwrap().cubemap;
    let renamed = rewrite_texture_refs(&p, "textures/sky", Some("textures/sky2")).unwrap();
    assert_eq!(renamed, ["a", "sub/b"]);
    assert_eq!(cubemap("a").as_deref(), Some("textures/sky2"));
    assert_eq!(cubemap("sub/b").as_deref(), Some("textures/sky2"));
    assert_eq!(cubemap("c").as_deref(), Some("textures/other"));
    assert_eq!(cubemap("d"), None);
    let cleared = rewrite_texture_refs(&p, "textures/sky2", None).unwrap();
    assert_eq!(cleared, ["a", "sub/b"]);
    assert_eq!((cubemap("a"), cubemap("sub/b")), (None, None));
    assert_eq!(cubemap("c").as_deref(), Some("textures/other"));
    assert!(rewrite_texture_refs(&p, "textures/none", None).unwrap().is_empty());
}

/// The sample game's Light assets (`day` on the `sky` cube map) plan without an error and list `day`.
#[test]
fn sample_game_light_is_clean() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/sample_game");
    let p = Project::open(&dir).unwrap();
    let plan = LightBackend.plan(&p, false);
    let errors: Vec<_> = plan.diagnostics.iter().filter(|d| d.level == Level::Error).collect();
    assert!(errors.is_empty(), "{errors:?}");
    assert!(plan.diagnostics.is_empty(), "no warning either: {:?}", plan.diagnostics);
    let module = String::from_utf8(plan.output.files[0].bytes.clone()).unwrap();
    assert!(module.contains("pub const ENVIRONMENTS: &[&str] = &[\n    \"day\",\n];"), "{module}");
}
