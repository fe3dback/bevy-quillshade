//! `.collection.ron` master-detail editor (`CollectionEditor` + `collection_view`): topbar with DTO schema picker,
//! left searchable row list (`CollectionListBody`), right categorized field inspector (`CollectionDetailBody`),
//! and right sidebar (`About` + `Hotkeys`).

use std::collections::{BTreeMap, BTreeSet};

use bevy::feathers::controls::{
    ButtonVariant, ColorInputValue, FeathersButton, FeathersCheckbox, FeathersToolButton, NumberInputValue,
};
use bevy::input_focus::InputFocus;
use bevy::prelude::*;
use bevy::ui::Checked;
use bevy::ui_widgets::ValueChange;

use project_core::Diagnostic;
use project_core::asset_uuid::AssetUuid;

use editor_core::action::{Action, Actions, TabAction};
use editor_core::dynval::PropKey;
use editor_core::editors::{EditorAreas, SidebarKey, ViewOf, bar_row, fill_sidebar_with, sidebar_body};
use editor_core::state::editor::Editor;
use editor_core::tab::ViewKindId;
use editor_core::theme::{ACCENT, BORDER, PANEL_BG, TEXT, TEXT_DIM, text};
use editor_core::tooltip::Tooltip;
use editor_core::ui::icons::{IconId, icon};
use editor_core::ui::props::{PropField, select_values, text_hint};
use editor_core::ui::rich_text::rich_text;
use editor_core::ui::widgets::{scroll_column, subpane_box};
use editor_core::ui::{OnActivate, button, icon_button, icon_tool_button, tool_button};

use crate::backend::model::{DtoAsset, EnumAsset, Field, FieldType, Row};

use super::DTO_ICON;
use super::collection_doc::{COLLECTION_VIEW, CollectionAction, CollectionDoc, CollectionProp};
use super::state::{DataState, DataStep};
use super::widgets::{
    AssetPickerCatalog, ContainerEditorCallbacks, build_inline_value_and_subrows, container_summary_label,
    dto_form_row, reset_icon_color, sync_data_value_widgets,
};

const VIEW_BG: Color = Color::srgb_u8(24, 25, 29);
const ROW_LINE: Color = Color::srgb_u8(40, 42, 48);
const SELECTED_ROW_BG: Color = Color::srgba(0.98, 0.57, 0.24, 0.14);
const ERROR_TEXT: Color = Color::srgb(0.95, 0.42, 0.42);
const ERROR_BG: Color = Color::srgba(0.94, 0.27, 0.27, 0.14);
const ERROR_BORDER: Color = Color::srgba(0.94, 0.27, 0.27, 0.45);
const WARN_TEXT: Color = Color::srgb_u8(251, 191, 36);
const WARN_BG: Color = Color::srgba(0.96, 0.62, 0.04, 0.14);
const WARN_BORDER: Color = Color::srgba(0.96, 0.62, 0.04, 0.40);

#[derive(Component, Default, Clone)]
pub struct CollectionTopbar;

#[derive(Component, Default, Clone)]
pub struct CollectionSidebar;

#[derive(Component, Default, Clone)]
pub struct CollectionFilterBar;

#[derive(Component, Default, Clone)]
pub struct CollectionListBody;

#[derive(Component, Default, Clone)]
pub struct CollectionDetailBody;

/// Live-synced text label for a row in the left row list (`#<idx> - <short_view_string>`).
#[derive(Component, Default, Clone, Copy, Debug)]
pub struct CollectionRowLabel {
    pub idx: usize,
    pub row: AssetUuid,
}

/// Per-cell reset icon tinted [`ACCENT`] when the cell value differs from the DTO schema default.
#[derive(Component, Default, Clone, Debug, PartialEq, Eq)]
pub struct CollectionResetIcon {
    pub row: AssetUuid,
    pub path: Vec<AssetUuid>,
}

/// Live-synced `{ShortViewString ▾}` / `{ShortViewString ▸}` summary text on a `FieldType::Dto` row in [`CollectionEditor`].
#[derive(Component, Default, Clone, Debug, PartialEq, Eq)]
pub struct CollectionDtoSummaryText {
    pub row: AssetUuid,
    pub path: Vec<AssetUuid>,
}

pub struct CollectionEditor;

