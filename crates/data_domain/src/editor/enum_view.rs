//! `.enum.ron` visual schema editor (`EnumEditor` + `enum_view`): topbar, Variants table, Selected Variant
//! payload fields table with inline `Constraints` (`FieldSettings`) subpane, and right sidebar (`About` + `Hotkeys`).

use std::collections::{BTreeMap, BTreeSet};

use bevy::feathers::controls::{
    ButtonVariant, ColorInputValue, FeathersButton, FeathersCheckbox, FeathersToolButton, HardLimit, NumberInputRange,
    NumberInputValue, SoftLimit,
};
use bevy::input::keyboard::KeyboardInput;
use bevy::input_focus::{FocusedInput, InputFocus};
use bevy::prelude::*;
use bevy::ui::Checked;
use bevy::ui_widgets::ValueChange;

use project_core::asset_uuid::AssetUuid;

use editor_core::action::{Action, Actions, TabAction};
use editor_core::dynval::PropKey;
use editor_core::editors::{EditorAreas, SidebarKey, ViewOf, bar_row, fill_sidebar_with, sidebar_body};
use editor_core::state::editor::Editor;
use editor_core::tab::ViewKindId;
use editor_core::theme::{ACCENT, BORDER, PANEL_BG, TEXT, TEXT_DIM, text};
use editor_core::tooltip::Tooltip;
use editor_core::ui::icons::{IconId, icon};
use editor_core::ui::props::{PropField, checkbox, checkbox_w, number_input, select_values, text_hint, text_w};
use editor_core::ui::rich_text::rich_text;
use editor_core::ui::widgets::{scroll_column, subpane_box};
use editor_core::ui::{OnActivate, icon_button, icon_tool_button, tool_button};

use crate::backend::model::{
    DtoAsset, EnumAsset, EnumVariant, Field, FieldType, GameAssetKind, MAX_NESTING_DEPTH, path_step_option_some,
    to_pascal_case,
};

use super::dto_doc::ConstraintField;
use super::enum_doc::{ENUM_VIEW, EnumAction, EnumDoc, EnumProp};
use super::state::{DataState, DataStep};
use super::widgets::{
    AssetPickerCatalog, ContainerEditorCallbacks, build_container_items_rows, build_inline_value_and_subrows,
    container_summary_label, dto_form_row, dto_option_row, dto_reset_button, inner_type_select, key_type_select,
    leaf_target_select, numeric_limits, reset_icon_color, sync_data_value_widgets,
};

const VIEW_BG: Color = Color::srgb_u8(24, 25, 29);
const ROW_LINE: Color = Color::srgb_u8(40, 42, 48);
const SELECTED_ROW_BG: Color = Color::srgba(0.98, 0.57, 0.24, 0.10);
const ERROR_COLOR: Color = Color::srgb(0.95, 0.42, 0.42);

#[derive(Component, Default, Clone)]
pub struct EnumTopbar;

#[derive(Component, Default, Clone)]
pub struct EnumSidebar;

#[derive(Component, Default, Clone)]
pub struct EnumViewBody;

#[derive(Component, Default, Clone, Copy)]
pub struct EnumVariantRow(pub AssetUuid);

#[derive(Component, Default, Clone, Copy)]
pub struct EnumFieldRow(pub AssetUuid);

#[derive(Component, Default, Clone, Copy)]
pub struct EnumFieldConstraintsSlot(pub AssetUuid);

#[derive(Component, Default, Clone, Copy)]
pub struct EnumConstraintsBtnText(pub AssetUuid);

#[derive(Component, Default, Clone, Debug, PartialEq, Eq)]
pub struct EnumDefaultSummaryText {
    pub path: Vec<AssetUuid>,
}

#[derive(Component, Default, Clone, Debug, PartialEq, Eq)]
pub struct EnumDefaultResetIcon {
    pub path: Vec<AssetUuid>,
}

pub struct EnumEditor;

impl EditorAreas for EnumEditor {
    const VIEW: ViewKindId = ENUM_VIEW;
    const SIDEBAR_TITLE: &'static str = "Enum";

    fn topbar() -> Option<impl Scene> {
        Some(bsn! { @bar_row() EnumTopbar })
    }

    fn sidebar() -> Option<impl Scene> {
        Some(bsn! { @sidebar_body() EnumSidebar })
    }

    fn build(app: &mut App) {
        app.add_systems(
            Update,
            (redraw_topbar, redraw_sidebar, redraw_body, redraw_constraints_slots, sync_fields)
                .chain()
                .after(editor_core::apply_actions),
        )
        .add_observer(commit_color)
        .add_observer(add_on_enter);
    }
}

pub fn enum_view() -> impl Scene {
    bsn! {
        ViewOf(ENUM_VIEW)
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
            EnumViewBody
            Node {
                padding: UiRect::new(px(12), px(12), px(10), px(56)),
                row_gap: px(10),
            }
        ]
    }
}

fn active_enum(editor: &Editor) -> Option<&EnumDoc> {
    editor.active_doc::<EnumDoc>()
}

const fn is_integer_type(ty: &FieldType) -> bool {
    matches!(
        ty,
        FieldType::I8
            | FieldType::I16
            | FieldType::I32
            | FieldType::I64
            | FieldType::U8
            | FieldType::U16
            | FieldType::U32
            | FieldType::U64
    )
}

fn field_type_options() -> (Vec<String>, Vec<String>) {
    let labels = FieldType::all().iter().map(|t| t.label().to_string()).collect();
    let values = FieldType::all().iter().map(|t| t.label().to_string()).collect();
    (labels, values)
}

fn field_type_select(target: EnumProp, current: &FieldType) -> Box<dyn Scene> {
    let (labels, values) = field_type_options();
    let idx = FieldType::all().iter().position(|t| t.same_kind(current)).unwrap_or(0);
    select_values(target, labels, values, idx)
}

fn game_asset_kind_select(target: EnumProp, current: GameAssetKind) -> Box<dyn Scene> {
    let labels = GameAssetKind::ALL.iter().map(|k| k.label().to_string()).collect();
    let values = GameAssetKind::ALL.iter().map(|k| k.label().to_string()).collect();
    let idx = GameAssetKind::ALL.iter().position(|&k| k == current).unwrap_or(0);
    select_values(target, labels, values, idx)
}

fn resolve_doc_dtos(editor: &Editor, doc: &EnumDoc) -> BTreeMap<String, DtoAsset> {
    let mut dtos = doc.cached_dtos.clone();
    if let Some(ds) = editor.domains.get::<DataState>() {
        for (p, d) in &ds.assets.dtos {
            dtos.insert(p.clone(), d.clone());
        }
    }
    for tab in &editor.tabs {
        if let Some(doc_entry) = tab.get::<super::dto_doc::DtoDoc>() {
            dtos.insert(doc_entry.path.clone(), doc_entry.dto.clone());
        }
    }
    dtos
}

fn resolve_doc_enums(editor: &Editor, doc: &EnumDoc) -> BTreeMap<String, EnumAsset> {
    let mut enums = doc.cached_enums.clone();
    if let Some(ds) = editor.domains.get::<DataState>() {
        for (p, e) in &ds.assets.enums {
            enums.insert(p.clone(), e.clone());
        }
    }
    for tab in &editor.tabs {
        if let Some(doc_entry) = tab.get::<EnumDoc>() {
            enums.insert(doc_entry.path.clone(), doc_entry.en.clone());
        }
    }
    enums.insert(doc.path.clone(), doc.en.clone());
    enums
}

