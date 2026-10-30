//! The Light domain's editor (design §9.2, §14.4) next to a second texture domain, headless: the environment form's
//! edits, undo / redo and save, the Assets menu, the Game Assets menu of the active domain only, texture renames and
//! deletes reaching open environments, and Generate writing the Light module with the umbrella crate.
//!
//! The second texture domain is a test-only stand-in for Material (`material`, Material's texture policy: `ktx2` /
//! `png`, 2D, presets) built from `texture_kit` alone: `light_domain` never depends on `material_domain`, not even
//! for its tests.

use std::path::Path;
use std::sync::Arc;

use bevy::prelude::*;
use bevy::ui_widgets::ValueChange;
use editor_core::EditorPlugins;
use editor_core::action::{Action, Actions, TabAction, TreeId};
use editor_core::domain::{DomainInfo, DomainList, DomainRegistrar, DomainState, EditorDomain};
use editor_core::dynval::{Payload, PropKey, PropValue};
use editor_core::menu::MainMenu;
use editor_core::message::Level;
use editor_core::state::editor::Editor;
use editor_core::tab::ViewKindId;
use editor_core::ui::areas::{AreasPlugin, editor_topbar, right_sidebar};
use editor_core::ui::icons::IconId;
use editor_core::ui::menu::menu_entries;
use editor_core::ui::props::PropField;
use editor_core::ui::shell::{DomainWindowPlugin, domain_doc_views};
use editor_core::ui::tree::{GameAssetsTree, TreeItem, TreeRow, TreeSource, row_menu};
use light_domain::backend::{Environment, LIGHT, environment_ref, load_environment, save_environment, texture_ref};
use light_domain::{EnvironmentDoc, EnvironmentProp, LightDomain, LightState, LightTexture};
use project_core::regen::DomainBackend;
use project_core::{AssetRef, DomainId, Project};
use texture_kit::editor::{CreateTextureForFile, TextureAction, TextureDoc, TextureDomain, TextureEdit};
use texture_kit::{TextureAsset, TexturePolicy, TextureShape};

/// The stand-in for Material's texture domain.
struct Mat;

impl TextureDomain for Mat {
    const DOMAIN: DomainId = DomainId::new("material");
    const VIEW: ViewKindId = ViewKindId("material.texture");
    const IMAGE_VIEW: ViewKindId = ViewKindId("material.image");
    const POLICY: TexturePolicy =
        TexturePolicy { extensions: &["ktx2", "png"], shape: TextureShape::Plain2d, internal: None, presets: true };
}

/// A domain whose only assets are [`Mat`]'s textures.
struct MatDomain;

impl EditorDomain for MatDomain {
    fn id(&self) -> DomainId {
        Mat::DOMAIN
    }
    fn info(&self) -> DomainInfo {
        DomainInfo { title: "Material", icon: IconId::File.into(), summary: "" }
    }
    fn backend(&self) -> Option<Box<dyn DomainBackend>> {
        None
    }
    fn register(&self, r: &mut DomainRegistrar) {
        texture_kit::editor::register::<Mat>(r);
        r.state(|| Box::new(MatState));
    }
}

/// The stand-in's state: a save rewrites the umbrella crate files from every domain's contribution, as Material's
/// regeneration does (design §6.5), and nothing of Light's.
struct MatState;

impl DomainState for MatState {
    fn id(&self) -> DomainId {
        Mat::DOMAIN
    }
    fn after_save(&mut self, editor: &mut Editor, _tabs: &[usize]) {
        let project = editor.project.clone().unwrap();
        project_core::regen::write_umbrella(&project, editor.domain_list.crate_contributions()).unwrap();
    }
}

/// The stand-in Material first, then Light (as `src/main.rs` lists the real ones).
fn domains() -> Arc<DomainList> {
    Arc::new(DomainList::new(vec![Box::new(MatDomain), Box::new(LightDomain)]))
}

const SAMPLES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/sample_game/assets/textures");