impl EditorAreas for CollectionEditor {
    const VIEW: ViewKindId = COLLECTION_VIEW;
    const SIDEBAR_TITLE: &'static str = "Collection";

    fn topbar() -> Option<impl Scene> {
        Some(bsn! { @bar_row() CollectionTopbar })
    }

    fn sidebar() -> Option<impl Scene> {
        Some(bsn! { @sidebar_body() CollectionSidebar })
    }

    fn build(app: &mut App) {
        app.add_systems(
            Update,
            (redraw_topbar, redraw_sidebar, redraw_filter_bar, redraw_list, redraw_body, sync_fields)
                .chain()
                .after(editor_core::apply_actions),
        )
        .add_observer(commit_color);
    }
}

pub fn collection_view() -> impl Scene {
    bsn! {
        ViewOf(COLLECTION_VIEW)
        Node {
            position_type: PositionType::Absolute,
            left: px(0),
            right: px(0),
            top: px(0),
            bottom: px(0),
            flex_direction: FlexDirection::Row,
        }
        BackgroundColor(VIEW_BG)
        Visibility::Hidden
        Children [
            Node {
                width: px(280),
                flex_shrink: 0.0,
                flex_direction: FlexDirection::Column,
                border: UiRect::right(px(1)),
            }
            BackgroundColor(PANEL_BG)
            BorderColor::all(BORDER)
            Children [
                Node {
                    padding: UiRect::new(px(8), px(8), px(8), px(4)),
                    flex_direction: FlexDirection::Column,
                    row_gap: px(4),
                    border: UiRect::bottom(px(1)),
                }
                BorderColor::all(ROW_LINE)
                CollectionFilterBar
                --
                @scroll_column()
                CollectionListBody
                Node {
                    padding: UiRect::all(px(8)),
                    row_gap: px(4),
                }
            ]
            --
            Node {
                flex_grow: 1.0,
                min_width: px(0),
                flex_direction: FlexDirection::Column,
            }
            Children [
                @scroll_column()
                CollectionDetailBody
                Node {
                    padding: UiRect::new(px(12), px(12), px(10), px(56)),
                    row_gap: px(10),
                }
            ]
        ]
    }
}

fn active_collection(editor: &Editor) -> Option<&CollectionDoc> {
    editor.active_doc::<CollectionDoc>()
}

fn resolve_doc_dto<'a>(editor: &'a Editor, doc: &'a CollectionDoc) -> Option<&'a DtoAsset> {
    if let Some(st) = editor.domains.get::<DataState>()
        && let Some((_, d)) = st.dto_by_id(doc.col.dto)
    {
        return Some(d);
    }
    doc.cached_dto
        .as_ref()
        .filter(|d| d.id == doc.col.dto)
        .or_else(|| doc.cached_dtos.values().find(|d| d.id == doc.col.dto))
}

fn resolve_doc_dtos(editor: &Editor, doc: &CollectionDoc) -> BTreeMap<String, DtoAsset> {
    let mut dtos = doc.cached_dtos.clone();
    if let Some(st) = editor.domains.get::<DataState>() {
        for (p, d) in &st.assets.dtos {
            dtos.insert(p.clone(), d.clone());
        }
    }
    for tab in &editor.tabs {
        if let Some(dto_doc) = tab.get::<super::dto_doc::DtoDoc>() {
            dtos.insert(dto_doc.path.clone(), dto_doc.dto.clone());
        }
    }
    dtos
}

fn resolve_doc_enums(editor: &Editor, doc: &CollectionDoc) -> BTreeMap<String, EnumAsset> {
    let mut enums = doc.cached_enums.clone();
    if let Some(st) = editor.domains.get::<DataState>() {
        for (p, e) in &st.assets.enums {
            enums.insert(p.clone(), e.clone());
        }
    }
    for tab in &editor.tabs {
        if let Some(enum_doc) = tab.get::<super::enum_doc::EnumDoc>() {
            enums.insert(enum_doc.path.clone(), enum_doc.en.clone());
        }
    }
    enums
}

type CollectionTopbarKey = (Option<usize>, u64, usize);