fn dto_target_select(target: EnumProp, current: AssetUuid, dtos: &BTreeMap<String, DtoAsset>) -> Box<dyn Scene> {
    let mut labels = vec!["(Select DTO)".to_string()];
    let mut values = vec![AssetUuid::default().to_string()];
    for (path, dto) in dtos {
        if dto.id.is_nil() {
            continue;
        }
        labels.push(path.clone());
        values.push(dto.id.to_string());
    }
    if !current.is_nil() && !values.iter().any(|v| v == &current.to_string()) {
        labels.push(format!("{current} (missing)"));
        values.push(current.to_string());
    }
    let cur_str = current.to_string();
    let idx = values.iter().position(|v| v == &cur_str).unwrap_or(0);
    select_values(target, labels, values, idx)
}

fn enum_target_select(
    target: EnumProp,
    current: AssetUuid,
    self_id: AssetUuid,
    enums: &BTreeMap<String, EnumAsset>,
) -> Box<dyn Scene> {
    let mut labels = vec!["(Select Enum)".to_string()];
    let mut values = vec![AssetUuid::default().to_string()];
    for (path, en) in enums {
        if en.id.is_nil() || en.id == self_id {
            continue;
        }
        labels.push(path.clone());
        values.push(en.id.to_string());
    }
    if !current.is_nil() && !values.iter().any(|v| v == &current.to_string()) {
        labels.push(format!("{current} (missing)"));
        values.push(current.to_string());
    }
    let cur_str = current.to_string();
    let idx = values.iter().position(|v| v == &cur_str).unwrap_or(0);
    select_values(target, labels, values, idx)
}

fn enum_default_reset_button(path: Vec<AssetUuid>, action: Action, is_default: bool, tooltip: &str) -> Box<dyn Scene> {
    let color = reset_icon_color(is_default);
    let tag = EnumDefaultResetIcon { path };
    let act = OnActivate(action);
    let tip = Tooltip(tooltip.to_string());
    Box::new(bsn! {
        @FeathersToolButton {
            @variant: ButtonVariant::Plain,
            @caption: bsn! { @icon(IconId::Undo, 14.0, color) tag },
        }
        Node { flex_shrink: 0.0 }
        act
        tip
    })
}

fn enum_default_toggle_button(path: Vec<AssetUuid>, summary: String, expanded: bool, tooltip: &str) -> Box<dyn Scene> {
    let color = if expanded { ACCENT } else { TEXT };
    let caret = if expanded { IconId::CaretDown } else { IconId::CaretRight };
    let act = OnActivate(Action::from(EnumAction::ToggleNestedDefault(path.clone())));
    let tip = Tooltip(tooltip.to_string());
    Box::new(bsn! {
        @FeathersButton
        Node {
            height: px(24),
            padding: UiRect::axes(px(10), px(0)),
            justify_content: JustifyContent::SpaceBetween,
            align_items: AlignItems::Center,
            column_gap: px(6),
            flex_grow: 1.0,
            min_width: px(0),
        }
        act
        tip
        Children [
            @text(summary, color, 12.0)
            EnumDefaultSummaryText { path }
            --
            @icon(caret, 12.0, color)
        ]
    })
}

fn constraints_button(field_id: AssetUuid, has_some: bool) -> Box<dyn Scene> {
    let (label, color) = if has_some { ("Some", ACCENT) } else { ("None", TEXT_DIM) };
    let act = OnActivate(Action::from(EnumAction::ToggleConstraints(field_id)));
    Box::new(bsn! {
        @FeathersButton
        Node {
            height: px(24),
            padding: UiRect::axes(px(10), px(0)),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
        }
        act
        Children [
            @text(label, color, 12.0)
            EnumConstraintsBtnText(field_id)
        ]
    })
}

fn cell(width: Option<f32>, content: Box<dyn Scene>) -> Box<dyn Scene> {
    match width {
        Some(w) => Box::new(bsn! {
            Node { width: px(w), min_width: px(0), flex_shrink: 0.0, align_items: AlignItems::Center }
            Children [ @{content} ]
        }),
        None => Box::new(bsn! {
            Node { flex_grow: 1.0, min_width: px(120), align_items: AlignItems::Center }
            Children [ @{content} ]
        }),
    }
}

fn variants_subpane(doc: &EnumDoc) -> Box<dyn Scene> {
    let add_bar: Box<dyn Scene> = Box::new(bsn! {
        Node {
            flex_direction: FlexDirection::Row,
            flex_wrap: FlexWrap::Wrap,
            align_items: AlignItems::Center,
            column_gap: px(8),
            row_gap: px(6),
            padding: UiRect::bottom(px(6)),
            border: UiRect::bottom(px(1)),
        }
        BorderColor::all(ROW_LINE)
        Children [
            @text("New Variant:", TEXT_DIM, 11.0)
            --
            Node { width: px(180), flex_shrink: 0.0 }
            Children [ @{text_hint(EnumProp::DraftVariantName, doc.draft_variant_name.clone(), "variant_name")} ]
            --
            @{icon_tool_button(
                IconId::Plus,
                "Add Variant",
                Action::from(EnumAction::AddVariant),
                "Add a new variant to this enum schema",
            )}
        ]
    });

    let mut rows: Vec<Box<dyn Scene>> = vec![add_bar];
    if let Some(err) = &doc.variant_error {
        rows.push(Box::new(text(err.clone(), ERROR_COLOR, 12.0)));
    }

    let hdr = |label: &'static str| -> Box<dyn Scene> { Box::new(text(label, TEXT_DIM, 11.0)) };
    rows.push(Box::new(bsn! {
        Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: px(8),
            min_height: px(26),
            padding: UiRect::axes(px(6), px(2)),
            border: UiRect::bottom(px(1)),
        }
        BorderColor::all(ROW_LINE)
        Children [
            @{cell(Some(32.0), hdr("#"))}
            --
            @{cell(Some(160.0), hdr("Variant Name"))}
            --
            @{cell(Some(120.0), hdr("Rust Variant"))}
            --
            @{cell(Some(96.0), hdr("Payload"))}
            --
            @{cell(None, hdr("Doc Comment"))}
            --
            @{cell(Some(84.0), hdr("Order"))}
        ]
    }));

    if doc.en.variants.is_empty() {
        rows.push(Box::new(bsn! {
            Node { padding: UiRect::all(px(8)) }
            Children [ @text("No variants in this Enum yet. Use the New Variant bar above to add one.", TEXT_DIM, 12.0) ]
        }));
    } else {
        let selected_vid = doc.selected_variant.or_else(|| doc.en.variants.first().map(|v| v.id));
        for (idx, v) in doc.en.variants.iter().enumerate() {
            let selected = selected_vid == Some(v.id);
            let bg = if selected { SELECTED_ROW_BG } else { Color::NONE };
            let left_border = if selected { 2.0 } else { 0.0 };
            let select_act = Action::from(EnumAction::SelectVariant(Some(v.id)));
            let idx_btn = Box::new(tool_button(format!("{}", idx + 1), select_act));
            let name_in = text_w(EnumProp::VariantName(v.id), v.name.clone(), false);
            let pascal_lbl = Box::new(text(to_pascal_case(&v.name), if selected { ACCENT } else { TEXT }, 12.0));
            let payload_lbl: Box<dyn Scene> = if v.fields.is_empty() {
                Box::new(text("Unit", TEXT_DIM, 11.5))
            } else if v.fields.len() == 1 {
                Box::new(text("1 field", ACCENT, 11.5))
            } else {
                Box::new(text(format!("{} fields", v.fields.len()), ACCENT, 11.5))
            };
            let doc_in = text_hint(EnumProp::VariantDoc(v.id), v.doc.clone(), "Rust /// doc comment for variant");
            let order_box: Box<dyn Scene> = Box::new(bsn! {
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: px(2),
                }
                Children [
                    @{icon_button(
                        IconId::ArrowUp,
                        Action::from(EnumAction::MoveVariant { id: v.id, delta: -1 }),
                        "Move variant up",
                    )}
                    --
                    @{icon_button(
                        IconId::ArrowDown,
                        Action::from(EnumAction::MoveVariant { id: v.id, delta: 1 }),
                        "Move variant down",
                    )}
                    --
                    @{icon_button(
                        IconId::Delete,
                        Action::from(EnumAction::DeleteVariant(v.id)),
                        "Delete variant",
                    )}
                ]
            });

            let vid = v.id;
            rows.push(Box::new(bsn! {
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: px(8),
                    min_height: px(34),
                    padding: UiRect::axes(px(6), px(3)),
                    border: UiRect { left: px(left_border), bottom: px(1), ..UiRect::default() },
                }
                EnumVariantRow(vid)
                BackgroundColor(bg)
                BorderColor {
                    left: ACCENT,
                    bottom: ROW_LINE,
                    ..BorderColor::all(Color::NONE)
                }
                Children [
                    @{cell(Some(32.0), idx_btn)}
                    --
                    @{cell(Some(160.0), name_in)}
                    --
                    @{cell(Some(120.0), pascal_lbl)}
                    --
                    @{cell(Some(96.0), payload_lbl)}
                    --
                    @{cell(None, doc_in)}
                    --
                    @{cell(Some(84.0), order_box)}
                ]
            }));
        }
    }

    let kind_badge = if doc.en.is_unit_only() { "Unit Enum (Copy)" } else { "Tagged Union" };
    let title = format!("Ordered Variants ({}) — {}", doc.en.variants.len(), kind_badge);
    subpane_box(&title, rows)
}

