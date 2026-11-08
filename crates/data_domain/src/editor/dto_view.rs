//! `.dto.ron` visual schema editor (`DtoEditor` + `dto_view`): topbar, Add Field bar, ordered fields table
//! with inline `Constraints` (`FieldSettings`) internal-DTO subpane, and right sidebar (`About` + `Hotkeys`).

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
    DtoAsset, EnumAsset, Field, FieldType, GameAssetKind, MAX_NESTING_DEPTH, path_step_option_some,
};

use super::dto_doc::{ConstraintField, DTO_VIEW, DtoAction, DtoDoc, DtoProp};
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
pub struct DtoTopbar;

#[derive(Component, Default, Clone)]
pub struct DtoSidebar;

#[derive(Component, Default, Clone)]
pub struct DtoViewBody;

#[derive(Component, Default, Clone, Copy)]
pub struct DtoFieldRow(pub AssetUuid);

#[derive(Component, Default, Clone, Copy)]
pub struct DtoFieldConstraintsSlot(pub AssetUuid);

#[derive(Component, Default, Clone, Copy)]
pub struct DtoConstraintsBtnText(pub AssetUuid);

/// Live-synced `{ShortViewString ▾}` / `{ShortViewString ▸}` label for a `FieldType::Dto` default value in [`DtoEditor`].
#[derive(Component, Default, Clone, Debug, PartialEq, Eq)]
pub struct DtoDefaultSummaryText {
    pub path: Vec<AssetUuid>,
}

/// Per-field reset icon in [`DtoEditor`] tinted [`ACCENT`] when the default value at `path` differs from its schema default.
#[derive(Component, Default, Clone, Debug, PartialEq, Eq)]
pub struct DtoDefaultResetIcon {
    pub path: Vec<AssetUuid>,
}

pub struct DtoEditor;

impl EditorAreas for DtoEditor {
    const VIEW: ViewKindId = DTO_VIEW;
    const SIDEBAR_TITLE: &'static str = "Schema";

    fn topbar() -> Option<impl Scene> {
        Some(bsn! { @bar_row() DtoTopbar })
    }

