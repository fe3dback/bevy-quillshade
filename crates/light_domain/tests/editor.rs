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
use editor_core::tab::{TextDoc, ViewKindId};
use editor_core::ui::areas::{AreasPlugin, editor_topbar, right_sidebar};
use editor_core::ui::icons::IconId;
use editor_core::ui::menu::menu_entries;
use editor_core::ui::props::PropField;
use editor_core::ui::shell::{DomainWindowPlugin, domain_doc_views};
use editor_core::ui::tree::{GameAssetsTree, TreeItem, TreeRow, TreeSource, row_menu};
use light_domain::backend::{
    Environment, FogFalloffDef, LIGHT, TonemappingMode, curve_ref, environment_ref, load_environment, save_environment,
    texture_ref,
};
use light_domain::{
    BlendedSkybox, CgSection, EnvironmentAction, EnvironmentDoc, EnvironmentProp, INSPECT_GAME_TOOL, InspectGameDoc,
    LIGHT_PREVIEW_TOOL, LightCurve, LightCurveDoc, LightDomain, LightPreviewDoc, LightPreviewProp,
    LightPreviewSessions, LightState, LightTexture, interpolate_environments,
};
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

    fn disk_owners(editor: &Editor, file: &str) -> Vec<AssetRef> {
        let Some(project) = &editor.project else { return Vec::new() };
        editor
            .kind_files
            .get(&Self::DOMAIN)
            .into_iter()
            .flatten()
            .filter(|r| r.suffix == Self::SUFFIX && texture_kit::editor::load(project, r).is_ok_and(|t| t.path == file))
            .cloned()
            .collect()
    }
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
    fn after_save(editor: &mut Editor, _tabs: &[usize]) {
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
    assert_eq!(
        labels(&entries),
        ["New Environment...", "New Curve...", "New Texture...", "New folder...", "", "Generate"]
    );
    // All three templates make a file that opens in its tab.
    for (i, (path, suffix)) in
        [("outdoor/dusk", "environment"), ("outdoor/ease", "curve"), ("textures/stars", "texture")]
            .into_iter()
            .enumerate()
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
    assert_eq!(active_env(&e).env.skybox_brightness, 2500.0);
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
    assert_eq!(active_env(&e).env.env_intensity, 2500.0);
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
    assert_eq!(active_env(&e).env.env_intensity, 2500.0);
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
    app.add_plugins((EditorPlugins, AreasPlugin, DomainWindowPlugin, editor_core::ui::text_view::TextViewPlugin));
    for view in domain_doc_views(app.world().resource::<DomainList>()) {
        app.world_mut().spawn_scene(view).unwrap();
    }
    app.world_mut().spawn_scene(editor_core::ui::text_view::text_view()).unwrap();
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

/// Design §9.2: the form is core's property widgets: a select of "None" and the Light textures, number fields, and
/// `FeathersColorInput` pickers; changes reach the document and intermediate drag frames do not rebuild the sidebar.
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

    // Intermediate drag frames on an already-dirty document do not bump `editor.ui.props`.
    let props_before = app.world().resource::<Editor>().ui.props;
    app.world_mut().trigger(ValueChange { source: brightness, value: 300.0f32, is_final: false });
    app.update();
    assert_eq!(app.world().resource::<Editor>().ui.props, props_before);

    // `FeathersColorInput` updates color properties and ends its undo merge step on `is_final: true`.
    let (ambient_color, _) = field(&mut app, EnvironmentProp::AmbientColor).expect("the ambient color picker");
    app.world_mut().trigger(ValueChange {
        source: ambient_color,
        value: Color::linear_rgba(0.25, 0.5, 0.75, 1.0),
        is_final: true,
    });
    app.update();
    app.update();
    assert_eq!(active_env(app.world().resource::<Editor>()).env.ambient_color, [0.25, 0.5, 0.75, 1.0]);
}

/// Editing the extended EnvironmentProp fields (colors, directional lights, tonemapping, color grading, distance fog)
/// updates the document and supports undo / redo.
#[test]
fn extended_environment_fields_edit_and_undo() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(environment_ref("day")));
    assert_eq!(active_env(&e).env.lights.len(), 3);

    // Add a 4th directional light, edit its name and elevation, then remove the 2nd light.
    e.apply(EnvironmentAction::AddLight);
    assert_eq!(active_env(&e).env.lights.len(), 4);
    assert_eq!(active_env(&e).env.lights[3].name, "Light 4");
    e.apply(Action::Tab(TabAction::SetValue {
        key: key(EnvironmentProp::LightName(3)),
        value: PropValue::Str("Rim".into()),
        merge: false,
    }));
    assert_eq!(active_env(&e).env.lights[3].name, "Rim");
    set_number(&mut e, EnvironmentProp::LightElevation(3), 42.0);
    assert_eq!(active_env(&e).env.lights[3].elevation_deg, 42.0);
    e.apply(EnvironmentAction::RemoveLight(1));
    assert_eq!(active_env(&e).env.lights.len(), 3);
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(active_env(&e).env.lights.len(), 4);

    // Edit clear color component 0 (R)
    e.apply(Action::Tab(TabAction::SetComponent {
        key: key(EnvironmentProp::ClearColor),
        comp: 0,
        value: 0.25,
        is_final: true,
    }));
    assert_eq!(active_env(&e).env.clear_color[0], 0.25);

    // Select Tonemapping mode
    e.apply(Action::Tab(TabAction::SetValue {
        key: key(EnvironmentProp::Tonemapping),
        value: PropValue::Str("AgX".into()),
        merge: false,
    }));
    assert_eq!(active_env(&e).env.tonemapping, TonemappingMode::AgX);

    // Edit color grading shadows contrast
    set_number(&mut e, EnvironmentProp::CgSectionContrast(CgSection::Shadows), 1.35);
    assert_eq!(active_env(&e).env.color_grading.shadows.contrast, 1.35);

    // Switch fog falloff mode to Exponential and edit density
    e.apply(Action::Tab(TabAction::SetValue {
        key: key(EnvironmentProp::FogFalloffMode),
        value: PropValue::Str("Exponential".into()),
        merge: false,
    }));
    assert_eq!(active_env(&e).env.fog.falloff, FogFalloffDef::Exponential { density: 0.01 });
    set_number(&mut e, EnvironmentProp::FogDensity, 0.04);
    assert_eq!(active_env(&e).env.fog.falloff, FogFalloffDef::Exponential { density: 0.04 });

    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(active_env(&e).env.fog.falloff, FogFalloffDef::Exponential { density: 0.01 });
}

