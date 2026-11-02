//! Data-driven user nodes from `quillshade/material/.nodes/*.ron` (no editor rebuild needed).
//!
//! ```ron
//! UserNode(
//!     format_version: 2,
//!     ident: "soft_clamp",
//!     name: "SoftClamp",
//!     category: "User",
//!     keywords: ["clamp"],
//!     inputs: [UserPin(name: "x", ty: Float3), UserPin(name: "k", ty: Float, default: Some(Float(0.5)))],
//!     output: Float3,
//!     template: "return x / (vec3<f32>(1.0) + abs(x) * k);",
//! )
//! ```
//! The template is the body of a generated `fn` whose arguments are the inputs; an expression
//! without `return` is wrapped automatically.
//!
//! User nodes are editor-owned assets (ADR 0047 §16), format [`USER_NODE_FORMAT_VERSION`]: the code identifier in
//! `ident`, the asset UUID in `id`. A hand-written node like the example above has no `id` yet: it loads and
//! registers, but graphs can reference it only once it has one (AE004, quick fix "Assign UUID"). Graphs reference a
//! node as `user::<uuid>`, which the resolve seam maps to the registered kind `user::<ident>`. Format 1 (no
//! `format_version` key, the code identifier in `id`) is upgraded by the Migration Tool, never by the loader.

use project_core::{AssetUuid, check_version};
use serde::{Deserialize, Serialize};
use shader_graph_core::registry::{NodeDef, NodeInfo, PinDef, Registry, SigCtx, Signature};
use shader_graph_core::{Code, EmitCtx, PinType, Value, ident::check_code_ident};

/// Current user node format: an asset `id` and the code identifier in `ident` (ADR 0047 §16, written by the v3
/// migration and [`UserNode::to_ron`]). Format 1 had no `format_version` key and the code identifier in `id`.
pub const USER_NODE_FORMAT_VERSION: u32 = 2;

/// Kind prefix of user nodes: `user::<ident>` in the registry, `user::<uuid>` in graphs.
pub const USER_PREFIX: &str = "user::";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserPin {
    pub name: String,
    pub ty: PinType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
}

/// A user node definition (one `.nodes/*.ron` file).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct UserNode {
    /// [`USER_NODE_FORMAT_VERSION`].
    pub format_version: u32,
    /// The asset UUID; nil (omitted) for a hand-written node without one (AE004).
    #[serde(skip_serializing_if = "AssetUuid::is_nil")]
    pub id: AssetUuid,
    /// The code identifier: registry kind `user::<ident>`, WESL function `sg_user_<ident>`; unique among user nodes.
    pub ident: String,
    pub name: String,
    pub category: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub keywords: Vec<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    pub inputs: Vec<UserPin>,
    pub output: PinType,
    pub template: String,
}

/// The file as written; the version is checked before the fields are used.
#[derive(Deserialize)]
#[serde(rename = "UserNode")]
struct RawUserNode {
    /// 0 when the key is missing (format 1; versions start at 1).
    #[serde(default)]
    format_version: u32,
    #[serde(default)]
    id: String,
    #[serde(default)]
    ident: String,
    name: String,
    #[serde(default = "default_category")]
    category: String,
    #[serde(default)]
    keywords: Vec<String>,
    #[serde(default)]
    description: String,
    inputs: Vec<UserPin>,
    output: PinType,
    template: String,
}

fn default_category() -> String {
    "User".into()
}

struct UserDef {
    info: NodeInfo,
    node: UserNode,
}

impl UserNode {
    /// Parses and validates a `.nodes/*.ron` file of the current format (see the module docs). A file without
    /// `format_version` was written for an older editor (format 1): it is refused, like any other version.
    pub fn from_ron(text: &str) -> Result<UserNode, String> {
        let raw: RawUserNode = ron::from_str(text).map_err(|e| e.to_string())?;
        let format_version = raw.format_version;
        if format_version == 0 {
            return Err(format!(
                "written for an older editor (no `format_version`); run `shader-editor migrate` or add \
                 `format_version: {USER_NODE_FORMAT_VERSION}`"
            ));
        }
        check_version(format_version, USER_NODE_FORMAT_VERSION).map_err(|e| e.to_string())?;
        let id = match raw.id.as_str() {
            "" => AssetUuid::NIL,
            s => AssetUuid::parse(s).ok_or_else(|| format!("user node id: `{s}` is not a UUID"))?,
        };
        let n = UserNode {
            format_version,
            id,
            ident: raw.ident,
            name: raw.name,
            category: raw.category,
            keywords: raw.keywords,
            description: raw.description,
            inputs: raw.inputs,
            output: raw.output,
            template: raw.template,
        };
        check_code_ident(&n.ident).map_err(|e| format!("user node ident: {e}"))?;
        for p in &n.inputs {
            check_code_ident(&p.name).map_err(|e| format!("user node input: {e}"))?;
            if p.ty == PinType::Any {
                return Err("user node inputs must have concrete types".into());
            }
            if matches!(p.ty, PinType::MaterialAttributesPbr | PinType::CompileBool) {
                return Err(format!("user node input `{}`: type {} is not supported", p.name, p.ty.label()));
            }
        }
        if matches!(n.output, PinType::MaterialAttributesPbr | PinType::CompileBool) {
            return Err(format!("user node output: type {} is not supported", n.output.label()));
        }
        if matches!(n.output, PinType::Any | PinType::Texture2D) {
            return Err("user node output must be a value type".into());
        }
        Ok(n)
    }

