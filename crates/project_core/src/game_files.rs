//! The game's own files under `assets/` (design §4): what the editor's Game Assets tree lists. The editor's output
//! under `assets/quillshade/` is excluded.

use crate::layout::{OUTPUT_DIR, Project};

/// Files under `assets/` accepted by `keep` (called with the relative `/` path): sorted relative `/`-separated paths.
/// `assets/quillshade/**` (the editor's output), hidden files / directories and symlinked directories (a link loop
/// would never end) are skipped; symlinked files are listed.
pub fn scan(project: &Project, keep: impl Fn(&str) -> bool) -> Vec<String> {
    let root = project.assets_dir();
    let own = project.root.join(OUTPUT_DIR);
    let mut out = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            if e.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let p = e.path();
            let Ok(rel) = p.strip_prefix(&root) else { continue };
            let rel = rel.to_string_lossy().replace('\\', "/");
            let Ok(ty) = e.file_type() else { continue };
            if ty.is_dir() {
                if p != own {
                    stack.push(p);
                }
            } else if ty.is_symlink() && p.is_dir() {
                continue;
            } else if keep(&rel) {
                out.push(rel);
            }
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_skips_own_output_and_hidden() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"g\"\n").unwrap();
        for f in ["a.png", "quillshade/material/internal/default.png", ".x/b.png"] {
            let p = dir.path().join("assets").join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, b"x").unwrap();
        }
        let project = Project::open(dir.path()).unwrap();
        assert_eq!(scan(&project, |_| true), ["a.png"]);
    }

    /// A symlinked directory (here a loop back to `assets/`) is not followed; a symlinked file is listed.
    #[cfg(unix)]
    #[test]
    fn scan_skips_symlinked_dirs_and_recurses() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"g\"\n").unwrap();
        let assets = dir.path().join("assets");
        std::fs::create_dir_all(assets.join("env/deep")).unwrap();
        for f in ["env/rock.png", "env/deep/x.ktx2"] {
            std::fs::write(assets.join(f), b"x").unwrap();
        }
        std::os::unix::fs::symlink(&assets, assets.join("env/loop")).unwrap();
        std::os::unix::fs::symlink(assets.join("env/rock.png"), assets.join("linked.png")).unwrap();
        let project = Project::open(dir.path()).unwrap();
        assert_eq!(scan(&project, |_| true), ["env/deep/x.ktx2", "env/rock.png", "linked.png"]);
    }
}
