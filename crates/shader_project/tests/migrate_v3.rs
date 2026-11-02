//! The Material domain's v3 migration step (ADR 0047 §14, techdoc §4.4 "Tests", ADR 0046 "Orchestrator (D39)"):
//! golden before -> after for every kind, freshness after the step, idempotency and resume after an interruption,
//! dangling paths, agreement with the fixtures (`fixtures::save_all` = the migrated `fixtures::save_all_v1`), and
//! the frozen v2 sources of `examples/sample_game` (`tests/fixtures/sample_v2`, the sample as it was before the
//! Migration Tool upgraded it) migrating to the committed sample.
//!
//! Every run here targets v3 explicitly (`Migrator::with_target`), so the tests keep meaning the same once later
//! steps raise `ProjectVersion::CURRENT`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use project_core::{AssetUuid, DomainId, ProjectVersion};
use project_migrate::{DomainMigrator, MigrationError, MigrationFacts, Migrator, RawProject, StaleFile};
use shader_project::backend::MaterialBackend;
use shader_project::layout::MATERIAL;
use shader_project::migrate::{MaterialMigrator, V3};
use shader_project::{Project, fixtures};

const LIGHT: DomainId = DomainId::new("light");
const COMPILED: &[DomainId] = &[MATERIAL, LIGHT];

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

/// A temp copy of the golden `before` project (v2, format-1 Material sources).
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

/// Runs the v3 step with the Material migrator on `root`.
fn migrate(root: &Path) -> project_migrate::MigrationReport {
    migrator(root, &[&MaterialMigrator]).run(&mut |_| {}).unwrap()
}

fn stale(path: &str) -> StaleFile {
    let domain = if path == "quillshade/project.ron" { DomainId::PROJECT } else { MATERIAL };
    StaleFile { domain, path: path.to_string() }
}

/// The golden fixture's files in migration order (Material sorted by absolute path, `project.ron` last).
const BEFORE_STALE: [&str; 8] = [
    "quillshade/material/.nodes/soft_clamp.ron",
    "quillshade/material/common/layer.function.ron",
    "quillshade/material/common/pulse.curve.ron",
    "quillshade/material/common/terrain.gradients.ron",
    "quillshade/material/common/tuning.consts.ron",
    "quillshade/material/env/all_refs.material.ron",
    "quillshade/material/textures/rock.texture.ron",
    "quillshade/project.ron",
];

fn before_stale() -> Vec<StaleFile> {
    BEFORE_STALE.iter().map(|p| stale(p)).collect()
}

/// Golden before -> after for every Material kind: a graph with every reference kind (function, user node, curve,
/// constant table + constant, texture, gradient collection + slot, plus dangling paths, `@internal/default` and an
/// empty texture), a function referencing a curve, a curve, a constant table, a gradient collection, a texture, and
/// a hand-formatted one-line format-1 user node with a comment. The `after` files are frozen.
#[test]
fn golden_before_to_after() {
    let dir = before_copy();
    let plan = migrator(dir.path(), &[&MaterialMigrator]).plan().unwrap();
    assert_eq!((plan.from, plan.to), (ProjectVersion(2), V3));
    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].stale, before_stale());
    assert_eq!(snapshot(dir.path()), snapshot(&fixture("before")), "the dry run writes nothing");

    let report = migrate(dir.path());
    assert_eq!(report.plan, plan, "the dry run predicts the real run");
    assert_eq!(report.written, before_stale());
    assert_same(&snapshot(dir.path()), &snapshot(&fixture("after")), "golden");
}

/// Every migrated file is fresh at v3 and loads through the current loaders; the step then has nothing left to do,
/// and a second run changes nothing.
#[test]
fn fresh_after_migrate() {
    let dir = before_copy();
    migrate(dir.path());
    let raw = RawProject::open(dir.path()).unwrap();
    assert_eq!(raw.version, V3);
    let files = MaterialMigrator.files(&raw);
    assert_eq!(files.len(), 7);
    for file in &files {
        assert!(MaterialMigrator.is_fresh(file, &read(file), V3).unwrap(), "{}", file.display());
        // Earlier steps had nothing to do for Material sources, before and after.
        assert!(MaterialMigrator.is_fresh(file, &read(file), ProjectVersion(2)).unwrap(), "{}", file.display());
    }
    let before = snapshot(dir.path());
    let plan = migrator(dir.path(), &[&MaterialMigrator]).plan().unwrap();
    assert!(plan.is_up_to_date(), "{plan:?}");
    let report = migrate(dir.path());
    assert!(report.written.is_empty());
    assert_eq!(snapshot(dir.path()), before);

    // The current loaders read every migrated file.
    let project = Project::open(&raw.root).unwrap();
    let set = shader_project::AssetSet::load_all(&project);
    assert_eq!(set.len(), 6);
    for id in set.ids() {
        let id_of = |r: Result<project_core::AssetUuid, &String>| r.unwrap_or_else(|e| panic!("{id}: {e}"));
        let uuid = match id.ty {
            ty if ty.is_graph() => id_of(set.assets[id].as_ref().map(|a| a.id)),
            shader_project::AssetType::Curve => id_of(set.curves[id].as_ref().map(|c| c.id)),
            shader_project::AssetType::Consts => id_of(set.consts[id].as_ref().map(|c| c.id)),
            shader_project::AssetType::Texture => id_of(set.textures[id].as_ref().map(|t| t.id)),
            _ => id_of(set.gradients[id].as_ref().map(|g| g.id)),
        };
        assert_eq!(uuid, AssetUuid::legacy(&id.source_rel()), "{id}");
        assert!(uuid.is_v7());
    }
    let nodes = shader_project::assets::user_node_files(&project);
    let node = nodes[0].node.as_ref().unwrap();
    assert_eq!((node.format_version, node.ident.as_str()), (2, "soft_clamp"));
    assert_eq!(node.id, AssetUuid::legacy("quillshade/material/.nodes/soft_clamp.ron"));
    // The index sees the migrated project without problems except the references that were dangling already.
    let part = project_core::regen::DomainBackend::index(&MaterialBackend::default(), &project);
    assert_eq!(part.assets.len(), 7);
    let index = project_core::AssetIndex::build(vec![part]);
    assert!(index.diagnostics(&project).is_empty(), "{:?}", index.diagnostics(&project));
}