fn append_draft_secondary_type_items(
    bar_items: &mut Vec<Box<dyn Scene>>,
    ty: &FieldType,
    self_enum_id: AssetUuid,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
) {
    if let Some(target_id) = ty.dto_target().or_else(|| ty.soft_ref_target()) {
        bar_items.push(Box::new(bsn! {
            Node { width: px(160), flex_shrink: 0.0 }
            Children [ @{dto_target_select(EnumProp::DraftFieldDtoTarget, target_id, dtos)} ]
        }));
    } else if let Some(target_id) = ty.enum_target() {
        bar_items.push(Box::new(bsn! {
            Node { width: px(160), flex_shrink: 0.0 }
            Children [ @{enum_target_select(EnumProp::DraftFieldEnumTarget, target_id, self_enum_id, enums)} ]
        }));
    } else if let FieldType::GameAsset(kind) = *ty {
        bar_items.push(Box::new(bsn! {
            Node { width: px(120), flex_shrink: 0.0 }
            Children [ @{game_asset_kind_select(EnumProp::DraftFieldGameAssetKind, kind)} ]
        }));
    } else if let FieldType::Option(inner) | FieldType::Array(inner) | FieldType::Set(inner) = ty {
        let for_set = ty.is_set();
        bar_items.push(Box::new(bsn! {
            Node { width: px(120), flex_shrink: 0.0 }
            Children [ @{inner_type_select(EnumProp::DraftFieldInnerType, inner, for_set)} ]
        }));
        if let Some(target_sel) = leaf_target_select(EnumProp::DraftFieldInnerTarget, inner, None, true, dtos, enums) {
            bar_items.push(Box::new(bsn! {
                Node { width: px(150), flex_shrink: 0.0 }
                Children [ @{target_sel} ]
            }));
        }
    } else if let FieldType::Map(key_ty, val_ty) = ty {
        bar_items.push(Box::new(bsn! {
            Node { width: px(110), flex_shrink: 0.0 }
            Children [ @{key_type_select(EnumProp::DraftFieldKeyType, key_ty)} ]
        }));
        if let Some(key_target_sel) = leaf_target_select(EnumProp::DraftFieldKeyTarget, key_ty, None, true, dtos, enums)
        {
            bar_items.push(Box::new(bsn! {
                Node { width: px(140), flex_shrink: 0.0 }
                Children [ @{key_target_sel} ]
            }));
        }
        bar_items.push(Box::new(text("→", TEXT_DIM, 11.0)));
        bar_items.push(Box::new(bsn! {
            Node { width: px(120), flex_shrink: 0.0 }
            Children [ @{inner_type_select(EnumProp::DraftFieldInnerType, val_ty, false)} ]
        }));
        if let Some(val_target_sel) =
            leaf_target_select(EnumProp::DraftFieldInnerTarget, val_ty, None, true, dtos, enums)
        {
            bar_items.push(Box::new(bsn! {
                Node { width: px(150), flex_shrink: 0.0 }
                Children [ @{val_target_sel} ]
            }));
        }
    }
}

fn field_type_column_scene(
    f: &Field,
    self_enum_id: AssetUuid,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
) -> Box<dyn Scene> {
    let primary = field_type_select(EnumProp::FieldType(f.id), &f.ty);
    let mut col_items: Vec<Box<dyn Scene>> = vec![primary];
    if let Some(target_id) = f.ty.dto_target().or_else(|| f.ty.soft_ref_target()) {
        col_items.push(dto_target_select(EnumProp::FieldDtoTarget(f.id), target_id, dtos));
    } else if let Some(target_id) = f.ty.enum_target() {
        col_items.push(enum_target_select(EnumProp::FieldEnumTarget(f.id), target_id, self_enum_id, enums));
    } else if let FieldType::GameAsset(kind) = f.ty {
        col_items.push(game_asset_kind_select(EnumProp::FieldGameAssetKind(f.id), kind));
    } else if let FieldType::Option(inner) | FieldType::Array(inner) | FieldType::Set(inner) = &f.ty {
        let for_set = f.ty.is_set();
        col_items.push(inner_type_select(EnumProp::FieldInnerType(f.id), inner, for_set));
        if let Some(target_sel) = leaf_target_select(EnumProp::FieldInnerTarget(f.id), inner, None, true, dtos, enums) {
            col_items.push(target_sel);
        }
    } else if let FieldType::Map(key_ty, val_ty) = &f.ty {
        col_items.push(key_type_select(EnumProp::FieldKeyType(f.id), key_ty));
        if let Some(key_target_sel) =
            leaf_target_select(EnumProp::FieldKeyTarget(f.id), key_ty, None, true, dtos, enums)
        {
            col_items.push(key_target_sel);
        }
        col_items.push(inner_type_select(EnumProp::FieldInnerType(f.id), val_ty, false));
        if let Some(val_target_sel) =
            leaf_target_select(EnumProp::FieldInnerTarget(f.id), val_ty, None, true, dtos, enums)
        {
            col_items.push(val_target_sel);
        }
    }
    if col_items.len() == 1 {
        col_items.pop().unwrap()
    } else {
        Box::new(bsn! {
            Node {
                flex_direction: FlexDirection::Column,
                row_gap: px(2),
                flex_grow: 1.0,
                min_width: px(0),
            }
            Children [ {col_items} ]
        })
    }
}

fn field_has_expanded_default_subpane(doc: &EnumDoc, f: &Field) -> bool {
    let root_path = vec![f.id];
    match &f.ty {
        FieldType::Dto(_) | FieldType::Enum(_) | FieldType::Array(_) | FieldType::Set(_) | FieldType::Map(_, _) => {
            doc.expanded_nested.contains(&root_path)
        }
        FieldType::Option(inner)
            if matches!(inner.as_ref(), FieldType::Dto(_) | FieldType::Enum(_))
                && f.default.as_option().flatten().is_some() =>
        {
            let some_path = vec![f.id, path_step_option_some()];
            doc.expanded_nested.contains(&some_path)
        }
        _ => false,
    }
}

