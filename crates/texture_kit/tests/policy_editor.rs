//! The texture editor of `texture_kit` (design §7.2) on two test-only domains, without Material: `plain` (Material's
//! policy without built-in textures: `ktx2` / `png`, Easy mode) and `cube` (`ktx2` only, no presets). The policy
//! decides what the Texture tab offers and which game files get "Create Texture Asset"; the form's fields, the path
//! rules and the tab keys work through the kit and core alone.

use std::sync::Arc;

use bevy::prelude::*;
use bevy::ui_widgets::ValueChange;
use editor_core::EditorPlugins;
use editor_core::action::{Action, Actions, Dialog, TabAction, TreeId};
use editor_core::config::UserConfig;
use editor_core::domain::{DomainInfo, DomainList, DomainRegistrar, EditorDomain};
use editor_core::dynval::{KitProp, Payload, PropKey, PropValue};
use editor_core::state::editor::Editor;
use editor_core::tab::ViewKindId;
use editor_core::ui::OnActivate;
use editor_core::ui::areas::{AreasPlugin, editor_topbar, right_sidebar};
use editor_core::ui::icons::IconId;
use editor_core::ui::props::PropField;
use editor_core::ui::shell::{DomainWindowPlugin, domain_doc_views};
use editor_core::ui::tree::{TreeItem, TreeRow, row_menu};
use project_core::regen::DomainBackend;
use project_core::{AssetRef, DomainId, Project};
use texture_kit::editor::{
    ImageDoc, TextureAction, TextureDoc, TextureDomain, TextureEdit, TextureField, TextureProp, TextureViewBar,
    TextureViewForm, active, register,
};
use texture_kit::{TextureAsset, TexturePolicy, TextureShape};

struct Plain;
struct Cube;

fn scanned_owners<D: TextureDomain>(editor: &Editor, file: &str) -> Vec<AssetRef> {
    let Some(project) = &editor.project else { return Vec::new() };
    editor
        .kind_files
        .get(&D::DOMAIN)
        .into_iter()
        .flatten()
        .filter(|r| r.suffix == D::SUFFIX && texture_kit::editor::load(project, r).is_ok_and(|t| t.path == file))
        .cloned()
        .collect()
}

impl TextureDomain for Plain {
    const DOMAIN: DomainId = DomainId::new("plain");
    const VIEW: ViewKindId = ViewKindId("plain.texture");
    const IMAGE_VIEW: ViewKindId = ViewKindId("plain.image");
    const POLICY: TexturePolicy =
        TexturePolicy { extensions: &["ktx2", "png"], shape: TextureShape::Plain2d, internal: None, presets: true };

    fn disk_owners(editor: &Editor, file: &str) -> Vec<AssetRef> {
        scanned_owners::<Self>(editor, file)
    }
}

impl TextureDomain for Cube {
    const DOMAIN: DomainId = DomainId::new("cube");
    const VIEW: ViewKindId = ViewKindId("cube.texture");
    const IMAGE_VIEW: ViewKindId = ViewKindId("cube.image");
    const POLICY: TexturePolicy =
        TexturePolicy { extensions: &["ktx2"], shape: TextureShape::CubeMap, internal: None, presets: false };

    fn disk_owners(editor: &Editor, file: &str) -> Vec<AssetRef> {
        scanned_owners::<Self>(editor, file)
    }
}

/// A domain whose only assets are `T`'s textures.
struct TextureOnly<T>(std::marker::PhantomData<T>);

impl<T: TextureDomain> EditorDomain for TextureOnly<T> {
    fn id(&self) -> DomainId {
        T::DOMAIN
    }
    fn info(&self) -> DomainInfo {
        DomainInfo { title: T::DOMAIN.as_str(), icon: IconId::File.into(), summary: "" }
    }
    fn backend(&self) -> Option<Box<dyn DomainBackend>> {
        None
    }
    fn register(&self, r: &mut DomainRegistrar) {
        register::<T>(r);
    }
}

fn plain_only<T: TextureDomain>() -> Box<dyn EditorDomain> {
    Box::new(TextureOnly::<T>(Default::default()))
}

fn domains() -> Arc<DomainList> {
    Arc::new(DomainList::new(vec![plain_only::<Plain>(), plain_only::<Cube>()]))
}

