//! Review Focus 5 (design §7.1): two test-only domains, `a` and `b`, each register the curve editor through
//! `curve_kit::editor::register`. The same suffix routes by folder (`quillshade/a/x.curve.ron` is `a`'s curve,
//! `quillshade/b/x.curve.ron` is `b`'s), the two editors live side by side in one `Editor` and one Bevy app, and
//! each one's systems, observers and input context act only on its own tab.

use std::sync::Arc;

use bevy::input::ButtonState;
use bevy::input::keyboard::{Key, KeyboardInput, NativeKey};
use bevy::prelude::*;
use bevy_enhanced_input::prelude::{ContextActivity, EnhancedInputPlugin};
use curve_kit::Curve;
use curve_kit::editor::{
    CurveAction, CurveClipboard, CurveCtx, CurveDoc, CurveDomain, CurveEdit, CurveHandle, CurveProp, active, register,
};
use editor_core::EditorPlugins;
use editor_core::action::{Action, Actions, TabAction};
use editor_core::config::UserConfig;
use editor_core::domain::{DomainInfo, DomainList, DomainRegistrar, EditorDomain};
use editor_core::main_input::{InputContextsPlugin, InputRoot};
use editor_core::state::editor::Editor;
use editor_core::tab::ViewKindId;
use editor_core::ui::areas::AreasPlugin;
use editor_core::ui::icons::IconId;
use editor_core::ui::shell::{DomainWindowPlugin, domain_doc_views};
use project_core::regen::DomainBackend;
use project_core::{AssetRef, DomainId, Project};

struct CurveA;
struct CurveB;

impl CurveDomain for CurveA {
    const DOMAIN: DomainId = DomainId::new("a");
    const VIEW: ViewKindId = ViewKindId("a.curve");
}

impl CurveDomain for CurveB {
    const DOMAIN: DomainId = DomainId::new("b");
    const VIEW: ViewKindId = ViewKindId("b.curve");
}

/// A domain whose only assets are `C`'s curves.
struct CurveOnly<C>(std::marker::PhantomData<C>);

impl<C: CurveDomain> EditorDomain for CurveOnly<C> {
    fn id(&self) -> DomainId {
        C::DOMAIN
    }
    fn info(&self) -> DomainInfo {
        DomainInfo { title: C::DOMAIN.as_str(), icon: IconId::File.into(), summary: "" }
    }
    fn backend(&self) -> Option<Box<dyn DomainBackend>> {
        None
    }
    fn register(&self, r: &mut DomainRegistrar) {
        register::<C>(r);
    }
}

fn domains() -> Arc<DomainList> {
    Arc::new(DomainList::new(vec![
        Box::new(CurveOnly::<CurveA>(Default::default())),
        Box::new(CurveOnly::<CurveB>(Default::default())),
    ]))
}

/// `x` of domain `id`.
fn x(id: &'static str) -> AssetRef {
    AssetRef { domain: DomainId::new(id), path: "x".into(), suffix: "curve" }
}

/// A game with `quillshade/a/x.curve.ron` (a ramp) and `quillshade/b/x.curve.ron` (three points), open in an editor
/// of both domains.
fn project() -> (tempfile::TempDir, Editor) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"g\"\n").unwrap();
    std::fs::create_dir_all(dir.path().join("assets")).unwrap();
    let p = Project::open(dir.path()).unwrap();
    curve_kit::editor::save(&p, &x("a"), &Curve::ramp("x")).unwrap();
    curve_kit::editor::save(&p, &x("b"), &Curve::new("x", vec![(0.0, 1.0), (0.5, 0.2), (1.0, 1.0)])).unwrap();
    let mut e = Editor::new(domains());
    e.open_project(dir.path()).unwrap();
    (dir, e)
}

fn file(dir: &tempfile::TempDir, id: &str) -> String {
    std::fs::read_to_string(dir.path().join(format!("quillshade/{id}/x.curve.ron"))).unwrap()
}