/// An uncompressed RGBA8 KTX2 cube map of `side`-pixel faces, one mip; face `i` filled with byte `i * 40`.
fn cube_ktx2(side: u32) -> Vec<u8> {
    let face = (side * side * 4) as usize;
    let mut f = vec![0xAB, b'K', b'T', b'X', b' ', b'2', b'0', 0xBB, b'\r', b'\n', 0x1A, b'\n'];
    // vkFormat R8G8B8A8_UNORM, typeSize, width, height, depth, layers, faces, levels, supercompression.
    for v in [37, 1, side, side, 0, 0, 6, 1, 0] {
        f.extend(u32::to_le_bytes(v));
    }
    // An empty DFD (total size 4), no key/value or supercompression data, one level index entry.
    for v in [104, 4, 0, 0] {
        f.extend(u32::to_le_bytes(v));
    }
    f.extend([0u8; 16]);
    for v in [108, 6 * face, 6 * face] {
        f.extend(u64::to_le_bytes(v as u64));
    }
    f.extend(u32::to_le_bytes(4));
    for i in 0..6u8 {
        f.extend(std::iter::repeat_n(i * 40, face));
    }
    f
}

fn light_tex(path: &str) -> AssetRef {
    texture_ref(path)
}

fn mat_tex(path: &str) -> AssetRef {
    AssetRef { domain: Mat::DOMAIN, path: path.into(), suffix: "texture" }
}

/// A game crate with `assets/textures/{a.png, a.ktx2, sky.ktx2}` (`a.*` real images, `sky.ktx2` a cube map), the
/// stand-in Material's Texture asset `t` on `a.png`, Light's `sky` on `sky.ktx2`, the environments `day` (cube map
/// `sky`) and `night` (none), open in an editor of [`domains`].
fn project() -> (tempfile::TempDir, Editor) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Cargo.toml"), "[workspace]\nmembers = [\"crates/*\"]\n").unwrap();
    let textures = dir.path().join("assets/textures");
    std::fs::create_dir_all(&textures).unwrap();
    std::fs::copy(format!("{SAMPLES}/rock_albedo.png"), textures.join("a.png")).unwrap();
    std::fs::copy(format!("{SAMPLES}/rock_albedo.ktx2"), textures.join("a.ktx2")).unwrap();
    std::fs::write(textures.join("sky.ktx2"), cube_ktx2(4)).unwrap();
    let p = Project::open(dir.path()).unwrap();
    texture_kit::editor::save(&p, &mat_tex("t"), &TextureAsset::new("textures/a.png")).unwrap();
    texture_kit::editor::save(&p, &light_tex("sky"), &TextureAsset::new("textures/sky.ktx2")).unwrap();
    environment(&p, "day", Some("sky"));
    environment(&p, "night", None);
    let mut e = Editor::new(domains());
    e.open_project(dir.path()).unwrap();
    (dir, e)
}

fn environment(p: &Project, path: &str, cubemap: Option<&str>) {
    let env = Environment { cubemap: cubemap.map(str::to_string), ..Environment::default() };
    save_environment(&p.root.join(environment_ref(path).source_rel()), &env).unwrap();
}

fn env_on_disk(dir: &Path, path: &str) -> Environment {
    load_environment(&dir.join(environment_ref(path).source_rel())).unwrap()
}

fn active_env(e: &Editor) -> &EnvironmentDoc {
    e.active_doc::<EnvironmentDoc>().expect("an environment tab is active")
}

fn key(p: EnvironmentProp) -> PropKey {
    PropKey::new(p)
}

/// A number field's edit, as Feathers sends it at the end of a drag or a typed value.
fn set_number(e: &mut Editor, p: EnvironmentProp, value: f32) {
    e.apply(Action::Tab(TabAction::SetComponent { key: key(p), comp: 0, value, is_final: true }));
}

fn labels(entries: &[editor_core::menu::MenuEntry]) -> Vec<&str> {
    entries.iter().map(|m| m.label.as_str()).collect()
}

