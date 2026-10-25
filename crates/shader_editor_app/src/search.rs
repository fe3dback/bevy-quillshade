//! Node palette search (spec §9.3): name, category and synonyms, UE-like (`*` = Multiply, `lerp`, `+`).

use std::collections::BTreeMap;

use shader_graph_core::model::AssetKind;
use shader_graph_core::registry::{FUNCTION_PREFIX, FunctionLookup, NodeRole, Registry, SigCtx};
use shader_graph_core::{Env, FunctionSig, PinType, Value, conversion};

/// One palette item.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub kind: String,
    pub name: String,
    pub category: String,
    pub keywords: Vec<String>,
    pub description: String,
}

/// Which end of a wire the palette was opened from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireEnd {
    /// Dragged from an output of this type: the new node needs a compatible input.
    FromOutput(PinType),
    /// Dragged from an input of this type: the new node needs a compatible output.
    FromInput(PinType),
}

/// Palette entries available in an asset of `kind`.
pub fn entries(
    registry: &Registry,
    functions: &BTreeMap<String, FunctionSig>,
    kind: AssetKind,
    current: &str,
) -> Vec<Entry> {
    let mut out = Vec::new();
    for def in registry.defs() {
        let info = def.info();
        let allowed = match def.role() {
            NodeRole::MaterialOutput => false,
            NodeRole::FunctionInput | NodeRole::FunctionOutput => kind.is_function(),
            NodeRole::Parameter => !kind.is_function(),
            _ => info.id != shader_graph_core::registry::FUNCTION_CALL_KIND,
        };
        let sprite_only = info.id == "stdlib::SpriteTexture";
        if !allowed || (sprite_only && !kind.material().is_some_and(|m| m.is_2d())) {
            continue;
        }
        out.push(Entry {
            kind: info.id.clone(),
            name: info.name.clone(),
            category: info.category.clone(),
            keywords: info.keywords.clone(),
            description: info.description.clone(),
        });
    }
    for (path, sig) in functions {
        if path == current {
            continue;
        }
        out.push(Entry {
            kind: format!("{FUNCTION_PREFIX}{path}"),
            name: sig.name.clone(),
            category: "Functions".into(),
            keywords: vec![path.clone()],
            description: format!("Call function `{path}`"),
        });
    }
    out.sort_by(|a, b| (&a.category, &a.name).cmp(&(&b.category, &b.name)));
    out
}

fn squash(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace() && *c != '_').flat_map(char::to_lowercase).collect()
}

fn is_subsequence(needle: &str, hay: &str) -> bool {
    let mut it = hay.chars();
    needle.chars().all(|c| it.any(|h| h == c))
}

/// Relevance of `e` for `query` (0 = no match).
pub fn score(e: &Entry, query: &str) -> u32 {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return 1;
    }
    let qs = squash(&q);
    let name = e.name.to_lowercase();
    let sname = squash(&e.name);
    let kws: Vec<String> = e.keywords.iter().map(|k| k.to_lowercase()).collect();
    if kws.contains(&q) {
        return 100;
    }
    if sname == qs {
        return 95;
    }
    if sname.starts_with(&qs) {
        return 80;
    }
    if name.split(|c: char| c.is_whitespace() || c == '_').any(|w| w.starts_with(&q)) {
        return 70;
    }
    if kws.iter().any(|k| k.starts_with(&q)) {
        return 60;
    }
    if sname.contains(&qs) {
        return 50;
    }
    if e.category.to_lowercase().contains(&q) {
        return 30;
    }
    if e.description.to_lowercase().contains(&q) {
        return 10;
    }
    if qs.len() >= 2 && is_subsequence(&qs, &sname) {
        return 5;
    }
    0
}

/// Matching entries, best first (stable by category/name on ties).
pub fn search<'a>(entries: &'a [Entry], query: &str) -> Vec<&'a Entry> {
    let mut hits: Vec<(u32, usize)> =
        entries.iter().enumerate().map(|(i, e)| (score(e, query), i)).filter(|(s, _)| *s > 0).collect();
    hits.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    hits.into_iter().map(|(_, i)| &entries[i]).collect()
}

