//! The Migration Tool session (ADR 0046 "Editor (Migration Tool modal)", D28): an older project opens into a
//! blocking session instead of the editor; dry run, run (background thread, channel), open and cancel.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use bevy::prelude::{App, Update};
use project_core::layout::{ProjectSettings, settings_text};
use project_core::{DomainId, ProjectVersion};
use project_migrate::{DomainMigrator, MigrationError, MigrationFacts, RawProject, StaleFile};

use super::*;
use crate::action::{Action, Dialog, MigrationAction, Unsaved};
use crate::config::UserConfig;
use crate::message::Level;
use crate::state::editor::Editor;
use crate::state::migration::MigrationPhase;

const LEGACY: &str = "ProjectSettings(\n    format_version: 1,\n)\n";

/// A game crate whose `project.ron` is `settings`, plus `files` (created with `()`).
fn game_with(settings: &str, files: &[&str]) -> tempfile::TempDir {
    let dir = game(files);
    std::fs::create_dir_all(dir.path().join("quillshade")).unwrap();
    std::fs::write(dir.path().join("quillshade/project.ron"), settings).unwrap();
    dir
}

fn settings_of(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("quillshade/project.ron")).unwrap()
}

/// An editor with a user config file in `dir` (so config writes are observable).
fn editor_with_config(domains: Arc<DomainList>, dir: &Path) -> (Editor, PathBuf) {
    let config = dir.join("config/config.ron");
    (Editor::new(domains).with_config(UserConfig::default(), Some(config.clone())), config)
}

fn root_of(dir: &Path) -> PathBuf {
    RawProject::open(dir).unwrap().root
}

#[test]
fn old_project_opens_the_migration_tool() {
    let dir = game_with(LEGACY, &[]);
    let cfg = tempfile::tempdir().unwrap();
    let (mut e, config) = editor_with_config(one(), cfg.path());
    e.apply(Action::OpenProject(dir.path().into()));
    assert!(e.project.is_none(), "an older project does not open");
    assert_eq!(e.dialog, None, "no error dialog");
    let s = e.migration.as_ref().expect("a migration session");
    assert_eq!(s.root, root_of(dir.path()));
    assert_eq!((s.from, s.to), (ProjectVersion(1), ProjectVersion::CURRENT));
    assert_eq!((s.plan.from, s.plan.to), (ProjectVersion(1), ProjectVersion::CURRENT));
    // Without domain migrators every step rewrites only `project.ron`.
    assert_eq!(s.plan.steps.len(), project_migrate::STEPS.len());
    assert_eq!(s.plan.steps[0].to, ProjectVersion(2));
    for step in &s.plan.steps {
        assert_eq!(step.stale, [StaleFile { domain: DomainId::PROJECT, path: "quillshade/project.ron".into() }]);
    }
    let by_domain = s.plan.stale_by_domain();
    assert_eq!(by_domain.keys().copied().collect::<Vec<_>>(), [DomainId::PROJECT]);
    assert_eq!(s.phase, MigrationPhase::Ready);
    assert!(s.log.is_empty() && !s.is_running());
    assert_eq!(settings_of(dir.path()), LEGACY, "nothing written");
    assert!(e.config.recent_projects.is_empty(), "not a recent project until it opens");
    assert!(!config.exists() || UserConfig::load(&config).recent_projects.is_empty());
    // The direct call (startup's path goes through `Action::OpenProject` as above) also starts it and is no error.
    let mut direct = Editor::new(one());
    assert_eq!(direct.open_project(dir.path()), Ok(()));
    assert!(direct.project.is_none() && direct.migration.is_some());
}

