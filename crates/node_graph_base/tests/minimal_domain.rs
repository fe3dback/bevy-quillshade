//! The seam without Material (design §7.3): test-only graph domains whose nodes have one input and one output pin,
//! each with its own document type, registered through `register_graph`, driven headless; one alone, and two in one
//! app.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use bevy::camera::NormalizedRenderTarget;
use bevy::picking::backend::HitData;
use bevy::picking::pointer::{Location, PointerId};

use bevy::prelude::*;
use bevy_noodle::prelude::GraphEdit as NoodleEdit;
use bevy_noodle::prelude::*;
use bevy_noodle::style::kit;
use editor_core::EditorPlugins;
use editor_core::action::{Action, Actions, Handled, TabAction, ViewRequest};
use editor_core::doc_kind::{DocCtx, TabDoc};
use editor_core::domain::{DomainInfo, DomainList, DomainRegistrar, EditorDomain};
use editor_core::dynval::{NoTarget, PropKey};
use editor_core::editors::ViewOf;
use editor_core::menu::MenuEntry;
use editor_core::state::editor::Editor;
use editor_core::tab::{Tab, ViewKindId};
use editor_core::ui::icons::{IconId, IconRef};
use node_graph_base::canvas::{ERROR, NodeHeader, NodeKey};
use node_graph_base::{
    DiagMark, DropItem, EditorCanvas, EditorCanvasMarker, FrameField, GRID_SNAP, GraphCtxRef, GraphDomain, GraphEdit,
    GraphHotkey, GraphView, MovePlan, NodeLook, NodeView, PaletteEntry, PinKey, PinLook, PinRef, PlaceSpec, PreviewRow,
    register_graph,
};
use project_core::regen::DomainBackend;
use project_core::{AssetRef, DomainId, Project};

/// The two test domains: `K` 0 and 1.
const DOMAINS: [DomainId; 2] = [DomainId::new("tiny"), DomainId::new("tiny2")];
const VIEWS: [ViewKindId; 2] = [ViewKindId("tiny_graph"), ViewKindId("tiny_graph2")];

/// How often the canvas asked [`TinyGraph::can_connect`].
static CHECKS: AtomicUsize = AtomicUsize::new(0);

/// A test domain's graph document: nodes at positions (stored as given), wires `(from node, to node)` (always
/// `out` → `in`), nodes with an error.
#[derive(Debug, Clone)]
struct TinyDoc<const K: usize> {
    aref: AssetRef,
    nodes: BTreeMap<String, (f32, f32)>,
    edges: Vec<(String, String)>,
    errors: BTreeSet<String>,
    selection: BTreeSet<String>,
    enclosed: BTreeSet<String>,
    revision: u64,
}

impl<const K: usize> TinyDoc<K> {
    fn new() -> Self {
        let nodes = [("a", (0.0, 0.0)), ("b", (300.0, 0.0)), ("c", (300.0, 200.0))];
        TinyDoc {
            aref: AssetRef { domain: DOMAINS[K], path: "g".into(), suffix: "tiny" },
            nodes: nodes.into_iter().map(|(id, p)| (id.to_string(), p)).collect(),
            edges: Vec::new(),
            errors: BTreeSet::new(),
            selection: BTreeSet::new(),
            enclosed: BTreeSet::new(),
            revision: 1,
        }
    }

    /// The model takes positions verbatim: snapping is the canvas's (the base's) job.
    fn edit(&mut self, edit: &GraphEdit) {
        match edit {
            GraphEdit::Connect { from, to } => self.edges.push((from.0.clone(), to.0.clone())),
            GraphEdit::Moved { nodes, .. } => {
                for (id, p) in nodes {
                    if let Some(at) = self.nodes.get_mut(id) {
                        *at = *p;
                    }
                }
            }
            GraphEdit::Select(sel) => self.selection = sel.clone(),
            _ => return,
        }
        self.revision += 1;
    }
}

