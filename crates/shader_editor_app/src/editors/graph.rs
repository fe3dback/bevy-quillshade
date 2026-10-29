//! Graph editor kind: node graph of a material or function on the bevy_noodle canvas ([`Doc`]).

use std::collections::BTreeMap;

use bevy::feathers::containers::flex_spacer;
use bevy::prelude::*;
use bevy_enhanced_input::prelude::{Actions as InputActions, Complete, Fire, InputAction, Start};
use bevy_noodle::prelude::{CanvasView, GraphQuery};
use shader_graph_core::model::{Asset, AssetKind, BlendMode, MaterialKind, PbrOutputMode, PreviewMode, Shading};
use shader_graph_core::registry::{NodeRole, ParamDef, ParamKind};
use shader_graph_core::{Hotkey, HotkeyKey, Value};
use shader_project::{AssetId, AssetSet, AssetType, Project, assets, ops};

use super::main_input::{
    EditCopy, EditCut, EditDelete, EditDuplicate, EditSelectAll, add_kind, ctrl, held_hotkey, hotkey, on_combo,
    on_plain, plain, shift,
};
use super::{EditorAreas, EditorInput, SidebarKey, StatusCtx, StatusInfo, fill_sidebar_with};
use crate::canvas::{EditorCanvas, frame_color, view_center};
use crate::doc::{Doc, node_signature, route_declarations};
use crate::doc_kind::{TabDoc, asset_title, io_error};
use crate::inline::{Inline, inline_for_param, inline_for_pin};
use crate::keys::no_command_mods;
use crate::state::{Action, Actions, Editor, PropTarget, ViewRequest, preview_mode_label};
use crate::tab::{Lang, ViewKind};
use crate::ui::icons::IconId;
use crate::ui::input::CanvasCursor;
use crate::ui::palette::Palette;
use crate::ui::props::{
    Choices, GradientSlots, SlotEntry, checkbox_row, hue_input, labeled, pin_kind, select_w, text_area, text_w,
    value_row,
};
use crate::ui::widgets::{Caption, GroupButton, button_group, subpane_box};
use crate::ui::{TEXT, TEXT_DIM, button, icon_button, icon_tool_button, plural, text};

/// The graph editor's window areas.
pub struct GraphEditor;

impl EditorAreas for GraphEditor {
    const VIEW: ViewKind = ViewKind::Graph;
    const SIDEBAR_TITLE: &'static str = "Properties";

    /// The canvas tools, then [`GraphTopbarDynamic`]: the parts that depend on the tab (design §5.2, §5.4).
    fn topbar() -> Option<impl Scene> {
        Some(bsn! {
            @super::bar_row()
            Node { column_gap: px(2), padding: UiRect::axes(px(6), px(2)) }
            Children [
                @icon_tool_button(IconId::AutoLayout, "Auto-layout", Action::AutoLayout, "Auto-layout (Ctrl+L)")
                --
                @icon_tool_button(IconId::Comment, "Comment", Action::AddComment(None), COMMENT_TIP)
                --
                @icon_tool_button(IconId::Fit, "Fit", Action::View(ViewRequest::FitAll), "Fit the graph (Home)")
                --
                GraphTopbarDynamic
                Node { flex_grow: 1.0, flex_direction: FlexDirection::Row, align_items: AlignItems::Center, column_gap: px(2) }
            ]
        })
    }

    /// Properties of the selection (a node, a comment frame, several nodes) or of the asset.
    fn sidebar() -> Option<impl Scene> {
        Some(bsn! { @super::sidebar_body() GraphSidebarBody })
    }

    fn build(app: &mut App) {
        app.add_systems(
            Update,
            (
                comment_at_view_centre.before(crate::apply_actions),
                redraw_graph_topbar.after(crate::apply_actions),
                redraw_sidebar,
            ),
        );
    }

    fn input(app: &mut App) {
        add_kind::<Self>(app);
        on_combo::<AutoLayout>(app, Action::AutoLayout);
        on_plain::<FitSelection>(app, Action::View(ViewRequest::FitSelection));
        on_plain::<FitAll>(app, Action::View(ViewRequest::FitAll));
        app.init_resource::<PreviewKey>()
            .init_resource::<CommentKey>()
            .add_observer(paste_at_cursor)
            .add_observer(comment_press)
            .add_observer(comment_at_cursor)
            .add_observer(open_palette)
            .add_observer(node_hotkey::<ConvertPlus>)
            .add_observer(node_hotkey::<ConvertMinus>)
            .add_observer(node_hotkey::<ConvertStar>)
            .add_observer(node_hotkey::<ConvertSlash>)
            .add_observer(preview_press)
            .add_observer(preview_release)
            .add_observer(cancel_wire);
    }
}

