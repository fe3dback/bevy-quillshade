//! Orchestrator tests (techdoc §4.4 "Tests", ADR 0046 "Orchestrator (D39)"): the full chain v1 -> CURRENT on an old
//! fixture, the per-step barrier on a synthetic multi-step chain, interruption + rerun = clean run, a second run
//! changes nothing, the dry run simulates every step and writes nothing, and the error / warning cases.

mod common;

use common::*;
use project_core::{OpenError, Project};
use project_migrate::{
    DomainId, DomainMigrator, GENERIC_STEP_TITLE, MigrationError, MigrationEvent, Migrator, PROJECT_RON_REWRITTEN,
    PlannedStep, ProjectVersion, RawProject, STEPS, StaleFile, step_title,
};

const MATERIAL: DomainId = DomainId::new("material");
const LIGHT: DomainId = DomainId::new("light");
const COMPILED: &[DomainId] = &[MATERIAL, LIGHT];

const FIXTURE_V3: &str = "ProjectSettings(\n  project_version: 3,\n  header_date: false,\n  domains: {\n    \
                          \"light\": DomainEntry(\n      enabled: false,\n    ),\n  },\n)\n";

fn stale(domain: DomainId, path: &str) -> StaleFile {
    StaleFile { domain, path: path.to_string() }
}

fn project_ron() -> StaleFile {
    stale(DomainId::PROJECT, "quillshade/project.ron")
}

fn markers(names: &[&str]) -> Vec<StaleFile> {
    names.iter().map(|n| stale(MARKER, &format!("quillshade/marker/{n}"))).collect()
}

fn probe(root: &std::path::Path) -> ProjectVersion {
    project_core::probe_project_version(&read(root, "quillshade/project.ron")).unwrap()
}

/// A clean run of the marker chain to `target` on a fresh copy; returns the resulting snapshot.
fn clean_run(initial: &[(&str, u32)], target: u32) -> std::collections::BTreeMap<String, Vec<u8>> {
    let dir = v1_with_markers(initial);
    let marker = MarkerMigrator::default();
    let migrators: [&dyn DomainMigrator; 1] = [&marker];
    let mut migrator =
        Migrator::new(RawProject::open(dir.path()).unwrap(), &migrators, COMPILED).with_target(ProjectVersion(target));
    migrator.run(&mut |_| {}).unwrap();
    snapshot(dir.path())
}

#[test]
fn full_chain_v1_to_current_on_the_old_fixture() {
    let dir = v1_copy();
    let before = snapshot(dir.path());
    let raw = RawProject::open(dir.path()).unwrap();
    assert_eq!(raw.version, ProjectVersion::FIRST);
    assert!(matches!(Project::open(dir.path()), Err(OpenError::NeedsMigration { .. })));

    let mut migrator = Migrator::new(raw, &[], COMPILED);
    let plan = migrator.plan().unwrap();
    assert_eq!((plan.from, plan.to), (ProjectVersion::FIRST, ProjectVersion::CURRENT));
    assert_eq!(plan.steps.len(), STEPS.len());
    // Without domain migrators every step rewrites only `project.ron`.
    for (planned, step) in plan.steps.iter().zip(STEPS) {
        assert_eq!(*planned, PlannedStep { to: step.to, title: step.title, stale: vec![project_ron()] });
    }
    assert!(plan.warnings.is_empty(), "{:?}", plan.warnings);

    let mut events = Vec::new();
    let report = migrator.run(&mut record(&mut events)).unwrap();
    assert_eq!(report.plan, plan);
    assert_eq!(report.written, vec![project_ron(); STEPS.len()]);
    assert_eq!(
        events,
        [
            MigrationEvent::StepStarted { to: ProjectVersion(2), title: STEPS[0].title, stale: vec![project_ron()] },
            MigrationEvent::FileMigrated { domain: DomainId::PROJECT, path: "quillshade/project.ron".into() },
            MigrationEvent::Note(PROJECT_RON_REWRITTEN.into()),
            MigrationEvent::StepFinished { to: ProjectVersion(2) },
            MigrationEvent::StepStarted { to: ProjectVersion(3), title: STEPS[1].title, stale: vec![project_ron()] },
            MigrationEvent::FileMigrated { domain: DomainId::PROJECT, path: "quillshade/project.ron".into() },
            MigrationEvent::StepFinished { to: ProjectVersion(3) },
        ]
    );
    assert_eq!(migrator.project().version, ProjectVersion::CURRENT);

    // Golden project.ron; the domain files no registered migrator owns stay byte-identical.
    assert_eq!(read(dir.path(), "quillshade/project.ron"), FIXTURE_V3);
    let after = snapshot(dir.path());
    for (rel, bytes) in &before {
        if rel != "quillshade/project.ron" {
            assert_eq!(after.get(rel), Some(bytes), "{rel} changed");
        }
    }
    assert_eq!(after.len(), before.len(), "no file added or removed (no temp file left)");

    // The migrated project opens normally.
    let project = Project::open(dir.path()).unwrap();
    assert_eq!(project.settings.project_version, ProjectVersion::CURRENT);
    assert!(!project.domain_enabled(LIGHT) && project.domain_enabled(MATERIAL));
}