fn points<C: CurveDomain>(e: &Editor) -> Vec<(f32, f32)> {
    e.tabs.iter().find_map(|t| t.get::<CurveDoc<C>>()).expect("the curve tab").curve.points.clone()
}

#[test]
fn two_curve_domains_side_by_side() {
    let (dir, mut e) = project();
    e.apply(Action::OpenAsset(x("a")));
    e.apply(Action::OpenAsset(x("b")));
    assert_eq!(e.tabs.len(), 2);
    // The folder decides the type: one tab per domain, each of its own kind and view.
    assert!(e.tabs[0].get::<CurveDoc<CurveA>>().is_some() && e.tabs[0].get::<CurveDoc<CurveB>>().is_none());
    assert!(e.tabs[1].get::<CurveDoc<CurveB>>().is_some() && e.tabs[1].get::<CurveDoc<CurveA>>().is_none());
    let tabs: Vec<_> = e.tabs.iter().map(|t| (t.domain.as_str(), t.doc().view(), t.title())).collect();
    assert_eq!(
        tabs,
        [("a", CurveA::VIEW, "quillshade/a/x.curve".into()), ("b", CurveB::VIEW, "quillshade/b/x.curve".into())]
    );
    assert!(active::<CurveB>(&e).is_some() && active::<CurveA>(&e).is_none());
    let (a_file, b_file) = (file(&dir, "a"), file(&dir, "b"));
    let b_points = points::<CurveB>(&e);

    // An edit of `a`'s curve, saved, leaves `b`'s document and file alone.
    e.apply(Action::ActivateTab(0));
    e.apply(Action::from(CurveEdit::<CurveA>::new(CurveAction::Add((0.5, 0.25)))));
    assert_eq!(points::<CurveA>(&e), [(0.0, 0.0), (0.5, 0.25), (1.0, 1.0)]);
    assert_eq!(points::<CurveB>(&e), b_points);
    assert!(e.tabs[0].doc().is_dirty() && !e.tabs[1].doc().is_dirty());
    e.apply(Action::Save);
    assert_ne!(file(&dir, "a"), a_file);
    assert_eq!(file(&dir, "b"), b_file, "the other domain's file");
    let saved = curve_kit::editor::load(e.project.as_ref().unwrap(), &x("a")).unwrap();
    assert_eq!(saved.points, points::<CurveA>(&e));

    // One curve clipboard for every domain: points copied in `a` paste into `b`, one step further in t.
    e.apply(Action::Tab(TabAction::Copy));
    assert_eq!(e.clipboards.get::<CurveClipboard>().map(|c| c.points.clone()), Some(vec![(0.5, 0.25)]));
    e.apply(Action::ActivateTab(1));
    e.apply(Action::Tab(TabAction::Paste(None)));
    assert_eq!(points::<CurveB>(&e), [(0.0, 1.0), (0.5, 0.2), (0.55, 0.25), (1.0, 1.0)]);
    assert_eq!(points::<CurveA>(&e), [(0.0, 0.0), (0.5, 0.25), (1.0, 1.0)]);
    assert_eq!(file(&dir, "b"), b_file, "not saved");
}

/// The tab keys are the domains' (`a:quillshade/a/x.curve.ron`): both curve tabs come back, each as its own type,
/// through core's default for a kind's source path (design §14.4): the domains wire no tab keys.
#[test]
fn curve_tabs_of_two_domains_are_restored() {
    let (dir, mut e) = project();
    let config = dir.path().join("config/config.ron");
    e.config_path = Some(config.clone());
    e.apply(Action::OpenAsset(x("b")));
    e.apply(Action::OpenAsset(x("a")));
    e.apply(Action::Quit);
    let mut next = Editor::new(domains()).with_config(UserConfig::load(&config), Some(config));
    next.open_project(dir.path()).unwrap();
    assert_eq!(next.tabs.len(), 2);
    assert_eq!(next.tabs[0].get::<CurveDoc<CurveB>>().map(|c| &c.aref), Some(&x("b")));
    assert_eq!(next.tabs[1].get::<CurveDoc<CurveA>>().map(|c| &c.aref), Some(&x("a")));
    assert_eq!(active::<CurveA>(&next).map(|c| &c.aref), Some(&x("a")));
}

