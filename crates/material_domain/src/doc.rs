//! Open document: the model (source of truth), undo/redo history, clipboard ops.
//!
//! Pure data, no Bevy: the canvas is rebuilt from [`Doc::asset`] after every edit.

use std::collections::{BTreeMap, BTreeSet};

use shader_graph_core::consts::NoConsts;
use shader_graph_core::ir::{with_routes, would_create_cycle};
use shader_graph_core::model::{Asset, Edge, Frame, Node, NodeId, Preview, new_node_id};
use shader_graph_core::registry::{
    FUNCTION_PREFIX, FunctionLookup, NoCurves, NoGradients, NoTextures, NodeRole, ParamKind, ROUTE_PARAM, Registry,
    SigCtx, Signature,
};
use shader_graph_core::{Env, FunctionSig, PinType, Value, conversion};
use shader_project::AssetId;

use crate::history::{History, MergeKey};

/// Copied nodes with the edges between them.
#[derive(Debug, Clone, Default)]
pub struct ClipboardData {
    pub nodes: BTreeMap<NodeId, Node>,
    pub edges: Vec<Edge>,
    /// The graph the nodes were copied from: a Named Route declaration pasted back into it becomes a usage
    /// (decision 0038).
    pub source: Option<AssetId>,
}

#[derive(Debug, Clone)]
pub struct Doc {
    pub id: AssetId,
    /// [`Doc::id`] as the core names it ([`crate::doc_kind::TabDoc::asset`]); follows renames.
    pub(crate) aref: project_core::AssetRef,
    pub asset: Asset,
    /// RON text of the last saved state (dirty check).
    pub saved: String,
    history: History<Asset>,
    pub selection: BTreeSet<NodeId>,
    /// Selected comment frame (frames are not nodes of the model graph).
    pub selected_frame: Option<String>,
    /// Comment frames a box selection fully enclosed together with `selection` (design §4.5): Delete removes them
    /// with the nodes. Only frames whose `Frame::nodes` are all selected ([`Doc::enclosed`]); cleared by a new node
    /// selection ([`Doc::set_selection`]).
    pub enclosed_frames: BTreeSet<String>,
    /// Bumped on every model change; the canvas rebuilds when it differs.
    pub revision: u64,
    /// The generated files were written from this unsaved document (a preview toggle, decision 0022): closing it
    /// unsaved regenerates the asset from disk.
    pub regen_from_memory: bool,
    /// The preview the editor last followed (decision 0022): when `asset.preview` moves away from it, the material
    /// regenerates from memory (`Editor::material_after_apply`). Set on load and reload.
    pub preview_seen: Preview,
    /// [`Doc::reload`] dropped unsaved edits the generated files were written from (decision 0022): the material
    /// regenerates from disk (`Editor::material_after_apply`).
    pub restore_from_disk: bool,
}

