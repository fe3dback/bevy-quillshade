//! The Migration Tool session (ADR 0046 "Editor (Migration Tool modal)", decision D28, techdoc §4.4 "UI"): the state
//! layer of the modal that opens instead of a project whose version is older than [`ProjectVersion::CURRENT`].
//!
//! - [`Editor::open_project`] (and so [`Editor::switch_project`]: the CLI argument and the last project at startup,
//!   recent projects, the Open dialog) does not open such a project: on [`project_core::OpenError::NeedsMigration`]
//!   it plans the migration ([`Migrator::plan`], a dry run), reads the [`GitState`] and starts a
//!   [`MigrationSession`] in [`Editor::migration`] where the project would have opened (the open project, if any,
//!   is closed first). Nothing is added to the recent projects until the project actually opens.
//! - While a session exists the rest of the editor is unavailable: [`Editor::apply`] ignores every action but the
//!   [`MigrationAction`]s and quitting, and no other dialog opens.
//! - [`MigrationAction::Run`] runs [`Migrator::run`] on a background thread with the domains' migrators
//!   ([`crate::domain::DomainList::migrators`]), then, on the same thread and as part of the run, the full regen of
//!   every enabled domain with the domains' generators ([`crate::domain::DomainList::backends`], headless like the
//!   CLI's `migrate`): generated files are never migrated (ADR 0046 "then: full regen"), and quitting at the result
//!   must not leave them stale (the next open sees a current project). Its events, the regen's changes and
//!   diagnostics, and the result come back over a channel, which [`Editor::poll_migration`] (a system of
//!   [`crate::app::EditorCorePlugin`]) drains each frame into the phase (progress) and the log.
//!   [`Editor::wait_migration`] blocks on it instead (headless tests).
//! - After a successful run [`MigrationAction::Open`] just opens the project normally. A failed run (the migration,
//!   or the regen after it) keeps the session (the error is logged); a later run resumes the interrupted step
//!   (per-file idempotency of the orchestrator), or only regenerates when the migration itself had completed.
//! - Quitting (Quit, ForceQuit, closing the window) is deferred while the run thread writes (ADR 0046 "Quit while a
//!   migration runs is deferred"): [`MigrationSession::quit_requested`] is set and the editor quits once the run
//!   reports its result, success or failure. A crash midway is safe (the next run resumes), but a deliberate quit
//!   should not leave a half step behind.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use project_core::regen::DomainBackend;
use project_core::writer::Change;
use project_core::{DomainId, Project, ProjectVersion};
use project_migrate::{
    DomainMigrator, GitState, MigrationError, MigrationEvent, MigrationPlan, Migrator, RawProject, git_state,
};

use crate::action::{MigrationAction, Unsaved};
use crate::domain::DomainList;
use crate::message::Level;
use crate::util::text::plural;

use super::editor::Editor;

/// Where the Migration Tool is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigrationPhase {
    /// The plan is shown; nothing written yet (also after a dry run).
    Ready,
    /// The migration runs on its thread: `done` of `total` file migrations written.
    Running { done: usize, total: usize },
    /// The project is at the target version and regenerated; `written` file migrations were written by the run.
    Done { written: usize },
    /// The run (the plan before it, the migration or the regen after it) failed with `message`; files already
    /// written stay, a rerun resumes.
    Failed { message: String },
}

/// What the migration thread reports.
enum RunMessage {
    Event(MigrationEvent),
    /// A line for the log (the migration's result, the regen after it).
    Log(String),
    /// The run ended (migration and regen): the number of file migrations written, or the error.
    Finished(Result<usize, String>),
}

/// The Migration Tool's state ([`Editor::migration`]): what the modal shows and where the run is.
pub struct MigrationSession {
    /// The project root (canonical, as [`RawProject::root`]).
    pub root: PathBuf,
    /// The project version on disk when the session started.
    pub from: ProjectVersion,
    /// The target version ([`ProjectVersion::CURRENT`]).
    pub to: ProjectVersion,
    /// The latest plan (dry run): the steps with titles, their stale files (`MigrationPlan::stale_by_domain`,
    /// `PlannedStep::by_domain`) and warnings. Recomputed by [`MigrationAction::DryRun`] and before each run.
    pub plan: MigrationPlan,
    /// The git state of the project directory when the session started (the "commit first" warning; never blocks).
    pub git: GitState,
    pub phase: MigrationPhase,
    /// The log lines (dry runs, the run's steps and files, warnings, the result), oldest first.
    pub log: Vec<String>,
    /// Bumped on every change of the session: the modal redraws when it moves.
    pub rev: u64,
    /// Quit was asked for while the migration ran: the editor quits as soon as the run ends (ADR 0046).
    pub quit_requested: bool,
    /// The running migration's channel (`Mutex`: a receiver is not `Sync`, the editor is a resource).
    pending: Option<Mutex<Receiver<RunMessage>>>,
}