/// Design §9.2: with Light active, the Assets menu offers its own kinds' templates, then New folder and Generate.
#[test]
fn light_assets_menu() {
    let (_dir, mut e) = project();
    e.apply(Action::SwitchDomain(LIGHT));
    let entries = menu_entries(MainMenu::Assets, &e);
    assert_eq!(labels(&entries), ["New Environment...", "New Texture...", "New folder...", "", "Generate"]);
    // Both templates make a file that opens in its tab.
    for (i, (path, suffix)) in [("outdoor/dusk", "environment"), ("textures/stars", "texture")].into_iter().enumerate()
    {
        e.apply(entries[i].action.clone());
        e.apply(Action::Submit(path.into()));
        let r = AssetRef { domain: LIGHT, path: path.into(), suffix };
        assert!(e.project.as_ref().unwrap().root.join(r.source_rel()).is_file(), "{r}: {:?}", e.messages);
        assert_eq!(e.tab().and_then(|t| t.doc().asset()), Some(&r));
    }
    assert_eq!(active_env_or_texture_path(&e), Some(String::new()), "a new Texture asset has no image yet");
    let dusk = env_on_disk(e.project.as_ref().unwrap().root.as_path(), "outdoor/dusk");
    assert_eq!(dusk, Environment::default());
}

fn active_env_or_texture_path(e: &Editor) -> Option<String> {
    e.active_doc::<TextureDoc<LightTexture>>().map(|t| t.asset.path.clone())
}

/// Design §9.2: a form edit is one undo step; Undo / Redo; Save writes the file and regenerates the Light module.
#[test]
fn environment_edit_undo_save() {
    let (dir, mut e) = project();
    e.apply(Action::OpenAsset(environment_ref("night")));
    assert_eq!(e.active_domain, Some(LIGHT));
    set_number(&mut e, EnvironmentProp::SkyboxBrightness, 500.0);
    assert_eq!(active_env(&e).env.skybox_brightness, 500.0);
    assert!(e.is_dirty());
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(active_env(&e).env.skybox_brightness, 1000.0);
    assert!(!e.is_dirty());
    e.apply(Action::Tab(TabAction::Redo));
    assert_eq!(active_env(&e).env.skybox_brightness, 500.0);
    e.apply(Action::Save);
    assert!(!e.is_dirty());
    let text = std::fs::read_to_string(dir.path().join(environment_ref("night").source_rel())).unwrap();
    assert!(text.contains("skybox_brightness: 500.0"), "{text}");
    let module = std::fs::read_to_string(dir.path().join("crates/quillshade/src/light/mod.rs")).unwrap();
    assert!(module.contains("\"day\"") && module.contains("\"night\""), "{module}");
    // A drag is one step; a negative value is no brightness.
    for v in [10.0, 20.0, 30.0] {
        e.apply(Action::Tab(TabAction::SetComponent {
            key: key(EnvironmentProp::EnvIntensity),
            comp: 0,
            value: v,
            is_final: false,
        }));
    }
    e.apply(Action::Tab(TabAction::CommitInline));
    assert_eq!(active_env(&e).env.env_intensity, 30.0);
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(active_env(&e).env.env_intensity, 1.0);
    let layout = active_env(&e).layout;
    set_number(&mut e, EnvironmentProp::EnvIntensity, -2.0);
    assert_eq!(active_env(&e).env.env_intensity, 0.0);
    assert!(active_env(&e).layout > layout, "the form is rebuilt to show 0");
}

/// Design §9.2: the cube-map picker sets the texture by path; "None" (an empty value) clears it. The diagnostics
/// follow the open (unsaved) document: LW101 for no cube map, LE001 for an unknown one.
#[test]
fn cube_map_picker_and_live_diagnostics() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(environment_ref("day")));
    let codes = |e: &Editor| {
        let s = e.domains.get::<LightState>().unwrap();
        let day = environment_ref("day").source_rel();
        s.diagnostics
            .iter()
            .filter(|d| d.asset.as_deref() == Some(day.as_str()))
            .map(|d| d.code.clone())
            .collect::<Vec<_>>()
    };
    assert!(codes(&e).is_empty());
    let pick = |v: &str| {
        Action::Tab(TabAction::SetValue {
            key: key(EnvironmentProp::Cubemap),
            value: PropValue::Str(v.into()),
            merge: false,
        })
    };
    e.apply(pick(""));
    assert_eq!(active_env(&e).env.cubemap, None);
    assert_eq!(codes(&e), ["LW101"]);
    e.apply(pick("gone"));
    assert_eq!(codes(&e), ["LE001"]);
    e.apply(pick("sky"));
    assert_eq!(active_env(&e).env.cubemap.as_deref(), Some("sky"));
    assert!(codes(&e).is_empty());
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(active_env(&e).env.cubemap.as_deref(), Some("gone"));
}