/// Editing an open LightCurve tab updates LW103 live in LightState::diagnostics.
#[test]
fn curve_tab_edits_update_live_lw103_diagnostics() {
    let (_dir, mut e) = project();
    let p = e.project.as_ref().unwrap().clone();
    curve_kit::editor::save(&p, &curve_ref("ease"), &curve_kit::Curve::ramp("ease")).unwrap();
    e.rescan();
    e.apply(Action::OpenAsset(curve_ref("ease")));
    assert!(e.active_doc::<LightCurveDoc>().is_some());

    let curve_codes = |e: &Editor| {
        let s = e.domains.get::<LightState>().unwrap();
        let rel = curve_ref("ease").source_rel();
        s.diagnostics
            .iter()
            .filter(|d| d.asset.as_deref() == Some(rel.as_str()))
            .map(|d| d.code.clone())
            .collect::<Vec<_>>()
    };
    assert!(curve_codes(&e).is_empty());

    // Change the curve's Max range from 1.0 to 2.0: LW103 appears immediately on the unsaved tab.
    let range_key = curve_kit::editor::curve_key::<LightCurve>(curve_kit::editor::CurveProp::Range);
    e.apply(Action::Tab(TabAction::SetComponent { key: range_key, comp: 1, value: 2.0, is_final: true }));
    assert_eq!(curve_codes(&e), ["LW103"]);

    // Undo restores Max = 1.0 and clears LW103.
    e.apply(Action::Tab(TabAction::Undo));
    assert!(curve_codes(&e).is_empty());
}

/// Focusing an LW102 diagnostic opens a read-only Rust TextDoc tab for the conflicting game source file with the
/// conflicting line highlighted.
#[test]
fn focus_lw102_diagnostic_opens_game_rust_file_in_text_tab() {
    use std::collections::BTreeSet;

    let (dir, mut e) = project();
    let src_dir = dir.path().join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(src_dir.join("main.rs"), "fn setup() {\n    let _ = DirectionalLight::default();\n}\n").unwrap();
    e.rescan();

    let diag = e
        .domains
        .get::<LightState>()
        .unwrap()
        .diagnostics
        .iter()
        .find(|d| d.code == "LW102")
        .cloned()
        .expect("LW102 diagnostic for src/main.rs");
    assert_eq!(diag.asset.as_deref(), Some("src/main.rs"));
    assert_eq!(diag.location.as_deref(), Some("line 2"));

    e.apply(Action::FocusDiagnostic(diag.clone()));
    let text_doc = e.active_doc::<TextDoc>().expect("opened a read-only TextDoc tab");
    assert_eq!(text_doc.rel, "src/main.rs");
    assert_eq!(text_doc.lang, "rust");
    assert_eq!(text_doc.highlights, BTreeSet::from([2]));
    assert!(text_doc.text.as_deref().unwrap_or_default().contains("DirectionalLight"));
    assert_eq!(e.status_info().map(|s| s.counts), Some(vec!["3 lines".to_string(), "highlight=[2]".to_string()]));

    // Focusing the same diagnostic again activates the existing tab rather than duplicating it.
    e.apply(Action::OpenAsset(environment_ref("day")));
    let tab_count = e.tabs.len();
    e.apply(Action::FocusDiagnostic(diag));
    assert_eq!(e.tabs.len(), tab_count);
    assert_eq!(e.active_doc::<TextDoc>().map(|d| d.rel.as_str()), Some("src/main.rs"));
}

/// The `Inspect Game` tool in `Tools -> Inspect Game...` scans game `.rs` files, refreshes on rescan, and focuses
/// conflicts into read-only Rust `TextDoc` tabs with line highlighting.
#[test]
fn inspect_game_tool_in_tools_menu_scans_and_focuses_conflicts() {
    use std::collections::BTreeSet;

    let (dir, mut e) = project();
    e.apply(Action::SwitchDomain(LIGHT));
    let entries = menu_entries(MainMenu::Tools, &e);
    assert_eq!(labels(&entries), ["Inspect Game...", "Light Transition Preview...", "HDR Converter..."]);
    assert_eq!(entries[0].action, Action::OpenTool { domain: LIGHT, id: INSPECT_GAME_TOOL, arg: None });
    assert_eq!(entries[1].action, Action::OpenTool { domain: LIGHT, id: LIGHT_PREVIEW_TOOL, arg: None });

    e.apply(entries[0].action.clone());
    assert!(e.active_doc::<InspectGameDoc>().is_some_and(|d| d.conflicts.is_empty()));

    let src_dir = dir.path().join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(
        src_dir.join("main.rs"),
        "fn setup() {\n    let _ = DirectionalLight::default();\n    let _ = DistanceFog::default();\n}\n",
    )
    .unwrap();
    e.apply(Action::Rescan);

    let conflicts = e.active_doc::<InspectGameDoc>().expect("Inspect Game tab active").conflicts.clone();
    assert_eq!(conflicts.len(), 2);
    assert!(conflicts.iter().all(|d| d.code == "LW102" && d.asset.as_deref() == Some("src/main.rs")));
    assert_eq!(conflicts[0].location.as_deref(), Some("line 2"));
    assert_eq!(conflicts[1].location.as_deref(), Some("line 3"));

    e.apply(Action::FocusDiagnostic(conflicts[0].clone()));
    let text_doc = e.active_doc::<TextDoc>().expect("opened src/main.rs in a read-only TextDoc tab");
    assert_eq!(text_doc.rel, "src/main.rs");
    assert_eq!(text_doc.lang, "rust");
    assert_eq!(text_doc.highlights, BTreeSet::from([2]));

    // Focusing the second conflict in the same file updates `highlights` on the already-open TextDoc tab.
    e.apply(Action::FocusDiagnostic(conflicts[1].clone()));
    let text_doc = e.active_doc::<TextDoc>().expect("reused src/main.rs TextDoc tab");
    assert_eq!(text_doc.highlights, BTreeSet::from([3]));
}