/// ADR 0046 "Missing `project.ron`": sources without `project.ron` are a v1 project. Opening it or "New project" on
/// it opens the Migration Tool like any older project and writes nothing (a `project.ron` written at open would stamp
/// it current); the run creates `project.ron` at the current version.
#[test]
fn legacy_layout_without_project_ron_opens_the_migration_tool() {
    let dir = game(&[]);
    std::fs::create_dir_all(dir.path().join("quillshade/other")).unwrap();
    std::fs::write(dir.path().join("quillshade/other/b.note.ron"), "()").unwrap();
    let project_ron = dir.path().join("quillshade/project.ron");
    for action in [Action::OpenProject(dir.path().to_path_buf()), Action::NewProject(dir.path().to_path_buf())] {
        let mut e = Editor::new(one());
        e.apply(action.clone());
        assert!(e.project.is_none() && e.dialog.is_none(), "{action:?}: {:?}", e.dialog);
        let s = e.migration.as_ref().expect("a migration session");
        assert_eq!((s.from, s.to), (ProjectVersion(1), ProjectVersion::CURRENT), "{action:?}");
        assert_eq!(
            s.plan.steps[0].stale,
            [StaleFile { domain: DomainId::PROJECT, path: "quillshade/project.ron".into() }]
        );
        assert!(!project_ron.exists(), "{action:?} wrote project.ron");
    }
    let mut e = Editor::new(one());
    e.apply(Action::OpenProject(dir.path().into()));
    e.apply(MigrationAction::Run);
    e.wait_migration();
    assert_eq!(e.migration.as_ref().unwrap().phase, MigrationPhase::Done { written: 2 }, "created, then bumped");
    assert_eq!(settings_of(dir.path()), settings_text(&ProjectSettings::default()).unwrap(), "at the current version");
}

/// ADR 0046 "Quit while a migration runs is deferred": Quit, ForceQuit and closing the window (`Action::Quit`) while
/// the run thread writes do not quit; the editor quits once the run reports its result.
#[test]
fn quit_while_running_waits_for_the_run() {
    let dir = game_with(LEGACY, &[]);
    let cfg = tempfile::tempdir().unwrap();
    let (mut e, config) = editor_with_config(one(), cfg.path());
    e.apply(Action::OpenProject(dir.path().into()));
    e.apply(MigrationAction::Run);
    for a in [Action::Quit, Action::ForceQuit, Action::Quit] {
        e.apply(a.clone());
        assert!(!e.quit, "{a:?} quit while the migration runs");
    }
    let s = e.migration.as_ref().unwrap();
    assert!(s.is_running() && s.quit_requested);
    let deferred = "quit requested: the editor quits once the migration finishes";
    assert_eq!(s.log.iter().filter(|l| *l == deferred).count(), 1, "logged once: {:?}", s.log);
    assert!(!config.exists(), "nothing saved yet");
    e.wait_migration();
    assert_eq!(e.migration.as_ref().unwrap().phase, MigrationPhase::Done { written: 2 });
    assert!(e.quit, "the deferred quit happens once the run ends");
    assert!(dir.path().join("crates/quillshade/src/lib.rs").is_file(), "after the run's regen, never before it");
    assert!(config.exists(), "the config is saved on the way out, as for a normal quit");
    assert_eq!(settings_of(dir.path()), settings_text(&ProjectSettings::default()).unwrap(), "the step completed");

    // A failed run ends the wait too.
    let dir = game_with(LEGACY, &["quillshade/other/a.note.ron"]);
    let mut e = Editor::new(list(vec![Box::new(Dummy::<0>) as Box<dyn EditorDomain>, Box::new(FailsInTheRunDomain)]));
    e.apply(Action::OpenProject(dir.path().into()));
    e.apply(MigrationAction::Run);
    e.apply(Action::Quit);
    assert!(!e.quit);
    e.wait_migration();
    assert!(matches!(e.migration.as_ref().unwrap().phase, MigrationPhase::Failed { .. }));
    assert!(e.quit, "quits after a failed run as well");
    assert_eq!(settings_of(dir.path()), LEGACY);

    // Outside `Running` (the plan or the result shown) quitting works at once.
    let dir = game_with(LEGACY, &[]);
    let mut e = Editor::new(one());
    e.apply(Action::OpenProject(dir.path().into()));
    e.apply(Action::Quit);
    assert!(e.quit, "Ready: quits at once");
    assert!(!e.migration.as_ref().unwrap().quit_requested);
}