/// Review Focus 4, design §14.4: with Light active, a `.ktx2` game file's menu has exactly Light's "Create Texture
/// Asset", and `.png` files (the other domain's) are not listed; with Material active, its own item.
#[test]
fn game_assets_menu_is_light_only() {
    let (dir, mut e) = project();
    let menu =
        |e: &Editor, f: &str| row_menu(e, &TreeRow { tree: TreeId::GameAssets, item: TreeItem::GameFile(f.into()) });
    let files = |e: &Editor| {
        GameAssetsTree
            .rows(e)
            .into_iter()
            .filter_map(|(_, i)| match i {
                TreeItem::GameFile(f) => Some(f),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    e.apply(Action::SwitchDomain(Mat::DOMAIN));
    assert!(files(&e).contains(&"textures/a.png".to_string()));
    let mat = menu(&e, "textures/a.ktx2");
    assert_eq!(labels(&mat), ["Create Texture Asset"]);
    assert!(matches!(&mat[0].action, Action::Domain(id, _) if *id == Mat::DOMAIN));

    e.apply(Action::SwitchDomain(LIGHT));
    assert_eq!(files(&e), ["textures/a.ktx2", "textures/sky.ktx2"]);
    assert!(menu(&e, "textures/a.png").is_empty());
    let items = menu(&e, "textures/a.ktx2");
    assert_eq!(labels(&items), ["Create Texture Asset"]);
    let create = Action::Domain(LIGHT, Payload::new(CreateTextureForFile("textures/a.ktx2".into())));
    assert_eq!(items[0].action, create);
    assert!(items[0].disabled.is_none());
    let taken = menu(&e, "textures/sky.ktx2");
    assert!(taken[0].disabled.as_deref().is_some_and(|t| t.contains("quillshade/light/sky.texture.ron")));
    e.apply(create);
    assert!(dir.path().join("quillshade/light/textures/a.texture.ron").is_file(), "{:?}", e.messages);
    assert!(!dir.path().join("quillshade/material/textures/a.texture.ron").exists());
    // A plain 2D image is no cube map: Light reports it.
    let s = e.domains.get::<LightState>().unwrap();
    assert!(s.diagnostics.iter().any(|d| d.code == "LE014"), "{:?}", s.diagnostics);
}

/// Design §9.2: renaming a Light texture rewrites the environments naming it, the open one included (it stays clean);
/// deleting it clears them with a warning.
#[test]
fn rename_light_texture_updates_open_environment() {
    let (dir, mut e) = project();
    environment(e.project.as_ref().unwrap(), "noon", Some("sky"));
    e.rescan();
    e.apply(Action::OpenAsset(environment_ref("day")));
    // An unsaved edit of another field survives the rename.
    set_number(&mut e, EnvironmentProp::EnvIntensity, 2.0);
    e.apply(Action::OpenAsset(environment_ref("noon")));
    e.rename(&light_tex("sky"), "skies/blue");
    assert!(dir.path().join("quillshade/light/skies/blue.texture.ron").is_file(), "{:?}", e.messages);
    assert_eq!(env_on_disk(dir.path(), "day").cubemap.as_deref(), Some("skies/blue"));
    let noon = active_env(&e);
    assert_eq!(noon.env.cubemap.as_deref(), Some("skies/blue"));
    assert!(!crate_is_dirty(&e, "noon"), "the clean tab follows the file");
    let day = e.tabs.iter().find_map(|t| t.get::<EnvironmentDoc>().filter(|d| d.aref.path == "day")).unwrap();
    assert_eq!(day.env.cubemap.as_deref(), Some("skies/blue"));
    assert_eq!(day.env.env_intensity, 2.0);
    assert!(crate_is_dirty(&e, "day"), "only the unsaved edit stays");
    let s = e.domains.get::<LightState>().unwrap();
    assert!(!s.diagnostics.iter().any(|d| d.code == "LE001"), "{:?}", s.diagnostics);
    // Undoing the unsaved edit keeps the renamed cube map: the undo state followed too.
    e.apply(Action::OpenAsset(environment_ref("day")));
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(active_env(&e).env.env_intensity, 1.0);
    assert_eq!(active_env(&e).env.cubemap.as_deref(), Some("skies/blue"));
    assert!(!crate_is_dirty(&e, "day"));

    e.apply(Action::OpenAsset(environment_ref("noon")));
    e.delete(&light_tex("skies/blue"));
    assert_eq!(env_on_disk(dir.path(), "noon").cubemap, None);
    assert_eq!(active_env(&e).env.cubemap, None);
    let warning = e.messages.iter().rev().find(|m| m.level == Level::Warning).expect("a warning");
    assert!(warning.text.contains("skies/blue") && warning.text.contains("noon"), "{}", warning.text);
}

fn crate_is_dirty(e: &Editor, path: &str) -> bool {
    use editor_core::doc_kind::TabDoc;
    e.tabs.iter().find_map(|t| t.get::<EnvironmentDoc>().filter(|d| d.aref.path == path)).unwrap().is_dirty()
}

/// Design §7.2, Review Focus 4: a Material and a Light Texture tab side by side; each edit lands in its own file,
/// and a Material edit queued while the Light tab is active is dropped.
#[test]
fn material_and_light_texture_tabs_side_by_side() {
    let (dir, mut e) = project();
    e.apply(Action::OpenAsset(mat_tex("t")));
    e.apply(Action::OpenAsset(light_tex("sky")));
    assert_eq!(e.active_domain, Some(LIGHT));
    let light = e.active_doc::<TextureDoc<LightTexture>>().unwrap();
    assert!(!light.easy, "Light has no presets");
    let mut linear = light.asset.settings.clone();
    linear.is_srgb = false;
    // Queued for Material's tab: dropped on Light's.
    e.apply(Action::from(TextureEdit::<Mat>::new(TextureAction::SetSettings(linear.clone()))));
    assert!(!e.is_dirty());
    e.apply(Action::from(TextureEdit::<LightTexture>::new(TextureAction::SetSettings(linear.clone()))));
    e.apply(Action::Save);
    e.apply(Action::OpenAsset(mat_tex("t")));
    assert_eq!(e.active_domain, Some(Mat::DOMAIN));
    let mut nearest = e.active_doc::<TextureDoc<Mat>>().unwrap().asset.settings.clone();
    nearest.expand_grayscale = !nearest.expand_grayscale;
    e.apply(Action::from(TextureEdit::<Mat>::new(TextureAction::SetSettings(nearest.clone()))));
    e.apply(Action::Save);
    let load = |r: AssetRef| texture_kit::load(&dir.path().join(r.source_rel())).unwrap();
    let (sky, t) = (load(light_tex("sky")), load(mat_tex("t")));
    assert_eq!((sky.settings, sky.path), (linear, "textures/sky.ktx2".to_string()));
    assert_eq!((t.settings, t.path), (nearest, "textures/a.png".to_string()));
}

/// Design §6.5, §9.2: Generate in the editor writes Light's module and the umbrella crate naming it.
#[test]
fn generate_writes_light_module_and_umbrella() {
    let (dir, mut e) = project();
    let lib = dir.path().join("crates/quillshade/src/lib.rs");
    let module = dir.path().join("crates/quillshade/src/light/mod.rs");
    std::fs::remove_file(&lib).unwrap();
    std::fs::remove_file(&module).unwrap();
    e.apply(Action::RegenAll);
    let lib = std::fs::read_to_string(lib).unwrap();
    assert!(lib.contains("pub mod light;"), "{lib}");
    let module = std::fs::read_to_string(module).unwrap();
    assert!(module.contains("QuillshadeLightInternalPlugin") && module.contains("\"day\""), "{module}");
    // Nothing changed: nothing written.
    let n = e.messages.len();
    e.apply(Action::RegenAll);
    assert!(e.messages[n..].iter().any(|m| m.text.contains("no changes")), "{:?}", &e.messages[n..]);
}

/// Design §6.5: opening a project that never generated Light writes its module, so saving another domain's tab, which
/// rewrites the umbrella's `src/lib.rs` naming `light`, leaves a crate with `light/mod.rs`.
#[test]
fn open_generates_light_so_other_saves_keep_the_crate_whole() {
    let (dir, mut e) = project();
    let lib = dir.path().join("crates/quillshade/src/lib.rs");
    let module = dir.path().join("crates/quillshade/src/light/mod.rs");
    assert!(module.is_file(), "generated on open: {:?}", e.messages);
    // Generated before: opening again writes nothing.
    std::fs::remove_file(&lib).unwrap();
    let mut again = Editor::new(domains());
    again.open_project(dir.path()).unwrap();
    assert!(!lib.exists(), "the module exists: Light does not generate");
    e.apply(Action::OpenAsset(mat_tex("t")));
    let mut settings = e.active_doc::<TextureDoc<Mat>>().unwrap().asset.settings.clone();
    settings.is_srgb = !settings.is_srgb;
    e.apply(Action::from(TextureEdit::<Mat>::new(TextureAction::SetSettings(settings))));
    e.apply(Action::Save);
    assert!(std::fs::read_to_string(&lib).unwrap().contains("pub mod light;"));
    assert!(module.is_file());
}

/// A headless window app over `editor`, built the way the window builds the domains' parts.
fn app(editor: Editor) -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default(), bevy::scene::ScenePlugin, bevy::input::InputPlugin));
    app.init_asset::<Font>().init_asset::<Image>();
    app.init_resource::<bevy::input_focus::InputFocus>().init_resource::<bevy::input_focus::InputFocusVisible>();
    app.init_resource::<bevy::feathers::controls::UnitsRegistry>();
    app.insert_resource(bevy::feathers::theme::UiTheme(bevy::feathers::dark_theme::create_dark_theme()));
    app.insert_resource(editor);
    app.add_plugins((EditorPlugins, AreasPlugin, DomainWindowPlugin));
    for view in domain_doc_views(app.world().resource::<DomainList>()) {
        app.world_mut().spawn_scene(view).unwrap();
    }
    app.world_mut().spawn_scene(editor_topbar()).unwrap();
    app.world_mut().spawn_scene(right_sidebar()).unwrap();
    app.finish();
    app.update();
    app
}