impl MigrationSession {
    /// Whether the migration thread is running.
    pub fn is_running(&self) -> bool {
        matches!(self.phase, MigrationPhase::Running { .. })
    }

    fn push_log(&mut self, line: impl Into<String>) {
        self.log.push(line.into());
        self.rev += 1;
    }

    /// Appends `plan` as a dry run lists it: per step its stale files, grouped by domain (migration order).
    fn log_plan(&mut self, plan: &MigrationPlan) {
        self.push_log("dry run: nothing is written");
        if plan.is_up_to_date() {
            return self.push_log(format!("project is up to date ({})", plan.to));
        }
        self.push_log(format!("project {} -> {}", plan.from, plan.to));
        for warning in &plan.warnings {
            self.push_log(format!("warning: {warning}"));
        }
        for step in &plan.steps {
            self.push_log(format!("step {}: {}", step.to, step.title));
            if step.stale.is_empty() {
                self.push_log("  nothing to migrate");
            }
            // `stale` is in migration order (domains by id, the project domain last): consecutive runs per domain.
            let mut i = 0;
            while i < step.stale.len() {
                let domain = step.stale[i].domain;
                let n = step.stale[i..].iter().take_while(|f| f.domain == domain).count();
                self.push_log(format!("  {domain}: {n} file(s)"));
                for file in &step.stale[i..i + n] {
                    self.push_log(format!("    {}", file.path));
                }
                i += n;
            }
        }
    }

    /// One message of the migration thread: progress, a log line, or the end of the run.
    fn handle(&mut self, message: RunMessage) {
        match message {
            RunMessage::Event(MigrationEvent::Warning(warning)) => self.push_log(format!("warning: {warning}")),
            RunMessage::Event(MigrationEvent::Note(note)) => self.push_log(format!("note: {note}")),
            RunMessage::Log(line) => self.push_log(line),
            RunMessage::Event(MigrationEvent::StepStarted { to, title, stale }) => {
                if let MigrationPhase::Running { done, total } = &mut self.phase {
                    *total = (*total).max(*done + stale.len());
                }
                self.push_log(format!("step {to}: {title}"));
            }
            RunMessage::Event(MigrationEvent::FileMigrated { domain, path }) => {
                if let MigrationPhase::Running { done, total } = &mut self.phase {
                    *done += 1;
                    *total = (*total).max(*done);
                }
                self.push_log(format!("  migrate {domain} {path}"));
            }
            RunMessage::Event(MigrationEvent::StepFinished { .. }) => {}
            RunMessage::Finished(Ok(written)) => {
                self.pending = None;
                self.phase = MigrationPhase::Done { written };
                self.rev += 1;
            }
            RunMessage::Finished(Err(message)) => self.fail(message),
        }
    }

    fn fail(&mut self, message: String) {
        self.pending = None;
        self.push_log(format!("error: {message}"));
        self.phase = MigrationPhase::Failed { message };
    }
}

/// Generated files are never migrated (ADR 0046): once the migration succeeded every enabled domain regenerates from
/// the migrated sources, headless like the CLI's `migrate` (`project_core::regen::run` with every backend, no compile
/// check). Its diagnostics and changes go to `log` (as the CLI prints them); an error when the migrated project does
/// not open or a write fails.
fn regenerate(root: &Path, backends: &[Arc<dyn DomainBackend>], log: &mut dyn FnMut(String)) -> Result<(), String> {
    let project = Project::open(root).map_err(|e| format!("the migrated project does not open: {e}"))?;
    let backends: Vec<&dyn DomainBackend> = backends.iter().map(|b| b.as_ref()).collect();
    log("regenerate every domain".into());
    let report = project_core::regen::run(&project, &backends, &[], false, false)
        .map_err(|e| format!("regeneration failed: {e}"))?;
    for d in &report.diagnostics {
        let level = match d.level {
            project_core::Level::Error => "error",
            project_core::Level::Warning => "warning",
            project_core::Level::Info => "info",
        };
        log(format!("{level}: {d}"));
    }
    for change in &report.changes {
        let (tag, path) = match change {
            Change::Create(p) => ("create", p),
            Change::Modify(p) => ("modify", p),
            Change::Delete(p) => ("delete", p),
        };
        log(format!("  {tag} {}", project.rel(path)));
    }
    let errors = report.diagnostics.iter().filter(|d| d.is_error()).count();
    let mut summary = format!("regenerated: {} changed", plural(report.changes.len(), "file", "files"));
    if errors > 0 {
        summary.push_str(&format!(", {}", plural(errors, "error", "errors")));
    }
    log(summary);
    Ok(())
}

