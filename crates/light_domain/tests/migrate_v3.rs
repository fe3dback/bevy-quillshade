//! The Light domain's v3 migration step (ADR 0047 §14, techdoc §4.4 "Tests", ADR 0046 "Orchestrator (D39)"):
//! golden before -> after for every kind, freshness after the step, idempotency and resume after an interruption,
//! dangling paths, and an end-to-end run with both migrators (Material + Light) on the frozen v2 sources of
//! `examples/sample_game` (`shader_project`'s `tests/fixtures/sample_v2`, the sample before the Migration Tool
//! upgraded it): they become the committed sources, and a regeneration changes no committed generated file.
//!
//! Every run here targets v3 explicitly (`Migrator::with_target`), so the tests keep meaning the same once later
//! steps raise `ProjectVersion::CURRENT`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use light_domain::backend::migrate::{LightMigrator, V3};
use light_domain::backend::{LIGHT, LightAssets, LightBackend, texture_ref};
use project_core::regen::DomainBackend;
use project_core::{AssetIndex, AssetUuid, DomainId, Project, ProjectVersion};
use project_migrate::{DomainMigrator, MigrationError, MigrationFacts, Migrator, RawProject, StaleFile};

const MATERIAL: DomainId = DomainId::new("material");
const COMPILED: &[DomainId] = &[MATERIAL, LIGHT];

/// The UUID the `before` fixture's format-2 texture `skies/storm` already has.
const STORM: &str = "019b8000-0000-7000-8000-000000000001";

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/migrate_v3").join(name)
}

fn copy_dir(from: &Path, to: &Path, skip: &[&str]) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        if skip.iter().any(|s| name == *s) {
            continue;
        }
        let target = to.join(&name);
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target, &[]);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// A temp copy of the golden `before` project (v2, format-1 Light sources plus one format-2 texture).
fn before_copy() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    copy_dir(&fixture("before"), dir.path(), &[]);
    dir
}

/// Every file below `dir`: `dir`-relative path -> bytes.
fn snapshot(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                let rel = path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
                out.insert(rel, std::fs::read(&path).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    if dir.is_dir() {
        walk(dir, dir, &mut out);
    }
    out
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Asserts that two snapshots hold the same files with the same bytes, naming the first difference.
fn assert_same(actual: &BTreeMap<String, Vec<u8>>, expected: &BTreeMap<String, Vec<u8>>, what: &str) {
    assert_eq!(actual.keys().collect::<Vec<_>>(), expected.keys().collect::<Vec<_>>(), "{what}: file lists differ");
    for (file, bytes) in expected {
        assert!(
            &actual[file] == bytes,
            "{what}: {file} differs\n--- actual\n{}\n--- expected\n{}",
            String::from_utf8_lossy(&actual[file]),
            String::from_utf8_lossy(bytes)
        );
    }
}

fn migrator<'a>(root: &Path, migrators: &[&'a dyn DomainMigrator]) -> Migrator<'a> {
    Migrator::new(RawProject::open(root).unwrap(), migrators, COMPILED).with_target(V3)
}

/// Runs the v3 step with the Light migrator on `root`.
fn migrate(root: &Path) -> project_migrate::MigrationReport {
    migrator(root, &[&LightMigrator]).run(&mut |_| {}).unwrap()
}

fn stale(path: &str) -> StaleFile {
    let domain = if path == "quillshade/project.ron" { DomainId::PROJECT } else { LIGHT };
    StaleFile { domain, path: path.to_string() }
}

/// The golden fixture's stale files in migration order (Light sorted by absolute path, `project.ron` last). The
/// format-2 `skies/storm` texture is fresh; the hidden `.drafts/` folder is not a Light source.
const BEFORE_STALE: [&str; 6] = [
    "quillshade/light/day.environment.ron",
    "quillshade/light/night.environment.ron",
    "quillshade/light/outdoor/dusk.environment.ron",
    "quillshade/light/skies/clear.texture.ron",
    "quillshade/light/smooth.curve.ron",
    "quillshade/project.ron",
];

fn before_stale() -> Vec<StaleFile> {
    BEFORE_STALE.iter().map(|p| stale(p)).collect()
}

/// Golden before -> after for every Light kind: a hand-formatted one-line environment with a comment on a format-1
/// texture, an environment on a texture that already has its UUID, one on a dangling path, a texture, and a curve.
/// The `after` files are frozen.
#[test]
fn golden_before_to_after() {
    let dir = before_copy();
    let plan = migrator(dir.path(), &[&LightMigrator]).plan().unwrap();
    assert_eq!((plan.from, plan.to), (ProjectVersion(2), V3));
    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].stale, before_stale());
    assert_eq!(snapshot(dir.path()), snapshot(&fixture("before")), "the dry run writes nothing");

    let report = migrate(dir.path());
    assert_eq!(report.plan, plan, "the dry run predicts the real run");
    assert_eq!(report.written, before_stale());
    assert_same(&snapshot(dir.path()), &snapshot(&fixture("after")), "golden");
}

