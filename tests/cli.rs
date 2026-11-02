//! The headless CLI (design §10): `--domain` selection and exit codes; `migrate` and the old / too-new project
//! refusals (ADR 0046 "CLI").

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
    std::fs::write(dir.path().join("quillshade/project.ron"), "(project_version: 3, header_date: tru)").unwrap();
    let out = editor(&["regen"], dir.path());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("error: cannot read project settings `"), "{stderr}");
    assert!(stderr.contains("project.ron"), "{stderr}");
    assert!(!dir.path().join("crates/quillshade").exists());
}

/// A v1 project (ADR 0046): `project.ron` still spells the version `format_version`, and a Material and a Light
/// curve are in format 1 (no `id`, ADR 0047).
fn old_game() -> tempfile::TempDir {
    let dir = game();
    std::fs::create_dir(dir.path().join("quillshade")).unwrap();
    std::fs::write(dir.path().join("quillshade/project.ron"), "ProjectSettings(\n    format_version: 1,\n)\n").unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("crates/project_migrate/tests/fixtures/v1/quillshade");
    for curve in ["material/x.curve.ron", "light/y.curve.ron"] {
        let to = dir.path().join("quillshade").join(curve);
        std::fs::create_dir_all(to.parent().unwrap()).unwrap();
        std::fs::copy(fixture.join(curve), to).unwrap();
    }
    dir
}

/// What the migration of [`old_game`] lists, step by step (the v2 step, then the v3 step).
const OLD_GAME_STEPS: &str = "step v2: Rename format_version to project_version in project.ron\n  migrate project \
                              quillshade/project.ron\n";
const OLD_GAME_V3: &str = "step v3: Assign asset UUIDs and reference assets by UUID\n  migrate light \
                           quillshade/light/y.curve.ron\n  migrate material quillshade/material/x.curve.ron\n  \
                           migrate project quillshade/project.ron\n";

/// Every file below `root`: relative path -> bytes.
fn snapshot(root: &Path) -> std::collections::BTreeMap<String, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, out: &mut std::collections::BTreeMap<String, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                let rel = path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
                out.insert(rel, std::fs::read(&path).unwrap());
            }
        }
    }
    let mut out = std::collections::BTreeMap::new();
    walk(root, root, &mut out);
    out
}

#[test]
fn migrate_dry_run_lists_and_writes_nothing() {
    let dir = old_game();
    let before = snapshot(dir.path());
    let out = editor(&["migrate", "--dry-run"], dir.path());
    let (stdout, stderr) = (String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert_eq!(out.status.code(), Some(0), "{stderr}");
    assert_eq!(stdout, format!("dry run: nothing is written\nproject v1 -> v3\n{OLD_GAME_STEPS}{OLD_GAME_V3}"));
    assert_eq!(snapshot(dir.path()), before);
}

#[test]
fn old_projects_are_refused_with_a_migrate_hint() {
    let dir = old_game();
    let before = snapshot(dir.path());
    for cmd in ["check", "regen", "diff"] {
        let out = editor(&[cmd], dir.path());
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(2), "{cmd}: {stderr}");
        assert!(stderr.contains("error: project version v1 is older than this editor (v3)"), "{cmd}: {stderr}");
        let hint = format!("hint: run `shader-editor migrate {}`", dir.path().display());
        assert!(stderr.contains(&hint), "{cmd}: {stderr}");
        assert_eq!(snapshot(dir.path()), before, "{cmd} wrote nothing");
    }
}

