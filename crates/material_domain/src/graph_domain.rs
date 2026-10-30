//! Material's graph behind `node_graph_base` (design §3.3, §7.3): [`MaterialGraph`] maps the base's calls onto the
//! material / function document ([`Doc`]) and the `Editor`'s Material methods, and [`graph_edit`] applies the base's
//! [`GraphEdit`]s to the model. Material keeps its pin colours, categories and titles, signatures and connection
//! rules, Named Routes, the preview (`V`) and Convert hotkey data, inline widgets, the four node preview kinds,
//! palette entries, tree drops, the node / asset subpanes and the editor topbar's dynamic part.

use std::sync::Arc;

use bevy::prelude::*;
use editor_core::action::Action;
use editor_core::dynval::PropKey;
use editor_core::main_input::{hotkey, plain};
use editor_core::menu::MenuEntry;
use editor_core::tab::ViewKindId;
use node_graph_base::{
    DiagMark, DropItem, Emphasis, FrameField, FrameView, GraphCtxRef, GraphDomain, GraphEdit, GraphHotkey, GraphView,
    MovePlan, NodeLook, NodeShape, NodeView, PaletteEntry, PinKey, PinLook, PinRef, PlaceSpec, PreviewRow,
};
use shader_graph_core::model::{Asset, AssetKind};
use shader_graph_core::registry::NodeRole;
use shader_graph_core::{Code, Hotkey, HotkeyKey, PinType, Value};

use crate::canvas::{header_color, pin_color};
use crate::doc::{Doc, connection_error, node_signature, node_title_in, route_declarations};
use crate::editors::graph::{GRAPH, GraphTopbarDynamic, PreviewNode, asset_subpanes, node_subpanes};
use crate::inline::{Inline, inline_for_param, inline_for_pin, inline_scene};
use crate::material_state::MaterialStates;
use crate::ops::droppable::Dropped;
use crate::ops::prelude::*;
use crate::ops::{Dialog, Editor, GraphProp, Link, MaterialAction, Submit};
use crate::ui::input::NODE_HOTKEYS;
use crate::ui::palette::{entries_for, pick_for};

/// Material's graph: materials and functions on the bevy_noodle canvas.
pub struct MaterialGraph;

/// The node a material previews (decision 0022).
fn previewed_node(asset: &Asset) -> Option<&str> {
    asset.preview.node.as_deref().filter(|_| matches!(asset.kind, AssetKind::Material(_)))
}

/// A wire end as the palette's [`Link`].
fn link_of(pin: &PinRef) -> Link {
    Link { node: pin.node.clone(), pin: pin.pin.clone(), from_output: pin.output }
}

/// Menu rows as core menu entries (no icons).
fn entries(items: Vec<(String, Action)>) -> Vec<MenuEntry> {
    items.into_iter().map(|(label, action)| MenuEntry::from((None, label, action))).collect()
}

impl GraphDomain for MaterialGraph {
    type Doc = Doc;
    const VIEW: ViewKindId = GRAPH;

