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
    /// Exclusive node preview of a material (spec §5, decision 0022); ignored in functions.
    #[serde(default, skip_serializing_if = "Preview::is_default")]
    pub preview: Preview,
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
    /// Pin layout of the output node (decision 0023); omitted from the file when `Default`.
    #[serde(default, skip_serializing_if = "PbrOutputMode::is_default")]
    pub output: PbrOutputMode,
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
        PbrSettings {
            blend: BlendMode::Opaque,
            two_sided: false,
            shading: Shading::Lit,
            output: PbrOutputMode::Default,
        }
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

/// Inputs of the Pbr3d output node: one pin per attribute, or one `MaterialAttributes (PBR)` pin + WPO.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum PbrOutputMode {
    #[default]
    Default,
    Wrapped,
}

impl PbrOutputMode {
    pub const ALL: [PbrOutputMode; 2] = [PbrOutputMode::Default, PbrOutputMode::Wrapped];

    pub fn label(self) -> &'static str {
        match self {
            PbrOutputMode::Default => "Default",
            PbrOutputMode::Wrapped => "Wrapped",
        }
    }

    pub fn is_default(&self) -> bool {
        *self == PbrOutputMode::Default
    }
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
    /// Editor-only title of a constant node; codegen never reads it (decision 0021).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    /// Editor-only note shown above the node (any node, at most [`Node::COMMENT_MAX`] characters); codegen never
    /// reads it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub comment: String,
}

impl Node {
    /// Longest [`Node::comment`], in characters.
    pub const COMMENT_MAX: usize = 512;

    pub fn new(kind: impl Into<String>, pos: (f32, f32)) -> Self {
        Node {
            kind: kind.into(),
            pos,
            params: BTreeMap::new(),
            inputs: BTreeMap::new(),
            label: String::new(),
            comment: String::new(),
        }
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
    /// Hue in degrees `0..360` picked in the editor (saturation / lightness are the editor's); wins over `color`.
    /// Unset: the frame keeps `color` or the default look, and the field is not written (design §4.6).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hue: Option<f32>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Meta {
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// How a previewed node's output is rendered (spec §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PreviewMode {
    /// Albedo only, no lighting.
    #[default]
    Unlit,
    /// Albedo only, through the material's normal lit template.
    Lit,
    /// Every slot of the node's `ShaderOutputDto`.
    Full,
}

impl PreviewMode {
    pub const ALL: [PreviewMode; 3] = [PreviewMode::Unlit, PreviewMode::Lit, PreviewMode::Full];

    pub fn label(self) -> &'static str {
        match self {
            PreviewMode::Unlit => "unlit",
            PreviewMode::Lit => "lit",
            PreviewMode::Full => "full",
        }
    }
}

/// Previewed node and preview mode, stored in the material file (decision 0022).
/// The mode is kept while no node is previewed.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Preview {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<NodeId>,
    #[serde(default)]
    pub mode: PreviewMode,
}

impl Preview {
    pub fn is_default(&self) -> bool {
        *self == Preview::default()
    }
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
            preview: Preview::default(),
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