fn redraw_topbar(
    editor: Res<Editor>,
    bar: Query<Entity, With<CollectionTopbar>>,
    mut last: Local<Option<CollectionTopbarKey>>,
    mut commands: Commands,
) {
    let Some(doc) = active_collection(&editor) else {
        *last = None;
        return;
    };
    let dto_opts = editor.domains.get::<DataState>().map(DataState::dto_options).unwrap_or_default();
    let key = (editor.active, doc.layout, dto_opts.len());
    if *last == Some(key) {
        return;
    }
    let Ok(bar) = bar.single() else { return };
    *last = Some(key);
    commands.entity(bar).despawn_children();

    let mut labels = vec!["(None)".to_string()];
    let mut values = vec![String::new()];
    let mut cur_idx = 0usize;
    for (i, (id, path)) in dto_opts.iter().enumerate() {
        labels.push(format!("{path}.dto"));
        values.push(id.to_string());
        if *id == doc.col.dto {
            cur_idx = i + 1;
        }
    }
    if cur_idx == 0 && !doc.col.dto.is_nil() {
        labels.push(format!("(Missing: {})", doc.col.dto));
        values.push(doc.col.dto.to_string());
        cur_idx = labels.len() - 1;
    }

    let schema_select = select_values(CollectionProp::DtoSchema, labels, values, cur_idx);
    let items: Vec<Box<dyn Scene>> = vec![
        Box::new(text("Schema:", TEXT_DIM, 11.0)),
        Box::new(bsn! {
            Node { width: px(190), flex_shrink: 0.0 }
            Children [ @{schema_select} ]
        }),
        Box::new(icon_tool_button(
            DTO_ICON,
            "Open Schema",
            Action::from(DataStep::OpenDtoSchema(doc.col.dto)),
            "Open the bound DTO schema tab",
        )),
        Box::new(icon_tool_button(
            IconId::Plus,
            "Add Row",
            Action::from(CollectionAction::AddRow),
            "Add a new row populated with DTO defaults",
        )),
        Box::new(icon_tool_button(
            IconId::Copy,
            "Duplicate",
            Action::from(CollectionAction::DuplicateRow(None)),
            "Duplicate the selected row",
        )),
        Box::new(icon_tool_button(
            IconId::Delete,
            "Delete",
            Action::from(CollectionAction::DeleteRow(None)),
            "Delete the selected row",
        )),
        Box::new(icon_tool_button(
            IconId::Code,
            "Preview Rust",
            Action::from(DataStep::OpenRustPreview(doc.aref.clone())),
            "Open the generated Rust collection module",
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
    body: Query<Entity, With<CollectionSidebar>>,
    mut key: Local<SidebarKey>,
    mut commands: Commands,
) {
    let Some(doc) = active_collection(&editor) else {
        key.forget();
        return;
    };
    let path = doc.path.clone();
    fill_sidebar_with(&mut commands, &body, &mut key, SidebarKey::now(&editor), || {
        let about = format!(
            "# Collection (`{path}.collection.ron`)\n\
            Typed row table bound to a `.dto.ron` schema, emitted to `crates/quillshade/src/data/collections/{path}.rs`, and baked to `assets/quillshade/data/autogen/{path}.collection.qsd`.\n\n\
            - **Hot-reload**: editing row values updates `{path}.collection.qsd` in-game without recompiling Rust."
        );
        vec![
            subpane_box("About", vec![Box::new(rich_text(&about))]),
            subpane_box(
                "Hotkeys",
                vec![
                    hotkey_row("Delete", "Delete selected row"),
                    hotkey_row("Ctrl+Z / Y", "Undo / Redo"),
                    hotkey_row("Ctrl+S", "Save & bake (.qsd)"),
                ],
            ),
        ]
    });
}

fn diag_targets_row_id(d: &Diagnostic, row_id: AssetUuid) -> bool {
    let Some(loc) = d.location.as_deref() else { return false };
    let Some(rest) = loc.strip_prefix("row:") else { return false };
    let hex = rest.split(':').next().unwrap_or(rest);
    AssetUuid::parse(hex) == Some(row_id)
}

fn row_matches_diagnostic(idx: usize, row_id: AssetUuid, d: &Diagnostic) -> bool {
    if !diag_targets_row_id(d, row_id) {
        return false;
    }
    if d.message.starts_with("row #") { d.message.starts_with(&format!("row #{idx} ")) } else { true }
}

fn collection_diagnostics(doc: &CollectionDoc, state: Option<&DataState>) -> Vec<Diagnostic> {
    let Some(st) = state else { return Vec::new() };
    let rel = doc.aref.source_rel();
    st.diagnostics.iter().filter(|d| d.asset.as_deref() == Some(rel.as_str())).cloned().collect()
}

fn build_row_list(
    doc: &CollectionDoc,
    dto: Option<&DtoAsset>,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
    diags: &[Diagnostic],
) -> Vec<Box<dyn Scene>> {
    let mut items: Vec<Box<dyn Scene>> = Vec::new();
    let needle = doc.filter.trim().to_ascii_lowercase();
    let selected_id = doc.selected_row.or_else(|| doc.col.rows.first().map(|r| r.id));

    if doc.col.rows.is_empty() {
        items.push(Box::new(bsn! {
            Node { padding: UiRect::vertical(px(8)) }
            Children [ @text("No rows yet. Click + Add Row.", TEXT_DIM, 12.0) ]
        }));
        return items;
    }

    for (idx, row) in doc.col.rows.iter().enumerate() {
        let summary = match dto {
            Some(d) => format!("#{idx} - {}", d.short_view_string_indexed_with_schemas(row, Some(idx), dtos, enums)),
            None => format!("#{idx} - {}", row.id),
        };
        if !needle.is_empty()
            && !summary.to_ascii_lowercase().contains(&needle)
            && !row.id.to_string().to_ascii_lowercase().contains(&needle)
        {
            continue;
        }
        let row_diag = diags.iter().filter(|d| row_matches_diagnostic(idx, row.id, d)).max_by_key(|d| d.is_error());
        let has_err = row_diag.is_some_and(Diagnostic::is_error);
        let is_selected = selected_id == Some(row.id);
        let bg = if is_selected {
            SELECTED_ROW_BG
        } else if has_err {
            ERROR_BG
        } else {
            Color::NONE
        };
        let left_border = if is_selected || row_diag.is_some() { 2.0 } else { 0.0 };
        let left_color = if has_err {
            ERROR_TEXT
        } else if is_selected {
            ACCENT
        } else if row_diag.is_some() {
            WARN_TEXT
        } else {
            Color::NONE
        };
        let label_color = if is_selected { ACCENT } else { TEXT };
        let label_tag = CollectionRowLabel { idx, row: row.id };

        let mut btn_children: Vec<Box<dyn Scene>> = vec![Box::new(bsn! {
            Node { flex_grow: 1.0, min_width: px(0), overflow: Overflow::clip() }
            Children [
                @text(summary, label_color, 12.0)
                TextLayout::linebreak(LineBreak::NoWrap)
                label_tag
            ]
        })];

        if let Some(d) = row_diag {
            let (badge_fg, badge_bg, badge_border) =
                if d.is_error() { (ERROR_TEXT, ERROR_BG, ERROR_BORDER) } else { (WARN_TEXT, WARN_BG, WARN_BORDER) };
            btn_children.push(Box::new(bsn! {
                Node {
                    flex_shrink: 0.0,
                    padding: UiRect::axes(px(5), px(1)),
                    border: UiRect::all(px(1)),
                    border_radius: BorderRadius::all(px(3)),
                }
                BackgroundColor(badge_bg)
                BorderColor::all(badge_border)
                Children [ @text(d.code.clone(), badge_fg, 10.5) ]
            }));
        }

        let select_act = OnActivate(Action::from(CollectionAction::SelectRow(Some(row.id))));
        items.push(Box::new(bsn! {
            Node {
                width: percent(100),
                border: UiRect { left: px(left_border), bottom: px(1), ..UiRect::default() },
            }
            BackgroundColor(bg)
            BorderColor {
                left: left_color,
                bottom: ROW_LINE,
                ..BorderColor::all(Color::NONE)
            }
            Children [
                @FeathersButton { @variant: ButtonVariant::Plain }
                Node {
                    width: percent(100),
                    min_height: px(28),
                    padding: UiRect::axes(px(8), px(4)),
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::SpaceBetween,
                    column_gap: px(6),
                }
                select_act
                Children [ {btn_children} ]
            ]
        }));
    }
    items
}

fn reset_cell_button(row_id: AssetUuid, path: Vec<AssetUuid>, def_str: String, is_default: bool) -> Box<dyn Scene> {
    let color = reset_icon_color(is_default);
    let tag = CollectionResetIcon { row: row_id, path: path.clone() };
    let act = OnActivate(CollectionAction::ResetCell { row: row_id, path }.into());
    let tip = if is_default {
        Tooltip(format!("At schema default ({def_str})"))
    } else {
        Tooltip(format!("Reset to default ({def_str})"))
    };
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

fn dto_field_toggle_button(
    row_id: AssetUuid,
    path: Vec<AssetUuid>,
    summary: String,
    expanded: bool,
    tooltip: &str,
) -> Box<dyn Scene> {
    let color = if expanded { ACCENT } else { TEXT };
    let caret = if expanded { IconId::CaretDown } else { IconId::CaretRight };
    let act = OnActivate(CollectionAction::ToggleDtoField { path: path.clone() }.into());
    let tip = Tooltip(tooltip.to_string());
    let tag = CollectionDtoSummaryText { row: row_id, path };
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
            tag
            --
            @icon(caret, 12.0, color)
        ]
    })
}

fn field_label_tooltip(f: &Field, dtos: &BTreeMap<String, DtoAsset>, enums: &BTreeMap<String, EnumAsset>) -> Tooltip {
    let mut flags = Vec::new();
    if f.settings.required {
        flags.push("required".to_string());
    }
    if f.settings.unique_in_collection {
        flags.push("unique".to_string());
    }
    if f.is_name {
        flags.push("name".to_string());
    }
    if let (Some(lo), Some(hi)) = (f.settings.min, f.settings.max) {
        flags.push(format!("{lo}..{hi}"));
    }
    let def_val = f.default_with_schemas(dtos, enums);
    let def_str = def_val.to_short_view_string_with_schemas(&f.ty, dtos, enums);
    flags.push(format!("default: {def_str}"));
    let ty_label = f.ty.describe(dtos, enums);
    let mut tip = format!("{} ({ty_label} | {})", f.name, flags.join(", "));
    if !f.settings.doc.trim().is_empty() {
        tip.push('\n');
        tip.push_str(f.settings.doc.trim());
    }
    Tooltip(tip)
}

#[allow(clippy::too_many_arguments)]
fn append_dto_field_row(
    out: &mut Vec<Box<dyn Scene>>,
    doc: &CollectionDoc,
    row: &Row,
    f: &Field,
    path: Vec<AssetUuid>,
    depth: usize,
    visited: &mut BTreeSet<AssetUuid>,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
    catalog: &AssetPickerCatalog,
) {
    let expected_def = f.default_with_schemas(dtos, enums);
    let val = doc.get_value_at_path_with_schemas(row.id, &path, dtos, enums).unwrap_or_else(|| expected_def.clone());
    let is_default = val == expected_def;
    let def_str = expected_def.to_short_view_string_with_schemas(&f.ty, dtos, enums);
    let reset_btn = reset_cell_button(row.id, path.clone(), def_str, is_default);
    let tip = field_label_tooltip(f, dtos, enums);
    let name_color = if f.is_name { ACCENT } else { TEXT };

    let row_id = row.id;
    let value_prop = |p: Vec<AssetUuid>| PropKey::new(CollectionProp::RowValue { row: row_id, path: p });
    let toggle_expand =
        |p: Vec<AssetUuid>, sum: String, exp: bool, t: &str| dto_field_toggle_button(row_id, p, sum, exp, t);
    let set_option_some =
        |p: Vec<AssetUuid>, some: bool| Action::from(CollectionAction::SetOptionSome { row: row_id, path: p, some });
    let container_add = |p: Vec<AssetUuid>| Action::from(CollectionAction::ContainerAdd { row: row_id, path: p });
    let container_remove = |p: Vec<AssetUuid>, index: usize| {
        Action::from(CollectionAction::ContainerRemove { row: row_id, path: p, index })
    };
    let container_move = |p: Vec<AssetUuid>, index: usize, delta: isize| {
        Action::from(CollectionAction::ContainerMove { row: row_id, path: p, index, delta })
    };
    let render_nested =
        |sub_fields: &[Field], sub_prefix: Vec<AssetUuid>, sub_depth: usize, vis: &mut BTreeSet<AssetUuid>| {
            let mut child_rows: Vec<Box<dyn Scene>> = Vec::with_capacity(sub_fields.len());
            for cf in sub_fields {
                let mut cpath = sub_prefix.clone();
                cpath.push(cf.id);
                append_dto_field_row(&mut child_rows, doc, row, cf, cpath, sub_depth, vis, dtos, enums, catalog);
            }
            child_rows
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
        &f.ty,
        &val,
        &f.settings,
        &path,
        depth,
        visited,
        &doc.expanded_dtos,
        dtos,
        enums,
        catalog,
        &cb,
    );
    out.push(dto_form_row(f.name.clone(), name_color, f.settings.required, tip, widget, reset_btn));
    out.extend(sub_rows);
}

fn build_row_detail(
    doc: &CollectionDoc,
    dto: Option<&DtoAsset>,
    dtos: &BTreeMap<String, DtoAsset>,
    enums: &BTreeMap<String, EnumAsset>,
    row_opt: Option<(usize, &Row)>,
    diags: &[Diagnostic],
    catalog: &AssetPickerCatalog,
) -> Vec<Box<dyn Scene>> {
    let Some(dto) = dto else {
        return vec![subpane_box(
            "No DTO Schema Bound",
            vec![Box::new(text(
                "Select a DTO schema in the topbar dropdown to inspect and edit rows.",
                TEXT_DIM,
                12.0,
            ))],
        )];
    };
    let Some((idx, row)) = row_opt else {
        return vec![subpane_box(
            "No Row Selected",
            vec![Box::new(text("Click + Add Row in the topbar to create the first row.", TEXT_DIM, 12.0))],
        )];
    };

    let mut out: Vec<Box<dyn Scene>> = Vec::new();

    for d in diags.iter().filter(|d| diag_targets_row_id(d, row.id)) {
        let (fg, bg, border) =
            if d.is_error() { (ERROR_TEXT, ERROR_BG, ERROR_BORDER) } else { (WARN_TEXT, WARN_BG, WARN_BORDER) };
        let mut banner_items: Vec<Box<dyn Scene>> = vec![Box::new(bsn! {
            Node { flex_grow: 1.0, min_width: px(0) }
            Children [ @text(format!("[{}] {}", d.code, d.message), fg, 12.0) ]
        })];
        if d.code == "DE002" {
            banner_items.push(Box::new(button(
                "Regenerate Row UUID",
                Action::from(CollectionAction::RegenerateRowId(row.id)),
                true,
            )));
        }
        out.push(Box::new(bsn! {
            Node {
                flex_direction: FlexDirection::Row,
                flex_wrap: FlexWrap::Wrap,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::SpaceBetween,
                column_gap: px(10),
                row_gap: px(6),
                padding: UiRect::axes(px(12), px(8)),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(5)),
            }
            BackgroundColor(bg)
            BorderColor::all(border)
            Children [ {banner_items} ]
        }));
    }

    let header_title =
        format!("Row #{idx} - {}", dto.short_view_string_indexed_with_schemas(row, Some(idx), dtos, enums));
    let copy_btn = icon_tool_button(
        IconId::Copy,
        "Copy",
        Action::from(CollectionAction::CopyRowId(row.id)),
        "Copy UUID to clipboard",
    );
    let fill_tip = Tooltip("Populate any missing schema fields in all rows with their DTO default values".to_string());
    let fill_btn = Box::new(bsn! {
        Node { flex_shrink: 0.0 }
        fill_tip
        Children [ @{tool_button("Fill Defaults", Action::from(CollectionAction::FillDefaults))} ]
    });
    let prune_tip = Tooltip(
        "Remove cell values from all rows whose field UUID no longer exists in the bound DTO schema".to_string(),
    );
    let prune_btn = Box::new(bsn! {
        Node { flex_shrink: 0.0 }
        prune_tip
        Children [ @{tool_button("Prune Unknown Fields", Action::from(CollectionAction::PruneUnknownFields))} ]
    });
    out.push(subpane_box(
        &header_title,
        vec![Box::new(bsn! {
            Node {
                flex_direction: FlexDirection::Row,
                flex_wrap: FlexWrap::Wrap,
                align_items: AlignItems::Center,
                column_gap: px(8),
                row_gap: px(6),
                min_height: px(28),
            }
            Children [
                Node { width: px(148), flex_shrink: 0.0 }
                Children [ @text("UUID", TEXT_DIM, 12.0) ]
                --
                Node {
                    flex_grow: 1.0,
                    min_width: px(220),
                    padding: UiRect::axes(px(8), px(4)),
                    border: UiRect::all(px(1)),
                    border_radius: BorderRadius::all(px(4)),
                }
                BackgroundColor(PANEL_BG)
                BorderColor::all(ROW_LINE)
                Children [ @text(row.id.to_string(), TEXT_DIM, 12.0) ]
                --
                @{copy_btn}
                --
                @{fill_btn}
                --
                @{prune_btn}
            ]
        })],
    ));

    for (cat_name, fields) in dto.categories() {
        let mut cat_rows: Vec<Box<dyn Scene>> = Vec::with_capacity(fields.len());
        let mut visited = BTreeSet::new();
        visited.insert(dto.id);
        for (_, f) in fields {
            append_dto_field_row(&mut cat_rows, doc, row, f, vec![f.id], 0, &mut visited, dtos, enums, catalog);
        }
        out.push(subpane_box(cat_name, cat_rows));
    }

    out
}

