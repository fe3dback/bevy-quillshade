//! The migration orchestrator (techdoc §4.4 "Orchestrator", decision D39, ADR 0046 "Orchestrator (D39)").
//!
//! ```text
//! if version > target: TooNew;  if version == target: nothing to do
//! for v in version+1 ..= target:                      // barrier: the whole project reaches v before v+1
//!     facts = {}
//!     for each domain (by id, project last), each file (sorted): collect(file, text, v, &mut facts)
//!     stale = [(domain, file) | !is_fresh(file, text, v)]      // all computed before any write of this step
//!     for (domain, file) in stale: write_atomic(file, migrate(file, text, v, &facts))
//!     // project.ron is the project migrator's stale file, always last: the step's commit point
//! ```
//!
//! [`Migrator::plan`] (dry run) runs the same loop over an in-memory overlay of file texts and writes nothing, so
//! later steps see earlier steps' output; [`Migrator::run`] writes each migrated file at once (temp file + rename).
//! A failing `is_fresh` aborts a step before it writes anything; a failing `migrate` aborts after the files already
//! written in that step. Either way `project.ron` is not bumped and a rerun resumes the step (per-file idempotency).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use project_core::writer::write_atomic;
use project_core::{DomainId, ProjectVersion};

use crate::{DomainMigrator, MigrationError, MigrationFacts, ProjectMigrator, RawProject, step_title};

/// The built-in migrator of `project.ron`, appended to every migration.
static PROJECT_MIGRATOR: ProjectMigrator = ProjectMigrator;

/// One file a step migrates: its domain and project-relative path (`/` separators).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct StaleFile {
    pub domain: DomainId,
    pub path: String,
}

/// One step of a [`MigrationPlan`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedStep {
    pub to: ProjectVersion,
    pub title: &'static str,
    /// In migration order: domains by id, the project domain last; files sorted within a domain.
    pub stale: Vec<StaleFile>,
}

impl PlannedStep {
    /// The stale files of this step grouped by domain (paths in migration order).
    pub fn by_domain(&self) -> BTreeMap<DomainId, Vec<&str>> {
        let mut out: BTreeMap<DomainId, Vec<&str>> = BTreeMap::new();
        for file in &self.stale {
            out.entry(file.domain).or_default().push(&file.path);
        }
        out
    }
}

/// What a migration does (or did): the version range, the steps with their stale files, and warnings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationPlan {
    pub from: ProjectVersion,
    pub to: ProjectVersion,
    pub steps: Vec<PlannedStep>,
    /// Warnings such as uncompiled domains (only computed when the project is not up to date).
    pub warnings: Vec<String>,
}

impl MigrationPlan {
    /// `true` when the project already is at the target version: nothing to migrate.
    pub fn is_up_to_date(&self) -> bool {
        self.steps.is_empty()
    }

    /// The number of file migrations over all steps (a file stale in two steps counts twice).
    pub fn total_stale(&self) -> usize {
        self.steps.iter().map(|s| s.stale.len()).sum()
    }

    /// Every file migrated by any step, grouped by domain, each file once.
    pub fn stale_by_domain(&self) -> BTreeMap<DomainId, BTreeSet<&str>> {
        let mut out: BTreeMap<DomainId, BTreeSet<&str>> = BTreeMap::new();
        for file in self.steps.iter().flat_map(|s| &s.stale) {
            out.entry(file.domain).or_default().insert(&file.path);
        }
        out
    }
}

/// Progress of [`Migrator::run`], reported to its observer in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigrationEvent {
    /// A step begins; `stale` is the files it is about to migrate, in order.
    StepStarted { to: ProjectVersion, title: &'static str, stale: Vec<StaleFile> },
    /// One file was migrated and written.
    FileMigrated { domain: DomainId, path: String },
    /// A step is complete: the whole project is at `to`.
    StepFinished { to: ProjectVersion },
    /// A non-fatal problem (sent before the first step).
    Warning(String),
    /// Information for the user about what was written: [`PROJECT_RON_REWRITTEN`] right after the first rewrite of
    /// a `project.ron` that existed before the run.
    Note(String),
}

