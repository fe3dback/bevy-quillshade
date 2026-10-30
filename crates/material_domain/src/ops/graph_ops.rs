use super::droppable::{DroppableFile, Dropped, ImageFile, droppable_for};
use super::*;
use crate::doc::snap;
use shader_graph_core::model::Edge;
use shader_stdlib::nodes::color::{GRADIENT_COLLECTION_PARAM, SAMPLE_GRADIENT_KIND};
use shader_stdlib::nodes::utility::{CURVE_PARAM, SAMPLE_CURVE_KIND};

/// Graph editing (spec §9): nodes, wires, previews, the palette and the node menus.
pub trait MaterialGraphOps {
    /// Adds a `kind` node at `pos` (snapped), connected to `link` when it has a compatible pin, in one undo step, and
    /// selects it. Returns its id (`None` without an active graph).
    fn add_node(
        &mut self,
        kind: &str,
        pos: (f32, f32),
        link: Option<Link>,
        params: &[(String, Value)],
    ) -> Option<NodeId>;

    /// A drop from a sidebar tree onto the active graph (design §4.3, §4.11). A game file is resolved through its
    /// [`DroppableFile`] (an image's Texture asset is created when there is none: on disk, outside the undo history);
    /// a Texture asset / built-in texture, a Curve asset or a gradient collection is the reference itself. Over a node
    /// with a param of that reference's kind the param is set (a collection resets the node's slot, as in
    /// a [`GraphProp::Param`] edit); otherwise (empty canvas or any other node) the droppable's node is added at
    /// `pos`. A Function asset always adds its call node, unless the call would be recursive (as in the palette).
    /// Either way one undo step.
    fn drop_item(&mut self, item: Dropped, pos: (f32, f32), node: Option<NodeId>);

    fn connect(&mut self, from: (NodeId, String), to: (NodeId, String));

    /// Connects `from` → `to` in the active graph when [`connection_error`] allows it, else logs "Connection refused:
    /// …" and changes nothing. With `into_last_step` the wire joins the last undo step (the node [`Self::add_node`]
    /// just added) instead of being a step of its own.
    fn connect_checked(&mut self, from: (NodeId, String), to: (NodeId, String), into_last_step: bool);

    /// Moves the wires of the pin `from` (an output when `output`) to the pin `to` of the same direction (Ctrl+LMB
    /// drag, decision 0032) as one undo step, or logs why not ([`Self::plan_move_wires`]).
    fn move_wires(&mut self, from: (NodeId, String), output: bool, to: (NodeId, String));

    /// The active graph after moving the wires of `from` onto `to`, or why the move is refused; changes nothing (a
    /// Ctrl+drag checks every pin it hovers with it). An input's single wire becomes `src → to`, replacing a wire
    /// already into `to`; every wire of an output becomes `to → dst`. All or nothing: `to` of the other direction, or
    /// any rewired wire the connection check refuses, is an error naming the first failing wire. Nothing to move
    /// (`from == to`, no wire at `from`, no graph tab) gives the asset unchanged.
    fn plan_move_wires(&self, from: &(NodeId, String), output: bool, to: &(NodeId, String)) -> Result<Asset, String>;

    /// Context menu of an input pin (RMB): type of polymorphic inputs while no poly pin is wired, reset of the
    /// inline value, or disconnect.
    fn pin_menu(&self, node: &str, pin: &str) -> Vec<(String, Action)>;

    /// Context menu of a node (RMB): its own actions, then the generic selection actions (decision 0021).
    /// A convert acts on every selected node offering it when `node` is selected, otherwise on `node` alone.
    fn node_menu(&self, node: &str) -> Vec<(String, Action)>;

    /// Actions of the selected nodes bound to `key` (decision 0021): selected nodes are grouped by the target
    /// kind of their convert, one [`MaterialAction::ConvertNodes`] per kind.
    fn hotkey_actions(&self, key: Hotkey) -> Vec<Action>;

    /// Why `node` of the active document cannot be previewed (`None`: it can). Only material nodes whose
    /// resolved signature gives a color-like output can (decision 0022); the MaterialOutput node has no outputs.
    fn preview_refusal(&self, node: &str) -> Option<String>;

    /// Sets the active material's previewed node to `node`, or clears it when it already is (one undo step).
    fn toggle_preview(&mut self, node: NodeId);

    /// The node's own context actions (none for an unknown kind). Functions have no parameters, so a convert to a
    /// parameter node ("Convert to Parameter", design §4.6) is not offered in them.
    fn node_actions(&self, node: &str) -> Vec<NodeAction>;

    /// Converts `nodes` in one undo step, keeping id, position, inline values and wires; wires and inline
    /// values of pins the new kind lacks are dropped (spec §2 of the QA design). See [`Conversion`] for the
    /// param mapping, unique name and output pin renames.
    fn convert_nodes(&mut self, nodes: &[NodeId], conv: &Conversion);

    /// What renaming `node` of the active graph edits (design §5.5): the `name` param of a parameter, a function's
    /// input / output and a Named Route declaration, the label of a Constant node (set or not); `None` for the rest.
    fn rename_target(&self, node: &str) -> Option<GraphProp>;

    /// What a double click on the header of `node` does (design §5.5), first that applies: opens the file it refers
    /// to ([`Self::open_action`]), renames it ([`Self::rename_target`]), edits its comment.
    fn header_action(&self, node: &str) -> Option<Action>;