/// The read-only code view (`text_view`) renders highlighted lines (`highlight=[1,3,5-6]`) with `ACCENT2` background
/// and unhighlighted lines with `Color::NONE`.
#[test]
fn text_view_renders_highlighted_lines_with_accent2_background() {
    use editor_core::tab::Tab;
    use editor_core::theme::ACCENT2;
    use editor_core::ui::text_view::TextViewLine;

    let (dir, mut editor) = project();
    let p = editor.project.as_ref().unwrap().clone();
    let rs_path = dir.path().join("src/main.rs");
    std::fs::create_dir_all(rs_path.parent().unwrap()).unwrap();
    std::fs::write(&rs_path, "line1();\nline2();\nline3();\nline4();\nline5();\nline6();\n").unwrap();

    let source = AssetRef { domain: LIGHT, path: "src/main.rs".into(), suffix: "rs" };
    let doc = TextDoc::open(&p, source, "rust", rs_path).with_highlight_spec("highlight=[1,3,5-6]");
    editor.push_tab(Tab::new(LIGHT, doc));

    let mut app = app(editor);
    app.update();

    let mut runs: Vec<(TextViewLine, Color)> = app
        .world_mut()
        .query::<(&TextViewLine, &BackgroundColor)>()
        .iter(app.world())
        .map(|(line, bg)| (*line, bg.0))
        .collect();
    runs.sort_by_key(|(line, _)| line.start);

    assert_eq!(
        runs,
        vec![
            (TextViewLine { start: 1, end: 1, highlighted: true }, ACCENT2),
            (TextViewLine { start: 2, end: 2, highlighted: false }, Color::NONE),
            (TextViewLine { start: 3, end: 3, highlighted: true }, ACCENT2),
            (TextViewLine { start: 4, end: 4, highlighted: false }, Color::NONE),
            (TextViewLine { start: 5, end: 6, highlighted: true }, ACCENT2),
        ]
    );
}

/// Opening an Environment tab starts a 3D RTT preview studio (`start_preview`), syncs edited environment properties to
/// the preview camera and lights, and cleans up all preview entities and resources on tab close (RAII drop) or
/// explicit `.clean()`.
#[test]
fn environment_preview_starts_on_open_and_cleans_on_close_or_explicit_clean() {
    use bevy::camera::Exposure;
    use editor_core::preview::{PreviewEntity, PreviewTracker};

    let (_dir, editor) = project();
    let mut app = app(editor);

    app.world_mut().resource_mut::<Actions>().push(Action::OpenAsset(environment_ref("day")));
    app.update();

    let id = active_env(app.world().resource::<Editor>()).preview.id();
    assert!(id > 0);
    assert!(app.world().resource::<PreviewTracker>().is_spawned(id));
    let cam_entity = app.world().resource::<LightPreviewSessions>().sessions.get(&id).expect("session spawned").camera;
    let entity_count = app.world_mut().query::<&PreviewEntity>().iter(app.world()).filter(|e| e.0 == id).count();
    assert!(entity_count >= 10, "camera, 3 directional lights, and 7 demo meshes spawned: got {entity_count}");

    // Edit Ev100 and verify the preview camera's Exposure updates in the world.
    app.world_mut().resource_mut::<Actions>().push(Action::Tab(TabAction::SetComponent {
        key: key(EnvironmentProp::Ev100),
        comp: 0,
        value: 15.5,
        is_final: true,
    }));
    app.update();
    let exposure = app.world().entity(cam_entity).get::<Exposure>().expect("Exposure on preview camera");
    assert_eq!(exposure.ev100, 15.5);

    // Close the tab (RAII drop of PreviewHandle) -> sync_previews cleans up session and despawns all PreviewEntity(id).
    app.world_mut().resource_mut::<Actions>().push(Action::DiscardTab(0));
    app.update();
    assert!(!app.world().resource::<PreviewTracker>().is_spawned(id));
    assert!(!app.world().resource::<LightPreviewSessions>().sessions.contains_key(&id));
    assert_eq!(app.world_mut().query::<&PreviewEntity>().iter(app.world()).filter(|e| e.0 == id).count(), 0);

    // Reopen `night`, then explicitly call `preview.clean()` while the tab stays open.
    app.world_mut().resource_mut::<Actions>().push(Action::OpenAsset(environment_ref("night")));
    app.update();
    let id2 = active_env(app.world().resource::<Editor>()).preview.id();
    assert!(id2 > id);
    assert!(app.world().resource::<PreviewTracker>().is_spawned(id2));
    active_env(app.world().resource::<Editor>()).preview.clean();
    app.update();
    assert!(!app.world().resource::<PreviewTracker>().is_spawned(id2));
    assert!(!app.world().resource::<LightPreviewSessions>().sessions.contains_key(&id2));
    assert_eq!(app.world_mut().query::<&PreviewEntity>().iter(app.world()).filter(|e| e.0 == id2).count(), 0);
}

