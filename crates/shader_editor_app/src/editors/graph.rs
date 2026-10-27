//! Graph editor kind: node graph of a material or function on the bevy_noodle canvas ([`Doc`]).

use bevy::prelude::*;
use bevy_enhanced_input::prelude::{Actions as InputActions, Complete, InputAction, Start};
use shader_graph_core::model::{Asset, AssetKind, BlendMode, MaterialKind, PbrOutputMode, PreviewMode, Shading};
use shader_graph_core::registry::{NodeRole, ParamKind};
use shader_graph_core::{Hotkey, HotkeyKey, Value};
use shader_project::{AssetId, AssetSet, AssetType, Project, assets, ops};

use super::main_input::{
    EditCopy, EditCut, EditDelete, EditDuplicate, EditSelectAll, add_kind, ctrl, hotkey, no_command_mods, on_combo,
    on_plain, plain, shift,
};
use super::{EditorAreas, EditorInput, SidebarKey, StatusCtx, StatusInfo, fill_sidebar};
use crate::canvas::frame_color;
use crate::doc::{Doc, node_signature, route_declarations};
use crate::doc_kind::{TabDoc, asset_title, io_error};
use crate::state::{Action, Actions, Editor, PropTarget, ViewRequest, preview_mode_label};
use crate::tab::ViewKind;
use crate::ui::icons::IconId;
use crate::ui::input::{CanvasCursor, PreviewKey};
use crate::ui::palette::Palette;
use crate::ui::props::{Choices, checkbox_w, hue_slider, labeled, pin_kind, section, select_w, text_w, value_widget};
use crate::ui::{TEXT, TEXT_DIM, button, icon_tool_button, text};

/// The graph editor's window areas.
pub struct GraphEditor;

impl EditorAreas for GraphEditor {
    const VIEW: ViewKind = ViewKind::Graph;
    const SIDEBAR_TITLE: &'static str = "Properties";

    fn topbar() -> Option<impl Scene> {
        Some(bsn! {
            @super::bar_row()
            Node { column_gap: px(2), padding: UiRect::axes(px(6), px(2)) }
            Children [
                @icon_tool_button(IconId::AutoLayout, "Auto-layout", Action::AutoLayout, "Auto-layout (Ctrl+L)")
                --
                @icon_tool_button(IconId::Comment, "Comment", Action::AddComment(None), "Add a comment frame (C)")
                --
                @icon_tool_button(IconId::Fit, "Fit", Action::View(ViewRequest::FitAll), "Fit the graph (Home)")
            ]
        })
    }

    /// Properties of the selection (a node, a comment frame, several nodes) or of the asset.
    fn sidebar() -> Option<impl Scene> {
        Some(bsn! { @super::sidebar_body() GraphSidebarBody })
    }

    fn build(app: &mut App) {
        app.add_systems(Update, redraw_sidebar);
    }