fn redraw_filter_bar(
    editor: Res<Editor>,
    filter_bar: Query<Entity, With<CollectionFilterBar>>,
    mut last: Local<Option<(Option<usize>, usize, bool)>>,
    mut commands: Commands,
) {
    let Some(doc) = active_collection(&editor) else {
        *last = None;
        return;
    };
    let key = (editor.active, doc.col.rows.len(), !doc.filter.is_empty());
    if *last == Some(key) {
        return;
    }
    let Ok(bar_e) = filter_bar.single() else { return };
    *last = Some(key);
    commands.entity(bar_e).despawn_children();

    let mut filter_items: Vec<Box<dyn Scene>> = vec![Box::new(bsn! {
        Node { flex_grow: 1.0, min_width: px(0) }
        Children [ @{text_hint(CollectionProp::Filter, doc.filter.clone(), "Filter by text or UUID...")} ]
    })];
    if !doc.filter.is_empty() {
        filter_items.push(Box::new(icon_button(
            IconId::Close,
            Action::from(CollectionAction::ClearFilter),
            "Clear filter",
        )));
    }

    let items: Vec<Box<dyn Scene>> = vec![
        Box::new(text(format!("Rows ({})", doc.col.rows.len()), TEXT_DIM, 11.0)),
        Box::new(bsn! {
            Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: px(4),
                width: percent(100),
            }
            Children [ {filter_items} ]
        }),
    ];
    for item in items {
        commands.spawn_scene(bsn! { @{item} ChildOf(bar_e) });
    }
}

