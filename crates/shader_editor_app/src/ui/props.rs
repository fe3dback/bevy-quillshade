//! Properties panel (spec §9.4): node parameters and input defaults, asset settings, frame titles.

use bevy::feathers::controls::{
    ColorSwatchValue, FeathersCheckbox, FeathersColorSwatch, FeathersNumberInput, FeathersSelect, FeathersTextInput,
    FeathersTextInputContainer, NumberInputPrecision, NumberInputValue, OptionIndex, list_rows_from_strings,
};
use bevy::feathers::display::caption;
use bevy::feathers::tokens;
use bevy::prelude::*;
use bevy::text::{EditableText, TextEditChange};
use bevy::ui::Checked;
use bevy::ui_widgets::ValueChange;
use bevy_noodle::prelude::GraphQuery;
use shader_graph_core::model::{AssetKind, BlendMode, MaterialKind, Shading};
use shader_graph_core::registry::{NodeRole, ParamKind};
use shader_graph_core::{PinType, Value};
use shader_project::{AssetId, AssetType};

use super::dialog::InitText;
use super::{TEXT, TEXT_DIM, button, text};
use crate::curve_doc::CurveDoc;
use crate::doc::{node_signature, route_declarations};
use crate::state::{Action, Actions, Editor, GenFile, PropTarget, display_value};
use crate::tab::{Lang, Tab, TextDoc};

#[derive(Component, Default, Clone)]
pub struct PropsBody;

/// A property widget: what it edits and how to read its value.
#[derive(Component, Default, Clone)]
pub struct PropField {
    pub target: PropTarget,
    pub comp: usize,
    /// Option labels of a select.
    pub options: Vec<String>,
    /// For pin-type selects: the types behind `options`.
    pub types: Vec<PinType>,
    /// For selects whose stored value differs from the label (route node ids).
    pub values: Vec<String>,
}

/// Last text of a text field that reached the model. `TextEditChange` also fires for cursor moves and while
/// the field is being filled with its initial value; those must not become edits (each would bump the revision).
#[derive(Component, Default, Clone)]
pub struct FieldText {
    last: String,
    /// The initial value has been shown.
    ready: bool,
}

/// Color preview kept in sync with the model.
#[derive(Component, Default, Clone)]
pub struct SwatchOf(pub PropTarget);

pub struct PropsPlugin;

impl Plugin for PropsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (redraw, update_swatches))
            .add_observer(number_f32)
            .add_observer(number_i32)
            .add_observer(color)
            .add_observer(checkbox)
            .add_observer(select)
            .add_observer(text_changed);
    }
}

fn section(title: impl Into<String>) -> impl Scene {
    let title = title.into();
    bsn! {
        Node { margin: UiRect::top(px(6)), padding: UiRect::bottom(px(2)), border: UiRect::bottom(px(1)) }
        BorderColor::all(Color::srgb_u8(60, 62, 70))
        Children [ @text(title, TEXT_DIM, 12.0) ]
    }
}

fn labeled(label: String, widget: Box<dyn Scene>) -> impl Scene {
    bsn! {
        Node { flex_direction: FlexDirection::Row, align_items: AlignItems::Center, column_gap: px(6), min_height: px(26) }
        Children [
            Node { width: px(110), flex_shrink: 0.0 } Children [ @text(label, TEXT, 13.0) ]
            --
            Node { flex_grow: 1.0, flex_direction: FlexDirection::Row, column_gap: px(3), align_items: AlignItems::Center } Children [ @{widget} ]
        ]
    }
}

pub(crate) fn number(target: PropTarget, comp: usize, v: f32, sigil: Option<&'static str>) -> Box<dyn Scene> {
    let field = PropField { target, comp, ..Default::default() };
    let v = display_value(v);
    let token = match comp {
        0 => tokens::TEXT_INPUT_X_AXIS,
        1 => tokens::TEXT_INPUT_Y_AXIS,
        2 => tokens::TEXT_INPUT_Z_AXIS,
        _ => tokens::TEXT_INPUT_W_AXIS,
    };
    match sigil {
        Some(s) => Box::new(bsn! {
            @FeathersNumberInput { @sigil_color: token, @label_text: s }
            NumberInputValue::F32(v)
            NumberInputPrecision(3)
            Node { flex_grow: 1.0, min_width: px(40) }
            field
        }),
        None => Box::new(bsn! {
            @FeathersNumberInput
            NumberInputValue::F32(v)
            NumberInputPrecision(3)
            Node { flex_grow: 1.0 }
            field
        }),
    }
}