/// Texture asset `path` of domain `T`.
fn tex<T: TextureDomain>(path: &str) -> AssetRef {
    AssetRef { domain: T::DOMAIN, path: path.into(), suffix: T::SUFFIX }
}

const SAMPLES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/sample_game/assets/textures");

/// A game with `assets/textures/{a,b}.{png,ktx2}` (real images), `plain`'s `t` on `a.png`, `cube`'s `t` on `a.ktx2`
/// and `cube`'s `u` on `b.ktx2`, open in an editor of `list`.
fn project(list: Arc<DomainList>) -> (tempfile::TempDir, Editor) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"g\"\n").unwrap();
    let textures = dir.path().join("assets/textures");
    std::fs::create_dir_all(&textures).unwrap();
    for (to, from) in [("a.png", "rock_albedo.png"), ("a.ktx2", "rock_albedo.ktx2"), ("b.ktx2", "rock_albedo.ktx2")] {
        std::fs::copy(format!("{SAMPLES}/{from}"), textures.join(to)).unwrap();
    }
    let p = Project::open(dir.path()).unwrap();
    texture_kit::editor::save(&p, &tex::<Plain>("t"), &TextureAsset::new("textures/a.png")).unwrap();
    texture_kit::editor::save(&p, &tex::<Cube>("t"), &TextureAsset::new("textures/a.ktx2")).unwrap();
    texture_kit::editor::save(&p, &tex::<Cube>("u"), &TextureAsset::new("textures/b.ktx2")).unwrap();
    let mut e = Editor::new(list);
    e.open_project(dir.path()).unwrap();
    (dir, e)
}

/// A headless window app over `editor` built the way the window builds the domains' parts.
fn app(editor: Editor) -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default(), bevy::scene::ScenePlugin, bevy::input::InputPlugin));
    app.init_asset::<Font>().init_asset::<Image>();
    app.init_resource::<bevy::input_focus::InputFocus>().init_resource::<bevy::input_focus::InputFocusVisible>();
    // The form's fields are Feathers widgets.
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

fn act(app: &mut App, a: impl Into<Action>) {
    app.world_mut().resource_mut::<Actions>().push(a);
    app.update();
    app.update();
}

/// Whether `e` lies in the subtree of `root`.
fn inside(w: &World, mut e: Entity, root: Entity) -> bool {
    loop {
        if e == root {
            return true;
        }
        match w.get::<ChildOf>(e) {
            Some(p) => e = p.parent(),
            None => return false,
        }
    }
}

/// The actions of the buttons in `T`'s Texture editor topbar.
fn bar_actions<T: TextureDomain>(app: &mut App) -> Vec<Action> {
    let w = app.world_mut();
    let bar = w.query_filtered::<Entity, With<TextureViewBar<T>>>().single(w).unwrap();
    let buttons: Vec<(Entity, Action)> =
        w.query::<(Entity, &OnActivate)>().iter(w).map(|(e, a)| (e, a.0.clone())).collect();
    buttons.into_iter().filter(|(e, _)| inside(w, *e, bar)).map(|(_, a)| a).collect()
}

fn form_display<T: TextureDomain>(app: &mut App) -> Display {
    let w = app.world_mut();
    w.query_filtered::<&Node, With<TextureViewForm<T>>>().single(w).unwrap().display
}

fn is_mode_or_preset(a: &Action) -> bool {
    let Action::Tab(TabAction::Doc(p)) = a else { return false };
    let edit = p.downcast::<TextureEdit<Plain>>().map(|e| &e.0).or(p.downcast::<TextureEdit<Cube>>().map(|e| &e.0));
    matches!(edit, Some(TextureAction::SetEasy(_) | TextureAction::Preset(_)))
}

