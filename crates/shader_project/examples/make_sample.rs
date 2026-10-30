//! Writes the reference graphs and curves into a project's `quillshade/material/` and regenerates it:
//! `cargo run -p shader_project --example make_sample -- examples/sample_game`

use shader_project::{Project, RegenOptions, fixtures, regenerate};

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| "examples/sample_game".into());
    let project = Project::open(&dir).unwrap_or_else(|e| panic!("{e}"));
    fixtures::save_all(&project).expect("save");
    let report = regenerate(&project, &RegenOptions::default()).expect("regen");
    for d in report.diagnostics() {
        eprintln!("{d}");
    }
    println!("{} file(s) changed", report.changes.len());
}