    /// The canonical file text (ADR 0047 §16): struct-named, 2-space indent; empty keywords and description and
    /// absent pin defaults are left out. Hand-written comments do not survive (D27).
    pub fn to_ron(&self) -> String {
        let cfg =
            ron::ser::PrettyConfig::new().struct_names(true).indentor("  ".to_string()).new_line("\n".to_string());
        let mut s = ron::ser::to_string_pretty(self, cfg).expect("user node serialization");
        s.push('\n');
        s
    }

    /// Mints the node's UUID when it has none (ADR 0047 §6); returns it.
    pub fn ensure_id(&mut self) -> AssetUuid {
        self.id.ensure()
    }

    /// The registry kind: `user::<ident>`.
    pub fn kind(&self) -> String {
        format!("{USER_PREFIX}{}", self.ident)
    }

    /// Registers the node under [`UserNode::kind`] and, when it has an id, records that kind for its UUID
    /// ([`Registry::asset_kind`]), so graphs storing `user::<uuid>` resolve to it; a node without an id (AE004) has
    /// no UUID to be referenced by until it gets one (ADR 0047 §16).
    pub fn register(self, r: &mut Registry) {
        if !self.id.is_nil() {
            r.register_asset_kind(self.id, &self.kind());
        }
        let mut info = NodeInfo::new(&self.kind(), &self.name, &self.category).describe(&self.description);
        info.keywords = self.keywords.clone();
        r.register(UserDef { info, node: self });
    }
}

impl NodeDef for UserDef {
    fn info(&self) -> &NodeInfo {
        &self.info
    }

    fn signature(&self, _ctx: &SigCtx) -> Signature {
        Signature {
            inputs: self
                .node
                .inputs
                .iter()
                .map(|p| {
                    let pin = PinDef::new(&p.name, &p.name, p.ty);
                    match &p.default {
                        Some(v) => pin.default(v.coerce(p.ty)),
                        None => pin,
                    }
                })
                .collect(),
            outputs: vec![PinDef::new("out", "", self.node.output)],
        }
    }

    fn emit(&self, c: &mut EmitCtx) {
        let name = format!("sg_user_{}", self.node.ident);
        let mut args = Vec::new();
        let mut call = Vec::new();
        for p in &self.node.inputs {
            if p.ty == PinType::Texture2D {
                args.push(format!("{0}: texture_2d<f32>, {0}_sampler: sampler", p.name));
                match c.input_texture(&p.name) {
                    Some((t, s)) => call.push(format!("{t}, {s}")),
                    None => {
                        c.diag_pin(Code::E004, &p.name, "texture input is not connected");
                        return;
                    }
                }
            } else {
                args.push(format!("{}: {}", p.name, p.ty.wesl()));
                call.push(c.input(&p.name));
            }
        }
        let t = self.node.template.trim();
        let body = if has_return_keyword(t) { t.to_string() } else { format!("return {};", t.trim_end_matches(';')) };
        let body: String = body.lines().map(|l| format!("    {l}\n")).collect();
        c.add_fn(&name, format!("fn {name}({}) -> {} {{\n{body}}}\n", args.join(", "), self.node.output.wesl()));
        c.define("out", format!("{name}({})", call.join(", ")));
    }
}