#[test]
fn the_migration_tool_blocks_everything_else() {
    let dir = game_with(LEGACY, &[]);
    let mut e = Editor::new(one());
    e.apply(Action::OpenProject(dir.path().into()));
    let show_log = e.config.show_log;
    for a in [
        Action::OpenTool { domain: IDS[0], id: STATS, arg: None },
        Action::NewAssetDialog { kind: (IDS[0], NOTE), template: 0 },
        Action::OpenProjectDialog,
        Action::NewProjectDialog,
        Action::About,
        Action::QuickStart(0),
        Action::ToggleLog,
        Action::RegenAll,
        Action::CloseProject(Unsaved::Discard),
        Action::OpenProject(dir.path().into()),
    ] {
        e.apply(a.clone());
        assert_eq!(e.dialog, None, "{a:?} opened a dialog");
        assert!(e.tabs.is_empty() && e.project.is_none(), "{a:?} acted");
        assert!(!e.pick_folder, "{a:?}");
    }
    assert_eq!(e.config.show_log, show_log, "View toggles are ignored too");
    assert!(e.migration.as_ref().is_some_and(|s| s.log.is_empty()), "the session is untouched");
    // Quitting still works (nothing is dirty without a project).
    e.apply(Action::Quit);
    assert!(e.quit);
}

#[test]
fn dry_run_writes_nothing_and_logs_the_plan() {
    let dir = game_with(LEGACY, &[]);
    let mut e = Editor::new(one());
    e.apply(Action::OpenProject(dir.path().into()));
    e.apply(MigrationAction::DryRun);
    assert_eq!(settings_of(dir.path()), LEGACY);
    let s = e.migration.as_ref().unwrap();
    assert_eq!(s.phase, MigrationPhase::Ready);
    let title = |v| project_migrate::step_title(ProjectVersion(v));
    let expected = [
        "dry run: nothing is written".to_string(),
        "project v1 -> v3".into(),
        format!("step v2: {}", title(2)),
        "  project: 1 file(s)".into(),
        "    quillshade/project.ron".into(),
        format!("step v3: {}", title(3)),
        "  project: 1 file(s)".into(),
        "    quillshade/project.ron".into(),
    ];
    assert_eq!(s.log, expected);
    assert!(s.rev > 0, "the modal redraws");
    e.apply(MigrationAction::DryRun);
    assert_eq!(e.migration.as_ref().unwrap().log.len(), 2 * expected.len(), "each dry run appends");
}

/// A generator writing its module of the umbrella crate (`crates/quillshade/src/dummy/mod.rs`): what the run's regen
/// after the migration produces.
struct ModBackend;

impl project_core::regen::DomainBackend for ModBackend {
    fn id(&self) -> DomainId {
        IDS[0]
    }
    fn crate_contribution(&self) -> project_core::umbrella::CrateContribution {
        project_core::umbrella::CrateContribution {
            domain: IDS[0],
            plugin: "DummyPlugin",
            loader: None,
            bevy_features: &[],
            dependencies: &[],
        }
    }
    fn index(&self, _project: &project_core::Project) -> project_core::DomainIndex {
        project_core::DomainIndex::empty(IDS[0])
    }
    fn plan(
        &self,
        project: &project_core::Project,
        _index: &project_core::AssetIndex,
        _compile: bool,
    ) -> project_core::regen::DomainPlan {
        let module = project.module_dir(IDS[0]);
        let text = format!("{}\n", project_core::marker::header("//", None));
        let file = project_core::regen::OutFile { path: module.join("mod.rs"), bytes: text.into_bytes(), owner: None };
        project_core::regen::DomainPlan {
            diagnostics: Vec::new(),
            output: project_core::regen::DomainOutput {
                domain: IDS[0],
                files: vec![file],
                keep: Default::default(),
                roots: vec![project_core::regen::OutRoot { dir: module, binary: false }],
                orphan_scan: true,
            },
        }
    }
}

/// [`Dummy`] 0 with [`ModBackend`] as its generator.
struct GeneratedDomain;

impl EditorDomain for GeneratedDomain {
    fn id(&self) -> DomainId {
        IDS[0]
    }
    fn info(&self) -> DomainInfo {
        Dummy::<0>.info()
    }
    fn backend(&self) -> Option<Box<dyn project_core::regen::DomainBackend>> {
        Some(Box::new(ModBackend))
    }
    fn register(&self, r: &mut DomainRegistrar) {
        Dummy::<0>.register(r);
    }
}