fn redraw_list(
    editor: Res<Editor>,
    list_body: Query<Entity, With<CollectionListBody>>,
    mut last: Local<Option<(Option<usize>, u64)>>,
    mut commands: Commands,
) {
    let Some(doc) = active_collection(&editor) else {
        *last = None;
        return;
    };
    let key = (editor.active, doc.list_layout);
    if *last == Some(key) {
        return;
    }
    let Ok(list_e) = list_body.single() else { return };
    *last = Some(key);
    commands.entity(list_e).despawn_children();

    let dto = resolve_doc_dto(&editor, doc);
    let dtos = resolve_doc_dtos(&editor, doc);
    let enums = resolve_doc_enums(&editor, doc);
    let diags = collection_diagnostics(doc, editor.domains.get::<DataState>());
    for item in build_row_list(doc, dto, &dtos, &enums, &diags) {
        commands.spawn_scene(bsn! { @{item} ChildOf(list_e) });
    }
}

fn redraw_body(
    editor: Res<Editor>,
    detail_body: Query<Entity, With<CollectionDetailBody>>,
    mut last: Local<Option<(Option<usize>, u64)>>,
    mut commands: Commands,
) {
    let Some(doc) = active_collection(&editor) else {
        *last = None;
        return;
    };
    let key = (editor.active, doc.layout);
    if *last == Some(key) {
        return;
    };
    let Ok(detail_e) = detail_body.single() else {
        return;
    };
    *last = Some(key);
    commands.entity(detail_e).despawn_children();

    let dto = resolve_doc_dto(&editor, doc);
    let dtos = resolve_doc_dtos(&editor, doc);
    let enums = resolve_doc_enums(&editor, doc);
    let diags = collection_diagnostics(doc, editor.domains.get::<DataState>());
    let catalog = AssetPickerCatalog::from_editor(&editor);
    let selected_row = doc
        .selected_row
        .and_then(|id| doc.col.rows.iter().enumerate().rfind(|(_, r)| r.id == id))
        .or_else(|| doc.col.rows.first().map(|r| (0, r)));
    for section in build_row_detail(doc, dto, &dtos, &enums, selected_row, &diags, &catalog) {
        commands.spawn_scene(bsn! { @{section} ChildOf(detail_e) });
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn sync_fields(
    editor: Res<Editor>,
    focus: Option<Res<InputFocus>>,
    numbers: Query<(Entity, &PropField, &NumberInputValue)>,
    pickers: Query<(Entity, &PropField, &ColorInputValue)>,
    checkboxes: Query<(Entity, &PropField, Has<Checked>), With<FeathersCheckbox>>,
    mut resets: Query<(&CollectionResetIcon, &mut ImageNode)>,
    mut row_labels: Query<(&CollectionRowLabel, &mut Text), Without<CollectionDtoSummaryText>>,
    mut dto_summaries: Query<(&CollectionDtoSummaryText, &mut Text, &mut TextColor), Without<CollectionRowLabel>>,
    parents: Query<&ChildOf>,
    mut commands: Commands,
) {
    if !editor.is_changed() {
        return;
    }
    let Some(doc) = active_collection(&editor) else {
        return;
    };
    sync_data_value_widgets(
        &editor,
        focus.as_deref(),
        &numbers,
        &pickers,
        &checkboxes,
        &parents,
        &mut commands,
        |key| key.downcast::<CollectionProp>().is_some(),
    );
    let dtos = resolve_doc_dtos(&editor, doc);
    let enums = resolve_doc_enums(&editor, doc);
    if let Some(dto) = resolve_doc_dto(&editor, doc) {
        for (reset, mut img) in &mut resets {
            if let Some((_, _, default_val, _)) = dto.field_meta_at_path_with_schemas(&reset.path, &dtos, &enums, false)
                && let Some(val) = doc.get_value_at_path_with_schemas(reset.row, &reset.path, &dtos, &enums)
            {
                let want = reset_icon_color(val == default_val);
                if img.color != want {
                    img.color = want;
                }
            }
        }
        for (summary_tag, mut txt, mut color) in &mut dto_summaries {
            if let Some((ty, _, _, _)) = dto.field_meta_at_path_with_schemas(&summary_tag.path, &dtos, &enums, false)
                && let Some(val) = doc.get_value_at_path_with_schemas(summary_tag.row, &summary_tag.path, &dtos, &enums)
            {
                let expanded = doc.expanded_dtos.contains(&summary_tag.path);
                let want_txt = if matches!(ty, FieldType::Array(_) | FieldType::Set(_) | FieldType::Map(_, _)) {
                    container_summary_label(&ty, &val, &dtos, &enums)
                } else if let FieldType::Option(ref inner_ty) = ty {
                    match val.as_option().flatten() {
                        Some(inner_val) => {
                            inner_val.to_short_view_string_with_schemas(inner_ty.as_ref(), &dtos, &enums)
                        }
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
        for (lbl, mut txt) in &mut row_labels {
            if let Some(r) = doc.col.rows.get(lbl.idx).filter(|r| r.id == lbl.row) {
                let want = format!(
                    "#{} - {}",
                    lbl.idx,
                    dto.short_view_string_indexed_with_schemas(r, Some(lbl.idx), &dtos, &enums)
                );
                if txt.0 != want {
                    txt.0 = want;
                }
            }
        }
    }
}

fn commit_color(ev: On<ValueChange<Color>>, fields: Query<&PropField>, mut actions: ResMut<Actions>) {
    if ev.is_final
        && fields.get(ev.source).is_ok_and(|f| matches!(f.target.downcast(), Some(CollectionProp::RowValue { .. })))
    {
        actions.push(Action::Tab(TabAction::CommitInline));
    }
}