/// The graph editor's input context (UE-like, spec §9). Held-key + click node placement and mouse modifiers stay
/// with the canvas gestures (`ui::input`).
#[derive(Component, Default, Clone)]
pub struct GraphCtx;

impl EditorInput for GraphEditor {
    type Ctx = GraphCtx;

    fn bindings() -> impl Scene {
        use KeyCode::{
            Backspace, Delete, Digit8, Equal, Escape, Home, KeyA, KeyC, KeyD, KeyF, KeyL, KeyV, KeyX, Minus, NumpadAdd,
            NumpadDivide, NumpadMultiply, NumpadSubtract, Slash, Space,
        };
        bsn! {
            InputActions<GraphCtx> [
                @{hotkey::<EditCopy>(ctrl(KeyC))}
                --
                @{hotkey::<EditCut>(ctrl(KeyX))}
                --
                @{hotkey::<GraphPaste>(ctrl(KeyV))}
                --
                @{hotkey::<EditDuplicate>(ctrl(KeyD))}
                --
                @{hotkey::<EditSelectAll>(ctrl(KeyA))}
                --
                @{hotkey::<AutoLayout>(ctrl(KeyL))}
                --
                @{hotkey::<EditDelete>(plain([Delete, Backspace]))}
                --
                @{hotkey::<FitSelection>(plain([KeyF]))}
                --
                @{hotkey::<FitAll>(plain([Home]))}
                --
                @{held_hotkey::<AddComment>(plain([KeyC]), COMMENT_HOLD_SECS)}
                --
                @{hotkey::<OpenPalette>(plain([Space]))}
                --
                // Node action hotkeys (decision 0021): Shift + the main-row key, or the numpad key alone.
                @{hotkey::<ConvertPlus>([shift(Equal), NumpadAdd.into()])}
                --
                @{hotkey::<ConvertMinus>([shift(Minus), NumpadSubtract.into()])}
                --
                @{hotkey::<ConvertStar>([shift(Digit8), NumpadMultiply.into()])}
                --
                @{hotkey::<ConvertSlash>([shift(Slash), NumpadDivide.into()])}
                --
                @{hotkey::<PreviewNode>(plain([KeyV]))}
                --
                @{hotkey::<CancelWire>(plain([Escape]))}
            ]
        }
    }
}

/// Ctrl+V: paste at the cursor.
#[derive(InputAction)]
#[action_output(bool)]
pub struct GraphPaste;

#[derive(InputAction)]
#[action_output(bool)]
pub struct AutoLayout;

#[derive(InputAction)]
#[action_output(bool)]
pub struct FitSelection;

#[derive(InputAction)]
#[action_output(bool)]
pub struct FitAll;

/// Esc: drops the wire being dragged, or puts the wires a Ctrl+drag lifted back (design §5.3); nothing is edited.
#[derive(InputAction)]
#[action_output(bool)]
pub struct CancelWire;

/// Hold C: a comment frame at the cursor (around the selection), design §6.2.
#[derive(InputAction)]
#[action_output(bool)]
pub struct AddComment;

/// How long C is held before [`AddComment`] fires (design §6.2).
pub const COMMENT_HOLD_SECS: f32 = 0.5;

/// The Comment button's tooltip (design §6.2).
const COMMENT_TIP: &str = "Add a comment frame (hold C)";

/// Space: the node palette at the cursor (design §4).
#[derive(InputAction)]
#[action_output(bool)]
pub struct OpenPalette;

/// V: toggles the node preview on release (decision 0022), unless V + click placed a node meanwhile.
#[derive(InputAction)]
#[action_output(bool)]
pub struct PreviewNode;

/// A node action hotkey (decision 0021): runs the selected nodes' actions bound to `Shift + KEY`.
pub trait NodeHotkey: InputAction {
    const KEY: HotkeyKey;
}

macro_rules! node_hotkeys {
    ($($name:ident => $key:ident),* $(,)?) => {$(
        #[derive(InputAction)]
        #[action_output(bool)]
        pub struct $name;

        impl NodeHotkey for $name {
            const KEY: HotkeyKey = HotkeyKey::$key;
        }
    )*};
}

node_hotkeys!(ConvertPlus => Plus, ConvertMinus => Minus, ConvertStar => Star, ConvertSlash => Slash);

fn paste_at_cursor(_: On<Start<GraphPaste>>, cursor: CanvasCursor, mut actions: ResMut<Actions>) {
    actions.push(Action::Paste(cursor.graph_pos()));
}

