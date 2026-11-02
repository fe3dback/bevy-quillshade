//! Domain-neutral regeneration (design §6.5): every domain plans its files, then [`commit`] compares them with the
//! disk, refuses to overwrite foreign files (E008, spec §12), deletes orphans inside each domain's own roots and writes
//! one atomic batch together with the umbrella crate files (design §10: written by every run, whatever domains are
//! selected). Every run first builds the project's [`AssetIndex`] from all compiled-in backends (ADR 0047 §9);
//! [`run_affected`] regenerates only what a set of changed assets affects, across domains (techdoc §4.3).

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::PathBuf;

use crate::asset_index::{AssetIndex, DomainIndex};
use crate::asset_ref::AssetRef;
use crate::diag::Diagnostic;
use crate::domain::DomainId;
use crate::files::walk_files;
use crate::layout::Project;
use crate::marker::{civil_today, has_marker};
use crate::umbrella::{self, CrateContribution};
use crate::writer::{Change, WriteBatch};

/// One generated file.
#[derive(Debug, Clone)]
pub struct OutFile {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
    /// Asset key the domain attributes an E008 on this file to (e.g. its `source_rel`); `None` = the domain / project.
    pub owner: Option<String>,
}

/// A directory a domain owns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutRoot {
    pub dir: PathBuf,
    /// Owned through [`crate::files::OWNED_DIR_MARKER`]: every file in it is ours, there is no marker check per file.
    pub binary: bool,
}

/// What one domain wants on disk.
#[derive(Debug, Clone)]
pub struct DomainOutput {
    pub domain: DomainId,
    /// In write order.
    pub files: Vec<OutFile>,
    /// Existing files to keep although they are not produced this time (sources with errors, out of scope).
    pub keep: BTreeSet<PathBuf>,
    /// Directories this domain owns: the orphan scan and the empty-directory cleanup run only there.
    pub roots: Vec<OutRoot>,
    /// Delete orphans in `roots` (a full regeneration of the domain).
    pub orphan_scan: bool,
}

/// A file not written because it may belong to the user (E008).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blocked {
    /// `None` for the umbrella crate files.
    pub domain: Option<DomainId>,
    pub path: PathBuf,
    pub owner: Option<String>,
    /// The E008 text.
    pub message: String,
}

#[derive(Debug, Clone, Default)]
pub struct Committed {
    /// What the batch changes (or would change, in a dry run).
    pub changes: Vec<Change>,
    pub blocked: Vec<Blocked>,
}