/// Every migrated file is fresh at v3 and loads through the current loaders with its id; the index of the migrated
/// project resolves every cube map, and the step then has nothing left to do.
#[test]
fn fresh_after_migrate() {
    let dir = before_copy();
    migrate(dir.path());
    let raw = RawProject::open(dir.path()).unwrap();
    assert_eq!(raw.version, V3);
    let files = LightMigrator.files(&raw);
    assert_eq!(files.len(), 6);
    for file in &files {
        assert!(LightMigrator.is_fresh(file, &read(file), V3).unwrap(), "{}", file.display());
        // Earlier steps had nothing to do for Light sources, before and after.
        assert!(LightMigrator.is_fresh(file, &read(file), ProjectVersion(2)).unwrap(), "{}", file.display());
    }
    let before = snapshot(dir.path());
    assert!(migrator(dir.path(), &[&LightMigrator]).plan().unwrap().is_up_to_date());
    assert!(migrate(dir.path()).written.is_empty());
    assert_eq!(snapshot(dir.path()), before);

    let project = Project::open(&raw.root).unwrap();
    let assets = LightAssets::load(&project);
    assert!(assets.load_errors.is_empty(), "{:?}", assets.load_errors);
    assert_eq!((assets.environments.len(), assets.textures.len(), assets.curves.len()), (3, 2, 1));
    let legacy = |rel: &str| AssetUuid::legacy(rel);
    assert_eq!(assets.environments["day"].id, legacy("quillshade/light/day.environment.ron"));
    assert_eq!(assets.curves["smooth"].id, legacy("quillshade/light/smooth.curve.ron"));
    assert_eq!(assets.textures["skies/clear"].id, legacy("quillshade/light/skies/clear.texture.ron"));
    assert_eq!(assets.textures["skies/storm"].id.to_string(), STORM, "an existing id is kept");
    assert_eq!(assets.environments["day"].ev100, 12.5);

    let index = AssetIndex::build(vec![LightBackend.index(&project)]);
    assert_eq!(index.assets_of(LIGHT, "environment").count(), 3);
    let mut resolved = assets.clone();
    resolved.resolve(&index);
    let cubemap = |path: &str| resolved.environments[path].cubemap.clone();
    assert_eq!(cubemap("day").as_deref(), Some("skies/clear"));
    assert_eq!(cubemap("night").as_deref(), Some("skies/storm"));
    assert_eq!(cubemap("outdoor/dusk").as_deref(), Some("gone"), "a dangling path stays");
    // Nothing dangles by UUID: the only broken reference was a path already.
    assert!(index.diagnostics(&project).is_empty(), "{:?}", index.diagnostics(&project));
    assert_eq!(index.refs_to(legacy(&texture_ref("skies/clear").source_rel())).count(), 1);
}

/// A Light migrator that fails its `n`-th `migrate` call (an interrupted run).
struct Interrupted {
    fail_at: usize,
    calls: Mutex<usize>,
}

impl DomainMigrator for Interrupted {
    fn domain(&self) -> DomainId {
        LightMigrator.domain()
    }

    fn files(&self, project: &RawProject) -> Vec<PathBuf> {
        LightMigrator.files(project)
    }

    fn is_fresh(&self, file: &Path, text: &str, target: ProjectVersion) -> Result<bool, MigrationError> {
        LightMigrator.is_fresh(file, text, target)
    }

    fn collect(&self, file: &Path, text: &str, target: ProjectVersion, facts: &mut MigrationFacts) {
        LightMigrator.collect(file, text, target, facts);
    }

    fn migrate(
        &self,
        file: &Path,
        text: &str,
        target: ProjectVersion,
        facts: &MigrationFacts,
    ) -> Result<String, MigrationError> {
        let mut calls = self.calls.lock().unwrap();
        *calls += 1;
        if *calls - 1 == self.fail_at {
            return Err(MigrationError::domain(LIGHT, file, "interrupted"));
        }
        LightMigrator.migrate(file, text, target, facts)
    }
}

/// A run interrupted after any number of migrated files, then rerun with fresh facts, gives exactly the bytes of a
/// clean run: already migrated files are fresh and keep their ids, the rest get the same `AssetUuid::legacy` ids the
/// first run's references point at.
#[test]
fn idempotent_and_interruptible() {
    let clean = snapshot(&fixture("after"));
    let light_files = before_stale().len() - 1;
    for fail_at in 0..light_files {
        let dir = before_copy();
        let interrupted = Interrupted { fail_at, calls: Mutex::new(0) };
        let err = migrator(dir.path(), &[&interrupted]).run(&mut |_| {}).unwrap_err();
        assert!(err.to_string().contains("interrupted"), "{err}");
        assert_eq!(RawProject::open(dir.path()).unwrap().version, ProjectVersion(2), "project.ron is not bumped");
        let plan = migrator(dir.path(), &[&LightMigrator]).plan().unwrap();
        assert_eq!(plan.steps[0].stale.len(), before_stale().len() - fail_at, "fail_at {fail_at}");
        let report = migrate(dir.path());
        assert_eq!(report.written.len(), before_stale().len() - fail_at);
        assert_same(&snapshot(dir.path()), &clean, &format!("resumed after {fail_at} files"));
    }
}