fn variant_payload_subpane(
    doc: &EnumDoc,
    variant: &EnumVariant,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
    catalog: &AssetPickerCatalog,
) -> Box<dyn Scene> {
    let mut bar_items: Vec<Box<dyn Scene>> = vec![
        Box::new(text("New Payload Field:", TEXT_DIM, 11.0)),
        Box::new(bsn! {
            Node { width: px(160), flex_shrink: 0.0 }
            Children [ @{text_hint(EnumProp::DraftFieldName, doc.draft_field_name.clone(), "field_name")} ]
        }),
        Box::new(bsn! {
            Node { width: px(130), flex_shrink: 0.0 }
            Children [ @{field_type_select(EnumProp::DraftFieldType, &doc.draft_field_ty)} ]
        }),
    ];
    append_draft_secondary_type_items(&mut bar_items, &doc.draft_field_ty, doc.en.id, dtos, enums);
    bar_items.push(checkbox("Name", EnumProp::DraftFieldIsName, doc.draft_field_is_name));
    bar_items.push(Box::new(icon_tool_button(
        IconId::Plus,
        "Add Field",
        Action::from(EnumAction::AddField),
        "Add payload field to the selected variant",
    )));

    let mut rows: Vec<Box<dyn Scene>> = vec![Box::new(bsn! {
        Node {
            flex_direction: FlexDirection::Row,
            flex_wrap: FlexWrap::Wrap,
            align_items: AlignItems::Center,
            column_gap: px(8),
            row_gap: px(6),
            padding: UiRect::bottom(px(6)),
            border: UiRect::bottom(px(1)),
        }
        BorderColor::all(ROW_LINE)
        Children [ {bar_items} ]
    })];
    if let Some(err) = &doc.field_error {
        rows.push(Box::new(text(err.clone(), ERROR_COLOR, 12.0)));
    }

    let hdr = |label: &'static str| -> Box<dyn Scene> { Box::new(text(label, TEXT_DIM, 11.0)) };
    rows.push(Box::new(bsn! {
        Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: px(8),
            min_height: px(26),
            padding: UiRect::axes(px(6), px(2)),
            border: UiRect::bottom(px(1)),
        }
        BorderColor::all(ROW_LINE)
        Children [
            @{cell(Some(32.0), hdr("#"))}
            --
            @{cell(Some(44.0), hdr("Name"))}
            --
            @{cell(Some(140.0), hdr("Field Name"))}
            --
            @{cell(Some(148.0), hdr("Field Type"))}
            --
            @{cell(None, hdr("Default"))}
            --
            @{cell(Some(84.0), hdr("Constraints"))}
            --
            @{cell(Some(84.0), hdr("Order"))}
        ]
    }));

    if variant.fields.is_empty() {
        rows.push(Box::new(bsn! {
            Node { padding: UiRect::all(px(8)) }
            Children [
                @text(
                    format!("Variant `{}` is a unit variant (0 payload fields). Add a field above to make it a struct variant.", to_pascal_case(&variant.name)),
                    TEXT_DIM,
                    12.0,
                )
            ]
        }));
    } else {
        for (idx, f) in variant.fields.iter().enumerate() {
            let selected = doc.selected_field == Some(f.id);
            let bg = if selected { SELECTED_ROW_BG } else { Color::NONE };
            let left_border = if selected { 2.0 } else { 0.0 };
            let select_act = Action::from(EnumAction::SelectField(Some(f.id)));
            let idx_btn = Box::new(tool_button(format!("{}", idx + 1), select_act));
            let name_cb = checkbox_w(EnumProp::FieldIsName(f.id), f.is_name);
            let name_in = text_w(EnumProp::FieldName(f.id), f.name.clone(), false);
            let type_in = field_type_column_scene(f, doc.en.id, dtos, enums);

            let path = vec![f.id];
            let expected_default = f.ty.default_value_with_schemas(dtos, enums);
            let is_default = f.default == expected_default;
            let value_prop = |p: Vec<AssetUuid>| {
                if p.len() == 1 {
                    PropKey::new(EnumProp::FieldDefault(p[0]))
                } else {
                    PropKey::new(EnumProp::NestedFieldDefault(p))
                }
            };
            let toggle_expand =
                |p: Vec<AssetUuid>, sum: String, exp: bool, tip: &str| enum_default_toggle_button(p, sum, exp, tip);
            let set_option_some =
                |p: Vec<AssetUuid>, some: bool| Action::from(EnumAction::SetOptionSome { path: p, some });
            let container_add = |p: Vec<AssetUuid>| Action::from(EnumAction::ContainerAdd { path: p });
            let container_remove =
                |p: Vec<AssetUuid>, index: usize| Action::from(EnumAction::ContainerRemove { path: p, index });
            let container_move = |p: Vec<AssetUuid>, index: usize, delta: isize| {
                Action::from(EnumAction::ContainerMove { path: p, index, delta })
            };
            let render_nested = |_: &[Field], _: Vec<AssetUuid>, _: usize, _: &mut BTreeSet<AssetUuid>| Vec::new();
            let cb = ContainerEditorCallbacks {
                value_prop: &value_prop,
                toggle_expand: &toggle_expand,
                set_option_some: &set_option_some,
                container_add: &container_add,
                container_remove: &container_remove,
                container_move: &container_move,
                render_nested_fields: &render_nested,
            };
            let mut dummy_visited = BTreeSet::new();
            let (def_widget, _) = build_inline_value_and_subrows(
                &f.ty,
                &f.default,
                &f.settings,
                &path,
                MAX_NESTING_DEPTH,
                &mut dummy_visited,
                &doc.expanded_nested,
                dtos,
                enums,
                catalog,
                &cb,
            );

            let reset_btn = enum_default_reset_button(
                path.clone(),
                Action::from(EnumAction::ResetFieldDefault(f.id)),
                is_default,
                "Reset default value to type zero/default",
            );
            let def_box: Box<dyn Scene> = Box::new(bsn! {
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: px(4),
                    flex_grow: 1.0,
                    min_width: px(0),
                }
                Children [
                    Node { flex_grow: 1.0, min_width: px(0) }
                    Children [ @{def_widget} ]
                    --
                    @{reset_btn}
                ]
            });
            let constraints_btn = constraints_button(f.id, !f.settings.is_default());
            let order_box: Box<dyn Scene> = Box::new(bsn! {
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: px(2),
                }
                Children [
                    @{icon_button(
                        IconId::ArrowUp,
                        Action::from(EnumAction::MoveField { id: f.id, delta: -1 }),
                        "Move payload field up",
                    )}
                    --
                    @{icon_button(
                        IconId::ArrowDown,
                        Action::from(EnumAction::MoveField { id: f.id, delta: 1 }),
                        "Move payload field down",
                    )}
                    --
                    @{icon_button(
                        IconId::Delete,
                        Action::from(EnumAction::DeleteField(f.id)),
                        "Delete payload field",
                    )}
                ]
            });

            let mut slot_children: Vec<Box<dyn Scene>> = Vec::new();
            if field_has_expanded_default_subpane(doc, f) {
                slot_children.push(enum_nested_default_subpane(doc, f, dtos, enums, catalog));
            }
            if doc.expanded_constraints == Some(f.id) {
                slot_children.push(field_settings_subpane(f));
            }

            let fid = f.id;
            rows.push(Box::new(bsn! {
                Node { flex_direction: FlexDirection::Column }
                Children [
                    Node {
                        flex_direction: FlexDirection::Row,
                        align_items: AlignItems::Center,
                        column_gap: px(8),
                        min_height: px(34),
                        padding: UiRect::axes(px(6), px(3)),
                        border: UiRect { left: px(left_border), bottom: px(1), ..UiRect::default() },
                    }
                    EnumFieldRow(fid)
                    BackgroundColor(bg)
                    BorderColor {
                        left: ACCENT,
                        bottom: ROW_LINE,
                        ..BorderColor::all(Color::NONE)
                    }
                    Children [
                        @{cell(Some(32.0), idx_btn)}
                        --
                        @{cell(Some(44.0), name_cb)}
                        --
                        @{cell(Some(140.0), name_in)}
                        --
                        @{cell(Some(148.0), type_in)}
                        --
                        @{cell(None, def_box)}
                        --
                        @{cell(Some(84.0), constraints_btn)}
                        --
                        @{cell(Some(84.0), order_box)}
                    ]
                    --
                    Node { flex_direction: FlexDirection::Column }
                    EnumFieldConstraintsSlot(fid)
                    Children [ {slot_children} ]
                ]
            }));
        }
    }

    let title = format!(
        "Selected Variant Payload — {} (`{}`) ({} fields)",
        to_pascal_case(&variant.name),
        variant.name,
        variant.fields.len()
    );
    subpane_box(&title, rows)
}