/// Compares the domains' files and the umbrella crate files with the disk and writes the difference in one batch
/// (unless `dry_run`). `crates` are every compiled-in domain's contributions, so the umbrella is always complete.
pub fn commit(
    project: &Project,
    outputs: &[DomainOutput],
    crates: &[CrateContribution],
    dry_run: bool,
) -> io::Result<Committed> {
    // A domain owns only its own folders (design §6.5): an orphan scan elsewhere could delete another domain's files.
    debug_assert!(
        outputs.iter().all(|o| o.roots.iter().all(|r| {
            r.dir.starts_with(project.output_dir(o.domain)) || r.dir.starts_with(project.module_dir(o.domain))
        })),
        "a domain root lies outside its output / module directory"
    );
    let date = project.settings.header_date.then(civil_today);
    let umbrella = umbrella::files(project, crates, date.as_deref())
        .into_iter()
        .map(|(path, text)| (None, OutFile { path, bytes: text.into_bytes(), owner: None }));
    let desired: Vec<(Option<DomainId>, OutFile)> = outputs
        .iter()
        .flat_map(|o| {
            let enabled = project.domain_enabled(o.domain);
            let module_dir = project.module_dir(o.domain);
            o.files
                .iter()
                .filter(move |f| enabled || !f.path.starts_with(&module_dir))
                .map(move |f| (Some(o.domain), f.clone()))
        })
        .chain(umbrella)
        .collect();
    let binary_roots: Vec<&PathBuf> =
        outputs.iter().flat_map(|o| &o.roots).filter(|r| r.binary).map(|r| &r.dir).collect();
    let disabled_roots: Vec<OutRoot> = crates
        .iter()
        .map(|c| c.domain)
        .chain(outputs.iter().map(|o| o.domain))
        .filter(|&d| !project.domain_enabled(d))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|d| OutRoot { dir: project.module_dir(d), binary: false })
        .collect();

    // Compare with disk, refuse to overwrite foreign files (E008), binary ones included. Files in the binary roots
    // are owned through the directory marker (the domain produces them only when it is present).
    let mut batch = WriteBatch::default();
    let mut committed = Committed::default();
    let desired_paths: BTreeSet<PathBuf> = desired.iter().map(|(_, f)| f.path.clone()).collect();
    for (domain, OutFile { path, bytes, owner }) in desired {
        let message = match std::fs::read(&path) {
            Ok(old) if old == bytes => continue,
            Ok(old)
                if !binary_roots.iter().any(|d| path.starts_with(d))
                    && !std::str::from_utf8(&old).is_ok_and(has_marker) =>
            {
                format!("`{}` has no AUTOGENERATED marker; not overwriting", project.rel(&path))
            }
            Ok(_) => {
                batch.writes.push((path, bytes));
                continue;
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                batch.writes.push((path, bytes));
                continue;
            }
            // Unreadable (permissions, a directory in the way, …): it may be a foreign file, never replace it.
            Err(e) => format!("`{}` cannot be read ({e}); not overwriting", project.rel(&path)),
        };
        committed.blocked.push(Blocked { domain, path, owner, message });
    }

    // Orphans (a domain's full regeneration only, inside its own roots, plus disabled domains' module dirs): marker
    // files that no source produces any more; in a binary root every file nobody produces. A file under two nested
    // roots is deleted once.
    let keep: BTreeSet<&PathBuf> = outputs
        .iter()
        .flat_map(|o| {
            let enabled = project.domain_enabled(o.domain);
            let module_dir = project.module_dir(o.domain);
            o.keep.iter().filter(move |p| enabled || !p.starts_with(&module_dir))
        })
        .collect();
    let mut deleted: BTreeSet<PathBuf> = BTreeSet::new();
    for root in outputs.iter().filter(|o| o.orphan_scan).flat_map(|o| &o.roots).chain(&disabled_roots) {
        for f in walk_files(&root.dir) {
            if desired_paths.contains(&f) || keep.contains(&f) || deleted.contains(&f) {
                continue;
            }
            if f.file_name().is_some_and(|n| n.to_string_lossy().contains(".tmp-")) {
                continue;
            }
            if root.binary || std::fs::read_to_string(&f).is_ok_and(|t| has_marker(&t)) {
                deleted.insert(f.clone());
                batch.deletes.push(f);
            }
        }
    }

    committed.changes = batch.changes();
    if !dry_run && !batch.is_empty() {
        let roots: Vec<PathBuf> =
            outputs.iter().flat_map(|o| &o.roots).chain(&disabled_roots).map(|r| r.dir.clone()).collect();
        batch.apply(&roots)?;
    }
    Ok(committed)
}

/// Writes only the umbrella crate files (`Cargo.toml`, `src/lib.rs`) from `project`'s settings, with no domain output
/// and no orphan scan: what saving the project settings rewrites (design §14.2). A foreign file there is
/// [`Committed::blocked`] (E008), not overwritten.
pub fn write_umbrella(project: &Project, crates: &[CrateContribution]) -> io::Result<Committed> {
    commit(project, &[], crates, false)
}

/// A domain's generator (design §6.5). `Send + Sync`: the editor's Migration Tool regenerates the migrated project on
/// its background thread (ADR 0046 "Editor (Migration Tool modal)").
pub trait DomainBackend: Send + Sync {
    fn id(&self) -> DomainId;
    /// Its module of the umbrella crate.
    fn crate_contribution(&self) -> CrateContribution;
    /// The domain's part of the project's [`AssetIndex`] (ADR 0047 §7): its current-format source files with their
    /// ids, sub-items and references. Called for every compiled-in backend, enabled or not.
    fn index(&self, project: &Project) -> DomainIndex;
    /// Loads, validates and generates the whole domain; `compile` also runs its compile check (design §10). `index`
    /// spans every compiled-in domain: references are resolved against it.
    fn plan(&self, project: &Project, index: &AssetIndex, compile: bool) -> DomainPlan;
    /// Like [`DomainBackend::plan`], but only `affected` (the domain's assets among [`AssetIndex::affected`] of a
    /// change, the changed ones included) need to be regenerated: the files of the domain's other assets may be kept
    /// as they are ([`DomainOutput::keep`]). [`run_affected`] never lets it orphan-scan. By default the whole domain.
    fn plan_affected(
        &self,
        project: &Project,
        index: &AssetIndex,
        _affected: &BTreeSet<AssetRef>,
        compile: bool,
    ) -> DomainPlan {
        self.plan(project, index, compile)
    }
}