/// Hold C (design §6.2): only C pressed with no Ctrl, Alt or Super arms the hold. A C still down when Ctrl is let go
/// (a Ctrl+C: the copy consumed the C until then) starts the action too, but does not arm it.
#[derive(Resource, Default)]
struct CommentKey {
    armed: bool,
}

fn comment_press(_: On<Start<AddComment>>, keys: Res<ButtonInput<KeyCode>>, mut key: ResMut<CommentKey>) {
    key.armed = keys.just_pressed(KeyCode::KeyC) && no_command_mods(&keys);
}

/// C held long enough: a comment at the cursor; with the cursor off the canvas `None`, the visible canvas centre
/// like the Comment button (controller ruling over design §6.1's origin).
fn comment_at_cursor(
    _: On<Fire<AddComment>>,
    keys: Res<ButtonInput<KeyCode>>,
    cursor: CanvasCursor,
    mut key: ResMut<CommentKey>,
    mut actions: ResMut<Actions>,
) {
    if std::mem::take(&mut key.armed) && no_command_mods(&keys) {
        actions.push(Action::AddComment(cursor.graph_pos()));
    }
}

/// The editor topbar's Comment button and hold C off the canvas push `AddComment(None)`: with no selection the frame
/// goes at the visible canvas centre (design §6.1), filled in here before the action is applied.
fn comment_at_view_centre(
    mut actions: ResMut<Actions>,
    ec: Option<Res<EditorCanvas>>,
    views: Query<(&CanvasView, &ComputedNode)>,
) {
    if !actions.0.iter().any(|a| matches!(a, Action::AddComment(None))) {
        return;
    }
    let Some(centre) = ec.and_then(|ec| views.get(ec.canvas).ok()).map(|(v, c)| view_center(v, c)) else { return };
    for a in &mut actions.0 {
        if let Action::AddComment(at @ None) = a {
            *at = Some((centre.x, centre.y));
        }
    }
}

/// Esc ends the canvas's wire gesture the way bevy_noodle's own cancel does: by despawning its dragged wire or
/// lifted edges (`MovingEdges`). Any modifier may be held (a Ctrl+drag holds Ctrl). While typing, the keyboard gate
/// hides Esc from this context, so a text field, the palette or a dialog keeps it.
fn cancel_wire(_: On<Start<CancelWire>>, ec: Option<Res<EditorCanvas>>, graph: GraphQuery, mut commands: Commands) {
    if let Some(wire) = ec.and_then(|ec| graph.wire_of(ec.canvas)) {
        commands.entity(wire).try_despawn();
    }
}

fn open_palette(
    _: On<Start<OpenPalette>>,
    keys: Res<ButtonInput<KeyCode>>,
    cursor: CanvasCursor,
    palette: Option<ResMut<Palette>>,
) {
    if let (true, Some((screen, graph)), Some(mut palette)) = (no_command_mods(&keys), cursor.at(), palette) {
        palette.show(screen, graph, None);
    }
}

fn node_hotkey<A: NodeHotkey>(
    _: On<Start<A>>,
    keys: Res<ButtonInput<KeyCode>>,
    editor: Res<Editor>,
    mut actions: ResMut<Actions>,
) {
    if no_command_mods(&keys) {
        for a in editor.hotkey_actions(Hotkey::shift(A::KEY)) {
            actions.push(a);
        }
    }
}

fn preview_press(_: On<Start<PreviewNode>>, keys: Res<ButtonInput<KeyCode>>, mut preview: ResMut<PreviewKey>) {
    if no_command_mods(&keys) {
        preview.press();
    }
}

/// Only a real release toggles: the action also completes when the context is deactivated or typing starts.
fn preview_release(
    _: On<Complete<PreviewNode>>,
    keys: Res<ButtonInput<KeyCode>>,
    editor: Res<Editor>,
    mut preview: ResMut<PreviewKey>,
    mut actions: ResMut<Actions>,
) {
    if preview.release() && keys.just_released(KeyCode::KeyV) {
        for a in editor.hotkey_actions(Hotkey::plain(HotkeyKey::Letter('v'))) {
            actions.push(a);
        }
    }
}

/// `V` toggles the node preview on release, unless `V` + click dropped a `param::Vector` meanwhile
/// (decision 0021). Initialised by [`GraphEditor::input`]; the canvas click (`ui::input`) marks a placement.
#[derive(Resource, Default)]
pub(crate) struct PreviewKey {
    held: bool,
    placed: bool,
}