fn enum_nested_default_subpane(
    doc: &EnumDoc,
    f: &Field,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
    catalog: &AssetPickerCatalog,
) -> Box<dyn Scene> {
    let mut visited = BTreeSet::new();
    visited.insert(doc.en.id);
    let mut close_path = vec![f.id];
    let (title, form_rows) = match &f.ty {
        FieldType::Dto(target_id) => {
            let Some((target_path, target_dto)) = dtos.iter().find(|(_, d)| d.id == *target_id) else {
                return Box::new(bsn! {
                    Node {
                        margin: UiRect { left: px(28), right: px(6), top: px(4), bottom: px(8) },
                        padding: UiRect::all(px(8)),
                        border: UiRect::all(px(1)),
                    }
                    BackgroundColor(PANEL_BG)
                    BorderColor::all(BORDER)
                    Children [ @text(format!("Target DTO {} not found in project.", target_id), ERROR_COLOR, 12.0) ]
                });
            };
            visited.insert(*target_id);
            let rows =
                build_fields_default_rows(doc, &target_dto.fields, vec![f.id], 1, &mut visited, dtos, enums, catalog);
            (format!("Default Values - {} ({target_path})", f.name), rows)
        }
        FieldType::Enum(target_id) => {
            let Some((target_path, target_enum)) = enums.iter().find(|(_, e)| e.id == *target_id) else {
                return Box::new(bsn! {
                    Node {
                        margin: UiRect { left: px(28), right: px(6), top: px(4), bottom: px(8) },
                        padding: UiRect::all(px(8)),
                        border: UiRect::all(px(1)),
                    }
                    BackgroundColor(PANEL_BG)
                    BorderColor::all(BORDER)
                    Children [ @text(format!("Target Enum {} not found in project.", target_id), ERROR_COLOR, 12.0) ]
                });
            };
            let active_vid = f
                .default
                .as_enum()
                .map(|(vid, _)| vid)
                .or_else(|| target_enum.variants.first().map(|v| v.id))
                .unwrap_or(AssetUuid::NIL);
            let Some(var_def) = target_enum.variant_by_id(active_vid) else {
                return Box::new(bsn! { Node {} });
            };
            if var_def.fields.is_empty() {
                return Box::new(bsn! { Node {} });
            }
            visited.insert(*target_id);
            let rows =
                build_fields_default_rows(doc, &var_def.fields, vec![f.id], 1, &mut visited, dtos, enums, catalog);
            (format!("Default Values - {} ({target_path}::{})", f.name, var_def.name), rows)
        }
        FieldType::Option(inner_ty) => {
            let some_path = vec![f.id, path_step_option_some()];
            close_path = some_path.clone();
            let Some(inner_val) = f.default.as_option().flatten() else {
                return Box::new(bsn! { Node {} });
            };
            match inner_ty.as_ref() {
                FieldType::Dto(target_id) => {
                    let Some((target_path, target_dto)) = dtos.iter().find(|(_, d)| d.id == *target_id) else {
                        return Box::new(bsn! { Node {} });
                    };
                    visited.insert(*target_id);
                    let rows = build_fields_default_rows(
                        doc,
                        &target_dto.fields,
                        some_path,
                        1,
                        &mut visited,
                        dtos,
                        enums,
                        catalog,
                    );
                    (format!("Default Values - {} (Option<{target_path}>)", f.name), rows)
                }
                FieldType::Enum(target_id) => {
                    let Some((target_path, target_enum)) = enums.iter().find(|(_, e)| e.id == *target_id) else {
                        return Box::new(bsn! { Node {} });
                    };
                    let active_vid = inner_val
                        .as_enum()
                        .map(|(vid, _)| vid)
                        .or_else(|| target_enum.variants.first().map(|v| v.id))
                        .unwrap_or(AssetUuid::NIL);
                    let Some(var_def) = target_enum.variant_by_id(active_vid) else {
                        return Box::new(bsn! { Node {} });
                    };
                    if var_def.fields.is_empty() {
                        return Box::new(bsn! { Node {} });
                    }
                    visited.insert(*target_id);
                    let rows = build_fields_default_rows(
                        doc,
                        &var_def.fields,
                        some_path,
                        1,
                        &mut visited,
                        dtos,
                        enums,
                        catalog,
                    );
                    (format!("Default Values - {} (Option<{target_path}::{}>)", f.name, var_def.name), rows)
                }
                _ => return Box::new(bsn! { Node {} }),
            }
        }
        FieldType::Array(_) | FieldType::Set(_) | FieldType::Map(_, _) => {
            let value_prop = |p: Vec<AssetUuid>| {
                if p.len() == 1 {
                    PropKey::new(EnumProp::FieldDefault(p[0]))
                } else {
                    PropKey::new(EnumProp::NestedFieldDefault(p))
                }
            };
            let toggle_expand =
                |p: Vec<AssetUuid>, sum: String, exp: bool, tip: &str| enum_default_toggle_button(p, sum, exp, tip);
            let set_option_some =
                |p: Vec<AssetUuid>, some: bool| Action::from(EnumAction::SetOptionSome { path: p, some });
            let container_add = |p: Vec<AssetUuid>| Action::from(EnumAction::ContainerAdd { path: p });
            let container_remove =
                |p: Vec<AssetUuid>, index: usize| Action::from(EnumAction::ContainerRemove { path: p, index });
            let container_move = |p: Vec<AssetUuid>, index: usize, delta: isize| {
                Action::from(EnumAction::ContainerMove { path: p, index, delta })
            };
            let render_nested =
                |sub_fields: &[Field], prefix: Vec<AssetUuid>, sub_depth: usize, vis: &mut BTreeSet<AssetUuid>| {
                    build_fields_default_rows(doc, sub_fields, prefix, sub_depth, vis, dtos, enums, catalog)
                };
            let cb = ContainerEditorCallbacks {
                value_prop: &value_prop,
                toggle_expand: &toggle_expand,
                set_option_some: &set_option_some,
                container_add: &container_add,
                container_remove: &container_remove,
                container_move: &container_move,
                render_nested_fields: &render_nested,
            };
            let rows = build_container_items_rows(
                &f.ty,
                &f.default,
                &close_path,
                1,
                &mut visited,
                &doc.expanded_nested,
                dtos,
                enums,
                catalog,
                &cb,
            );
            (format!("Default Values - {} ({})", f.name, f.ty.describe(dtos, enums)), rows)
        }
        _ => return Box::new(bsn! { Node {} }),
    };
    Box::new(bsn! {
        Node {
            margin: UiRect { left: px(28), right: px(6), top: px(4), bottom: px(8) },
            border: UiRect::all(px(1)),
            flex_direction: FlexDirection::Column,
        }
        BackgroundColor(PANEL_BG)
        BorderColor::all(BORDER)
        Children [
            Node {
                padding: UiRect::axes(px(10), px(5)),
                border: UiRect::bottom(px(1)),
                flex_direction: FlexDirection::Row,
                justify_content: JustifyContent::SpaceBetween,
                align_items: AlignItems::Center,
            }
            BorderColor::all(BORDER)
            Children [
                @text(title, ACCENT, 11.0)
                --
                @{tool_button("Close", Action::from(EnumAction::ToggleNestedDefault(close_path)))}
            ]
            --
            Node { flex_direction: FlexDirection::Column }
            Children [ {form_rows} ]
        ]
    })
}