/// A domain's diagnostics and files.
#[derive(Debug, Clone)]
pub struct DomainPlan {
    pub diagnostics: Vec<Diagnostic>,
    pub output: DomainOutput,
}

#[derive(Debug, Clone, Default)]
pub struct RunReport {
    pub diagnostics: Vec<Diagnostic>,
    pub changes: Vec<Change>,
}

impl RunReport {
    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(Diagnostic::is_error)
    }
}

/// Regenerates the domains in `only` (all when empty) and the umbrella crate (design §10). The [`AssetIndex`] is built
/// from every backend in `backends`, enabled or not (a reference into a disabled domain is AE003). Diagnostics: W105
/// first, then the index's (`AE###`) of the selected enabled domains, then each selected domain's in backend order,
/// then one E008 per blocked file. Errors only on I/O failure of the final write.
pub fn run(
    project: &Project,
    backends: &[&dyn DomainBackend],
    only: &[DomainId],
    compile: bool,
    dry_run: bool,
) -> io::Result<RunReport> {
    let crates: Vec<CrateContribution> = backends.iter().map(|b| b.crate_contribution()).collect();
    let mut report = RunReport::default();
    if let Some(text) = project.workspace_problem() {
        report.diagnostics.push(Diagnostic::warning("W105", text).in_asset("Cargo.toml"));
    }
    let selected = |d: DomainId| (only.is_empty() || only.contains(&d)) && project.domain_enabled(d);
    let index = AssetIndex::build(backends.iter().map(|b| b.index(project)).collect());
    report.diagnostics.extend(index.diagnostics(project).into_iter().filter(|d| d.domain.is_some_and(selected)));
    let mut outputs = Vec::new();
    for b in backends.iter().filter(|b| selected(b.id())) {
        let plan = b.plan(project, &index, compile);
        report.diagnostics.extend(plan.diagnostics);
        outputs.push(plan.output);
    }
    finish(project, report, &outputs, &crates, dry_run)
}

/// Regenerates what changing `changed` affects, across domains (techdoc §4.3, ADR 0047 §9): the [`AssetIndex`] of
/// every backend gives [`AssetIndex::affected`] (`changed` plus its transitive referrers, whatever their domain); each
/// enabled domain with assets in it plans them ([`DomainBackend::plan_affected`], in backend order), and one
/// [`commit`] writes the result and the umbrella crate files. No domain orphan-scans (out-of-scope files are not
/// orphans). Diagnostics: W105 first, then the index's (`AE###`) of the replanned domains, then theirs, then E008.
/// A domain with none of its assets affected is not planned. Errors only on I/O failure of the final write.
pub fn run_affected(
    project: &Project,
    backends: &[&dyn DomainBackend],
    changed: &BTreeSet<AssetRef>,
    compile: bool,
    dry_run: bool,
) -> io::Result<RunReport> {
    let crates: Vec<CrateContribution> = backends.iter().map(|b| b.crate_contribution()).collect();
    let mut report = RunReport::default();
    if let Some(text) = project.workspace_problem() {
        report.diagnostics.push(Diagnostic::warning("W105", text).in_asset("Cargo.toml"));
    }
    let index = AssetIndex::build(backends.iter().map(|b| b.index(project)).collect());
    let mut by_domain: BTreeMap<DomainId, BTreeSet<AssetRef>> = BTreeMap::new();
    for r in index.affected(changed) {
        by_domain.entry(r.domain).or_default().insert(r);
    }
    let selected = |d: DomainId| by_domain.contains_key(&d) && project.domain_enabled(d);
    report.diagnostics.extend(index.diagnostics(project).into_iter().filter(|d| d.domain.is_some_and(selected)));
    let mut outputs = Vec::new();
    for b in backends.iter().filter(|b| selected(b.id())) {
        let mut plan = b.plan_affected(project, &index, &by_domain[&b.id()], compile);
        plan.output.orphan_scan = false;
        report.diagnostics.extend(plan.diagnostics);
        outputs.push(plan.output);
    }
    finish(project, report, &outputs, &crates, dry_run)
}