/// The note of the first rewrite of an existing `project.ron` ([`MigrationEvent::Note`]): the project migrator
/// writes the canonical text of the known settings, so the user's comments and unknown keys are gone.
pub const PROJECT_RON_REWRITTEN: &str = "project.ron rewritten: comments and unknown keys are not kept";

/// The result of a successful [`Migrator::run`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationReport {
    /// The plan actually executed (equal to what [`Migrator::plan`] returned before the run).
    pub plan: MigrationPlan,
    /// Every file written, in write order (a file migrated by two steps appears twice).
    pub written: Vec<StaleFile>,
}

/// The migration orchestrator over one project.
pub struct Migrator<'a> {
    project: RawProject,
    /// Sorted by domain id; the built-in project migrator is not in this list (it always runs last).
    migrators: Vec<&'a dyn DomainMigrator>,
    /// Domains compiled into this build (plus every migrator's domain and the project domain).
    compiled: BTreeSet<&'static str>,
    target: ProjectVersion,
}

impl<'a> Migrator<'a> {
    /// A migrator to [`ProjectVersion::CURRENT`]. `migrators` are the domains' migrators (the built-in project
    /// migrator is appended automatically); `compiled` lists every domain compiled into this build: a
    /// `quillshade/<dir>/` not in it yields a warning (its files are not migrated).
    pub fn new(project: RawProject, migrators: &[&'a dyn DomainMigrator], compiled: &[DomainId]) -> Migrator<'a> {
        let mut migrators = migrators.to_vec();
        migrators.sort_by_key(|m| m.domain());
        let compiled = compiled
            .iter()
            .copied()
            .chain(migrators.iter().map(|m| m.domain()))
            .chain([DomainId::PROJECT])
            .map(DomainId::as_str)
            .collect();
        Migrator { project, migrators, compiled, target: ProjectVersion::CURRENT }
    }

    /// Migrates to `target` instead of [`ProjectVersion::CURRENT`] (tests: synthetic multi-step chains). Steps not in
    /// [`crate::STEPS`] get [`crate::GENERIC_STEP_TITLE`].
    pub fn with_target(mut self, target: ProjectVersion) -> Migrator<'a> {
        self.target = target;
        self
    }

    /// The project (its `version` follows the steps [`Migrator::run`] committed).
    pub fn project(&self) -> &RawProject {
        &self.project
    }

    pub fn target(&self) -> ProjectVersion {
        self.target
    }

    /// Dry run: simulates every step in memory and returns what [`Migrator::run`] would do. Writes nothing.
    pub fn plan(&self) -> Result<MigrationPlan, MigrationError> {
        let mut project = self.project.clone();
        let mut overlay = Overlay::default();
        Ok(self.execute(&mut project, &mut overlay, &mut |_| {})?.plan)
    }

    /// Performs the migration, writing each migrated file at once, and reports progress to `observer`.
    pub fn run(&mut self, observer: &mut dyn FnMut(&MigrationEvent)) -> Result<MigrationReport, MigrationError> {
        let mut project = self.project.clone();
        let result = self.execute(&mut project, &mut Disk, observer);
        // Keep the committed version even when a later step failed.
        self.project = project;
        result
    }

    fn execute(
        &self,
        project: &mut RawProject,
        store: &mut dyn Store,
        observer: &mut dyn FnMut(&MigrationEvent),
    ) -> Result<MigrationReport, MigrationError> {
        let from = project.version;
        let to = self.target;
        if from > to {
            return Err(MigrationError::TooNew { found: from, supported: to });
        }
        let mut plan = MigrationPlan { from, to, steps: Vec::new(), warnings: Vec::new() };
        let mut written = Vec::new();
        if from == to {
            return Ok(MigrationReport { plan, written });
        }
        plan.warnings = self.uncompiled_warnings(project);
        for warning in &plan.warnings {
            observer(&MigrationEvent::Warning(warning.clone()));
        }
        // The first rewrite of a `project.ron` the user may have edited by hand is noted (a created one has nothing to
        // lose, a later step rewrites the canonical text written by an earlier one).
        let mut note_rewrite = store.exists(&project.settings_path());
        let ordered: Vec<&dyn DomainMigrator> =
            self.migrators.iter().copied().chain([&PROJECT_MIGRATOR as &dyn DomainMigrator]).collect();
        let mut v = from.next();
        while v <= to {
            let title = step_title(v);
            // Pre-pass: read every file of every domain and let its domain publish facts.
            let mut facts = MigrationFacts::default();
            let mut entries: Vec<(&dyn DomainMigrator, PathBuf, String)> = Vec::new();
            for &migrator in &ordered {
                let mut files = migrator.files(project);
                files.sort();
                files.dedup();
                for file in files {
                    // A v1 project may lack `project.ron` (ADR 0046): it reads as empty, and the project migrator
                    // creates it. Any other listed file must exist.
                    let missing = migrator.domain() == DomainId::PROJECT && !store.exists(&file);
                    let text = if missing { String::new() } else { store.read(&file)? };
                    migrator.collect(&file, &text, v, &mut facts);
                    entries.push((migrator, file, text));
                }
            }
            // Every freshness check before the first write of the step.
            let mut stale = Vec::new();
            for (migrator, file, text) in &entries {
                if !migrator.is_fresh(file, text, v)? {
                    stale.push((*migrator, file.as_path(), text.as_str()));
                }
            }
            let stale_files: Vec<StaleFile> =
                stale.iter().map(|(m, file, _)| StaleFile { domain: m.domain(), path: project.rel(file) }).collect();
            observer(&MigrationEvent::StepStarted { to: v, title, stale: stale_files.clone() });
            for ((migrator, file, text), stale_file) in stale.into_iter().zip(&stale_files) {
                let migrated = migrator.migrate(file, text, v, &facts)?;
                if !migrator.is_fresh(file, &migrated, v)? {
                    let message = format!("the migrated file is still stale for {v}");
                    return Err(MigrationError::domain(migrator.domain(), file, message));
                }
                store.write(file, migrated)?;
                written.push(stale_file.clone());
                observer(&MigrationEvent::FileMigrated { domain: stale_file.domain, path: stale_file.path.clone() });
                if note_rewrite && stale_file.domain == DomainId::PROJECT {
                    note_rewrite = false;
                    observer(&MigrationEvent::Note(PROJECT_RON_REWRITTEN.into()));
                }
            }
            project.version = v;
            observer(&MigrationEvent::StepFinished { to: v });
            plan.steps.push(PlannedStep { to: v, title, stale: stale_files });
            v = v.next();
        }
        Ok(MigrationReport { plan, written })
    }

    /// One warning per `quillshade/<dir>/` that is not hidden, not `project` and not compiled into this build.
    fn uncompiled_warnings(&self, project: &RawProject) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(project.quillshade_dir()) else {
            return Vec::new();
        };
        let dirs: BTreeSet<String> = entries
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .filter_map(|e| e.file_name().to_str().map(str::to_string))
            .filter(|name| !name.starts_with('.') && !self.compiled.contains(name.as_str()))
            .collect();
        dirs.into_iter()
            .map(|dir| format!("domain `{dir}` is not compiled into this editor; its files are not migrated"))
            .collect()
    }
}

