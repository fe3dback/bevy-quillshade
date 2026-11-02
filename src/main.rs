//! `shader-editor`: GUI without arguments; `regen` / `check` / `diff` run headless (spec §14); `migrate` upgrades an
//! older project to this editor's project version and regenerates it (techdoc §4.4, ADR 0046 "CLI"). An older project
//! is refused by `regen` / `check` / `diff` with a hint to run `migrate` (exit 2).

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use editor_core::domain::EditorDomain;
use project_core::regen::{DomainBackend, RunReport, run};
use project_core::{DomainId, OpenError};
use project_migrate::{DomainMigrator, GitState, MigrationError, MigrationEvent, Migrator, RawProject, git_state};
use shader_project::Project;
use shader_project::writer::Change;

/// The domains, one line each (design §6.1): the GUI and the CLI share this list.
fn domains() -> Vec<Box<dyn EditorDomain>> {
    vec![Box::new(material_domain::MaterialDomain), Box::new(light_domain::LightDomain)]
}

#[derive(Parser)]
#[command(
    name = "shader-editor",
    version = editor_core::EDITOR_VERSION,
    about = "Node-based shader editor for Bevy 0.20 (GUI; headless check / regen / diff / migrate)"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    /// Project to open in the GUI.
    project: Option<PathBuf>,
    /// GUI window width in logical pixels, for this run only (not saved to the user config).
    #[arg(long, requires = "height", value_parser = clap::value_parser!(u32).range(320..))]
    width: Option<u32>,
    /// GUI window height in logical pixels, for this run only (not saved to the user config).
    #[arg(long, requires = "width", value_parser = clap::value_parser!(u32).range(240..))]
    height: Option<u32>,
}

#[derive(Subcommand)]
enum Command {
    /// Regenerate the domains' output and the `crates/quillshade` crate.
    Regen {
        project: PathBuf,
        /// Only this domain (repeatable; default: all). The umbrella crate files are always written.
        #[arg(long = "domain", value_name = "ID")]
        domains: Vec<String>,
        /// Also compile generated WESL against Bevy sources from the cargo registry.
        #[arg(long)]
        compile: bool,
    },
    /// Validate without writing.
    Check {
        project: PathBuf,
        /// Only this domain (repeatable; default: all).
        #[arg(long = "domain", value_name = "ID")]
        domains: Vec<String>,
        #[arg(long)]
        compile: bool,
    },
    /// Show what `regen` would change.
    Diff {
        project: PathBuf,
        /// Only this domain (repeatable; default: all).
        #[arg(long = "domain", value_name = "ID")]
        domains: Vec<String>,
    },
    /// Upgrade an older project to this editor's project version, then regenerate everything.
    Migrate {
        project: PathBuf,
        /// Only list the steps and the files they would migrate; write nothing.
        #[arg(long)]
        dry_run: bool,
    },
}

fn print_report(project: &Project, report: &RunReport) {
    for d in &report.diagnostics {
        eprintln!("{d}");
    }
    for c in &report.changes {
        let (tag, p) = match c {
            Change::Create(p) => ("create", p),
            Change::Modify(p) => ("modify", p),
            Change::Delete(p) => ("delete", p),
        };
        println!("{tag} {}", project.rel(p));
    }
}

/// The selected domains (design §10); an unknown id is an error naming the known ones, sorted. The editor's project
/// domain is not one of them (design §14.2).
fn select_domains(backends: &[&dyn DomainBackend], names: &[String]) -> Result<Vec<DomainId>, String> {
    let ids: Vec<DomainId> = backends.iter().map(|b| b.id()).filter(|id| *id != DomainId::PROJECT).collect();
    names
        .iter()
        .map(|n| {
            ids.iter().copied().find(|id| id.as_str() == n).ok_or_else(|| {
                let mut known: Vec<&str> = ids.iter().map(|id| id.as_str()).collect();
                known.sort_unstable();
                let known = known.join(", ");
                format!("unknown domain `{n}` (known: {known})")
            })
        })
        .collect()
}

/// Each domain's generator; a `--compile` plan finds the Bevy sources itself.
fn backends() -> Vec<Box<dyn DomainBackend>> {
    self::domains().iter().filter_map(|d| d.backend()).collect()
}

/// Prints an open error; an older project gets the `migrate` hint (ADR 0046 "CLI"). Exit code 2.
fn open_failed(path: &Path, e: &OpenError) -> ExitCode {
    eprintln!("error: {e}");
    if matches!(e, OpenError::NeedsMigration { .. }) {
        eprintln!("hint: run `shader-editor migrate {}`", path.display());
    }
    ExitCode::from(2)
}

