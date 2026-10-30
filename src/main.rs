//! `shader-editor`: GUI without arguments; `regen` / `check` / `diff` run headless (spec §14).

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use editor_core::domain::EditorDomain;
use project_core::DomainId;
use project_core::regen::{DomainBackend, RunReport, run};
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
    about = "Node-based shader editor for Bevy 0.20"
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

fn headless(cmd: Command) -> ExitCode {
    let (path, domains, dry_run, compile) = match &cmd {
        Command::Regen { project, domains, compile } => (project, domains, false, *compile),
        Command::Check { project, domains, compile } => (project, domains, true, *compile),
        Command::Diff { project, domains } => (project, domains, true, false),
    };
    // Each domain's generator; a `--compile` plan finds the Bevy sources itself.
    let owned: Vec<Box<dyn DomainBackend>> = self::domains().iter().filter_map(|d| d.backend()).collect();
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
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
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
