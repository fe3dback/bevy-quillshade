//! Shared helpers of the `project_migrate` integration tests: fixture copies, file snapshots and a synthetic
//! `marker` domain migrator for multi-step chains (ADR 0046 "Orchestrator (D39)" tests).

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use project_migrate::{
    DomainId, DomainMigrator, MigrationError, MigrationEvent, MigrationFacts, ProjectVersion, RawProject,
};

/// The synthetic test domain.
pub const MARKER: DomainId = DomainId::new("marker");

/// The checked-in v1 fixture.
pub fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/v1")
}

/// A fresh temp copy of the v1 fixture.
pub fn v1_copy() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    copy_dir(&fixture_dir(), dir.path());
    dir
}

/// A v1 copy plus `quillshade/marker/<name>` files holding `(marker: <n>)`.
pub fn v1_with_markers(markers: &[(&str, u32)]) -> tempfile::TempDir {
    let dir = v1_copy();
    for (name, n) in markers {
        let path = dir.path().join("quillshade/marker").join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, marker_text(*n)).unwrap();
    }
    dir
}

pub fn copy_dir(from: &Path, to: &Path) {
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

/// Every file below `root`: project-relative path -> bytes.
pub fn snapshot(root: &Path) -> BTreeMap<String, Vec<u8>> {
    files(root).into_iter().map(|(rel, path)| (rel, std::fs::read(path).unwrap())).collect()
}

/// Every file below `root`: project-relative path -> modification time.
pub fn mtimes(root: &Path) -> BTreeMap<String, SystemTime> {
    files(root).into_iter().map(|(rel, path)| (rel, std::fs::metadata(path).unwrap().modified().unwrap())).collect()
}

fn files(root: &Path) -> BTreeMap<String, PathBuf> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                let rel = path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
                out.insert(rel, path);
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

pub fn read(root: &Path, rel: &str) -> String {
    std::fs::read_to_string(root.join(rel)).unwrap()
}

pub fn marker_text(n: u32) -> String {
    format!("(marker: {n})\n")
}

pub fn parse_marker(text: &str) -> Option<u32> {
    text.trim().strip_prefix("(marker: ")?.strip_suffix(')')?.parse().ok()
}

/// Collects every event of a run.
pub fn record(events: &mut Vec<MigrationEvent>) -> impl FnMut(&MigrationEvent) + '_ {
    move |e| events.push(e.clone())
}

/// A synthetic domain (`quillshade/marker/*.ron`, text `(marker: N)`): a file is fresh for `v` when `N >= v`;
/// migrating requires `N == v - 1` and writes `v`. `collect` publishes `file name -> N`; every call is logged.
#[derive(Default)]
pub struct MarkerMigrator {
    /// `collect v3 a.ron 2`, `migrate v3 a.ron facts=a.ron:2,b.ron:2`, …
    pub log: Mutex<Vec<String>>,
    /// Fail the `n`-th (0-based) `migrate` call of step `v`.
    pub fail_migrate: Option<(ProjectVersion, usize)>,
    /// Fail `is_fresh` of the named file at step `v`.
    pub fail_fresh: Option<(ProjectVersion, &'static str)>,
    /// `migrate` calls per step so far.
    pub migrate_calls: Mutex<BTreeMap<ProjectVersion, usize>>,
}

impl MarkerMigrator {
    pub fn log(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }

    fn name(file: &Path) -> String {
        file.file_name().unwrap().to_string_lossy().into_owned()
    }

    fn marker(file: &Path, text: &str) -> Result<u32, MigrationError> {
        parse_marker(text).ok_or_else(|| MigrationError::domain(MARKER, file, "not a marker file"))
    }
}

impl DomainMigrator for MarkerMigrator {
    fn domain(&self) -> DomainId {
        MARKER
    }

    fn files(&self, project: &RawProject) -> Vec<PathBuf> {
        let Ok(entries) = std::fs::read_dir(project.domain_dir(MARKER)) else {
            return Vec::new();
        };
        // Deliberately unsorted (reverse): the orchestrator sorts.
        let mut files: Vec<PathBuf> = entries.map(|e| e.unwrap().path()).collect();
        files.sort();
        files.reverse();
        files
    }

    fn is_fresh(&self, file: &Path, text: &str, target: ProjectVersion) -> Result<bool, MigrationError> {
        if self.fail_fresh.is_some_and(|(v, name)| v == target && Self::name(file) == name) {
            return Err(MigrationError::domain(MARKER, file, "injected is_fresh failure"));
        }
        Ok(Self::marker(file, text)? >= target.get())
    }

    fn collect(&self, file: &Path, text: &str, target: ProjectVersion, facts: &mut MigrationFacts) {
        let n = parse_marker(text).unwrap();
        facts.insert(MARKER, Self::name(file), n.to_string());
        self.log.lock().unwrap().push(format!("collect {target} {} {n}", Self::name(file)));
    }

    fn migrate(
        &self,
        file: &Path,
        text: &str,
        target: ProjectVersion,
        facts: &MigrationFacts,
    ) -> Result<String, MigrationError> {
        let call = {
            let mut calls = self.migrate_calls.lock().unwrap();
            let slot = calls.entry(target).or_default();
            *slot += 1;
            *slot - 1
        };
        if self.fail_migrate == Some((target, call)) {
            return Err(MigrationError::domain(MARKER, file, "injected migrate failure"));
        }
        let n = Self::marker(file, text)?;
        if n + 1 != target.get() {
            return Err(MigrationError::domain(MARKER, file, format!("marker {n} cannot step to {target}")));
        }
        let seen: Vec<String> = facts.iter(MARKER).map(|(k, v)| format!("{k}:{v}")).collect();
        self.log.lock().unwrap().push(format!("migrate {target} {} facts={}", Self::name(file), seen.join(",")));
        Ok(marker_text(target.get()))
    }
}