/// Whether `code` contains `return` as a standalone WGSL keyword token (outside `//` line comments, `/* ... */`
/// block comments, and string literals).
pub(crate) fn has_return_keyword(code: &str) -> bool {
    let bytes = code.as_bytes();
    let len = bytes.len();
    let is_ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let mut i = 0;
    while i < len {
        if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            i += 2;
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            i += 2;
            let mut depth = 1usize;
            while i < len && depth > 0 {
                if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'*' {
                    depth += 1;
                    i += 2;
                } else if i + 1 < len && bytes[i] == b'*' && bytes[i + 1] == b'/' {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            continue;
        }
        if bytes[i] == b'"' {
            i += 1;
            while i < len && bytes[i] != b'"' {
                if bytes[i] == b'\\' && i + 1 < len {
                    i += 2;
                } else {
                    i += 1;
                }
            }
            if i < len {
                i += 1;
            }
            continue;
        }
        if is_ident(bytes[i]) {
            let start = i;
            while i < len && is_ident(bytes[i]) {
                i += 1;
            }
            if &bytes[start..i] == b"return" {
                return true;
            }
            continue;
        }
        i += 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Custom-node and user-node input names are embedded verbatim in user WGSL, so unlike graph names they keep
    /// the keyword check (decision 0020).
    #[test]
    fn code_embedded_names_reject_keywords() {
        let e = crate::nodes::utility::parse_custom_inputs("loop: Float").unwrap_err();
        assert!(e.contains("reserved"), "{e}");
        assert!(crate::nodes::utility::parse_custom_inputs("strength: Float").is_ok());
        let node = |input: &str| {
            format!(
                "UserNode(format_version: 2, ident: \"soft\", name: \"Soft\", inputs: [UserPin(name: \"{input}\", ty: Float)], output: Float, \
                 template: \"x\")"
            )
        };
        let e = UserNode::from_ron(&node("fn")).unwrap_err();
        assert!(e.contains("user node input") && e.contains("reserved"), "{e}");
        assert!(UserNode::from_ron(&node("x")).is_ok());
    }

    /// Struct pins are not available to user code (decision 0023).
    #[test]
    fn attributes_type_rejected() {
        let input = "UserNode(format_version: 2, ident: \"soft\", name: \"Soft\", inputs: [UserPin(name: \"x\", ty: MaterialAttributesPbr)], \
                     output: Float, template: \"x\")";
        let e = UserNode::from_ron(input).unwrap_err();
        assert!(e.contains("MaterialAttributes (PBR)"), "{e}");
        let output = "UserNode(format_version: 2, ident: \"soft\", name: \"Soft\", inputs: [], output: MaterialAttributesPbr, template: \"x\")";
        let e = UserNode::from_ron(output).unwrap_err();
        assert!(e.contains("MaterialAttributes (PBR)"), "{e}");
        let e = crate::nodes::utility::parse_custom_inputs("a: MaterialAttributes (PBR)").unwrap_err();
        assert!(e.starts_with("unknown type") && e.ends_with("Int, Texture2D)"), "{e}");
        let cb_in = "UserNode(format_version: 2, ident: \"soft\", name: \"Soft\", inputs: [UserPin(name: \"x\", ty: CompileBool)], \
                     output: Float, template: \"1.0\")";
        assert!(UserNode::from_ron(cb_in).unwrap_err().contains("CompileBool"));
        let cb_out = "UserNode(format_version: 2, ident: \"soft\", name: \"Soft\", inputs: [], output: CompileBool, template: \"true\")";
        assert!(UserNode::from_ron(cb_out).unwrap_err().contains("CompileBool"));
        assert!(crate::nodes::utility::parse_custom_inputs("a: CompileBool").is_err());
        assert!(crate::nodes::utility::parse_custom_inputs("a: RuntimeBool, b: Bool").is_ok());
    }

    /// The module docs' example (hand-written: no `id` yet).
    const SOFT_CLAMP: &str = r#"UserNode(
    format_version: 2,
    ident: "soft_clamp",
    name: "SoftClamp",
    category: "User",
    keywords: ["clamp"],
    inputs: [UserPin(name: "x", ty: Float3), UserPin(name: "k", ty: Float, default: Some(Float(0.5)))],
    output: Float3,
    template: "return x / (vec3<f32>(1.0) + abs(x) * k);",
)"#;

    /// ADR 0047 §16: the current format reads with or without an id (the index reports AE004 without one); format 1
    /// (no `format_version`, the code identifier in `id`) and other versions are refused.
    #[test]
    fn current_format_parses_older_is_refused() {
        let hand = UserNode::from_ron(SOFT_CLAMP).unwrap();
        assert_eq!((hand.format_version, hand.id, hand.ident.as_str()), (2, AssetUuid::NIL, "soft_clamp"));
        assert_eq!(hand.kind(), "user::soft_clamp");

        let id = AssetUuid::new_v7();
        let with_id = SOFT_CLAMP.replace("ident:", &format!("id: \"{id}\", ident:"));
        let n = UserNode::from_ron(&with_id).unwrap();
        assert_eq!(n, UserNode { id, ..hand.clone() });
        assert_eq!(n.kind(), hand.kind(), "the registry kind stays `user::<ident>`");

        let e = UserNode::from_ron(&with_id.replace(&id.to_string(), "soft_clamp")).unwrap_err();
        assert!(e.contains("is not a UUID"), "{e}");
        let v1 = SOFT_CLAMP.replace("    format_version: 2,\n", "").replace("ident:", "id:");
        let e = UserNode::from_ron(&v1).unwrap_err();
        assert!(e.contains("older editor") && e.contains("shader-editor migrate"), "{e}");
        assert!(e.contains("add `format_version: 2`"), "{e}");
        let e = UserNode::from_ron(&SOFT_CLAMP.replace("format_version: 2", "format_version: 1")).unwrap_err();
        assert!(e.contains("older than this editor"), "{e}");
        let e = UserNode::from_ron(&SOFT_CLAMP.replace("format_version: 2", "format_version: 3")).unwrap_err();
        assert!(e.contains("newer"), "{e}");
    }

    /// The canonical writer (ADR 0047 §16) reads back to the same node, ids and pins included.
    #[test]
    fn to_ron_round_trip() {
        let mut n = UserNode::from_ron(SOFT_CLAMP).unwrap();
        let without_id = n.to_ron();
        assert!(without_id.starts_with("UserNode(\n  format_version: 2,\n  ident: \"soft_clamp\",\n"), "{without_id}");
        assert!(!without_id.contains("description") && !without_id.contains("\n  id: "), "{without_id}");
        assert_eq!(UserNode::from_ron(&without_id).unwrap(), n);

        let id = n.ensure_id();
        assert!(id.is_v7() && n.ensure_id() == id, "an id is minted once");
        n.description = "Soft clamp.\nLine two".into();
        n.template = "let y = x * k;\nreturn y;".into();
        let text = n.to_ron();
        assert!(text.starts_with(&format!("UserNode(\n  format_version: 2,\n  id: \"{id}\",\n  ident: ")), "{text}");
        assert_eq!(UserNode::from_ron(&text).unwrap(), n);
        assert_eq!(UserNode::from_ron(&text).unwrap().to_ron(), text);
    }

    /// The code identifier is checked.
    #[test]
    fn ident_is_checked() {
        for bad in ["SoftClamp", "fn", "soft-clamp", ""] {
            let text = SOFT_CLAMP.replace("\"soft_clamp\"", &format!("\"{bad}\""));
            let e = UserNode::from_ron(&text).unwrap_err();
            assert!(e.starts_with("user node ident:"), "{bad}: {e}");
        }
    }

    /// The registry kind is `user::<ident>`; a node with an id also maps its UUID to that kind, first one wins.
    #[test]
    fn register_records_the_asset_kind() {
        let mut r = Registry::default();
        let node = UserNode::from_ron(SOFT_CLAMP).unwrap();
        let id = AssetUuid::new_v7();
        UserNode { id, ident: "soft_a".into(), ..node.clone() }.register(&mut r);
        UserNode { id, ident: "soft_b".into(), ..node }.register(&mut r);
        assert_eq!(r.asset_kind(id), Some("user::soft_a"));
        assert!(r.get("user::soft_b").is_some());
        assert_eq!(r.asset_kind(AssetUuid::new_v7()), None);
        // The reverse: what a graph stores for a kind.
        assert_eq!(r.kind_assets("user::soft_a").collect::<Vec<_>>(), [id]);
        assert_eq!(r.kind_assets("user::soft_b").count(), 0);
    }

    /// ADR 0047 §16: a hand-written node without an id registers, with no UUID to be referenced by (AE004).
    #[test]
    fn register_records_a_node_without_id() {
        let mut r = Registry::default();
        UserNode::from_ron(SOFT_CLAMP).unwrap().register(&mut r);
        assert!(r.get("user::soft_clamp").is_some());
        assert_eq!(r.kind_assets("user::soft_clamp").count(), 0);
    }

    #[test]
    fn return_keyword_detection_skips_identifiers_and_comments() {
        assert!(!has_return_keyword("returned_val * 2.0"));
        assert!(!has_return_keyword("x + 1.0 // return x"));
        assert!(!has_return_keyword("x + 1.0 /* return */"));
        assert!(has_return_keyword("return x + 1.0"));
        assert!(has_return_keyword("if (x > 0.0) { return 1.0; } return 0.0"));
    }
}