impl Doc {
    pub fn new(id: AssetId, asset: Asset) -> Self {
        let saved = asset.to_ron();
        let asset_preview = asset.preview.clone();
        Doc {
            aref: id.to_ref(),
            id,
            asset,
            saved,
            history: History::default(),
            selection: BTreeSet::new(),
            selected_frame: None,
            enclosed_frames: BTreeSet::new(),
            revision: 1,
            regen_from_memory: false,
            preview_seen: asset_preview,
            restore_from_disk: false,
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.asset.to_ron() != self.saved
    }

    /// Replaces the node selection; the frames a box enclosed with the old one are forgotten. Every direct write of
    /// `selection` goes through here (pruning deleted nodes aside).
    pub fn set_selection(&mut self, sel: impl IntoIterator<Item = NodeId>) {
        self.selection = sel.into_iter().collect();
        self.enclosed_frames.clear();
    }

    pub fn mark_saved(&mut self) {
        self.saved = self.asset.to_ron();
        self.regen_from_memory = false;
    }

    /// Applies `f` as one undoable step; returns its result. No-op edits are not recorded.
    pub fn edit<R>(&mut self, f: impl FnOnce(&mut Asset) -> R) -> R {
        self.edit_merge(None, f)
    }

    /// Like [`Doc::edit`]; consecutive edits with the same `key` share one undo step.
    pub fn edit_merge<R>(&mut self, key: Option<MergeKey>, f: impl FnOnce(&mut Asset) -> R) -> R {
        let before = self.asset.clone();
        let r = f(&mut self.asset);
        self.asset.normalize();
        // Deleting the previewed node turns the preview off (decision 0022).
        self.asset.clear_dangling_preview();
        let changed = self.asset != before;
        self.history.record(changed.then_some(before), key);
        if changed {
            self.revision += 1;
            self.selection.retain(|n| self.asset.nodes.contains_key(n));
        }
        r
    }

    /// Ends the current merge group (e.g. a number drag finished).
    pub fn end_merge(&mut self) {
        self.history.end_merge();
    }

    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    pub fn undo(&mut self) -> bool {
        let done = self.history.undo(&mut self.asset);
        if done {
            self.after_history();
        }
        done
    }

    pub fn redo(&mut self) -> bool {
        let done = self.history.redo(&mut self.asset);
        if done {
            self.after_history();
        }
        done
    }

    fn after_history(&mut self) {
        self.revision += 1;
        self.selection.retain(|n| self.asset.nodes.contains_key(n));
    }

    /// Replaces the model (external reload) and clears history.
    pub fn reload(&mut self, asset: Asset) {
        self.restore_from_disk |= self.regen_from_memory && self.is_dirty();
        self.preview_seen = asset.preview.clone();
        self.asset = asset;
        self.mark_saved();
        self.history.clear();
        self.after_history();
    }

    pub fn add_node(&mut self, kind: &str, pos: (f32, f32)) -> NodeId {
        self.add_node_with(kind, pos, &[])
    }

    /// Adds a node with preset parameters (e.g. the route of a Named Route Usage).
    pub fn add_node_with(&mut self, kind: &str, pos: (f32, f32), params: &[(String, Value)]) -> NodeId {
        let id = new_node_id();
        let mut node = Node::new(kind, pos);
        node.params.extend(params.iter().cloned());
        let nid = id.clone();
        self.edit(move |a| {
            a.nodes.insert(nid, node);
        });
        id
    }

    /// Deletes nodes; output nodes of materials are kept (spec §7.3).
    pub fn delete_nodes(&mut self, ids: &BTreeSet<NodeId>, registry: &Registry) -> usize {
        self.delete_nodes_and_frames(ids, &BTreeSet::new(), registry)
    }

    /// [`Doc::delete_nodes`] and the comment frames `frames` in one undoable step. A frame with a member node that
    /// survives (an output node) is kept (decision 0033).
    pub fn delete_nodes_and_frames(
        &mut self,
        ids: &BTreeSet<NodeId>,
        frames: &BTreeSet<String>,
        registry: &Registry,
    ) -> usize {
        let deletable: Vec<NodeId> = ids
            .iter()
            .filter(|id| {
                self.asset
                    .nodes
                    .get(*id)
                    .is_some_and(|n| registry.get(&n.kind).is_none_or(|d| d.role() != NodeRole::MaterialOutput))
            })
            .cloned()
            .collect();
        let n = deletable.len();
        self.edit(|a| {
            for id in &deletable {
                a.remove_node(id);
            }
            let nodes = &a.nodes;
            a.comments.retain(|f| !frames.contains(&f.id) || f.nodes.iter().any(|n| nodes.contains_key(n)));
        });
        n
    }

    pub fn copy(&self, ids: &BTreeSet<NodeId>, registry: &Registry) -> ClipboardData {
        let nodes: BTreeMap<NodeId, Node> = self
            .asset
            .nodes
            .iter()
            .filter(|(id, n)| {
                ids.contains(*id) && registry.get(&n.kind).is_none_or(|d| d.role() != NodeRole::MaterialOutput)
            })
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let edges = self
            .asset
            .edges
            .iter()
            .filter(|e| nodes.contains_key(&e.from.0) && nodes.contains_key(&e.to.0))
            .cloned()
            .collect();
        ClipboardData { nodes, edges, source: Some(self.id.clone()) }
    }

    /// Pastes with fresh ids at `offset`; selects and returns the new nodes.
    ///
    /// A Named Route declaration whose original still is a declaration of this graph is pasted as a usage of it
    /// (decision 0038): its output wires leave the usage, wires into it are dropped (a usage has no input).
    pub fn paste(&mut self, clip: &ClipboardData, offset: (f32, f32), registry: &Registry) -> BTreeSet<NodeId> {
        let map: BTreeMap<NodeId, NodeId> = clip.nodes.keys().map(|k| (k.clone(), new_node_id())).collect();
        let new_ids: BTreeSet<NodeId> = map.values().cloned().collect();
        let role = |n: &Node| registry.get(&n.kind).map(|d| d.role());
        let declaration = |n: &Node| role(n) == Some(NodeRole::RouteDeclaration);
        let usage_kind = registry.defs().find(|d| d.role() == NodeRole::RouteUsage).map(|d| d.info().id.clone());
        let as_usage: BTreeSet<&NodeId> = match (&usage_kind, &clip.source) {
            (Some(_), Some(source)) if *source == self.id => clip
                .nodes
                .iter()
                .filter(|(id, n)| declaration(n) && self.asset.nodes.get(*id).is_some_and(declaration))
                .map(|(id, _)| id)
                .collect(),
            _ => BTreeSet::new(),
        };
        self.edit(|a| {
            for (old, node) in &clip.nodes {
                let mut n = node.clone();
                if as_usage.contains(old)
                    && let Some(kind) = &usage_kind
                {
                    n = Node::new(kind.clone(), n.pos).with_param(ROUTE_PARAM, Value::Str(old.clone()));
                }
                n.pos = tidy((n.pos.0 + offset.0, n.pos.1 + offset.1));
                // A usage pasted together with a duplicated declaration follows the copy, otherwise the original.
                if let Some(Value::Str(route)) = n.params.get_mut(ROUTE_PARAM)
                    && !as_usage.contains(route)
                    && let Some(new) = map.get(route.as_str())
                {
                    *route = new.clone();
                }
                a.nodes.insert(map[old].clone(), n);
            }
            // Both route nodes are pass-throughs with the output `out`, so output wires keep their pin.
            for e in clip.edges.iter().filter(|e| !as_usage.contains(&e.to.0)) {
                a.edges.push(Edge {
                    from: (map[&e.from.0].clone(), e.from.1.clone()),
                    to: (map[&e.to.0].clone(), e.to.1.clone()),
                });
            }
        });
        self.set_selection(new_ids.iter().cloned());
        new_ids
    }

    /// The frames of `frames` that may be deleted together with the node selection `nodes`: there is a node
    /// selection and every node of the frame is in it.
    pub fn enclosed(&self, frames: &BTreeSet<String>, nodes: &BTreeSet<NodeId>) -> BTreeSet<String> {
        let comments = self.asset.comments.iter().map(|f| (f.id.as_str(), f.nodes.as_slice()));
        node_graph_base::layout::enclosed(comments, frames, nodes)
    }

    /// Adds a comment frame around `nodes` (one edit); around no node it is [`EMPTY_FRAME_SIZE`] with its top-left
    /// corner at `at` (design §6.1). Returns its id.
    pub fn add_frame(&mut self, title: &str, nodes: &BTreeSet<NodeId>, at: (f32, f32)) -> String {
        let id = format!("c{}", new_node_id());
        // `bounds` of an empty set is the whole graph.
        let around = if nodes.is_empty() { None } else { bounds(&self.asset, nodes) };
        let rect = around.map(|(x0, y0, x1, y1)| (x0 - 30.0, y0 - 50.0, x1 - x0 + 260.0, y1 - y0 + 220.0));
        let fid = id.clone();
        self.edit(|a| {
            a.comments.push(Frame {
                id: fid,
                title: title.into(),
                rect: rect.unwrap_or((at.0, at.1, EMPTY_FRAME_SIZE.0, EMPTY_FRAME_SIZE.1)),
                nodes: nodes.iter().cloned().collect(),
                color: None,
                hue: None,
            });
        });
        id
    }
}

pub use node_graph_base::layout::{EMPTY_FRAME_SIZE, GRID_SNAP, grid_point, snap, tidy};

/// Bounding box of node positions (top-left corners).
pub fn bounds(asset: &Asset, ids: &BTreeSet<NodeId>) -> Option<(f32, f32, f32, f32)> {
    node_graph_base::layout::bounds(
        asset.nodes.iter().filter(|(k, _)| ids.is_empty() || ids.contains(*k)).map(|(_, n)| n.pos),
    )
}

/// The declared name of the route a Named Route Usage reads (its resolved `params`), looked up in `routes`
/// ([`route_declarations`] of its asset); `None` when the declaration is missing.
pub(crate) fn usage_route<'a>(routes: &'a [(NodeId, String)], params: &BTreeMap<String, Value>) -> Option<&'a str> {
    let route = params.get(ROUTE_PARAM).map_or("", Value::as_str);
    routes.iter().find(|r| r.0 == route).map(|r| r.1.as_str())
}