/// A Material migrator that fails its `n`-th `migrate` call (an interrupted run).
struct Interrupted {
    fail_at: usize,
    calls: Mutex<usize>,
}

impl DomainMigrator for Interrupted {
    fn domain(&self) -> DomainId {
        MaterialMigrator.domain()
    }

    fn files(&self, project: &RawProject) -> Vec<PathBuf> {
        MaterialMigrator.files(project)
    }

    fn is_fresh(&self, file: &Path, text: &str, target: ProjectVersion) -> Result<bool, MigrationError> {
        MaterialMigrator.is_fresh(file, text, target)
    }

    fn collect(&self, file: &Path, text: &str, target: ProjectVersion, facts: &mut MigrationFacts) {
        MaterialMigrator.collect(file, text, target, facts);
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
            return Err(MigrationError::domain(MATERIAL, file, "interrupted"));
        }
        MaterialMigrator.migrate(file, text, target, facts)
    }
}

/// A run interrupted after any number of migrated files, then rerun with fresh facts, gives exactly the bytes of a
/// clean run: already migrated files are fresh and keep their ids, the rest get the same `AssetUuid::legacy` ids the
/// first run's references point at.
#[test]
fn idempotent_and_interruptible() {
    let clean = snapshot(&fixture("after"));
    let material_files = before_stale().len() - 1;
    for fail_at in 0..material_files {
        let dir = before_copy();
        let interrupted = Interrupted { fail_at, calls: Mutex::new(0) };
        let err = migrator(dir.path(), &[&interrupted]).run(&mut |_| {}).unwrap_err();
        assert!(err.to_string().contains("interrupted"), "{err}");
        assert_eq!(RawProject::open(dir.path()).unwrap().version, ProjectVersion(2), "project.ron is not bumped");
        let plan = migrator(dir.path(), &[&MaterialMigrator]).plan().unwrap();
        assert_eq!(plan.steps[0].stale.len(), before_stale().len() - fail_at, "fail_at {fail_at}");
        let report = migrate(dir.path());
        assert_eq!(report.written.len(), before_stale().len() - fail_at);
        assert_same(&snapshot(dir.path()), &clean, &format!("resumed after {fail_at} files"));
    }
}

/// References that name no file stay as they were (they were dangling already), as do `@internal/*` and `""`; no
/// `(missing)` comment is written, because only UUIDs are annotated.
#[test]
fn dangling_paths_stay() {
    let after = read(&fixture("after/quillshade/material/env/all_refs.material.ron"));
    for kept in [
        "\"curve\": Str(\"common/missing\"),\n",
        "kind: \"function::common/missing\",\n",
        "kind: \"user::missing\",\n",
        "\"default\": Str(\"@internal/default\"),\n",
        "\"texture\": Str(\"\"),\n",
    ] {
        assert!(after.contains(kept), "{kept}");
    }
    assert!(!after.contains("(missing)"));
    let pulse = AssetUuid::legacy("quillshade/material/common/pulse.curve.ron");
    let soft = AssetUuid::legacy("quillshade/material/.nodes/soft_clamp.ron");
    let layer = AssetUuid::legacy("quillshade/material/common/layer.function.ron");
    for migrated in [
        format!("\"curve\": Str(\"{pulse}\"), // common/pulse\n"),
        format!("kind: \"user::{soft}\", // .nodes/soft_clamp\n"),
        format!("kind: \"function::{layer}\", // common/layer\n"),
        "\"const\": Int(1001),\n".to_string(),
        "\"gradient\": Int(1007),\n".to_string(),
    ] {
        assert!(after.contains(&migrated), "{migrated}");
    }
}