    fn view(doc: &Doc) -> GraphView<'_> {
        let pin = |(node, pin): &(String, String), output| PinKey { node: node.clone(), pin: pin.clone(), output };
        let frame = |f: &shader_graph_core::model::Frame| FrameView {
            id: f.id.clone(),
            rect: f.rect,
            title: f.title.clone(),
            hue: f.hue,
            color: f.color,
            nodes: f.nodes.clone(),
        };
        GraphView {
            nodes: doc
                .asset
                .nodes
                .iter()
                .map(|(id, n)| NodeView { id: id.clone(), pos: Vec2::new(n.pos.0, n.pos.1) })
                .collect(),
            edges: doc.asset.edges.iter().map(|e| (pin(&e.from, true), pin(&e.to, false))).collect(),
            frames: doc.asset.comments.iter().map(frame).collect(),
            selection: &doc.selection,
            selected_frame: doc.selected_frame.as_deref(),
            enclosed_frames: &doc.enclosed_frames,
        }
    }

    /// Title and header colour by category or name, pins by the resolved signature (colour by type), inline editors
    /// of the node's value and of unconnected inputs, the previewed node's pulse (decision 0022).
    fn node_look(ctx: &GraphCtxRef, d: &Doc, id: &str) -> NodeLook {
        let m = ctx.editor.domains.material();
        let asset = &d.asset;
        let Some(n) = asset.nodes.get(id) else { return NodeLook::default() };
        let routes = route_declarations(asset, &m.registry);
        let def = m.registry.get(&n.kind);
        let sig = node_signature(asset, id, &m.registry, &m.functions, &d.id.path);
        let emphasis = Emphasis { pulse: previewed_node(asset) == Some(id) };
        let comment = (!n.comment.is_empty()).then(|| n.comment.clone());
        if def.is_some_and(|d| d.role() == NodeRole::Reroute) {
            let ty = sig.as_ref().and_then(|s| s.outputs.first()).map_or(PinType::Any, |p| p.ty);
            let color = pin_color(ty);
            let pin = |id: &str, output| PinLook { id: id.into(), color, output, ..default() };
            return NodeLook {
                shape: NodeShape::Pill,
                pins: vec![pin("in", false), pin("out", true)],
                emphasis,
                comment,
                ..default()
            };
        }
        let mut look = NodeLook {
            title: node_title_in(&m.registry, &m.functions, asset, id, &routes),
            header: header_color(&m.registry, asset, id, &routes),
            emphasis,
            comment,
            ..default()
        };
        let params = m.registry.resolved_params(&n.kind, &n.params);
        if let Some((key, inline)) = inline_for_param(&n.kind, &params) {
            // Vector editors (a number field per component) need a wider node.
            look.wide |= matches!(inline, Inline::Vector(_));
            let v = params.get(&key).cloned().unwrap_or(Value::Float(0.0));
            look.body = Some(inline_scene(GraphProp::Param { node: id.into(), key }, inline, &v));
        }
        let Some(sig) = &sig else {
            look.note = Some("unknown node kind".into());
            return look;
        };
        for p in &sig.outputs {
            let label = if p.label.is_empty() { p.ty.label().to_string() } else { p.label.clone() };
            look.pins.push(PinLook { id: p.id.clone(), label, color: pin_color(p.ty), output: true, ..default() });
        }
        // A usage's input is fed by its declaration, not by a wire.
        let usage = def.is_some_and(|d| d.role() == NodeRole::RouteUsage);
        for p in &sig.inputs {
            let mut pin = PinLook {
                id: p.id.clone(),
                label: if p.label.is_empty() { p.id.clone() } else { p.label.clone() },
                color: pin_color(p.ty),
                output: false,
                implicit: p.implicit.is_some(),
                required: p.required,
                inactive: p.inactive,
                hidden: usage,
            };
            if !usage && asset.edge_into(id, &p.id).is_none() {
                if let Some(what) = &p.implicit {
                    pin.label.push_str(&format!(" ({what})"));
                } else if !p.required
                    && let Some(inline) = inline_for_pin(p.ty, p.range)
                {
                    look.wide |= matches!(inline, Inline::Vector(_));
                    let v = n.inputs.get(&p.id).cloned().unwrap_or_else(|| p.default.clone()).coerce(p.ty);
                    let target = GraphProp::Input { node: id.into(), pin: p.id.clone() };
                    look.inline.push((p.id.clone(), inline_scene(target, inline, &v)));
                }
            }
            look.pins.push(pin);
        }
        look
    }

    /// The active graph's diagnostics by node; `W102` does not colour a pin label.
    fn diagnostics(ctx: &GraphCtxRef, _d: &Doc) -> Vec<DiagMark> {
        let mut out = Vec::new();
        for (node, diags) in ctx.editor.node_diags() {
            for diag in diags {
                let pin = diag.pin.clone().filter(|_| diag.code != Code::W102);
                out.push(DiagMark { node: node.clone(), pin, error: diag.is_error() });
            }
        }
        out
    }

    /// The model rules (types spec §7.2, cycles): [`connection_error`].
    fn can_connect(ctx: &GraphCtxRef, d: &Doc, from: &PinRef, to: &PinRef) -> Result<(), String> {
        let m = ctx.editor.domains.material();
        let error = connection_error(
            &d.asset,
            (&from.node, &from.pin),
            (&to.node, &to.pin),
            &m.registry,
            &m.functions,
            &d.id.path,
        );
        error.map_or(Ok(()), Err)
    }

    fn plan_move_wires(ctx: &GraphCtxRef, _d: &Doc, from: &PinRef, to: &PinRef) -> MovePlan {
        let (f, t) = ((from.node.clone(), from.pin.clone()), (to.node.clone(), to.pin.clone()));
        ctx.editor.plan_move_wires(&f, from.output, &t).map(drop)
    }

    fn header_action(ctx: &GraphCtxRef, _d: &Doc, node: &str) -> Option<Action> {
        ctx.editor.header_action(node)
    }

    fn open_action(ctx: &GraphCtxRef, _d: &Doc, node: &str) -> Option<Action> {
        ctx.editor.open_action(node)
    }

    fn pin_menu(ctx: &GraphCtxRef, _d: &Doc, pin: &PinRef) -> Vec<MenuEntry> {
        entries(ctx.editor.pin_menu(&pin.node, &pin.pin))
    }

    fn node_menu(ctx: &GraphCtxRef, _d: &Doc, nodes: &[String]) -> Vec<MenuEntry> {
        nodes.first().map(|n| entries(ctx.editor.node_menu(n))).unwrap_or_default()
    }

    /// Shift + - * / (decision 0021): the selected nodes' converts bound to that key.
    fn hotkey_actions(ctx: &GraphCtxRef, _d: &Doc, key: GraphHotkey) -> Vec<Action> {
        let key = match key {
            GraphHotkey::Plus => HotkeyKey::Plus,
            GraphHotkey::Minus => HotkeyKey::Minus,
            GraphHotkey::Star => HotkeyKey::Star,
            GraphHotkey::Slash => HotkeyKey::Slash,
        };
        ctx.editor.hotkey_actions(Hotkey::shift(key))
    }

    fn placement_hotkeys() -> &'static [(KeyCode, PlaceSpec)] {
        &NODE_HOTKEYS
    }

    /// A `spec` node at `at`, on a graph whose kind is known.
    fn place(ctx: &GraphCtxRef, d: &Doc, spec: PlaceSpec, at: Vec2) -> Option<Action> {
        ctx.editor.asset_kind(&d.id)?;
        Some(MaterialAction::AddNode { kind: spec.into(), pos: (at.x, at.y), link: None, params: Vec::new() }.into())
    }

    /// [`entries_for`]; after a wire from an output the Named Route declaration wins ties (design §6.4), and picking
    /// one opens its rename dialog.
    fn palette(ctx: &GraphCtxRef, _d: &Doc, link: Option<&PinRef>) -> Vec<PaletteEntry> {
        let link = link.map(link_of);
        let registry = &ctx.editor.domains.material().registry;
        let from_output = link.as_ref().is_some_and(|l| l.from_output);
        let all = entries_for(ctx.editor, link.as_ref());
        all.into_iter()
            .map(|e| {
                let route = crate::search::is_route_declaration(registry, &e.kind);
                let (entry, link) = (e.clone(), link.clone());
                PaletteEntry {
                    id: e.kind,
                    name: e.name,
                    category: e.category,
                    keywords: e.keywords,
                    description: e.description,
                    lead: from_output && route,
                    pick: Arc::new(move |pos| pick_for(route, &entry, (pos.x, pos.y), link.clone())),
                }
            })
            .collect()
    }

    /// [`Dropped`] rows: [`MaterialAction::Drop`].
    fn drop(_ctx: &GraphCtxRef, _d: &Doc, item: &DropItem, pos: Vec2, over: Option<&str>) -> Option<Action> {
        let item = Dropped::from_row(item)?;
        Some(MaterialAction::Drop { item, pos: (pos.x, pos.y), node: over.map(str::to_string) }.into())
    }

    /// One selected node: its subpanes; none: the asset's.
    fn sidebar(ctx: &GraphCtxRef, d: &Doc) -> Vec<Box<dyn Scene>> {
        if d.selection.len() == 1 {
            let id = d.selection.iter().next().cloned().unwrap_or_default();
            node_subpanes(ctx.editor, &id)
        } else {
            asset_subpanes(ctx.editor, d)
        }
    }

    fn frame_prop(frame: &str, field: FrameField) -> PropKey {
        PropKey::new(match field {
            FrameField::Title => GraphProp::FrameTitle(frame.into()),
            FrameField::Hue => GraphProp::FrameHue(frame.into()),
        })
    }

    /// [`GraphTopbarDynamic`]: the Preview mode group and the "Show generated" buttons (design §5.2, §5.4).
    fn topbar_extras() -> Vec<Box<dyn Scene>> {
        vec![Box::new(bsn! {
            GraphTopbarDynamic
            Node { flex_grow: 1.0, flex_direction: FlexDirection::Row, align_items: AlignItems::Center, column_gap: px(2) }
        })]
    }

    fn previews(ctx: &GraphCtxRef, d: &Doc, node: &str) -> Vec<PreviewRow> {
        let Some(n) = d.asset.nodes.get(node) else { return Vec::new() };
        let def = ctx.editor.domains.material().registry.get(&n.kind);
        def.map(|def| crate::ui::node_preview::previews(def.as_ref(), &node.to_string())).unwrap_or_default()
    }

    /// `V`: the node preview on release (decision 0022).
    fn bindings() -> Vec<Box<dyn Scene>> {
        vec![Box::new(hotkey::<PreviewNode>(plain([KeyCode::KeyV])))]
    }

    fn build(app: &mut App) {
        crate::editors::graph::build(app);
    }

    fn input(app: &mut App) {
        crate::editors::graph::input(app);
    }
}