/// ADR 0046 "then: full regen": the run itself regenerates every enabled domain (on its thread, before it reports
/// Done), so the generated files are current even if the editor quits at the result; Open then only opens.
#[test]
fn run_migrates_and_regenerates_then_open_opens() {
    let dir = game_with(LEGACY, &[]);
    let cfg = tempfile::tempdir().unwrap();
    let (mut e, config) = editor_with_config(list(vec![Box::new(GeneratedDomain)]), cfg.path());
    e.apply(Action::OpenProject(dir.path().into()));
    e.apply(MigrationAction::Run);
    assert_eq!(e.migration.as_ref().unwrap().phase, MigrationPhase::Running { done: 0, total: 2 });
    // While it runs: no cancel, no second run, no open.
    for a in [MigrationAction::Cancel, MigrationAction::Run, MigrationAction::Open, MigrationAction::DryRun] {
        e.apply(a);
        assert!(e.migration.as_ref().is_some_and(|s| s.is_running()), "{a:?}");
    }
    e.wait_migration();
    let s = e.migration.as_ref().unwrap();
    assert_eq!(s.phase, MigrationPhase::Done { written: 2 });
    let title = |v| project_migrate::step_title(ProjectVersion(v));
    let expected = [
        "project v1 -> v3".to_string(),
        format!("step v2: {}", title(2)),
        "  migrate project quillshade/project.ron".into(),
        format!("note: {}", project_migrate::PROJECT_RON_REWRITTEN),
        format!("step v3: {}", title(3)),
        "  migrate project quillshade/project.ron".into(),
        "migrated 2 file(s); the project is at v3".into(),
        "regenerate every domain".into(),
    ];
    assert_eq!(s.log[..expected.len()], expected, "{:#?}", s.log);
    for line in ["  create crates/quillshade/src/dummy/mod.rs", "  create crates/quillshade/src/lib.rs"] {
        assert!(s.log.contains(&line.to_string()), "{line:?} in {:#?}", s.log);
    }
    assert!(s.log.last().is_some_and(|l| l.starts_with("regenerated: ") && l.contains("files changed")), "{:?}", s.log);
    assert_eq!(settings_of(dir.path()), settings_text(&ProjectSettings::default()).unwrap(), "canonical v3");
    // Regenerated without Open: quitting here leaves no stale generated file behind.
    let module = dir.path().join("crates/quillshade/src/dummy/mod.rs");
    assert!(module.is_file() && dir.path().join("crates/quillshade/src/lib.rs").is_file(), "regenerated by the run");
    assert!(e.messages.iter().any(|m| m.text.ends_with("every domain regenerated")), "the console/logs say so");
    assert!(e.project.is_none(), "the project opens only on Open");
    assert!(e.config.recent_projects.is_empty());
    let before = std::fs::metadata(&module).unwrap().modified().unwrap();
    e.apply(MigrationAction::Open);
    assert!(e.migration.is_none(), "the session is gone");
    let p = e.project.as_ref().expect("the project opens normally");
    assert_eq!(p.root, root_of(dir.path()));
    assert_eq!(e.messages.iter().filter(|m| m.text == "generated").count(), 0, "Open does not regenerate again");
    assert_eq!(std::fs::metadata(&module).unwrap().modified().unwrap(), before);
    assert_eq!(e.config.recent_projects, std::slice::from_ref(&p.root), "a recent project once open");
    assert_eq!(UserConfig::load(&config).recent_projects, std::slice::from_ref(&p.root));
    // The rest of the editor works again.
    e.apply(Action::About);
    assert_eq!(e.dialog, Some(Dialog::About));
}

