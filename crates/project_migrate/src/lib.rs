//! The Migration Tool engine (techdoc §4.4, decisions D28/D38/D39, ADR 0046 "Crate `project_migrate` (D38)" and
//! "Orchestrator (D39)").
//!
//! Upgrades a whole project from its project version to [`ProjectVersion::CURRENT`] one step at a time: every step
//! is a barrier (the whole project reaches `v` before `v + 1`), files are owned and migrated by their domains through
//! [`DomainMigrator`], and `quillshade/project.ron` is owned by the built-in [`ProjectMigrator`], which always runs
//! last in a step so that `project.ron` is the step's commit point. Interrupted migrations resume on rerun (per-file
//! idempotency through [`DomainMigrator::is_fresh`]).
//!
//! Bevy-free; depends only on `project_core`. Generated files are never migrated: after a successful run the caller
//! regenerates everything.
//!
//! - [`RawProject`]: a project handle that opens old, current and too-new projects (no version comparison).
//! - [`DomainMigrator`] / [`MigrationFacts`]: the domain-side API.
//! - [`STEPS`]: the steps table (titles shown by the UI and `--dry-run`).
//! - [`Migrator`]: the orchestrator ([`Migrator::plan`] = dry run, [`Migrator::run`] = real run).
//! - [`git_state`]: the "commit first" check of the UI and CLI.

mod domain;
mod error;
mod git;
mod orchestrator;
mod project;
mod raw;
mod steps;

pub use domain::{DomainMigrator, MigrationFacts};
pub use error::MigrationError;
pub use git::{GitState, git_state};
pub use orchestrator::{
    MigrationEvent, MigrationPlan, MigrationReport, Migrator, PROJECT_RON_REWRITTEN, PlannedStep, StaleFile,
};
pub use project::ProjectMigrator;
pub use raw::RawProject;
pub use steps::{GENERIC_STEP_TITLE, STEPS, StepInfo, step_title};

pub use project_core::{DomainId, ProjectVersion};