fn headless(cmd: Command) -> ExitCode {
    let (path, domains, dry_run, compile) = match &cmd {
        Command::Regen { project, domains, compile } => (project, domains, false, *compile),
        Command::Check { project, domains, compile } => (project, domains, true, *compile),
        Command::Diff { project, domains } => (project, domains, true, false),
        Command::Migrate { project, dry_run } => return migrate(project, *dry_run),
    };
    let owned = backends();
    let backends: Vec<&dyn DomainBackend> = owned.iter().map(|b| b.as_ref()).collect();
    let only = match select_domains(&backends, domains) {
        Ok(only) => only,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };
    let project = match Project::open(path) {
        Ok(p) => p,
        Err(e) => return open_failed(path, &e),
    };
    let report = match run(&project, &backends, &only, compile, dry_run) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: write failed: {e}");
            return ExitCode::from(2);
        }
    };
    match cmd {
        Command::Check { .. } => {
            for d in &report.diagnostics {
                eprintln!("{d}");
            }
        }
        _ => print_report(&project, &report),
    }
    if report.has_errors() { ExitCode::from(1) } else { ExitCode::SUCCESS }
}

/// `migrate <project> [--dry-run]` (ADR 0046 "CLI"): exit 0 on success or when up to date, 1 on a migration error or
/// error diagnostics of the regen that follows a real run, 2 when the project cannot be opened or is too new.
fn migrate(path: &Path, dry_run: bool) -> ExitCode {
    let raw = match RawProject::open(path) {
        Ok(raw) => raw,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };
    // One registration list for GUI and CLI: the migrators and the compiled domains come from `domains()`.
    let domains = self::domains();
    let owned: Vec<Box<dyn DomainMigrator>> = domains.iter().filter_map(|d| d.migrator()).collect();
    let migrators: Vec<&dyn DomainMigrator> = owned.iter().map(|m| m.as_ref()).collect();
    let compiled: Vec<DomainId> = domains.iter().map(|d| d.id()).collect();
    let mut migrator = Migrator::new(raw, &migrators, &compiled);
    let failed = |e: &MigrationError| {
        eprintln!("error: {e}");
        match e {
            MigrationError::Open(_) | MigrationError::TooNew { .. } => ExitCode::from(2),
            MigrationError::Io { .. } | MigrationError::Domain { .. } => ExitCode::from(1),
        }
    };
    let plan = match migrator.plan() {
        Ok(plan) => plan,
        Err(e) => return failed(&e),
    };
    if plan.is_up_to_date() {
        println!("project is up to date ({})", plan.to);
        return ExitCode::SUCCESS;
    }
    if dry_run {
        println!("dry run: nothing is written");
        println!("project {} -> {}", plan.from, plan.to);
        for warning in &plan.warnings {
            eprintln!("warning: {warning}");
        }
        for step in &plan.steps {
            println!("step {}: {}", step.to, step.title);
            for file in &step.stale {
                println!("  migrate {} {}", file.domain, file.path);
            }
        }
        return ExitCode::SUCCESS;
    }
    match git_state(&migrator.project().root) {
        GitState::Clean => {}
        GitState::Dirty { changed } => eprintln!(
            "warning: {changed} uncommitted change(s) in the project; commit first so the migration can be reviewed \
             and reverted"
        ),
        GitState::NotARepo => eprintln!("warning: not a git repository; back up the project first"),
        GitState::Unavailable => eprintln!("note: cannot check the git state; back up the project first"),
    }
    println!("project {} -> {}", plan.from, plan.to);
    let result = migrator.run(&mut |event| match event {
        MigrationEvent::StepStarted { to, title, .. } => println!("step {to}: {title}"),
        MigrationEvent::FileMigrated { domain, path } => println!("  migrate {domain} {path}"),
        MigrationEvent::StepFinished { .. } => {}
        MigrationEvent::Warning(warning) => eprintln!("warning: {warning}"),
        MigrationEvent::Note(note) => println!("note: {note}"),
    });
    if let Err(e) = result {
        return failed(&e);
    }
    // Generated files are never migrated: regenerate every domain (like `regen` without a filter).
    let project = match Project::open(path) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: the migrated project does not open: {e}");
            return ExitCode::from(1);
        }
    };
    let owned = backends();
    let backends: Vec<&dyn DomainBackend> = owned.iter().map(|b| b.as_ref()).collect();
    let report = match run(&project, &backends, &[], false, false) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: write failed: {e}");
            return ExitCode::from(1);
        }
    };
    print_report(&project, &report);
    if report.has_errors() { ExitCode::from(1) } else { ExitCode::SUCCESS }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Some(cmd) => headless(cmd),
        None => {
            editor_core::run(domains(), cli.project, cli.width.zip(cli.height));
            ExitCode::SUCCESS
        }
    }
}