impl<const K: usize> TabDoc for TinyDoc<K> {
    fn asset(&self) -> Option<&AssetRef> {
        Some(&self.aref)
    }
    fn view(&self) -> ViewKindId {
        VIEWS[K]
    }
    fn base_title(&self) -> String {
        "tiny".into()
    }
    fn icon(&self) -> IconRef {
        IconId::File.into()
    }
    fn watermark(&self) -> Option<&'static str> {
        None
    }
    fn is_dirty(&self) -> bool {
        false
    }
    fn is_read_only(&self) -> bool {
        false
    }
    fn revision(&self) -> u64 {
        self.revision
    }
    fn save(&mut self, _ctx: &mut DocCtx) -> std::io::Result<()> {
        Ok(())
    }
    fn changed_on_disk(&self, _project: &Project) -> bool {
        false
    }
    fn reload_from_disk(&mut self, _ctx: &mut DocCtx) -> Result<(), String> {
        Ok(())
    }
    fn keep_mine(&mut self, _project: &Project) {}
    fn rename(&mut self, _project: &Project, _from: &AssetRef, _to: &AssetRef) {}
    fn config_key(&self) -> Option<String> {
        Some("g".into())
    }
    fn apply(&mut self, _ctx: &mut DocCtx, action: &TabAction) -> Handled {
        let TabAction::Doc(p) = action else { return Handled::No };
        let Some(edit) = p.downcast::<GraphEdit>() else { return Handled::No };
        self.edit(edit);
        Handled::Yes
    }
}

struct TinyGraph<const K: usize>;

/// The single-domain tests' graph.
type Tiny = TinyGraph<0>;

impl<const K: usize> GraphDomain for TinyGraph<K> {
    type Doc = TinyDoc<K>;
    const VIEW: ViewKindId = VIEWS[K];

    fn view(doc: &TinyDoc<K>) -> GraphView<'_> {
        let pin = |node: &str, pin: &str, output| PinKey { node: node.into(), pin: pin.into(), output };
        GraphView {
            nodes: doc.nodes.iter().map(|(id, p)| NodeView { id: id.clone(), pos: Vec2::new(p.0, p.1) }).collect(),
            edges: doc.edges.iter().map(|(a, b)| (pin(a, "out", true), pin(b, "in", false))).collect(),
            frames: Vec::new(),
            selection: &doc.selection,
            selected_frame: None,
            enclosed_frames: &doc.enclosed,
        }
    }

    fn node_look(_: &GraphCtxRef, _: &TinyDoc<K>, node: &str) -> NodeLook {
        let pin = |id: &str, output| PinLook { id: id.into(), label: id.into(), output, ..default() };
        NodeLook { title: node.to_uppercase(), pins: vec![pin("out", true), pin("in", false)], ..default() }
    }

    fn diagnostics(_: &GraphCtxRef, doc: &TinyDoc<K>) -> Vec<DiagMark> {
        doc.errors.iter().map(|n| DiagMark { node: n.clone(), pin: Some("in".into()), error: true }).collect()
    }

    /// Outputs into inputs only; node `c` takes no wires.
    fn can_connect(_: &GraphCtxRef, _: &TinyDoc<K>, from: &PinRef, to: &PinRef) -> Result<(), String> {
        CHECKS.fetch_add(1, Ordering::SeqCst);
        if !from.output || to.output {
            return Err("an output must connect to an input".into());
        }
        if to.node == "c" { Err("`c` takes no wires".into()) } else { Ok(()) }
    }

    fn plan_move_wires(_: &GraphCtxRef, _: &TinyDoc<K>, _: &PinRef, _: &PinRef) -> MovePlan {
        Ok(())
    }
    fn header_action(_: &GraphCtxRef, _: &TinyDoc<K>, _: &str) -> Option<Action> {
        None
    }
    fn open_action(_: &GraphCtxRef, _: &TinyDoc<K>, _: &str) -> Option<Action> {
        None
    }
    fn pin_menu(_: &GraphCtxRef, _: &TinyDoc<K>, _: &PinRef) -> Vec<MenuEntry> {
        Vec::new()
    }
    fn node_menu(_: &GraphCtxRef, _: &TinyDoc<K>, _: &[String]) -> Vec<MenuEntry> {
        Vec::new()
    }
    fn hotkey_actions(_: &GraphCtxRef, _: &TinyDoc<K>, _: GraphHotkey) -> Vec<Action> {
        Vec::new()
    }
    fn placement_hotkeys() -> &'static [(KeyCode, PlaceSpec)] {
        &[]
    }
    fn place(_: &GraphCtxRef, _: &TinyDoc<K>, _: PlaceSpec, _: Vec2) -> Option<Action> {
        None
    }
    fn palette(_: &GraphCtxRef, _: &TinyDoc<K>, _: Option<&PinRef>) -> Vec<PaletteEntry> {
        Vec::new()
    }
    fn drop(_: &GraphCtxRef, _: &TinyDoc<K>, _: &DropItem, _: Vec2, _: Option<&str>) -> Option<Action> {
        None
    }
    fn sidebar(_: &GraphCtxRef, _: &TinyDoc<K>) -> Vec<Box<dyn Scene>> {
        Vec::new()
    }
    fn frame_prop(_: &str, _: FrameField) -> PropKey {
        PropKey::new(NoTarget)
    }
    fn topbar_extras() -> Vec<Box<dyn Scene>> {
        Vec::new()
    }
    fn previews(_: &GraphCtxRef, _: &TinyDoc<K>, _: &str) -> Vec<PreviewRow> {
        Vec::new()
    }
}