/// A game crate (workspace `crates/*`, empty `assets/`) at project version v2 (before the v3 step).
fn v2_game() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Cargo.toml"), "[workspace]\nmembers = [\"crates/*\"]\n").unwrap();
    std::fs::create_dir_all(dir.path().join("assets")).unwrap();
    std::fs::create_dir_all(dir.path().join("quillshade")).unwrap();
    let settings = "ProjectSettings(\n  project_version: 2,\n  header_date: false,\n)\n";
    std::fs::write(dir.path().join("quillshade/project.ron"), settings).unwrap();
    dir
}

/// The fixtures written as projects before v3 have them (`fixtures::save_all_v1`) and migrated by the tool equal
/// the fixtures as `fixtures::save_all` writes them, byte for byte: what lets `make_sample` reproduce a migrated
/// sample.
#[test]
fn migration_equals_the_fixtures() {
    let migrated = v2_game();
    fixtures::save_all_v1(migrated.path()).unwrap();
    let report = migrate(migrated.path());
    assert_eq!(report.written.len(), report.plan.steps[0].stale.len());
    assert!(report.written.len() > 25, "{:?}", report.written);

    let written = tempfile::tempdir().unwrap();
    std::fs::write(written.path().join("Cargo.toml"), "[workspace]\nmembers = [\"crates/*\"]\n").unwrap();
    std::fs::create_dir_all(written.path().join("assets")).unwrap();
    fixtures::save_all(&Project::open(written.path()).unwrap()).unwrap();
    let material = |root: &Path| snapshot(&root.join("quillshade/material"));
    assert_same(&material(migrated.path()), &material(written.path()), "migrated vs written");
}

/// The committed sample game (`examples/sample_game`).
fn sample() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/sample_game")
}

/// The frozen v2 sources of the sample (`quillshade/` before the v3 step).
fn sample_v2() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sample_v2")
}

/// Every fixture file `fixtures::save_all` writes is the committed sample's file, byte for byte (the sample's
/// sources are the migrated v2 sample, so the fixtures reproduce what the tool wrote; `make_sample` stays in sync).
#[test]
fn fixtures_equal_the_migrated_sample() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Cargo.toml"), "[workspace]\nmembers = [\"crates/*\"]\n").unwrap();
    std::fs::create_dir_all(dir.path().join("assets")).unwrap();
    fixtures::save_all(&Project::open(dir.path()).unwrap()).unwrap();
    let written = snapshot(&dir.path().join("quillshade/material"));
    let committed = snapshot(&sample().join("quillshade/material"));
    assert!(written.len() > 25, "{:?}", written.keys());
    for (file, bytes) in &written {
        let Some(sample) = committed.get(file) else { panic!("{file} is not in the sample") };
        assert!(
            sample == bytes,
            "{file}: rerun `cargo run -p shader_project --example make_sample`\n--- fixture\n{}\n--- sample\n{}",
            String::from_utf8_lossy(bytes),
            String::from_utf8_lossy(sample)
        );
    }
}

/// The frozen v2 sample (`tests/fixtures/sample_v2`) migrated by the Material migrator alone: every Material source
/// is upgraded to exactly the committed sample's file, the Light sources are not touched (the Light migrator owns
/// them; `light_domain`'s test runs both and regenerates).
#[test]
fn sample_v2_material_migrates_to_the_committed_sample() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Cargo.toml"), "[workspace]\nmembers = [\"crates/*\"]\n").unwrap();
    std::fs::create_dir_all(dir.path().join("assets")).unwrap();
    copy_dir(&sample_v2(), dir.path(), &[]);
    let light_before = snapshot(&dir.path().join("quillshade/light"));

    let report = migrate(dir.path());
    let material: Vec<&StaleFile> = report.written.iter().filter(|f| f.domain == MATERIAL).collect();
    assert_eq!(material.len(), 28, "{material:?}");
    assert_eq!(report.written.len(), 29, "plus project.ron");
    assert_eq!(snapshot(&dir.path().join("quillshade/light")), light_before);
    for f in &material {
        let text = read(&dir.path().join(&f.path));
        let uuid = AssetUuid::legacy(&f.path);
        assert!(text.contains(&format!("\n  format_version: 2,\n  id: \"{uuid}\",\n")), "{}:\n{text}", f.path);
    }
    let node = read(&dir.path().join("quillshade/material/.nodes/soft_clamp.ron"));
    assert!(node.contains("\n  ident: \"soft_clamp\",\n"), "{node}");
    let soft = AssetUuid::legacy("quillshade/material/.nodes/soft_clamp.ron");
    let user = read(&dir.path().join("quillshade/material/fx/user_node.material.ron"));
    assert!(user.contains(&format!("kind: \"user::{soft}\", // .nodes/soft_clamp\n")), "{user}");
    let material = |root: &Path| snapshot(&root.join("quillshade/material"));
    assert_same(&material(dir.path()), &material(&sample()), "migrated vs committed sample");
}