/// Where the orchestrator reads and writes file texts.
trait Store {
    fn exists(&self, path: &Path) -> bool;
    fn read(&mut self, path: &Path) -> Result<String, MigrationError>;
    fn write(&mut self, path: &Path, text: String) -> Result<(), MigrationError>;
}

/// The real files: one temp file + rename per write.
struct Disk;

impl Store for Disk {
    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn read(&mut self, path: &Path) -> Result<String, MigrationError> {
        std::fs::read_to_string(path).map_err(|e| MigrationError::io(path, e))
    }

    fn write(&mut self, path: &Path, text: String) -> Result<(), MigrationError> {
        write_atomic(path, text).map_err(|e| MigrationError::io(path, e))
    }
}

/// The dry run's in-memory overlay over the real files.
#[derive(Default)]
struct Overlay(BTreeMap<PathBuf, String>);

impl Store for Overlay {
    fn exists(&self, path: &Path) -> bool {
        self.0.contains_key(path) || Disk.exists(path)
    }

    fn read(&mut self, path: &Path) -> Result<String, MigrationError> {
        match self.0.get(path) {
            Some(text) => Ok(text.clone()),
            None => Disk.read(path),
        }
    }

    fn write(&mut self, path: &Path, text: String) -> Result<(), MigrationError> {
        self.0.insert(path.to_path_buf(), text);
        Ok(())
    }
}
