//! Editor asset model (spec §6): the RON source of truth, independent from the canvas.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::value::Value;

/// Current asset file format version.
pub const FORMAT_VERSION: u32 = 1;

pub type NodeId = String;

/// Generates a new stable node id (lowercase ULID, sortable by creation time).
pub fn new_node_id() -> NodeId {
    ulid::Ulid::new().to_string().to_lowercase()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Asset {
    pub format_version: u32,
    pub kind: AssetKind,
    pub name: String,
    #[serde(default)]
    pub nodes: BTreeMap<NodeId, Node>,
    #[serde(default)]
    pub edges: Vec<Edge>,
    #[serde(default)]
    pub comments: Vec<Frame>,
    #[serde(default)]
    pub meta: Meta,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum AssetKind {
    Material(MaterialKind),
    Function,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum MaterialKind {
    Pbr3d(PbrSettings),
    Unlit3d(UnlitSettings),
    Sprite2d(SpriteSettings),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PbrSettings {
    pub blend: BlendMode,
    pub two_sided: bool,
    pub shading: Shading,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnlitSettings {
    pub blend: BlendMode,
    pub two_sided: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpriteSettings {
    pub blend: BlendMode,
}

impl Default for PbrSettings {
    fn default() -> Self {
        PbrSettings { blend: BlendMode::Opaque, two_sided: false, shading: Shading::Lit }
    }
}

impl Default for UnlitSettings {
    fn default() -> Self {
        UnlitSettings { blend: BlendMode::Opaque, two_sided: false }
    }
}

impl Default for SpriteSettings {
    fn default() -> Self {
        SpriteSettings { blend: BlendMode::Blend }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BlendMode {
    Opaque,
    Mask,
    Blend,
    Add,
    Multiply,
}

impl BlendMode {
    pub const ALL: [BlendMode; 5] =
        [BlendMode::Opaque, BlendMode::Mask, BlendMode::Blend, BlendMode::Add, BlendMode::Multiply];
    /// Blend modes supported by 2D materials (`AlphaMode2d`).
    pub const ALL_2D: [BlendMode; 3] = [BlendMode::Opaque, BlendMode::Mask, BlendMode::Blend];

    pub fn label(self) -> &'static str {
        match self {
            BlendMode::Opaque => "Opaque",
            BlendMode::Mask => "Mask",
            BlendMode::Blend => "Blend",
            BlendMode::Add => "Add",
            BlendMode::Multiply => "Multiply",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Shading {
    Lit,
    Unlit,
}

impl MaterialKind {
    pub fn blend(&self) -> BlendMode {
        match self {
            MaterialKind::Pbr3d(s) => s.blend,
            MaterialKind::Unlit3d(s) => s.blend,
            MaterialKind::Sprite2d(s) => s.blend,
        }
    }

    pub fn set_blend(&mut self, blend: BlendMode) {
        match self {
            MaterialKind::Pbr3d(s) => s.blend = blend,
            MaterialKind::Unlit3d(s) => s.blend = blend,
            MaterialKind::Sprite2d(s) => s.blend = blend,
        }
    }

    /// Node kind of the output node required by this material kind.
    pub fn output_node_kind(&self) -> &'static str {
        match self {
            MaterialKind::Pbr3d(_) => "output::Pbr3d",
            MaterialKind::Unlit3d(_) => "output::Unlit3d",
            MaterialKind::Sprite2d(_) => "output::Sprite2d",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            MaterialKind::Pbr3d(_) => "Pbr3d",
            MaterialKind::Unlit3d(_) => "Unlit3d",
            MaterialKind::Sprite2d(_) => "Sprite2d",
        }
    }

    pub fn is_2d(&self) -> bool {
        matches!(self, MaterialKind::Sprite2d(_))
    }
}

impl AssetKind {
    pub fn is_function(&self) -> bool {
        matches!(self, AssetKind::Function)
    }

    pub fn material(&self) -> Option<&MaterialKind> {
        match self {
            AssetKind::Material(m) => Some(m),
            AssetKind::Function => None,
        }
    }

    /// File suffix (`material` / `function`).
    pub fn suffix(&self) -> &'static str {
        match self {
            AssetKind::Material(_) => "material",
            AssetKind::Function => "function",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub kind: String,
    pub pos: (f32, f32),
    #[serde(default)]
    pub params: BTreeMap<String, Value>,
    /// Overrides of default values of unconnected inputs, edited inline in the node.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub inputs: BTreeMap<String, Value>,
}

impl Node {
    pub fn new(kind: impl Into<String>, pos: (f32, f32)) -> Self {
        Node { kind: kind.into(), pos, params: BTreeMap::new(), inputs: BTreeMap::new() }
    }

    pub fn with_param(mut self, id: &str, v: Value) -> Self {
        self.params.insert(id.to_string(), v);
        self
    }

    pub fn with_input(mut self, id: &str, v: Value) -> Self {
        self.inputs.insert(id.to_string(), v);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Edge {
    pub from: (NodeId, String),
    pub to: (NodeId, String),
}

impl Edge {
    pub fn new(from: (&str, &str), to: (&str, &str)) -> Self {
        Edge { from: (from.0.to_string(), from.1.to_string()), to: (to.0.to_string(), to.1.to_string()) }
    }
}

/// Comment frame (UE "Comment" box).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Frame {
    pub id: String,
    pub title: String,
    /// x, y, width, height in graph space.
    pub rect: (f32, f32, f32, f32),
    #[serde(default)]
    pub nodes: Vec<NodeId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<[f32; 4]>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Meta {
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("parse error: {0}")]
    Parse(String),
    #[error("format_version {found} is newer than supported {supported}; update the editor")]
    TooNew { found: u32, supported: u32 },
    #[error("unsupported format_version {0}")]
    Unsupported(u32),
}

#[derive(Deserialize)]
#[serde(rename = "Asset")]
struct VersionProbe {
    format_version: u32,
}

impl Asset {
    pub fn new(name: impl Into<String>, kind: AssetKind) -> Self {
        Asset {
            format_version: FORMAT_VERSION,
            kind,
            name: name.into(),
            nodes: BTreeMap::new(),
            edges: Vec::new(),
            comments: Vec::new(),
            meta: Meta::default(),
        }
    }

    /// Parses an asset file, migrating older format versions.
    pub fn from_ron(text: &str) -> Result<Asset, LoadError> {
        let probe: VersionProbe =
            ron::Options::default().from_str(text).map_err(|e| LoadError::Parse(e.to_string()))?;
        match probe.format_version {
            v if v > FORMAT_VERSION => Err(LoadError::TooNew { found: v, supported: FORMAT_VERSION }),
            FORMAT_VERSION => {
                let mut asset: Asset =
                    ron::Options::default().from_str(text).map_err(|e| LoadError::Parse(e.to_string()))?;
                asset.normalize();
                Ok(asset)
            }
            // Migrations from older versions go here: parse the legacy struct, convert, recurse.
            v => Err(LoadError::Unsupported(v)),
        }
    }

    /// Serializes with sorted keys and fixed field order for readable VCS diffs.
    pub fn to_ron(&self) -> String {
        let mut asset = self.clone();
        asset.normalize();
        let cfg =
            ron::ser::PrettyConfig::new().struct_names(true).indentor("  ".to_string()).new_line("\n".to_string());
        let mut s = ron::ser::to_string_pretty(&asset, cfg).expect("asset serialization");
        s.push('\n');
        s
    }

    /// Sorts collections into canonical order and drops dangling references.
    pub fn normalize(&mut self) {
        self.edges.retain(|e| self.nodes.contains_key(&e.from.0) && self.nodes.contains_key(&e.to.0));
        self.edges.sort_by(|a, b| (&a.to, &a.from).cmp(&(&b.to, &b.from)));
        self.edges.dedup();
        self.comments.sort_by(|a, b| a.id.cmp(&b.id));
        for c in &mut self.comments {
            c.nodes.retain(|n| self.nodes.contains_key(n));
            c.nodes.sort();
            c.nodes.dedup();
        }
    }

    pub fn add_node(&mut self, node: Node) -> NodeId {
        let id = new_node_id();
        self.nodes.insert(id.clone(), node);
        id
    }

    pub fn remove_node(&mut self, id: &str) -> Option<Node> {
        let n = self.nodes.remove(id)?;
        self.edges.retain(|e| e.from.0 != id && e.to.0 != id);
        for c in &mut self.comments {
            c.nodes.retain(|x| x != id);
        }
        Some(n)
    }

    /// Connects `from` → `to`, replacing any existing edge into `to` (one edge per input).
    pub fn connect(&mut self, from: (&str, &str), to: (&str, &str)) {
        self.edges.retain(|e| !(e.to.0 == to.0 && e.to.1 == to.1));
        self.edges.push(Edge::new(from, to));
    }

    pub fn disconnect_input(&mut self, to: (&str, &str)) {
        self.edges.retain(|e| !(e.to.0 == to.0 && e.to.1 == to.1));
    }

    pub fn edge_into(&self, node: &str, pin: &str) -> Option<&Edge> {
        self.edges.iter().find(|e| e.to.0 == node && e.to.1 == pin)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_ron() {
        let mut a = Asset::new("rock", AssetKind::Material(MaterialKind::Pbr3d(PbrSettings::default())));
        let n1 = a.add_node(Node::new("stdlib::Constant", (1.0, 2.0)).with_param("value", Value::Float(0.5)));
        let n2 = a.add_node(Node::new("output::Pbr3d", (100.0, 0.0)));
        a.connect((&n1, "out"), (&n2, "roughness"));
        let text = a.to_ron();
        assert!(text.starts_with("Asset("));
        let b = Asset::from_ron(&text).unwrap();
        assert_eq!(a, b);
        assert_eq!(text, b.to_ron());
    }

    #[test]
    fn too_new_version_rejected() {
        let text = "Asset(format_version: 99, kind: Function, name: \"x\")";
        assert!(matches!(Asset::from_ron(text), Err(LoadError::TooNew { .. })));
    }
}