struct TinyDomain<const K: usize>;

impl<const K: usize> EditorDomain for TinyDomain<K> {
    fn id(&self) -> DomainId {
        DOMAINS[K]
    }
    fn info(&self) -> DomainInfo {
        DomainInfo { title: "Tiny", icon: IconId::File.into(), summary: "" }
    }
    fn backend(&self) -> Option<Box<dyn DomainBackend>> {
        None
    }
    fn register(&self, r: &mut DomainRegistrar) {
        register_graph::<TinyGraph<K>>(r);
    }
}

/// A headless app with bevy_noodle's core and interaction plugins (pointer drags) and `editor`, and one bare canvas per view
/// in `views` (`None`: a canvas without a view tag).
fn app_with(editor: Editor, views: &[Option<ViewKindId>]) -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default(), bevy::scene::ScenePlugin, bevy::input::InputPlugin));
    app.add_plugins((NoodleCorePlugin, NoodleInteractionPlugin));
    // Pointer events bubble through `PointerTraversal`, which reads `Window`.
    app.world_mut().register_component::<Window>();
    app.init_resource::<bevy::picking::hover::HoverMap>().init_resource::<UiScale>();
    app.init_asset::<Font>().init_asset::<Image>();
    app.init_resource::<bevy::input_focus::InputFocus>();
    app.insert_resource(editor);
    app.add_plugins(EditorPlugins);
    for view in views {
        match *view {
            Some(v) => app.world_mut().spawn_scene(bsn! { @kit::canvas() EditorCanvasMarker ViewOf(v) }).unwrap(),
            None => app.world_mut().spawn_scene(bsn! { @kit::canvas() EditorCanvasMarker }).unwrap(),
        };
    }
    app.update();
    app.update();
    app
}

/// A headless app with only the first tiny domain, its graph tab open on the canvas.
fn app() -> App {
    let mut editor = Editor::new(Arc::new(DomainList::new(vec![Box::new(TinyDomain::<0>)])));
    editor.push_tab(Tab::new(DOMAINS[0], TinyDoc::<0>::new()));
    app_with(editor, &[None])
}

fn doc(app: &App) -> &TinyDoc<0> {
    app.world().resource::<Editor>().active_doc::<TinyDoc<0>>().expect("the tiny graph is active")
}

fn act(app: &mut App, a: impl Into<Action>) {
    app.world_mut().resource_mut::<Actions>().push(a);
    app.update();
    app.update();
}

fn port(app: &mut App, node: &str, pin: &str, output: bool) -> Entity {
    let w = app.world_mut();
    let mut q = w.query::<(Entity, &PinKey)>();
    q.iter(w).find(|(_, k)| k.node == node && k.pin == pin && k.output == output).map(|(e, _)| e).unwrap()
}

fn canvas_nodes(app: &mut App) -> BTreeSet<String> {
    let w = app.world_mut();
    w.query::<&NodeKey>().iter(w).map(|k| k.0.clone()).collect()
}

#[test]
fn opening_the_tab_builds_one_canvas_node_per_model_node() {
    let mut app = app();
    assert!(app.world().contains_resource::<EditorCanvas<Tiny>>());
    let model: BTreeSet<String> = doc(&app).nodes.keys().cloned().collect();
    assert_eq!(canvas_nodes(&mut app), model);
    // Each node has its two pins.
    for n in &model {
        port(&mut app, n, "in", false);
        port(&mut app, n, "out", true);
    }
}

#[test]
fn a_scripted_connect_adds_an_edge_and_the_canvas_shows_it() {
    let mut app = app();
    act(&mut app, GraphEdit::Connect { from: ("a".into(), "out".into()), to: ("b".into(), "in".into()) });
    assert_eq!(doc(&app).edges, [("a".to_string(), "b".to_string())]);
    let w = app.world_mut();
    assert_eq!(w.query::<&Edge>().iter(w).count(), 1, "the rebuilt canvas has the wire");
}