/// Opens the project at `root` for migration and plans it with the registered domains' migrators (the compiled
/// domains: every real domain).
fn plan_project(root: &Path, list: &DomainList) -> Result<(RawProject, MigrationPlan), MigrationError> {
    let raw = RawProject::open(root)?;
    let migrators: Vec<&dyn DomainMigrator> = list.migrators().iter().map(|m| m.as_ref()).collect();
    let compiled: Vec<DomainId> = list.real_ids().collect();
    let plan = Migrator::new(raw.clone(), &migrators, &compiled).plan()?;
    Ok((raw, plan))
}

impl Editor {
    /// [`Editor::open_project`] found an older project: plans its migration and starts the Migration Tool in place of
    /// opening it. The open project, if any, closes first (its unsaved changes were handled by
    /// [`Editor::switch_project`]; whatever is left is dropped). An error (the plan fails) leaves everything as it
    /// was; the caller shows it.
    pub(crate) fn start_migration(&mut self, path: &Path) -> Result<(), String> {
        let (raw, plan) = plan_project(path, &self.domain_list).map_err(|e| e.to_string())?;
        let git = git_state(&raw.root);
        if self.project.is_some() {
            self.close_project(Unsaved::Discard);
        }
        self.dialog = None;
        self.pick_folder = false;
        let root = raw.root.display().to_string();
        self.info(format!("Project {root} is at {} and needs migration to {}", plan.from, plan.to));
        self.migration = Some(MigrationSession {
            root: raw.root,
            from: plan.from,
            to: plan.to,
            plan,
            git,
            phase: MigrationPhase::Ready,
            log: Vec::new(),
            rev: 0,
            quit_requested: false,
            pending: None,
        });
        self.touch_all();
        Ok(())
    }

    /// A button of the Migration Tool ([`crate::action::Action::Migration`]); without a session nothing happens.
    pub fn apply_migration(&mut self, action: MigrationAction) {
        let Some(session) = self.migration.as_mut() else {
            return bevy::log::debug!("{action:?} ignored: no migration session");
        };
        match action {
            MigrationAction::DryRun => {
                if session.is_running() {
                    return;
                }
                match plan_project(&session.root, &self.domain_list) {
                    Ok((_, plan)) => {
                        session.log_plan(&plan);
                        session.plan = plan;
                    }
                    Err(e) => session.push_log(format!("error: {e}")),
                }
            }
            MigrationAction::Run => {
                if !matches!(session.phase, MigrationPhase::Ready | MigrationPhase::Failed { .. }) {
                    return;
                }
                self.run_migration();
            }
            MigrationAction::Cancel => {
                if session.is_running() {
                    return;
                }
                let root = session.root.display().to_string();
                self.migration = None;
                self.info(format!("Migration of {root} closed; no project is open"));
                self.touch_all();
            }
            MigrationAction::Open => {
                let Some(session) = self.migration.take_if(|s| matches!(s.phase, MigrationPhase::Done { .. })) else {
                    return;
                };
                // The run already regenerated every domain; still older (changed meanwhile) starts a new session.
                if let Err(e) = self.open_project(&session.root) {
                    self.io_error("Cannot open project", e);
                }
                self.touch_all();
            }
        }
    }