fn key(app: &mut App, key_code: KeyCode, state: ButtonState) {
    let logical_key = Key::Unidentified(NativeKey::Unidentified);
    let input = KeyboardInput { key_code, logical_key, state, text: None, repeat: false, window: Entity::PLACEHOLDER };
    app.world_mut().write_message(input);
    app.update();
}

fn act(app: &mut App, a: impl Into<Action>) {
    app.world_mut().resource_mut::<Actions>().push(a);
    app.update();
    app.update();
}

fn handles<C: CurveDomain>(app: &mut App) -> usize {
    let w = app.world_mut();
    w.query::<&CurveHandle<C>>().iter(w).count()
}

fn context_active<C: CurveDomain>(app: &mut App) -> Option<bool> {
    let w = app.world_mut();
    let root = w.query_filtered::<Entity, With<InputRoot>>().single(w).unwrap();
    w.get::<ContextActivity<CurveCtx<C>>>(root).map(|a| **a)
}

/// Both domains' window parts in one headless app: each registers its plugin, areas, view and input context without
/// a clash; only the active domain's view gets point handles, only its context is active and its arrow keys move only
/// its points.
#[test]
fn two_curve_editors_in_one_app() {
    let (_dir, mut editor) = project();
    // Both tabs open before the app runs, `a`'s active: `b`'s view never showed a curve.
    editor.apply(Action::OpenAsset(x("b")));
    editor.apply(Action::OpenAsset(x("a")));
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin::default(),
        bevy::scene::ScenePlugin,
        bevy::input::InputPlugin,
        EnhancedInputPlugin,
    ));
    app.init_asset::<Font>().init_asset::<Image>();
    app.init_resource::<bevy::input_focus::InputFocus>();
    app.insert_resource(editor);
    // What the window's `UiPlugin` does with the domains: their Bevy parts and their views.
    app.add_plugins((EditorPlugins, AreasPlugin, DomainWindowPlugin, InputContextsPlugin));
    for view in domain_doc_views(app.world().resource::<DomainList>()) {
        app.world_mut().spawn_scene(view).unwrap();
    }
    app.finish();
    app.update();

    app.update();
    assert_eq!((handles::<CurveA>(&mut app), handles::<CurveB>(&mut app)), (2, 0));
    assert_eq!((context_active::<CurveA>(&mut app), context_active::<CurveB>(&mut app)), (Some(true), Some(false)));
    act(&mut app, Action::OpenAsset(x("b")));
    assert_eq!(handles::<CurveB>(&mut app), 3);
    assert_eq!(handles::<CurveA>(&mut app), 2, "`a`'s view keeps its handles while hidden");
    assert_eq!((context_active::<CurveA>(&mut app), context_active::<CurveB>(&mut app)), (Some(false), Some(true)));

    // Arrow up nudges the selected point of the active curve only.
    act(&mut app, CurveEdit::<CurveB>::new(CurveAction::Select([1].into())));
    let a_before = points::<CurveA>(app.world().resource::<Editor>());
    key(&mut app, KeyCode::ArrowUp, ButtonState::Pressed);
    app.update();
    key(&mut app, KeyCode::ArrowUp, ButtonState::Released);
    app.update();
    let e = app.world().resource::<Editor>();
    assert!((points::<CurveB>(e)[1].1 - 0.21).abs() < 1e-5, "{:?}", points::<CurveB>(e));
    assert_eq!(points::<CurveA>(e), a_before);
}