#[test]
fn a_wire_drawn_on_the_canvas_becomes_a_connect() {
    let mut app = app();
    let canvas = app.world().resource::<EditorCanvas<Tiny>>().canvas;
    let (from, to) = (port(&mut app, "a", "out", true), port(&mut app, "b", "in", false));
    app.world_mut().graph_edit_with_origin(canvas, NoodleEdit::Connect { from, to }, EditOrigin::Interaction).unwrap();
    app.update();
    app.update();
    assert_eq!(doc(&app).edges, [("a".to_string(), "b".to_string())]);
}

#[test]
fn a_refused_connection_leaves_the_model_unchanged() {
    let mut app = app();
    let canvas = app.world().resource::<EditorCanvas<Tiny>>().canvas;
    let before = (doc(&app).edges.clone(), doc(&app).revision);
    // In → in: bevy_noodle refuses it itself (no output among the two ports), before any `ConnectionCheck`, so this
    // case does not reach `can_connect`; the next one does.
    let (a_in, b_in) = (port(&mut app, "a", "in", false), port(&mut app, "b", "in", false));
    let r = app.world_mut().graph_edit_with_origin(
        canvas,
        NoodleEdit::Connect { from: a_in, to: b_in },
        EditOrigin::Interaction,
    );
    assert!(r.is_err(), "in → in is refused");
    // Out → in that the domain refuses: the canvas asks it.
    let checks = CHECKS.load(Ordering::SeqCst);
    let (a_out, c_in) = (port(&mut app, "a", "out", true), port(&mut app, "c", "in", false));
    let r = app.world_mut().graph_edit_with_origin(
        canvas,
        NoodleEdit::Connect { from: a_out, to: c_in },
        EditOrigin::Interaction,
    );
    assert!(r.is_err(), "the domain refuses a wire into `c`");
    assert!(CHECKS.load(Ordering::SeqCst) > checks, "`can_connect` decided");
    app.update();
    app.update();
    assert_eq!((doc(&app).edges.clone(), doc(&app).revision), before);
}

/// A pointer event on `target` (bevy_noodle's picking events, as its own tests trigger them).
fn pointer(app: &mut App, target: Entity, trigger: impl FnOnce(&mut World, Entity, Pointer)) {
    let location = Location { target: NormalizedRenderTarget::None { width: 1, height: 1 }, position: Vec2::ZERO };
    let w = app.world_mut();
    trigger(w, target, Pointer::new(PointerId::Mouse, location));
    w.flush();
}

/// Snapping is the base's: the attached canvas snaps pointer drags to [`GRID_SNAP`] and the finished drag becomes one
/// `GraphEdit::Moved` the model stores as it is.
#[test]
fn a_dragged_node_lands_on_the_grid() {
    let mut app = app();
    let canvas = app.world().resource::<EditorCanvas<Tiny>>().canvas;
    assert_eq!(app.world().get::<CanvasInteraction>(canvas).unwrap().snap, Some(GRID_SNAP));
    // The model takes any position verbatim.
    act(&mut app, GraphEdit::Moved { nodes: vec![("b".into(), (123.4, -56.6))], frames: Vec::new() });
    assert_eq!(doc(&app).nodes["b"], (123.4, -56.6));
    // A pointer drag of `c` from (300, 200) by (13, 7); headless, the canvas gets its size by hand (no UI layout).
    let size = ComputedNode { size: Vec2::new(800.0, 600.0), inverse_scale_factor: 1.0, ..default() };
    app.world_mut().entity_mut(canvas).insert(size);
    // By its title bar, the kit node's drag handle.
    let node = {
        let w = app.world_mut();
        let c = w.query::<(Entity, &NodeKey)>().iter(w).find(|(_, k)| k.0 == "c").unwrap().0;
        w.query_filtered::<(Entity, &ChildOf), With<NodeHeader>>().iter(w).find(|(_, p)| p.parent() == c).unwrap().0
    };
    let (button, by) = (PointerButton::Primary, Vec2::new(13.0, 7.0));
    let hit = HitData::new(Entity::PLACEHOLDER, 0.0, None, None);
    pointer(&mut app, node, |w, entity, pointer| w.trigger(PointerDragStart { entity, pointer, button, hit }));
    pointer(&mut app, node, |w, entity, pointer| {
        w.trigger(PointerDrag { entity, pointer, button, distance: by, delta: by })
    });
    pointer(&mut app, node, |w, entity, pointer| w.trigger(PointerDragEnd { entity, pointer, button, distance: by }));
    for _ in 0..4 {
        app.update();
    }
    let p = doc(&app).nodes["c"];
    assert_eq!(p, (310.0, 210.0), "on the grid, not (313, 207)");
    assert_eq!((p.0 % GRID_SNAP, p.1 % GRID_SNAP), (0.0, 0.0));
    // The canvas follows the model.
    let w = app.world_mut();
    let shown = w.query::<(&NodeKey, &NodePosition)>().iter(w).find(|(k, _)| k.0 == "c").map(|(_, p)| p.0);
    assert_eq!(shown, Some(Vec2::new(310.0, 210.0)));
}

