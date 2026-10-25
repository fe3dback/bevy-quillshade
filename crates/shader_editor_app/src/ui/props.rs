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
use shader_graph_core::model::{AssetKind, BlendMode, MaterialKind, Shading};
use shader_graph_core::registry::ParamKind;
use shader_graph_core::{PinType, Value};

use super::dialog::InitText;
use super::{TEXT, TEXT_DIM, text};
use crate::doc::node_signature;
use crate::state::{Action, Actions, Editor, PropTarget};

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

fn number(target: PropTarget, comp: usize, v: f32, sigil: Option<&'static str>) -> Box<dyn Scene> {
    let field = PropField { target, comp, ..Default::default() };
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

fn integer(target: PropTarget, v: i32) -> Box<dyn Scene> {
    let field = PropField { target, ..Default::default() };
    Box::new(bsn! { @FeathersNumberInput NumberInputValue::I32(v) Node { flex_grow: 1.0 } field })
}

fn vector(target: PropTarget, v: &[f32], color: bool) -> Box<dyn Scene> {
    const XYZW: [&str; 4] = ["X", "Y", "Z", "W"];
    const RGBA: [&str; 4] = ["R", "G", "B", "A"];
    let names = if color { RGBA } else { XYZW };
    let mut items: Vec<Box<dyn Scene>> =
        v.iter().enumerate().map(|(i, x)| number(target.clone(), i, *x, Some(names[i]))).collect();
    if color {
        let c = Color::srgba(v[0], v[1], v[2], v.get(3).copied().unwrap_or(1.0));
        let sw = SwatchOf(target);
        items.insert(0, Box::new(bsn! { @FeathersColorSwatch ColorSwatchValue(c) Node { width: px(22), height: px(22), flex_shrink: 0.0 } sw }));
    }
    Box::new(
        bsn! { Node { flex_direction: FlexDirection::Row, column_gap: px(3), flex_grow: 1.0 } Children [ {items} ] },
    )
}

fn checkbox_w(target: PropTarget, v: bool) -> Box<dyn Scene> {
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
            ]
        })
    } else {
        Box::new(
            bsn! { @FeathersTextInputContainer Node { flex_grow: 0.0, width: percent(100) } Children [ @FeathersTextInput field init ] },
        )
    }
}

fn value_widget(target: PropTarget, kind: &ParamKind, v: &Value) -> Box<dyn Scene> {
    match kind {
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
    let defs = def.params();
    if !defs.is_empty() {
        out.push(Box::new(section("Parameters")));
    }
    for p in &defs {
        let v = params.get(&p.id).cloned().unwrap_or_else(|| p.default.clone());
        let target = PropTarget::Param { node: id.to_string(), key: p.id.clone() };
        out.push(Box::new(labeled(p.label.clone(), value_widget(target, &p.kind, &v))));
    }
    let Some(sig) = node_signature(&d.asset, id, &editor.registry, &editor.functions, &d.id.path) else { return };
    let mut header = false;
    for pin in &sig.inputs {
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
        out.push(Box::new(labeled(label, value_widget(target, &kind, &v))));
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
    match editor.doc() {
        None => rows.push(Box::new(text("Nothing open.", TEXT_DIM, 13.0))),
        Some(d) => {
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
            let f = val.coerce(PinType::Color).floats();
            let c = Color::srgba(f[0], f[1], f[2], f[3]);
            if v.0 != c {
                v.0 = c;
            }
        }
    }
}

fn number_f32(ev: On<ValueChange<f32>>, fields: Query<&PropField>, mut actions: ResMut<Actions>) {
    if let Ok(f) = fields.get(ev.source) {
        actions.push(Action::SetComponent {
            target: f.target.clone(),
            comp: f.comp,
            value: ev.value,
            is_final: ev.is_final,
        });
    }
}

fn number_i32(ev: On<ValueChange<i32>>, fields: Query<&PropField>, mut actions: ResMut<Actions>) {
    if let Ok(f) = fields.get(ev.source) {
        actions.push(Action::SetComponent {
            target: f.target.clone(),
            comp: 0,
            value: ev.value as f32,
            is_final: ev.is_final,
        });
    }
}

fn checkbox(
    ev: On<ValueChange<bool>>,
    fields: Query<&PropField>,
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
    }
}

fn select(
    ev: On<ValueChange<Entity>>,
    fields: Query<&PropField>,
    idx: Query<&OptionIndex>,
    mut actions: ResMut<Actions>,
) {
    let (Ok(f), Ok(i)) = (fields.get(ev.source), idx.get(ev.value)) else { return };
    let value = match f.types.get(i.0) {
        Some(t) => Value::Type(*t),
        None => Value::Str(f.options.get(i.0).cloned().unwrap_or_default()),
    };
    actions.push(Action::SetValue { target: f.target.clone(), value, merge: false });
}

fn text_changed(ev: On<TextEditChange>, fields: Query<(&PropField, &EditableText)>, mut actions: ResMut<Actions>) {
    if let Ok((f, t)) = fields.get(ev.event_target()) {
        actions.push(Action::SetValue {
            target: f.target.clone(),
            value: Value::Str(t.value().to_string()),
            merge: true,
        });
    }
}