pub(crate) fn integer(target: PropTarget, v: i32) -> Box<dyn Scene> {
    let field = PropField { target, ..Default::default() };
    Box::new(bsn! { @FeathersNumberInput NumberInputValue::I32(v) Node { flex_grow: 1.0 } field })
}

/// One number field per component, labelled X/Y/Z/W or R/G/B/A.
pub(crate) fn vector_fields(target: &PropTarget, v: &[f32], color: bool) -> Vec<Box<dyn Scene>> {
    const XYZW: [&str; 4] = ["X", "Y", "Z", "W"];
    const RGBA: [&str; 4] = ["R", "G", "B", "A"];
    let names = if color { RGBA } else { XYZW };
    v.iter().enumerate().map(|(i, x)| number(target.clone(), i, *x, Some(names[i]))).collect()
}

/// Stored color components are linear (spec §7.1: `Color` → `LinearRgba`).
pub(crate) fn linear_color(v: &[f32]) -> Color {
    Color::linear_rgba(v[0], v[1], v[2], v.get(3).copied().unwrap_or(1.0))
}

fn vector(target: PropTarget, v: &[f32], color: bool) -> Box<dyn Scene> {
    let mut items = vector_fields(&target, v, color);
    if color {
        let c = linear_color(v);
        let sw = SwatchOf(target);
        items.insert(0, Box::new(bsn! { @FeathersColorSwatch ColorSwatchValue(c) Node { width: px(22), height: px(22), flex_shrink: 0.0 } sw }));
    }
    Box::new(
        bsn! { Node { flex_direction: FlexDirection::Row, column_gap: px(3), flex_grow: 1.0 } Children [ {items} ] },
    )
}

pub(crate) fn checkbox_w(target: PropTarget, v: bool) -> Box<dyn Scene> {
    let field = PropField { target, ..Default::default() };
    if v {
        Box::new(bsn! { @FeathersCheckbox { @caption: bsn! { @caption("") } } Checked field })
    } else {
        Box::new(bsn! { @FeathersCheckbox { @caption: bsn! { @caption("") } } field })
    }
}

fn select_w(target: PropTarget, options: Vec<String>, types: Vec<PinType>, current: usize) -> Box<dyn Scene> {
    let rows = list_rows_from_strings(options.clone(), Some(current));
    let field = PropField { target, options, types, ..Default::default() };
    Box::new(bsn! { @FeathersSelect { @options: rows, @max_visible: 10 } Node { flex_grow: 1.0 } field })
}

fn text_w(target: PropTarget, v: String, multiline: bool) -> Box<dyn Scene> {
    let sync = FieldText { ready: v.is_empty(), last: v.clone() };
    let (field, init) = (PropField { target, ..Default::default() }, InitText(v));
    if multiline {
        Box::new(bsn! {
            @FeathersTextInputContainer
            Node { height: auto(), min_height: px(160), flex_grow: 0.0, width: percent(100), align_items: AlignItems::FlexStart, padding: UiRect::all(px(4)) }
            Children [
                @FeathersTextInput
                EditableText { cursor_width: 0.3, allow_newlines: true, visible_lines: {Some(12.0)} }
                TextLayout { linebreak: LineBreak::WordOrCharacter }
                field
                init
                sync
            ]
        })
    } else {
        Box::new(
            bsn! { @FeathersTextInputContainer Node { flex_grow: 0.0, width: percent(100) } Children [ @FeathersTextInput field init sync ] },
        )
    }
}

/// Select of `(stored value, label)` choices with a leading `(none)` (= empty string): named route declarations
/// (node ids), curve assets (paths).
fn keyed_select(target: PropTarget, choices: &[(String, String)], current: &str) -> Box<dyn Scene> {
    let options: Vec<String> =
        std::iter::once("(none)".to_string()).chain(choices.iter().map(|r| r.1.clone())).collect();
    let values: Vec<String> = std::iter::once(String::new()).chain(choices.iter().map(|r| r.0.clone())).collect();
    let cur = values.iter().position(|v| v == current).unwrap_or(0);
    let rows = list_rows_from_strings(options.clone(), Some(cur));
    let field = PropField { target, options, values, ..Default::default() };
    Box::new(bsn! { @FeathersSelect { @options: rows, @max_visible: 10 } Node { flex_grow: 1.0 } field })
}

/// Choices of reference parameters: named route declarations of the asset, curve assets of the project.
#[derive(Default)]
struct Choices {
    routes: Vec<(String, String)>,
    curves: Vec<(String, String)>,
}