impl PreviewKey {
    fn press(&mut self) {
        *self = Self { held: true, placed: false };
    }

    /// A node was placed with `V` held: this release does not toggle.
    pub(crate) fn placed(&mut self) {
        self.placed = self.held;
    }

    /// True when the release toggles the preview.
    fn release(&mut self) -> bool {
        let toggle = self.held && !self.placed;
        *self = Self::default();
        toggle
    }
}

/// The graph editor topbar's part that depends on the tab, refilled by [`redraw_graph_topbar`]: the Preview mode
/// group (materials, design §5.4) and, right-aligned, the "Show generated" buttons (design §5.2).
#[derive(Component, Default, Clone)]
pub struct GraphTopbarDynamic;

/// What [`GraphTopbarDynamic`] shows: the active tab, its asset (the buttons' target), whether that is a material,
/// the stored preview mode and whether a node is previewed.
type TopbarKey = (Option<usize>, AssetId, bool, PreviewMode, bool);

fn redraw_graph_topbar(
    editor: Res<Editor>,
    bar: Query<Entity, With<GraphTopbarDynamic>>,
    mut last: Local<Option<TopbarKey>>,
    mut commands: Commands,
) {
    let Some(d) = editor.doc() else { return *last = None };
    let preview = &d.asset.preview;
    // A dangling id (read from a file) previews nothing (W107).
    let previewing = preview.node.as_ref().is_some_and(|n| d.asset.nodes.contains_key(n));
    let material = matches!(d.asset.kind, AssetKind::Material(_));
    let key = (editor.active, d.id.clone(), material, preview.mode, previewing);
    if last.as_ref() == Some(&key) {
        return;
    }
    let Ok(bar) = bar.single() else { return };
    commands.entity(bar).despawn_children();
    for item in topbar_items(&d.id, material, preview.mode, previewing) {
        commands.spawn_scene(bsn! { @{item} ChildOf(bar) });
    }
    *last = Some(key);
}

/// The action of the topbar's Preview `mode` button: a `PropTarget::PreviewMode` edit (one undo step).
pub fn preview_mode_action(mode: PreviewMode) -> Action {
    Action::SetValue {
        target: PropTarget::PreviewMode,
        value: Value::Str(preview_mode_label(mode).into()),
        merge: false,
    }
}

fn preview_mode_icon(mode: PreviewMode) -> IconId {
    match mode {
        PreviewMode::Unlit => IconId::Unlit,
        PreviewMode::Lit => IconId::Pbr,
        PreviewMode::Full => IconId::Layers,
    }
}

/// The hover hint of a Preview mode button: it names the mode even while the group is disabled (design §5.4; the UI
/// font has no em dash).
fn preview_mode_tip(mode: PreviewMode, previewing: bool) -> String {
    let label = preview_mode_label(mode);
    if previewing { format!("Preview: {label}") } else { format!("Preview: {label} - preview a node first (V)") }
}

/// [`GraphTopbarDynamic`]'s items. The Preview mode group stays disabled, showing the stored mode, while no node is
/// previewed.
fn topbar_items(id: &AssetId, material: bool, mode: PreviewMode, previewing: bool) -> Vec<Box<dyn Scene>> {
    let mut out: Vec<Box<dyn Scene>> = Vec::new();
    if material {
        let buttons = PreviewMode::ALL.map(|m| GroupButton {
            caption: Caption::Icon(preview_mode_icon(m)),
            action: preview_mode_action(m),
            selected: m == mode,
            disabled: !previewing,
            tip: preview_mode_tip(m, previewing),
        });
        let buttons: Vec<GroupButton> = buttons.into();
        out.push(Box::new(bsn! { @button_group(buttons) Node { margin: UiRect::left(px(8)) } }));
    }
    out.push(Box::new(flex_spacer()));
    let wesl = Action::ShowGenerated(id.clone(), Lang::Wesl);
    out.push(Box::new(icon_button(IconId::Code, wesl, "Show generated WESL")));
    if material {
        let rust = Action::ShowGenerated(id.clone(), Lang::Rust);
        out.push(Box::new(icon_button(IconId::Rust, rust, "Show generated Rust")));
    }
    out
}

/// The graph editor's right sidebar body ("Properties").
#[derive(Component, Default, Clone)]
struct GraphSidebarBody;

fn redraw_sidebar(
    editor: Res<Editor>,
    body: Query<Entity, With<GraphSidebarBody>>,
    mut key: Local<SidebarKey>,
    mut commands: Commands,
) {
    let Some(d) = editor.doc() else { return key.forget() };
    fill_sidebar_with(&mut commands, &body, &mut key, SidebarKey::now(&editor), || sidebar_rows(&editor, d));
}

