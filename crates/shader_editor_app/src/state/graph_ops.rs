use super::*;
use crate::doc::snap;
use crate::droppable::{DroppableFile, Dropped, ImageFile, droppable_for};
use shader_graph_core::model::Edge;
use shader_stdlib::nodes::utility::{CURVE_PARAM, SAMPLE_CURVE_KIND};

impl Editor {
    pub(super) fn copy(&mut self) {
        match self.tab() {
            Some(Tab::Text(t)) => {
                self.copy_text = t.text.clone();
                return;
            }
            Some(Tab::Curve(c)) => {
                let points = c.selected_points();
                if !points.is_empty() {
                    self.curve_clipboard = Some(points);
                    self.paste_count = 0;
                }
                return;
            }
            _ => {}
        }
        let Some(d) = self.doc() else { return };
        let clip = d.copy(&d.selection, &self.registry);
        if !clip.nodes.is_empty() {
            self.clipboard = Some(clip);
            self.paste_count = 0;
        }
    }

    pub(super) fn paste(&mut self, at: Option<(f32, f32)>) {
        if self.curve_doc().is_some() {
            if let Some(points) = self.curve_clipboard.clone() {
                // Shifted in t: a copy on top of the original would be invisible but change the slopes.
                self.paste_count += 1;
                let dt = CURVE_PASTE_STEP * self.paste_count as f32;
                let points: Vec<(f32, f32)> = points.into_iter().map(|(t, v)| (t + dt, v)).collect();
                if let Some(c) = self.curve_doc_mut() {
                    c.insert_points(&points);
                }
                self.ui.props += 1;
            }
            return;
        }
        let Some(clip) = self.clipboard.clone() else { return };
        self.paste_count += 1;
        let step = 40.0 * self.paste_count as f32;
        let Some(d) = self.doc_mut() else { return };
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
        d.paste(&clip, offset);
        self.ui.props += 1;
    }

    pub(super) fn delete_selection(&mut self) {
        if let Some(c) = self.curve_doc_mut() {
            c.delete_selected();
            self.ui.props += 1;
            return;
        }
        let registry = &self.registry;
        let Some(d) = self.active.and_then(|i| self.tabs.get_mut(i)).and_then(Tab::graph_mut) else { return };
        if let Some(f) = d.selected_frame.take() {
            d.edit(|a| a.comments.retain(|c| c.id != f));
        } else {
            // Frames enclosed by the selection go in the same step (design §4.5, one undo).
            let sel = d.selection.clone();
            let marked = std::mem::take(&mut d.enclosed_frames);
            let frames = d.enclosed(&marked, &sel);
            d.delete_nodes_and_frames(&sel, &frames, registry);
        }
        self.ui.props += 1;
    }