#[test]
fn multi_step_barrier_with_a_synthetic_domain() {
    let dir = v1_with_markers(&[("a.ron", 1), ("b.ron", 1), ("c.ron", 1)]);
    let original = snapshot(dir.path());
    let marker = MarkerMigrator::default();
    let migrators: [&dyn DomainMigrator; 1] = [&marker];
    let mut migrator =
        Migrator::new(RawProject::open(dir.path()).unwrap(), &migrators, COMPILED).with_target(ProjectVersion(4));
    let mut events = Vec::new();
    let report = migrator.run(&mut record(&mut events)).unwrap();

    // Step v sees every file still at v - 1 (collect and the published facts), then bumps each by one.
    let mut expected_log = Vec::new();
    for v in 2..=4 {
        for name in ["a.ron", "b.ron", "c.ron"] {
            expected_log.push(format!("collect v{v} {name} {}", v - 1));
        }
        for name in ["a.ron", "b.ron", "c.ron"] {
            let p = v - 1;
            expected_log.push(format!("migrate v{v} {name} facts=a.ron:{p},b.ron:{p},c.ron:{p}"));
        }
    }
    assert_eq!(marker.log(), expected_log);

    // Per step: the marker files (sorted), then project.ron as the commit point.
    let mut expected_events = Vec::new();
    for v in 2..=4 {
        let to = ProjectVersion(v);
        let title = step_title(to);
        let mut files = markers(&["a.ron", "b.ron", "c.ron"]);
        files.push(project_ron());
        expected_events.push(MigrationEvent::StepStarted { to, title, stale: files.clone() });
        for f in files {
            expected_events.push(MigrationEvent::FileMigrated { domain: f.domain, path: f.path });
        }
        // The hand-written `project.ron` is noted once, at its first rewrite.
        if v == 2 {
            expected_events.push(MigrationEvent::Note(PROJECT_RON_REWRITTEN.into()));
        }
        expected_events.push(MigrationEvent::StepFinished { to });
    }
    assert_eq!(events, expected_events);
    assert_eq!(report.written.len(), 12);

    for name in ["a.ron", "b.ron", "c.ron"] {
        assert_eq!(read(dir.path(), &format!("quillshade/marker/{name}")), marker_text(4));
    }
    assert_eq!(probe(dir.path()), ProjectVersion(4));
    assert_eq!(
        read(dir.path(), "quillshade/project.ron"),
        FIXTURE_V3.replace("project_version: 3", "project_version: 4")
    );
    assert_eq!(migrator.project().version, ProjectVersion(4));
    let after = snapshot(dir.path());
    for rel in ["quillshade/material/x.curve.ron", "quillshade/light/y.curve.ron", "Cargo.toml"] {
        assert_eq!(after[rel], original[rel], "{rel}");
    }
}