/// The property field editing `p`.
fn field(app: &mut App, p: EnvironmentProp) -> Option<(Entity, PropField)> {
    let w = app.world_mut();
    let mut q = w.query::<(Entity, &PropField)>();
    q.iter(w).find(|(_, f)| f.target == p).map(|(e, f)| (e, f.clone()))
}

/// Design §9.2: the form is core's property widgets: a select of "None" and the Light textures, two number fields;
/// a number field's change reaches the document.
#[test]
fn environment_form_uses_core_widgets() {
    let (_dir, editor) = project();
    let mut app = app(editor);
    app.world_mut().resource_mut::<Actions>().push(Action::OpenAsset(environment_ref("day")));
    for _ in 0..3 {
        app.update();
    }
    let (_, picker) = field(&mut app, EnvironmentProp::Cubemap).expect("the cube-map picker");
    assert_eq!(picker.options, ["None", "sky"]);
    assert_eq!(picker.values, ["", "sky"]);
    let (brightness, _) = field(&mut app, EnvironmentProp::SkyboxBrightness).expect("the brightness field");
    assert!(field(&mut app, EnvironmentProp::EnvIntensity).is_some());
    app.world_mut().trigger(ValueChange { source: brightness, value: 250.0f32, is_final: true });
    app.update();
    app.update();
    assert_eq!(active_env(app.world().resource::<Editor>()).env.skybox_brightness, 250.0);
}