fn short(v: &Value) -> String {
    let f = |x: f32| {
        let s = format!("{x:.3}");
        let s = s.trim_end_matches('0').trim_end_matches('.').to_string();
        if s.is_empty() || s == "-" { "0".into() } else { s }
    };
    match v {
        Value::Float(x) => f(*x),
        Value::Int(i) => i.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Str(s) => s.clone(),
        Value::Type(t) => t.label().to_string(),
        other => format!("({})", other.floats().iter().map(|x| f(*x)).collect::<Vec<_>>().join(", ")),
    }
}

/// Title shown on a node: kind name, or the parameter/function/constant it stands for.
/// Constant nodes carry an editor-only label that replaces the value in the title (decision 0021).
pub fn has_label(kind: &str) -> bool {
    matches!(
        kind,
        "stdlib::Constant" | "stdlib::Constant2Vector" | "stdlib::Constant3Vector" | "stdlib::Constant4Vector"
    )
}

pub fn node_title(registry: &Registry, functions: &BTreeMap<String, FunctionSig>, asset: &Asset, id: &str) -> String {
    node_title_in(registry, functions, asset, id, &route_declarations(asset, registry))
}

/// [`node_title`] with the asset's [`route_declarations`] already at hand (a canvas rebuild computes them once).
pub(crate) fn node_title_in(
    registry: &Registry,
    functions: &BTreeMap<String, FunctionSig>,
    asset: &Asset,
    id: &str,
    routes: &[(NodeId, String)],
) -> String {
    let Some(n) = asset.nodes.get(id) else { return String::new() };
    let Some(def) = registry.get(&n.kind) else { return format!("? {}", n.kind) };
    let params = registry.resolved_params(&n.kind, &n.params);
    let name = def.info().name.clone();
    let p = |k: &str| params.get(k).map(short).unwrap_or_default();
    match def.role() {
        NodeRole::Parameter | NodeRole::RouteDeclaration => format!("{} ({name})", p("name")),
        NodeRole::RouteUsage => format!("{} ({name})", usage_route(routes, &params).unwrap_or("?")),
        NodeRole::FunctionInput => format!("Input: {}", p("name")),
        NodeRole::FunctionOutput => format!("Output: {}", p("name")),
        _ if n.kind.starts_with(FUNCTION_PREFIX) => {
            let path = &n.kind[FUNCTION_PREFIX.len()..];
            format!("fn {}", functions.get(path).map_or(path, |s| s.name.as_str()))
        }
        _ if has_label(&n.kind) && !n.label.is_empty() => n.label.clone(),
        _ if n.kind.starts_with("stdlib::Constant") => params.get("value").map(short).unwrap_or(name),
        _ => {
            // Sample Curve: the curve's name (decision 0015); Global*: the constant's name (decision 0017); texture
            // nodes: the Texture asset's name; Sample Gradient: the collection's name (the gradient's name shows in
            // its properties: the canvas rebuilds only on graph revisions, not on edits of the collection).
            let curve = def
                .params()
                .into_iter()
                .find(|p| {
                    matches!(
                        p.kind,
                        ParamKind::Curve
                            | ParamKind::GlobalConst(_)
                            | ParamKind::Texture
                            | ParamKind::GradientCollection
                    )
                })
                .map(|c| p(&c.id));
            match curve.filter(|c| !c.is_empty()) {
                Some(path) => format!("{} ({name})", path.rsplit('/').next().unwrap_or(&path)),
                None => name,
            }
        }
    }
}