    fn sidebar() -> Option<impl Scene> {
        Some(bsn! { @sidebar_body() DtoSidebar })
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

pub fn dto_view() -> impl Scene {
    bsn! {
        ViewOf(DTO_VIEW)
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
            DtoViewBody
            Node {
                padding: UiRect::new(px(12), px(12), px(10), px(56)),
                row_gap: px(10),
            }
        ]
    }
}

fn active_dto(editor: &Editor) -> Option<&DtoDoc> {
    editor.active_doc::<DtoDoc>()
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

fn field_type_select(target: DtoProp, current: &FieldType) -> Box<dyn Scene> {
    let (labels, values) = field_type_options();
    let idx = FieldType::all().iter().position(|t| t.same_kind(current)).unwrap_or(0);
    select_values(target, labels, values, idx)
}

fn game_asset_kind_select(target: DtoProp, current: GameAssetKind) -> Box<dyn Scene> {
    let labels = GameAssetKind::ALL.iter().map(|k| k.label().to_string()).collect();
    let values = GameAssetKind::ALL.iter().map(|k| k.label().to_string()).collect();
    let idx = GameAssetKind::ALL.iter().position(|&k| k == current).unwrap_or(0);
    select_values(target, labels, values, idx)
}

fn resolve_doc_dtos(editor: &Editor, doc: &DtoDoc) -> BTreeMap<String, DtoAsset> {
    let mut dtos = doc.cached_dtos.clone();
    if let Some(ds) = editor.domains.get::<DataState>() {
        for (p, d) in &ds.assets.dtos {
            dtos.insert(p.clone(), d.clone());
        }
    }
    for tab in &editor.tabs {
        if let Some(doc_entry) = tab.get::<DtoDoc>() {
            dtos.insert(doc_entry.path.clone(), doc_entry.dto.clone());
        }
    }
    dtos.insert(doc.path.clone(), doc.dto.clone());
    dtos
}

fn resolve_doc_enums(editor: &Editor, doc: &DtoDoc) -> BTreeMap<String, EnumAsset> {
    let mut enums = doc.cached_enums.clone();
    if let Some(ds) = editor.domains.get::<DataState>() {
        for (p, e) in &ds.assets.enums {
            enums.insert(p.clone(), e.clone());
        }
    }
    for tab in &editor.tabs {
        if let Some(doc_entry) = tab.get::<super::enum_doc::EnumDoc>() {
            enums.insert(doc_entry.path.clone(), doc_entry.en.clone());
        }
    }
    enums
}

fn dto_target_select(
    target: DtoProp,
    current: AssetUuid,
    self_id: AssetUuid,
    allow_self: bool,
    dtos: &BTreeMap<String, DtoAsset>,
) -> Box<dyn Scene> {
    let mut labels = vec!["(Select DTO)".to_string()];
    let mut values = vec![AssetUuid::default().to_string()];
    for (path, dto) in dtos {
        if dto.id.is_nil() || (!allow_self && dto.id == self_id) {
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
    target: DtoProp,
    current: AssetUuid,
    self_id: AssetUuid,
    allow_self: bool,
    enums: &BTreeMap<String, EnumAsset>,
) -> Box<dyn Scene> {
    let mut labels = vec!["(Select Enum)".to_string()];
    let mut values = vec![AssetUuid::default().to_string()];
    for (path, en) in enums {
        if en.id.is_nil() || (!allow_self && en.id == self_id) {
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

fn dto_default_reset_button(path: Vec<AssetUuid>, action: Action, is_default: bool, tooltip: &str) -> Box<dyn Scene> {
    let color = reset_icon_color(is_default);
    let tag = DtoDefaultResetIcon { path };
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

fn dto_default_toggle_button(path: Vec<AssetUuid>, summary: String, expanded: bool, tooltip: &str) -> Box<dyn Scene> {
    let color = if expanded { ACCENT } else { TEXT };
    let caret = if expanded { IconId::CaretDown } else { IconId::CaretRight };
    let act = OnActivate(Action::from(DtoAction::ToggleDtoDefault(path.clone())));
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
            DtoDefaultSummaryText { path }
            --
            @icon(caret, 12.0, color)
        ]
    })
}

fn category_select(target: DtoProp, current: &str, categories: &[String]) -> Box<dyn Scene> {
    let trimmed = current.trim();
    let mut labels = vec!["General".to_string()];
    let mut values = vec![String::new()];
    for c in categories {
        labels.push(c.clone());
        values.push(c.clone());
    }
    if !trimmed.is_empty() && !values.iter().any(|v| v == trimmed) {
        labels.push(trimmed.to_string());
        values.push(trimmed.to_string());
    }
    let idx = values.iter().position(|v| v == trimmed).unwrap_or(0);
    select_values(target, labels, values, idx)
}

fn constraints_button(field_id: AssetUuid, has_some: bool) -> Box<dyn Scene> {
    let (label, color) = if has_some { ("Some", ACCENT) } else { ("None", TEXT_DIM) };
    let act = OnActivate(Action::from(DtoAction::ToggleConstraints(field_id)));
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
            DtoConstraintsBtnText(field_id)
        ]
    })
}

fn append_draft_secondary_type_items(
    bar_items: &mut Vec<Box<dyn Scene>>,
    ty: &FieldType,
    self_dto_id: AssetUuid,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
) {
    if let Some(target_id) = ty.dto_target().or_else(|| ty.soft_ref_target()) {
        let allow_self = ty.is_soft_ref();
        bar_items.push(Box::new(bsn! {
            Node { width: px(160), flex_shrink: 0.0 }
            Children [ @{dto_target_select(DtoProp::DraftDtoTarget, target_id, self_dto_id, allow_self, dtos)} ]
        }));
    } else if let Some(target_id) = ty.enum_target() {
        bar_items.push(Box::new(bsn! {
            Node { width: px(160), flex_shrink: 0.0 }
            Children [ @{enum_target_select(DtoProp::DraftEnumTarget, target_id, self_dto_id, true, enums)} ]
        }));
    } else if let FieldType::GameAsset(kind) = *ty {
        bar_items.push(Box::new(bsn! {
            Node { width: px(120), flex_shrink: 0.0 }
            Children [ @{game_asset_kind_select(DtoProp::DraftGameAssetKind, kind)} ]
        }));
    } else if let FieldType::Option(inner) | FieldType::Array(inner) | FieldType::Set(inner) = ty {
        let for_set = ty.is_set();
        let allow_self = ty.is_option() || inner.is_soft_ref();
        bar_items.push(Box::new(bsn! {
            Node { width: px(120), flex_shrink: 0.0 }
            Children [ @{inner_type_select(DtoProp::DraftInnerType, inner, for_set)} ]
        }));
        if let Some(target_sel) =
            leaf_target_select(DtoProp::DraftInnerTarget, inner, Some(self_dto_id), allow_self, dtos, enums)
        {
            bar_items.push(Box::new(bsn! {
                Node { width: px(150), flex_shrink: 0.0 }
                Children [ @{target_sel} ]
            }));
        }
    } else if let FieldType::Map(key_ty, val_ty) = ty {
        bar_items.push(Box::new(bsn! {
            Node { width: px(110), flex_shrink: 0.0 }
            Children [ @{key_type_select(DtoProp::DraftKeyType, key_ty)} ]
        }));
        if let Some(key_target_sel) =
            leaf_target_select(DtoProp::DraftKeyTarget, key_ty, Some(self_dto_id), true, dtos, enums)
        {
            bar_items.push(Box::new(bsn! {
                Node { width: px(140), flex_shrink: 0.0 }
                Children [ @{key_target_sel} ]
            }));
        }
        bar_items.push(Box::new(text("→", TEXT_DIM, 11.0)));
        bar_items.push(Box::new(bsn! {
            Node { width: px(120), flex_shrink: 0.0 }
            Children [ @{inner_type_select(DtoProp::DraftInnerType, val_ty, false)} ]
        }));
        if let Some(val_target_sel) =
            leaf_target_select(DtoProp::DraftInnerTarget, val_ty, Some(self_dto_id), val_ty.is_soft_ref(), dtos, enums)
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
    self_dto_id: AssetUuid,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
) -> Box<dyn Scene> {
    let primary = field_type_select(DtoProp::FieldType(f.id), &f.ty);
    let mut col_items: Vec<Box<dyn Scene>> = vec![primary];
    if let Some(target_id) = f.ty.dto_target().or_else(|| f.ty.soft_ref_target()) {
        let allow_self = f.ty.is_soft_ref();
        col_items.push(dto_target_select(DtoProp::FieldDtoTarget(f.id), target_id, self_dto_id, allow_self, dtos));
    } else if let Some(target_id) = f.ty.enum_target() {
        col_items.push(enum_target_select(DtoProp::FieldEnumTarget(f.id), target_id, self_dto_id, true, enums));
    } else if let FieldType::GameAsset(kind) = f.ty {
        col_items.push(game_asset_kind_select(DtoProp::FieldGameAssetKind(f.id), kind));
    } else if let FieldType::Option(inner) | FieldType::Array(inner) | FieldType::Set(inner) = &f.ty {
        let for_set = f.ty.is_set();
        let allow_self = f.ty.is_option() || inner.is_soft_ref();
        col_items.push(inner_type_select(DtoProp::FieldInnerType(f.id), inner, for_set));
        if let Some(target_sel) =
            leaf_target_select(DtoProp::FieldInnerTarget(f.id), inner, Some(self_dto_id), allow_self, dtos, enums)
        {
            col_items.push(target_sel);
        }
    } else if let FieldType::Map(key_ty, val_ty) = &f.ty {
        col_items.push(key_type_select(DtoProp::FieldKeyType(f.id), key_ty));
        if let Some(key_target_sel) =
            leaf_target_select(DtoProp::FieldKeyTarget(f.id), key_ty, Some(self_dto_id), true, dtos, enums)
        {
            col_items.push(key_target_sel);
        }
        col_items.push(inner_type_select(DtoProp::FieldInnerType(f.id), val_ty, false));
        if let Some(val_target_sel) = leaf_target_select(
            DtoProp::FieldInnerTarget(f.id),
            val_ty,
            Some(self_dto_id),
            val_ty.is_soft_ref(),
            dtos,
            enums,
        ) {
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

fn add_field_subpane(
    doc: &DtoDoc,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
) -> Box<dyn Scene> {
    let mut bar_items: Vec<Box<dyn Scene>> = vec![
        Box::new(text("New Field:", TEXT_DIM, 11.0)),
        Box::new(bsn! {
            Node { width: px(160), flex_shrink: 0.0 }
            Children [ @{text_hint(DtoProp::DraftName, doc.draft_name.clone(), "field_name")} ]
        }),
        Box::new(bsn! {
            Node { width: px(130), flex_shrink: 0.0 }
            Children [ @{field_type_select(DtoProp::DraftType, &doc.draft_ty)} ]
        }),
    ];
    append_draft_secondary_type_items(&mut bar_items, &doc.draft_ty, doc.dto.id, dtos, enums);
    bar_items.push(Box::new(bsn! {
        Node { width: px(140), flex_shrink: 0.0 }
        Children [ @{text_hint(DtoProp::DraftCategory, doc.draft_category.clone(), "Category")} ]
    }));
    let cats = doc.category_names();
    if !cats.is_empty() {
        let mut labels = vec!["(Pick Category)".to_string()];
        let mut values = vec![String::new()];
        for c in &cats {
            labels.push(c.clone());
            values.push(c.clone());
        }
        let cur = cats.iter().position(|c| c == doc.draft_category.trim()).map(|i| i + 1).unwrap_or(0);
        bar_items.push(Box::new(bsn! {
            Node { width: px(130), flex_shrink: 0.0 }
            Children [ @{select_values(DtoProp::DraftCategoryPick, labels, values, cur)} ]
        }));
    }
    bar_items.push(checkbox("Name", DtoProp::DraftIsName, doc.draft_is_name));
    bar_items.push(Box::new(icon_tool_button(
        IconId::Plus,
        "Add Field",
        Action::from(DtoAction::AddField),
        "Add field to this DTO schema",
    )));

    let mut rows: Vec<Box<dyn Scene>> = vec![Box::new(bsn! {
        Node {
            flex_direction: FlexDirection::Row,
            flex_wrap: FlexWrap::Wrap,
            align_items: AlignItems::Center,
            column_gap: px(8),
            row_gap: px(6),
        }
        Children [ {bar_items} ]
    })];
    if let Some(err) = &doc.form_error {
        rows.push(Box::new(text(err.clone(), ERROR_COLOR, 12.0)));
    }
    subpane_box("Add Field", rows)
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

fn field_has_expanded_default_subpane(doc: &DtoDoc, f: &Field) -> bool {
    let root_path = vec![f.id];
    match &f.ty {
        FieldType::Dto(_) | FieldType::Enum(_) | FieldType::Array(_) | FieldType::Set(_) | FieldType::Map(_, _) => {
            doc.expanded_dtos.contains(&root_path)
        }
        FieldType::Option(inner)
            if matches!(inner.as_ref(), FieldType::Dto(_) | FieldType::Enum(_))
                && f.default.as_option().flatten().is_some() =>
        {
            let some_path = vec![f.id, path_step_option_some()];
            doc.expanded_dtos.contains(&some_path)
        }
        _ => false,
    }
}

fn fields_table_subpane(
    doc: &DtoDoc,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
    catalog: &AssetPickerCatalog,
) -> Box<dyn Scene> {
    let hdr = |label: &'static str| -> Box<dyn Scene> { Box::new(text(label, TEXT_DIM, 11.0)) };
    let mut rows: Vec<Box<dyn Scene>> = vec![Box::new(bsn! {
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
            @{cell(Some(132.0), hdr("Field Name"))}
            --
            @{cell(Some(140.0), hdr("Field Type"))}
            --
            @{cell(Some(114.0), hdr("Category"))}
            --
            @{cell(None, hdr("Default"))}
            --
            @{cell(Some(84.0), hdr("Constraints"))}
            --
            @{cell(Some(84.0), hdr("Order"))}
        ]
    })];

    if doc.dto.fields.is_empty() {
        rows.push(Box::new(bsn! {
            Node { padding: UiRect::all(px(8)) }
            Children [ @text("No fields in this DTO yet. Use the Add Field bar above to add one.", TEXT_DIM, 12.0) ]
        }));
    } else {
        let cats = doc.category_names();
        for (idx, f) in doc.dto.fields.iter().enumerate() {
            let selected = doc.selected_field == Some(f.id);
            let bg = if selected { SELECTED_ROW_BG } else { Color::NONE };
            let left_border = if selected { 2.0 } else { 0.0 };
            let select_act = Action::from(DtoAction::SelectField(Some(f.id)));
            let idx_btn = Box::new(tool_button(format!("{}", idx + 1), select_act));
            let name_cb = checkbox_w(DtoProp::FieldIsName(f.id), f.is_name);
            let name_in = text_w(DtoProp::FieldName(f.id), f.name.clone(), false);
            let type_in = field_type_column_scene(f, doc.dto.id, dtos, enums);
            let cat_in = category_select(DtoProp::FieldCategory(f.id), &f.category, &cats);
            let path = vec![f.id];
            let expected_default = f.ty.default_value_with_schemas(dtos, enums);
            let is_default = f.default == expected_default;
            let value_prop = |p: Vec<AssetUuid>| {
                if p.len() == 1 {
                    PropKey::new(DtoProp::FieldDefault(p[0]))
                } else {
                    PropKey::new(DtoProp::NestedFieldDefault(p))
                }
            };
            let toggle_expand =
                |p: Vec<AssetUuid>, sum: String, exp: bool, tip: &str| dto_default_toggle_button(p, sum, exp, tip);
            let set_option_some =
                |p: Vec<AssetUuid>, some: bool| Action::from(DtoAction::SetOptionSome { path: p, some });
            let container_add = |p: Vec<AssetUuid>| Action::from(DtoAction::ContainerAdd { path: p });
            let container_remove =
                |p: Vec<AssetUuid>, index: usize| Action::from(DtoAction::ContainerRemove { path: p, index });
            let container_move = |p: Vec<AssetUuid>, index: usize, delta: isize| {
                Action::from(DtoAction::ContainerMove { path: p, index, delta })
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
                &doc.expanded_dtos,
                dtos,
                enums,
                catalog,
                &cb,
            );
            let reset_btn = dto_default_reset_button(
                path.clone(),
                Action::from(DtoAction::ResetFieldDefault(f.id)),
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
                        Action::from(DtoAction::MoveField { id: f.id, delta: -1 }),
                        "Move field up",
                    )}
                    --
                    @{icon_button(
                        IconId::ArrowDown,
                        Action::from(DtoAction::MoveField { id: f.id, delta: 1 }),
                        "Move field down",
                    )}
                    --
                    @{icon_button(
                        IconId::Delete,
                        Action::from(DtoAction::DeleteField(f.id)),
                        "Delete field",
                    )}
                ]
            });

            let mut slot_children: Vec<Box<dyn Scene>> = Vec::new();
            if field_has_expanded_default_subpane(doc, f) {
                slot_children.push(dto_default_subpane(doc, f, dtos, enums, catalog));
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
                    DtoFieldRow(fid)
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
                        @{cell(Some(132.0), name_in)}
                        --
                        @{cell(Some(140.0), type_in)}
                        --
                        @{cell(Some(114.0), cat_in)}
                        --
                        @{cell(None, def_box)}
                        --
                        @{cell(Some(84.0), constraints_btn)}
                        --
                        @{cell(Some(84.0), order_box)}
                    ]
                    --
                    Node { flex_direction: FlexDirection::Column }
                    DtoFieldConstraintsSlot(fid)
                    Children [ {slot_children} ]
                ]
            }));
        }
    }

    let title = format!("Ordered Fields ({})", doc.dto.fields.len());
    subpane_box(&title, rows)
}