/// Signature of a freshly created node of `kind` (default params, nothing connected).
pub fn fresh_pins(
    registry: &Registry,
    functions: &dyn FunctionLookup,
    asset_kind: AssetKind,
    kind: &str,
) -> Option<(Vec<(String, PinType, bool)>, Vec<(String, PinType)>)> {
    let def = registry.get(kind)?;
    let params = registry.resolved_params(kind, &BTreeMap::new());
    let connected = BTreeMap::new();
    let inputs: BTreeMap<String, Value> = BTreeMap::new();
    let env = Env { asset_kind, asset_path: "", functions };
    let sig = def.signature(&SigCtx {
        node_id: "new",
        kind,
        params: &params,
        connected: &connected,
        input_values: &inputs,
        env: &env,
    });
    Some((
        sig.inputs.iter().filter(|p| !p.inactive).map(|p| (p.id.clone(), p.ty, p.poly)).collect(),
        sig.outputs.iter().map(|p| (p.id.clone(), p.ty)).collect(),
    ))
}

/// First pin of a new `kind` node that a wire of `end` can attach to.
pub fn compatible_pin(
    registry: &Registry,
    functions: &dyn FunctionLookup,
    asset_kind: AssetKind,
    kind: &str,
    end: WireEnd,
) -> Option<String> {
    let (ins, outs) = fresh_pins(registry, functions, asset_kind, kind)?;
    match end {
        WireEnd::FromOutput(t) => {
            ins.into_iter().find(|(_, ty, poly)| (*poly && t.is_numeric()) || conversion(t, *ty).is_some()).map(|p| p.0)
        }
        WireEnd::FromInput(t) => outs.into_iter().find(|(_, ty)| conversion(*ty, t).is_some()).map(|p| p.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shader_graph_core::model::{MaterialKind, PbrSettings};
    use shader_graph_core::registry::NoFunctions;

    fn all() -> Vec<Entry> {
        entries(
            &shader_stdlib::registry(),
            &BTreeMap::new(),
            AssetKind::Material(MaterialKind::Pbr3d(PbrSettings::default())),
            "x",
        )
    }

    #[test]
    fn synonyms_rank_first() {
        let e = all();
        assert_eq!(search(&e, "*")[0].kind, "stdlib::Multiply");
        assert_eq!(search(&e, "+")[0].kind, "stdlib::Add");
        assert_eq!(search(&e, "lerp")[0].kind, "stdlib::Lerp");
        assert_eq!(search(&e, "texture sample")[0].kind, "stdlib::TextureSample");
        assert!(search(&e, "zzzzqqq").is_empty());
        assert!(!e.iter().any(|x| x.kind.starts_with("output::")));
    }

    #[test]
    fn every_stdlib_node_is_findable() {
        let e = all();
        for entry in &e {
            assert!(search(&e, &entry.name).iter().any(|x| x.kind == entry.kind), "{} not found", entry.name);
        }
    }

    #[test]
    fn wire_filter() {
        let reg = shader_stdlib::registry();
        let k = AssetKind::Material(MaterialKind::Pbr3d(PbrSettings::default()));
        assert!(
            compatible_pin(&reg, &NoFunctions, k, "stdlib::Multiply", WireEnd::FromOutput(PinType::Float3)).is_some()
        );
        assert!(
            compatible_pin(&reg, &NoFunctions, k, "stdlib::Multiply", WireEnd::FromOutput(PinType::Texture2D))
                .is_none()
        );
        assert_eq!(
            compatible_pin(&reg, &NoFunctions, k, "stdlib::TextureSample", WireEnd::FromOutput(PinType::Texture2D))
                .as_deref(),
            Some("tex")
        );
        assert!(compatible_pin(&reg, &NoFunctions, k, "stdlib::Time", WireEnd::FromInput(PinType::Float)).is_some());
    }
}
