//! Editor asset discovery and loading (`shaders/**/*.{material,function,curve,consts,texture,gradients}.ron`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use shader_graph_core::consts::GlobalConsts;
use shader_graph_core::curve::Curve;
use shader_graph_core::gradient::GradientCollection;
use shader_graph_core::ident::check_ident;
use shader_graph_core::model::{Asset, AssetKind};
use shader_graph_core::texture::TextureAsset;
use shader_graph_core::{Code, Diagnostic, Registry};
use shader_stdlib::user::UserNode;

use crate::layout::{Project, USER_NODES_DIR};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AssetType {
    /// Global constant table (decision 0017), not a node graph.
    Consts,
    /// Curve asset (decision 0015), not a node graph.
    Curve,
    Function,
    /// Gradient collection (gradient design §3): 128 gradient slots baked into one texture, not a node graph; no WESL
    /// module of its own.
    Gradients,
    Material,
    /// Texture asset (design §1.1, decision 0024): a game image + loader settings, not a node graph; no file of its
    /// own, only its entry in the baked manifest.
    Texture,
}

impl AssetType {
    pub fn suffix(self) -> &'static str {
        match self {
            AssetType::Material => "material",
            AssetType::Function => "function",
            AssetType::Curve => "curve",
            AssetType::Consts => "consts",
            AssetType::Texture => "texture",
            AssetType::Gradients => "gradients",
        }
    }

    /// Node graph assets (materials, functions), stored as [`Asset`].
    pub fn is_graph(self) -> bool {
        matches!(self, AssetType::Material | AssetType::Function)
    }

    /// Assets with a generated WESL module (constant tables have none: their values are inlined; Texture assets and
    /// gradient collections have none either).
    pub fn has_wesl(self) -> bool {
        !matches!(self, AssetType::Consts | AssetType::Texture | AssetType::Gradients)
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
    [
        AssetType::Material,
        AssetType::Function,
        AssetType::Curve,
        AssetType::Consts,
        AssetType::Texture,
        AssetType::Gradients,
    ]
    .into_iter()
    .find_map(|ty| rel.strip_suffix(&format!(".{}.ron", ty.suffix())).map(|p| AssetId::new(ty, p)))
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

/// Loads a graph asset file; the file name wins over the `name` stored inside.
pub fn load(project: &Project, id: &AssetId) -> Result<Asset, String> {
    if !id.ty.is_graph() {
        return Err(format!("{} is not a node graph", id.source_rel()));
    }
    let text = std::fs::read_to_string(id.file(project)).map_err(|e| e.to_string())?;
    let mut asset = Asset::from_ron(&text).map_err(|e| e.to_string())?;
    asset.name = id.name().to_string();
    if AssetType::of(&asset.kind) != id.ty {
        return Err(format!("file suffix `.{}.ron` does not match asset kind", id.ty.suffix()));
    }
    // Texture references are stdlib params (user nodes have none).
    static STDLIB: std::sync::OnceLock<Registry> = std::sync::OnceLock::new();
    shader_stdlib::nodes::texture::check_texture_refs(&asset, STDLIB.get_or_init(shader_stdlib::registry))?;
    Ok(asset)
}

/// Loads a curve file; the file name wins over the `name` stored inside.
pub fn load_curve(project: &Project, id: &AssetId) -> Result<Curve, String> {
    if id.ty != AssetType::Curve {
        return Err(format!("{} is not a curve", id.source_rel()));
    }
    let text = std::fs::read_to_string(id.file(project)).map_err(|e| e.to_string())?;
    let mut curve = Curve::from_ron(&text).map_err(|e| e.to_string())?;
    curve.name = id.name().to_string();
    Ok(curve)
}

/// Loads a global constant table; the file name wins over the `name` stored inside.
pub fn load_consts(project: &Project, id: &AssetId) -> Result<GlobalConsts, String> {
    if id.ty != AssetType::Consts {
        return Err(format!("{} is not a constant table", id.source_rel()));
    }
    let text = std::fs::read_to_string(id.file(project)).map_err(|e| e.to_string())?;
    let mut table = GlobalConsts::from_ron(&text).map_err(|e| e.to_string())?;
    table.name = id.name().to_string();
    Ok(table)
}

/// Loads a Texture asset file (its name is the file name; the image path is checked by validation, E012).
pub fn load_texture(project: &Project, id: &AssetId) -> Result<TextureAsset, String> {
    if id.ty != AssetType::Texture {
        return Err(format!("{} is not a texture", id.source_rel()));
    }
    let text = std::fs::read_to_string(id.file(project)).map_err(|e| e.to_string())?;
    TextureAsset::from_ron(&text)
}

/// Loads a gradient collection file (its name is the file name). Structural problems (slot count, ids, stop count)
/// are load errors.
pub fn load_gradients(project: &Project, id: &AssetId) -> Result<GradientCollection, String> {
    if id.ty != AssetType::Gradients {
        return Err(format!("{} is not a gradient collection", id.source_rel()));
    }
    let text = std::fs::read_to_string(id.file(project)).map_err(|e| e.to_string())?;
    GradientCollection::from_ron(&text).map_err(|e| e.to_string())
}

/// Checks the asset path (spec §5): snake_case components. Generated modules are prefixed (`dir_`, `mat_`;
/// decision 0020), so keywords, a material next to a directory of the same name and top-level `stdlib/`,
/// `functions/`, `curves/` directories are fine.
pub fn validate_path(id: &AssetId) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let parts: Vec<&str> = id.path.split('/').collect();
    for (i, part) in parts.iter().enumerate() {
        let what = if i + 1 == parts.len() { "asset name" } else { "directory name" };
        if let Err(e) = check_ident(part) {
            out.push(Diagnostic::new(Code::E005, format!("{what}: {e}")).in_asset(id.source_rel()));
        }
    }
    out
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
    /// Node graphs (materials, functions).
    pub assets: BTreeMap<AssetId, Result<Asset, String>>,
    pub curves: BTreeMap<AssetId, Result<Curve, String>>,
    /// Global constant tables.
    pub consts: BTreeMap<AssetId, Result<GlobalConsts, String>>,
    /// Texture assets (design §1.1).
    pub textures: BTreeMap<AssetId, Result<TextureAsset, String>>,
    /// Gradient collections (gradient design §3).
    pub gradients: BTreeMap<AssetId, Result<GradientCollection, String>>,
}