    /// [`MigrationAction::Run`]: plans again (the project may have moved since, e.g. after a failed run) and spawns the
    /// migration thread, which migrates, then regenerates ([`regenerate`]); its messages arrive through
    /// [`Editor::poll_migration`] / [`Editor::wait_migration`].
    fn run_migration(&mut self) {
        let list = self.domain_list.clone();
        let Some(session) = self.migration.as_mut() else { return };
        let (raw, plan) = match plan_project(&session.root, &list) {
            Ok(planned) => planned,
            Err(e) => {
                session.fail(e.to_string());
                return self.after_migration_moved();
            }
        };
        session.push_log(format!("project {} -> {}", plan.from, plan.to));
        let total = plan.total_stale();
        session.plan = plan;
        session.phase = MigrationPhase::Running { done: 0, total };
        let migrators: Vec<Arc<dyn DomainMigrator>> = list.migrators().to_vec();
        let backends: Vec<Arc<dyn DomainBackend>> = list.backends().to_vec();
        let compiled: Vec<DomainId> = list.real_ids().collect();
        let (tx, rx) = channel();
        session.pending = Some(Mutex::new(rx));
        std::thread::spawn(move || {
            // A closed channel (the session went away) is no error: the run completes unobserved.
            let send = |message: RunMessage| drop(tx.send(message));
            let root = raw.root.clone();
            let refs: Vec<&dyn DomainMigrator> = migrators.iter().map(|m| m.as_ref()).collect();
            let mut migrator = Migrator::new(raw, &refs, &compiled);
            let result = migrator.run(&mut |event| send(RunMessage::Event(event.clone())));
            let result = result.map_err(|e| e.to_string()).and_then(|report| {
                let written = report.written.len();
                send(RunMessage::Log(format!("migrated {written} file(s); the project is at {}", report.plan.to)));
                regenerate(&root, &backends, &mut |line| send(RunMessage::Log(line)))?;
                Ok(written)
            });
            send(RunMessage::Finished(result));
        });
    }

    /// Drains the running migration's messages without blocking (every frame, [`crate::app::EditorCorePlugin`]);
    /// `true` when something arrived. A thread that stopped without a result fails the run.
    pub fn poll_migration(&mut self) -> bool {
        let Some(session) = self.migration.as_mut() else { return false };
        let mut moved = false;
        while let Some(rx) = session.pending.as_ref() {
            let received = rx.lock().map_or(Err(TryRecvError::Disconnected), |rx| rx.try_recv());
            match received {
                Ok(message) => session.handle(message),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => session.fail("the migration stopped unexpectedly".into()),
            }
            moved = true;
        }
        if moved {
            self.after_migration_moved();
        }
        moved
    }

    /// Blocks until the running migration ends, handling its messages (headless tests; the app polls instead).
    pub fn wait_migration(&mut self) {
        let Some(session) = self.migration.as_mut() else { return };
        let mut moved = false;
        while let Some(rx) = session.pending.as_ref() {
            let received = rx.lock().ok().and_then(|rx| rx.recv().ok());
            match received {
                Some(message) => session.handle(message),
                None => session.fail("the migration stopped unexpectedly".into()),
            }
            moved = true;
        }
        if moved {
            self.after_migration_moved();
        }
    }

    /// Quit, ForceQuit or closing the window while the migration runs ([`Editor::apply`]): instead of quitting now,
    /// the session remembers it and the editor quits once the run ends ([`Editor::after_migration_moved`]), so a
    /// deliberate quit never leaves a half-written step behind (ADR 0046). `true` when the quit was deferred.
    pub(crate) fn defer_quit(&mut self) -> bool {
        let Some(session) = self.migration.as_mut().filter(|s| s.is_running()) else { return false };
        if !session.quit_requested {
            session.quit_requested = true;
            session.push_log("quit requested: the editor quits once the migration finishes");
            self.info("The editor quits once the running migration finishes");
            self.touch_all();
        }
        true
    }

    /// Once the run ended its result also goes to the console/logs, and a quit deferred meanwhile
    /// ([`MigrationSession::quit_requested`]) happens.
    fn after_migration_moved(&mut self) {
        let Some(session) = self.migration.as_ref() else { return };
        let root = session.root.display().to_string();
        let quit = session.quit_requested;
        let (level, text) = match &session.phase {
            MigrationPhase::Done { written } => {
                (Level::Info, format!("Migrated {root}: {written} file(s), every domain regenerated"))
            }
            MigrationPhase::Failed { message } => (Level::Error, format!("Migration of {root} failed: {message}")),
            MigrationPhase::Ready | MigrationPhase::Running { .. } => return,
        };
        self.push_msg(level, text);
        if quit {
            self.save_config();
            self.quit = true;
        }
    }
}
