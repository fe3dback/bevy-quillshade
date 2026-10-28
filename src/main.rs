//! `shader-editor`: GUI without arguments; `regen` / `check` / `diff` run headless (spec §14).

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use shader_codegen_wesl::check::BevySources;
use shader_project::writer::Change;
use shader_project::{Project, RegenOptions, RegenReport, regenerate};

#[derive(Parser)]
#[command(name = "shader-editor", version = shader_editor_app::EDITOR_VERSION, about = "Node-based shader editor for Bevy 0.20")]
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
    /// Regenerate all WESL and the `crates/shaders` crate.
    Regen {
        project: PathBuf,
        /// Also compile generated WESL against Bevy sources from the cargo registry.
        #[arg(long)]
        compile: bool,
    },
    /// Validate without writing.
    Check {
        project: PathBuf,
        #[arg(long)]
        compile: bool,
    },
    /// Show what `regen` would change.
    Diff { project: PathBuf },
}

fn print_report(project: &Project, report: &RegenReport) {
    for d in report.diagnostics() {
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

fn bevy_sources(compile: bool) -> Option<BevySources> {
    if !compile {
        return None;
    }
    let s = BevySources::discover();
    if s.is_none() {
        eprintln!("warning: Bevy 0.20 WESL sources not found in the cargo registry; skipping compile check");
    }
    s
}

fn headless(cmd: Command) -> ExitCode {
    let (path, dry_run, compile) = match &cmd {
        Command::Regen { project, compile } => (project, false, *compile),
        Command::Check { project, compile } => (project, true, *compile),
        Command::Diff { project } => (project, true, false),
    };
    let project = match Project::open(path) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };
    let bevy = bevy_sources(compile);
    let opts = RegenOptions { dry_run, compile_check: bevy.as_ref(), ..Default::default() };
    let report = match regenerate(&project, &opts) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: write failed: {e}");
            return ExitCode::from(2);
        }
    };
    match cmd {
        Command::Check { .. } => {
            for d in report.diagnostics() {
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
            shader_editor_app::run(cli.project, cli.width.zip(cli.height));
            ExitCode::SUCCESS
        }
    }
}