/// Applies a [`GraphEdit`] to the active graph (a step [`Doc`]'s `apply` hands to the domain, as it needs the
/// editor: connection rules, function signatures, the canvas's move tracking, dialogs).
pub(crate) fn graph_edit(e: &mut Editor, edit: GraphEdit) {
    match edit {
        GraphEdit::Select(sel) => {
            if let Some(d) = e.doc_mut()
                && (d.selection != sel || d.selected_frame.is_some() || !d.enclosed_frames.is_empty())
            {
                d.set_selection(sel);
                d.selected_frame = None;
                e.ui.props += 1;
            }
        }
        GraphEdit::SelectFrame(f) => {
            if let Some(d) = e.doc_mut()
                && d.selected_frame != f
            {
                d.selected_frame = f;
                d.enclosed_frames.clear();
                e.ui.props += 1;
            }
        }
        // The sidebar keeps showing the node selection (design §4.5): no props refresh.
        GraphEdit::SelectFrames(frames) => {
            if let Some(d) = e.doc_mut() {
                d.enclosed_frames = d.enclosed(&frames, &d.selection);
            }
        }
        GraphEdit::Connect { from, to } => e.connect(from, to),
        GraphEdit::Disconnect { to } => {
            if let Some(d) = e.doc_mut() {
                d.edit(|a| a.disconnect_input((&to.0, &to.1)));
            }
        }
        GraphEdit::DisconnectPin { node, pin, output } => {
            let at_pin = |edge: &shader_graph_core::model::Edge| {
                let end = if output { &edge.from } else { &edge.to };
                end.0 == node && end.1 == pin
            };
            if let Some(d) = e.doc_mut() {
                d.edit(|a| a.edges.retain(|edge| !at_pin(edge)));
            }
        }
        GraphEdit::MoveWires { node, pin, output, to } => e.move_wires((node, pin), output, to),
        GraphEdit::Moved { nodes, frames } => {
            // The canvas already shows the move: it follows the document's new revision without a rebuild
            // (`crate::follow_moves`).
            let shown = |e: &Editor| e.doc().and_then(|d| Some((e.active_key()?, d.revision)));
            let before = shown(e);
            if let Some(d) = e.doc_mut() {
                d.edit(|a| {
                    for (id, p) in &nodes {
                        if let Some(n) = a.nodes.get_mut(id) {
                            n.pos = *p;
                        }
                    }
                    for (id, p) in &frames {
                        if let Some(f) = a.comments.iter_mut().find(|f| f.id == *id) {
                            f.rect.0 = p.0;
                            f.rect.1 = p.1;
                        }
                    }
                });
            }
            let after = shown(e);
            e.domains.material_mut().moves.push((before, after));
        }
        GraphEdit::AutoLayout => e.auto_layout(),
        // Around the selection; with none, a default-size frame at `at` on the grid (the graph editor resolves
        // `None` to the visible canvas centre; headless it is the origin).
        GraphEdit::AddComment(at) => {
            if let Some(d) = e.doc_mut() {
                let sel = d.selection.clone();
                d.add_frame("Comment", &sel, crate::doc::snap(at.unwrap_or((0.0, 0.0))));
            }
        }
        GraphEdit::AddReroute { from, to, pos } => {
            if let Some(d) = e.doc_mut() {
                let r = shader_graph_core::model::new_node_id();
                d.edit(|a| {
                    a.nodes.insert(r.clone(), shader_graph_core::Node::new("stdlib::Reroute", crate::doc::snap(pos)));
                    a.connect((&from.0, &from.1), (&r, "in"));
                    a.connect((&r, "out"), (&to.0, &to.1));
                });
            }
        }
        GraphEdit::FrameTitle(id) => text_dialog(e, GraphProp::FrameTitle(id)),
        GraphEdit::View(v) => e.view_requests.push(v),
    }
}

/// Asks for the text of a node's name / label or a comment's title (double click, design §5.5); the dialog submits it
/// as a `TabAction::SetValue` of `target`.
pub(crate) fn text_dialog(e: &mut Editor, target: GraphProp) {
    let (title, label) = match target {
        GraphProp::FrameTitle(_) => ("Comment", "Title"),
        GraphProp::NodeComment(_) => ("Node comment", "Comment (shown above the node, editor only)"),
        _ => ("Rename node", "Name"),
    };
    let key = PropKey::new(target);
    let initial = e.current_value(&key).map(|v| v.as_str().to_string()).unwrap_or_default();
    e.dialog = Some(Dialog::Input { title: title.into(), label: label.into(), initial, submit: Submit::SetText(key) });
}
