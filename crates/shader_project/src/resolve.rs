//! The resolve seam (ADR 0047 §4): sources and documents store a referenced asset's UUID (`function::<uuid>` and
//! `user::<uuid>` node kinds, `Str("<uuid>")` values of the reference params), and everything that *reads*
//! references — [`plan_set`], `DepGraph`, the IR builder, codegen — sees the target's current path instead (a user
//! node's registered kind `user::<ident>`, §16), so generated code is identical to the path form.
//!
//! Anything that is not the UUID of a known Material asset of the expected kind — a path (hand-written),
//! `@internal/*`, `""`, a dangling UUID — passes through unchanged; dangling references are then reported by the
//! existing "does not exist" checks (plus `AE002` from the index).
//!
//! [`plan_set`]: crate::regen::plan_set

use std::sync::OnceLock;

use project_core::AssetUuid;
use project_core::asset_index::AssetIndex;
use shader_graph_core::Value;
use shader_graph_core::model::Asset;
use shader_graph_core::registry::{FUNCTION_PREFIX, ParamKind, Registry};
use shader_stdlib::user::USER_PREFIX;

use crate::assets::{AssetSet, AssetType};
use crate::layout::MATERIAL;

/// The asset type a reference param of kind `kind` names (`None`: not a reference to an asset).
pub fn ref_type(kind: &ParamKind) -> Option<AssetType> {
    match kind {
        ParamKind::Curve => Some(AssetType::Curve),
        ParamKind::GlobalConsts => Some(AssetType::Consts),
        ParamKind::Texture => Some(AssetType::Texture),
        ParamKind::GradientCollection => Some(AssetType::Gradients),
        _ => None,
    }
}

/// The current path of the Material asset of type `ty` whose UUID is `value`; `None` when `value` is not a UUID or
/// names no such asset (unknown, another domain or another kind).
pub fn resolve_ref<'a>(value: &str, ty: AssetType, index: &'a AssetIndex) -> Option<&'a str> {
    let entry = index.get(AssetUuid::parse(value)?)?;
    (entry.asset.domain == MATERIAL && entry.asset.suffix == ty.suffix()).then_some(entry.asset.path.as_str())
}

/// The registered kind (`user::<ident>`) of the user node whose UUID is `value`; `None` when `value` is not a UUID,
/// names no Material user node in `index` (unknown, another kind) or no node registered in `registry`.
pub fn resolve_user<'a>(value: &str, index: &AssetIndex, registry: &'a Registry) -> Option<&'a str> {
    let id = AssetUuid::parse(value)?;
    let entry = index.get(id)?;
    (entry.asset.domain == MATERIAL && entry.asset.suffix.is_empty()).then(|| registry.asset_kind(id)).flatten()
}

/// The value a reference to the Material asset of type `ty` at `path` stores (the write side of ADR 0047 §4): the
/// target's UUID from `index`. `None` when the target has no index entry (no such asset, or a file without an id
/// yet, AE004) or is a copy sharing another file's UUID (a duplicate loser, AE001: its UUID names the original, §8):
/// graphs reference assets by UUID only. A value that is no asset path — `@internal/*`, `""`, a UUID
/// stored already — is stored as it is.
pub fn ref_value(path: &str, ty: AssetType, index: &AssetIndex) -> Option<String> {
    if path.is_empty() || path.starts_with('@') || project_core::is_uuid_shaped(path) {
        return Some(path.to_string());
    }
    let r = project_core::AssetRef { domain: MATERIAL, path: path.to_string(), suffix: ty.suffix() };
    index.ref_id_of(&r).map(|id| id.to_string())
}

/// The node kind a graph stores for the kind `kind` as the editor shows it (the write side of ADR 0047 §4):
/// `function::<path>` becomes `function::<uuid>` ([`ref_value`]); a user node's `user::<ident>` becomes
/// `user::<uuid>` of the file the index knows for it ([`Registry::kind_assets`], resolving back to `kind`). `None`
/// for a function or user node without an id (AE004) or unknown; any other kind, and one stored as a UUID already,
/// is returned unchanged.
pub fn stored_kind(kind: &str, index: &AssetIndex, registry: &Registry) -> Option<String> {
    if let Some(path) = kind.strip_prefix(FUNCTION_PREFIX) {
        return ref_value(path, AssetType::Function, index).map(|v| format!("{FUNCTION_PREFIX}{v}"));
    }
    if let Some(ident) = kind.strip_prefix(USER_PREFIX) {
        if project_core::is_uuid_shaped(ident) {
            return Some(kind.to_string());
        }
        let id = registry.kind_assets(kind).find(|id| resolve_user(&id.to_string(), index, registry) == Some(kind));
        return id.map(|id| format!("{USER_PREFIX}{id}"));
    }
    Some(kind.to_string())
}