/// A Float edited in the sidebar gets the soft limit of its inline field on the node (design §3).
fn ranged(kind: &ParamKind, on_node: Option<Inline>) -> ParamKind {
    match (kind, on_node) {
        (ParamKind::Float { .. }, Some(Inline::Ranged { min, max })) => {
            ParamKind::Float { min: Some(min), max: Some(max) }
        }
        _ => kind.clone(),
    }
}

/// The selected comment frame, the one selected node, several nodes, or (no selection) the asset: one subpane per
/// kind of content.
fn sidebar_rows(editor: &Editor, d: &Doc) -> Vec<Box<dyn Scene>> {
    if let Some(f) = d.selected_frame.as_ref().and_then(|f| d.asset.comments.iter().find(|c| c.id == *f)) {
        let mut rows: Vec<Box<dyn Scene>> = Vec::new();
        rows.push(Box::new(labeled(
            "Title".into(),
            text_w(PropTarget::FrameTitle(f.id.clone()), f.title.clone(), false),
        )));
        // Unset: the field starts at the default colour's hue (design §4.6).
        let hue = f.hue.unwrap_or_else(|| Hsla::from(frame_color(f)).hue);
        rows.push(Box::new(labeled("Hue".into(), hue_input(PropTarget::FrameHue(f.id.clone()), hue))));
        let reset = Action::SetValue {
            target: PropTarget::FrameHue(f.id.clone()),
            value: Value::Str(String::new()),
            merge: false,
        };
        rows.push(Box::new(labeled(String::new(), Box::new(button("Default colour", reset, false)))));
        vec![subpane_box("Comment", rows)]
    } else if d.selection.len() == 1 {
        let id = d.selection.iter().next().cloned().unwrap_or_default();
        node_subpanes(editor, &id)
    } else if d.selection.len() > 1 {
        let selected = format!("{} selected", plural(d.selection.len(), "node", "nodes"));
        let rows: Vec<Box<dyn Scene>> = vec![
            Box::new(text(selected, TEXT, 14.0)),
            Box::new(text("Hold C: comment around them | Auto-layout arranges them.", TEXT_DIM, 12.0)),
        ];
        vec![subpane_box("Selection", rows)]
    } else {
        asset_subpanes(editor, d)
    }
}

/// Hotkey hints of the graph canvas (decision 0032), shown with no selection.
const GRAPH_HOTKEYS: [&str; 4] = [
    "Space or right-click on the canvas opens the node palette.",
    "Add a comment frame (hold C).",
    "Alt+click a pin or a wire: break its wires. Ctrl+drag from a pin onto another pin of the same side: move its wires.",
    "Hold 1-4, S, V, T, M, A, L, U, R, D, O, E, N, P or I and click the canvas: drop that node.",
];

/// No selection: the asset's settings (material, preview), the canvas hotkeys and its description.
fn asset_subpanes(editor: &Editor, d: &Doc) -> Vec<Box<dyn Scene>> {
    let mut out = Vec::new();
    let mut asset: Vec<Box<dyn Scene>> = Vec::new();
    asset.push(Box::new(text(d.id.source_rel(), TEXT, 15.0)));
    let kind = match d.asset.kind {
        AssetKind::Function => "Function",
        AssetKind::Material(m) => m.label(),
    };
    asset.push(Box::new(text(format!("Type: {kind}"), TEXT_DIM, 12.0)));
    let AssetKind::Material(m) = d.asset.kind else {
        out.push(subpane_box("Function", asset));
        return common_subpanes(d, out);
    };
    let blends = [BlendMode::Opaque, BlendMode::Mask, BlendMode::Blend, BlendMode::Add, BlendMode::Multiply];
    let labels: Vec<String> = blends.iter().map(|b| b.label().to_string()).collect();
    let cur = blends.iter().position(|b| *b == m.blend()).unwrap_or(0);
    asset.push(Box::new(labeled("Blend mode".into(), select_w(PropTarget::Blend, labels, Vec::new(), cur))));
    let two_sided = match m {
        MaterialKind::Pbr3d(s) => Some(s.two_sided),
        MaterialKind::Unlit3d(s) => Some(s.two_sided),
        MaterialKind::Sprite2d(_) => None,
    };
    if let Some(t) = two_sided {
        asset.push(checkbox_row("Two sided", PropTarget::TwoSided, t));
    }
    if let MaterialKind::Pbr3d(s) = m {
        let cur = usize::from(s.shading == Shading::Unlit);
        asset.push(Box::new(labeled(
            "Shading".into(),
            select_w(PropTarget::Shading, vec!["Lit".into(), "Unlit".into()], Vec::new(), cur),
        )));
        // Decision 0023: one `Material Attributes` input (+ WPO) instead of one pin per attribute.
        let modes: Vec<String> = PbrOutputMode::ALL.iter().map(|m| m.label().to_string()).collect();
        let cur = PbrOutputMode::ALL.iter().position(|m| *m == s.output).unwrap_or(0);
        asset.push(Box::new(labeled("Output".into(), select_w(PropTarget::OutputMode, modes, Vec::new(), cur))));
    }
    out.push(subpane_box("Material", asset));
    // Exclusive node preview (decision 0022); its mode is the editor topbar's group (design §5.4).
    let status = match &d.asset.preview.node {
        Some(n) if d.asset.nodes.contains_key(n) => {
            format!("Previewing: {}", crate::canvas::node_title(editor, &d.asset, n))
        }
        Some(n) => format!("Previewed node `{n}` does not exist (ignored)"),
        // ASCII dash: the UI font has no em dash glyph.
        None => "No node previewed - select a node and press V".to_string(),
    };
    out.push(subpane_box("Preview", vec![Box::new(text(status, TEXT_DIM, 12.0))]));
    common_subpanes(d, out)
}

