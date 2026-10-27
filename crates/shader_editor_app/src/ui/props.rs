//! Property widgets of the right sidebar (spec §9.4) and of inline editors on nodes: number / vector / color / select
//! / text fields editing a [`PropTarget`], labelled and hotkey rows, and the observers turning their changes into
//! [`Action`]s. The rows each editor kind shows are built in `editors/<kind>.rs` (decision 0031), grouped into
//! subpanes ([`super::widgets::subpane_box`]).

use bevy::feathers::controls::{
    ColorSwatchValue, FeathersCheckbox, FeathersColorSwatch, FeathersNumberInput, FeathersSelect, FeathersTextInput,
    FeathersTextInputContainer, HardLimit, NumberInputPrecision, NumberInputRange, NumberInputValue, OptionIndex,
    SoftLimit, list_rows_from_strings,
};
use bevy::feathers::display::caption;
use bevy::feathers::tokens;
use bevy::prelude::*;
use bevy::text::{EditableText, TextEditChange};
use bevy::ui::Checked;
use bevy::ui_widgets::ValueChange;
use bevy_noodle::prelude::GraphQuery;
use shader_graph_core::registry::ParamKind;
use shader_graph_core::{PinType, Value};

use super::dialog::InitText;
use super::widgets::checkbox;
use super::{TEXT, TEXT_DIM, text};
use crate::state::{Action, Actions, Editor, PropTarget, display_value};

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
        app.add_systems(Update, update_swatches)
            .add_observer(number_f32)
            .add_observer(number_i32)
            .add_observer(color)
            .add_observer(checkbox_changed)
            .add_observer(select)
            .add_observer(text_changed);
    }
}

pub(crate) fn labeled(label: String, widget: Box<dyn Scene>) -> impl Scene {
    bsn! {
        Node { flex_direction: FlexDirection::Row, align_items: AlignItems::Center, column_gap: px(6), min_height: px(26) }
        Children [
            Node { width: px(110), flex_shrink: 0.0 } Children [ @text(label, TEXT, 13.0) ]
            --
            Node { flex_grow: 1.0, flex_direction: FlexDirection::Row, column_gap: px(3), align_items: AlignItems::Center } Children [ @{widget} ]
        ]
    }
}

/// A bool as a captioned checkbox ([`checkbox`]) filling a row as tall as a [`labeled`] one: the box starts where the
/// other rows' labels do and its caption is the clickable label (no empty label column in front of it).
pub(crate) fn checkbox_row(label: &str, target: PropTarget, checked: bool) -> Box<dyn Scene> {
    let checkbox = checkbox(label, target, checked);
    Box::new(bsn! {
        Node { flex_direction: FlexDirection::Row, align_items: AlignItems::Center, min_height: px(26) }
        Children [ @{checkbox} ]
    })
}

/// A row editing a value of `kind`: [`labeled`] [`value_widget`], or a [`checkbox_row`] for a bool.
pub(crate) fn value_row(
    label: String,
    target: PropTarget,
    kind: &ParamKind,
    v: &Value,
    choices: &Choices,
) -> Box<dyn Scene> {
    match kind {
        ParamKind::Bool => checkbox_row(&label, target, v.as_bool()),
        _ => Box::new(labeled(label, value_widget(target, kind, v, choices))),
    }
}

/// A number input editing component `comp` of `target`; a component's field (`sigil` X/Y/Z/W or R/G/B/A) is
/// marked with its axis colour.
pub(crate) fn number(target: PropTarget, comp: usize, v: f32, sigil: Option<&'static str>) -> Box<dyn Scene> {
    let Some(s) = sigil else { return number_ranged(target, comp, v, None, None) };
    let field = PropField { target, comp, ..Default::default() };
    let v = display_value(v);
    let token = match comp {
        0 => tokens::TEXT_INPUT_X_AXIS,
        1 => tokens::TEXT_INPUT_Y_AXIS,
        2 => tokens::TEXT_INPUT_Z_AXIS,
        _ => tokens::TEXT_INPUT_W_AXIS,
    };
    Box::new(bsn! {
        @FeathersNumberInput { @sigil_color: token, @label_text: s }
        NumberInputValue::F32(v)
        NumberInputPrecision(3)
        Node { flex_grow: 1.0, min_width: px(40) }
        field
    })
}

/// A number input with optional limits (design §3): dragging stays inside the `soft` range (its fill bar shows
/// where the value is), a click enters text editing and a typed value may leave it; a `hard` limit clamps both.
/// Drags round to 2 decimals with a soft limit (as the sliders it replaces did), else to 3 — without a soft limit
/// Feathers also derives the drag speed from the precision (`10^-precision` per pixel).
pub(crate) fn number_ranged(
    target: PropTarget,
    comp: usize,
    v: f32,
    soft: Option<(f32, f32)>,
    hard: Option<(f32, f32)>,
) -> Box<dyn Scene> {
    let precision = if soft.is_some() { 2 } else { 3 };
    number_input(target, comp, v, soft, hard, precision)
}

