//! Atomic batch writes (spec §12): every file goes to a temp file in its directory first, then a
//! series of renames; if any temp write fails, nothing is replaced. Contents are bytes: generated text and binary
//! files (internal textures, design §2) go through the same batch.

use std::path::{Path, PathBuf};

/// Planned change of one file.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Change {
    Create(PathBuf),
    Modify(PathBuf),
    Delete(PathBuf),
}

impl Change {
    pub fn path(&self) -> &Path {
        match self {
            Change::Create(p) | Change::Modify(p) | Change::Delete(p) => p,
        }
    }
}

#[derive(Debug, Default)]
pub struct WriteBatch {
    /// Files to (re)write with their full contents; callers add only files whose bytes differ from the disk.
    pub writes: Vec<(PathBuf, Vec<u8>)>,
    pub deletes: Vec<PathBuf>,
}

impl WriteBatch {
    pub fn is_empty(&self) -> bool {
        self.writes.is_empty() && self.deletes.is_empty()
    }

    /// Changes this batch would make (for `diff`).
    pub fn changes(&self) -> Vec<Change> {
        let mut c: Vec<Change> = self
            .writes
            .iter()
            .map(|(p, _)| if p.exists() { Change::Modify(p.clone()) } else { Change::Create(p.clone()) })
            .collect();
        c.extend(self.deletes.iter().cloned().map(Change::Delete));
        c.sort();
        c
    }

    /// Applies the batch. `cleanup_roots` are directories whose empty sub-directories are removed.
    pub fn apply(&self, cleanup_roots: &[PathBuf]) -> std::io::Result<()> {
        let mut temps: Vec<(PathBuf, PathBuf)> = Vec::new();
        let result = (|| -> std::io::Result<()> {
            for (path, bytes) in &self.writes {
                let dir = path.parent().ok_or_else(|| std::io::Error::other("file without parent"))?;
                std::fs::create_dir_all(dir)?;
                let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                let tmp = dir.join(format!(".{name}.tmp-{}", std::process::id()));
                std::fs::write(&tmp, bytes)?;
                temps.push((tmp, path.clone()));
            }
            Ok(())
        })();
        if let Err(e) = result {
            for (tmp, _) in &temps {
                let _ = std::fs::remove_file(tmp);
            }
            return Err(e);
        }
        for (tmp, path) in &temps {
            std::fs::rename(tmp, path)?;
        }
        for p in &self.deletes {
            match std::fs::remove_file(p) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
        }
        for root in cleanup_roots {
            remove_empty_dirs(root);
        }
        Ok(())
    }
}

/// Removes empty directories below `root` (not `root` itself). Symlinked directories are not followed (a link loop,
/// or directories outside the project); a link keeps its directory non-empty.
pub fn remove_empty_dirs(root: &Path) {
    fn walk(dir: &Path) -> bool {
        let Ok(rd) = std::fs::read_dir(dir) else { return false };
        let mut empty = true;
        for e in rd.flatten() {
            let p = e.path();
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                if walk(&p) {
                    let _ = std::fs::remove_dir(&p);
                } else {
                    empty = false;
                }
            } else {
                empty = false;
            }
        }
        empty
    }
    walk(root);
}