    /// Opens the file `node` refers to in a tab ([`Self::referenced_asset`]): a double click on the node's body and
    /// its menu's "Open in tab". `None` when it refers to nothing.
    fn open_action(&self, node: &str) -> Option<Action>;

    /// The editor asset node `node` of the active graph refers to (decision 0023): a function call's function, a
    /// Sample Curve's curve, a `Global*` node's constant table, a texture node's Texture asset, a Sample Gradient's
    /// collection. `None` for user nodes, unset params and dangling targets. Drives node double-click and the props
    /// panel's "Open …" button.
    fn referenced_asset(&self, node: &str) -> Option<AssetId>;

    /// Rewrites every unconnected polymorphic input of `node` to `ty`: they resolve to one common type
    /// (the widest), so changing only one pin could not narrow it.
    fn set_pin_type(&mut self, node: &str, ty: PinType);

    /// Estimated node height for layout (title + one row per pin).
    fn node_height(&self, id: &str) -> f32;

    fn auto_layout(&mut self);

    /// Display data of a node pin, used by the canvas and the properties panel.
    fn pin_type(&self, node: &str, pin: &str, output: bool) -> Option<PinType>;

    fn is_output_node(&self, node: &str) -> bool;
}

impl MaterialGraphOps for Editor {
    fn add_node(
        &mut self,
        kind: &str,
        pos: (f32, f32),
        link: Option<Link>,
        params: &[(String, Value)],
    ) -> Option<NodeId> {
        let d = self.doc_mut()?;
        let asset_kind = d.asset.kind;
        let id = d.add_node_with(kind, snap(pos), params);
        if let Some(link) = link {
            let d = self.doc().expect("active graph");
            let path = d.id.path.clone();
            let ty = d.asset.nodes.get(&link.node).and_then(|_| {
                let sig = node_signature(
                    &d.asset,
                    &link.node,
                    &self.domains.material().registry,
                    &self.domains.material().functions,
                    &path,
                )?;
                if link.from_output { sig.output(&link.pin).map(|p| p.ty) } else { sig.input(&link.pin).map(|p| p.ty) }
            });
            let end = ty.map(|t| if link.from_output { WireEnd::FromOutput(t) } else { WireEnd::FromInput(t) });
            if let Some(pin) = end.and_then(|e| {
                search::compatible_pin(
                    &self.domains.material().registry,
                    &self.domains.material().functions,
                    asset_kind,
                    kind,
                    e,
                )
            }) {
                let (from, to) = if link.from_output {
                    ((link.node, link.pin), (id.clone(), pin))
                } else {
                    ((id.clone(), pin), (link.node, link.pin))
                };
                // One undo step for node and wire.
                self.connect_checked(from, to, true);
            }
        }
        let d = self.doc_mut().expect("active graph");
        d.set_selection([id.clone()]);
        d.selected_frame = None;
        self.ui.props += 1;
        Some(id)
    }

    fn drop_item(&mut self, item: Dropped, pos: (f32, f32), node: Option<NodeId>) {
        let Some(d) = self.doc() else { return };
        let (node_kind, param, param_kind, value) = match item {
            Dropped::GameFile(path) => {
                let Some(kind) = droppable_for(&path) else {
                    return self.push_msg(Level::Warning, format!("`{path}` cannot be dropped onto a graph"));
                };
                match kind.resolve(self, &path) {
                    Ok(value) => (kind.node_kind(), kind.param(), kind.param_kind(), value),
                    Err(e) => return self.io_error(&format!("Cannot drop `assets/{path}`"), e),
                }
            }
            Dropped::Texture(value) => (ImageFile.node_kind(), ImageFile.param(), ImageFile.param_kind(), value),
            Dropped::Curve(value) => (SAMPLE_CURVE_KIND, CURVE_PARAM, ParamKind::Curve, value),
            Dropped::Gradients(value) => {
                (SAMPLE_GRADIENT_KIND, GRADIENT_COLLECTION_PARAM, ParamKind::GradientCollection, value)
            }
            Dropped::Function(path) => {
                if search::is_self_call(&path, &d.id.path) {
                    let msg = format!("Function `{path}` cannot call itself (recursion)");
                    return self.push_msg(Level::Warning, msg);
                }
                self.add_node(&format!("{FUNCTION_PREFIX}{path}"), pos, None, &[]);
                return;
            }
        };
        let target = node.and_then(|node| {
            let n = self.doc()?.asset.nodes.get(&node)?;
            let def = self.domains.material().registry.get(&n.kind)?;
            let param = def.params().into_iter().find(|p| p.kind == param_kind)?;
            Some((node, param.id))
        });
        match target {
            Some((node, key)) => {
                let key = PropKey::new(GraphProp::Param { node: node.clone(), key });
                self.apply_tab(TabAction::SetValue { key, value: PropValue::Str(value), merge: false });
                if let Some(d) = self.doc_mut() {
                    d.set_selection([node]);
                    d.selected_frame = None;
                }
                self.ui.props += 1;
            }
            None => {
                self.add_node(node_kind, pos, None, &[(param.to_string(), Value::Str(value))]);
            }
        }
    }

    fn connect(&mut self, from: (NodeId, String), to: (NodeId, String)) {
        self.connect_checked(from, to, false);
    }