    /// Adds a `kind` node at `pos` (snapped), connected to `link` when it has a compatible pin, in one undo step, and
    /// selects it. Returns its id (`None` without an active graph).
    pub(super) fn add_node(
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
                let sig = node_signature(&d.asset, &link.node, &self.registry, &self.functions, &path)?;
                if link.from_output { sig.output(&link.pin).map(|p| p.ty) } else { sig.input(&link.pin).map(|p| p.ty) }
            });
            let end = ty.map(|t| if link.from_output { WireEnd::FromOutput(t) } else { WireEnd::FromInput(t) });
            if let Some(pin) =
                end.and_then(|e| search::compatible_pin(&self.registry, &self.functions, asset_kind, kind, e))
            {
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

    /// A drop from a sidebar tree onto the active graph (design §4.3, §4.11). A game file is resolved through its
    /// [`DroppableFile`] (an image's Texture asset is created when there is none: on disk, outside the undo history);
    /// a Texture asset / built-in texture or a Curve asset is the reference itself. Over a node with a param of that
    /// reference's kind the param is set; otherwise (empty canvas or any other node) the droppable's node is added at
    /// `pos`. A Function asset always adds its call node, unless the call would be recursive (as in the palette).
    /// Either way one undo step.
    pub(super) fn drop_item(&mut self, item: Dropped, pos: (f32, f32), node: Option<NodeId>) {
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
            let def = self.registry.get(&n.kind)?;
            let param = def.params().into_iter().find(|p| p.kind == param_kind)?;
            Some((node, param.id))
        });
        match target {
            Some((node, key)) => {
                self.set_value(PropTarget::Param { node: node.clone(), key }, Value::Str(value), false);
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

    pub(super) fn connect(&mut self, from: (NodeId, String), to: (NodeId, String)) {
        self.connect_checked(from, to, false);
    }

    /// Connects `from` → `to` in the active graph when [`connection_error`] allows it, else logs "Connection refused:
    /// …" and changes nothing. With `into_last_step` the wire joins the last undo step (the node [`Self::add_node`]
    /// just added) instead of being a step of its own.
    fn connect_checked(&mut self, from: (NodeId, String), to: (NodeId, String), into_last_step: bool) {
        let Some(d) = self.doc() else { return };
        if let Some(e) =
            connection_error(&d.asset, (&from.0, &from.1), (&to.0, &to.1), &self.registry, &self.functions, &d.id.path)
        {
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

    /// Moves the wires of the pin `from` (an output when `output`) to the pin `to` of the same direction (Ctrl+LMB
    /// drag, decision 0032) as one undo step, or logs why not ([`Self::plan_move_wires`]).
    pub(super) fn move_wires(&mut self, from: (NodeId, String), output: bool, to: (NodeId, String)) {
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

    /// The active graph after moving the wires of `from` onto `to`, or why the move is refused; changes nothing (a
    /// Ctrl+drag checks every pin it hovers with it). An input's single wire becomes `src → to`, replacing a wire
    /// already into `to`; every wire of an output becomes `to → dst`. All or nothing: `to` of the other direction, or
    /// any rewired wire the connection check refuses, is an error naming the first failing wire. Nothing to move
    /// (`from == to`, no wire at `from`, no graph tab) gives the asset unchanged.
    pub fn plan_move_wires(
        &self,
        from: &(NodeId, String),
        output: bool,
        to: &(NodeId, String),
    ) -> Result<Asset, String> {
        let Some(d) = self.doc() else { return Err("no graph is open".into()) };
        let mut moved = d.asset.clone();
        if from == to {
            return Ok(moved);
        }
        let path = &d.id.path;
        let label = |asset: &Asset, (node, pin): &(NodeId, String)| {
            format!("{}.{pin}", crate::canvas::node_title(self, asset, node))
        };
        let sig = node_signature(&moved, &to.0, &self.registry, &self.functions, path);
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
            if let Some(why) = connection_error(&moved, f, t, &self.registry, &self.functions, path) {
                return Err(format!("Wires not moved: {} → {}: {why}", label(&moved, &e.from), label(&moved, &e.to)));
            }
            moved.connect(f, t);
        }
        moved.normalize();
        Ok(moved)
    }

    /// Value a property widget shows: the stored one, or the pin's default.
    pub fn current_value(&self, target: &PropTarget) -> Option<Value> {
        match target {
            PropTarget::CurvePoint(i) => {
                let (t, v) = *self.curve_doc()?.curve.points.get(*i)?;
                return Some(Value::Float2([t, v]));
            }
            PropTarget::CurvePreview => return Some(Value::Str(self.curve_doc()?.preview.label().into())),
            PropTarget::Const(name) => return Some(self.consts_doc()?.table.get(name)?.value.clone()),
            PropTarget::ConstType(name) => {
                return Some(Value::Str(self.consts_doc()?.table.get(name)?.ty.label().into()));
            }
            PropTarget::ConstDraftName => return Some(Value::Str(self.consts_doc()?.draft_name.clone())),
            PropTarget::ConstDraftType => return Some(Value::Str(self.consts_doc()?.draft_ty.label().into())),
            PropTarget::Texture(f) => return Some(f.get(&self.texture_doc()?.asset.settings)),
            PropTarget::PreviewTiled => return Some(Value::Bool(self.tab()?.preview()?.tiled())),
            _ => {}
        }
        let d = self.doc()?;
        match target {
            PropTarget::OutputMode => match d.asset.kind {
                AssetKind::Material(MaterialKind::Pbr3d(s)) => Some(Value::Str(s.output.label().into())),
                _ => None,
            },
            PropTarget::Param { node, key } => {
                let n = d.asset.nodes.get(node)?;
                Some(self.registry.resolved_params(&n.kind, &n.params).get(key).cloned().unwrap_or(Value::Float(0.0)))
            }
            PropTarget::Input { node, pin } => {
                let n = d.asset.nodes.get(node)?;
                let pin_def = node_signature(&d.asset, node, &self.registry, &self.functions, &d.id.path)
                    .and_then(|sig| sig.input(pin).cloned());
                // In the pin's resolved type: a wire on another poly pin may widen a stored scalar.
                match (n.inputs.get(pin), pin_def) {
                    (Some(v), Some(p)) => Some(v.coerce(p.ty)),
                    (Some(v), None) => Some(v.clone()),
                    (None, p) => p.map(|p| p.default.coerce(p.ty)),
                }
            }
            PropTarget::Description => Some(Value::Str(d.asset.meta.description.clone())),
            PropTarget::PreviewMode => Some(Value::Str(preview_mode_label(d.asset.preview.mode).into())),
            PropTarget::NodeLabel(id) => d.asset.nodes.get(id).map(|n| Value::Str(n.label.clone())),
            PropTarget::FrameTitle(id) => {
                d.asset.comments.iter().find(|f| f.id == *id).map(|f| Value::Str(f.title.clone()))
            }
            PropTarget::FrameHue(id) => d
                .asset
                .comments
                .iter()
                .find(|f| f.id == *id)
                .map(|f| f.hue.map_or(Value::Str(String::new()), Value::Float)),
            _ => None,
        }
    }

    pub(super) fn set_component(&mut self, target: PropTarget, comp: usize, value: f32, is_final: bool) {
        let Some(cur) = self.current_value(&target) else { return };
        // A field sends its shown (rounded) text back on focus loss: that is not an edit.
        let old = cur.floats().get(comp).copied();
        if old.is_some_and(|old| old != value && display_value(old) == value) {
            return;
        }
        let new = match cur {
            Value::Float(_) => Value::Float(value),
            Value::Int(_) => Value::Int(value.round() as i32),
            Value::Float2(mut v) => {
                if comp < 2 {
                    v[comp] = value;
                }
                Value::Float2(v)
            }
            Value::Float3(mut v) => {
                if comp < 3 {
                    v[comp] = value;
                }
                Value::Float3(v)
            }
            Value::Float4(mut v) => {
                if comp < 4 {
                    v[comp] = value;
                }
                Value::Float4(v)
            }
            Value::Color(mut v) => {
                if comp < 4 {
                    v[comp] = value;
                }
                Value::Color(v)
            }
            // A frame without a hue gets one from its Hue field.
            Value::Str(_) if matches!(target, PropTarget::FrameHue(_)) => Value::Float(value),
            other => other,
        };
        self.set_value(target, new, true);
        if is_final && let Some(d) = self.doc_mut() {
            d.end_merge();
        }
        if is_final && let Some(c) = self.curve_doc_mut() {
            c.end_merge();
        }
        if is_final && let Some(c) = self.consts_doc_mut() {
            c.end_merge();
        }
        if is_final && let Some(t) = self.texture_doc_mut() {
            t.end_merge();
        }
    }

    /// Context menu of an input pin (RMB): type of polymorphic inputs while no poly pin is wired, reset of the
    /// inline value, or disconnect.
    pub fn pin_menu(&self, node: &str, pin: &str) -> Vec<(String, Action)> {
        let Some(d) = self.doc() else { return Vec::new() };
        let Some(sig) = node_signature(&d.asset, node, &self.registry, &self.functions, &d.id.path) else {
            return Vec::new();
        };
        let Some(p) = sig.input(pin) else { return Vec::new() };
        let wired = |id: &str| d.asset.edge_into(node, id).is_some();
        if wired(pin) {
            return vec![("Disconnect".into(), Action::Disconnect { to: (node.into(), pin.into()) })];
        }
        let mut items = Vec::new();
        if p.poly && !sig.inputs.iter().any(|q| q.poly && wired(&q.id)) {
            for ty in [PinType::Float, PinType::Float2, PinType::Float3, PinType::Float4] {
                let label = if ty == p.ty { format!("{} (current)", ty.label()) } else { ty.label().to_string() };
                items.push((label, Action::SetPinType { node: node.into(), ty }));
            }
        }
        if p.implicit.is_none() && !p.required {
            items.push(("Reset to default".into(), Action::ResetInput { node: node.into(), pin: pin.into() }));
        }
        items
    }

    /// Context menu of a node (RMB): its own actions, then the generic selection actions (decision 0021).
    /// A convert acts on every selected node offering it when `node` is selected, otherwise on `node` alone.
    pub fn node_menu(&self, node: &str) -> Vec<(String, Action)> {
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
            items.push((label, Action::ConvertNodes { nodes, conv: conv.clone() }));
        }
        if !items.is_empty() {
            items.push((String::new(), Action::None));
        }
        if matches!(d.asset.kind, AssetKind::Material(_)) {
            // The previewed node can always be turned off, even when its output stopped converting to a color.
            let previewed = d.asset.preview.node.as_deref() == Some(node);
            items.push(match self.preview_refusal(node) {
                _ if previewed => ("Stop preview\tV".into(), Action::TogglePreview(node.into())),
                None => ("Exclusive preview\tV".into(), Action::TogglePreview(node.into())),
                Some(why) => ("Exclusive preview (not available)".into(), Action::Notify(why)),
            });
        }
        items.extend([
            ("Copy\tCtrl+C".into(), Action::Copy),
            ("Duplicate\tCtrl+D".into(), Action::Duplicate),
            ("Delete\tDel".into(), Action::DeleteSelection),
        ]);
        items
    }

    /// Actions of the selected nodes bound to `key` (decision 0021): selected nodes are grouped by the target
    /// kind of their convert, one [`Action::ConvertNodes`] per kind.
    pub fn hotkey_actions(&self, key: Hotkey) -> Vec<Action> {
        let Some(d) = self.doc() else { return Vec::new() };
        // `V` previews the one selected node, or stops previewing it (decision 0022).
        if key == Hotkey::plain(HotkeyKey::Letter('v')) {
            return match d.selection.iter().collect::<Vec<_>>()[..] {
                [n] if d.asset.preview.node.as_ref() == Some(n) || self.preview_refusal(n).is_none() => {
                    vec![Action::TogglePreview(n.clone())]
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
        by_kind.into_iter().map(|(conv, nodes)| Action::ConvertNodes { nodes, conv }).collect()
    }

    /// Why `node` of the active document cannot be previewed (`None`: it can). Only material nodes whose
    /// resolved signature gives a color-like output can (decision 0022); the MaterialOutput node has no outputs.
    pub(super) fn preview_refusal(&self, node: &str) -> Option<String> {
        let d = self.doc()?;
        if !matches!(d.asset.kind, AssetKind::Material(_)) {
            return Some("only material nodes can be previewed".into());
        }
        let Some(n) = d.asset.nodes.get(node) else { return Some(format!("node `{node}` does not exist")) };
        let sig = node_signature(&d.asset, node, &self.registry, &self.functions, &d.id.path);
        let previewable =
            self.registry.get(&n.kind).zip(sig).is_some_and(|(def, sig)| {
                def.role() != NodeRole::MaterialOutput && !def.preview_output(&sig).is_empty()
            });
        let title = crate::canvas::node_title(self, &d.asset, node);
        (!previewable).then(|| format!("`{title}` has no output that converts to a color"))
    }

    /// Sets the active material's previewed node to `node`, or clears it when it already is (one undo step).
    pub(super) fn toggle_preview(&mut self, node: NodeId) {
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

    /// The node's own context actions (none for an unknown kind). Functions have no parameters, so a convert to a
    /// parameter node ("Convert to Parameter", design §4.6) is not offered in them.
    pub(super) fn node_actions(&self, node: &str) -> Vec<NodeAction> {
        let Some(d) = self.doc() else { return Vec::new() };
        let Some(n) = d.asset.nodes.get(node) else { return Vec::new() };
        let mut actions = self.registry.get(&n.kind).map(|def| def.context_actions()).unwrap_or_default();
        if d.asset.kind.is_function() {
            actions.retain(|a| {
                let NodeOp::Convert(c) = &a.op;
                self.registry.get(&c.kind).is_none_or(|t| t.role() != NodeRole::Parameter)
            });
        }
        actions
    }

    /// Converts `nodes` in one undo step, keeping id, position, inline values and wires; wires and inline
    /// values of pins the new kind lacks are dropped (spec §2 of the QA design). See [`Conversion`] for the
    /// param mapping, unique name and output pin renames.
    pub(super) fn convert_nodes(&mut self, nodes: &[NodeId], conv: &Conversion) {
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
                    .filter_map(|(_, n)| match self.registry.resolved_params(&n.kind, &n.params).remove(param) {
                        Some(Value::Str(s)) => Some(s),
                        _ => None,
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
            let Some(sig) = node_signature(&asset, id, &self.registry, &self.functions, &d.id.path) else { continue };
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

    /// What renaming `node` of the active graph edits (design §5.5): the `name` param of a parameter, a function's
    /// input / output and a Named Route declaration, the label of a Constant node (set or not); `None` for the rest.
    pub fn rename_target(&self, node: &str) -> Option<PropTarget> {
        let n = self.doc()?.asset.nodes.get(node)?;
        if crate::canvas::has_label(&n.kind) {
            return Some(PropTarget::NodeLabel(node.to_string()));
        }
        match self.registry.get(&n.kind)?.role() {
            NodeRole::Parameter | NodeRole::FunctionInput | NodeRole::FunctionOutput | NodeRole::RouteDeclaration => {
                Some(PropTarget::Param { node: node.to_string(), key: "name".into() })
            }
            _ => None,
        }
    }

    /// What a double click on the header of `node` does (design §5.5): renames it ([`Self::rename_target`]), else
    /// opens the file it refers to ([`Self::referenced_asset`]), else nothing.
    pub fn header_action(&self, node: &str) -> Option<Action> {
        match self.rename_target(node) {
            Some(target) => Some(Action::TextDialog(target)),
            None => self.referenced_asset(node).map(Action::OpenAsset),
        }
    }

    /// The editor asset node `node` of the active graph refers to (decision 0023): a function call's function, a
    /// Sample Curve's curve, a `Global*` node's constant table, a texture node's Texture asset. `None` for user nodes,
    /// unset params and dangling targets. Drives node double-click and the props panel's "Open …" button.
    pub fn referenced_asset(&self, node: &str) -> Option<AssetId> {
        let n = self.doc()?.asset.nodes.get(node)?;
        if let Some(path) = n.kind.strip_prefix(FUNCTION_PREFIX) {
            return self.functions.contains_key(path).then(|| AssetId::new(AssetType::Function, path));
        }
        let def = self.registry.get(&n.kind)?;
        let params = self.registry.resolved_params(&n.kind, &n.params);
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
                ParamKind::Texture => {
                    let id = AssetId::new(AssetType::Texture, v);
                    // Built-in textures open read-only.
                    if self.disk.textures.contains_key(&id) || shader_stdlib::internal::internal_texture(v).is_some() {
                        return Some(id);
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// Switches the Pbr3d output layout (decision 0023) in one undo step. Like [`Self::convert_nodes`], wires and
    /// inline values of material output pins the new layout lacks are dropped; undo brings them back.
    pub(super) fn set_pbr_output_mode(&mut self, mode: PbrOutputMode) {
        let Some(d) = self.doc() else { return };
        let mut asset = d.asset.clone();
        let AssetKind::Material(MaterialKind::Pbr3d(s)) = &mut asset.kind else { return };
        s.output = mode;
        let outputs: Vec<NodeId> = asset
            .nodes
            .iter()
            .filter(|(_, n)| self.registry.get(&n.kind).is_some_and(|def| def.role() == NodeRole::MaterialOutput))
            .map(|(id, _)| id.clone())
            .collect();
        for id in &outputs {
            let Some(sig) = node_signature(&asset, id, &self.registry, &self.functions, &d.id.path) else { continue };
            asset.edges.retain(|e| e.to.0 != *id || sig.input(&e.to.1).is_some());
            if let Some(n) = asset.nodes.get_mut(id) {
                n.inputs.retain(|pin, _| sig.input(pin).is_some());
            }
        }
        if let Some(d) = self.doc_mut() {
            d.edit(|a| *a = asset);
        }
        self.ui.props += 1;
    }

    /// Rewrites every unconnected polymorphic input of `node` to `ty`: they resolve to one common type
    /// (the widest), so changing only one pin could not narrow it.
    pub(super) fn set_pin_type(&mut self, node: &str, ty: PinType) {
        let Some(d) = self.doc() else { return };
        let Some(n) = d.asset.nodes.get(node) else { return };
        let Some(sig) = node_signature(&d.asset, node, &self.registry, &self.functions, &d.id.path) else { return };
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

    pub(super) fn set_value(&mut self, target: PropTarget, value: Value, merge: bool) {
        if self.current_value(&target).as_ref() == Some(&value) {
            return;
        }
        let key = format!("{target:?}");
        let key = merge.then_some(key.as_str());
        match &target {
            PropTarget::CurvePoint(i) => {
                if let Some(c) = self.curve_doc_mut() {
                    let mut points = c.curve.points.clone();
                    let Some(p) = points.get_mut(*i) else { return };
                    let f = value.floats();
                    *p = (f[0], f.get(1).copied().unwrap_or(p.1));
                    // The point stays selected wherever the re-sort puts it.
                    let (points, selection) = CurveDoc::moved(&points, &[*i].into(), (0.0, 0.0));
                    let moved = selection != BTreeSet::from([*i]);
                    c.set_points(points, selection, key);
                    if moved {
                        // The fields are bound to the old index: rebind them, and the next edit is a new step.
                        c.end_merge();
                        self.ui.props += 1;
                    }
                }
                if !merge {
                    self.ui.props += 1;
                }
                return;
            }
            PropTarget::CurvePreview => {
                if let (Some(c), Some(interp)) = (self.curve_doc_mut(), Interp::from_label(value.as_str())) {
                    c.set_preview(interp);
                }
                self.ui.props += 1;
                return;
            }
            PropTarget::Const(name) => {
                if let Some(c) = self.consts_doc_mut() {
                    c.set_value(name, value, merge);
                }
                return;
            }
            PropTarget::ConstType(name) => {
                if let Some(ty) = ConstType::from_label(value.as_str()) {
                    self.consts_action(ConstsAction::SetType { name: name.clone(), ty, confirmed: false });
                }
                return;
            }
            PropTarget::ConstDraftName => {
                if let Some(c) = self.consts_doc_mut() {
                    c.draft_name = value.as_str().to_string();
                }
                return;
            }
            PropTarget::ConstDraftType => {
                if let (Some(c), Some(ty)) = (self.consts_doc_mut(), ConstType::from_label(value.as_str())) {
                    c.draft_ty = ty;
                }
                return;
            }
            PropTarget::OutputMode => {
                if let Some(mode) = PbrOutputMode::ALL.into_iter().find(|m| m.label() == value.as_str()) {
                    self.set_pbr_output_mode(mode);
                }
                return;
            }
            PropTarget::Texture(f) => {
                let Some(t) = self.texture_doc() else { return };
                let settings = f.set(&t.asset.settings, &value);
                match key {
                    // A dragged number field: one step, the form is kept.
                    Some(key) => {
                        if let Some(t) = self.texture_doc_mut() {
                            t.set_settings(settings, Some(key));
                        }
                    }
                    None => self.texture_action(TextureAction::SetSettings(settings)),
                }
                return;
            }
            PropTarget::PreviewTiled => {
                if let Some(p) = self.preview_doc_mut() {
                    p.set_tiled(value.as_bool());
                }
                return;
            }
            _ => {}
        }
        let Some(d) = self.doc_mut() else { return };
        let mut refresh = !merge;
        match &target {
            PropTarget::None => {}
            PropTarget::Param { node, key: k } => {
                d.edit_merge(key, |a| {
                    if let Some(n) = a.nodes.get_mut(node) {
                        n.params.insert(k.clone(), value.clone());
                    }
                });
            }
            PropTarget::Input { node, pin } => {
                d.edit_merge(key, |a| {
                    if let Some(n) = a.nodes.get_mut(node) {
                        n.inputs.insert(pin.clone(), value.clone());
                    }
                });
            }
            PropTarget::Blend => {
                let blend = match value.as_str() {
                    "Mask" => BlendMode::Mask,
                    "Blend" => BlendMode::Blend,
                    "Add" => BlendMode::Add,
                    "Multiply" => BlendMode::Multiply,
                    _ => BlendMode::Opaque,
                };
                d.edit(|a| {
                    if let AssetKind::Material(m) = &mut a.kind {
                        m.set_blend(blend);
                    }
                });
            }
            PropTarget::TwoSided => {
                d.edit(|a| match &mut a.kind {
                    AssetKind::Material(MaterialKind::Pbr3d(s)) => s.two_sided = value.as_bool(),
                    AssetKind::Material(MaterialKind::Unlit3d(s)) => s.two_sided = value.as_bool(),
                    _ => {}
                });
            }
            PropTarget::Shading => {
                let shading = if value.as_str() == "Unlit" { Shading::Unlit } else { Shading::Lit };
                d.edit(|a| {
                    if let AssetKind::Material(MaterialKind::Pbr3d(s)) = &mut a.kind {
                        s.shading = shading;
                    }
                });
            }
            PropTarget::Description => {
                d.edit_merge(key, |a| a.meta.description = value.as_str().to_string());
                refresh = false;
            }
            PropTarget::PreviewMode => {
                if let Some(mode) = PreviewMode::ALL.into_iter().find(|m| preview_mode_label(*m) == value.as_str()) {
                    d.edit(|a| a.preview.mode = mode);
                }
            }
            PropTarget::NodeLabel(id) => {
                d.edit_merge(key, |a| {
                    if let Some(n) = a.nodes.get_mut(id) {
                        n.label = value.as_str().to_string();
                    }
                });
                refresh = false;
            }
            PropTarget::FrameTitle(id) => {
                d.edit_merge(key, |a| {
                    if let Some(f) = a.comments.iter_mut().find(|f| f.id == *id) {
                        f.title = value.as_str().to_string();
                    }
                });
                refresh = false;
            }
            PropTarget::FrameHue(id) => {
                let hue = match &value {
                    Value::Float(h) => Some(h.clamp(0.0, 360.0)),
                    _ => None,
                };
                d.edit_merge(key, |a| {
                    if let Some(f) = a.comments.iter_mut().find(|f| f.id == *id) {
                        f.hue = hue;
                    }
                });
                // A Hue drag must not rebuild its own sidebar; "Default colour" moves the field back.
                refresh = hue.is_none();
            }
            PropTarget::CurvePoint(_)
            | PropTarget::OutputMode
            | PropTarget::CurvePreview
            | PropTarget::Const(_)
            | PropTarget::ConstType(_)
            | PropTarget::ConstDraftName
            | PropTarget::ConstDraftType
            | PropTarget::Texture(_)
            | PropTarget::PreviewTiled => {}
        }
        if refresh {
            self.ui.props += 1;
        }
    }

    /// Estimated node height for layout (title + one row per pin).
    pub fn node_height(&self, id: &str) -> f32 {
        let Some(d) = self.doc() else { return 60.0 };
        let rows = node_signature(&d.asset, id, &self.registry, &self.functions, &d.id.path)
            .map_or(1, |s| s.inputs.len() + s.outputs.len());
        40.0 + rows as f32 * 24.0
    }

    pub(super) fn auto_layout(&mut self) {
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

    /// Display data of a node pin, used by the canvas and the properties panel.
    pub fn pin_type(&self, node: &str, pin: &str, output: bool) -> Option<PinType> {
        let d = self.doc()?;
        let sig = node_signature(&d.asset, node, &self.registry, &self.functions, &d.id.path)?;
        if output { sig.output(pin).map(|p| p.ty) } else { sig.input(pin).map(|p| p.ty) }
    }

    pub fn is_output_node(&self, node: &str) -> bool {
        self.doc()
            .and_then(|d| d.asset.nodes.get(node))
            .and_then(|n| self.registry.get(&n.kind))
            .is_some_and(|def| def.role() == NodeRole::MaterialOutput)
    }
}