/// [`number_ranged`] rounding drags to `precision` decimals.
pub(crate) fn number_input(
    target: PropTarget,
    comp: usize,
    v: f32,
    soft: Option<(f32, f32)>,
    hard: Option<(f32, f32)>,
    precision: i32,
) -> Box<dyn Scene> {
    let field = PropField { target, comp, ..Default::default() };
    let v = display_value(v);
    let soft: Box<dyn Scene> = match soft {
        Some((min, max)) => Box::new(bsn! { SoftLimit(NumberInputRange::F32({min..=max})) }),
        None => Box::new(bsn! {}),
    };
    let hard: Box<dyn Scene> = match hard {
        Some((min, max)) => Box::new(bsn! { HardLimit(NumberInputRange::F32({min..=max})) }),
        None => Box::new(bsn! {}),
    };
    Box::new(bsn! {
        @FeathersNumberInput
        NumberInputValue::F32(v)
        NumberInputPrecision(precision)
        @{soft}
        @{hard}
        Node { flex_grow: 1.0 }
        field
    })
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

/// Hue in whole degrees, `0..=360` both soft and hard; a drag is one undo step (`SetComponent`, `is_final` at the
/// end).
pub(crate) fn hue_input(target: PropTarget, hue: f32) -> Box<dyn Scene> {
    number_input(target, 0, hue, Some((0.0, 360.0)), Some((0.0, 360.0)), 0)
}

/// An uncaptioned checkbox, where the label is elsewhere (an inline editor on a node, a consts table cell); a
/// labelled row uses [`checkbox_row`].
pub(crate) fn checkbox_w(target: PropTarget, v: bool) -> Box<dyn Scene> {
    let field = PropField { target, ..Default::default() };
    if v {
        Box::new(bsn! { @FeathersCheckbox { @caption: bsn! { @caption("") } } Checked field })
    } else {
        Box::new(bsn! { @FeathersCheckbox { @caption: bsn! { @caption("") } } field })
    }
}

pub(crate) fn select_w(
    target: PropTarget,
    options: Vec<String>,
    types: Vec<PinType>,
    current: usize,
) -> Box<dyn Scene> {
    let rows = list_rows_from_strings(options.clone(), Some(current));
    let field = PropField { target, options, types, ..Default::default() };
    Box::new(bsn! { @FeathersSelect { @options: rows, @max_visible: 10 } Node { flex_grow: 1.0 } field })
}

pub(crate) fn text_w(target: PropTarget, v: String, multiline: bool) -> Box<dyn Scene> {
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
/// (node ids), curve assets (paths), global constants (names).
fn keyed_select(target: PropTarget, choices: &[(String, String)], current: &str) -> Box<dyn Scene> {
    let options: Vec<String> =
        std::iter::once("(none)".to_string()).chain(choices.iter().map(|r| r.1.clone())).collect();
    let values: Vec<String> = std::iter::once(String::new()).chain(choices.iter().map(|r| r.0.clone())).collect();
    let cur = values.iter().position(|v| v == current).unwrap_or(0);
    let rows = list_rows_from_strings(options.clone(), Some(cur));
    let field = PropField { target, options, values, ..Default::default() };
    Box::new(bsn! { @FeathersSelect { @options: rows, @max_visible: 10 } Node { flex_grow: 1.0 } field })
}

/// Choices of reference parameters: named route declarations of the asset, curve assets of the project, global
/// constants (of the parameter's type), Texture assets and built-in textures.
#[derive(Default, Clone)]
pub(crate) struct Choices {
    pub routes: Vec<(String, String)>,
    pub curves: Vec<(String, String)>,
    pub consts: Vec<(String, String)>,
    pub textures: Vec<(String, String)>,
}

/// Widget editing a value of `kind` (a bool: an uncaptioned checkbox, for rows that show the label themselves).
pub(crate) fn value_widget(target: PropTarget, kind: &ParamKind, v: &Value, choices: &Choices) -> Box<dyn Scene> {
    match kind {
        ParamKind::Route => keyed_select(target, &choices.routes, v.as_str()),
        ParamKind::Curve => keyed_select(target, &choices.curves, v.as_str()),
        ParamKind::GlobalConst(_) => keyed_select(target, &choices.consts, v.as_str()),
        // `(none)`: the node uses the default texture (W110).
        ParamKind::Texture => keyed_select(target, &choices.textures, v.as_str()),
        ParamKind::Float { min, max } => number_ranged(target, 0, v.as_f32(), soft_range(*min, *max), None),
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

/// The soft limit of a Float with `min` and `max` (`min < max`).
pub(crate) fn soft_range(min: Option<f32>, max: Option<f32>) -> Option<(f32, f32)> {
    match (min, max) {
        (Some(min), Some(max)) if min < max => Some((min, max)),
        _ => None,
    }
}

/// The widget kind editing an input default of type `t` (`None`: the type has no value editor).
pub(crate) fn pin_kind(t: PinType) -> Option<ParamKind> {
    Some(match t {
        PinType::Float => ParamKind::Float { min: None, max: None },
        PinType::Float2 => ParamKind::Float2,
        PinType::Float3 => ParamKind::Float3,
        PinType::Float4 => ParamKind::Float4,
        PinType::Color => ParamKind::Color,
        PinType::Bool => ParamKind::Bool,
        PinType::Int => ParamKind::Int { min: None, max: None },
        PinType::Texture2D | PinType::MaterialAttributesPbr | PinType::Any => return None,
    })
}

/// One hotkey row: the keys as a key cap, then what they do.
pub(crate) fn hotkey_row(keys: &'static str, what: &'static str) -> impl Scene {
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

fn checkbox_changed(
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