/// Opening the `Light Preview` tool (`Tools -> Light Preview`) spawns a 3D RTT preview studio, interpolates between
/// `Env A` and `Env B` using `Linear` or a custom `.curve.ron`, and cleans up all preview entities on tab close.
#[test]
fn light_preview_tool_interpolates_transition_and_cleans_raii_handle_on_close() {
    use bevy::camera::Exposure;
    use bevy::core_pipeline::tonemapping::Tonemapping;
    use editor_core::preview::{PreviewEntity, PreviewTracker};

    let (dir, mut editor) = project();
    let p = editor.project.as_ref().unwrap().clone();

    // Configure `night` with ev100 = 9.0, skybox_brightness = 500.0, tonemapping = AgX.
    let night_env = Environment {
        cubemap: None,
        skybox_brightness: 500.0,
        ev100: 9.0,
        tonemapping: TonemappingMode::AgX,
        ..Environment::default()
    };
    save_environment(&dir.path().join(environment_ref("night").source_rel()), &night_env).unwrap();

    // Save a custom curve `smooth` that maps t = 0.5 -> s = 0.25.
    let mut smooth = curve_kit::Curve::ramp("smooth");
    smooth.points = vec![(0.0, 0.0), (0.5, 0.25), (1.0, 1.0)];
    curve_kit::editor::save(&p, &curve_ref("smooth"), &smooth).unwrap();
    editor.rescan();

    let mut app = app(editor);
    app.world_mut().resource_mut::<Actions>().push(Action::OpenTool {
        domain: LIGHT,
        id: LIGHT_PREVIEW_TOOL,
        arg: None,
    });
    app.update();

    let (id, env_a, env_b, curve, t) = {
        let ed = app.world().resource::<Editor>();
        let doc = ed.active_doc::<LightPreviewDoc>().expect("Light Preview tool tab active");
        (doc.preview.id(), doc.env_a.clone(), doc.env_b.clone(), doc.curve.clone(), doc.t)
    };
    assert!(id > 0);
    assert_eq!((env_a.as_str(), env_b.as_str(), curve.as_str(), t), ("day", "night", "", 0.0));
    assert!(app.world().resource::<PreviewTracker>().is_spawned(id));
    let cam_entity = app.world().resource::<LightPreviewSessions>().sessions.get(&id).expect("session spawned").camera;

    // At t = 0.0, preview camera has day's ev100 = 13.0 and TonyMcMapface tonemapping.
    assert_eq!(app.world().entity(cam_entity).get::<Exposure>().unwrap().ev100, 13.0);
    assert_eq!(*app.world().entity(cam_entity).get::<Tonemapping>().unwrap(), Tonemapping::TonyMcMapface);
    // Run one more frame so `style_select_widgets` applies `LineBreak::NoWrap` and `column_gap` to `FeathersSelect`.
    app.update();
    {
        use bevy::feathers::controls::{FeathersMenuButton, FeathersSelect};
        let w = app.world_mut();
        let selects: Vec<Entity> = w.query_filtered::<Entity, With<FeathersSelect>>().iter(w).collect();
        assert_eq!(selects.len(), 3, "Env A, Env B, and Curve selects in LightPreviewBar");
        let btns: Vec<&Node> = w.query_filtered::<&Node, With<FeathersMenuButton>>().iter(w).collect();
        assert!(btns.iter().all(|n| n.column_gap == Val::Px(6.0) && n.overflow == Overflow::clip()));
    }

    // Scrub `t` to 0.5 with Linear curve -> ev100 = lerp(13.0, 9.0, 0.5) = 11.0.
    app.world_mut().resource_mut::<Actions>().push(Action::Tab(TabAction::SetComponent {
        key: PropKey::new(LightPreviewProp::T),
        comp: 0,
        value: 0.5,
        is_final: true,
    }));
    app.update();
    assert_eq!(app.world().entity(cam_entity).get::<Exposure>().unwrap().ev100, 11.0);

    // Switch curve to `"smooth"` (where smooth(0.5) == 0.25) -> ev100 = lerp(13.0, 9.0, 0.25) = 12.0.
    app.world_mut().resource_mut::<Actions>().push(Action::Tab(TabAction::SetValue {
        key: PropKey::new(LightPreviewProp::Curve),
        value: PropValue::Str("smooth".into()),
        merge: false,
    }));
    app.update();
    let ev_smooth = app.world().entity(cam_entity).get::<Exposure>().unwrap().ev100;
    assert!((ev_smooth - 12.0).abs() < 1e-4, "expected ~12.0, got {ev_smooth}");

    // Scrub `t` to 1.0 -> binary fields switch to `night` (Tonemapping::AgX) and ev100 = 9.0.
    app.world_mut().resource_mut::<Actions>().push(Action::Tab(TabAction::SetComponent {
        key: PropKey::new(LightPreviewProp::T),
        comp: 0,
        value: 1.0,
        is_final: true,
    }));
    app.update();
    assert_eq!(app.world().entity(cam_entity).get::<Exposure>().unwrap().ev100, 9.0);
    assert_eq!(*app.world().entity(cam_entity).get::<Tonemapping>().unwrap(), Tonemapping::AgX);

    // Close the Light Preview tab -> RAII handle drops and despawns all preview entities.
    app.world_mut().resource_mut::<Actions>().push(Action::DiscardTab(0));
    app.update();
    assert!(!app.world().resource::<PreviewTracker>().is_spawned(id));
    assert!(!app.world().resource::<LightPreviewSessions>().sessions.contains_key(&id));
    assert_eq!(app.world_mut().query::<&PreviewEntity>().iter(app.world()).filter(|e| e.0 == id).count(), 0);

    // Verify direct `interpolate_environments` cubemap cross-fade and fog cross-mode promotion.
    let mut a = Environment { cubemap: Some("sky_a".into()), skybox_brightness: 1000.0, ..Environment::default() };
    let mut b = Environment { cubemap: Some("sky_b".into()), skybox_brightness: 2000.0, ..Environment::default() };
    a.fog.falloff = FogFalloffDef::Exponential { density: 0.02 };
    b.fog.falloff = FogFalloffDef::Atmospheric { extinction: [0.06, 0.06, 0.06], inscattering: [0.04, 0.04, 0.04] };
    let mid_quarter = interpolate_environments(&a, &b, 0.25, 0.25);
    assert_eq!(mid_quarter.cubemap.as_deref(), Some("sky_a"));
    assert!((mid_quarter.skybox_brightness - 500.0).abs() < 1e-4);
    let mid_three_quarters = interpolate_environments(&a, &b, 0.75, 0.75);
    assert_eq!(mid_three_quarters.cubemap.as_deref(), Some("sky_b"));
    assert!((mid_three_quarters.skybox_brightness - 1000.0).abs() < 1e-4);
    assert!(matches!(mid_three_quarters.fog.falloff, FogFalloffDef::Atmospheric { .. }));
}