/// Design §7.2: a domain whose policy has no presets gets the plain sampler form, without the Easy / Advanced switch
/// or the use-case presets; a domain with presets (side by side, in the same app) keeps them.
#[test]
fn cube_policy_hides_presets() {
    let (_dir, editor) = project(domains());
    let mut app = app(editor);
    act(&mut app, Action::OpenAsset(tex::<Plain>("t")));
    let plain = bar_actions::<Plain>(&mut app);
    assert_eq!(plain.iter().filter(|a| is_mode_or_preset(a)).count(), 2 + 6, "Easy, Advanced and six presets");
    assert!(active::<Plain>(app.world().resource::<Editor>()).is_some_and(|t| t.easy), "a default sampler: Easy");
    assert_eq!(form_display::<Plain>(&mut app), Display::None, "Easy has no form");

    act(&mut app, Action::OpenAsset(tex::<Cube>("t")));
    let cube = bar_actions::<Cube>(&mut app);
    assert!(!cube.is_empty(), "the preview's controls are there: {cube:?}");
    assert!(!cube.iter().any(is_mode_or_preset), "{cube:?}");
    let t = active::<Cube>(app.world().resource::<Editor>()).expect("cube's tab");
    assert!(!t.easy, "no Easy without presets");
    assert_eq!(form_display::<Cube>(&mut app), Display::Flex, "the form shows");
    // Easy cannot be forced either.
    act(&mut app, Action::from(TextureEdit::<Cube>::new(TextureAction::SetEasy(true))));
    assert!(!active::<Cube>(app.world().resource::<Editor>()).unwrap().easy);
    assert_eq!(bar_actions::<Plain>(&mut app), plain, "`plain`'s bar is its own");
}

/// Design §7.2, the texture form: a field edit (the "Default" sampler checkbox, as Feathers sends it) reaches the
/// doc of a kit-only domain through core's observers.
#[test]
fn form_field_edits_the_doc_without_material() {
    let (_dir, editor) = project(domains());
    let mut app = app(editor);
    act(&mut app, Action::OpenAsset(tex::<Cube>("t")));
    let default = {
        let w = app.world_mut();
        let key = PropKey::new(KitProp::<Cube, _>::new(TextureProp::Field(TextureField::DefaultSampler)));
        let mut q = w.query::<(Entity, &PropField)>();
        q.iter(w).find(|(_, f)| f.target == key).map(|(e, _)| e).expect("the Default checkbox")
    };
    app.world_mut().trigger(ValueChange { source: default, value: false, is_final: true });
    app.update();
    app.update();
    let e = app.world().resource::<Editor>();
    let t = active::<Cube>(e).unwrap();
    assert_eq!(t.asset.settings.sampler, Some(texture_kit::SamplerSettings::image_plugin_default()));
    assert!(e.tab().unwrap().doc().is_dirty());
}

/// Design §4.3, §7.2: "Create Texture Asset" is offered only for the game files the domain's policy accepts; the
/// created asset lands in the domain's own `textures/` folder and the menu item is disabled afterwards.
#[test]
fn create_texture_respects_extensions() {
    let (dir, mut e) = project(Arc::new(DomainList::new(vec![plain_only::<Cube>()])));
    std::fs::copy(format!("{SAMPLES}/rock_albedo.ktx2"), dir.path().join("assets/textures/c.ktx2")).unwrap();
    std::fs::copy(format!("{SAMPLES}/rock_albedo.png"), dir.path().join("assets/textures/c.png")).unwrap();
    e.refresh_game_files();
    assert!(e.game_files.contains(&"textures/c.ktx2".to_string()));
    assert!(!e.game_files.iter().any(|f| f.ends_with(".png")), "{:?}", e.game_files);
    let menu =
        |e: &Editor, f: &str| row_menu(e, &TreeRow { tree: TreeId::GameAssets, item: TreeItem::GameFile(f.into()) });
    assert!(menu(&e, "textures/c.png").is_empty(), "no item for a file the policy refuses");
    let items = menu(&e, "textures/c.ktx2");
    assert_eq!(items.iter().map(|m| m.label.as_str()).collect::<Vec<_>>(), ["Create Texture Asset"]);
    assert!(items[0].disabled.is_none());
    e.apply(items[0].action.clone());
    let created = tex::<Cube>("textures/c");
    assert!(dir.path().join(created.source_rel()).is_file(), "{:?}", e.messages);
    assert!(
        e.messages
            .iter()
            .any(|m| m.text == "Created quillshade/cube/textures/c.texture.ron for assets/textures/c.ktx2")
    );
    let again = menu(&e, "textures/c.ktx2");
    assert!(again[0].disabled.as_deref().is_some_and(|t| t.contains("quillshade/cube/textures/c.texture.ron")));
    // Taken files are refused, as are files the policy does not accept.
    assert_eq!(texture_kit::editor::create_for_file::<Cube>(&mut e, "textures/a.ktx2"), None, "`t` points there");
    e.apply(Action::CloseDialog);
    assert_eq!(texture_kit::editor::create_for_file::<Cube>(&mut e, "textures/c.png"), None);
    assert!(e.messages.last().is_some_and(|m| m.text.contains("unsupported image format")), "{:?}", e.messages.last());
    assert!(!dir.path().join("quillshade/cube/textures/c_2.texture.ron").exists());
}