#[allow(clippy::too_many_arguments)]
fn build_fields_default_rows(
    doc: &EnumDoc,
    fields: &[Field],
    prefix: Vec<AssetUuid>,
    depth: usize,
    visited: &mut BTreeSet<AssetUuid>,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
    catalog: &AssetPickerCatalog,
) -> Vec<Box<dyn Scene>> {
    if depth >= MAX_NESTING_DEPTH {
        return vec![Box::new(bsn! {
            Node { padding: UiRect::all(px(6)) }
            Children [ @text("Maximum nesting depth reached.", ERROR_COLOR, 11.0) ]
        })];
    }
    let mut out: Vec<Box<dyn Scene>> = Vec::new();
    for cf in fields {
        let mut path = prefix.clone();
        path.push(cf.id);
        let expected_def = cf.default_with_schemas(dtos, enums);
        let val = doc.get_default_at_path(&path, &expected_def).clone();
        let is_default = val == expected_def;
        let def_str = expected_def.to_short_view_string_with_schemas(&cf.ty, dtos, enums);
        let ty_label = cf.ty.describe(dtos, enums);
        let mut tip = format!("{}: {ty_label} (default: {def_str})", cf.name);
        if !cf.settings.doc.trim().is_empty() {
            tip.push('\n');
            tip.push_str(cf.settings.doc.trim());
        }
        let label_color = if cf.is_name { ACCENT } else { TEXT };
        let reset_btn = enum_default_reset_button(
            path.clone(),
            Action::from(EnumAction::ResetNestedDefault(path.clone())),
            is_default,
            &format!("Reset {} to default ({def_str})", cf.name),
        );
        let value_prop = |p: Vec<AssetUuid>| PropKey::new(EnumProp::NestedFieldDefault(p));
        let toggle_expand =
            |p: Vec<AssetUuid>, sum: String, exp: bool, t: &str| enum_default_toggle_button(p, sum, exp, t);
        let set_option_some = |p: Vec<AssetUuid>, some: bool| Action::from(EnumAction::SetOptionSome { path: p, some });
        let container_add = |p: Vec<AssetUuid>| Action::from(EnumAction::ContainerAdd { path: p });
        let container_remove =
            |p: Vec<AssetUuid>, index: usize| Action::from(EnumAction::ContainerRemove { path: p, index });
        let container_move = |p: Vec<AssetUuid>, index: usize, delta: isize| {
            Action::from(EnumAction::ContainerMove { path: p, index, delta })
        };
        let render_nested =
            |sub_fields: &[Field], sub_prefix: Vec<AssetUuid>, sub_depth: usize, vis: &mut BTreeSet<AssetUuid>| {
                build_fields_default_rows(doc, sub_fields, sub_prefix, sub_depth, vis, dtos, enums, catalog)
            };
        let cb = ContainerEditorCallbacks {
            value_prop: &value_prop,
            toggle_expand: &toggle_expand,
            set_option_some: &set_option_some,
            container_add: &container_add,
            container_remove: &container_remove,
            container_move: &container_move,
            render_nested_fields: &render_nested,
        };
        let (widget, sub_rows) = build_inline_value_and_subrows(
            &cf.ty,
            &val,
            &cf.settings,
            &path,
            depth,
            visited,
            &doc.expanded_nested,
            dtos,
            enums,
            catalog,
            &cb,
        );
        out.push(dto_form_row(&cf.name, label_color, cf.settings.required, Tooltip(tip), widget, reset_btn));
        out.extend(sub_rows);
    }
    out
}

fn field_settings_subpane(f: &Field) -> Box<dyn Scene> {
    let mut form_rows: Vec<Box<dyn Scene>> = Vec::new();

    let doc_is_default = f.settings.doc.is_empty();
    form_rows.push(dto_form_row(
        "doc",
        if doc_is_default { TEXT } else { ACCENT },
        false,
        Tooltip("doc: String (default: \"\")\nRust /// doc comment for generated payload field".to_string()),
        text_hint(EnumProp::FieldDoc(f.id), f.settings.doc.clone(), "Rust /// doc comment for generated payload field"),
        dto_reset_button(
            Action::from(EnumAction::ResetConstraint { id: f.id, field: ConstraintField::Doc }),
            doc_is_default,
            "\"\"",
        ),
    ));

    if f.ty.is_numeric() {
        let prec = if is_integer_type(&f.ty) { 0 } else { 2 };
        let opt_ty = if is_integer_type(&f.ty) { "Option<i64>" } else { "Option<f64>" };

        form_rows.push(dto_option_row(
            "min",
            Tooltip(format!("min: {opt_ty} (default: None)\nMinimum allowed value")),
            f.settings.min.is_some(),
            Action::from(EnumAction::SetConstraintOption { id: f.id, field: ConstraintField::Min, some: false }),
            Action::from(EnumAction::SetConstraintOption { id: f.id, field: ConstraintField::Min, some: true }),
            f.settings.min.map(|v| number_input(EnumProp::FieldMin(f.id), 0, v as f32, None, None, prec)),
            Action::from(EnumAction::ResetConstraint { id: f.id, field: ConstraintField::Min }),
        ));

        form_rows.push(dto_option_row(
            "max",
            Tooltip(format!("max: {opt_ty} (default: None)\nMaximum allowed value")),
            f.settings.max.is_some(),
            Action::from(EnumAction::SetConstraintOption { id: f.id, field: ConstraintField::Max, some: false }),
            Action::from(EnumAction::SetConstraintOption { id: f.id, field: ConstraintField::Max, some: true }),
            f.settings.max.map(|v| number_input(EnumProp::FieldMax(f.id), 0, v as f32, None, None, prec)),
            Action::from(EnumAction::ResetConstraint { id: f.id, field: ConstraintField::Max }),
        ));

        form_rows.push(dto_option_row(
            "step",
            Tooltip(format!("step: {opt_ty} (default: None)\nStep increment (> 0)")),
            f.settings.step.is_some(),
            Action::from(EnumAction::SetConstraintOption { id: f.id, field: ConstraintField::Step, some: false }),
            Action::from(EnumAction::SetConstraintOption { id: f.id, field: ConstraintField::Step, some: true }),
            f.settings.step.map(|v| number_input(EnumProp::FieldStep(f.id), 0, v as f32, None, None, prec)),
            Action::from(EnumAction::ResetConstraint { id: f.id, field: ConstraintField::Step }),
        ));

        form_rows.push(dto_form_row(
            "slider",
            if f.settings.slider { ACCENT } else { TEXT },
            false,
            Tooltip("slider: bool (default: false)\nShow horizontal slider (enables min & max if unset)".to_string()),
            checkbox_w(EnumProp::FieldSlider(f.id), f.settings.slider),
            dto_reset_button(
                Action::from(EnumAction::ResetConstraint { id: f.id, field: ConstraintField::Slider }),
                !f.settings.slider,
                "false",
            ),
        ));
    }

    if f.ty.is_string() {
        form_rows.push(dto_form_row(
            "multiline",
            if f.settings.multiline { ACCENT } else { TEXT },
            false,
            Tooltip("multiline: bool (default: false)\nUse multiline text area".to_string()),
            checkbox_w(EnumProp::FieldMultiline(f.id), f.settings.multiline),
            dto_reset_button(
                Action::from(EnumAction::ResetConstraint { id: f.id, field: ConstraintField::Multiline }),
                !f.settings.multiline,
                "false",
            ),
        ));

        form_rows.push(dto_option_row(
            "max_len",
            Tooltip("max_len: Option<usize> (default: None)\nMaximum character count".to_string()),
            f.settings.max_len.is_some(),
            Action::from(EnumAction::SetConstraintOption { id: f.id, field: ConstraintField::MaxLen, some: false }),
            Action::from(EnumAction::SetConstraintOption { id: f.id, field: ConstraintField::MaxLen, some: true }),
            f.settings
                .max_len
                .map(|ml| number_input(EnumProp::FieldMaxLen(f.id), 0, ml as f32, None, Some((1.0, 65536.0)), 0)),
            Action::from(EnumAction::ResetConstraint { id: f.id, field: ConstraintField::MaxLen }),
        ));
    }

    if f.ty.is_string() || f.ty.is_soft_ref() || f.ty.is_asset_ref() || f.ty.is_container() {
        form_rows.push(dto_form_row(
            "required",
            if f.settings.required { ACCENT } else { TEXT },
            false,
            Tooltip(
                "required: bool (default: false)\nRequire non-empty string, container, or populated reference in collections"
                    .to_string(),
            ),
            checkbox_w(EnumProp::FieldRequired(f.id), f.settings.required),
            dto_reset_button(
                Action::from(EnumAction::ResetConstraint { id: f.id, field: ConstraintField::Required }),
                !f.settings.required,
                "false",
            ),
        ));
    }

    let title = format!("Constraints - {} ({}) | UUID {}", f.name, f.ty.label(), f.id);
    Box::new(bsn! {
        Node {
            margin: UiRect { left: px(28), right: px(6), top: px(4), bottom: px(8) },
            border: UiRect::all(px(1)),
            flex_direction: FlexDirection::Column,
        }
        BackgroundColor(PANEL_BG)
        BorderColor::all(BORDER)
        Children [
            Node {
                padding: UiRect::axes(px(10), px(5)),
                border: UiRect::bottom(px(1)),
                flex_direction: FlexDirection::Row,
                justify_content: JustifyContent::SpaceBetween,
                align_items: AlignItems::Center,
            }
            BorderColor::all(BORDER)
            Children [
                @text(title, ACCENT, 11.0)
                --
                @{tool_button("Close", Action::from(EnumAction::ToggleConstraints(f.id)))}
            ]
            --
            Node { flex_direction: FlexDirection::Column }
            Children [ {form_rows} ]
        ]
    })
}