/// Transitioning between `envA (cubemap = None, clear_color = light)` and `envB (cubemap = Some("sky"), clear_color =
/// dark)` in `interpolate_environments` and `LightPreviewDoc` preserves `envA.clear_color` while `raw_t < 1.0`,
/// switches to `envB.clear_color` at `raw_t = 1.0`, and configures `BlendedSkybox` on the preview camera to blend
/// directly between `envA.clear_color` and `envB`'s cubemap texture.
#[test]
fn light_preview_sky_color_to_cubemap_blending() {
    let (dir, mut editor) = project();

    let env_a = Environment {
        cubemap: None,
        clear_color: [0.8, 0.85, 0.9, 1.0],
        skybox_brightness: 500.0,
        sky_top_color: [0.9, 0.95, 1.0, 1.0],
        sky_horizon_color: [0.8, 0.85, 0.9, 1.0],
        ground_color: [0.4, 0.38, 0.35, 1.0],
        ..Environment::default()
    };
    let env_b = Environment {
        cubemap: Some("sky".into()),
        clear_color: [0.02, 0.02, 0.05, 1.0],
        skybox_brightness: 2500.0,
        sky_top_color: [0.05, 0.08, 0.15, 1.0],
        sky_horizon_color: [0.03, 0.04, 0.08, 1.0],
        ground_color: [0.01, 0.01, 0.02, 1.0],
        ..Environment::default()
    };
    save_environment(&dir.path().join(environment_ref("night").source_rel()), &env_a).unwrap();
    save_environment(&dir.path().join(environment_ref("day").source_rel()), &env_b).unwrap();
    editor.rescan();

    // Direct `interpolate_environments` check:
    let mid = interpolate_environments(&env_a, &env_b, 0.3, 0.3);
    assert_eq!(mid.clear_color, env_a.clear_color);
    let done = interpolate_environments(&env_a, &env_b, 1.0, 1.0);
    assert_eq!(done.clear_color, env_b.clear_color);

    // Reverse (`Some -> None`):
    let rev_mid = interpolate_environments(&env_b, &env_a, 0.3, 0.3);
    assert_eq!(rev_mid.clear_color, env_a.clear_color);
    let rev_done = interpolate_environments(&env_b, &env_a, 1.0, 1.0);
    assert_eq!(rev_done.clear_color, env_a.clear_color);

    // Verify `BlendedSkybox` on the preview camera in `LightPreviewDoc` (`night` -> `day` at t = 0.3):
    let mut app = app(editor);
    app.world_mut().resource_mut::<Actions>().push(Action::OpenTool {
        domain: LIGHT,
        id: LIGHT_PREVIEW_TOOL,
        arg: None,
    });
    app.update();

    app.world_mut().resource_mut::<Actions>().push(Action::Tab(TabAction::SetValue {
        key: PropKey::new(LightPreviewProp::EnvA),
        value: PropValue::Str("night".into()),
        merge: false,
    }));
    app.world_mut().resource_mut::<Actions>().push(Action::Tab(TabAction::SetValue {
        key: PropKey::new(LightPreviewProp::EnvB),
        value: PropValue::Str("day".into()),
        merge: false,
    }));
    app.world_mut().resource_mut::<Actions>().push(Action::Tab(TabAction::SetComponent {
        key: PropKey::new(LightPreviewProp::T),
        comp: 0,
        value: 0.3,
        is_final: true,
    }));
    app.update();

    let id = app.world().resource::<Editor>().active_doc::<LightPreviewDoc>().unwrap().preview.id();
    let cam_entity = app.world().resource::<LightPreviewSessions>().sessions.get(&id).unwrap().camera;
    let ibl_source = |app: &App| {
        app.world()
            .entity(cam_entity)
            .get::<bevy::light::GeneratedEnvironmentMapLight>()
            .expect("GeneratedEnvironmentMapLight on preview camera")
            .environment_map
            .id()
    };
    let ibl_id_before = ibl_source(&app);
    let skybox = app.world().entity(cam_entity).get::<BlendedSkybox>().expect("BlendedSkybox on preview camera");
    assert!(skybox.cubemap_a.is_none());
    assert!(skybox.cubemap_b.is_some());
    assert_eq!(skybox.color_a, Vec4::from_array(env_a.clear_color));
    assert_eq!(skybox.color_b, Vec4::from_array(env_b.clear_color));
    assert!((skybox.blend - 0.3).abs() < 1e-5);
    assert!((skybox.alpha - 1.0).abs() < 1e-5);

    // `night` has no cube map, so below the blend midpoint the IBL is filtered from the procedural sky colors; scrubbing
    // to 0.45 updates that image in place on the exact same `AssetId<Image>` (zero 1-frame flicker).
    let scrub = |app: &mut App, t: f32| {
        app.world_mut().resource_mut::<Actions>().push(Action::Tab(TabAction::SetComponent {
            key: PropKey::new(LightPreviewProp::T),
            comp: 0,
            value: t,
            is_final: true,
        }));
        app.update();
    };
    scrub(&mut app, 0.45);
    assert_eq!(ibl_id_before, ibl_source(&app), "procedural IBL updates in-place without reallocating AssetId<Image>");

    // Past the midpoint `day`'s cube map dominates the skybox blend and becomes the IBL source.
    scrub(&mut app, 0.6);
    let cubemap_b = app.world().entity(cam_entity).get::<BlendedSkybox>().unwrap().cubemap_b.clone().unwrap();
    assert_eq!(ibl_source(&app), cubemap_b.id(), "the dominant cube map is filtered for the IBL");
}