/// A cube map path that names no file stays as it was (it was dangling already) and gets no comment; the others
/// become the texture's UUID with the D27 path comment; the game file of a texture stays a path (D26).
#[test]
fn dangling_paths_stay() {
    let light = |rel: &str| read(&fixture("after/quillshade/light").join(rel));
    assert!(light("outdoor/dusk.environment.ron").contains("\n  cubemap: Some(\"gone\"),\n"));
    assert!(!light("outdoor/dusk.environment.ron").contains("//"));
    let clear = AssetUuid::legacy("quillshade/light/skies/clear.texture.ron");
    assert!(light("day.environment.ron").contains(&format!("\n  cubemap: Some(\"{clear}\"), // skies/clear\n")));
    assert!(light("night.environment.ron").contains(&format!("\n  cubemap: Some(\"{STORM}\"), // skies/storm\n")));
    assert!(light("skies/clear.texture.ron").contains("\n  path: \"textures/clear.ktx2\",\n"));
    assert!(!light("day.environment.ron").contains("by hand"), "hand-written comments are not kept");
}

/// Every generated file under `assets/quillshade/` and `crates/quillshade/`, by project-relative path.
fn generated_tree(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    for dir in ["assets/quillshade", "crates/quillshade"] {
        for (rel, bytes) in snapshot(&root.join(dir)) {
            out.insert(format!("{dir}/{rel}"), bytes);
        }
    }
    out
}

/// End to end with both migrators on the sample before the v3 step: a copy of `examples/sample_game` (its game
/// files and committed generated files) whose `quillshade/` is the frozen v2 one. The v3 step migrates every Material
/// and Light source into exactly the committed sources (the environments store the cube map's UUID), and a full
/// regeneration afterwards (every backend, the project index) changes no generated file.
#[test]
fn sample_v2_migrates_both_domains_to_the_committed_sample() {
    use shader_project::backend::MaterialBackend;
    use shader_project::migrate::MaterialMigrator;

    let sample = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/sample_game");
    let sample_v2 = Path::new(env!("CARGO_MANIFEST_DIR")).join("../shader_project/tests/fixtures/sample_v2");
    let dir = tempfile::tempdir().unwrap();
    copy_dir(&sample, dir.path(), &["target", "quillshade"]);
    copy_dir(&sample_v2, dir.path(), &[]);
    let generated = generated_tree(dir.path());
    assert!(generated.len() > 50, "{:?}", generated.keys());

    let report = migrator(dir.path(), &[&MaterialMigrator, &LightMigrator]).run(&mut |_| {}).unwrap();
    let count = |d: DomainId| report.written.iter().filter(|f| f.domain == d).count();
    assert_eq!((count(MATERIAL), count(LIGHT), count(DomainId::PROJECT)), (28, 4, 1), "{:?}", report.written);
    for f in report.written.iter().filter(|f| f.domain != DomainId::PROJECT) {
        let text = read(&dir.path().join(&f.path));
        let uuid = AssetUuid::legacy(&f.path);
        assert!(text.contains(&format!("\n  format_version: 2,\n  id: \"{uuid}\",\n")), "{}:\n{text}", f.path);
    }
    let sky = AssetUuid::legacy("quillshade/light/sky.texture.ron");
    for env in ["day", "night"] {
        let text = read(&dir.path().join(format!("quillshade/light/{env}.environment.ron")));
        assert!(text.contains(&format!("\n  cubemap: Some(\"{sky}\"), // sky\n")), "{env}:\n{text}");
    }
    let sources = |root: &Path| snapshot(&root.join("quillshade"));
    assert_same(&sources(dir.path()), &sources(&sample), "migrated vs committed sources");

    let project = Project::open(dir.path()).unwrap();
    let backends: [&dyn DomainBackend; 2] = [&MaterialBackend::default(), &LightBackend];
    let run = project_core::regen::run(&project, &backends, &[], false, false).unwrap();
    assert!(!run.has_errors(), "{:?}", run.diagnostics);
    assert!(run.diagnostics.iter().all(|d| !d.code.starts_with("AE")), "{:?}", run.diagnostics);
    assert!(run.changes.is_empty(), "{:?}", run.changes);
    assert_same(&generated_tree(dir.path()), &generated, "generated files after the v3 step");
}
