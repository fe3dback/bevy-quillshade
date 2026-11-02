//! The Migration Tool modal without a window (ADR 0046 "Editor (Migration Tool modal)", D28): an older project
//! opens into the modal, its buttons drive the session, and after the run the project opens normally.

use std::path::Path;

use bevy::prelude::*;
use bevy::ui_widgets::Activate;
use editor_core::EditorPlugins;
use editor_core::action::{Action, Actions, MigrationAction};
use editor_core::state::editor::Editor;
use editor_core::state::migration::MigrationPhase;
use editor_core::ui::OnActivate;
use editor_core::ui::migration::{MigrationPlugin, MigrationRoot};

/// The permanent v1 fixture of `project_migrate` (never updated: it stays v1).
fn fixture() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../project_migrate/tests/fixtures/v1")
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// A copy of the v1 fixture and a headless editor app with the modal and Feathers' modal dialog plugins; the
/// project is not opened yet.
fn app() -> (tempfile::TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    copy_dir(&fixture(), dir.path());
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default(), bevy::scene::ScenePlugin, bevy::input::InputPlugin));
    // Feathers' widgets load fonts and images in their templates.
    app.init_asset::<Font>().init_asset::<Image>().init_asset::<bevy::shader::Shader>();
    app.init_resource::<bevy::input_focus::InputFocus>().init_resource::<bevy::input_focus::InputFocusVisible>();
    app.insert_resource(bevy::feathers::theme::UiTheme(bevy::feathers::dark_theme::create_dark_theme()));
    app.insert_resource(material_domain::test_editor());
    app.add_plugins(EditorPlugins);
    app.add_plugins((MigrationPlugin, bevy::ui_widgets::DialogPlugin, bevy::ui_widgets::ModalDialogPlugin));
    app.add_observer(editor_core::ui::shell::activate_action);
    app.update();
    (dir, app)
}

fn act(app: &mut App, a: impl Into<Action>) {
    app.world_mut().resource_mut::<Actions>().push(a);
    app.update();
    app.update();
}

fn root(app: &mut App) -> Option<Entity> {
    let w = app.world_mut();
    let roots: Vec<Entity> = w.query_filtered::<Entity, With<MigrationRoot>>().iter(w).collect();
    assert!(roots.len() <= 1, "one modal at most: {roots:?}");
    roots.first().copied()
}

/// Every text under the modal, in tree order.
fn texts(app: &mut App) -> Vec<String> {
    fn walk(w: &World, e: Entity, out: &mut Vec<String>) {
        if let Some(t) = w.get::<Text>(e) {
            out.push(t.0.clone());
        }
        for &c in w.get::<Children>(e).into_iter().flatten() {
            walk(w, c, out);
        }
    }
    let root = root(app).expect("the modal");
    let mut out = Vec::new();
    walk(app.world(), root, &mut out);
    out
}

/// The modal's buttons: their actions, in tree order.
fn buttons(app: &mut App) -> Vec<(Entity, Action)> {
    let root = root(app).expect("the modal");
    let w = app.world_mut();
    let mut q = w.query::<(Entity, &OnActivate)>();
    let mut out: Vec<(Entity, Action)> = q.iter(w).map(|(e, a)| (e, a.0.clone())).collect();
    let w = app.world();
    out.retain(|(e, _)| {
        std::iter::successors(Some(*e), |x| w.get::<ChildOf>(*x).map(ChildOf::parent)).any(|x| x == root)
    });
    out
}

/// Presses the modal's button for `action` as a click does (`Activate`).
fn press(app: &mut App, action: MigrationAction) {
    let (button, _) = buttons(app)
        .into_iter()
        .find(|(_, a)| *a == Action::Migration(action))
        .unwrap_or_else(|| panic!("no {action:?} button"));
    app.world_mut().trigger(Activate { entity: button });
    app.update();
    app.update();
}

fn has(texts: &[String], needle: &str) -> bool {
    texts.iter().any(|t| t.contains(needle))
}

