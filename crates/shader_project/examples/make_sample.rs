//! Writes the reference graphs into a project and regenerates it:
//! `cargo run -p shader_project --example make_sample -- examples/sample_game`

use shader_project::assets::{AssetId, AssetType};
use shader_project::{Project, RegenOptions, fixtures, ops, regenerate};

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| "examples/sample_game".into());
    let project = Project::open(&dir).unwrap_or_else(|e| panic!("{e}"));
    for (path, is_fn, asset) in fixtures::all() {
        let id = AssetId::new(if is_fn { AssetType::Function } else { AssetType::Material }, path);
        ops::save(&project, &id, &asset).expect("save");
    }
    let report = regenerate(&project, &RegenOptions::default()).expect("regen");
    for d in report.diagnostics() {
        eprintln!("{d}");
    }
    println!("{} file(s) changed", report.changes.len());
}