/// The end of [`run`] / [`run_affected`]: [`commit`], one E008 per blocked file, the changes.
fn finish(
    project: &Project,
    mut report: RunReport,
    outputs: &[DomainOutput],
    crates: &[CrateContribution],
    dry_run: bool,
) -> io::Result<RunReport> {
    let committed = commit(project, outputs, crates, dry_run)?;
    for b in committed.blocked {
        let mut d = Diagnostic::error("E008", b.message);
        if let Some(domain) = b.domain {
            d = d.in_domain(domain);
        }
        if let Some(owner) = b.owner {
            d = d.in_asset(owner);
        }
        report.diagnostics.push(d);
    }
    report.changes = committed.changes;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::asset_index::IndexedAsset;
    use crate::asset_ref::AssetRef;
    use crate::asset_uuid::AssetUuid;
    use crate::files::OWNED_DIR_MARKER;
    use crate::marker::header;

    const A: DomainId = DomainId::new("alpha");
    const B: DomainId = DomainId::new("beta");

    /// Test backend: marker text files under its output and module directories, optionally an owned binary root
    /// `bin/` under its output directory; `assets` is its index part, `seen` the ids of the index `plan` received,
    /// `affected` the paths `plan_affected` received (it then plans everything, orphan scan included).
    struct Fake {
        id: DomainId,
        files: Vec<&'static str>,
        binary: bool,
        assets: Vec<IndexedAsset>,
        seen: Mutex<Option<Vec<AssetUuid>>>,
        affected: Mutex<Option<Vec<String>>>,
    }

    impl Fake {
        fn new(id: DomainId, files: &[&'static str]) -> Fake {
            Fake {
                id,
                files: files.to_vec(),
                binary: false,
                assets: Vec::new(),
                seen: Mutex::new(None),
                affected: Mutex::new(None),
            }
        }

        fn affected(&self) -> Option<Vec<String>> {
            self.affected.lock().unwrap().clone()
        }
    }

    impl DomainBackend for Fake {
        fn id(&self) -> DomainId {
            self.id
        }

        fn crate_contribution(&self) -> CrateContribution {
            CrateContribution { domain: self.id, plugin: "FakePlugin", bevy_features: &[], dependencies: &[] }
        }

        fn index(&self, _project: &Project) -> DomainIndex {
            DomainIndex { domain: self.id, assets: self.assets.clone() }
        }

        fn plan(&self, project: &Project, index: &AssetIndex, _compile: bool) -> DomainPlan {
            let ids = [A, B].into_iter().flat_map(|d| index.assets_of(d, "x").map(|e| e.id)).collect();
            *self.seen.lock().unwrap() = Some(ids);
            let out = project.output_dir(self.id);
            let module = project.module_dir(self.id);
            let mut files = Vec::new();
            for f in &self.files {
                for dir in [&out, &module] {
                    let text = format!("{}\n{f}\n", header("//", Some(f)));
                    files.push(OutFile { path: dir.join(f), bytes: text.into_bytes(), owner: Some(f.to_string()) });
                }
            }
            let mut roots = vec![OutRoot { dir: out.clone(), binary: false }, OutRoot { dir: module, binary: false }];
            if self.binary {
                let bin = out.join("bin");
                let marker = format!("{}\n", header("#", None));
                files.push(OutFile { path: bin.join(OWNED_DIR_MARKER), bytes: marker.into_bytes(), owner: None });
                roots.push(OutRoot { dir: bin, binary: true });
            }
            DomainPlan {
                diagnostics: Vec::new(),
                output: DomainOutput { domain: self.id, files, keep: BTreeSet::new(), roots, orphan_scan: true },
            }
        }

        fn plan_affected(
            &self,
            project: &Project,
            index: &AssetIndex,
            affected: &BTreeSet<AssetRef>,
            compile: bool,
        ) -> DomainPlan {
            *self.affected.lock().unwrap() = Some(affected.iter().map(|r| r.path.clone()).collect());
            self.plan(project, index, compile)
        }
    }

    fn project() -> (tempfile::TempDir, Project) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[workspace]\nmembers = [\"crates/*\"]\n").unwrap();
        std::fs::create_dir(dir.path().join("assets")).unwrap();
        let p = Project::open(dir.path()).unwrap();
        (dir, p)
    }

    #[test]
    fn regenerating_one_domain_keeps_the_other() {
        let (_d, p) = project();
        let (a, b) = (Fake::new(A, &["a.txt"]), Fake::new(B, &["b.txt"]));
        let r = run(&p, &[&a, &b], &[], false, false).unwrap();
        assert!(!r.has_errors(), "{:?}", r.diagnostics);
        for f in [p.output_dir(A).join("a.txt"), p.module_dir(A).join("a.txt"), p.output_dir(B).join("b.txt")] {
            assert!(f.is_file(), "{}", f.display());
        }
        let empty_a = Fake::new(A, &[]);
        let r = run(&p, &[&empty_a, &b], &[A], false, false).unwrap();
        assert!(!r.has_errors(), "{:?}", r.diagnostics);
        assert!(p.output_dir(B).join("b.txt").is_file() && p.module_dir(B).join("b.txt").is_file());
        assert!(!p.output_dir(A).join("a.txt").exists() && !p.module_dir(A).join("a.txt").exists());
        assert!(r.changes.contains(&Change::Delete(p.output_dir(A).join("a.txt"))), "{:?}", r.changes);
        // The umbrella still names both domains.
        let lib = std::fs::read_to_string(p.crate_dir().join("src/lib.rs")).unwrap();
        assert!(lib.contains("pub mod alpha;") && lib.contains("pub mod beta;"), "{lib}");
    }

    #[test]
    fn foreign_umbrella_file_is_blocked() {
        let (_d, p) = project();
        let lib = p.crate_dir().join("src/lib.rs");
        std::fs::create_dir_all(lib.parent().unwrap()).unwrap();
        std::fs::write(&lib, "// mine\n").unwrap();
        let a = Fake::new(A, &["a.txt"]);
        let r = run(&p, &[&a], &[], false, false).unwrap();
        let e008: Vec<_> = r.diagnostics.iter().filter(|d| d.code == "E008").collect();
        assert_eq!(e008.len(), 1, "{:?}", r.diagnostics);
        assert_eq!(e008[0].domain, None);
        assert!(e008[0].is_error() && e008[0].message.contains("crates/quillshade/src/lib.rs"), "{:?}", e008[0]);
        assert_eq!(std::fs::read_to_string(&lib).unwrap(), "// mine\n");
        assert!(p.output_dir(A).join("a.txt").is_file() && p.crate_dir().join("Cargo.toml").is_file());
    }

    #[test]
    fn binary_root_files_need_no_marker() {
        let (_d, p) = project();
        let bin = p.output_dir(A).join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join(OWNED_DIR_MARKER), format!("{}\n", header("#", None))).unwrap();
        std::fs::write(bin.join("x.png"), [0x89, b'P', b'N', b'G']).unwrap();
        // A foreign file outside the binary root is never deleted.
        std::fs::write(p.output_dir(A).join("foreign.txt"), "mine\n").unwrap();
        let a = Fake { binary: true, ..Fake::new(A, &["a.txt"]) };
        let r = run(&p, &[&a], &[], false, false).unwrap();
        assert!(!r.has_errors(), "{:?}", r.diagnostics);
        assert!(!bin.join("x.png").exists());
        assert!(bin.join(OWNED_DIR_MARKER).is_file() && p.output_dir(A).join("foreign.txt").is_file());
    }

    /// The umbrella files and disabled domains' module dirs only: a domain's output files and orphans stay.
    #[test]
    fn write_umbrella_touches_only_the_umbrella() {
        let (_d, p) = project();
        let a = Fake::new(A, &["a.txt"]);
        run(&p, &[&a], &[], false, false).unwrap();
        let orphan = p.output_dir(A).join("old.txt");
        std::fs::write(&orphan, format!("{}\nold\n", header("//", None))).unwrap();
        let mut off = p.clone();
        off.settings.domains.insert(A.as_str().into(), crate::layout::DomainEntry { enabled: false });
        let c = write_umbrella(&off, &[a.crate_contribution()]).unwrap();
        assert_eq!(
            c.changes,
            [Change::Modify(p.crate_dir().join("src/lib.rs")), Change::Delete(p.module_dir(A).join("a.txt"))],
        );
        assert!(c.blocked.is_empty());
        let lib = std::fs::read_to_string(p.crate_dir().join("src/lib.rs")).unwrap();
        assert!(!lib.contains("pub mod alpha;") && !lib.contains("alpha::FakePlugin"), "{lib}");
        assert!(!p.module_dir(A).exists());
        assert!(p.output_dir(A).join("a.txt").is_file());
        assert!(orphan.is_file(), "no orphan scan in output_dir");
    }

    #[test]
    fn run_skips_disabled_domain_and_removes_its_module_dir() {
        let (_d, mut p) = project();
        let (a, b) = (Fake::new(A, &["mod.rs"]), Fake::new(B, &["mod.rs"]));
        // Any domain not listed in `project.ron` is enabled by default.
        assert!(p.domain_enabled(A) && p.domain_enabled(B));
        run(&p, &[&a, &b], &[], false, false).unwrap();
        assert!(p.module_dir(A).join("mod.rs").is_file() && p.module_dir(B).join("mod.rs").is_file());

        // Disable `alpha` while leaving `beta` unlisted in `domains` (enabled by default).
        p.settings.domains.insert(A.as_str().into(), crate::layout::DomainEntry { enabled: false });
        assert!(!p.domain_enabled(A) && p.domain_enabled(B));

        let a_next = Fake::new(A, &["mod.rs", "extra.rs"]);
        let r = run(&p, &[&a_next, &b], &[], false, false).unwrap();
        assert!(!r.has_errors(), "{:?}", r.diagnostics);
        assert!(!p.module_dir(A).exists(), "disabled domain's module dir should be deleted");
        assert!(!p.output_dir(A).join("extra.rs").exists(), "disabled domain should not plan new files");
        assert!(p.output_dir(A).join("mod.rs").is_file(), "existing output_dir files are kept");
        assert!(p.module_dir(B).join("mod.rs").is_file());
        let lib = std::fs::read_to_string(p.crate_dir().join("src/lib.rs")).unwrap();
        assert!(!lib.contains("pub mod alpha;") && lib.contains("pub mod beta;"), "{lib}");
    }

    #[test]
    fn dry_run_writes_nothing() {
        let (_d, p) = project();
        let a = Fake::new(A, &["a.txt"]);
        let r = run(&p, &[&a], &[], false, true).unwrap();
        assert!(!r.changes.is_empty());
        assert!(r.changes.contains(&Change::Create(p.output_dir(A).join("a.txt"))), "{:?}", r.changes);
        assert!(!p.output_dir(A).exists() && !p.crate_dir().exists());
    }

    #[test]
    fn workspace_warning_comes_first() {
        let (_d, p) = project();
        std::fs::write(p.root.join("Cargo.toml"), "[workspace]\nmembers = [\"game\"]\n").unwrap();
        let a = Fake::new(A, &[]);
        let r = run(&p, &[&a], &[], false, true).unwrap();
        assert_eq!(r.diagnostics[0].code, "W105");
        assert_eq!(r.diagnostics[0].asset.as_deref(), Some("Cargo.toml"));
        assert!(!r.has_errors());
    }

    fn x(domain: DomainId, path: &str, id: u128, to: &[u128]) -> IndexedAsset {
        let uuid = |n: u128| AssetUuid::from_u128(0x0190_0000_0000_7000_8000_0000_0000_0000 | n);
        IndexedAsset {
            asset: AssetRef { domain, path: path.into(), suffix: "x" },
            id: Ok(uuid(id)),
            sub_items: Vec::new(),
            refs: to
                .iter()
                .map(|&n| crate::asset_index::IndexedRef { to: uuid(n), sub: None, location: Some("f".into()) })
                .collect(),
        }
    }

    /// Every `plan` gets the index of all backends, unselected and disabled ones included.
    #[test]
    fn plan_receives_the_index_of_every_backend() {
        let (_d, mut p) = project();
        p.settings.domains.insert(B.as_str().into(), crate::layout::DomainEntry { enabled: false });
        let a = Fake { assets: vec![x(A, "one", 1, &[2])], ..Fake::new(A, &[]) };
        let b = Fake { assets: vec![x(B, "two", 2, &[])], ..Fake::new(B, &[]) };
        let r = run(&p, &[&a, &b], &[], false, true).unwrap();
        let seen = a.seen.lock().unwrap().clone().expect("alpha was planned");
        assert_eq!(seen.len(), 2, "{seen:?}");
        assert!(b.seen.lock().unwrap().is_none(), "a disabled domain is not planned");
        // The reference into the disabled domain is AE003.
        let codes: Vec<_> = r.diagnostics.iter().map(|d| d.code.as_str()).collect();
        assert_eq!(codes, ["AE003"]);
    }

    /// Index diagnostics come right after W105, only for the selected enabled domains, before the domains' own.
    #[test]
    fn index_diagnostics_are_in_the_report() {
        let (_d, p) = project();
        std::fs::write(p.root.join("Cargo.toml"), "[workspace]\nmembers = [\"game\"]\n").unwrap();
        let a = Fake { assets: vec![x(A, "one", 1, &[9]), x(A, "copy", 1, &[])], ..Fake::new(A, &[]) };
        let b = Fake { assets: vec![x(B, "two", 2, &[8])], ..Fake::new(B, &[]) };
        let r = run(&p, &[&a, &b], &[], false, true).unwrap();
        let got: Vec<_> =
            r.diagnostics.iter().map(|d| (d.code.as_str(), d.domain, d.asset.as_deref().unwrap_or("-"))).collect();
        assert_eq!(
            got,
            [
                ("W105", None, "Cargo.toml"),
                ("AE001", Some(A), "quillshade/alpha/one.x.ron"),
                ("AE002", Some(A), "quillshade/alpha/one.x.ron"),
                ("AE002", Some(B), "quillshade/beta/two.x.ron"),
            ]
        );
        assert!(r.has_errors());
        let only_b = run(&p, &[&a, &b], &[B], false, true).unwrap();
        let codes: Vec<_> = only_b.diagnostics.iter().map(|d| (d.code.as_str(), d.domain)).collect();
        assert_eq!(codes, [("W105", None), ("AE002", Some(B))]);
    }

    fn changed(domain: DomainId, path: &str) -> BTreeSet<AssetRef> {
        BTreeSet::from([AssetRef { domain, path: path.into(), suffix: "x" }])
    }

    /// Only the domains with affected assets are replanned, each with its own affected assets; the others' files and
    /// their index diagnostics stay out of the run.
    #[test]
    fn run_affected_replans_only_affected_domains() {
        let (_d, p) = project();
        // beta/two refers to alpha/one; alpha/lone and beta/other refer to nothing (beta/other dangles: AE002).
        let a = Fake { assets: vec![x(A, "one", 1, &[]), x(A, "lone", 3, &[])], ..Fake::new(A, &["a.txt"]) };
        let b = Fake { assets: vec![x(B, "two", 2, &[1]), x(B, "other", 4, &[9])], ..Fake::new(B, &["b.txt"]) };
        let r = run_affected(&p, &[&a, &b], &changed(A, "one"), false, true).unwrap();
        assert_eq!(a.affected(), Some(vec!["one".to_string()]));
        assert_eq!(b.affected(), Some(vec!["two".to_string()]), "the referrer in the other domain");
        assert!(a.seen.lock().unwrap().is_some(), "the default `plan_affected` is the full plan");
        let codes: Vec<_> = r.diagnostics.iter().map(|d| (d.code.as_str(), d.domain)).collect();
        assert_eq!(codes, [("AE002", Some(B))], "the index diagnostics of the replanned domains");
        assert!(r.changes.contains(&Change::Create(p.output_dir(B).join("b.txt"))), "{:?}", r.changes);

        // A change nothing refers to replans its own domain only.
        let (a, b) = (Fake { assets: a.assets.clone(), ..Fake::new(A, &["a.txt"]) }, Fake::new(B, &["b.txt"]));
        let r = run_affected(&p, &[&a, &b], &changed(A, "lone"), false, true).unwrap();
        assert_eq!((a.affected(), b.affected()), (Some(vec!["lone".to_string()]), None));
        assert!(b.seen.lock().unwrap().is_none(), "beta is not planned at all");
        assert!(!r.changes.iter().any(|c| c.path().starts_with(p.output_dir(B))), "{:?}", r.changes);
        // The umbrella is written by every run.
        assert!(r.changes.contains(&Change::Create(p.crate_dir().join("src/lib.rs"))), "{:?}", r.changes);

        // A change outside the index (a legacy file) is still its own domain's; a disabled domain is never planned.
        let mut off = p.clone();
        off.settings.domains.insert(B.as_str().into(), crate::layout::DomainEntry { enabled: false });
        let (a, b) = (Fake::new(A, &[]), Fake::new(B, &[]));
        run_affected(&off, &[&a, &b], &changed(A, "legacy"), false, true).unwrap();
        run_affected(&off, &[&a, &b], &changed(B, "legacy"), false, true).unwrap();
        assert_eq!((a.affected(), b.affected()), (Some(vec!["legacy".to_string()]), None));
    }

    /// alpha/one <- beta/two <- alpha/three <- beta/four: a change of alpha/one reaches all four, across the domain
    /// boundary twice.
    #[test]
    fn run_affected_is_transitive_across_domains() {
        let (_d, p) = project();
        let a =
            Fake { assets: vec![x(A, "one", 1, &[]), x(A, "three", 3, &[2]), x(A, "x", 5, &[])], ..Fake::new(A, &[]) };
        let b = Fake { assets: vec![x(B, "two", 2, &[1]), x(B, "four", 4, &[3])], ..Fake::new(B, &[]) };
        let r = run_affected(&p, &[&a, &b], &changed(A, "one"), false, true).unwrap();
        assert!(!r.has_errors(), "{:?}", r.diagnostics);
        assert_eq!(a.affected(), Some(vec!["one".to_string(), "three".to_string()]));
        assert_eq!(b.affected(), Some(vec!["four".to_string(), "two".to_string()]));
        // From the middle of the chain: only what lies downstream.
        let (a, b) = (
            Fake { assets: a.assets.clone(), ..Fake::new(A, &[]) },
            Fake { assets: b.assets.clone(), ..Fake::new(B, &[]) },
        );
        run_affected(&p, &[&a, &b], &changed(A, "three"), false, true).unwrap();
        assert_eq!((a.affected(), b.affected()), (Some(vec!["three".to_string()]), Some(vec!["four".to_string()])));
    }

    /// Even a domain whose `plan_affected` asks for an orphan scan (the default full plan does) deletes nothing.
    #[test]
    fn run_affected_never_orphan_scans() {
        let (_d, p) = project();
        let a = Fake { assets: vec![x(A, "one", 1, &[])], ..Fake::new(A, &["a.txt"]) };
        run(&p, &[&a], &[], false, false).unwrap();
        let orphan = p.output_dir(A).join("old.txt");
        std::fs::write(&orphan, format!("{}\nold\n", header("//", None))).unwrap();
        let r = run_affected(&p, &[&a], &changed(A, "one"), false, false).unwrap();
        assert!(!r.has_errors(), "{:?}", r.diagnostics);
        assert!(a.affected().is_some());
        assert!(orphan.is_file(), "{:?}", r.changes);
        assert!(!r.changes.iter().any(|c| matches!(c, Change::Delete(_))), "{:?}", r.changes);
        // A full run would have deleted it.
        let r = run(&p, &[&a], &[], false, true).unwrap();
        assert_eq!(r.changes, [Change::Delete(orphan)]);
    }
}