/// One game file, one Texture asset (design §1.1) without Material: a path edit onto another asset's file and one
/// outside the policy are refused (the kit's deferred step), a free file is taken; undo works.
#[test]
fn path_edits_follow_the_policy() {
    let (_dir, mut e) = project(domains());
    e.apply(Action::OpenAsset(tex::<Cube>("t")));
    let path = |e: &Editor| active::<Cube>(e).unwrap().asset.path.clone();
    let set_path = |p: &str| Action::from(TextureEdit::<Cube>::new(TextureAction::SetPath(p.into())));
    e.apply(set_path("textures/b.ktx2"));
    assert_eq!(path(&e), "textures/a.ktx2", "`u` points there");
    assert!(matches!(e.dialog, Some(Dialog::Message { .. })));
    assert!(e.messages.last().unwrap().text.contains("quillshade/cube/u.texture.ron"), "{:?}", e.messages.last());
    e.apply(Action::CloseDialog);
    e.apply(set_path("textures/a.png"));
    assert_eq!(path(&e), "textures/a.ktx2", "png is not `cube`'s");
    e.apply(Action::CloseDialog);
    e.apply(set_path("textures/z.ktx2"));
    assert_eq!(path(&e), "textures/z.ktx2", "a free file (missing is only a warning)");
    assert!(e.is_dirty());
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(path(&e), "textures/a.ktx2");
    assert!(!e.is_dirty());
    // `plain` may point at `a.ktx2` too: one asset per file and domain.
    e.apply(Action::OpenAsset(tex::<Plain>("t")));
    e.apply(Action::from(TextureEdit::<Plain>::new(TextureAction::SetPath("textures/a.ktx2".into()))));
    assert_eq!(active::<Plain>(&e).unwrap().asset.path, "textures/a.ktx2");
}

/// Tab keys stay the domain's (`cube:quillshade/cube/t.texture.ron`, `cube:@game/textures/a.ktx2`): both come back as
/// `cube`'s tabs on the next launch, through what the kit's `register` wires (design §14.4): the domains wire none.
#[test]
fn texture_and_image_tabs_are_restored() {
    let (dir, mut e) = project(domains());
    let config = dir.path().join("config/config.ron");
    e.config_path = Some(config.clone());
    e.apply(Action::OpenAsset(tex::<Cube>("t")));
    e.apply(Action::OpenGameImage("textures/a.ktx2".into()));
    // Both domains show `.ktx2` game files: only the active one (`cube`) opens it (design §14.4).
    assert_eq!(e.tabs.len(), 2, "{:?}", e.tabs);
    assert!(e.tabs.iter().all(|t| t.domain == Cube::DOMAIN));
    e.apply(Action::Quit);
    let saved = UserConfig::load(&config);
    let keys: Vec<String> = saved.tabs.values().flat_map(|t| t.open.clone()).collect();
    assert!(keys.contains(&"cube:quillshade/cube/t.texture.ron".to_string()), "{keys:?}");
    assert!(keys.contains(&"cube:@game/textures/a.ktx2".to_string()), "{keys:?}");
    let mut next = Editor::new(domains()).with_config(saved, Some(config));
    next.open_project(dir.path()).unwrap();
    assert!(next.tabs.iter().any(|t| t.get::<TextureDoc<Cube>>().is_some_and(|d| d.aref == tex::<Cube>("t"))));
    assert!(next.tabs.iter().any(|t| t.get::<ImageDoc<Cube>>().is_some_and(|i| i.path == "textures/a.ktx2")));
}