impl AssetSet {
    pub fn load_all(project: &Project) -> AssetSet {
        let mut set = AssetSet::default();
        for id in scan(project) {
            match id.ty {
                AssetType::Material | AssetType::Function => {
                    let a = load(project, &id);
                    set.assets.insert(id, a);
                }
                AssetType::Curve => {
                    let c = load_curve(project, &id);
                    set.curves.insert(id, c);
                }
                AssetType::Consts => {
                    let c = load_consts(project, &id);
                    set.consts.insert(id, c);
                }
                AssetType::Texture => {
                    let t = load_texture(project, &id);
                    set.textures.insert(id, t);
                }
                AssetType::Gradients => {
                    let g = load_gradients(project, &id);
                    set.gradients.insert(id, g);
                }
            }
        }
        set
    }

    /// Every asset: graphs, curves, constant tables, textures, gradient collections (sorted within each kind).
    pub fn ids(&self) -> impl Iterator<Item = &AssetId> {
        self.assets
            .keys()
            .chain(self.curves.keys())
            .chain(self.consts.keys())
            .chain(self.textures.keys())
            .chain(self.gradients.keys())
    }

    pub fn contains(&self, id: &AssetId) -> bool {
        self.assets.contains_key(id)
            || self.curves.contains_key(id)
            || self.consts.contains_key(id)
            || self.textures.contains_key(id)
            || self.gradients.contains_key(id)
    }

    pub fn len(&self) -> usize {
        self.assets.len() + self.curves.len() + self.consts.len() + self.textures.len() + self.gradients.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Relative path helper for `notify` events.
pub fn rel_in_shaders(project: &Project, p: &Path) -> Option<String> {
    p.strip_prefix(project.shaders_dir()).ok().map(|r| r.to_string_lossy().replace('\\', "/"))
}