/// Renders an inline recursive default-value editor for a `FieldType::Dto`, `FieldType::Enum`, `Option`, or container field in `.dto.ron`.
fn dto_default_subpane(
    doc: &DtoDoc,
    f: &Field,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
    catalog: &AssetPickerCatalog,
) -> Box<dyn Scene> {
    let mut visited = BTreeSet::new();
    visited.insert(doc.dto.id);
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
        FieldType::Option(inner) => {
            let some_path = vec![f.id, path_step_option_some()];
            close_path = some_path.clone();
            let Some(inner_val) = f.default.as_option().flatten() else {
                return Box::new(bsn! { Node {} });
            };
            match inner.as_ref() {
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
                    PropKey::new(DtoProp::FieldDefault(p[0]))
                } else {
                    PropKey::new(DtoProp::NestedFieldDefault(p))
                }
            };
            let toggle_expand =
                |p: Vec<AssetUuid>, sum: String, exp: bool, tip: &str| dto_default_toggle_button(p, sum, exp, tip);
            let set_option_some =
                |p: Vec<AssetUuid>, some: bool| Action::from(DtoAction::SetOptionSome { path: p, some });
            let container_add = |p: Vec<AssetUuid>| Action::from(DtoAction::ContainerAdd { path: p });
            let container_remove =
                |p: Vec<AssetUuid>, index: usize| Action::from(DtoAction::ContainerRemove { path: p, index });
            let container_move = |p: Vec<AssetUuid>, index: usize, delta: isize| {
                Action::from(DtoAction::ContainerMove { path: p, index, delta })
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
                &doc.expanded_dtos,
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
                @{tool_button("Close", Action::from(DtoAction::ToggleDtoDefault(close_path)))}
            ]
            --
            Node { flex_direction: FlexDirection::Column }
            Children [ {form_rows} ]
        ]
    })
}