    fn connect_checked(&mut self, from: (NodeId, String), to: (NodeId, String), into_last_step: bool) {
        let Some(d) = self.doc() else { return };
        if let Some(e) = connection_error(
            &d.asset,
            (&from.0, &from.1),
            (&to.0, &to.1),
            &self.domains.material().registry,
            &self.domains.material().functions,
            &d.id.path,
        ) {
            self.push_msg(Level::Warning, format!("Connection refused: {e}"));
            return;
        }
        let Some(d) = self.doc_mut() else { return };
        let current = into_last_step.then(|| d.asset.clone());
        if current.is_some() {
            // Pops the last step (restores the model before it); the edit below records it again with the wire.
            let undone = d.undo();
            debug_assert!(undone, "no undo step to join the wire to");
        }
        d.edit(|a| {
            if let Some(current) = current {
                *a = current;
            }
            a.connect((&from.0, &from.1), (&to.0, &to.1));
        });
    }

    fn move_wires(&mut self, from: (NodeId, String), output: bool, to: (NodeId, String)) {
        if self.doc().is_none() {
            return;
        }
        match self.plan_move_wires(&from, output, &to) {
            Ok(moved) => {
                if let Some(d) = self.doc_mut()
                    && d.asset != moved
                {
                    d.edit(|a| *a = moved);
                }
            }
            Err(why) => self.push_msg(Level::Warning, why),
        }
    }

    fn plan_move_wires(&self, from: &(NodeId, String), output: bool, to: &(NodeId, String)) -> Result<Asset, String> {
        let Some(d) = self.doc() else { return Err("no graph is open".into()) };
        let mut moved = d.asset.clone();
        if from == to {
            return Ok(moved);
        }
        let path = &d.id.path;
        let label = |asset: &Asset, (node, pin): &(NodeId, String)| {
            format!(
                "{}.{pin}",
                crate::doc::node_title(
                    &self.domains.material().registry,
                    &self.domains.material().functions,
                    asset,
                    node
                )
            )
        };
        let sig =
            node_signature(&moved, &to.0, &self.domains.material().registry, &self.domains.material().functions, path);
        let fits = sig.is_some_and(|s| if output { s.output(&to.1).is_some() } else { s.input(&to.1).is_some() });
        if !fits {
            let dir = if output { "an output" } else { "an input" };
            return Err(format!("Wires not moved: {} is not {dir}", label(&moved, to)));
        }
        let at_pin = |e: &Edge| if output { e.from == *from } else { e.to == *from };
        let rewired: Vec<Edge> = moved
            .edges
            .iter()
            .filter(|e| at_pin(e))
            .map(|e| if output { Edge { from: to.clone(), ..e.clone() } } else { Edge { to: to.clone(), ..e.clone() } })
            .collect();
        if rewired.is_empty() {
            return Ok(moved);
        }
        // Checked one by one against the graph without the old wires — nor, for an input, the wire into `to` it
        // replaces — each accepted one added before the next.
        moved.edges.retain(|e| !at_pin(e) && (output || e.to != *to));
        for e in &rewired {
            let (f, t) = ((e.from.0.as_str(), e.from.1.as_str()), (e.to.0.as_str(), e.to.1.as_str()));
            if let Some(why) = connection_error(
                &moved,
                f,
                t,
                &self.domains.material().registry,
                &self.domains.material().functions,
                path,
            ) {
                return Err(format!("Wires not moved: {} → {}: {why}", label(&moved, &e.from), label(&moved, &e.to)));
            }
            moved.connect(f, t);
        }
        moved.normalize();
        Ok(moved)
    }

    fn pin_menu(&self, node: &str, pin: &str) -> Vec<(String, Action)> {
        let Some(d) = self.doc() else { return Vec::new() };
        let Some(sig) = node_signature(
            &d.asset,
            node,
            &self.domains.material().registry,
            &self.domains.material().functions,
            &d.id.path,
        ) else {
            return Vec::new();
        };
        let Some(p) = sig.input(pin) else { return Vec::new() };
        let wired = |id: &str| d.asset.edge_into(node, id).is_some();
        if wired(pin) {
            return vec![("Disconnect".into(), GraphEdit::Disconnect { to: (node.into(), pin.into()) }.into())];
        }
        let mut items = Vec::new();
        if p.poly && !sig.inputs.iter().any(|q| q.poly && wired(&q.id)) {
            for ty in [PinType::Float, PinType::Float2, PinType::Float3, PinType::Float4] {
                let label = if ty == p.ty { format!("{} (current)", ty.label()) } else { ty.label().to_string() };
                items.push((label, MaterialAction::SetPinType { node: node.into(), ty }.into()));
            }
        }
        if p.implicit.is_none() && !p.required {
            items.push((
                "Reset to default".into(),
                MaterialAction::ResetInput { node: node.into(), pin: pin.into() }.into(),
            ));
        }
        items
    }