/// Named route declarations of `asset` as (node id, name), sorted by name.
pub fn route_declarations(asset: &Asset, registry: &Registry) -> Vec<(NodeId, String)> {
    let mut out: Vec<(NodeId, String)> = asset
        .nodes
        .iter()
        .filter(|(_, n)| registry.get(&n.kind).is_some_and(|d| d.role() == NodeRole::RouteDeclaration))
        .map(|(id, n)| {
            let params = registry.resolved_params(&n.kind, &n.params);
            (id.clone(), params.get("name").map(|v| v.as_str().to_string()).unwrap_or_default())
        })
        .collect();
    out.sort_by(|a, b| (&a.1, &a.0).cmp(&(&b.1, &b.0)));
    out
}

/// Typed view of a node for the UI: signature resolved against the current model.
pub fn node_signature(
    asset: &Asset,
    id: &str,
    registry: &Registry,
    functions: &dyn FunctionLookup,
    path: &str,
) -> Option<Signature> {
    let routed = with_routes(asset, registry);
    let asset = routed.as_ref();
    let node = asset.nodes.get(id)?;
    let def = registry.get(&node.kind)?;
    let params = registry.resolved_params(&node.kind, &node.params);
    let connected = connected_types(asset, id, registry, functions, path, 0);
    let env = Env {
        asset_kind: asset.kind,
        asset_path: path,
        functions,
        curves: &NoCurves,
        consts: &NoConsts,
        textures: &NoTextures,
        gradients: &NoGradients,
    };
    Some(def.signature(&SigCtx {
        node_id: id,
        kind: &node.kind,
        params: &params,
        connected: &connected,
        input_values: &node.inputs,
        env: &env,
    }))
}

/// Source types of the connected inputs of `id` (recursive through upstream polymorphic nodes).
fn connected_types(
    asset: &Asset,
    id: &str,
    registry: &Registry,
    functions: &dyn FunctionLookup,
    path: &str,
    depth: u32,
) -> BTreeMap<String, PinType> {
    let mut out = BTreeMap::new();
    if depth > 64 {
        return out;
    }
    for e in asset.edges.iter().filter(|e| e.to.0 == id) {
        if let Some(t) = output_type(asset, &e.from.0, &e.from.1, registry, functions, path, depth + 1) {
            out.insert(e.to.1.clone(), t);
        }
    }
    out
}

fn output_type(
    asset: &Asset,
    node: &str,
    pin: &str,
    registry: &Registry,
    functions: &dyn FunctionLookup,
    path: &str,
    depth: u32,
) -> Option<PinType> {
    let n = asset.nodes.get(node)?;
    let def = registry.get(&n.kind)?;
    let params = registry.resolved_params(&n.kind, &n.params);
    let connected = connected_types(asset, node, registry, functions, path, depth);
    let env = Env {
        asset_kind: asset.kind,
        asset_path: path,
        functions,
        curves: &NoCurves,
        consts: &NoConsts,
        textures: &NoTextures,
        gradients: &NoGradients,
    };
    let sig = def.signature(&SigCtx {
        node_id: node,
        kind: &n.kind,
        params: &params,
        connected: &connected,
        input_values: &n.inputs,
        env: &env,
    });
    sig.output(pin).map(|p| p.ty)
}