/// `asset` with every UUID reference replaced by the target's path: function node kinds, user node kinds (by the
/// kind `registry` records for the node's UUID, [`Registry::asset_kind`]) and the values of [`ParamKind::Curve`],
/// [`ParamKind::GlobalConsts`], [`ParamKind::Texture`] and [`ParamKind::GradientCollection`] params (found through
/// `registry`). Sub-item ids (`GlobalConst`, `GradientSlot`) are local and stay.
pub fn resolve_asset(asset: &Asset, index: &AssetIndex, registry: &Registry) -> Asset {
    let mut out = asset.clone();
    resolve_in_place(&mut out, index, registry);
    out
}

/// [`resolve_asset`] for every loaded graph of `set`, `registry` being the project's (stdlib + user nodes). Curves,
/// tables, textures and gradient collections store no references and are copied as they are.
pub fn resolve_set(set: &AssetSet, index: &AssetIndex, registry: &Registry) -> AssetSet {
    let mut out = set.clone();
    for asset in out.assets.values_mut().flatten() {
        resolve_in_place(asset, index, registry);
    }
    out
}

fn resolve_in_place(asset: &mut Asset, index: &AssetIndex, registry: &Registry) {
    for n in asset.nodes.values_mut() {
        if let Some(path) =
            n.kind.strip_prefix(FUNCTION_PREFIX).and_then(|t| resolve_ref(t, AssetType::Function, index))
        {
            n.kind = format!("{FUNCTION_PREFIX}{path}");
        }
        if let Some(kind) = n.kind.strip_prefix(USER_PREFIX).and_then(|t| resolve_user(t, index, registry)) {
            n.kind = kind.to_string();
        }
        let Some(def) = registry.get(&n.kind) else { continue };
        for p in def.params() {
            let Some(ty) = ref_type(&p.kind) else { continue };
            if let Some(Value::Str(v)) = n.params.get_mut(&p.id)
                && let Some(path) = resolve_ref(v, ty, index)
            {
                *v = path.to_string();
            }
        }
    }
}