    fn node_menu(&self, node: &str) -> Vec<(String, Action)> {
        let Some(d) = self.doc() else { return Vec::new() };
        let mut items = Vec::new();
        for a in self.node_actions(node) {
            let NodeOp::Convert(conv) = &a.op;
            let nodes = if d.selection.contains(node) {
                d.selection.iter().filter(|n| self.node_actions(n).iter().any(|b| b.op == a.op)).cloned().collect()
            } else {
                vec![node.to_string()]
            };
            let label = match a.hotkey {
                Some(h) => format!("{}\t{}", a.label, h.label()),
                None => a.label.clone(),
            };
            items.push((label, MaterialAction::ConvertNodes { nodes, conv: conv.clone() }.into()));
        }
        if !items.is_empty() {
            items.push((String::new(), Action::None));
        }
        if matches!(d.asset.kind, AssetKind::Material(_)) {
            // The previewed node can always be turned off, even when its output stopped converting to a color.
            let previewed = d.asset.preview.node.as_deref() == Some(node);
            items.push(match self.preview_refusal(node) {
                _ if previewed => ("Stop preview\tV".into(), MaterialAction::TogglePreview(node.into()).into()),
                None => ("Exclusive preview\tV".into(), MaterialAction::TogglePreview(node.into()).into()),
                Some(why) => ("Exclusive preview (not available)".into(), Action::Notify(why)),
            });
        }
        if let Some(open) = self.open_action(node) {
            items.push(("Open in tab".into(), open));
        }
        let has_comment = d.asset.nodes.get(node).is_some_and(|n| !n.comment.is_empty());
        let comment = if has_comment { "Edit comment" } else { "Add comment" };
        items.push((comment.into(), MaterialAction::TextDialog(GraphProp::NodeComment(node.into())).into()));
        items.extend([
            ("Copy\tCtrl+C".into(), Action::Tab(TabAction::Copy)),
            ("Duplicate\tCtrl+D".into(), Action::Tab(TabAction::Duplicate)),
            ("Delete\tDel".into(), Action::Tab(TabAction::DeleteSelection)),
        ]);
        items
    }

    fn hotkey_actions(&self, key: Hotkey) -> Vec<Action> {
        let Some(d) = self.doc() else { return Vec::new() };
        // `V` previews the one selected node, or stops previewing it (decision 0022).
        if key == Hotkey::plain(HotkeyKey::Letter('v')) {
            return match d.selection.iter().collect::<Vec<_>>()[..] {
                [n] if d.asset.preview.node.as_ref() == Some(n) || self.preview_refusal(n).is_none() => {
                    vec![MaterialAction::TogglePreview(n.clone()).into()]
                }
                _ => Vec::new(),
            };
        }
        let mut by_kind: BTreeMap<Conversion, Vec<NodeId>> = BTreeMap::new();
        for n in &d.selection {
            for a in self.node_actions(n).into_iter().filter(|a| a.hotkey == Some(key)) {
                let NodeOp::Convert(conv) = a.op;
                by_kind.entry(conv).or_default().push(n.clone());
            }
        }
        by_kind.into_iter().map(|(conv, nodes)| MaterialAction::ConvertNodes { nodes, conv }.into()).collect()
    }

    fn preview_refusal(&self, node: &str) -> Option<String> {
        let d = self.doc()?;
        if !matches!(d.asset.kind, AssetKind::Material(_)) {
            return Some("only material nodes can be previewed".into());
        }
        let Some(n) = d.asset.nodes.get(node) else { return Some(format!("node `{node}` does not exist")) };
        let sig = node_signature(
            &d.asset,
            node,
            &self.domains.material().registry,
            &self.domains.material().functions,
            &d.id.path,
        );
        let previewable =
            self.domains.material().registry.get(&n.kind).zip(sig).is_some_and(|(def, sig)| {
                def.role() != NodeRole::MaterialOutput && !def.preview_output(&sig).is_empty()
            });
        let title = crate::doc::node_title(
            &self.domains.material().registry,
            &self.domains.material().functions,
            &d.asset,
            node,
        );
        (!previewable).then(|| format!("`{title}` has no output that converts to a color"))
    }

    fn toggle_preview(&mut self, node: NodeId) {
        let Some(d) = self.doc() else { return };
        let on = d.asset.preview.node.as_ref() != Some(&node);
        if on && let Some(why) = self.preview_refusal(&node) {
            return self.push_msg(Level::Warning, why);
        }
        if let Some(d) = self.doc_mut() {
            d.edit(|a| a.preview.node = on.then_some(node));
        }
        self.ui.props += 1;
    }

    fn node_actions(&self, node: &str) -> Vec<NodeAction> {
        let Some(d) = self.doc() else { return Vec::new() };
        let Some(n) = d.asset.nodes.get(node) else { return Vec::new() };
        let mut actions =
            self.domains.material().registry.get(&n.kind).map(|def| def.context_actions()).unwrap_or_default();
        if d.asset.kind.is_function() {
            actions.retain(|a| {
                let NodeOp::Convert(c) = &a.op;
                self.domains.material().registry.get(&c.kind).is_none_or(|t| t.role() != NodeRole::Parameter)
            });
        }
        actions
    }