/// Moved from `shader_project::game_assets` (the "Create Texture Asset" half, design §4.3): new Texture assets mirror
/// the game file's folders, a taken name gets a number, a claimed file is refused.
#[test]
fn created_ids_mirror_folders_and_avoid_clashes() {
    use texture_kit::editor::{texture_for_file, try_create_for_file};
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"g\"\n").unwrap();
    let textures = dir.path().join("assets/textures");
    std::fs::create_dir_all(&textures).unwrap();
    for f in ["a.png", "a.ktx2"] {
        std::fs::write(textures.join(f), b"x").unwrap();
    }
    let mut e = Editor::new(Arc::new(DomainList::new(vec![plain_only::<Plain>()])));
    e.open_project(dir.path()).unwrap();
    let project = e.project.clone().unwrap();
    let taken = |p: &str| project.root.join(tex::<Plain>(p).source_rel()).exists();

    let a = try_create_for_file::<Plain>(&mut e, "textures/a.png").unwrap();
    assert_eq!(a, tex::<Plain>("textures/a"));
    assert_eq!(texture_for_file::<Plain>(&e, "textures/a.png"), Some(a.clone()));
    assert_eq!(texture_for_file::<Plain>(&e, "textures/a.ktx2"), None);
    // Same stem, other file: a suffix. The same file again: refused.
    assert_eq!(texture_kit::new_texture_path_for_file("textures/a.ktx2", taken), "textures/a_2");
    assert!(try_create_for_file::<Plain>(&mut e, "textures/a.png").is_err());
    let b = try_create_for_file::<Plain>(&mut e, "textures/a.ktx2").unwrap();
    assert_eq!(b.path, "textures/a_2");
    assert_eq!(texture_kit::new_texture_path_for_file("a.ktx2", taken), "textures/a_3");
    assert_eq!(texture_kit::editor::load(&project, &b).unwrap(), TextureAsset::new("textures/a.ktx2"));
}

/// Review Focus 4 (design §14.4): kit edits carry their domain. A `plain` texture edit queued before switching to a
/// `cube` Texture tab, and a `plain` property write, are dropped: `cube`'s document does not change and nothing
/// panics; `plain`'s inactive tab is not touched either.
#[test]
fn texture_edit_for_other_domain_is_dropped() {
    let (_dir, mut e) = project(domains());
    e.apply(Action::OpenAsset(tex::<Plain>("t")));
    e.apply(Action::OpenAsset(tex::<Cube>("t")));
    let cube = e.active.unwrap();
    assert!(e.tabs[cube].get::<TextureDoc<Cube>>().is_some());
    let revision = e.tabs[cube].doc().revision();
    let settings = |e: &Editor| {
        let plain = e.tabs.iter().find_map(|t| t.get::<TextureDoc<Plain>>()).unwrap().asset.clone();
        (active::<Cube>(e).unwrap().asset.clone(), plain)
    };
    let before = settings(&e);
    let mut custom = before.0.settings.clone();
    custom.is_srgb = !custom.is_srgb;

    let stale = |a: TextureAction| Action::Tab(TabAction::Doc(Payload::new(TextureEdit::<Plain>::new(a))));
    e.apply(stale(TextureAction::SetSettings(custom.clone())));
    e.apply(stale(TextureAction::SetPath("textures/b.ktx2".into())));
    e.apply(stale(TextureAction::History { undo: true }));
    let srgb = PropKey::new(KitProp::<Plain, _>::new(TextureProp::Field(TextureField::IsSrgb)));
    e.apply(Action::Tab(TabAction::SetValue { key: srgb, value: PropValue::Bool(custom.is_srgb), merge: false }));
    assert_eq!(e.tabs[cube].doc().revision(), revision);
    assert_eq!(settings(&e), before);
    assert!(!e.is_dirty() && e.dialog.is_none(), "{:?}", e.dialog);
    // `cube`'s own edit still applies.
    e.apply(Action::from(TextureEdit::<Cube>::new(TextureAction::SetSettings(custom.clone()))));
    assert_eq!(active::<Cube>(&e).unwrap().asset.settings, custom);
}