#[allow(clippy::too_many_arguments)]
fn build_fields_default_rows(
    doc: &DtoDoc,
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
            Children [ @text("Maximum DTO nesting depth reached.", ERROR_COLOR, 11.0) ]
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
        let reset_btn = dto_default_reset_button(
            path.clone(),
            Action::from(DtoAction::ResetNestedDefault(path.clone())),
            is_default,
            &format!("Reset {} to default ({def_str})", cf.name),
        );
        let value_prop = |p: Vec<AssetUuid>| PropKey::new(DtoProp::NestedFieldDefault(p));
        let toggle_expand =
            |p: Vec<AssetUuid>, sum: String, exp: bool, t: &str| dto_default_toggle_button(p, sum, exp, t);
        let set_option_some = |p: Vec<AssetUuid>, some: bool| Action::from(DtoAction::SetOptionSome { path: p, some });
        let container_add = |p: Vec<AssetUuid>| Action::from(DtoAction::ContainerAdd { path: p });
        let container_remove =
            |p: Vec<AssetUuid>, index: usize| Action::from(DtoAction::ContainerRemove { path: p, index });
        let container_move = |p: Vec<AssetUuid>, index: usize, delta: isize| {
            Action::from(DtoAction::ContainerMove { path: p, index, delta })
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
            &doc.expanded_dtos,
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

/// Renders `FieldSettings` as an internal DTO using the shared `{label} | {value} | {reset}` form rows.
fn field_settings_subpane(f: &Field) -> Box<dyn Scene> {
    let mut form_rows: Vec<Box<dyn Scene>> = Vec::new();

    // 1. `doc: String` (all FieldTypes)
    let doc_is_default = f.settings.doc.is_empty();
    form_rows.push(dto_form_row(
        "doc",
        if doc_is_default { TEXT } else { ACCENT },
        false,
        Tooltip("doc: String (default: \"\")\nRust /// doc comment for generated field".to_string()),
        text_hint(DtoProp::FieldDoc(f.id), f.settings.doc.clone(), "Rust /// doc comment for generated field"),
        dto_reset_button(
            Action::from(DtoAction::ResetConstraint { id: f.id, field: ConstraintField::Doc }),
            doc_is_default,
            "\"\"",
        ),
    ));

    // 2. Numeric constraints (`min`, `max`, `step`, `slider`)
    if f.ty.is_numeric() {
        let prec = if is_integer_type(&f.ty) { 0 } else { 2 };
        let opt_ty = if is_integer_type(&f.ty) { "Option<i64>" } else { "Option<f64>" };

        form_rows.push(dto_option_row(
            "min",
            Tooltip(format!("min: {opt_ty} (default: None)\nMinimum allowed value")),
            f.settings.min.is_some(),
            Action::from(DtoAction::SetConstraintOption { id: f.id, field: ConstraintField::Min, some: false }),
            Action::from(DtoAction::SetConstraintOption { id: f.id, field: ConstraintField::Min, some: true }),
            f.settings.min.map(|v| number_input(DtoProp::FieldMin(f.id), 0, v as f32, None, None, prec)),
            Action::from(DtoAction::ResetConstraint { id: f.id, field: ConstraintField::Min }),
        ));

        form_rows.push(dto_option_row(
            "max",
            Tooltip(format!("max: {opt_ty} (default: None)\nMaximum allowed value")),
            f.settings.max.is_some(),
            Action::from(DtoAction::SetConstraintOption { id: f.id, field: ConstraintField::Max, some: false }),
            Action::from(DtoAction::SetConstraintOption { id: f.id, field: ConstraintField::Max, some: true }),
            f.settings.max.map(|v| number_input(DtoProp::FieldMax(f.id), 0, v as f32, None, None, prec)),
            Action::from(DtoAction::ResetConstraint { id: f.id, field: ConstraintField::Max }),
        ));

        form_rows.push(dto_option_row(
            "step",
            Tooltip(format!("step: {opt_ty} (default: None)\nStep increment (> 0)")),
            f.settings.step.is_some(),
            Action::from(DtoAction::SetConstraintOption { id: f.id, field: ConstraintField::Step, some: false }),
            Action::from(DtoAction::SetConstraintOption { id: f.id, field: ConstraintField::Step, some: true }),
            f.settings.step.map(|v| number_input(DtoProp::FieldStep(f.id), 0, v as f32, None, None, prec)),
            Action::from(DtoAction::ResetConstraint { id: f.id, field: ConstraintField::Step }),
        ));

        form_rows.push(dto_form_row(
            "slider",
            if f.settings.slider { ACCENT } else { TEXT },
            false,
            Tooltip("slider: bool (default: false)\nShow horizontal slider (enables min & max if unset)".to_string()),
            checkbox_w(DtoProp::FieldSlider(f.id), f.settings.slider),
            dto_reset_button(
                Action::from(DtoAction::ResetConstraint { id: f.id, field: ConstraintField::Slider }),
                !f.settings.slider,
                "false",
            ),
        ));
    }

    // 3. String / reference constraints (`multiline`, `max_len`, `required`, `unique_in_collection`)
    if f.ty.is_string() {
        form_rows.push(dto_form_row(
            "multiline",
            if f.settings.multiline { ACCENT } else { TEXT },
            false,
            Tooltip("multiline: bool (default: false)\nUse multiline text area".to_string()),
            checkbox_w(DtoProp::FieldMultiline(f.id), f.settings.multiline),
            dto_reset_button(
                Action::from(DtoAction::ResetConstraint { id: f.id, field: ConstraintField::Multiline }),
                !f.settings.multiline,
                "false",
            ),
        ));

        form_rows.push(dto_option_row(
            "max_len",
            Tooltip("max_len: Option<usize> (default: None)\nMaximum character count".to_string()),
            f.settings.max_len.is_some(),
            Action::from(DtoAction::SetConstraintOption { id: f.id, field: ConstraintField::MaxLen, some: false }),
            Action::from(DtoAction::SetConstraintOption { id: f.id, field: ConstraintField::MaxLen, some: true }),
            f.settings
                .max_len
                .map(|ml| number_input(DtoProp::FieldMaxLen(f.id), 0, ml as f32, None, Some((1.0, 65536.0)), 0)),
            Action::from(DtoAction::ResetConstraint { id: f.id, field: ConstraintField::MaxLen }),
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
            checkbox_w(DtoProp::FieldRequired(f.id), f.settings.required),
            dto_reset_button(
                Action::from(DtoAction::ResetConstraint { id: f.id, field: ConstraintField::Required }),
                !f.settings.required,
                "false",
            ),
        ));
    }

    if f.ty.is_string() {
        form_rows.push(dto_form_row(
            "unique_in_collection",
            if f.settings.unique_in_collection { ACCENT } else { TEXT },
            false,
            Tooltip(
                "unique_in_collection: bool (default: false)\nValues must be unique across rows in a collection"
                    .to_string(),
            ),
            checkbox_w(DtoProp::FieldUniqueInCollection(f.id), f.settings.unique_in_collection),
            dto_reset_button(
                Action::from(DtoAction::ResetConstraint { id: f.id, field: ConstraintField::UniqueInCollection }),
                !f.settings.unique_in_collection,
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
                @{tool_button("Close", Action::from(DtoAction::ToggleConstraints(f.id)))}
            ]
            --
            Node { flex_direction: FlexDirection::Column }
            Children [ {form_rows} ]
        ]
    })
}

fn redraw_topbar(
    editor: Res<Editor>,
    bar: Query<Entity, With<DtoTopbar>>,
    mut last: Local<Option<(Option<usize>, String)>>,
    mut commands: Commands,
) {
    let Some(doc) = active_dto(&editor) else {
        *last = None;
        return;
    };
    let key = (editor.active, doc.path.clone());
    if *last == Some(key.clone()) {
        return;
    }
    let Ok(bar) = bar.single() else { return };
    *last = Some(key);
    commands.entity(bar).despawn_children();

    let items: Vec<Box<dyn Scene>> = vec![
        Box::new(icon_tool_button(
            IconId::Plus,
            "Add Field",
            Action::from(DtoAction::AddField),
            "Add a new field to this DTO schema",
        )),
        Box::new(icon_tool_button(
            IconId::Code,
            "Preview Rust",
            Action::from(DataStep::OpenRustPreview(doc.aref.clone())),
            "Open the generated Rust struct preview",
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
    body: Query<Entity, With<DtoSidebar>>,
    mut key: Local<SidebarKey>,
    mut commands: Commands,
) {
    let Some(doc) = active_dto(&editor) else {
        key.forget();
        return;
    };
    let path = doc.path.clone();
    fill_sidebar_with(&mut commands, &body, &mut key, SidebarKey::now(&editor), || {
        let about = format!(
            "# DTO Schema (`{path}.dto.ron`)\n\
            Defines a strongly-typed Rust struct generated into `crates/quillshade/src/data/dtos/{path}.rs`.\n\n\
            - **Name**: checked fields form the compact row label (`short_view_string`) in collections.\n\
            - **Category**: groups fields into collapsible subpanes in the collection inspector.\n\n\
            ! Adding, removing, renaming, or retyping fields changes `DATA_SCHEMA_HASH` and requires rebuilding the game binary."
        );
        vec![
            subpane_box("About", vec![Box::new(rich_text(&about))]),
            subpane_box(
                "Hotkeys",
                vec![
                    hotkey_row("Enter", "Add field (in name input)"),
                    hotkey_row("Delete", "Delete selected field"),
                    hotkey_row("Ctrl+Z / Y", "Undo / Redo"),
                    hotkey_row("Ctrl+S", "Save schema"),
                ],
            ),
        ]
    });
}

fn redraw_body(
    editor: Res<Editor>,
    body: Query<Entity, With<DtoViewBody>>,
    mut last: Local<Option<(Option<usize>, u64)>>,
    mut last_constraints: Local<Option<(Option<usize>, u64)>>,
    mut commands: Commands,
) {
    let Some(doc) = active_dto(&editor) else {
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
    let sections: Vec<Box<dyn Scene>> =
        vec![add_field_subpane(doc, &dtos, &enums), fields_table_subpane(doc, &dtos, &enums, &catalog)];
    for s in sections {
        commands.spawn_scene(bsn! { @{s} ChildOf(body) });
    }
}

fn redraw_constraints_slots(
    editor: Res<Editor>,
    slots: Query<(Entity, &DtoFieldConstraintsSlot)>,
    mut last: Local<Option<(Option<usize>, u64)>>,
    mut commands: Commands,
) {
    let Some(doc) = active_dto(&editor) else {
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
    for (slot_entity, DtoFieldConstraintsSlot(fid)) in &slots {
        commands.entity(slot_entity).despawn_children();
        if let Some(field) = doc.dto.fields.iter().find(|f| f.id == *fid) {
            if field_has_expanded_default_subpane(doc, field) {
                let pane = dto_default_subpane(doc, field, &dtos, &enums, &catalog);
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
    mut rows: Query<(&DtoFieldRow, &mut Node, &mut BackgroundColor)>,
    mut btn_texts: Query<(&DtoConstraintsBtnText, &mut Text, &mut TextColor), Without<DtoDefaultSummaryText>>,
    mut summary_texts: Query<(&DtoDefaultSummaryText, &mut Text, &mut TextColor), Without<DtoConstraintsBtnText>>,
    mut reset_icons: Query<(&DtoDefaultResetIcon, &mut ImageNode)>,
    parents: Query<&ChildOf>,
    mut commands: Commands,
) {
    if !editor.is_changed() {
        return;
    }
    let Some(doc) = active_dto(&editor) else {
        return;
    };
    let dtos = resolve_doc_dtos(&editor, doc);
    let enums = resolve_doc_enums(&editor, doc);

    for (DtoFieldRow(fid), mut node, mut bg) in &mut rows {
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

    for (DtoConstraintsBtnText(fid), mut txt, mut color) in &mut btn_texts {
        let has_some = doc.dto.fields.iter().find(|f| f.id == *fid).is_some_and(|f| !f.settings.is_default());
        let (want_txt, want_color) = if has_some { ("Some", ACCENT) } else { ("None", TEXT_DIM) };
        if txt.0 != want_txt {
            txt.0 = want_txt.to_string();
        }
        if color.0 != want_color {
            color.0 = want_color;
        }
    }

    for (DtoDefaultSummaryText { path }, mut txt, mut color) in &mut summary_texts {
        if let Some((ty, _, def, _)) = doc.dto.field_meta_at_path_with_schemas(path, &dtos, &enums, true)
            && (matches!(ty, FieldType::Dto(_) | FieldType::Enum(_)) || ty.is_container())
        {
            let val = doc.get_default_at_path(path, &def);
            let expanded = doc.expanded_dtos.contains(path);
            let want_txt = if ty.is_container() {
                container_summary_label(&ty, val, &dtos, &enums)
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

    for (DtoDefaultResetIcon { path }, mut img) in &mut reset_icons {
        if let Some((_, _, def, _)) = doc.dto.field_meta_at_path_with_schemas(path, &dtos, &enums, true) {
            let val = doc.get_default_at_path(path, &def);
            let want = reset_icon_color(*val == def);
            if img.color != want {
                img.color = want;
            }
        }
    }

    for (e, field, cur_soft, cur_hard) in &limits_q {
        let path = match field.target.downcast::<DtoProp>() {
            Some(DtoProp::FieldDefault(fid)) => vec![*fid],
            Some(DtoProp::NestedFieldDefault(p)) => p.clone(),
            _ => continue,
        };
        let Some((ty, settings, _, _)) = doc.dto.field_meta_at_path_with_schemas(&path, &dtos, &enums, true) else {
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
        |key| key.downcast::<DtoProp>().is_some(),
    );
}

fn commit_color(ev: On<ValueChange<Color>>, fields: Query<&PropField>, mut actions: ResMut<Actions>) {
    if ev.is_final
        && fields.get(ev.source).is_ok_and(|f| {
            matches!(f.target.downcast(), Some(DtoProp::FieldDefault(_) | DtoProp::NestedFieldDefault(_)))
        })
    {
        actions.push(Action::Tab(TabAction::CommitInline));
    }
}

fn add_on_enter(ev: On<FocusedInput<KeyboardInput>>, fields: Query<&PropField>, mut actions: ResMut<Actions>) {
    if ev.input.key_code == KeyCode::Enter
        && ev.input.state.is_pressed()
        && fields.get(ev.focused_entity).is_ok_and(|f| f.target == PropKey::new(DtoProp::DraftName))
    {
        actions.push(Action::from(DtoAction::AddField));
    }
}