/// Widget editing a value of `kind`.
fn value_widget(target: PropTarget, kind: &ParamKind, v: &Value, choices: &Choices) -> Box<dyn Scene> {
    match kind {
        ParamKind::Route => keyed_select(target, &choices.routes, v.as_str()),
        ParamKind::Curve => keyed_select(target, &choices.curves, v.as_str()),
        ParamKind::Float { .. } => number(target, 0, v.as_f32(), None),
        ParamKind::Float2 => vector(target, &v.coerce(PinType::Float2).floats(), false),
        ParamKind::Float3 => vector(target, &v.coerce(PinType::Float3).floats(), false),
        ParamKind::Float4 => vector(target, &v.coerce(PinType::Float4).floats(), false),
        ParamKind::Color => vector(target, &v.coerce(PinType::Color).floats(), true),
        ParamKind::Bool => checkbox_w(target, v.as_bool()),
        ParamKind::Int { .. } => integer(target, v.as_i32()),
        ParamKind::Str => text_w(target, v.as_str().to_string(), false),
        ParamKind::Code => text_w(target, v.as_str().to_string(), true),
        ParamKind::Enum(opts) => {
            let cur = opts.iter().position(|o| o == v.as_str()).unwrap_or(0);
            select_w(target, opts.clone(), Vec::new(), cur)
        }
        ParamKind::PinType(types) => {
            let cur = types.iter().position(|t| Some(*t) == v.as_type()).unwrap_or(0);
            select_w(target, types.iter().map(|t| t.label().to_string()).collect(), types.clone(), cur)
        }
    }
}

fn pin_kind(t: PinType) -> Option<ParamKind> {
    Some(match t {
        PinType::Float => ParamKind::Float { min: None, max: None },
        PinType::Float2 => ParamKind::Float2,
        PinType::Float3 => ParamKind::Float3,
        PinType::Float4 => ParamKind::Float4,
        PinType::Color => ParamKind::Color,
        PinType::Bool => ParamKind::Bool,
        PinType::Int => ParamKind::Int { min: None, max: None },
        PinType::Texture2D | PinType::Any => return None,
    })
}

/// A generated file: where it comes from and how to get to the source.
fn text_rows(t: &TextDoc, out: &mut Vec<Box<dyn Scene>>) {
    let (lang, which) = match t.lang {
        Lang::Wesl => ("WESL", GenFile::Wesl),
        Lang::Rust => ("Rust", GenFile::Rust),
    };
    out.push(Box::new(text(t.rel.clone(), TEXT, 15.0)));
    out.push(Box::new(text(format!("Generated {lang}, read-only"), TEXT_DIM, 12.0)));
    out.push(Box::new(text(format!("Source: {}", t.source.source_rel()), TEXT_DIM, 12.0)));
    if let Some(src) = &t.text {
        out.push(Box::new(text(format!("{} lines", src.lines().count()), TEXT_DIM, 12.0)));
    }
    out.push(Box::new(button("Open source graph", Action::OpenAsset(t.source.clone()), true)));
    out.push(Box::new(button("Open in default application", Action::ShowFile(t.source.clone(), which), false)));
}

/// Hotkeys of the curve view, shown in its Properties.
const CURVE_HOTKEYS: [(&str, &str); 12] = [
    ("Double-click", "Add a point"),
    ("Drag", "Move selection"),
    ("X (held)", "Only along T"),
    ("Y (held)", "Only along value"),
    ("Shift (held)", "Snap to 0.1 grid"),
    ("Shift+click", "Toggle selection"),
    ("Arrows", "Nudge (Shift: x10)"),
    ("Del", "Delete selection"),
    ("Ctrl+A", "Select all"),
    ("Ctrl+C / V", "Copy / paste"),
    ("Ctrl+D", "Duplicate"),
    ("Right-click", "Menu"),
];

/// One hotkey row: the keys as a key cap, then what they do.
fn hotkey_row(keys: &'static str, what: &'static str) -> impl Scene {
    bsn! {
        Node { flex_direction: FlexDirection::Row, align_items: AlignItems::Center, column_gap: px(8), min_height: px(22) }
        Children [
            Node { width: px(100), flex_shrink: 0.0 }
            Children [
                Node { padding: UiRect::axes(px(5), px(1)), border: UiRect::all(px(1)), border_radius: BorderRadius::all(px(3)) }
                BackgroundColor(Color::srgb_u8(44, 46, 53))
                BorderColor::all(Color::srgb_u8(66, 68, 77))
                Children [ @text(keys, TEXT, 12.0) ]
            ]
            --
            Node { flex_grow: 1.0 } Children [ @text(what, TEXT_DIM, 12.0) ]
        ]
    }
}

