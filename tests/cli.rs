//! The headless CLI (design §10): `--domain` selection and exit codes.

use std::path::Path;
use std::process::{Command, Output};

/// A game crate with nothing in it yet: `Cargo.toml` and an empty `assets/`.
fn game() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"g\"\n").unwrap();
    std::fs::create_dir(dir.path().join("assets")).unwrap();
    dir
}

fn editor(args: &[&str], project: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_shader-editor")).args(&args[..1]).arg(project).args(&args[1..]).output().unwrap()
}

#[test]
fn unknown_domain_exits_2() {
    let dir = game();
    let out = editor(&["regen", "--domain", "nope"], dir.path());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("error: unknown domain `nope` (known: light, material)"), "{stderr}");
    assert!(!dir.path().join("crates/quillshade").exists());
    assert!(!dir.path().join("quillshade").exists());
}

#[test]
fn domain_filter_regenerates_material() {
    let dir = game();
    let out = editor(&["regen", "--domain", "material"], dir.path());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "{stderr}");
    assert!(dir.path().join("crates/quillshade/src/lib.rs").is_file());
    assert!(dir.path().join("crates/quillshade/src/material/mod.rs").is_file());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("create crates/quillshade/src/lib.rs"), "{stdout}");
}

/// `--domain light` selects the Light domain alone: its module and the umbrella, no Material output.
#[test]
fn cli_domain_light() {
    let dir = game();
    let out = editor(&["regen", "--domain", "light"], dir.path());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "{stderr}");
    assert!(dir.path().join("crates/quillshade/src/light/mod.rs").is_file());
    assert!(!dir.path().join("crates/quillshade/src/material").exists());
    let lib = std::fs::read_to_string(dir.path().join("crates/quillshade/src/lib.rs")).unwrap();
    assert!(lib.contains("pub mod light;") && lib.contains("pub mod material;"), "{lib}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("create crates/quillshade/src/light/mod.rs"), "{stdout}");
}

#[test]
fn malformed_project_ron_exits_2() {
    let dir = game();
    std::fs::create_dir(dir.path().join("quillshade")).unwrap();
    std::fs::write(dir.path().join("quillshade/project.ron"), "(format_version: 1, header_date: tru)").unwrap();
    let out = editor(&["regen"], dir.path());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("error: cannot read project settings `"), "{stderr}");
    assert!(stderr.contains("project.ron"), "{stderr}");
    assert!(!dir.path().join("crates/quillshade").exists());
}
