//! Node palette search (spec §9.3): name, category and synonyms, UE-like (`*` = Multiply, `lerp`, `+`).

use std::collections::BTreeMap;

pub use editor_core::search::{Searchable, score, search, search_indices, search_indices_leading};
use shader_graph_core::consts::NoConsts;
use shader_graph_core::model::{Asset, AssetKind};
use shader_graph_core::registry::{
    FUNCTION_PREFIX, FunctionLookup, NoCurves, NoGradients, NoTextures, NodeRole, ROUTE_PARAM, Registry, SigCtx,
};
use shader_graph_core::{Env, FunctionSig, PinType, Value, conversion};

/// One palette item.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub kind: String,
    pub name: String,
    pub category: String,
    pub keywords: Vec<String>,
    pub description: String,
    /// Preset parameters of the new node.
    pub params: Vec<(String, Value)>,
}

impl Searchable for Entry {
    fn name(&self) -> &str {
        &self.name
    }
    fn category(&self) -> &str {
        &self.category
    }
    fn keywords(&self) -> &[String] {
        &self.keywords
    }
    fn description(&self) -> &str {
        &self.description
    }
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
            // Usages are offered per declaration ([`route_entries`]).
            NodeRole::MaterialOutput | NodeRole::RouteUsage => false,
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
            params: Vec::new(),
        });
    }
    for (path, sig) in functions {
        if is_self_call(path, current) {
            continue;
        }
        out.push(Entry {
            kind: format!("{FUNCTION_PREFIX}{path}"),
            name: sig.name.clone(),
            category: "Functions".into(),
            keywords: vec![path.clone()],
            description: format!("Call function `{path}`"),
            params: Vec::new(),
        });
    }
    out.sort_by(|a, b| (&a.category, &a.name).cmp(&(&b.category, &b.name)));
    out
}

/// Whether calling the function `path` from the asset `current` would be recursive (E006): the palette hides such
/// entries and a drop of the function onto the graph is rejected (design §4.11).
pub fn is_self_call(path: &str, current: &str) -> bool {
    path == current
}

/// One "Named Route Usage" entry per declaration in `asset` (UE-like): picking it creates a usage of that route.
pub fn route_entries(registry: &Registry, asset: &Asset) -> Vec<Entry> {
    let Some(def) = registry.defs().find(|d| d.role() == NodeRole::RouteUsage) else { return Vec::new() };
    let info = def.info();
    crate::doc::route_declarations(asset, registry)
        .into_iter()
        .map(|(id, name)| Entry {
            kind: info.id.clone(),
            name: format!("{}: {name}", info.name),
            category: info.category.clone(),
            keywords: info.keywords.iter().cloned().chain([name.clone()]).collect(),
            description: format!("Outputs the input of Named Route `{name}`"),
            params: vec![(ROUTE_PARAM.to_string(), Value::Str(id))],
        })
        .collect()
}

/// Whether `kind` is a Named Route declaration (decision 0011): a name for a wire, renamed as soon as the palette
/// adds it (design §6.4).
pub fn is_route_declaration(registry: &Registry, kind: &str) -> bool {
    registry.get(kind).is_some_and(|d| d.role() == NodeRole::RouteDeclaration)
}

/// Input pin of a fresh node: id, type, polymorphic, also takes material attributes (decision 0023).
pub type FreshPin = (String, PinType, bool, bool);

/// Signature of a freshly created node of `kind` (default params, nothing connected).
pub fn fresh_pins(
    registry: &Registry,
    functions: &dyn FunctionLookup,
    asset_kind: AssetKind,
    kind: &str,
) -> Option<(Vec<FreshPin>, Vec<(String, PinType)>)> {
    let def = registry.get(kind)?;
    let params = registry.resolved_params(kind, &BTreeMap::new());
    let connected = BTreeMap::new();
    let inputs: BTreeMap<String, Value> = BTreeMap::new();
    let env = Env {
        asset_kind,
        asset_path: "",
        functions,
        curves: &NoCurves,
        consts: &NoConsts,
        textures: &NoTextures,
        gradients: &NoGradients,
    };
    let sig = def.signature(&SigCtx {
        node_id: "new",
        kind,
        params: &params,
        connected: &connected,
        input_values: &inputs,
        env: &env,
    });
    Some((
        sig.inputs.iter().filter(|p| !p.inactive).map(|p| (p.id.clone(), p.ty, p.poly, p.attributes)).collect(),
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
            let poly_ok =
                |poly: bool, attrs: bool| poly && (t.is_numeric() || (attrs && t == PinType::MaterialAttributesPbr));
            ins.into_iter()
                .find(|(_, ty, poly, attrs)| poly_ok(*poly, *attrs) || conversion(t, *ty).is_some())
                .map(|p| p.0)
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

    /// Design §6.4: a lead entry wins ties on the score (first with an empty query or its own keyword), never over a
    /// better match; the query still filters it.
    #[test]
    fn lead_entry_sorts_first_when_it_matches() {
        let reg = shader_stdlib::registry();
        let e = all();
        let lead = |x: &Entry| is_route_declaration(&reg, &x.kind);
        let first = |q: &str| search_indices_leading(&e, q, lead).first().map(|i| e[*i].kind.clone());
        assert_eq!(first("").as_deref(), Some("stdlib::NamedRoute"));
        assert_eq!(first("route").as_deref(), Some("stdlib::NamedRoute"));
        assert_eq!(first("*").as_deref(), Some("stdlib::Multiply"));
        // "namedroute" holds d-o-t as a subsequence: that weak match stays below the keyword matches (Reroute's
        // "dot", DotProduct) and leads only the other subsequence matches.
        let dot: Vec<&str> = search_indices_leading(&e, "dot", lead).into_iter().map(|i| e[i].kind.as_str()).collect();
        let at = |kind: &str| dot.iter().position(|k| *k == kind).unwrap();
        assert!(
            at("stdlib::Reroute") < at("stdlib::NamedRoute") && at("stdlib::DotProduct") < at("stdlib::NamedRoute")
        );
        assert!(
            dot[at("stdlib::NamedRoute") + 1..]
                .iter()
                .all(|k| score(e.iter().find(|x| x.kind == *k).unwrap(), "dot") <= 5)
        );
        assert!(!search_indices_leading(&e, "mul", lead).iter().any(|i| lead(&e[*i])));
        let rest: Vec<usize> = search_indices_leading(&e, "", lead).into_iter().skip(1).collect();
        let plain: Vec<usize> = search_indices(&e, "").into_iter().filter(|i| !lead(&e[*i])).collect();
        assert_eq!(rest, plain, "the others keep their order");
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

    #[test]
    fn wire_filter_attributes() {
        let reg = shader_stdlib::registry();
        let k = AssetKind::Material(MaterialKind::Pbr3d(PbrSettings::default()));
        let from = WireEnd::FromOutput(PinType::MaterialAttributesPbr);
        assert_eq!(compatible_pin(&reg, &NoFunctions, k, "stdlib::Lerp", from).as_deref(), Some("a"));
        assert_eq!(compatible_pin(&reg, &NoFunctions, k, "stdlib::Add", from), None);
        assert_eq!(compatible_pin(&reg, &NoFunctions, k, "stdlib::If", from), None);
        assert_eq!(
            compatible_pin(&reg, &NoFunctions, k, "stdlib::BreakMaterialAttributesPbr", from).as_deref(),
            Some("attributes")
        );
        assert_eq!(compatible_pin(&reg, &NoFunctions, k, "stdlib::Reroute", from).as_deref(), Some("in"));
    }
}