/// The stdlib registry, built once.
pub(crate) fn stdlib() -> &'static Registry {
    static STDLIB: OnceLock<Registry> = OnceLock::new();
    STDLIB.get_or_init(shader_stdlib::registry)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use project_core::AssetRef;
    use project_core::asset_index::{DomainIndex, IndexedAsset};
    use shader_graph_core::model::MaterialKind;
    use shader_graph_core::model::{AssetKind, Node, UnlitSettings};
    use shader_stdlib::nodes::color::{GRADIENT_COLLECTION_PARAM, GRADIENT_SLOT_PARAM, SAMPLE_GRADIENT_KIND};
    use shader_stdlib::nodes::utility::SAMPLE_CURVE_KIND;

    use super::*;

    fn entry(ty: AssetType, path: &str) -> (AssetUuid, IndexedAsset) {
        let r = AssetRef { domain: MATERIAL, path: path.into(), suffix: ty.suffix() };
        let id = AssetUuid::legacy(&r.source_rel());
        (id, IndexedAsset { asset: r, id: Ok(id), sub_items: Vec::new(), refs: Vec::new() })
    }

    /// An index with one asset of every referenceable kind; returns it and the UUIDs in the order
    /// curve, table, texture, gradients, function.
    fn index() -> (AssetIndex, [AssetUuid; 5]) {
        let parts = [
            entry(AssetType::Curve, "common/pulse"),
            entry(AssetType::Consts, "common/tuning"),
            entry(AssetType::Texture, "textures/rock"),
            entry(AssetType::Gradients, "common/terrain"),
            entry(AssetType::Function, "common/layer"),
        ];
        let ids = parts.each_ref().map(|(id, _)| *id);
        let index = AssetIndex::build(vec![DomainIndex {
            domain: MATERIAL,
            assets: parts.into_iter().map(|(_, a)| a).collect(),
        }]);
        (index, ids)
    }

    fn graph(nodes: Vec<Node>) -> Asset {
        let mut a = Asset::new("m", AssetKind::Material(MaterialKind::Unlit3d(UnlitSettings::default())));
        for n in nodes {
            a.add_node(n);
        }
        a
    }

    fn str_param<'a>(a: &'a Asset, kind: &str, param: &str) -> &'a str {
        a.nodes.values().find(|n| n.kind == kind).unwrap().params[param].as_str()
    }

    #[test]
    fn uuid_refs_become_paths() {
        let (index, [curve, table, texture, gradients, _]) = index();
        let s = |id: AssetUuid| Value::Str(id.to_string());
        let a = graph(vec![
            Node::new(SAMPLE_CURVE_KIND, (0.0, 0.0)).with_param("curve", s(curve)),
            Node::new("stdlib::GlobalConstF32", (0.0, 0.0))
                .with_param("table", s(table))
                .with_param("const", Value::Int(7)),
            Node::new("stdlib::TextureSample", (0.0, 0.0)).with_param("texture", s(texture)),
            Node::new(SAMPLE_GRADIENT_KIND, (0.0, 0.0))
                .with_param(GRADIENT_COLLECTION_PARAM, s(gradients))
                .with_param(GRADIENT_SLOT_PARAM, Value::Int(3)),
        ]);
        let r = resolve_asset(&a, &index, stdlib());
        assert_eq!(str_param(&r, SAMPLE_CURVE_KIND, "curve"), "common/pulse");
        assert_eq!(str_param(&r, "stdlib::GlobalConstF32", "table"), "common/tuning");
        assert_eq!(
            r.nodes.values().find(|n| n.kind == "stdlib::GlobalConstF32").unwrap().params["const"],
            Value::Int(7)
        );
        assert_eq!(str_param(&r, "stdlib::TextureSample", "texture"), "textures/rock");
        assert_eq!(str_param(&r, SAMPLE_GRADIENT_KIND, GRADIENT_COLLECTION_PARAM), "common/terrain");
        assert_eq!(
            r.nodes.values().find(|n| n.kind == SAMPLE_GRADIENT_KIND).unwrap().params[GRADIENT_SLOT_PARAM],
            Value::Int(3)
        );
        // Everything else is untouched; resolving a resolved asset changes nothing.
        assert_eq!((r.nodes.len(), &r.edges, r.format_version), (a.nodes.len(), &a.edges, a.format_version));
        assert_eq!(resolve_asset(&r, &index, stdlib()), r);
    }

    #[test]
    fn paths_internal_and_dangling_pass_through() {
        let (index, [curve, ..]) = index();
        let dangling = AssetUuid::new_v7().to_string();
        let a = graph(vec![
            Node::new(SAMPLE_CURVE_KIND, (0.0, 0.0)).with_param("curve", Value::Str("common/pulse".into())),
            Node::new("stdlib::TextureSample", (0.0, 0.0))
                .with_param("texture", Value::Str("@internal/default".into())),
            Node::new("stdlib::GlobalConstF32", (0.0, 0.0)).with_param("table", Value::Str(dangling.clone())),
            // A known UUID of the wrong kind is not resolved.
            Node::new(SAMPLE_GRADIENT_KIND, (0.0, 0.0))
                .with_param(GRADIENT_COLLECTION_PARAM, Value::Str(curve.to_string())),
            Node::new(format!("{FUNCTION_PREFIX}common/layer"), (0.0, 0.0)),
            Node::new(format!("{FUNCTION_PREFIX}{dangling}"), (0.0, 0.0)),
            // Not a reference param: a UUID-shaped string stays.
            Node::new("stdlib::GlobalConstF32", (0.0, 0.0)).with_param("label", Value::Str(curve.to_string())),
        ]);
        assert_eq!(resolve_asset(&a, &index, stdlib()), a);
        assert_eq!(resolve_asset(&a, &AssetIndex::default(), stdlib()), a);
    }

    /// The write side: an indexed target is stored as its UUID, a target without an index entry not at all, a value
    /// that names no asset as it is; resolving what is stored gives the path back.
    #[test]
    fn paths_are_stored_as_uuids_when_indexed() {
        let (index, [curve, table, texture, gradients, function]) = index();
        for (ty, path, id) in [
            (AssetType::Curve, "common/pulse", curve),
            (AssetType::Consts, "common/tuning", table),
            (AssetType::Texture, "textures/rock", texture),
            (AssetType::Gradients, "common/terrain", gradients),
        ] {
            let stored = ref_value(path, ty, &index).unwrap();
            assert_eq!(stored, id.to_string());
            assert_eq!(resolve_ref(&stored, ty, &index), Some(path));
            assert_eq!(ref_value(path, ty, &AssetIndex::default()), None, "no index entry: not referenced");
        }
        // Another kind at that path and an unknown path are not referenced; built-ins, nothing and a UUID stay.
        assert_eq!(ref_value("common/pulse", AssetType::Texture, &index), None);
        assert_eq!(ref_value("textures/gone", AssetType::Curve, &index), None);
        for v in ["@internal/default", "", &curve.to_string()] {
            assert_eq!(ref_value(v, AssetType::Curve, &index).as_deref(), Some(v));
        }
        let stored = format!("{FUNCTION_PREFIX}{function}");
        assert_eq!(stored_kind("function::common/layer", &index, stdlib()).as_ref(), Some(&stored));
        assert_eq!(stored_kind(&stored, &index, stdlib()).as_ref(), Some(&stored), "stored already");
        assert_eq!(stored_kind("stdlib::Lerp", &index, stdlib()).as_deref(), Some("stdlib::Lerp"));
        for kind in ["function::common/gone", "user::soft_clamp"] {
            assert_eq!(stored_kind(kind, &index, stdlib()), None, "{kind}");
        }
    }

    #[test]
    fn function_kinds_resolve() {
        let (index, [curve, .., function]) = index();
        let a = graph(vec![
            Node::new(format!("{FUNCTION_PREFIX}{function}"), (0.0, 0.0)),
            Node::new(format!("{FUNCTION_PREFIX}{curve}"), (0.0, 0.0)),
        ]);
        let r = resolve_asset(&a, &index, stdlib());
        // Node ids are ULIDs: compare as a set, not in insertion order.
        let kinds: BTreeSet<&str> = r.nodes.values().map(|n| n.kind.as_str()).collect();
        let expected = [format!("{FUNCTION_PREFIX}common/layer"), format!("{FUNCTION_PREFIX}{curve}")];
        assert_eq!(kinds, expected.iter().map(String::as_str).collect());

        let mut set = AssetSet::default();
        set.assets.insert(crate::AssetId::new(AssetType::Material, "m"), Ok(a));
        set.assets.insert(crate::AssetId::new(AssetType::Material, "broken"), Err("bad".into()));
        let resolved = resolve_set(&set, &index, stdlib());
        assert_eq!(resolved.assets[&crate::AssetId::new(AssetType::Material, "m")], Ok(r));
        assert_eq!(resolved.assets[&crate::AssetId::new(AssetType::Material, "broken")], Err("bad".into()));
    }

    /// ADR 0047 §16: `user::<uuid>` resolves to the registered `user::<ident>` of the node the index knows under that
    /// UUID; idents, unknown UUIDs, UUIDs of other kinds and nodes the registry lacks pass through.
    #[test]
    fn user_node_kinds_resolve() {
        use shader_stdlib::user::UserNode;

        let (curves, [curve, ..]) = index();
        let node_ref = crate::assets::user_node_ref("soft_clamp");
        let soft = AssetUuid::legacy(&node_ref.source_rel());
        let unregistered_ref = crate::assets::user_node_ref("broken");
        let unregistered = AssetUuid::legacy(&unregistered_ref.source_rel());
        let node = |asset: AssetRef, id| IndexedAsset { asset, id: Ok(id), sub_items: Vec::new(), refs: Vec::new() };
        let mut parts = vec![node(node_ref.clone(), soft), node(unregistered_ref, unregistered)];
        parts.extend(curves.get(curve).map(|e| node(e.asset.clone(), curve)));
        let index = AssetIndex::build(vec![DomainIndex { domain: MATERIAL, assets: parts }]);
        assert_eq!(index.get(soft).unwrap().kind_id(), "material.node");

        let mut registry = shader_stdlib::registry();
        let text = "UserNode(format_version: 2, ident: \"soft_clamp\", name: \"SoftClamp\", inputs: [], output: Float, \
                    template: \"1.0\")";
        UserNode { id: soft, ..UserNode::from_ron(text).unwrap() }.register(&mut registry);
        // The curve's UUID maps to a kind in the registry, but the index says it is a curve.
        registry.register_asset_kind(curve, "user::soft_clamp");

        let dangling = AssetUuid::new_v7();
        let a = graph(vec![
            Node::new(format!("{USER_PREFIX}{soft}"), (0.0, 0.0)),
            Node::new(format!("{USER_PREFIX}soft_clamp"), (0.0, 0.0)),
            Node::new(format!("{USER_PREFIX}{dangling}"), (0.0, 0.0)),
            Node::new(format!("{USER_PREFIX}{curve}"), (0.0, 0.0)),
            Node::new(format!("{USER_PREFIX}{unregistered}"), (0.0, 0.0)),
        ]);
        let r = resolve_asset(&a, &index, &registry);
        for (id, n) in &a.nodes {
            let resolved = if n.kind == format!("{USER_PREFIX}{soft}") { "user::soft_clamp" } else { &n.kind };
            assert_eq!(r.nodes[id].kind, resolved, "{}", n.kind);
        }
        assert_eq!(resolve_asset(&a, &index, stdlib()), a, "not registered: passes through");
        // The write side: the ident becomes the UUID the index knows for it (not the curve's, mapped to it above).
        assert_eq!(stored_kind("user::soft_clamp", &index, &registry), Some(format!("{USER_PREFIX}{soft}")));
        assert_eq!(stored_kind("user::soft_clamp", &AssetIndex::default(), &registry), None);
        assert_eq!(stored_kind("user::soft_clamp", &index, stdlib()), None);
        let mut set = AssetSet::default();
        set.assets.insert(crate::AssetId::new(AssetType::Material, "m"), Ok(a));
        assert_eq!(resolve_set(&set, &index, &registry).assets[&crate::AssetId::new(AssetType::Material, "m")], Ok(r));
    }
}