/// The subpanes every graph asset shows after its own: canvas hotkeys, then the description last (design §5; the
/// counts are in the editor statusbar).
fn common_subpanes(d: &Doc, mut out: Vec<Box<dyn Scene>>) -> Vec<Box<dyn Scene>> {
    let hotkeys = GRAPH_HOTKEYS.map(|tip| -> Box<dyn Scene> { Box::new(text(tip, TEXT_DIM, 12.0)) });
    out.push(subpane_box("Hotkeys", hotkeys.into()));
    out.push(subpane_box(
        "Description",
        vec![text_w(PropTarget::Description, d.asset.meta.description.clone(), false)],
    ));
    out
}

/// The slots a node's `GradientSlot` param can pick (gradient design §6): the non-empty slots of the collection its
/// `GradientCollection` param names (an open tab's unsaved state first), plus the picked slot when it is empty (so the
/// list still shows the pick). Nothing without a loadable collection.
fn gradient_slots(editor: &Editor, defs: &[ParamDef], params: &BTreeMap<String, Value>) -> GradientSlots {
    let value = |kind: ParamKind| defs.iter().find(|p| p.kind == kind).and_then(|p| params.get(&p.id));
    let collection = value(ParamKind::GradientCollection).map(Value::as_str).unwrap_or_default();
    let picked = value(ParamKind::GradientSlot).map_or(0, Value::as_i32);
    let Some(c) = editor.gradient_collection(collection) else { return GradientSlots::default() };
    let slots = c.slots.iter().enumerate().filter(|(_, s)| !s.is_empty() || i64::from(s.id) == i64::from(picked));
    GradientSlots {
        collection: collection.to_string(),
        slots: slots.map(|(pos, s)| SlotEntry { pos, id: s.id, name: s.name.clone() }).collect(),
    }
}