    fn convert_nodes(&mut self, nodes: &[NodeId], conv: &Conversion) {
        let Some(d) = self.doc() else { return };
        let mut asset = d.asset.clone();
        for id in nodes {
            // A unique name is chosen against the other parameter nodes (not this one, which is converted).
            let unique = conv.unique_name.as_ref().map(|(param, base)| {
                let taken: BTreeSet<String> = asset
                    .nodes
                    .iter()
                    .filter(|(k, n)| *k != id && n.kind.starts_with("param::"))
                    // Resolved: a parameter node keeping its default name stores no `name`.
                    .filter_map(|(_, n)| {
                        match self.domains.material().registry.resolved_params(&n.kind, &n.params).remove(param) {
                            Some(Value::Str(s)) => Some(s),
                            _ => None,
                        }
                    })
                    .collect();
                let name = std::iter::once(base.clone())
                    .chain((1..).map(|i| format!("{base}_{i}")))
                    .find(|c| !taken.contains(c.as_str()))
                    .unwrap_or_default();
                (param.clone(), Value::Str(name))
            });
            if let Some(n) = asset.nodes.get_mut(id) {
                n.kind = conv.kind.clone();
                if !conv.params.is_empty() {
                    let old = std::mem::take(&mut n.params);
                    n.params =
                        conv.params.iter().filter_map(|(f, t)| old.get(f).map(|v| (t.clone(), v.clone()))).collect();
                }
                n.params.extend(unique);
            }
            for e in asset.edges.iter_mut().filter(|e| e.from.0 == *id) {
                if let Some((_, new)) = conv.pins.iter().find(|(old, _)| *old == e.from.1) {
                    e.from.1 = new.clone();
                }
            }
        }
        for id in nodes {
            let Some(sig) = node_signature(
                &asset,
                id,
                &self.domains.material().registry,
                &self.domains.material().functions,
                &d.id.path,
            ) else {
                continue;
            };
            asset.edges.retain(|e| {
                (e.to.0 != *id || sig.input(&e.to.1).is_some()) && (e.from.0 != *id || sig.output(&e.from.1).is_some())
            });
            if let Some(n) = asset.nodes.get_mut(id) {
                n.inputs.retain(|pin, _| sig.input(pin).is_some());
            }
        }
        if let Some(d) = self.doc_mut() {
            d.edit(|a| *a = asset);
        }
        self.ui.props += 1;
    }

    fn rename_target(&self, node: &str) -> Option<GraphProp> {
        let n = self.doc()?.asset.nodes.get(node)?;
        if crate::doc::has_label(&n.kind) {
            return Some(GraphProp::NodeLabel(node.to_string()));
        }
        match self.domains.material().registry.get(&n.kind)?.role() {
            NodeRole::Parameter | NodeRole::FunctionInput | NodeRole::FunctionOutput | NodeRole::RouteDeclaration => {
                Some(GraphProp::Param { node: node.to_string(), key: "name".into() })
            }
            _ => None,
        }
    }

    fn header_action(&self, node: &str) -> Option<Action> {
        self.doc()?.asset.nodes.get(node)?;
        if let Some(open) = self.open_action(node) {
            return Some(open);
        }
        let target = self.rename_target(node).unwrap_or_else(|| GraphProp::NodeComment(node.to_string()));
        Some(MaterialAction::TextDialog(target).into())
    }

    fn open_action(&self, node: &str) -> Option<Action> {
        self.referenced_asset(node).map(|id| Action::OpenAsset(id.to_ref()))
    }

    fn referenced_asset(&self, node: &str) -> Option<AssetId> {
        let n = self.doc()?.asset.nodes.get(node)?;
        if let Some(path) = n.kind.strip_prefix(FUNCTION_PREFIX) {
            return self
                .domains
                .material()
                .functions
                .contains_key(path)
                .then(|| AssetId::new(AssetType::Function, path));
        }
        let def = self.domains.material().registry.get(&n.kind)?;
        let params = self.domains.material().registry.resolved_params(&n.kind, &n.params);
        for p in def.params() {
            let v = params.get(&p.id).map(Value::as_str).unwrap_or_default();
            match p.kind {
                ParamKind::Curve if self.curve_paths().iter().any(|c| c == v) => {
                    return Some(AssetId::new(AssetType::Curve, v));
                }
                ParamKind::GlobalConst(_) => {
                    if let Some((table, _)) = self.global_consts().remove(v) {
                        return Some(table);
                    }
                }
                ParamKind::GradientCollection => {
                    let id = AssetId::new(AssetType::Gradients, v);
                    if self.domains.material().disk.gradients.contains_key(&id) {
                        return Some(id);
                    }
                }
                ParamKind::Texture => {
                    let id = AssetId::new(AssetType::Texture, v);
                    // Built-in textures open read-only.
                    if self.domains.material().disk.textures.contains_key(&id)
                        || shader_stdlib::internal::internal_texture(v).is_some()
                    {
                        return Some(id);
                    }
                }
                _ => {}
            }
        }
        None
    }

    fn set_pin_type(&mut self, node: &str, ty: PinType) {
        let Some(d) = self.doc() else { return };
        let Some(n) = d.asset.nodes.get(node) else { return };
        let Some(sig) = node_signature(
            &d.asset,
            node,
            &self.domains.material().registry,
            &self.domains.material().functions,
            &d.id.path,
        ) else {
            return;
        };
        let values: Vec<(String, Value)> = sig
            .inputs
            .iter()
            .filter(|p| p.poly && d.asset.edge_into(node, &p.id).is_none())
            .map(|p| (p.id.clone(), n.inputs.get(&p.id).unwrap_or(&p.default).coerce(ty)))
            .collect();
        if values.is_empty() {
            return;
        }
        if let Some(d) = self.doc_mut() {
            d.edit(|a| {
                if let Some(n) = a.nodes.get_mut(node) {
                    n.inputs.extend(values);
                }
            });
        }
        self.ui.props += 1;
    }