#[test]
fn interruption_and_rerun_equal_a_clean_run() {
    let initial = [("a.ron", 1), ("b.ron", 1), ("c.ron", 1), ("d.ron", 1)];
    let clean = clean_run(&initial, 4);
    let original = snapshot(v1_with_markers(&initial).path());

    for (step, nth) in [(2, 0), (3, 1), (4, 3)] {
        let dir = v1_with_markers(&initial);
        let failing = MarkerMigrator { fail_migrate: Some((ProjectVersion(step), nth)), ..Default::default() };
        let migrators: [&dyn DomainMigrator; 1] = [&failing];
        let mut migrator =
            Migrator::new(RawProject::open(dir.path()).unwrap(), &migrators, COMPILED).with_target(ProjectVersion(4));
        let err = migrator.run(&mut |_| {}).unwrap_err();
        assert!(matches!(&err, MigrationError::Domain { domain, .. } if *domain == MARKER), "{err}");
        assert!(err.to_string().contains("injected migrate failure"), "{err}");

        // project.ron is not bumped for the failed step; the files before the failing one are migrated.
        let committed = ProjectVersion(step - 1);
        assert_eq!(migrator.project().version, committed);
        assert_eq!(RawProject::open(dir.path()).unwrap().version, committed);
        for (i, name) in ["a.ron", "b.ron", "c.ron", "d.ron"].into_iter().enumerate() {
            let expected = if i < nth { step } else { step - 1 };
            assert_eq!(read(dir.path(), &format!("quillshade/marker/{name}")), marker_text(expected), "{name}");
        }
        if step == 2 {
            assert_eq!(snapshot(dir.path()), original, "a failure on the first write leaves everything as it was");
        }

        // Rerun: resumes the failed step (already migrated files are fresh) and ends like a clean run.
        let marker = MarkerMigrator::default();
        let migrators: [&dyn DomainMigrator; 1] = [&marker];
        let mut rerun =
            Migrator::new(RawProject::open(dir.path()).unwrap(), &migrators, COMPILED).with_target(ProjectVersion(4));
        let plan = rerun.plan().unwrap();
        assert_eq!(plan.from, committed);
        let names = ["a.ron", "b.ron", "c.ron", "d.ron"];
        let mut resumed = markers(&names[nth..]);
        resumed.push(project_ron());
        assert_eq!(plan.steps[0].stale, resumed, "step {step}");
        rerun.run(&mut |_| {}).unwrap();
        assert_eq!(snapshot(dir.path()), clean, "failure at step {step} file {nth}");
    }
}

#[test]
fn a_failing_freshness_check_aborts_before_any_write_of_the_step() {
    let initial = [("a.ron", 1), ("b.ron", 1), ("c.ron", 1)];
    let at_v2 = clean_run(&initial, 2);
    let dir = v1_with_markers(&initial);
    let failing = MarkerMigrator { fail_fresh: Some((ProjectVersion(3), "c.ron")), ..Default::default() };
    let migrators: [&dyn DomainMigrator; 1] = [&failing];
    let mut migrator =
        Migrator::new(RawProject::open(dir.path()).unwrap(), &migrators, COMPILED).with_target(ProjectVersion(4));
    let mut events = Vec::new();
    let err = migrator.run(&mut record(&mut events)).unwrap_err();
    assert!(err.to_string().contains("injected is_fresh failure") && err.to_string().contains("c.ron"), "{err}");
    assert_eq!(snapshot(dir.path()), at_v2, "step v3 wrote nothing");
    assert!(!failing.log().iter().any(|l| l.starts_with("migrate v3")));
    assert_eq!(events.last(), Some(&MigrationEvent::StepFinished { to: ProjectVersion(2) }));
}

#[test]
fn a_second_run_changes_nothing() {
    // The production chain (no synthetic target).
    let dir = v1_copy();
    Migrator::new(RawProject::open(dir.path()).unwrap(), &[], COMPILED).run(&mut |_| {}).unwrap();
    let (bytes, times) = (snapshot(dir.path()), mtimes(dir.path()));
    let mut again = Migrator::new(RawProject::open(dir.path()).unwrap(), &[], COMPILED);
    let plan = again.plan().unwrap();
    assert!(plan.is_up_to_date() && plan.total_stale() == 0 && plan.warnings.is_empty(), "{plan:?}");
    assert_eq!((plan.from, plan.to), (ProjectVersion::CURRENT, ProjectVersion::CURRENT));
    let mut events = Vec::new();
    let report = again.run(&mut record(&mut events)).unwrap();
    assert!(events.is_empty() && report.written.is_empty() && report.plan.is_up_to_date());
    assert_eq!(snapshot(dir.path()), bytes);
    assert_eq!(mtimes(dir.path()), times);

    // The synthetic chain.
    let dir = v1_with_markers(&[("a.ron", 1), ("b.ron", 1)]);
    let marker = MarkerMigrator::default();
    let migrators: [&dyn DomainMigrator; 1] = [&marker];
    let target = ProjectVersion(4);
    Migrator::new(RawProject::open(dir.path()).unwrap(), &migrators, COMPILED)
        .with_target(target)
        .run(&mut |_| {})
        .unwrap();
    let (bytes, times) = (snapshot(dir.path()), mtimes(dir.path()));
    let mut again = Migrator::new(RawProject::open(dir.path()).unwrap(), &migrators, COMPILED).with_target(target);
    let mut events = Vec::new();
    let report = again.run(&mut record(&mut events)).unwrap();
    assert!(events.is_empty() && report.written.is_empty() && report.plan.is_up_to_date());
    assert_eq!(snapshot(dir.path()), bytes);
    assert_eq!(mtimes(dir.path()), times);
}