fn redraw_topbar(
    editor: Res<Editor>,
    bar: Query<Entity, With<EnumTopbar>>,
    mut last: Local<Option<(Option<usize>, String, bool)>>,
    mut commands: Commands,
) {
    let Some(doc) = active_enum(&editor) else {
        *last = None;
        return;
    };
    let key = (editor.active, doc.path.clone(), doc.en.is_unit_only());
    if *last == Some(key.clone()) {
        return;
    }
    let Ok(bar) = bar.single() else { return };
    *last = Some(key);
    commands.entity(bar).despawn_children();

    let items: Vec<Box<dyn Scene>> = vec![
        Box::new(icon_tool_button(
            IconId::Plus,
            "Add Variant",
            Action::from(EnumAction::AddVariant),
            "Add a new variant to this enum schema",
        )),
        Box::new(icon_tool_button(
            IconId::Plus,
            "Add Payload Field",
            Action::from(EnumAction::AddField),
            "Add a payload field to the selected variant",
        )),
        Box::new(icon_tool_button(
            IconId::Code,
            "Preview Rust",
            Action::from(DataStep::OpenRustPreview(doc.aref.clone())),
            "Open the generated Rust enum preview",
        )),
    ];
    for item in items {
        commands.spawn_scene(bsn! { @{item} ChildOf(bar) });
    }
}

fn hotkey_row(key: &'static str, desc: &'static str) -> Box<dyn Scene> {
    Box::new(bsn! {
        Node {
            flex_direction: FlexDirection::Row,
            justify_content: JustifyContent::SpaceBetween,
            align_items: AlignItems::Center,
            padding: UiRect::vertical(px(2)),
        }
        Children [
            Node {
                padding: UiRect::axes(px(5), px(1)),
                border: UiRect::all(px(1)),
            }
            BackgroundColor(PANEL_BG)
            BorderColor::all(ROW_LINE)
            Children [ @text(key, TEXT, 11.0) ]
            --
            @text(desc, TEXT_DIM, 11.0)
        ]
    })
}

fn redraw_sidebar(
    editor: Res<Editor>,
    body: Query<Entity, With<EnumSidebar>>,
    mut key: Local<SidebarKey>,
    mut commands: Commands,
) {
    let Some(doc) = active_enum(&editor) else {
        key.forget();
        return;
    };
    let path = doc.path.clone();
    let doc_text = doc.en.doc.clone();
    fill_sidebar_with(&mut commands, &body, &mut key, SidebarKey::now(&editor), || {
        let about = format!(
            "# Enum / Union (`{path}.enum.ron`)\n\
            Defines a strongly-typed Rust `enum` generated into `crates/quillshade/src/data/enums/{path}.rs`.\n\n\
            - **Unit Variants** (0 fields) derive `Copy, PartialEq, Eq, Hash, PartialOrd, Ord` when all variants are unit.\n\
            - **Struct Variants** (1+ fields) emit named payload fields and collapsible sub-forms in collections."
        );
        vec![
            subpane_box(
                "About",
                vec![
                    Box::new(rich_text(&about)),
                    Box::new(text("Doc Comment (`///`):", TEXT_DIM, 11.0)),
                    text_hint(EnumProp::EnumDoc, doc_text, "Rust /// doc comment for this enum"),
                ],
            ),
            subpane_box(
                "Hotkeys",
                vec![
                    hotkey_row("Enter", "Add variant / payload field"),
                    hotkey_row("Delete", "Delete selected item"),
                    hotkey_row("Ctrl+Z / Y", "Undo / Redo"),
                    hotkey_row("Ctrl+S", "Save schema"),
                ],
            ),
        ]
    });
}

fn redraw_body(
    editor: Res<Editor>,
    body: Query<Entity, With<EnumViewBody>>,
    mut last: Local<Option<(Option<usize>, u64)>>,
    mut last_constraints: Local<Option<(Option<usize>, u64)>>,
    mut commands: Commands,
) {
    let Some(doc) = active_enum(&editor) else {
        *last = None;
        *last_constraints = None;
        return;
    };
    let key = (editor.active, doc.layout);
    if *last == Some(key) {
        return;
    }
    let Ok(body) = body.single() else { return };
    *last = Some(key);
    *last_constraints = Some((editor.active, doc.constraints_layout));
    commands.entity(body).despawn_children();

    let dtos = resolve_doc_dtos(&editor, doc);
    let enums = resolve_doc_enums(&editor, doc);
    let catalog = AssetPickerCatalog::from_editor(&editor);
    let mut sections: Vec<Box<dyn Scene>> = vec![variants_subpane(doc)];
    if let Some(variant) = doc.active_variant() {
        sections.push(variant_payload_subpane(doc, variant, &dtos, &enums, &catalog));
    }
    for s in sections {
        commands.spawn_scene(bsn! { @{s} ChildOf(body) });
    }
}