/// Two graph domains in one app, the first one's tab active, each canvas tagged with its view.
fn two_domains() -> App {
    let domains: Vec<Box<dyn EditorDomain>> = vec![Box::new(TinyDomain::<0>), Box::new(TinyDomain::<1>)];
    let mut editor = Editor::new(Arc::new(DomainList::new(domains)));
    let mut first = TinyDoc::<0>::new();
    first.errors.insert("a".into());
    editor.push_tab(Tab::new(DOMAINS[1], TinyDoc::<1>::new()));
    editor.push_tab(Tab::new(DOMAINS[0], first));
    app_with(editor, &[Some(VIEWS[0]), Some(VIEWS[1])])
}

/// The outline of the active graph's node with an error survives frames in which the other graph domain's systems
/// run too.
#[test]
fn two_graph_domains_keep_the_active_ones_outlines() {
    let mut app = two_domains();
    assert!(app.world().contains_resource::<EditorCanvas<TinyGraph<1>>>());
    let outline = |app: &mut App, id: &str| {
        let w = app.world_mut();
        let node = w.query::<(Entity, &NodeKey)>().iter(w).find(|(_, k)| k.0 == id).unwrap().0;
        w.get::<Outline>(node).map(|o| o.color)
    };
    for frame in 0..3 {
        app.update();
        assert_eq!(outline(&mut app, "a"), Some(ERROR), "frame {frame}: the error outline");
        assert_eq!(outline(&mut app, "b"), None, "frame {frame}");
    }
    // Only the active graph's canvas shows nodes.
    assert_eq!(canvas_nodes(&mut app).len(), 3);
}

/// Queues a camera request, as a domain does for `GraphEdit::View`.
fn request(app: &mut App, r: ViewRequest) {
    app.world_mut().resource_mut::<Editor>().view_requests.push(r);
    app.update();
    app.update();
}

/// A camera request on the active graph is carried out by its canvas, never dropped by the other domain's systems.
#[test]
fn two_graph_domains_apply_the_active_ones_view_requests() {
    let mut app = two_domains();
    let canvas = app.world().resource::<EditorCanvas<Tiny>>().canvas;
    // Headless: no UI layout, so give the canvas and its nodes their size.
    let w = app.world_mut();
    w.entity_mut(canvas).insert(ComputedNode { size: Vec2::new(800.0, 600.0), inverse_scale_factor: 1.0, ..default() });
    let nodes: Vec<Entity> = w.query_filtered::<Entity, With<NodeKey>>().iter(w).collect();
    for n in nodes {
        w.entity_mut(n).insert(ComputedNode { size: Vec2::new(100.0, 50.0), inverse_scale_factor: 1.0, ..default() });
    }
    let before = *app.world().get::<CanvasView>(canvas).unwrap();
    // What a domain does with `GraphEdit::View`: queue the request (the test domains keep no state to do it).
    request(&mut app, ViewRequest::Set { centre: Vec2::new(150.0, 100.0), zoom: 0.5 });
    let view = *app.world().get::<CanvasView>(canvas).unwrap();
    assert_eq!(view.zoom, 0.5);
    assert_eq!(view.pan, Vec2::new(400.0, 300.0) - Vec2::new(150.0, 100.0) * 0.5);
    assert_ne!(view, before);
    request(&mut app, ViewRequest::FitAll);
    let fit = *app.world().get::<CanvasView>(canvas).unwrap();
    assert_ne!(fit, view, "fit moved the camera");
    assert!(app.world().resource::<Editor>().view_requests.is_empty());
}