#[test]
fn dry_run_simulates_every_step_and_writes_nothing() {
    // `c.ron` is already ahead: fresh at v2 and v3, stale only at v4.
    let dir = v1_with_markers(&[("a.ron", 1), ("b.ron", 1), ("c.ron", 3)]);
    let (bytes, times) = (snapshot(dir.path()), mtimes(dir.path()));
    let marker = MarkerMigrator::default();
    let migrators: [&dyn DomainMigrator; 1] = [&marker];
    let mut migrator =
        Migrator::new(RawProject::open(dir.path()).unwrap(), &migrators, COMPILED).with_target(ProjectVersion(4));
    let plan = migrator.plan().unwrap();
    assert_eq!(snapshot(dir.path()), bytes);
    assert_eq!(mtimes(dir.path()), times);
    assert_eq!(migrator.project().version, ProjectVersion::FIRST, "a dry run commits nothing");

    let with_project = |names: &[&str]| {
        let mut files = markers(names);
        files.push(project_ron());
        files
    };
    let expected = vec![
        PlannedStep { to: ProjectVersion(2), title: STEPS[0].title, stale: with_project(&["a.ron", "b.ron"]) },
        PlannedStep { to: ProjectVersion(3), title: STEPS[1].title, stale: with_project(&["a.ron", "b.ron"]) },
        PlannedStep {
            to: ProjectVersion(4),
            title: GENERIC_STEP_TITLE,
            stale: with_project(&["a.ron", "b.ron", "c.ron"]),
        },
    ];
    assert_eq!(plan.steps, expected);
    assert_eq!((plan.from, plan.to), (ProjectVersion(1), ProjectVersion(4)));
    assert!(!plan.is_up_to_date());
    assert_eq!(plan.total_stale(), 10);
    let grouped = plan.stale_by_domain();
    assert_eq!(grouped.keys().copied().collect::<Vec<_>>(), [MARKER, DomainId::PROJECT]);
    assert_eq!(
        grouped[&MARKER].iter().copied().collect::<Vec<_>>(),
        ["quillshade/marker/a.ron", "quillshade/marker/b.ron", "quillshade/marker/c.ron"]
    );
    let by_domain = plan.steps[0].by_domain();
    assert_eq!(by_domain[&DomainId::PROJECT], ["quillshade/project.ron"]);
    assert_eq!(by_domain[&MARKER].len(), 2);

    // The real run executes exactly the planned steps.
    let report = migrator.run(&mut |_| {}).unwrap();
    assert_eq!(report.plan, plan);
    assert_eq!(read(dir.path(), "quillshade/marker/c.ron"), marker_text(4));
}

#[test]
fn too_new_projects_are_refused() {
    let dir = v1_copy();
    std::fs::write(dir.path().join("quillshade/project.ron"), "ProjectSettings(project_version: 4)\n").unwrap();
    let bytes = snapshot(dir.path());
    let raw = RawProject::open(dir.path()).unwrap();
    assert_eq!(raw.version, ProjectVersion(4));
    let mut migrator = Migrator::new(raw, &[], COMPILED);
    let err = migrator.plan().unwrap_err();
    assert!(
        matches!(err, MigrationError::TooNew { found: ProjectVersion(4), supported: ProjectVersion::CURRENT }),
        "{err}"
    );
    assert_eq!(err.to_string(), "project version v4 is newer than this editor supports (v3); update the editor");
    assert!(matches!(migrator.run(&mut |_| {}), Err(MigrationError::TooNew { .. })));
    assert_eq!(snapshot(dir.path()), bytes);
}

#[test]
fn not_a_project() {
    let dir = tempfile::tempdir().unwrap();
    let err = RawProject::open(dir.path()).unwrap_err();
    assert!(matches!(err, MigrationError::Open(OpenError::NoCargoToml(_))), "{err}");
    assert!(err.to_string().contains("not a project"), "{err}");
}