#[test]
fn migrate_upgrades_and_regenerates() {
    let dir = old_game();
    let out = editor(&["migrate"], dir.path());
    let (stdout, stderr) = (String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert_eq!(out.status.code(), Some(0), "{stderr}");
    let note = "note: project.ron rewritten: comments and unknown keys are not kept\n";
    assert!(stdout.starts_with(&format!("project v1 -> v3\n{OLD_GAME_STEPS}{note}{OLD_GAME_V3}")), "{stdout}");
    assert!(stdout.contains("create crates/quillshade/src/lib.rs"), "the regen follows: {stdout}");
    let ron = std::fs::read_to_string(dir.path().join("quillshade/project.ron")).unwrap();
    assert_eq!(ron, "ProjectSettings(\n  project_version: 3,\n  header_date: false,\n)\n");
    // Both curves have their asset UUID now (ADR 0047 §14), the one the migration derives from the file's path.
    for (curve, id) in [
        ("material/x.curve.ron", "019b76da-a800-716a-a76f-930aca3c075c"),
        ("light/y.curve.ron", "019b76da-a800-79f8-aadc-64f47ac16f49"),
    ] {
        let text = std::fs::read_to_string(dir.path().join("quillshade").join(curve)).unwrap();
        assert!(text.starts_with(&format!("Curve(\n  format_version: 2,\n  id: \"{id}\",\n")), "{curve}: {text}");
    }
    assert!(dir.path().join("assets/quillshade/material/autogen/curves/crv_x.wesl").is_file());
    assert!(dir.path().join("crates/quillshade/src/lib.rs").is_file());
    let check = editor(&["check"], dir.path());
    assert_eq!(check.status.code(), Some(0), "{}", String::from_utf8_lossy(&check.stderr));

    // A second run is up to date and changes nothing.
    let before = snapshot(dir.path());
    let again = editor(&["migrate"], dir.path());
    let stdout = String::from_utf8_lossy(&again.stdout);
    assert_eq!(again.status.code(), Some(0), "{}", String::from_utf8_lossy(&again.stderr));
    assert_eq!(stdout, "project is up to date (v3)\n");
    assert_eq!(snapshot(dir.path()), before);
}

/// ADR 0046 "Missing `project.ron`": sources without `project.ron` are a v1 project; `check` refuses it, `migrate
/// --dry-run` lists the file and writes nothing, `migrate` creates it, and a second run is up to date.
#[test]
fn migrate_creates_project_ron_for_a_legacy_project() {
    let dir = game();
    std::fs::create_dir_all(dir.path().join("quillshade/light")).unwrap();
    let before = snapshot(dir.path());
    let check = editor(&["check"], dir.path());
    let stderr = String::from_utf8_lossy(&check.stderr);
    assert_eq!(check.status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("error: project version v1 is older than this editor (v3)"), "{stderr}");
    assert!(stderr.contains("hint: run `shader-editor migrate "), "{stderr}");

    let dry = editor(&["migrate", "--dry-run"], dir.path());
    let stdout = String::from_utf8_lossy(&dry.stdout);
    assert_eq!(dry.status.code(), Some(0), "{}", String::from_utf8_lossy(&dry.stderr));
    assert!(stdout.contains("project v1 -> v3\n") && stdout.contains("  migrate project quillshade/project.ron\n"));
    assert_eq!(snapshot(dir.path()), before);
    assert!(!dir.path().join("quillshade/project.ron").exists());

    let out = editor(&["migrate"], dir.path());
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    let ron = std::fs::read_to_string(dir.path().join("quillshade/project.ron")).unwrap();
    assert_eq!(ron, "ProjectSettings(\n  project_version: 3,\n  header_date: false,\n)\n");
    let check = editor(&["check"], dir.path());
    assert_eq!(check.status.code(), Some(0), "{}", String::from_utf8_lossy(&check.stderr));
    let again = editor(&["migrate"], dir.path());
    assert_eq!(String::from_utf8_lossy(&again.stdout), "project is up to date (v3)\n");
}

/// Exit code 1 (ADR 0046 "CLI"): a migration error. A v1 `project.ron` whose version probes fine but whose settings
/// do not parse fails the project migrator, in the dry run's simulation and in the real run; nothing is written.
#[test]
fn migration_errors_exit_1() {
    let dir = game();
    std::fs::create_dir(dir.path().join("quillshade")).unwrap();
    let bad = "ProjectSettings(\n    format_version: 1,\n    header_date: \"yes\",\n)\n";
    std::fs::write(dir.path().join("quillshade/project.ron"), bad).unwrap();
    let before = snapshot(dir.path());
    for cmd in [&["migrate", "--dry-run"][..], &["migrate"]] {
        let out = editor(cmd, dir.path());
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(1), "{cmd:?}: {stderr}");
        assert!(stderr.contains("error: ") && stderr.contains("quillshade/project.ron"), "{cmd:?}: {stderr}");
        assert_eq!(snapshot(dir.path()), before, "{cmd:?} wrote nothing");
    }
}

/// ADR 0046: only v1 is spelled `format_version` and versions start at 1. Such a `project.ron` is refused with a
/// clear settings error (exit 2) by `migrate` as by `check`; `migrate` never calls it up to date.
#[test]
fn impossible_project_versions_exit_2() {
    for (text, needle) in [
        ("ProjectSettings(format_version: 2)\n", "`format_version: 2` is not a project version"),
        ("ProjectSettings(project_version: 0)\n", "versions start at 1"),
    ] {
        let dir = game();
        std::fs::create_dir(dir.path().join("quillshade")).unwrap();
        std::fs::write(dir.path().join("quillshade/project.ron"), text).unwrap();
        let before = snapshot(dir.path());
        for cmd in [&["migrate"][..], &["migrate", "--dry-run"], &["check"]] {
            let out = editor(cmd, dir.path());
            let (stdout, stderr) = (String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
            assert_eq!(out.status.code(), Some(2), "{text} {cmd:?}: {stderr}");
            assert!(stderr.contains(needle), "{text} {cmd:?}: {stderr}");
            assert!(!stdout.contains("up to date"), "{text} {cmd:?}: {stdout}");
        }
        assert_eq!(snapshot(dir.path()), before);
    }
}

#[test]
fn too_new_projects_exit_2() {
    let dir = game();
    std::fs::create_dir(dir.path().join("quillshade")).unwrap();
    std::fs::write(dir.path().join("quillshade/project.ron"), "ProjectSettings(project_version: 4)\n").unwrap();
    let before = snapshot(dir.path());
    for cmd in [&["migrate"][..], &["migrate", "--dry-run"], &["check"]] {
        let out = editor(cmd, dir.path());
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(2), "{cmd:?}: {stderr}");
        assert!(stderr.contains("error: project version v4 is newer than this editor supports (v3)"), "{stderr}");
    }
    assert_eq!(snapshot(dir.path()), before);
}

#[test]
fn migrate_outside_a_project_exits_2() {
    let dir = tempfile::tempdir().unwrap();
    let out = editor(&["migrate"], dir.path());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("is not a project"), "{stderr}");
}
