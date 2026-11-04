//! Game Features table editor: an add-enum form above the list of static feature enums and their variants.

use bevy::input::keyboard::KeyboardInput;
use bevy::input_focus::FocusedInput;
use bevy::prelude::*;
use editor_core::dynval::PropKey;
use shader_graph_core::features::FeatureEnum;

use super::icons::IconId;
use super::props::{PropField, text_w};
use super::{BORDER, PANEL_BG, TEXT, TEXT_DIM, VIEW_BG, ViewOf, icon_button, icon_tool_button, scroll_column, text};
use crate::editors::features::FEATURES;
use crate::features_doc::FeaturesDoc;
use crate::ops::prelude::*;
use crate::ops::{Action, Actions, Editor, FeaturesAction, FeaturesProp};

const NAME_W: f32 = 240.0;
const ERROR: Color = Color::srgb(0.95, 0.42, 0.42);
const ROW_LINE: Color = Color::srgb_u8(40, 42, 48);

/// The editor topbar row of a features tab.
#[derive(Component, Default, Clone)]
pub(crate) struct FeaturesViewBar;

/// Form and enum cards, rebuilt on [`FeaturesDoc::layout`].
#[derive(Component, Default, Clone)]
pub struct FeaturesViewBody;

pub struct FeaturesViewPlugin;

impl Plugin for FeaturesViewPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, redraw.after(crate::apply_actions)).add_observer(add_on_enter);
    }
}

pub fn features_view() -> impl Scene {
    bsn! {
        ViewOf(FEATURES)
        Node {
            position_type: PositionType::Absolute,
            left: px(0),
            right: px(0),
            top: px(0),
            bottom: px(0),
            flex_direction: FlexDirection::Column,
        }
        BackgroundColor(VIEW_BG)
        Visibility::Hidden
        Children [
            @scroll_column()
            FeaturesViewBody
            Node { padding: UiRect::new(px(12), px(12), px(10), px(56)), row_gap: px(10) }
        ]
    }
}

fn cell(width: Option<f32>, content: Box<dyn Scene>) -> Box<dyn Scene> {
    match width {
        Some(w) => Box::new(bsn! {
            Node { width: px(w), flex_shrink: 0.0, align_items: AlignItems::Center } Children [ @{content} ]
        }),
        None => Box::new(bsn! {
            Node { flex_grow: 1.0, min_width: px(0), align_items: AlignItems::Center } Children [ @{content} ]
        }),
    }
}

fn row(cells: Vec<Box<dyn Scene>>, line: bool) -> Box<dyn Scene> {
    let border = if line { 1.0 } else { 0.0 };
    Box::new(bsn! {
        Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: px(10),
            min_height: px(34),
            padding: UiRect::vertical(px(3)),
            border: UiRect::bottom(px(border)),
        }
        BorderColor::all(ROW_LINE)
        Children [ {cells} ]
    })
}

fn add_enum_form(f: &FeaturesDoc) -> Vec<Box<dyn Scene>> {
    let mut out: Vec<Box<dyn Scene>> = vec![Box::new(text("Add a feature enum", TEXT_DIM, 12.0))];
    let add = Action::from(FeaturesAction::AddEnum);
    out.push(row(
        vec![
            cell(Some(NAME_W), text_w(FeaturesProp::DraftEnumName, f.draft_enum_name.clone(), false)),
            cell(
                None,
                Box::new(icon_tool_button(
                    IconId::Plus,
                    "Add Enum",
                    add,
                    "Add a new feature enum (lower snake_case name, starts with low/high variants)",
                )),
            ),
        ],
        false,
    ));
    if let Some(e) = &f.form_error {
        out.push(Box::new(text(e.clone(), ERROR, 12.0)));
    }
    out
}

