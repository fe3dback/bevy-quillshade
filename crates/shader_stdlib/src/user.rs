//! Data-driven user nodes from `quillshade/material/.nodes/*.ron` (no editor rebuild needed).
//!
//! ```ron
//! UserNode(
//!     id: "soft_clamp",
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

use serde::Deserialize;
use shader_graph_core::registry::{NodeDef, NodeInfo, PinDef, Registry, SigCtx, Signature};
use shader_graph_core::{Code, EmitCtx, PinType, Value, ident::check_code_ident};

#[derive(Debug, Clone, Deserialize)]
pub struct UserPin {
    pub name: String,
    pub ty: PinType,
    #[serde(default)]
    pub default: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UserNode {
    pub id: String,
    pub name: String,
    #[serde(default = "default_category")]
    pub category: String,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub description: String,
    pub inputs: Vec<UserPin>,
    pub output: PinType,
    pub template: String,
}

fn default_category() -> String {
    "User".into()
}

struct UserDef {
    info: NodeInfo,
    node: UserNode,
}

impl UserNode {
    /// Parses and validates a `.nodes/*.ron` file.
    pub fn from_ron(text: &str) -> Result<UserNode, String> {
        let n: UserNode = ron::from_str(text).map_err(|e| e.to_string())?;
        check_code_ident(&n.id).map_err(|e| format!("user node id: {e}"))?;
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

    pub fn kind(&self) -> String {
        format!("user::{}", self.id)
    }

    pub fn register(self, r: &mut Registry) {
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
        let name = format!("sg_user_{}", self.node.id);
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
                "UserNode(id: \"soft\", name: \"Soft\", inputs: [UserPin(name: \"{input}\", ty: Float)], output: Float, \
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
        let input = "UserNode(id: \"soft\", name: \"Soft\", inputs: [UserPin(name: \"x\", ty: MaterialAttributesPbr)], \
                     output: Float, template: \"x\")";
        let e = UserNode::from_ron(input).unwrap_err();
        assert!(e.contains("MaterialAttributes (PBR)"), "{e}");
        let output =
            "UserNode(id: \"soft\", name: \"Soft\", inputs: [], output: MaterialAttributesPbr, template: \"x\")";
        let e = UserNode::from_ron(output).unwrap_err();
        assert!(e.contains("MaterialAttributes (PBR)"), "{e}");
        let e = crate::nodes::utility::parse_custom_inputs("a: MaterialAttributes (PBR)").unwrap_err();
        assert!(e.starts_with("unknown type") && e.ends_with("Int, Texture2D)"), "{e}");
        let cb_in = "UserNode(id: \"soft\", name: \"Soft\", inputs: [UserPin(name: \"x\", ty: CompileBool)], \
                     output: Float, template: \"1.0\")";
        assert!(UserNode::from_ron(cb_in).unwrap_err().contains("CompileBool"));
        let cb_out = "UserNode(id: \"soft\", name: \"Soft\", inputs: [], output: CompileBool, template: \"true\")";
        assert!(UserNode::from_ron(cb_out).unwrap_err().contains("CompileBool"));
        assert!(crate::nodes::utility::parse_custom_inputs("a: CompileBool").is_err());
        assert!(crate::nodes::utility::parse_custom_inputs("a: RuntimeBool, b: Bool").is_ok());
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