    fn node_height(&self, id: &str) -> f32 {
        let Some(d) = self.doc() else { return 60.0 };
        let rows = node_signature(
            &d.asset,
            id,
            &self.domains.material().registry,
            &self.domains.material().functions,
            &d.id.path,
        )
        .map_or(1, |s| s.inputs.len() + s.outputs.len());
        40.0 + rows as f32 * 24.0
    }

    fn auto_layout(&mut self) {
        let Some(d) = self.doc() else { return };
        let pos = autolayout::layout(&d.asset, &d.selection, |n| self.node_height(n));
        if let Some(d) = self.doc_mut() {
            d.edit(|a| {
                for (id, p) in pos {
                    if let Some(n) = a.nodes.get_mut(&id) {
                        n.pos = p;
                    }
                }
            });
        }
        self.view_requests.push(ViewRequest::FitAll);
    }

    fn pin_type(&self, node: &str, pin: &str, output: bool) -> Option<PinType> {
        let d = self.doc()?;
        let sig = node_signature(
            &d.asset,
            node,
            &self.domains.material().registry,
            &self.domains.material().functions,
            &d.id.path,
        )?;
        if output { sig.output(pin).map(|p| p.ty) } else { sig.input(pin).map(|p| p.ty) }
    }

    fn is_output_node(&self, node: &str) -> bool {
        self.doc()
            .and_then(|d| d.asset.nodes.get(node))
            .and_then(|n| self.domains.material().registry.get(&n.kind))
            .is_some_and(|def| def.role() == NodeRole::MaterialOutput)
    }
}

/// The graph tab's edits (design §6.2): the body of [`TabDoc::apply`] / [`TabDoc::current_value`] of [`Doc`]. Steps
/// that need other tabs (function signatures, validation, the preview regeneration) follow in [`Editor::apply`].
impl Doc {
    pub(crate) fn apply_edit(&mut self, ctx: &mut DocCtx, action: &TabAction) -> Handled {
        let Some(m) = ctx.domain::<MaterialState>() else { return Handled::No };
        let redraw = match action {
            TabAction::Undo => self.undo(),
            TabAction::Redo => self.redo(),
            TabAction::Copy => {
                self.copy_to(m);
                false
            }
            TabAction::Cut => {
                self.copy_to(m);
                // The clipboard holds no frames: a cut keeps the enclosed ones (only Delete removes them, design §4.5).
                self.enclosed_frames.clear();
                self.delete_selected(&m.registry);
                true
            }
            TabAction::Paste(at) => self.paste_from(m, at.map(|p| (p.x, p.y))),
            TabAction::Duplicate => {
                let saved = m.clipboard.take();
                self.copy_to(m);
                let pasted = self.paste_from(m, None);
                m.clipboard = saved;
                pasted
            }
            TabAction::DeleteSelection => {
                self.delete_selected(&m.registry);
                true
            }
            TabAction::SelectAll => {
                let all: BTreeSet<NodeId> = self.asset.nodes.keys().cloned().collect();
                self.set_selection(all);
                self.selected_frame = None;
                self.revision += 1;
                true
            }
            TabAction::CommitInline => {
                self.end_merge();
                true
            }
            TabAction::SetValue { key, value, merge } => {
                let Some(target) = key.downcast::<GraphProp>() else { return Handled::No };
                let value = self.model_value_for(target, value, m);
                self.set_prop(m, key, target, value, *merge)
            }
            TabAction::SetComponent { key, comp, value, is_final } => {
                let Some(target) = key.downcast::<GraphProp>() else { return Handled::No };
                let Some(cur) = self.prop_of(target, m) else { return Handled::Yes };
                let new = match (&cur, target) {
                    // A frame without a hue gets one from its Hue field.
                    (Value::Str(_), GraphProp::FrameHue(_)) => Some(Value::Float(*value)),
                    _ => prop_value(&cur).with_component(*comp, *value).map(|v| self.model_value_for(target, &v, m)),
                };
                // `None`: a field sent its shown (rounded) text back on focus loss, which is not an edit.
                let Some(new) = new else { return Handled::Yes };
                let redraw = self.set_prop(m, key, target, new, true);
                if *is_final {
                    self.end_merge();
                }
                redraw
            }
            // The canvas's edits need the editor (rules, signatures, the canvas's move tracking, dialogs): the domain
            // applies them (`crate::graph_domain::graph_edit`).
            TabAction::Doc(p) if p.downcast::<GraphEdit>().is_some() => {
                ctx.deferred.push(p.clone());
                return Handled::Yes;
            }
            TabAction::Doc(_) => return Handled::No,
        };
        ctx.redraw_props |= redraw;
        Handled::Yes
    }

    /// The graph clipboard gets the selection (nothing when it is empty).
    fn copy_to(&self, m: &mut MaterialState) {
        let clip = self.copy(&self.selection, &m.registry);
        if !clip.nodes.is_empty() {
            m.clipboard = Some(clip);
            m.paste_count = 0;
        }
    }