fn enum_card(f: &FeaturesDoc, en: &FeatureEnum) -> Box<dyn Scene> {
    let delete_enum = Action::from(FeaturesAction::DeleteEnum { enum_id: en.id, confirmed: false });
    let mut items: Vec<Box<dyn Scene>> = Vec::new();
    items.push(row(
        vec![
            Box::new(text("Enum", TEXT_DIM, 12.0)),
            cell(Some(NAME_W), text_w(FeaturesProp::EnumName(en.id), en.name.clone(), false)),
            cell(None, Box::new(text(format!("{} variants", en.variants.len()), TEXT_DIM, 12.0))),
            Box::new(icon_button(IconId::Delete, delete_enum, "Delete the enum (asks when nodes read it)")),
        ],
        true,
    ));
    let n_vars = en.variants.len();
    for (idx, v) in en.variants.iter().enumerate() {
        let up = Action::from(FeaturesAction::MoveVariant { enum_id: en.id, variant_id: v.id, dir: -1 });
        let down = Action::from(FeaturesAction::MoveVariant { enum_id: en.id, variant_id: v.id, dir: 1 });
        let del = Action::from(FeaturesAction::DeleteVariant { enum_id: en.id, variant_id: v.id, confirmed: false });
        let tag = if idx == 0 { "default" } else { "" };
        let mut cells: Vec<Box<dyn Scene>> = vec![
            cell(Some(32.0), Box::new(text(format!("#{idx}"), TEXT_DIM, 12.0))),
            cell(Some(NAME_W), text_w(FeaturesProp::VariantName(en.id, v.id), v.name.clone(), false)),
            cell(None, Box::new(text(tag, TEXT_DIM, 11.0))),
        ];
        if idx > 0 {
            cells.push(Box::new(icon_button(IconId::ArrowUp, up, "Move variant up")));
        }
        if idx + 1 < n_vars {
            cells.push(Box::new(icon_button(IconId::ArrowDown, down, "Move variant down")));
        }
        if n_vars > 2 {
            cells.push(Box::new(icon_button(IconId::Delete, del, "Delete variant")));
        }
        items.push(row(cells, true));
    }
    let draft_var = f.draft_variant_names.get(&en.id).cloned().unwrap_or_default();
    let add_var = Action::from(FeaturesAction::AddVariant { enum_id: en.id });
    items.push(row(
        vec![
            cell(Some(32.0), Box::new(text("+", TEXT_DIM, 12.0))),
            cell(Some(NAME_W), text_w(FeaturesProp::DraftVariantName(en.id), draft_var, false)),
            cell(
                None,
                Box::new(icon_tool_button(
                    IconId::Plus,
                    "Add Variant",
                    add_var,
                    "Add a variant to this enum (lower snake_case name)",
                )),
            ),
        ],
        false,
    ));
    Box::new(bsn! {
        Node {
            flex_direction: FlexDirection::Column,
            padding: UiRect::all(px(10)),
            border: UiRect::all(px(1)),
            row_gap: px(2),
        }
        BackgroundColor(PANEL_BG)
        BorderColor::all(BORDER)
        Children [ {items} ]
    })
}

fn redraw(
    editor: Res<Editor>,
    bar: Query<Entity, With<FeaturesViewBar>>,
    body: Query<Entity, With<FeaturesViewBody>>,
    mut last: Local<Option<u64>>,
    mut commands: Commands,
) {
    let Some(f) = editor.features_doc() else { return };
    if *last == Some(f.layout) {
        return;
    }
    let (Ok(bar), Ok(body)) = (bar.single(), body.single()) else { return };
    *last = Some(f.layout);
    commands.entity(bar).despawn_children();
    commands.spawn_scene(bsn! { @text(f.id.source_rel(), TEXT, 13.0) ChildOf(bar) });
    commands.entity(body).despawn_children();
    for r in add_enum_form(f) {
        commands.spawn_scene(bsn! { @{r} ChildOf(body) });
    }
    if f.table.enums.is_empty() {
        commands.spawn_scene(bsn! { @text("No feature enums yet: add one above.", TEXT_DIM, 13.0) ChildOf(body) });
    }
    for en in &f.table.enums {
        let card = enum_card(f, en);
        commands.spawn_scene(bsn! { @{card} ChildOf(body) });
    }
}

fn add_on_enter(ev: On<FocusedInput<KeyboardInput>>, fields: Query<&PropField>, mut actions: ResMut<Actions>) {
    if ev.input.key_code == KeyCode::Enter
        && ev.input.state.is_pressed()
        && let Ok(f) = fields.get(ev.focused_entity)
    {
        if f.target == PropKey::new(FeaturesProp::DraftEnumName) {
            actions.push(Action::from(FeaturesAction::AddEnum));
        } else if let Some(FeaturesProp::DraftVariantName(enum_id)) = f.target.downcast::<FeaturesProp>() {
            actions.push(Action::from(FeaturesAction::AddVariant { enum_id: *enum_id }));
        }
    }
}