#[test]
fn migration_modal_follows_the_session() {
    let (dir, mut app) = app();
    assert!(root(&mut app).is_none(), "no modal without a session");
    act(&mut app, Action::OpenProject(dir.path().into()));
    assert!(app.world().resource::<Editor>().project.is_none());
    let t = texts(&mut app);
    for needle in [
        "Migrate project",
        "v1 -> v3",
        "v2: Rename format_version to project_version in project.ron",
        "project (1)",
        "quillshade/project.ron",
        // The Material migrator is registered (ADR 0047 §14): the v3 step upgrades the fixture's curve.
        "v3: Assign asset UUIDs and reference assets by UUID",
        "material (1)",
        "quillshade/material/x.curve.ron",
        "domain `light` is not compiled into this editor",
    ] {
        assert!(has(&t, needle), "{needle:?} in {t:#?}");
    }
    let actions: Vec<Action> = buttons(&mut app).into_iter().map(|(_, a)| a).collect();
    let expect = [MigrationAction::Cancel, MigrationAction::DryRun, MigrationAction::Run].map(Action::Migration);
    assert_eq!(actions, expect);

    press(&mut app, MigrationAction::DryRun);
    assert!(has(&texts(&mut app), "dry run: nothing is written"), "the log shows the dry run");
    let settings = std::fs::read_to_string(dir.path().join("quillshade/project.ron")).unwrap();
    assert!(settings.contains("format_version: 1"), "a dry run writes nothing: {settings}");

    press(&mut app, MigrationAction::Run);
    app.world_mut().resource_mut::<Editor>().wait_migration();
    app.update();
    let phase = app.world().resource::<Editor>().migration.as_ref().map(|s| s.phase.clone());
    assert_eq!(phase, Some(MigrationPhase::Done { written: 3 }), "project.ron twice, the curve once");
    let t = texts(&mut app);
    assert!(has(&t, "3 / 3 file") && has(&t, "migrated 3 file(s)"), "progress and result: {t:#?}");
    let actions: Vec<Action> = buttons(&mut app).into_iter().map(|(_, a)| a).collect();
    assert_eq!(actions, [Action::Migration(MigrationAction::Open)]);

    press(&mut app, MigrationAction::Open);
    assert!(root(&mut app).is_none(), "the modal goes");
    let editor = app.world().resource::<Editor>();
    assert!(editor.migration.is_none() && editor.project.is_some(), "the project opened normally");
}

/// Without a renderer nothing propagates visibility, and tab navigation skips hidden entities: show everything.
fn all_visible(mut q: Query<&mut InheritedVisibility>) {
    for mut v in &mut q {
        *v = InheritedVisibility::VISIBLE;
    }
}

/// Lets Feathers' modal focus its "last focusable child" (a delayed command, 0.1 s after the spawn) happen.
fn settle(app: &mut App) {
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(std::time::Duration::from_millis(50)));
    for _ in 0..6 {
        app.update();
    }
}

/// The focused entity and the modal's button for `action`.
fn focus_and(app: &mut App, action: MigrationAction) -> (Option<Entity>, Entity) {
    let button = buttons(app).into_iter().find(|(_, a)| *a == Action::Migration(action)).map(|(e, _)| e);
    (app.world().resource::<bevy::input_focus::InputFocus>().get(), button.expect("the button"))
}

/// Enter / Space press a focused Feathers button past the editor's hotkey gate: the run button must never get the
/// initial focus, neither when the modal opens nor when it respawns after a Dry run (Cancel has it).
#[test]
fn the_run_button_never_gets_the_initial_focus() {
    let (dir, mut app) = app();
    app.add_systems(PostUpdate, all_visible);
    act(&mut app, Action::OpenProject(dir.path().into()));
    settle(&mut app);
    let (focus, run) = focus_and(&mut app, MigrationAction::Run);
    assert_ne!(focus, Some(run), "Ready: Migrate is not focused");
    let (focus, cancel) = focus_and(&mut app, MigrationAction::Cancel);
    assert_eq!(focus, Some(cancel), "Ready: Cancel is");

    press(&mut app, MigrationAction::DryRun);
    settle(&mut app);
    assert!(has(&texts(&mut app), "dry run: nothing is written"), "the modal respawned with the dry run's log");
    let (focus, run) = focus_and(&mut app, MigrationAction::Run);
    assert_ne!(focus, Some(run), "after a dry run: Migrate is not focused");
    let (focus, cancel) = focus_and(&mut app, MigrationAction::Cancel);
    assert_eq!(focus, Some(cancel), "after a dry run: Cancel is");
}

#[test]
fn cancel_removes_the_modal() {
    let (dir, mut app) = app();
    act(&mut app, Action::OpenProject(dir.path().into()));
    assert!(root(&mut app).is_some());
    press(&mut app, MigrationAction::Cancel);
    assert!(root(&mut app).is_none(), "the modal goes");
    let editor = app.world().resource::<Editor>();
    assert!(editor.migration.is_none() && editor.project.is_none(), "no project is open");
}