    fn input(app: &mut App) {
        add_kind::<Self>(app);
        on_combo::<AutoLayout>(app, Action::AutoLayout);
        on_plain::<FitSelection>(app, Action::View(ViewRequest::FitSelection));
        on_plain::<FitAll>(app, Action::View(ViewRequest::FitAll));
        app.init_resource::<PreviewKey>()
            .add_observer(paste_at_cursor)
            .add_observer(comment_at_cursor)
            .add_observer(open_palette)
            .add_observer(node_hotkey::<ConvertPlus>)
            .add_observer(node_hotkey::<ConvertMinus>)
            .add_observer(node_hotkey::<ConvertStar>)
            .add_observer(node_hotkey::<ConvertSlash>)
            .add_observer(preview_press)
            .add_observer(preview_release);
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
            Backspace, Delete, Digit8, Equal, Home, KeyA, KeyC, KeyD, KeyF, KeyL, KeyV, KeyX, Minus, NumpadAdd,
            NumpadDivide, NumpadMultiply, NumpadSubtract, Slash, Tab,
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
                @{hotkey::<AddComment>(plain([KeyC]))}
                --
                @{hotkey::<OpenPalette>(plain([Tab]))}
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

/// C: a comment frame at the cursor (around the selection).
#[derive(InputAction)]
#[action_output(bool)]
pub struct AddComment;

/// Tab: the node palette at the cursor.
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

fn comment_at_cursor(
    _: On<Start<AddComment>>,
    keys: Res<ButtonInput<KeyCode>>,
    cursor: CanvasCursor,
    mut actions: ResMut<Actions>,
) {
    if no_command_mods(&keys) {
        actions.push(Action::AddComment(cursor.graph_pos()));
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

/// The graph editor's right sidebar body ("Properties").
#[derive(Component, Default, Clone)]
pub struct GraphSidebarBody;

fn redraw_sidebar(
    editor: Res<Editor>,
    body: Query<Entity, With<GraphSidebarBody>>,
    mut key: Local<SidebarKey>,
    mut commands: Commands,
) {
    let Some(d) = editor.doc() else { return key.forget() };
    let Some(body) = key.changed(&editor, &body) else { return };
    let mut rows: Vec<Box<dyn Scene>> = Vec::new();
    if let Some(f) = d.selected_frame.as_ref().and_then(|f| d.asset.comments.iter().find(|c| c.id == *f)) {
        rows.push(Box::new(text("Comment", TEXT, 15.0)));
        rows.push(Box::new(labeled(
            "Title".into(),
            text_w(PropTarget::FrameTitle(f.id.clone()), f.title.clone(), false),
        )));
        // Unset: the slider starts at the default colour's hue (design §4.6).
        let hue = f.hue.unwrap_or_else(|| Hsla::from(frame_color(f)).hue);
        rows.push(Box::new(labeled("Hue".into(), hue_slider(PropTarget::FrameHue(f.id.clone()), hue))));
        let reset = Action::SetValue {
            target: PropTarget::FrameHue(f.id.clone()),
            value: Value::Str(String::new()),
            merge: false,
        };
        rows.push(Box::new(labeled(String::new(), Box::new(button("Default colour", reset, false)))));
    } else if d.selection.len() == 1 {
        let id = d.selection.iter().next().cloned().unwrap_or_default();
        node_rows(&editor, &id, &mut rows);
    } else if d.selection.len() > 1 {
        rows.push(Box::new(text(format!("{} nodes selected", d.selection.len()), TEXT, 14.0)));
        rows.push(Box::new(text("C: comment around them | Auto-layout arranges them.", TEXT_DIM, 12.0)));
    } else {
        asset_rows(&editor, &mut rows);
    }
    fill_sidebar(&mut commands, body, rows);
}

/// No selection: the asset's settings (material, preview, description) and size.
fn asset_rows(editor: &Editor, out: &mut Vec<Box<dyn Scene>>) {
    let Some(d) = editor.doc() else { return };
    out.push(Box::new(text(d.id.source_rel(), TEXT, 15.0)));
    out.push(Box::new(text(
        format!(
            "Type: {}",
            match d.asset.kind {
                AssetKind::Function => "Function",
                AssetKind::Material(m) => m.label(),
            }
        ),
        TEXT_DIM,
        12.0,
    )));
    if let AssetKind::Material(m) = d.asset.kind {
        out.push(Box::new(section("Material")));
        let blends = [BlendMode::Opaque, BlendMode::Mask, BlendMode::Blend, BlendMode::Add, BlendMode::Multiply];
        let labels: Vec<String> = blends.iter().map(|b| b.label().to_string()).collect();
        let cur = blends.iter().position(|b| *b == m.blend()).unwrap_or(0);
        out.push(Box::new(labeled("Blend mode".into(), select_w(PropTarget::Blend, labels, Vec::new(), cur))));
        let two_sided = match m {
            MaterialKind::Pbr3d(s) => Some(s.two_sided),
            MaterialKind::Unlit3d(s) => Some(s.two_sided),
            MaterialKind::Sprite2d(_) => None,
        };
        if let Some(t) = two_sided {
            out.push(Box::new(labeled("Two sided".into(), checkbox_w(PropTarget::TwoSided, t))));
        }
        if let MaterialKind::Pbr3d(s) = m {
            let cur = usize::from(s.shading == Shading::Unlit);
            out.push(Box::new(labeled(
                "Shading".into(),
                select_w(PropTarget::Shading, vec!["Lit".into(), "Unlit".into()], Vec::new(), cur),
            )));
            // Decision 0023: one `Material Attributes` input (+ WPO) instead of one pin per attribute.
            let modes: Vec<String> = PbrOutputMode::ALL.iter().map(|m| m.label().to_string()).collect();
            let cur = PbrOutputMode::ALL.iter().position(|m| *m == s.output).unwrap_or(0);
            out.push(Box::new(labeled("Output".into(), select_w(PropTarget::OutputMode, modes, Vec::new(), cur))));
        }
        // Exclusive node preview (decision 0022): the mode is kept even while no node is previewed.
        out.push(Box::new(section("Preview")));
        let modes: Vec<String> = PreviewMode::ALL.iter().map(|m| preview_mode_label(*m).to_string()).collect();
        let cur = PreviewMode::ALL.iter().position(|m| *m == d.asset.preview.mode).unwrap_or(0);
        out.push(Box::new(labeled("Mode".into(), select_w(PropTarget::PreviewMode, modes, Vec::new(), cur))));
        let status = match &d.asset.preview.node {
            Some(n) if d.asset.nodes.contains_key(n) => {
                format!("Previewing: {}", crate::canvas::node_title(editor, &d.asset, n))
            }
            Some(n) => format!("Previewed node `{n}` does not exist (ignored)"),
            // ASCII dash: the UI font has no em dash glyph.
            None => "No node previewed - select a node and press V".to_string(),
        };
        out.push(Box::new(text(status, TEXT_DIM, 12.0)));
    }
    out.push(Box::new(section("Description")));
    out.push(Box::new(text_w(PropTarget::Description, d.asset.meta.description.clone(), false)));
    out.push(Box::new(section("Graph")));
    out.push(Box::new(text(
        format!(
            "{} node(s), {} wire(s), {} comment(s)",
            d.asset.nodes.len(),
            d.asset.edges.len(),
            d.asset.comments.len()
        ),
        TEXT_DIM,
        12.0,
    )));
    // Hotkey hints (decision 0032).
    for tip in [
        "Tip: Tab or right-click on the canvas opens the node palette.",
        "Alt+click a pin or a wire: break its wires. Ctrl+drag from a pin onto another pin of the same side: move its wires.",
        "Hold 1-4, S, V, T, M, A, L, U, R, D, O, E, N, P or I and click the canvas: drop that node.",
    ] {
        out.push(Box::new(text(tip, TEXT_DIM, 12.0)));
    }
}

/// One selected node: title, parameters, the asset it refers to, input defaults and problems.
fn node_rows(editor: &Editor, id: &str, out: &mut Vec<Box<dyn Scene>>) {
    let Some(d) = editor.doc() else { return };
    let Some(n) = d.asset.nodes.get(id) else { return };
    let Some(def) = editor.registry.get(&n.kind) else {
        out.push(Box::new(text(format!("Unknown node kind {}", n.kind), TEXT, 13.0)));
        return;
    };
    let info = def.info();
    out.push(Box::new(text(crate::canvas::node_title(editor, &d.asset, id), TEXT, 15.0)));
    out.push(Box::new(text(format!("{} | {}", info.category, n.kind), TEXT_DIM, 11.0)));
    if !info.description.is_empty() {
        out.push(Box::new(text(info.description.clone(), TEXT_DIM, 12.0)));
    }
    let params = editor.registry.resolved_params(&n.kind, &n.params);
    let choices = Choices {
        routes: route_declarations(&d.asset, &editor.registry),
        curves: editor.curve_paths().into_iter().map(|p| (p.clone(), p)).collect(),
        consts: Vec::new(),
        textures: editor.texture_choices(),
    };
    let global_consts = editor.global_consts();
    let defs = def.params();
    if crate::canvas::has_label(&n.kind) {
        let target = PropTarget::NodeLabel(id.to_string());
        out.push(Box::new(labeled("Label".to_string(), text_w(target, n.label.clone(), false))));
    }
    if !defs.is_empty() {
        out.push(Box::new(section("Parameters")));
    }
    // Params that only apply to other settings (FunctionInput `min` / `max` of a non-Float input) are hidden.
    for p in defs.iter().filter(|p| p.visible(&params)) {
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
        // A deleted Texture asset stays visible as the current choice (the node reports E006).
        if p.kind == ParamKind::Texture && !v.as_str().is_empty() && !choices.textures.iter().any(|c| c.0 == v.as_str())
        {
            let mut c = choices.clone();
            c.textures.push((v.as_str().to_string(), format!("{} (missing)", v.as_str())));
            choices_here = Some(c);
        }
        let choices = choices_here.as_ref().unwrap_or(&choices);
        out.push(Box::new(labeled(p.label.clone(), value_widget(target, &p.kind, &v, choices))));
    }
    // The file the node refers to; double-clicking the node opens it too (decision 0023).
    if let Some(target) = editor.referenced_asset(id) {
        let label = match target.ty {
            AssetType::Function => "Open function",
            AssetType::Curve => "Open curve",
            AssetType::Consts => "Open constant table",
            AssetType::Material => "Open material",
            AssetType::Texture => "Open texture",
        };
        out.push(Box::new(button(label, Action::OpenAsset(target), false)));
    }
    let Some(sig) = node_signature(&d.asset, id, &editor.registry, &editor.functions, &d.id.path) else { return };
    let mut header = false;
    // A usage's input is the declaration's, never a value of its own.
    let inputs = if def.role() == NodeRole::RouteUsage { &[][..] } else { &sig.inputs[..] };
    for pin in inputs {
        if d.asset.edge_into(id, &pin.id).is_some() || pin.implicit.is_some() {
            continue;
        }
        let Some(kind) = pin_kind(pin.ty) else { continue };
        if !header {
            out.push(Box::new(section("Input defaults (used when not connected)")));
            header = true;
        }
        let v = n.inputs.get(&pin.id).cloned().unwrap_or_else(|| pin.default.coerce(pin.ty));
        let target = PropTarget::Input { node: id.to_string(), pin: pin.id.clone() };
        let label = if pin.label.is_empty() { pin.id.clone() } else { pin.label.clone() };
        out.push(Box::new(labeled(label, value_widget(target, &kind, &v, &Choices::default()))));
    }
    let diags = editor.node_diags();
    if let Some(list) = diags.get(id) {
        out.push(Box::new(section("Problems")));
        for diag in list {
            let c = if diag.is_error() { Color::srgb(0.95, 0.42, 0.42) } else { Color::srgb(0.95, 0.78, 0.35) };
            out.push(Box::new(text(format!("{}: {}", diag.code, diag.message), c, 12.0)));
        }
    }
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
        let mut counts = vec![format!("{} nodes", self.asset.nodes.len())];
        counts.extend(ctx.wesl_lines.map(|n| format!("{n} WESL lines")));
        Some(StatusInfo::new(Doc::is_dirty(self), ctx, counts))
    }
}