    /// Pastes the graph clipboard at `at`, or offset from the last paste; `false` when the clipboard is empty.
    fn paste_from(&mut self, m: &mut MaterialState, at: Option<(f32, f32)>) -> bool {
        let Some(clip) = m.clipboard.clone() else { return false };
        m.paste_count += 1;
        let step = 40.0 * m.paste_count as f32;
        let ids: BTreeSet<NodeId> = BTreeSet::new();
        let tmp = Asset { nodes: clip.nodes.clone(), ..Asset::new("clip", AssetKind::Function) };
        let offset = match (at, bounds(&tmp, &ids)) {
            (Some(p), Some(b)) => (p.0 - b.0, p.1 - b.1),
            _ => (step, step),
        };
        // The first copied node lands on the grid (design §4.1); the others keep their places relative to it.
        let offset = clip.nodes.values().next().map_or(offset, |n| {
            let to = snap((n.pos.0 + offset.0, n.pos.1 + offset.1));
            (to.0 - n.pos.0, to.1 - n.pos.1)
        });
        self.paste(&clip, offset, &m.registry);
        true
    }

    /// Deletes the selected comment frame, or the selected nodes with the frames they enclose (one undo step).
    fn delete_selected(&mut self, registry: &Registry) {
        if let Some(f) = self.selected_frame.take() {
            self.edit(|a| a.comments.retain(|c| c.id != f));
        } else {
            // Frames enclosed by the selection go in the same step (design §4.5, one undo).
            let sel = self.selection.clone();
            let marked = std::mem::take(&mut self.enclosed_frames);
            let frames = self.enclosed(&marked, &sel);
            self.delete_nodes_and_frames(&sel, &frames, registry);
        }
    }

    /// `v` as the model value of `target`: a param holding a pin type gets the type back from its label.
    fn model_value_for(&self, target: &GraphProp, v: &PropValue, m: &MaterialState) -> Value {
        if let (GraphProp::Param { node, key }, PropValue::Str(s)) = (target, v)
            && let Some(n) = self.asset.nodes.get(node)
            && let Some(def) = m.registry.get(&n.kind)
            && def.params().iter().any(|p| p.id == *key && matches!(p.kind, ParamKind::PinType(_)))
            && let Some(t) = PinType::from_label(s)
        {
            return Value::Type(t);
        }
        model_value(v)
    }

    /// Value a property widget shows: the stored one, or the pin's default.
    pub(crate) fn prop_of(&self, target: &GraphProp, m: &MaterialState) -> Option<Value> {
        match target {
            GraphProp::OutputMode => match self.asset.kind {
                AssetKind::Material(MaterialKind::Pbr3d(s)) => Some(Value::Str(s.output.label().into())),
                _ => None,
            },
            GraphProp::Param { node, key } => {
                let n = self.asset.nodes.get(node)?;
                Some(m.registry.resolved_params(&n.kind, &n.params).get(key).cloned().unwrap_or(Value::Float(0.0)))
            }
            GraphProp::Input { node, pin } => {
                let n = self.asset.nodes.get(node)?;
                let pin_def = node_signature(&self.asset, node, &m.registry, &m.functions, &self.id.path)
                    .and_then(|sig| sig.input(pin).cloned());
                // In the pin's resolved type: a wire on another poly pin may widen a stored scalar.
                match (n.inputs.get(pin), pin_def) {
                    (Some(v), Some(p)) => Some(v.coerce(p.ty)),
                    (Some(v), None) => Some(v.clone()),
                    (None, p) => p.map(|p| p.default.coerce(p.ty)),
                }
            }
            GraphProp::Description => Some(Value::Str(self.asset.meta.description.clone())),
            GraphProp::PreviewMode => Some(Value::Str(preview_mode_label(self.asset.preview.mode).into())),
            GraphProp::NodeLabel(id) => self.asset.nodes.get(id).map(|n| Value::Str(n.label.clone())),
            GraphProp::NodeComment(id) => self.asset.nodes.get(id).map(|n| Value::Str(n.comment.clone())),
            GraphProp::FrameTitle(id) => {
                self.asset.comments.iter().find(|f| f.id == *id).map(|f| Value::Str(f.title.clone()))
            }
            GraphProp::FrameHue(id) => self
                .asset
                .comments
                .iter()
                .find(|f| f.id == *id)
                .map(|f| f.hue.map_or(Value::Str(String::new()), Value::Float)),
            GraphProp::Blend | GraphProp::TwoSided | GraphProp::Shading => None,
        }
    }