#[test]
fn cancel_leaves_no_project_and_files_untouched() {
    let dir = game_with(LEGACY, &[]);
    let cfg = tempfile::tempdir().unwrap();
    let (mut e, config) = editor_with_config(one(), cfg.path());
    e.config.recent_projects = vec![root_of(dir.path())];
    let before = e.config.clone();
    e.apply(Action::OpenProject(dir.path().into()));
    e.apply(MigrationAction::DryRun);
    e.apply(MigrationAction::Open);
    assert!(e.migration.is_some(), "Open needs a finished run");
    e.apply(MigrationAction::Cancel);
    assert!(e.migration.is_none() && e.project.is_none() && e.dialog.is_none());
    assert_eq!(settings_of(dir.path()), LEGACY);
    assert_eq!(e.config, before, "the user config is unchanged");
    assert!(!config.exists(), "and not written");
    e.apply(Action::About);
    assert_eq!(e.dialog, Some(Dialog::About), "the editor is available again");
}

#[test]
fn too_new_project_shows_the_error() {
    let dir = game_with("ProjectSettings(project_version: 4)\n", &[]);
    let mut e = Editor::new(one());
    e.apply(Action::OpenProject(dir.path().into()));
    assert!(e.migration.is_none() && e.project.is_none());
    let Some(Dialog::Message { message, .. }) = &e.dialog else { panic!("{:?}", e.dialog) };
    assert_eq!(
        message,
        "Cannot open project: project version v4 is newer than this editor supports (v3); update the editor"
    );
}

#[test]
fn switching_to_an_old_project_closes_the_open_one() {
    let current = game(&["quillshade/dummy/a.note.ron"]);
    let old = game_with(LEGACY, &[]);
    let mut e = open(one(), current.path());
    let current_root = e.project.as_ref().unwrap().root.clone();
    e.apply(Action::OpenAsset(note(0, "a")));
    assert_eq!(e.tabs.len(), 1);
    e.apply(Action::OpenProjectDialog);
    e.apply(Action::OpenProject(old.path().into()));
    assert!(e.project.is_none() && e.tabs.is_empty(), "the open project closed");
    assert_eq!(e.dialog, None, "the Open dialog closed, no error");
    assert!(e.migration.as_ref().is_some_and(|s| s.root == root_of(old.path())));
    assert_eq!(e.config.recent_projects, std::slice::from_ref(&current_root), "the old project is not recent yet");
    let saved = e.config.tabs.get(&current_root).expect("the closed project's tabs are remembered");
    assert_eq!(saved.open, ["dummy:quillshade/dummy/a.note.ron"]);
    // New project on an existing older game crate goes the same way.
    let mut fresh = Editor::new(one());
    fresh.apply(Action::NewProject(old.path().into()));
    assert!(fresh.migration.is_some() && fresh.dialog.is_none());
    assert_eq!(settings_of(old.path()), LEGACY);
}

/// D28 "Opening a project older than the editor blocks work and opens the Migration Tool", for every GUI entry point:
/// the startup argument, the most recent project at startup, a recent project on the welcome screen
/// (`Action::OpenProject`), the Open dialog (typed path and folder picker) and New Project (typed path and folder
/// picker) on an older game crate. None of them opens the project, shows an error or writes a file.
#[test]
fn every_gui_entry_point_starts_the_migration_tool() {
    let old = game_with(LEGACY, &[]);
    let path = old.path().to_path_buf();
    let started = |e: &Editor, how: &str| {
        assert!(e.project.is_none(), "{how}: the project does not open");
        assert_eq!(e.dialog, None, "{how}: no error dialog");
        assert!(e.migration.as_ref().is_some_and(|s| s.root == root_of(&path)), "{how}: a migration session");
        assert_eq!(settings_of(&path), LEGACY, "{how}: nothing written");
    };
    // Startup: the CLI argument, else the most recent project (`open_startup_project` + `apply_actions`).
    for (arg, recent) in [(Some(path.clone()), vec![]), (None, vec![path.clone()])] {
        let config = UserConfig { recent_projects: recent, ..UserConfig::default() };
        let mut app = App::new();
        app.insert_resource(Editor::new(one()).with_config(config, None))
            .insert_resource(crate::action::Actions::default())
            .insert_resource(crate::app::StartupProject(arg))
            .add_systems(Update, (crate::app::open_startup_project, crate::app::apply_actions).chain());
        app.update();
        started(app.world().resource::<Editor>(), "startup");
    }
    // A recent project on the welcome screen.
    let mut e = Editor::new(one());
    e.apply(Action::OpenProject(path.clone()));
    started(&e, "recent project");
    // The Open and New Project dialogs, by typed path and by the folder picker.
    for (dialog, answer) in [
        (Action::OpenProjectDialog, Action::Submit(path.display().to_string())),
        (Action::OpenProjectDialog, Action::FolderPicked(path.clone())),
        (Action::NewProjectDialog, Action::Submit(path.display().to_string())),
        (Action::NewProjectDialog, Action::FolderPicked(path.clone())),
    ] {
        let how = format!("{dialog:?} + {answer:?}");
        let mut e = Editor::new(one());
        e.apply(dialog);
        e.apply(answer);
        started(&e, &how);
    }
}