    /// Sorts collections into canonical order, drops dangling references and the `order` property material
    /// parameter nodes no longer have (decision 0027).
    pub fn normalize(&mut self) {
        for n in self.nodes.values_mut().filter(|n| n.kind.starts_with("param::")) {
            n.params.remove("order");
        }
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

    /// Clears `preview.node` when that node no longer exists (the mode stays). Called by editor edits, not by
    /// [`Asset::normalize`], so a dangling id read from a file reaches `build_ir` and is reported (W107).
    pub fn clear_dangling_preview(&mut self) {
        if self.preview.node.as_ref().is_some_and(|n| !self.nodes.contains_key(n)) {
            self.preview.node = None;
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
    fn parameter_order_property_is_dropped() {
        let mut a = Asset::new("rock", AssetKind::Material(MaterialKind::Pbr3d(PbrSettings::default())));
        let p = a.add_node(Node::new("param::Scalar", (0.0, 0.0)).with_param("order", Value::Int(3)));
        let f = a.add_node(Node::new("output::FunctionInput", (0.0, 0.0)).with_param("order", Value::Int(3)));
        let b = Asset::from_ron(&a.to_ron()).unwrap();
        assert!(!b.nodes[&p].params.contains_key("order"), "decision 0027");
        assert_eq!(b.nodes[&f].params.get("order"), Some(&Value::Int(3)), "function pins keep their order");
    }

    #[test]
    fn node_label_roundtrip() {
        let mut a = Asset::new("rock", AssetKind::Material(MaterialKind::Pbr3d(PbrSettings::default())));
        let mut n = Node::new("stdlib::Constant", (0.0, 0.0));
        n.label = "rim width".into();
        a.add_node(n);
        let text = a.to_ron();
        assert!(text.contains("label: \"rim width\""), "{text}");
        assert_eq!(Asset::from_ron(&text).unwrap(), a);
        let mut b = a.clone();
        b.nodes.values_mut().for_each(|n| n.label.clear());
        assert!(!b.to_ron().contains("label:"));
    }

    #[test]
    fn node_comment_roundtrip() {
        let mut a = Asset::new("rock", AssetKind::Material(MaterialKind::Pbr3d(PbrSettings::default())));
        let mut n = Node::new("stdlib::Add", (0.0, 0.0));
        n.comment = "wet\nlayer".into();
        a.add_node(n);
        let text = a.to_ron();
        assert!(text.contains("comment: \"wet\\nlayer\""), "{text}");
        assert_eq!(Asset::from_ron(&text).unwrap(), a);
        let mut b = a.clone();
        b.nodes.values_mut().for_each(|n| n.comment.clear());
        assert!(!b.to_ron().contains("comment:"), "an empty comment is not written");
    }

    #[test]
    fn frame_without_hue_roundtrips_byte_identical() {
        let mut a = Asset::new("rock", AssetKind::Material(MaterialKind::Pbr3d(PbrSettings::default())));
        let n = a.add_node(Node::new("stdlib::Constant", (0.0, 0.0)));
        a.comments.push(Frame {
            id: "c1".into(),
            title: "Rim".into(),
            rect: (-30.0, -50.0, 400.0, 300.0),
            nodes: vec![n],
            color: None,
            hue: None,
        });
        // A file written before frames had a hue (design §4.6) loads and saves unchanged.
        let text = a.to_ron();
        assert!(!text.contains("hue"), "{text}");
        let b = Asset::from_ron(&text).unwrap();
        assert_eq!(b.comments[0].hue, None);
        assert_eq!(b.to_ron(), text);

        a.comments[0].hue = Some(200.0);
        let text = a.to_ron();
        assert!(text.contains("hue: Some(200.0)"), "{text}");
        assert_eq!(Asset::from_ron(&text).unwrap(), a);
    }

    #[test]
    fn preview_roundtrip_and_default_not_serialized() {
        let mut a = Asset::new("rock", AssetKind::Material(MaterialKind::Pbr3d(PbrSettings::default())));
        let n = a.add_node(Node::new("stdlib::Constant", (0.0, 0.0)));
        assert!(!a.to_ron().contains("preview"));
        a.preview = Preview { node: Some(n.clone()), mode: PreviewMode::Full };
        let text = a.to_ron();
        assert!(text.contains("preview: Preview("), "{text}");
        assert!(text.contains("mode: Full"), "{text}");
        assert_eq!(Asset::from_ron(&text).unwrap(), a);
        a.preview = Preview { node: None, mode: PreviewMode::Lit };
        assert_eq!(Asset::from_ron(&a.to_ron()).unwrap().preview, a.preview);
    }

    #[test]
    fn preview_clear_dangling_keeps_mode() {
        let mut a = Asset::new("rock", AssetKind::Material(MaterialKind::Pbr3d(PbrSettings::default())));
        let n = a.add_node(Node::new("stdlib::Constant", (0.0, 0.0)));
        a.preview = Preview { node: Some(n.clone()), mode: PreviewMode::Lit };
        a.clear_dangling_preview();
        assert_eq!(a.preview.node.as_deref(), Some(n.as_str()));
        a.remove_node(&n);
        a.clear_dangling_preview();
        assert_eq!(a.preview, Preview { node: None, mode: PreviewMode::Lit });
    }

    #[test]
    fn preview_dangling_id_survives_loading() {
        let mut a = Asset::new("rock", AssetKind::Material(MaterialKind::Pbr3d(PbrSettings::default())));
        a.preview = Preview { node: Some("zzz".into()), mode: PreviewMode::Full };
        let b = Asset::from_ron(&a.to_ron()).unwrap();
        assert_eq!(b.preview, a.preview);
    }

    #[test]
    fn pbr_default_output_mode_not_serialized() {
        let mut a = Asset::new("rock", AssetKind::Material(MaterialKind::Pbr3d(PbrSettings::default())));
        assert!(!a.to_ron().contains("output"), "{}", a.to_ron());
        a.kind = AssetKind::Material(MaterialKind::Pbr3d(PbrSettings {
            output: PbrOutputMode::Wrapped,
            ..Default::default()
        }));
        let text = a.to_ron();
        assert!(text.contains("output: Wrapped"), "{text}");
        assert_eq!(Asset::from_ron(&text).unwrap(), a);
        let legacy = "Asset(format_version: 1, kind: Material(Pbr3d(PbrSettings(blend: Opaque, two_sided: false, \
                      shading: Lit))), name: \"x\")";
        let b = Asset::from_ron(legacy).unwrap();
        assert!(matches!(b.kind, AssetKind::Material(MaterialKind::Pbr3d(s)) if s.output == PbrOutputMode::Default));
    }

    #[test]
    fn too_new_version_rejected() {
        let text = "Asset(format_version: 99, kind: Function, name: \"x\")";
        assert!(matches!(Asset::from_ron(text), Err(LoadError::TooNew { .. })));
    }
}