/// Design §7.1: the number fields of a kit's right sidebar work without Material: a Min field edit of `a`'s curve (a
/// `ValueChange<f32>` of its `PropField`, as Feathers sends it) reaches `a`'s document, through core's observers.
#[test]
fn kit_sidebar_number_field_edits_its_doc() {
    use bevy::ui_widgets::ValueChange;
    use editor_core::dynval::{KitProp, PropKey};
    use editor_core::ui::areas::right_sidebar;
    use editor_core::ui::props::PropField;

    let (_dir, mut editor) = project();
    editor.apply(Action::OpenAsset(x("a")));
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin::default(),
        bevy::scene::ScenePlugin,
        bevy::input::InputPlugin,
        EnhancedInputPlugin,
    ));
    app.init_asset::<Font>().init_asset::<Image>();
    app.init_resource::<bevy::input_focus::InputFocus>().init_resource::<bevy::input_focus::InputFocusVisible>();
    // The sidebar's number fields are Feathers widgets.
    app.init_resource::<bevy::feathers::controls::UnitsRegistry>();
    app.insert_resource(bevy::feathers::theme::UiTheme(bevy::feathers::dark_theme::create_dark_theme()));
    app.insert_resource(editor);
    app.add_plugins((EditorPlugins, AreasPlugin, DomainWindowPlugin, InputContextsPlugin));
    app.world_mut().spawn_scene(right_sidebar()).unwrap();
    app.finish();
    app.update();
    app.update();

    let min = {
        let w = app.world_mut();
        let key = PropKey::new(KitProp::<CurveA, _>::new(CurveProp::Range));
        let mut q = w.query::<(Entity, &PropField)>();
        q.iter(w).find(|(_, f)| f.target == key && f.comp == 0).map(|(e, _)| e).expect("the Min field")
    };
    app.world_mut().trigger(ValueChange { source: min, value: -2.0f32, is_final: true });
    app.update();
    app.update();
    let curve = |app: &App| {
        let e = app.world().resource::<Editor>();
        e.tabs.iter().find_map(|t| t.get::<CurveDoc<CurveA>>()).map(|c| (c.curve.min, c.curve.max))
    };
    assert_eq!(curve(&app), Some((-2.0, 1.0)), "the edit reached `a`'s curve");
    assert!(app.world().resource::<Editor>().tabs[0].doc().is_dirty());
}

/// Design §14.4: kit edits carry their domain. An edit or a property write meant for `a`'s curve tab that arrives while
/// `b`'s curve tab is active is dropped: `b`'s document does not change and nothing panics.
#[test]
fn curve_edit_for_other_domain_is_dropped() {
    use editor_core::dynval::{KitProp, Payload, PropKey, PropValue};

    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(x("a")));
    e.apply(Action::OpenAsset(x("b")));
    e.apply(Action::from(CurveEdit::<CurveB>::new(CurveAction::Select([1].into()))));
    let b = e.active.unwrap();
    let (revision, points_b, points_a) = (e.tabs[b].doc().revision(), points::<CurveB>(&e), points::<CurveA>(&e));

    e.apply(Action::Tab(TabAction::Doc(Payload::new(CurveEdit::<CurveA>::new(CurveAction::Nudge((0.1, 0.1)))))));
    e.apply(Action::Tab(TabAction::Doc(Payload::new(CurveEdit::<CurveA>::new(CurveAction::Add((0.9, 0.9)))))));
    let range = PropKey::new(KitProp::<CurveA, _>::new(CurveProp::Range));
    e.apply(Action::Tab(TabAction::SetValue { key: range, value: PropValue::Float2([-3.0, 3.0]), merge: false }));
    assert_eq!(e.tabs[b].doc().revision(), revision);
    assert_eq!(points::<CurveB>(&e), points_b);
    assert_eq!(points::<CurveA>(&e), points_a, "nor does it reach `a`'s inactive tab");
    assert!(!e.is_dirty());
    // `b`'s own edit still applies.
    e.apply(Action::from(CurveEdit::<CurveB>::new(CurveAction::Nudge((0.0, -0.1)))));
    assert_ne!(e.tabs[b].doc().revision(), revision);
}