/// One selected node: what it is (title, kind, label, the asset it refers to), its parameters, input defaults and
/// problems.
fn node_subpanes(editor: &Editor, id: &str) -> Vec<Box<dyn Scene>> {
    let mut out = Vec::new();
    let Some(d) = editor.doc() else { return out };
    let Some(n) = d.asset.nodes.get(id) else { return out };
    let Some(def) = editor.registry.get(&n.kind) else {
        let unknown: Box<dyn Scene> = Box::new(text(format!("Unknown node kind {}", n.kind), TEXT, 13.0));
        out.push(subpane_box("Node", vec![unknown]));
        return out;
    };
    let info = def.info();
    let mut node: Vec<Box<dyn Scene>> = Vec::new();
    node.push(Box::new(text(crate::canvas::node_title(editor, &d.asset, id), TEXT, 15.0)));
    node.push(Box::new(text(format!("{} | {}", info.category, n.kind), TEXT_DIM, 11.0)));
    if !info.description.is_empty() {
        node.push(Box::new(text(info.description.clone(), TEXT_DIM, 12.0)));
    }
    if crate::canvas::has_label(&n.kind) {
        let target = PropTarget::NodeLabel(id.to_string());
        node.push(Box::new(labeled("Label".to_string(), text_w(target, n.label.clone(), false))));
    }
    // Editor-only, shown above the node on the canvas.
    node.push(Box::new(text("Comment", TEXT, 13.0)));
    let comment = PropTarget::NodeComment(id.to_string());
    node.push(text_area(comment, n.comment.clone(), shader_graph_core::model::Node::COMMENT_MAX));
    // The file the node refers to; double-clicking the node opens it too (decision 0023).
    if let Some(target) = editor.referenced_asset(id) {
        let label = match target.ty {
            AssetType::Function => "Open function",
            AssetType::Curve => "Open curve",
            AssetType::Consts => "Open constant table",
            AssetType::Material => "Open material",
            AssetType::Texture => "Open texture",
            AssetType::Gradients => "Open gradients",
        };
        node.push(Box::new(button(label, Action::OpenAsset(target), false)));
    }
    out.push(subpane_box("Node", node));
    let params = editor.registry.resolved_params(&n.kind, &n.params);
    let choices = Choices {
        routes: route_declarations(&d.asset, &editor.registry),
        curves: editor.curve_paths().into_iter().map(|p| (p.clone(), p)).collect(),
        consts: Vec::new(),
        textures: editor.texture_choices(),
        gradients: editor.gradient_collections(),
        slots: gradient_slots(editor, &def.params(), &params),
    };
    let global_consts = editor.global_consts();
    let inline = inline_for_param(&n.kind, &params);
    let mut rows: Vec<Box<dyn Scene>> = Vec::new();
    // Params that only apply to other settings (FunctionInput `min` / `max` of a non-Float input) are hidden.
    for p in def.params().iter().filter(|p| p.visible(&params)) {
        let v = params.get(&p.id).cloned().unwrap_or_else(|| p.default.clone());
        let target = PropTarget::Param { node: id.to_string(), key: p.id.clone() };
        // A deleted or broken curve stays visible as the current choice (the node reports E006).
        let mut choices_here = None;
        if p.kind == ParamKind::Curve && !v.as_str().is_empty() && !choices.curves.iter().any(|c| c.0 == v.as_str()) {
            let mut c = choices.clone();
            c.curves.push((v.as_str().to_string(), format!("{} (missing)", v.as_str())));
            choices_here = Some(c);
        }
        // Constants of the node's type; a missing one or one of another type stays visible (E006 / E003).
        if let ParamKind::GlobalConst(ty) = p.kind {
            let mut c = choices.clone();
            c.consts = global_consts.iter().filter(|k| k.1.1.ty == ty).map(|k| (k.0.clone(), k.0.clone())).collect();
            if !v.as_str().is_empty() && !c.consts.iter().any(|k| k.0 == v.as_str()) {
                let why = global_consts.get(v.as_str()).map_or("missing".into(), |k| format!("is {}", k.1.ty.label()));
                c.consts.push((v.as_str().to_string(), format!("{} ({why})", v.as_str())));
            }
            choices_here = Some(c);
        }
        // A deleted or broken collection stays visible as the current choice (the node reports E006).
        if p.kind == ParamKind::GradientCollection
            && !v.as_str().is_empty()
            && !choices.gradients.iter().any(|c| c.0 == v.as_str())
        {
            let mut c = choices.clone();
            c.gradients.push((v.as_str().to_string(), format!("{} (missing)", v.as_str())));
            choices_here = Some(c);
        }
        // A deleted Texture asset stays visible as the current choice (the node reports E006).
        if p.kind == ParamKind::Texture && !v.as_str().is_empty() && !choices.textures.iter().any(|c| c.0 == v.as_str())
        {
            let mut c = choices.clone();
            c.textures.push((v.as_str().to_string(), format!("{} (missing)", v.as_str())));
            choices_here = Some(c);
        }
        let choices = choices_here.as_ref().unwrap_or(&choices);
        let on_node = inline.as_ref().filter(|i| i.0 == p.id).map(|i| i.1);
        rows.push(value_row(p.label.clone(), target, &ranged(&p.kind, on_node), &v, choices));
    }
    if !rows.is_empty() {
        out.push(subpane_box("Parameters", rows));
    }
    if let Some(sig) = node_signature(&d.asset, id, &editor.registry, &editor.functions, &d.id.path) {
        let mut rows: Vec<Box<dyn Scene>> = Vec::new();
        // A usage's input is the declaration's, never a value of its own.
        let inputs = if def.role() == NodeRole::RouteUsage { &[][..] } else { &sig.inputs[..] };
        for pin in inputs {
            if d.asset.edge_into(id, &pin.id).is_some() || pin.implicit.is_some() {
                continue;
            }
            let Some(kind) = pin_kind(pin.ty) else { continue };
            let kind = ranged(&kind, inline_for_pin(pin.ty, pin.range));
            let v = n.inputs.get(&pin.id).cloned().unwrap_or_else(|| pin.default.coerce(pin.ty));
            let target = PropTarget::Input { node: id.to_string(), pin: pin.id.clone() };
            let label = if pin.label.is_empty() { pin.id.clone() } else { pin.label.clone() };
            rows.push(value_row(label, target, &kind, &v, &Choices::default()));
        }
        if !rows.is_empty() {
            rows.insert(0, Box::new(text("Used when not connected.", TEXT_DIM, 12.0)));
            out.push(subpane_box("Input defaults", rows));
        }
    }
    if let Some(list) = editor.node_diags().get(id) {
        let rows = list.iter().map(|diag| -> Box<dyn Scene> {
            let c = if diag.is_error() { Color::srgb(0.95, 0.42, 0.42) } else { Color::srgb(0.95, 0.78, 0.35) };
            Box::new(text(format!("{}: {}", diag.code, diag.message), c, 12.0))
        });
        out.push(subpane_box("Problems", rows.collect()));
    }
    out
}