/// A curve tab: the selected point and the hotkeys (preview mode and generated WESL are in the view's header, select
/// all / delete in the toolbar).
fn curve_rows(c: &CurveDoc, out: &mut Vec<Box<dyn Scene>>) {
    out.push(Box::new(text(c.id.source_rel(), TEXT, 15.0)));
    out.push(Box::new(text(format!("Type: Curve, {} point(s)", c.curve.points.len()), TEXT_DIM, 12.0)));
    match c.selection.len() {
        0 => {}
        1 => {
            let i = *c.selection.first().expect("one");
            if let Some((t, v)) = c.curve.points.get(i) {
                out.push(Box::new(section(format!("Point {}", i + 1))));
                let target = PropTarget::CurvePoint(i);
                out.push(Box::new(labeled("T".into(), number(target.clone(), 0, *t, None))));
                out.push(Box::new(labeled("Value".into(), number(target, 1, *v, None))));
            }
        }
        n => {
            out.push(Box::new(section("Selection")));
            out.push(Box::new(text(format!("{n} points selected"), TEXT, 13.0)));
        }
    }
    out.push(Box::new(section("Hotkeys")));
    for (keys, what) in CURVE_HOTKEYS {
        out.push(Box::new(hotkey_row(keys, what)));
    }
}

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
        }
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
    out.push(Box::new(text("Tip: Tab or right-click on the canvas opens the node palette.", TEXT_DIM, 12.0)));
}

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
    };
    let defs = def.params();
    if !defs.is_empty() {
        out.push(Box::new(section("Parameters")));
    }
    for p in &defs {
        let v = params.get(&p.id).cloned().unwrap_or_else(|| p.default.clone());
        let target = PropTarget::Param { node: id.to_string(), key: p.id.clone() };
        // A deleted or broken curve stays visible as the current choice (the node reports E006).
        let mut choices_here = None;
        if p.kind == ParamKind::Curve && !v.as_str().is_empty() && !choices.curves.iter().any(|c| c.0 == v.as_str()) {
            let mut c = Choices { routes: Vec::new(), curves: choices.curves.clone() };
            c.curves.push((v.as_str().to_string(), format!("{} (missing)", v.as_str())));
            choices_here = Some(c);
        }
        let choices = choices_here.as_ref().unwrap_or(&choices);
        out.push(Box::new(labeled(p.label.clone(), value_widget(target, &p.kind, &v, choices))));
        if p.kind == ParamKind::Curve && !v.as_str().is_empty() {
            let open = Action::OpenAsset(AssetId::new(AssetType::Curve, v.as_str()));
            out.push(Box::new(button("Open curve", open, false)));
        }
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

fn redraw(
    editor: Res<Editor>,
    body: Query<Entity, With<PropsBody>>,
    mut last: Local<Option<(u64, Option<usize>)>>,
    mut commands: Commands,
) {
    let key = (editor.ui.props, editor.active);
    if last.as_ref() == Some(&key) {
        return;
    }
    let Ok(body) = body.single() else { return };
    *last = Some(key);
    commands.entity(body).despawn_children();
    let mut rows: Vec<Box<dyn Scene>> = Vec::new();
    match editor.tab() {
        None => rows.push(Box::new(text("Nothing open.", TEXT_DIM, 13.0))),
        Some(Tab::Text(t)) => text_rows(t, &mut rows),
        Some(Tab::Curve(c)) => curve_rows(c, &mut rows),
        Some(Tab::Graph(d)) => {
            if let Some(f) = d.selected_frame.as_ref().and_then(|f| d.asset.comments.iter().find(|c| c.id == *f)) {
                rows.push(Box::new(text("Comment", TEXT, 15.0)));
                rows.push(Box::new(labeled(
                    "Title".into(),
                    text_w(PropTarget::FrameTitle(f.id.clone()), f.title.clone(), false),
                )));
            } else if d.selection.len() == 1 {
                let id = d.selection.iter().next().cloned().unwrap_or_default();
                node_rows(&editor, &id, &mut rows);
            } else if d.selection.len() > 1 {
                rows.push(Box::new(text(format!("{} nodes selected", d.selection.len()), TEXT, 14.0)));
                rows.push(Box::new(text("C: comment around them | Auto-layout arranges them.", TEXT_DIM, 12.0)));
            } else {
                asset_rows(&editor, &mut rows);
            }
        }
    }
    for r in rows {
        commands.spawn_scene(bsn! { @{r} ChildOf(body) });
    }
}

fn update_swatches(editor: Res<Editor>, mut q: Query<(&SwatchOf, &mut ColorSwatchValue)>) {
    if !editor.is_changed() {
        return;
    }
    let Some(d) = editor.doc() else { return };
    for (sw, mut v) in &mut q {
        let val = match &sw.0 {
            PropTarget::Param { node, key } => d.asset.nodes.get(node).and_then(|n| n.params.get(key)).cloned(),
            PropTarget::Input { node, pin } => d.asset.nodes.get(node).and_then(|n| n.inputs.get(pin)).cloned(),
            _ => None,
        };
        if let Some(val) = val {
            let c = linear_color(&val.coerce(PinType::Color).floats());
            if v.0 != c {
                v.0 = c;
            }
        }
    }
}

/// Edits made inside a canvas node end with [`Action::CommitInline`] (one undo step, Properties catches up).
fn commit_inline(source: Entity, is_final: bool, graph: &GraphQuery, actions: &mut Actions) {
    if is_final && graph.node_of(source).is_some() {
        actions.push(Action::CommitInline);
    }
}

fn number_f32(ev: On<ValueChange<f32>>, fields: Query<&PropField>, graph: GraphQuery, mut actions: ResMut<Actions>) {
    if let Ok(f) = fields.get(ev.source) {
        actions.push(Action::SetComponent {
            target: f.target.clone(),
            comp: f.comp,
            value: ev.value,
            is_final: ev.is_final,
        });
        commit_inline(ev.source, ev.is_final, &graph, &mut actions);
    }
}

fn number_i32(ev: On<ValueChange<i32>>, fields: Query<&PropField>, graph: GraphQuery, mut actions: ResMut<Actions>) {
    if let Ok(f) = fields.get(ev.source) {
        actions.push(Action::SetComponent {
            target: f.target.clone(),
            comp: 0,
            value: ev.value as f32,
            is_final: ev.is_final,
        });
        commit_inline(ev.source, ev.is_final, &graph, &mut actions);
    }
}

/// Color picker on a node: writes all components as linear values (one merged undo step per drag).
fn color(ev: On<ValueChange<Color>>, fields: Query<&PropField>, graph: GraphQuery, mut actions: ResMut<Actions>) {
    let Ok(f) = fields.get(ev.source) else { return };
    let c = ev.value.to_linear();
    for (comp, value) in [c.red, c.green, c.blue, c.alpha].into_iter().enumerate() {
        actions.push(Action::SetComponent { target: f.target.clone(), comp, value, is_final: false });
    }
    commit_inline(ev.source, ev.is_final, &graph, &mut actions);
}

fn checkbox(
    ev: On<ValueChange<bool>>,
    fields: Query<&PropField>,
    graph: GraphQuery,
    mut actions: ResMut<Actions>,
    mut commands: Commands,
) {
    if let Ok(f) = fields.get(ev.source) {
        if ev.value {
            commands.entity(ev.source).insert(Checked);
        } else {
            commands.entity(ev.source).remove::<Checked>();
        }
        actions.push(Action::SetValue { target: f.target.clone(), value: Value::Bool(ev.value), merge: false });
        commit_inline(ev.source, true, &graph, &mut actions);
    }
}

fn select(
    ev: On<ValueChange<Entity>>,
    fields: Query<&PropField>,
    idx: Query<&OptionIndex>,
    mut actions: ResMut<Actions>,
) {
    let (Ok(f), Ok(i)) = (fields.get(ev.source), idx.get(ev.value)) else { return };
    let value = match (f.types.get(i.0), f.values.get(i.0)) {
        (Some(t), _) => Value::Type(*t),
        (None, Some(v)) => Value::Str(v.clone()),
        (None, None) => Value::Str(f.options.get(i.0).cloned().unwrap_or_default()),
    };
    actions.push(Action::SetValue { target: f.target.clone(), value, merge: false });
}

fn text_changed(
    ev: On<TextEditChange>,
    mut fields: Query<(&PropField, &EditableText, &mut FieldText)>,
    mut actions: ResMut<Actions>,
) {
    let Ok((f, t, mut sync)) = fields.get_mut(ev.event_target()) else { return };
    let value = t.value().to_string();
    if !sync.ready {
        sync.ready = value == sync.last;
        return;
    }
    if value == sync.last {
        return;
    }
    sync.last = value.clone();
    actions.push(Action::SetValue { target: f.target.clone(), value: Value::Str(value), merge: true });
}