/// Opening a cube-map Texture asset (`sky.texture.ron`) in Light (and Material) uses the shared `CubeMapPreview` with
/// all 6 faces decoded (never just 1 face), supporting `Sphere` (`3D`: grey sphere inside skybox) and `Strip2d`
/// (`2D`: all 6 faces stacked vertically).
#[test]
fn cubemap_texture_preview_supports_sphere_skybox_and_6_face_strip() {
    use bevy::core_pipeline::Skybox;
    use bevy::light::EnvironmentMapLight;
    use bevy::pbr::StandardMaterial;
    use texture_kit::editor::{
        CubeMapPreview, CubeMapPreviewSessions, CubeMapViewMode, GameImageCache, PreviewCubeMode, PreviewPic,
    };

    assert_eq!(CubeMapViewMode::ALL.map(|m| m.label()), ["3D", "2D"]);

    let (_dir, editor) = project();
    let mut app = app(editor);

    app.world_mut().resource_mut::<Actions>().push(Action::OpenAsset(light_tex("sky")));
    app.update();
    app.update();

    // 1. Verify cache decoded all 6 faces into the 2D strip (4x24, not 4x4!) plus a Cube handle.
    {
        let p = app.world().resource::<Editor>().project.clone().unwrap();
        let settings =
            app.world().resource::<Editor>().active_doc::<TextureDoc<LightTexture>>().unwrap().asset.settings.clone();
        let cached = app.world_mut().resource_scope(|world, mut cache: Mut<GameImageCache<LightTexture>>| {
            let mut images = world.resource_mut::<Assets<Image>>();
            cache.get_cached(&p, "textures/sky.ktx2", &settings, None, &mut images).unwrap()
        });
        assert!(cached.is_cubemap);
        assert!(cached.cube_handle.is_some());
        let images = app.world().resource::<Assets<Image>>();
        assert_eq!(images.get(&cached.handle).unwrap().size(), UVec2::new(4, 24), "all 6 faces stacked in LightDomain");
    }

    // 2. Default mode is `CubeMapViewMode::Sphere` (`3D` = grey sphere inside cube-map skybox).
    let id = app.world().resource::<Editor>().active_doc::<TextureDoc<LightTexture>>().unwrap().preview.id();
    let (cam_entity, sphere_entity, mat_handle) = {
        let sess = app.world().resource::<CubeMapPreviewSessions>().sessions.get(&id).expect("cubemap preview session");
        (sess.camera, sess.sphere, sess.sphere_material.clone().expect("sphere material"))
    };
    {
        let w = app.world_mut();
        let preview_display =
            w.query_filtered::<&Node, With<CubeMapPreview<LightTexture>>>().single(w).unwrap().display;
        let (pic, pic_node) = w.query::<(&PreviewPic<LightTexture>, &Node)>().single(w).unwrap();
        assert_eq!(preview_display, Display::Flex);
        assert_eq!(pic_node.display, Display::None);
        assert_eq!(pic.0, UVec2::new(4, 24));
        assert_eq!(*w.entity(sphere_entity).get::<Visibility>().unwrap(), Visibility::Inherited);
        assert!(w.entity(cam_entity).get::<Transform>().unwrap().translation.length() > 1.0);
        assert!(w.entity(cam_entity).get::<Skybox>().is_some());
        assert!(w.entity(cam_entity).get::<EnvironmentMapLight>().is_some());
        let mat = w.resource::<Assets<StandardMaterial>>().get(&mat_handle).expect("sphere StandardMaterial");
        assert_eq!(mat.base_color, Color::srgb(0.5, 0.5, 0.5));
        assert_eq!(mat.metallic, 0.25);
        assert_eq!(mat.perceptual_roughness, 0.5);
    }

    // 3. Switch to `CubeMapViewMode::Strip2d` (`2D` 6-face strip view).
    app.world_mut().resource_mut::<Actions>().push(PreviewCubeMode::<LightTexture>::new(CubeMapViewMode::Strip2d));
    app.update();
    app.update();
    {
        let w = app.world_mut();
        let preview_display =
            w.query_filtered::<&Node, With<CubeMapPreview<LightTexture>>>().single(w).unwrap().display;
        let (pic, pic_node) = w.query::<(&PreviewPic<LightTexture>, &Node)>().single(w).unwrap();
        assert_eq!(preview_display, Display::None);
        assert_eq!(pic_node.display, Display::Flex);
        assert_eq!(pic.0, UVec2::new(4, 24));
        assert!(!w.entity(cam_entity).get::<Camera>().unwrap().is_active);
    }
}

/// `EnvironmentDoc` supports `Easy / Advanced` mode (`SetEasy(true)` resets advanced fields to defaults,
/// `SetEasy(false)` switches to `Advanced` keeping those defaults) and per-property `ResetProp(prop)`.
#[test]
fn environment_easy_advanced_mode_and_reset_prop() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(environment_ref("day")));
    assert!(active_env(&e).easy, "default environment starts in Easy mode");
    assert!(active_env(&e).is_default(EnvironmentProp::SkyboxBrightness));

    // Edit SkyboxBrightness -> is_default becomes false; ResetProp restores 2500.0 -> is_default becomes true.
    set_number(&mut e, EnvironmentProp::SkyboxBrightness, 750.0);
    assert_eq!(active_env(&e).env.skybox_brightness, 750.0);
    assert!(!active_env(&e).is_default(EnvironmentProp::SkyboxBrightness));
    e.apply(EnvironmentAction::ResetProp(EnvironmentProp::SkyboxBrightness));
    assert_eq!(active_env(&e).env.skybox_brightness, 2500.0);
    assert!(active_env(&e).is_default(EnvironmentProp::SkyboxBrightness));

    // Switch to Advanced mode (`SetEasy(false)`) and modify advanced fields:
    e.apply(EnvironmentAction::SetEasy(false));
    assert!(!active_env(&e).easy);
    set_number(&mut e, EnvironmentProp::SkyboxRotation, 45.0);
    set_number(&mut e, EnvironmentProp::LightShadowDepthBias(0), 0.12);
    e.apply(Action::Tab(TabAction::SetValue {
        key: key(EnvironmentProp::Tonemapping),
        value: PropValue::Str("AgX".into()),
        merge: false,
    }));
    e.apply(Action::Tab(TabAction::SetValue {
        key: key(EnvironmentProp::DebandDither),
        value: PropValue::Bool(false),
        merge: false,
    }));
    assert!(!light_domain::fits_easy(&active_env(&e).env));

    // Switching back to Easy mode (`SetEasy(true)`) resets all advanced fields to their defaults!
    e.apply(EnvironmentAction::SetEasy(true));
    assert!(active_env(&e).easy);
    assert!(light_domain::fits_easy(&active_env(&e).env));
    assert_eq!(active_env(&e).env.skybox_rotation, 0.0);
    assert_eq!(active_env(&e).env.lights[0].shadow_depth_bias, 0.02);
    assert_eq!(active_env(&e).env.tonemapping, TonemappingMode::TonyMcMapface);
    assert!(active_env(&e).env.deband_dither);

    // Undo restores the Advanced edits and automatically switches back to Advanced mode (`easy == false`).
    e.apply(Action::Tab(TabAction::Undo));
    assert!(!active_env(&e).easy);
    assert_eq!(active_env(&e).env.skybox_rotation, 45.0);
    assert_eq!(active_env(&e).env.tonemapping, TonemappingMode::AgX);
}