    /// Sets a property (merged into the last undo step of the same `key` while `merge`); returns whether the right
    /// sidebar must redraw (not during merged edits, which would take the widget being edited).
    fn set_prop(&mut self, m: &MaterialState, key: &PropKey, target: &GraphProp, value: Value, merge: bool) -> bool {
        if self.prop_of(target, m).as_ref() == Some(&value) {
            return false;
        }
        let mkey = merge.then(|| MergeKey::Prop(key.clone()));
        if let GraphProp::OutputMode = target {
            return PbrOutputMode::ALL
                .into_iter()
                .find(|mode| mode.label() == value.as_str())
                .is_some_and(|mode| self.set_pbr_output_mode(mode, m));
        }
        // Gradient design §6: slot ids of one collection mean nothing in another, so picking another collection resets
        // the node's slot (`0`: none) in the same undo step.
        let reset_slot = match target {
            GraphProp::Param { node, key: k } => self.slot_param_reset_by(node, k, &value, &m.registry),
            _ => None,
        };
        let mut refresh = !merge;
        match target {
            GraphProp::Param { node, key: k } => {
                self.edit_merge(mkey, |a| {
                    if let Some(n) = a.nodes.get_mut(node) {
                        n.params.insert(k.clone(), value.clone());
                        if let Some(slot) = &reset_slot {
                            n.params.insert(slot.clone(), Value::Int(0));
                        }
                    }
                });
            }
            GraphProp::Input { node, pin } => {
                self.edit_merge(mkey, |a| {
                    if let Some(n) = a.nodes.get_mut(node) {
                        n.inputs.insert(pin.clone(), value.clone());
                    }
                });
            }
            GraphProp::Blend => {
                let blend = match value.as_str() {
                    "Mask" => BlendMode::Mask,
                    "Blend" => BlendMode::Blend,
                    "Add" => BlendMode::Add,
                    "Multiply" => BlendMode::Multiply,
                    _ => BlendMode::Opaque,
                };
                self.edit(|a| {
                    if let AssetKind::Material(m) = &mut a.kind {
                        m.set_blend(blend);
                    }
                });
            }
            GraphProp::TwoSided => {
                self.edit(|a| match &mut a.kind {
                    AssetKind::Material(MaterialKind::Pbr3d(s)) => s.two_sided = value.as_bool(),
                    AssetKind::Material(MaterialKind::Unlit3d(s)) => s.two_sided = value.as_bool(),
                    _ => {}
                });
            }
            GraphProp::Shading => {
                let shading = if value.as_str() == "Unlit" { Shading::Unlit } else { Shading::Lit };
                self.edit(|a| {
                    if let AssetKind::Material(MaterialKind::Pbr3d(s)) = &mut a.kind {
                        s.shading = shading;
                    }
                });
            }
            GraphProp::Description => {
                self.edit_merge(mkey, |a| a.meta.description = value.as_str().to_string());
                refresh = false;
            }
            GraphProp::PreviewMode => {
                if let Some(mode) = PreviewMode::ALL.into_iter().find(|m| preview_mode_label(*m) == value.as_str()) {
                    self.edit(|a| a.preview.mode = mode);
                }
            }
            GraphProp::NodeLabel(id) => {
                self.edit_merge(mkey, |a| {
                    if let Some(n) = a.nodes.get_mut(id) {
                        n.label = value.as_str().to_string();
                    }
                });
                refresh = false;
            }
            GraphProp::NodeComment(id) => {
                let comment: String =
                    value.as_str().chars().take(shader_graph_core::model::Node::COMMENT_MAX).collect();
                self.edit_merge(mkey, |a| {
                    if let Some(n) = a.nodes.get_mut(id) {
                        n.comment = comment;
                    }
                });
                refresh = false;
            }
            GraphProp::FrameTitle(id) => {
                self.edit_merge(mkey, |a| {
                    if let Some(f) = a.comments.iter_mut().find(|f| f.id == *id) {
                        f.title = value.as_str().to_string();
                    }
                });
                refresh = false;
            }
            GraphProp::FrameHue(id) => {
                let hue = match &value {
                    Value::Float(h) => Some(h.clamp(0.0, 360.0)),
                    _ => None,
                };
                self.edit_merge(mkey, |a| {
                    if let Some(f) = a.comments.iter_mut().find(|f| f.id == *id) {
                        f.hue = hue;
                    }
                });
                // A Hue drag must not rebuild its own sidebar; "Default colour" moves the field back.
                refresh = hue.is_none();
            }
            GraphProp::OutputMode => {}
        }
        refresh
    }

    /// Switches the Pbr3d output layout (decision 0023) in one undo step; `false` on another kind of asset. Like
    /// [`Editor::convert_nodes`], wires and inline values of material output pins the new layout lacks are dropped;
    /// undo brings them back.
    fn set_pbr_output_mode(&mut self, mode: PbrOutputMode, m: &MaterialState) -> bool {
        let mut asset = self.asset.clone();
        let AssetKind::Material(MaterialKind::Pbr3d(s)) = &mut asset.kind else { return false };
        s.output = mode;
        let outputs: Vec<NodeId> = asset
            .nodes
            .iter()
            .filter(|(_, n)| m.registry.get(&n.kind).is_some_and(|def| def.role() == NodeRole::MaterialOutput))
            .map(|(id, _)| id.clone())
            .collect();
        for id in &outputs {
            let Some(sig) = node_signature(&asset, id, &m.registry, &m.functions, &self.id.path) else { continue };
            asset.edges.retain(|e| e.to.0 != *id || sig.input(&e.to.1).is_some());
            if let Some(n) = asset.nodes.get_mut(id) {
                n.inputs.retain(|pin, _| sig.input(pin).is_some());
            }
        }
        self.edit(|a| *a = asset);
        true
    }

    /// The `GradientSlot` param of `node` that setting its param `key` to `value` resets: `key` is a
    /// `GradientCollection` param, `value` another collection than the current one and a slot is picked.
    fn slot_param_reset_by(&self, node: &str, key: &str, value: &Value, registry: &Registry) -> Option<String> {
        let n = self.asset.nodes.get(node)?;
        let params = registry.get(&n.kind)?.params();
        if !params.iter().any(|p| p.id == key && p.kind == ParamKind::GradientCollection) {
            return None;
        }
        if registry.resolved_params(&n.kind, &n.params).get(key) == Some(value) {
            return None;
        }
        let slot = params.into_iter().find(|p| p.kind == ParamKind::GradientSlot)?;
        n.params.get(&slot.id).is_some_and(|v| *v != Value::Int(0)).then_some(slot.id)
    }
}