impl TabDoc for Doc {
    fn asset(&self) -> Option<&AssetId> {
        Some(&self.id)
    }

    fn view(&self) -> ViewKind {
        ViewKind::Graph
    }

    fn base_title(&self) -> String {
        asset_title(&self.id)
    }

    fn icon(&self) -> IconId {
        IconId::for_asset(Some(self.asset.kind))
    }

    fn watermark(&self) -> Option<&'static str> {
        Some(match self.asset.kind {
            AssetKind::Function => "function",
            AssetKind::Material(MaterialKind::Pbr3d(_)) => "pbr3d",
            AssetKind::Material(MaterialKind::Unlit3d(_)) => "unlit3d",
            AssetKind::Material(MaterialKind::Sprite2d(_)) => "sprite2d",
        })
    }

    fn is_dirty(&self) -> bool {
        Doc::is_dirty(self)
    }

    fn is_read_only(&self) -> bool {
        false
    }

    fn revision(&self) -> u64 {
        self.revision
    }

    fn save(&mut self, project: &Project) -> std::io::Result<()> {
        ops::save(project, &self.id, &self.asset).map_err(io_error)?;
        self.mark_saved();
        Ok(())
    }

    fn changed_on_disk(&self, project: &Project) -> bool {
        assets::load(project, &self.id).is_ok_and(|a| Asset::from_ron(&self.saved).ok() != Some(a))
    }

    fn reload_from_disk(&mut self, project: &Project) -> Result<(), String> {
        self.reload(assets::load(project, &self.id)?);
        Ok(())
    }

    fn keep_mine(&mut self, project: &Project) {
        if let Ok(on_disk) = assets::load(project, &self.id) {
            self.saved = on_disk.to_ron();
        }
    }

    fn rename(&mut self, _project: &Project, from: &AssetId, to: &AssetId) {
        if self.id != *from {
            return;
        }
        self.id = to.clone();
        self.asset.name = to.name().to_string();
        self.mark_saved();
        self.revision += 1;
    }

    fn config_key(&self) -> Option<String> {
        Some(self.id.source_rel())
    }

    fn override_into(&self, set: &mut AssetSet) {
        set.assets.insert(self.id.clone(), Ok(self.asset.clone()));
    }

    fn status(&self, ctx: &StatusCtx) -> Option<StatusInfo> {
        let mut counts = vec![
            plural(self.asset.nodes.len(), "node", "nodes"),
            plural(self.asset.edges.len(), "wire", "wires"),
            plural(self.asset.comments.len(), "comment", "comments"),
        ];
        counts.extend(ctx.wesl_lines.map(|n| plural(n, "WESL line", "WESL lines")));
        Some(StatusInfo::new(Doc::is_dirty(self), ctx, counts))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_key_toggles_on_release_unless_a_node_was_placed() {
        let mut k = PreviewKey::default();
        k.press();
        assert!(k.release());
        k.press();
        k.placed();
        assert!(!k.release());
        assert!(!k.release(), "release without press");
    }

    #[test]
    fn disabled_preview_mode_tips_still_name_their_mode() {
        assert_eq!(preview_mode_tip(PreviewMode::Lit, true), "Preview: Lit");
        assert_eq!(preview_mode_tip(PreviewMode::Lit, false), "Preview: Lit - preview a node first (V)");
        assert_eq!(preview_mode_tip(PreviewMode::Full, false), "Preview: Full - preview a node first (V)");
    }
}