/// The `HDR Converter` tool (`Tools -> HDR Converter...`) imports a Radiance `.hdr` panorama, renders both the 2D
/// equirectangular preview and the 3D cubemap / 2D 6-face strip preview, updates conversion settings live, exports a
/// valid Bevy `.ktx2` cubemap without creating any `.texture.ron` asset, and cleans up its RAII 3D preview studio on
/// tab close.
#[test]
fn hdr_converter_tool_imports_previews_and_exports_ktx2_without_creating_texture_asset() {
    use bevy::asset::RenderAssetUsages;
    use bevy::core_pipeline::Skybox;
    use bevy::image::{CompressedImageFormats, Image, ImageSampler, ImageType};
    use bevy::light::EnvironmentMapLight;
    use bevy::render::render_resource::{TextureFormat, TextureViewDimension};
    use editor_core::preview::{PreviewEntity, PreviewTracker};
    use light_domain::backend::{CubemapFormat, encode_radiance_hdr};
    use light_domain::{
        HDR_CONVERTER_TOOL, Hdr2dPreviewImage, HdrConverterAction, HdrConverterDoc, HdrConverterProp,
        HdrCubePreviewImage, HdrCubeStripImage, demo_sky_hdr,
    };
    use texture_kit::editor::{CubeMapPreviewSessions, CubeMapViewMode};
    use texture_kit::shape_problem;

    let (dir, mut editor) = project();
    editor.apply(Action::SwitchDomain(LIGHT));
    let entries = menu_entries(MainMenu::Tools, &editor);
    assert_eq!(labels(&entries), ["Inspect Game...", "Light Transition Preview...", "HDR Converter..."]);
    assert_eq!(entries[2].action, Action::OpenTool { domain: LIGHT, id: HDR_CONVERTER_TOOL, arg: None });

    // Write a synthetic HDR panorama to disk.
    let hdr_src = demo_sky_hdr(32, 16);
    let hdr_path = dir.path().join("sunset_test.hdr");
    std::fs::write(&hdr_path, encode_radiance_hdr(hdr_src.width, hdr_src.height, &hdr_src.pixels)).unwrap();

    let mut app = app(editor);

    // 1. Open HDR Converter tool.
    app.world_mut().resource_mut::<Actions>().push(Action::OpenTool {
        domain: LIGHT,
        id: HDR_CONVERTER_TOOL,
        arg: None,
    });
    app.update();

    let id = {
        let ed = app.world().resource::<Editor>();
        let doc = ed.active_doc::<HdrConverterDoc>().expect("HDR Converter tab active");
        assert!(doc.hdr.is_none());
        assert_eq!(doc.output_summary(), "6x 512x512 | RGBA16Float (HDR) | 10 mips");
        doc.preview.id()
    };
    assert!(id > 0);
    assert!(app.world().resource::<PreviewTracker>().is_spawned(id));

    // 2. Import the `.hdr` file via `HdrConverterAction::ImportPath`.
    app.world_mut().resource_mut::<Actions>().push(HdrConverterAction::ImportPath(hdr_path.clone()));
    app.update();

    let cam_entity =
        app.world().resource::<CubeMapPreviewSessions>().sessions.get(&id).expect("session spawned").camera;
    {
        let w = app.world_mut();
        let doc = w.resource::<Editor>().active_doc::<HdrConverterDoc>().unwrap();
        assert_eq!(doc.source_path.as_deref(), Some(hdr_path.as_path()));
        assert_eq!(doc.hdr.as_ref().map(|h| (h.width, h.height)), Some((32, 16)));
        assert_eq!(doc.source_summary(), "sunset_test.hdr (32x16)");

        // Verify 2D equirectangular preview ImageNode is populated.
        let img_2d = w.query_filtered::<&ImageNode, With<Hdr2dPreviewImage>>().single(w).expect("2D preview ImageNode");
        let tex_2d = w.resource::<Assets<Image>>().get(&img_2d.image).expect("2D preview Image asset");
        assert_eq!(tex_2d.size(), UVec2::new(32, 16));

        // Verify 3D cubemap preview camera is active and has Skybox + EnvironmentMapLight bound.
        assert!(w.entity(cam_entity).get::<Camera>().unwrap().is_active);
        let skybox = w.entity(cam_entity).get::<Skybox>().expect("Skybox on cubemap preview camera");
        assert!(skybox.image.is_some());
        assert!(w.entity(cam_entity).get::<EnvironmentMapLight>().is_some());
        let img_3d =
            w.query_filtered::<&ImageNode, With<HdrCubePreviewImage>>().single(w).expect("3D preview ImageNode");
        assert!(w.resource::<Assets<Image>>().get(&img_3d.image).is_some());
    }

    // 3. Switch to 2D strip mode (`CubeMapViewMode::Strip2d`) and back to `Sphere`.
    app.world_mut().resource_mut::<Actions>().push(HdrConverterAction::SetCubeMode(CubeMapViewMode::Strip2d));
    app.update();
    {
        let w = app.world_mut();
        assert!(!w.entity(cam_entity).get::<Camera>().unwrap().is_active, "3D camera deactivated in Strip2d mode");
        let strip_node =
            w.query_filtered::<&ImageNode, With<HdrCubeStripImage>>().single(w).expect("2D strip ImageNode");
        let strip_img = w.resource::<Assets<Image>>().get(&strip_node.image).expect("2D strip Image asset");
        assert_eq!(strip_img.width() * 6, strip_img.height(), "6 cube faces stacked vertically");
    }
    app.world_mut().resource_mut::<Actions>().push(HdrConverterAction::SetCubeMode(CubeMapViewMode::Sphere));
    app.update();
    assert!(app.world().entity(cam_entity).get::<Camera>().unwrap().is_active);

    // 4. Modify FaceSize (to 256), Brightness, ExposureEv, Format, Mipmaps, and Zstd.
    app.world_mut().resource_mut::<Actions>().push(Action::Tab(TabAction::SetValue {
        key: PropKey::new(HdrConverterProp::FaceSize),
        value: PropValue::Str("256".into()),
        merge: false,
    }));
    app.world_mut().resource_mut::<Actions>().push(Action::Tab(TabAction::SetComponent {
        key: PropKey::new(HdrConverterProp::Brightness),
        comp: 0,
        value: 1.5,
        is_final: true,
    }));
    app.world_mut().resource_mut::<Actions>().push(Action::Tab(TabAction::SetComponent {
        key: PropKey::new(HdrConverterProp::ExposureEv),
        comp: 0,
        value: -0.5,
        is_final: true,
    }));
    app.world_mut().resource_mut::<Actions>().push(Action::Tab(TabAction::SetValue {
        key: PropKey::new(HdrConverterProp::Format),
        value: PropValue::Str(CubemapFormat::Rgba8Srgb.label().into()),
        merge: false,
    }));
    app.world_mut().resource_mut::<Actions>().push(Action::Tab(TabAction::SetValue {
        key: PropKey::new(HdrConverterProp::Mipmaps),
        value: PropValue::Bool(false),
        merge: false,
    }));
    app.world_mut().resource_mut::<Actions>().push(Action::Tab(TabAction::SetValue {
        key: PropKey::new(HdrConverterProp::Zstd),
        value: PropValue::Bool(true),
        merge: false,
    }));
    app.update();

    {
        let doc = app.world().resource::<Editor>().active_doc::<HdrConverterDoc>().unwrap();
        assert_eq!(doc.settings.face_size, 256);
        assert_eq!(doc.settings.brightness, 1.5);
        assert_eq!(doc.settings.exposure_ev, -0.5);
        assert_eq!(doc.settings.format, CubemapFormat::Rgba8Srgb);
        assert!(!doc.settings.mipmaps);
        assert!(doc.settings.zstd);
        assert_eq!(doc.output_summary(), "6x 256x256 | RGBA8 (sRGB) | 1 mip | Zstd");
    }

    // 5. Export to `assets/textures/converted_sky.ktx2` and verify NO `.texture.ron` asset is created.
    let ktx2_path = dir.path().join("assets/textures/converted_sky.ktx2");
    let light_textures_before: Vec<String> =
        app.world().resource::<Editor>().domains.get::<LightState>().unwrap().assets.textures.keys().cloned().collect();

    app.world_mut().resource_mut::<Actions>().push(HdrConverterAction::ExportPath(ktx2_path.clone()));
    app.update();

    assert!(ktx2_path.is_file(), "exported .ktx2 file exists on disk");
    let ktx2_bytes = std::fs::read(&ktx2_path).unwrap();
    assert_eq!(shape_problem(&ktx2_bytes, TextureShape::CubeMap), None);
    let decoded = Image::from_buffer(
        &ktx2_bytes,
        ImageType::Extension("ktx2"),
        CompressedImageFormats::NONE,
        true,
        ImageSampler::Default,
        RenderAssetUsages::default(),
    )
    .expect("Bevy decodes exported KTX2 cubemap");
    assert_eq!(
        (
            decoded.texture_descriptor.size.width,
            decoded.texture_descriptor.size.height,
            decoded.texture_descriptor.size.depth_or_array_layers,
        ),
        (256, 256, 6)
    );
    assert_eq!(decoded.texture_descriptor.mip_level_count, 1);
    assert_eq!(decoded.texture_descriptor.format, TextureFormat::Rgba8UnormSrgb);
    assert_eq!(decoded.texture_view_descriptor.as_ref().and_then(|v| v.dimension), Some(TextureViewDimension::Cube));

    // Verify no `.texture.ron` file was created for `converted_sky.ktx2`.
    app.world_mut().resource_mut::<Editor>().rescan();
    let light_textures_after: Vec<String> =
        app.world().resource::<Editor>().domains.get::<LightState>().unwrap().assets.textures.keys().cloned().collect();
    assert_eq!(light_textures_before, light_textures_after, "no .texture.ron asset created by HDR Converter");
    assert!(!dir.path().join("quillshade/light/textures/converted_sky.texture.ron").exists());
    assert!(!dir.path().join("quillshade/light/converted_sky.texture.ron").exists());

    // 6. Close the tab -> RAII handle drops and cleans up CubeMapPreviewSession and PreviewEntitys.
    app.world_mut().resource_mut::<Actions>().push(Action::DiscardTab(0));
    app.update();
    assert!(!app.world().resource::<PreviewTracker>().is_spawned(id));
    assert!(!app.world().resource::<CubeMapPreviewSessions>().sessions.contains_key(&id));
    assert_eq!(app.world_mut().query::<&PreviewEntity>().iter(app.world()).filter(|e| e.0 == id).count(), 0);
}
