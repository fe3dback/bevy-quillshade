//! Editor asset discovery and loading (`shaders/**/*.{material,function}.ron`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use shader_graph_core::ident::check_ident;
use shader_graph_core::model::{Asset, AssetKind};
use shader_graph_core::{Code, Diagnostic, Registry};
use shader_stdlib::user::UserNode;

use crate::layout::{Project, USER_NODES_DIR};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AssetType {
    Function,
    Material,
}

impl AssetType {
    pub fn suffix(self) -> &'static str {
        match self {
            AssetType::Material => "material",
            AssetType::Function => "function",
        }
    }

    pub fn of(kind: &AssetKind) -> Self {
        if kind.is_function() { AssetType::Function } else { AssetType::Material }
    }
}

/// Asset identity: type + path without suffix (`env/rock_wet`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AssetId {
    pub ty: AssetType,
    pub path: String,
}

impl AssetId {
    pub fn new(ty: AssetType, path: impl Into<String>) -> Self {
        AssetId { ty, path: path.into() }
    }

    pub fn name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }

    /// `shaders/{path}.{suffix}.ron` relative to the project root.
    pub fn source_rel(&self) -> String {
        format!("shaders/{}.{}.ron", self.path, self.ty.suffix())
    }

    pub fn file(&self, project: &Project) -> PathBuf {
        project.asset_file(&self.path, self.ty.suffix())
    }
}

impl std::fmt::Display for AssetId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.source_rel())
    }
}

/// Parses `shaders/`-relative file path into an asset id.
pub fn id_from_rel(rel: &str) -> Option<AssetId> {
    let rel = rel.replace('\\', "/");
    if let Some(p) = rel.strip_suffix(".material.ron") {
        Some(AssetId::new(AssetType::Material, p))
    } else {
        rel.strip_suffix(".function.ron").map(|p| AssetId::new(AssetType::Function, p))
    }
}

/// Inverse of [`AssetId::source_rel`]: `shaders/env/rock_wet.material.ron` (relative to the project root).
pub fn id_from_source_rel(rel: &str) -> Option<AssetId> {
    id_from_rel(rel.replace('\\', "/").strip_prefix("shaders/")?)
}

/// Lists all editor assets under `shaders/` (hidden directories skipped), sorted.
pub fn scan(project: &Project) -> Vec<AssetId> {
    let root = project.shaders_dir();
    let mut out = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            if p.is_dir() {
                stack.push(p);
            } else if let Ok(rel) = p.strip_prefix(&root)
                && let Some(id) = id_from_rel(&rel.to_string_lossy())
            {
                out.push(id);
            }
        }
    }
    out.sort();
    out
}

/// Lists sub-directories of `shaders/` (relative, sorted), hidden ones skipped.
pub fn scan_dirs(project: &Project) -> Vec<String> {
    let root = project.shaders_dir();
    let mut out = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() && !e.file_name().to_string_lossy().starts_with('.') {
                if let Ok(rel) = p.strip_prefix(&root) {
                    out.push(rel.to_string_lossy().replace('\\', "/"));
                }
                stack.push(p);
            }
        }
    }
    out.sort();
    out
}

/// Loads an asset file; the file name wins over the `name` stored inside.
pub fn load(project: &Project, id: &AssetId) -> Result<Asset, String> {
    let text = std::fs::read_to_string(id.file(project)).map_err(|e| e.to_string())?;
    let mut asset = Asset::from_ron(&text).map_err(|e| e.to_string())?;
    asset.name = id.name().to_string();
    if AssetType::of(&asset.kind) != id.ty {
        return Err(format!("file suffix `.{}.ron` does not match asset kind", id.ty.suffix()));
    }
    Ok(asset)
}

/// Checks the asset path (spec §5): snake_case components, no reserved autogen directories.
pub fn validate_path(id: &AssetId) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let parts: Vec<&str> = id.path.split('/').collect();
    for (i, part) in parts.iter().enumerate() {
        let what = if i + 1 == parts.len() { "asset name" } else { "directory name" };
        if let Err(e) = check_ident(part) {
            out.push(Diagnostic::new(Code::E005, format!("{what}: {e}")).in_asset(id.source_rel()));
        }
    }
    if id.ty == AssetType::Material && parts.len() > 1 && matches!(parts[0], "stdlib" | "functions") {
        out.push(
            Diagnostic::new(
                Code::E005,
                format!("`shaders/{}/` is reserved for generated {}; move the material", parts[0], parts[0]),
            )
            .in_asset(id.source_rel()),
        );
    }
    out
}

/// Module-tree conflicts among materials: `env.material.ron` next to an `env/` directory.
pub fn module_conflicts(materials: &[&AssetId]) -> Vec<Diagnostic> {
    let mut dirs = std::collections::BTreeSet::new();
    for m in materials {
        let parts: Vec<&str> = m.path.split('/').collect();
        for i in 1..parts.len() {
            dirs.insert(parts[..i].join("/"));
        }
    }
    materials
        .iter()
        .filter(|m| dirs.contains(&m.path))
        .map(|m| {
            Diagnostic::new(
                Code::E005,
                format!("material `{}` clashes with directory `{}/` (Rust module tree)", m.path, m.path),
            )
            .in_asset(m.source_rel())
        })
        .collect()
}

/// Registers user nodes from `shaders/.nodes/*.ron`; returns load problems.
pub fn load_user_nodes(project: &Project, registry: &mut Registry) -> Vec<Diagnostic> {
    let dir = project.shaders_dir().join(USER_NODES_DIR);
    let mut diags = Vec::new();
    let Ok(rd) = std::fs::read_dir(&dir) else { return diags };
    let mut files: Vec<PathBuf> =
        rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "ron")).collect();
    files.sort();
    for f in files {
        let rel = project.rel(&f);
        match std::fs::read_to_string(&f).map_err(|e| e.to_string()).and_then(|t| UserNode::from_ron(&t)) {
            Ok(n) => n.register(registry),
            Err(e) => diags.push(Diagnostic::new(Code::E009, format!("user node: {e}")).in_asset(rel)),
        }
    }
    diags
}

/// Loaded project assets.
#[derive(Default, Clone)]
pub struct AssetSet {
    pub assets: BTreeMap<AssetId, Result<Asset, String>>,
}

impl AssetSet {
    pub fn load_all(project: &Project) -> AssetSet {
        let assets = scan(project).into_iter().map(|id| {
            let a = load(project, &id);
            (id, a)
        });
        AssetSet { assets: assets.collect() }
    }

    pub fn ids(&self) -> impl Iterator<Item = &AssetId> {
        self.assets.keys()
    }
}

/// Relative path helper for `notify` events.
pub fn rel_in_shaders(project: &Project, p: &Path) -> Option<String> {
    p.strip_prefix(project.shaders_dir()).ok().map(|r| r.to_string_lossy().replace('\\', "/"))
}