/// ADR 0046 "Missing `project.ron`": sources without `project.ron` are v1; the project migrator creates the file
/// (the dry run only lists it), and a second run is a no-op. An empty `quillshade/` is a new project: nothing to do.
#[test]
fn legacy_project_without_project_ron_migrates_and_is_idempotent() {
    let dir = v1_copy();
    std::fs::remove_file(dir.path().join("quillshade/project.ron")).unwrap();
    let before = snapshot(dir.path());
    let raw = RawProject::open(dir.path()).unwrap();
    assert_eq!(raw.version, ProjectVersion::FIRST);
    assert!(matches!(Project::open(dir.path()), Err(OpenError::NeedsMigration { from: ProjectVersion::FIRST, .. })));

    let mut migrator = Migrator::new(raw, &[], COMPILED);
    let plan = migrator.plan().unwrap();
    assert_eq!(plan.steps[0], PlannedStep { to: ProjectVersion(2), title: STEPS[0].title, stale: vec![project_ron()] });
    assert_eq!(snapshot(dir.path()), before, "the dry run writes nothing");

    let mut events = Vec::new();
    let report = migrator.run(&mut record(&mut events)).unwrap();
    assert_eq!(report.plan, plan);
    assert_eq!(report.written, vec![project_ron(); STEPS.len()], "created, then bumped");
    assert!(!events.iter().any(|e| matches!(e, MigrationEvent::Note(_))), "a created file loses nothing: {events:?}");
    assert_eq!(
        read(dir.path(), "quillshade/project.ron"),
        "ProjectSettings(\n  project_version: 3,\n  header_date: false,\n)\n"
    );
    assert_eq!(Project::open(dir.path()).unwrap().settings.project_version, ProjectVersion::CURRENT);

    let (bytes, times) = (snapshot(dir.path()), mtimes(dir.path()));
    let mut again = Migrator::new(RawProject::open(dir.path()).unwrap(), &[], COMPILED);
    let mut events = Vec::new();
    let report = again.run(&mut record(&mut events)).unwrap();
    assert!(events.is_empty() && report.written.is_empty() && report.plan.is_up_to_date());
    assert_eq!((snapshot(dir.path()), mtimes(dir.path())), (bytes, times));

    // A synthetic chain creates the file in the first step and bumps it in the next ones.
    let dir = v1_with_markers(&[("a.ron", 1)]);
    std::fs::remove_file(dir.path().join("quillshade/project.ron")).unwrap();
    let marker = MarkerMigrator::default();
    let migrators: [&dyn DomainMigrator; 1] = [&marker];
    let mut migrator =
        Migrator::new(RawProject::open(dir.path()).unwrap(), &migrators, COMPILED).with_target(ProjectVersion(4));
    let plan = migrator.plan().unwrap();
    assert!(plan.steps.iter().all(|s| s.stale.last() == Some(&project_ron())), "{plan:?}");
    migrator.run(&mut |_| {}).unwrap();
    assert_eq!(probe(dir.path()), ProjectVersion(4));

    // Without sources (no or an empty `quillshade/`) the project is new: up to date, nothing written.
    let dir = v1_copy();
    std::fs::remove_dir_all(dir.path().join("quillshade")).unwrap();
    std::fs::create_dir(dir.path().join("quillshade")).unwrap();
    let raw = RawProject::open(dir.path()).unwrap();
    assert_eq!(raw.version, ProjectVersion::CURRENT);
    let mut events = Vec::new();
    let report = Migrator::new(raw, &[], COMPILED).run(&mut record(&mut events)).unwrap();
    assert!(report.plan.is_up_to_date() && events.is_empty());
    assert!(!dir.path().join("quillshade/project.ron").exists());
}

#[test]
fn uncompiled_domains_are_reported() {
    let dir = v1_copy();
    std::fs::create_dir(dir.path().join("quillshade/.cache")).unwrap();
    let warning = "domain `light` is not compiled into this editor; its files are not migrated".to_string();

    let migrator = Migrator::new(RawProject::open(dir.path()).unwrap(), &[], &[MATERIAL]);
    assert_eq!(migrator.plan().unwrap().warnings, std::slice::from_ref(&warning));
    let all = Migrator::new(RawProject::open(dir.path()).unwrap(), &[], COMPILED);
    assert!(all.plan().unwrap().warnings.is_empty(), "hidden directories are skipped");

    let mut migrator = migrator;
    let mut events = Vec::new();
    let report = migrator.run(&mut record(&mut events)).unwrap();
    assert_eq!(events.first(), Some(&MigrationEvent::Warning(warning.clone())));
    assert_eq!(report.plan.warnings, [warning]);
    // The uncompiled domain's files are left alone.
    assert_eq!(read(dir.path(), "quillshade/light/y.curve.ron"), read(&fixture_dir(), "quillshade/light/y.curve.ron"));
}