fn redraw_constraints_slots(
    editor: Res<Editor>,
    slots: Query<(Entity, &EnumFieldConstraintsSlot)>,
    mut last: Local<Option<(Option<usize>, u64)>>,
    mut commands: Commands,
) {
    let Some(doc) = active_enum(&editor) else {
        *last = None;
        return;
    };
    let key = (editor.active, doc.constraints_layout);
    if *last == Some(key) {
        return;
    }
    if slots.is_empty() {
        return;
    }
    *last = Some(key);
    let dtos = resolve_doc_dtos(&editor, doc);
    let enums = resolve_doc_enums(&editor, doc);
    let catalog = AssetPickerCatalog::from_editor(&editor);
    for (slot_entity, EnumFieldConstraintsSlot(fid)) in &slots {
        commands.entity(slot_entity).despawn_children();
        if let Some((_, field)) = doc.en.find_variant_field(*fid) {
            if field_has_expanded_default_subpane(doc, field) {
                let pane = enum_nested_default_subpane(doc, field, &dtos, &enums, &catalog);
                commands.spawn_scene(bsn! { @{pane} ChildOf(slot_entity) });
            }
            if doc.expanded_constraints == Some(*fid) {
                let pane = field_settings_subpane(field);
                commands.spawn_scene(bsn! { @{pane} ChildOf(slot_entity) });
            }
        }
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn sync_fields(
    editor: Res<Editor>,
    focus: Option<Res<InputFocus>>,
    numbers: Query<(Entity, &PropField, &NumberInputValue)>,
    limits_q: Query<(Entity, &PropField, Option<&SoftLimit>, Option<&HardLimit>)>,
    pickers: Query<(Entity, &PropField, &ColorInputValue)>,
    checkboxes: Query<(Entity, &PropField, Has<Checked>), With<FeathersCheckbox>>,
    mut var_rows: Query<(&EnumVariantRow, &mut Node, &mut BackgroundColor), Without<EnumFieldRow>>,
    mut field_rows: Query<(&EnumFieldRow, &mut Node, &mut BackgroundColor), Without<EnumVariantRow>>,
    mut btn_texts: Query<(&EnumConstraintsBtnText, &mut Text, &mut TextColor), Without<EnumDefaultSummaryText>>,
    mut summary_texts: Query<(&EnumDefaultSummaryText, &mut Text, &mut TextColor), Without<EnumConstraintsBtnText>>,
    mut reset_icons: Query<(&EnumDefaultResetIcon, &mut ImageNode)>,
    parents: Query<&ChildOf>,
    mut commands: Commands,
) {
    if !editor.is_changed() {
        return;
    }
    let Some(doc) = active_enum(&editor) else {
        return;
    };
    let dtos = resolve_doc_dtos(&editor, doc);
    let enums = resolve_doc_enums(&editor, doc);

    let selected_vid = doc.selected_variant.or_else(|| doc.en.variants.first().map(|v| v.id));
    for (EnumVariantRow(vid), mut node, mut bg) in &mut var_rows {
        let selected = selected_vid == Some(*vid);
        let want_bg = if selected { SELECTED_ROW_BG } else { Color::NONE };
        let want_left = px(if selected { 2 } else { 0 });
        if bg.0 != want_bg {
            bg.0 = want_bg;
        }
        if node.border.left != want_left {
            node.border.left = want_left;
        }
    }

    for (EnumFieldRow(fid), mut node, mut bg) in &mut field_rows {
        let selected = doc.selected_field == Some(*fid);
        let want_bg = if selected { SELECTED_ROW_BG } else { Color::NONE };
        let want_left = px(if selected { 2 } else { 0 });
        if bg.0 != want_bg {
            bg.0 = want_bg;
        }
        if node.border.left != want_left {
            node.border.left = want_left;
        }
    }

    for (EnumConstraintsBtnText(fid), mut txt, mut color) in &mut btn_texts {
        let has_some = doc.en.find_variant_field(*fid).is_some_and(|(_, f)| !f.settings.is_default());
        let (want_txt, want_color) = if has_some { ("Some", ACCENT) } else { ("None", TEXT_DIM) };
        if txt.0 != want_txt {
            txt.0 = want_txt.to_string();
        }
        if color.0 != want_color {
            color.0 = want_color;
        }
    }

    for (EnumDefaultSummaryText { path }, mut txt, mut color) in &mut summary_texts {
        if let Some(&first) = path.first()
            && let Some((variant, _)) = doc.en.find_variant_field(first)
            && let Some((ty, _, def, _)) = doc.en.variant_field_meta_at_path(variant.id, path, &dtos, &enums, true)
        {
            let val = doc.get_default_at_path(path, &def);
            let expanded = doc.expanded_nested.contains(path);
            let want_txt = if ty.is_container() {
                container_summary_label(&ty, val, &dtos, &enums)
            } else if let FieldType::Option(ref inner_ty) = ty {
                match val.as_option().flatten() {
                    Some(inner_val) => inner_val.to_short_view_string_with_schemas(inner_ty.as_ref(), &dtos, &enums),
                    None => "None".to_string(),
                }
            } else {
                val.to_short_view_string_with_schemas(&ty, &dtos, &enums)
            };
            let want_color = if expanded { ACCENT } else { TEXT };
            if txt.0 != want_txt {
                txt.0 = want_txt;
            }
            if color.0 != want_color {
                color.0 = want_color;
            }
        }
    }

    for (EnumDefaultResetIcon { path }, mut img) in &mut reset_icons {
        if let Some(&first) = path.first()
            && let Some((variant, _)) = doc.en.find_variant_field(first)
            && let Some((_, _, def, _)) = doc.en.variant_field_meta_at_path(variant.id, path, &dtos, &enums, true)
        {
            let val = doc.get_default_at_path(path, &def);
            let want = reset_icon_color(*val == def);
            if img.color != want {
                img.color = want;
            }
        }
    }

    for (e, field, cur_soft, cur_hard) in &limits_q {
        let path = match field.target.downcast::<EnumProp>() {
            Some(EnumProp::FieldDefault(fid)) => vec![*fid],
            Some(EnumProp::NestedFieldDefault(p)) => p.clone(),
            _ => continue,
        };
        let Some(&first) = path.first() else { continue };
        let Some((variant, _)) = doc.en.find_variant_field(first) else { continue };
        let Some((ty, settings, _, _)) = doc.en.variant_field_meta_at_path(variant.id, &path, &dtos, &enums, true)
        else {
            continue;
        };
        if matches!(ty, FieldType::Quat) {
            continue;
        }
        let (want_soft, want_hard) = numeric_limits(&ty, &settings);
        let want_soft_cmp = want_soft.map(|(lo, hi)| SoftLimit(NumberInputRange::F32(lo..=hi)));
        let want_hard_cmp = want_hard.map(|(lo, hi)| HardLimit(NumberInputRange::F32(lo..=hi)));
        if cur_soft.map(|s| s.0.clone()) != want_soft_cmp.as_ref().map(|s| s.0.clone()) {
            if let Some(s) = want_soft_cmp {
                commands.entity(e).try_insert(s);
            } else {
                commands.entity(e).remove::<SoftLimit>();
            }
        }
        if cur_hard.map(|h| h.0.clone()) != want_hard_cmp.as_ref().map(|h| h.0.clone()) {
            if let Some(h) = want_hard_cmp {
                commands.entity(e).try_insert(h);
            } else {
                commands.entity(e).remove::<HardLimit>();
            }
        }
    }

    sync_data_value_widgets(
        &editor,
        focus.as_deref(),
        &numbers,
        &pickers,
        &checkboxes,
        &parents,
        &mut commands,
        |key| key.downcast::<EnumProp>().is_some(),
    );
}

fn commit_color(ev: On<ValueChange<Color>>, fields: Query<&PropField>, mut actions: ResMut<Actions>) {
    if ev.is_final
        && fields.get(ev.source).is_ok_and(|f| {
            matches!(f.target.downcast(), Some(EnumProp::FieldDefault(_) | EnumProp::NestedFieldDefault(_)))
        })
    {
        actions.push(Action::Tab(TabAction::CommitInline));
    }
}

fn add_on_enter(ev: On<FocusedInput<KeyboardInput>>, fields: Query<&PropField>, mut actions: ResMut<Actions>) {
    if ev.input.key_code == KeyCode::Enter && ev.input.state.is_pressed() {
        if fields.get(ev.focused_entity).is_ok_and(|f| f.target == PropKey::new(EnumProp::DraftVariantName)) {
            actions.push(Action::from(EnumAction::AddVariant));
        } else if fields.get(ev.focused_entity).is_ok_and(|f| f.target == PropKey::new(EnumProp::DraftFieldName)) {
            actions.push(Action::from(EnumAction::AddField));
        }
    }
}