/// Why a connection is refused (`None` = allowed).
pub fn connection_error(
    asset: &Asset,
    from: (&str, &str),
    to: (&str, &str),
    registry: &Registry,
    functions: &dyn FunctionLookup,
    path: &str,
) -> Option<String> {
    let routed = with_routes(asset, registry);
    let asset = routed.as_ref();
    if would_create_cycle(asset, from.0, to.0) {
        return Some("would create a cycle (E002)".into());
    }
    let from_ty = output_type(asset, from.0, from.1, registry, functions, path, 0)?;
    let sig = node_signature(asset, to.0, registry, functions, path)?;
    let pin = sig.input(to.1)?;
    let target = if pin.poly && pin.attributes {
        // Inputs that also take attributes (Lerp `a`/`b`, decision 0023): attributes and numbers never mix. Only
        // the node's other such inputs count; an edge into `to` itself is replaced by the new one.
        let attrs = PinType::MaterialAttributesPbr;
        let others: Vec<PinType> = sig
            .inputs
            .iter()
            .filter(|p| p.attributes && p.id != to.1)
            .filter_map(|p| asset.edge_into(to.0, &p.id))
            .filter_map(|e| output_type(asset, &e.from.0, &e.from.1, registry, functions, path, 0))
            .collect();
        let clash = if from_ty == attrs {
            others.into_iter().find(|t| *t != attrs)
        } else if from_ty.is_numeric() {
            others.into_iter().find(|t| *t == attrs)
        } else {
            Some(pin.ty)
        };
        clash?
    } else if pin.poly {
        // Polymorphic inputs accept any float-like value; the node resolves its type afterwards.
        if from_ty.is_numeric() { return None } else { pin.ty }
    } else {
        pin.ty
    };
    match conversion(from_ty, target) {
        Some(_) => None,
        None => Some(format!("cannot connect {} to {} (E003)", from_ty.label(), target.label())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shader_graph_core::model::{AssetKind, MaterialKind, PbrSettings};
    use shader_graph_core::registry::NoFunctions;
    use shader_project::AssetType;

    fn doc() -> Doc {
        let a = shader_project::ops::template("m", AssetKind::Material(MaterialKind::Pbr3d(PbrSettings::default())));
        Doc::new(AssetId::new(AssetType::Material, "m"), a)
    }

    #[test]
    fn set_selection_clears_enclosed_frames() {
        let mut d = doc();
        let out = d.asset.nodes.keys().next().unwrap().clone();
        d.selection = [out.clone()].into();
        d.enclosed_frames = ["f1".to_string()].into();
        d.set_selection([out.clone()]);
        assert_eq!(d.selection, [out].into());
        assert!(d.enclosed_frames.is_empty(), "a new node selection forgets the enclosed frames");
    }

    #[test]
    fn create_connect_undo_redo() {
        let reg = shader_stdlib::registry();
        let mut d = doc();
        assert!(!d.is_dirty());
        let out = d.asset.nodes.keys().next().unwrap().clone();
        let c = d.add_node("stdlib::Constant3Vector", (0.0, 0.0));
        assert!(d.is_dirty());
        assert!(connection_error(&d.asset, (&c, "out"), (&out, "base_color"), &reg, &NoFunctions, "m").is_none());
        d.edit(|a| a.connect((&c, "out"), (&out, "base_color")));
        assert_eq!(d.asset.edges.len(), 1);
        assert!(d.undo());
        assert!(d.asset.edges.is_empty());
        assert!(d.redo());
        assert_eq!(d.asset.edges.len(), 1);
        d.mark_saved();
        assert!(!d.is_dirty());
    }

    #[test]
    fn deleting_previewed_node_clears_preview() {
        use shader_graph_core::model::{Preview, PreviewMode};
        let mut d = doc();
        let c = d.add_node("stdlib::Constant3Vector", (0.0, 0.0));
        d.edit(|a| a.preview = Preview { node: Some(c.clone()), mode: PreviewMode::Lit });
        d.edit(|a| {
            a.remove_node(&c);
        });
        assert_eq!(d.asset.preview, Preview { node: None, mode: PreviewMode::Lit });
        assert!(d.undo());
        assert_eq!(d.asset.preview.node.as_deref(), Some(c.as_str()));
        assert!(d.redo());
        assert_eq!(d.asset.preview.node, None);
    }

    #[test]
    fn merged_edits_are_one_undo_step() {
        use shader_graph_core::Value;
        let mut d = doc();
        let c = d.add_node("stdlib::Constant", (0.0, 0.0));
        for v in [0.1, 0.2, 0.3] {
            d.edit_merge(Some("c.value".into()), |a| {
                a.nodes.get_mut(&c).unwrap().params.insert("value".into(), Value::Float(v))
            });
        }
        d.end_merge();
        d.edit_merge(Some("c.value".into()), |a| {
            a.nodes.get_mut(&c).unwrap().params.insert("value".into(), Value::Float(0.9))
        });
        assert!(d.undo());
        assert_eq!(d.asset.nodes[&c].params["value"], Value::Float(0.3));
        assert!(d.undo());
        assert!(!d.asset.nodes[&c].params.contains_key("value"));
        assert!(d.undo());
        assert!(!d.asset.nodes.contains_key(&c));
    }

    #[test]
    fn rejects_bad_connections() {
        let reg = shader_stdlib::registry();
        let mut d = doc();
        let out = d.asset.nodes.keys().next().unwrap().clone();
        let v2 = d.add_node("stdlib::Constant2Vector", (0.0, 0.0));
        let e = connection_error(&d.asset, (&v2, "out"), (&out, "base_color"), &reg, &NoFunctions, "m");
        assert!(e.unwrap().contains("E003"));
        let a = d.add_node("stdlib::Add", (0.0, 0.0));
        let b = d.add_node("stdlib::Add", (0.0, 0.0));
        d.edit(|m| m.connect((&a, "out"), (&b, "a")));
        assert!(connection_error(&d.asset, (&b, "out"), (&a, "a"), &reg, &NoFunctions, "m").unwrap().contains("E002"));
    }

    #[test]
    fn copy_paste_and_output_protection() {
        let reg = shader_stdlib::registry();
        let mut d = doc();
        let out = d.asset.nodes.keys().next().unwrap().clone();
        let a = d.add_node("stdlib::Add", (0.0, 0.0));
        let b = d.add_node("stdlib::Multiply", (100.0, 0.0));
        d.edit(|m| m.connect((&a, "out"), (&b, "a")));
        let sel: BTreeSet<NodeId> = [a.clone(), b.clone(), out.clone()].into();
        let clip = d.copy(&sel, &reg);
        assert_eq!(clip.nodes.len(), 2);
        let pasted = d.paste(&clip, (40.0, 40.0), &reg);
        assert_eq!(pasted.len(), 2);
        assert_eq!(d.asset.nodes.len(), 5);
        assert_eq!(d.asset.edges.len(), 2);
        assert_eq!(d.delete_nodes(&sel, &reg), 2);
        assert!(d.asset.nodes.contains_key(&out));
    }

    const DECL: &str = "stdlib::NamedRoute";
    const USAGE: &str = "stdlib::NamedRouteUsage";

    fn route_of(d: &Doc, id: &str) -> Option<String> {
        match d.asset.nodes[id].params.get(ROUTE_PARAM) {
            Some(Value::Str(s)) => Some(s.clone()),
            _ => None,
        }
    }

    fn count(d: &Doc, kind: &str) -> usize {
        d.asset.nodes.values().filter(|n| n.kind == kind).count()
    }

    /// Decision 0038: a declaration pasted into its own graph becomes a usage of the original, at the pasted place.
    #[test]
    fn pasted_route_declaration_becomes_its_usage() {
        let reg = shader_stdlib::registry();
        let mut d = doc();
        let decl = d.add_node(DECL, (100.0, 50.0));
        let clip = d.copy(&[decl.clone()].into(), &reg);
        let pasted = d.paste(&clip, (40.0, 20.0), &reg);
        assert_eq!(pasted.len(), 1);
        let id = pasted.first().unwrap();
        assert_eq!(d.asset.nodes[id].kind, USAGE);
        assert_eq!(route_of(&d, id), Some(decl.clone()));
        assert_eq!(d.asset.nodes[id].pos, (140.0, 70.0));
        assert_eq!(count(&d, DECL), 1, "no duplicate declaration");
        assert_eq!(d.selection, pasted);
        d.undo();
        assert_eq!(count(&d, USAGE), 0, "one undo step");
    }

    /// Wires leaving the declaration leave the usage; wires into it are dropped (a usage has no input).
    #[test]
    fn pasted_route_declaration_keeps_output_wires() {
        let reg = shader_stdlib::registry();
        let mut d = doc();
        let src = d.add_node("stdlib::Constant", (0.0, 0.0));
        let decl = d.add_node(DECL, (100.0, 0.0));
        let add = d.add_node("stdlib::Add", (200.0, 0.0));
        d.edit(|m| m.connect((&src, "out"), (&decl, "in")));
        d.edit(|m| m.connect((&decl, "out"), (&add, "a")));
        let clip = d.copy(&[src.clone(), decl.clone(), add.clone()].into(), &reg);
        let pasted = d.paste(&clip, (0.0, 300.0), &reg);
        let of = |kind: &str| pasted.iter().find(|id| d.asset.nodes[*id].kind == kind).unwrap().clone();
        let (usage, add2, src2) = (of(USAGE), of("stdlib::Add"), of("stdlib::Constant"));
        assert_eq!(route_of(&d, &usage), Some(decl.clone()));
        let edges: Vec<_> = d.asset.edges.iter().filter(|e| pasted.contains(&e.to.0)).cloned().collect();
        assert_eq!(edges.len(), 1, "{edges:?}");
        assert_eq!((edges[0].from.clone(), edges[0].to.clone()), ((usage.clone(), "out".into()), (add2, "a".into())));
        assert!(d.asset.edges.iter().all(|e| e.from.0 != src2), "the wire into the declaration is dropped");
    }

    /// A usage copied with its declaration keeps pointing at the original once the declaration pastes as a usage.
    #[test]
    fn pasted_route_with_usage_points_at_original() {
        let reg = shader_stdlib::registry();
        let mut d = doc();
        let decl = d.add_node(DECL, (0.0, 0.0));
        let usage = d.add_node_with(USAGE, (200.0, 0.0), &[(ROUTE_PARAM.into(), Value::Str(decl.clone()))]);
        let clip = d.copy(&[decl.clone(), usage].into(), &reg);
        let pasted = d.paste(&clip, (0.0, 100.0), &reg);
        assert_eq!(pasted.len(), 2);
        for id in &pasted {
            assert_eq!(d.asset.nodes[id].kind, USAGE);
            assert_eq!(route_of(&d, id), Some(decl.clone()));
        }
        assert_eq!(count(&d, DECL), 1);
    }

    /// Without the original declaration in the target graph a usage would dangle: the declaration is duplicated and
    /// a usage pasted with it follows the copy (decision 0011).
    #[test]
    fn pasted_route_declaration_duplicates_without_original() {
        let reg = shader_stdlib::registry();
        let mut d = doc();
        let decl = d.add_node(DECL, (0.0, 0.0));
        let usage = d.add_node_with(USAGE, (200.0, 0.0), &[(ROUTE_PARAM.into(), Value::Str(decl.clone()))]);
        let sel: BTreeSet<NodeId> = [decl.clone(), usage].into();
        let clip = d.copy(&sel, &reg);
        // Another graph.
        let mut other = Doc::new(AssetId::new(AssetType::Material, "other"), d.asset.clone());
        let pasted = other.paste(&clip, (0.0, 100.0), &reg);
        let new_decl = pasted.iter().find(|id| other.asset.nodes[*id].kind == DECL).expect("a duplicate").clone();
        let new_usage = pasted.iter().find(|id| other.asset.nodes[*id].kind == USAGE).unwrap();
        assert_eq!(route_of(&other, new_usage), Some(new_decl.clone()));
        // The same graph after the original was deleted.
        d.delete_nodes(&sel, &reg);
        let pasted = d.paste(&clip, (0.0, 100.0), &reg);
        assert_eq!(count(&d, DECL), 1, "duplicated");
        let new_decl = pasted.iter().find(|id| d.asset.nodes[*id].kind == DECL).unwrap();
        let new_usage = pasted.iter().find(|id| d.asset.nodes[*id].kind == USAGE).unwrap();
        assert_eq!(route_of(&d, new_usage).as_ref(), Some(new_decl));
        // A node with the original id that is no declaration any more does not count either (same asset id).
        let mut e = doc();
        e.edit(|m| {
            m.nodes.insert(decl.clone(), Node::new("stdlib::Add", (0.0, 0.0)));
        });
        let pasted = e.paste(&clip, (0.0, 100.0), &reg);
        assert!(pasted.iter().any(|id| e.asset.nodes[id].kind == DECL));
    }

    const MAKE: &str = "stdlib::MakeMaterialAttributesPbr";

    /// Decision 0023: attributes connect to Lerp `a`/`b` only; Add and If refuse them.
    #[test]
    fn attributes_connect_to_lerp_only() {
        let reg = shader_stdlib::registry();
        let mut d = doc();
        let make = d.add_node(MAKE, (0.0, 0.0));
        let lerp = d.add_node("stdlib::Lerp", (100.0, 0.0));
        let add = d.add_node("stdlib::Add", (100.0, 0.0));
        let iff = d.add_node("stdlib::If", (100.0, 0.0));
        let err =
            |d: &Doc, to: (&str, &str)| connection_error(&d.asset, (&make, "attributes"), to, &reg, &NoFunctions, "m");
        assert_eq!(err(&d, (&lerp, "a")), None);
        assert_eq!(err(&d, (&lerp, "b")), None);
        assert!(err(&d, (&lerp, "alpha")).unwrap().contains("E003"));
        assert!(err(&d, (&add, "a")).unwrap().contains("E003"));
        assert!(err(&d, (&iff, "gt")).unwrap().contains("E003"));
        assert!(err(&d, (&iff, "a")).unwrap().contains("E003"));
    }

    #[test]
    fn lerp_mixed_refused_both_orders() {
        let reg = shader_stdlib::registry();
        let mut d = doc();
        let make = d.add_node(MAKE, (0.0, 0.0));
        let v3 = d.add_node("stdlib::Constant3Vector", (0.0, 0.0));
        let lerp = d.add_node("stdlib::Lerp", (100.0, 0.0));
        d.edit(|m| m.connect((&make, "attributes"), (&lerp, "a")));
        let e = connection_error(&d.asset, (&v3, "out"), (&lerp, "b"), &reg, &NoFunctions, "m");
        assert!(e.unwrap().contains("E003"));
        d.edit(|m| m.disconnect_input((&lerp, "a")));
        d.edit(|m| m.connect((&v3, "out"), (&lerp, "a")));
        let e = connection_error(&d.asset, (&make, "attributes"), (&lerp, "b"), &reg, &NoFunctions, "m");
        assert!(e.unwrap().contains("E003"));
        // A Lerp of attributes feeds only attribute inputs.
        let make2 = d.add_node(MAKE, (0.0, 0.0));
        d.edit(|m| m.connect((&make2, "attributes"), (&lerp, "a")));
        let add = d.add_node("stdlib::Add", (200.0, 0.0));
        assert!(connection_error(&d.asset, (&lerp, "out"), (&add, "a"), &reg, &NoFunctions, "m").is_some());
    }

    #[test]
    fn lerp_reconnect_same_pin() {
        let reg = shader_stdlib::registry();
        let mut d = doc();
        let make = d.add_node(MAKE, (0.0, 0.0));
        let make2 = d.add_node(MAKE, (0.0, 0.0));
        let v3 = d.add_node("stdlib::Constant3Vector", (0.0, 0.0));
        let lerp = d.add_node("stdlib::Lerp", (100.0, 0.0));
        d.edit(|m| m.connect((&make, "attributes"), (&lerp, "a")));
        // The new edge replaces the old one, so `a` itself does not count.
        assert_eq!(connection_error(&d.asset, (&v3, "out"), (&lerp, "a"), &reg, &NoFunctions, "m"), None);
        assert_eq!(connection_error(&d.asset, (&make2, "attributes"), (&lerp, "b"), &reg, &NoFunctions, "m"), None);
    }

    /// Attributes pass through reroutes and named routes (spec goal of decision 0023).
    #[test]
    fn attributes_pass_through_routes() {
        use shader_graph_core::registry::ROUTE_PARAM;
        let reg = shader_stdlib::registry();
        let mut d = doc();
        let err = |d: &Doc, from: (&str, &str), to: (&str, &str)| {
            connection_error(&d.asset, from, to, &reg, &NoFunctions, "m")
        };
        let make = d.add_node(MAKE, (0.0, 0.0));
        let reroute = d.add_node("stdlib::Reroute", (100.0, 0.0));
        let lerp = d.add_node("stdlib::Lerp", (200.0, 0.0));
        assert_eq!(err(&d, (&make, "attributes"), (&reroute, "in")), None);
        d.edit(|m| m.connect((&make, "attributes"), (&reroute, "in")));
        assert_eq!(err(&d, (&reroute, "out"), (&lerp, "a")), None);
        let add = d.add_node("stdlib::Add", (200.0, 0.0));
        assert!(err(&d, (&reroute, "out"), (&add, "a")).unwrap().contains("E003"));

        let decl = d.add_node("stdlib::NamedRoute", (100.0, 200.0));
        let usage = d.add_node_with(
            "stdlib::NamedRouteUsage",
            (300.0, 200.0),
            &[(ROUTE_PARAM.into(), Value::Str(decl.clone()))],
        );
        assert_eq!(err(&d, (&make, "attributes"), (&decl, "in")), None);
        d.edit(|m| m.connect((&make, "attributes"), (&decl, "in")));
        assert_eq!(err(&d, (&usage, "out"), (&lerp, "b")), None);
        // Numbers still pass, textures still do not reach a numeric reroute.
        let v3 = d.add_node("stdlib::Constant3Vector", (0.0, 0.0));
        let reroute2 = d.add_node("stdlib::Reroute", (100.0, 0.0));
        assert_eq!(err(&d, (&v3, "out"), (&reroute2, "in")), None);
    }

    #[test]
    fn snap_rounds_to_ten() {
        assert_eq!(snap((14.0, 15.0)), (10.0, 20.0));
        assert_eq!(snap((-4.0, -15.0)), (0.0, -20.0));
        // Ties round away from zero (`f32::round`); `-0.0` becomes `0.0`.
        assert_eq!(snap((25.0, -25.0)), (30.0, -30.0));
        assert!(snap((-4.0, 0.0)).0.is_sign_positive());
        assert_eq!(snap((40.0, -10.0)), (40.0, -10.0));
        assert_eq!(tidy((39.999996, -1.4e-6)), (40.0, 0.0));
        assert_eq!(tidy((12.3, 9.99)), (12.3, 9.99), "off-grid positions are kept");
    }
}