/// The app's per-frame system drains the background run into the session.
#[test]
fn the_app_polls_the_running_migration() {
    let dir = game_with(LEGACY, &[]);
    let mut e = Editor::new(one());
    e.apply(Action::OpenProject(dir.path().into()));
    e.apply(MigrationAction::Run);
    let mut app = App::new();
    app.insert_resource(e).add_systems(Update, crate::app::poll_migration);
    let start = std::time::Instant::now();
    while app.world().resource::<Editor>().migration.as_ref().is_some_and(|s| s.is_running()) {
        assert!(start.elapsed().as_secs() < 30, "the migration never finished");
        app.update();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let e = app.world().resource::<Editor>();
    assert_eq!(e.migration.as_ref().unwrap().phase, MigrationPhase::Done { written: 2 });
    assert!(e.messages.iter().any(|m| m.text.starts_with("Migrated ")), "the result reaches the console/logs");
}

/// Calls of [`Broken::migrate`] with `FAIL_FROM > 0` (only one test uses that variant).
static MIGRATE_CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// A domain whose one file is stale until it reads `migrated`. Its `migrate` fails from its `FAIL_FROM`th call on (`0`: always): the plan
/// at open and the plan before the run are calls 1 and 2, so `3` fails only in the real run.
struct Broken<const FAIL_FROM: usize>;

impl<const FAIL_FROM: usize> DomainMigrator for Broken<FAIL_FROM> {
    fn domain(&self) -> DomainId {
        IDS[1]
    }
    fn files(&self, project: &RawProject) -> Vec<PathBuf> {
        vec![project.domain_dir(IDS[1]).join("a.note.ron")]
    }
    fn is_fresh(&self, _file: &Path, text: &str, _target: ProjectVersion) -> Result<bool, MigrationError> {
        Ok(text == "migrated")
    }
    fn migrate(
        &self,
        file: &Path,
        _text: &str,
        _target: ProjectVersion,
        _facts: &MigrationFacts,
    ) -> Result<String, MigrationError> {
        let call = if FAIL_FROM == 0 { 0 } else { MIGRATE_CALLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1 };
        if call >= FAIL_FROM {
            return Err(MigrationError::domain(IDS[1], file, "unsupported note"));
        }
        Ok("migrated".into())
    }
}

struct BrokenDomain<const FAIL_FROM: usize>;

impl<const FAIL_FROM: usize> EditorDomain for BrokenDomain<FAIL_FROM> {
    fn id(&self) -> DomainId {
        IDS[1]
    }
    fn info(&self) -> DomainInfo {
        Dummy::<1>.info()
    }
    fn backend(&self) -> Option<Box<dyn project_core::regen::DomainBackend>> {
        None
    }
    fn migrator(&self) -> Option<Box<dyn DomainMigrator>> {
        Some(Box::new(Broken::<FAIL_FROM>))
    }
    fn register(&self, r: &mut DomainRegistrar) {
        Dummy::<1>.register(r);
    }
}

/// A plan that fails at open shows the usual error dialog; no session starts and the open project stays.
#[test]
fn a_failing_plan_shows_the_error() {
    let current = game(&[]);
    let old = game_with(LEGACY, &["quillshade/other/a.note.ron"]);
    let domains = list(vec![Box::new(Dummy::<0>) as Box<dyn EditorDomain>, Box::new(BrokenDomain::<0>)]);
    let mut e = open(domains, current.path());
    e.apply(Action::OpenProject(old.path().into()));
    assert!(e.migration.is_none());
    assert!(e.project.as_ref().is_some_and(|p| p.root == root_of(current.path())), "the open project stays");
    let Some(Dialog::Message { message, .. }) = &e.dialog else { panic!("{:?}", e.dialog) };
    assert!(message.starts_with("Cannot open project: cannot migrate `") && message.contains("unsupported note"));
    assert_eq!(settings_of(old.path()), LEGACY);
}

#[test]
fn a_failed_run_keeps_the_session_and_logs_the_error() {
    let dir = game_with(LEGACY, &["quillshade/other/a.note.ron"]);
    let mut e = Editor::new(list(vec![Box::new(Dummy::<0>) as Box<dyn EditorDomain>, Box::new(BrokenDomain::<3>)]));
    e.apply(Action::OpenProject(dir.path().into()));
    let s = e.migration.as_ref().unwrap();
    let domains: Vec<DomainId> = s.plan.steps[0].stale.iter().map(|f| f.domain).collect();
    assert_eq!(domains, [IDS[1], DomainId::PROJECT], "the domain's files first, project.ron last");
    e.apply(MigrationAction::Run);
    assert!(e.migration.as_ref().unwrap().is_running(), "the plan before the run worked");
    e.wait_migration();
    let s = e.migration.as_ref().unwrap();
    let MigrationPhase::Failed { message } = &s.phase else { panic!("{:?}", s.phase) };
    assert!(message.contains("unsupported note"), "{message}");
    assert!(s.log.iter().any(|l| l.starts_with("step v2: ")), "{:?}", s.log);
    assert!(s.log.last().is_some_and(|l| l.starts_with("error: ") && l.contains("unsupported note")), "{:?}", s.log);
    assert_eq!(settings_of(dir.path()), LEGACY, "project.ron is not bumped: a rerun resumes");
    assert!(e.messages.iter().any(|m| m.level == Level::Error && m.text.contains("unsupported note")));
    e.apply(MigrationAction::Open);
    assert!(e.migration.is_some() && e.project.is_none(), "no Open after a failure");
    // A rerun is allowed after a failure (it fails again here: every later call fails).
    e.apply(MigrationAction::Run);
    e.wait_migration();
    assert!(matches!(e.migration.as_ref().unwrap().phase, MigrationPhase::Failed { .. }));
    e.apply(MigrationAction::Cancel);
    assert!(e.migration.is_none() && e.project.is_none());
}

/// A domain whose one file migrates in both plans (at open and before the run) and fails in the real run: its own
/// call counter (not [`MIGRATE_CALLS`], which one other test owns).
#[derive(Default)]
struct FailsInTheRun(std::sync::atomic::AtomicUsize);

impl DomainMigrator for FailsInTheRun {
    fn domain(&self) -> DomainId {
        IDS[1]
    }
    fn files(&self, project: &RawProject) -> Vec<PathBuf> {
        vec![project.domain_dir(IDS[1]).join("a.note.ron")]
    }
    fn is_fresh(&self, _file: &Path, text: &str, _target: ProjectVersion) -> Result<bool, MigrationError> {
        Ok(text == "migrated")
    }
    fn migrate(
        &self,
        file: &Path,
        _text: &str,
        _target: ProjectVersion,
        _facts: &MigrationFacts,
    ) -> Result<String, MigrationError> {
        if self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst) >= 2 {
            return Err(MigrationError::domain(IDS[1], file, "unsupported note"));
        }
        Ok("migrated".into())
    }
}

struct FailsInTheRunDomain;

impl EditorDomain for FailsInTheRunDomain {
    fn id(&self) -> DomainId {
        IDS[1]
    }
    fn info(&self) -> DomainInfo {
        Dummy::<1>.info()
    }
    fn backend(&self) -> Option<Box<dyn project_core::regen::DomainBackend>> {
        None
    }
    fn migrator(&self) -> Option<Box<dyn DomainMigrator>> {
        Some(Box::new(FailsInTheRun::default()))
    }
    fn register(&self, r: &mut DomainRegistrar) {
        Dummy::<1>.register(r);
    }
}
