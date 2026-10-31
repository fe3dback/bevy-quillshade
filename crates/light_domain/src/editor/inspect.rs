//! The `Inspect Game` Light usage validator tool (`Tools -> Inspect Game`, design §9.2): scans `.rs` files in the game
//! workspace for manual Bevy lighting and camera-environment setup (`LW102`) and lets the user jump directly to each
//! conflicting file.

use std::collections::{BTreeMap, BTreeSet};

use bevy::feathers::controls::{ButtonVariant, FeathersButton};
use bevy::prelude::*;
use editor_core::action::{Action, Handled, TabAction};
use editor_core::doc_kind::{DocCtx, StatusCtx, StatusInfo, TabDoc};
use editor_core::domain::ToolId;
use editor_core::editors::{EditorAreas, ViewOf, bar_row, sidebar_body};
use editor_core::state::editor::Editor;
use editor_core::tab::{ViewKindId, format_highlights, parse_highlights};
use editor_core::theme::{TEXT, TEXT_DIM, text};
use editor_core::tooltip::Tooltip;
use editor_core::ui::icons::{IconId, IconRef, icon};
use editor_core::ui::shell::VIEW_BG;
use editor_core::ui::widgets::{scroll_column, subpane_box};
use editor_core::ui::{OnActivate, icon_tool_button};
use editor_core::util::text::plural;
use project_core::{AssetRef, Diagnostic, Project};

use crate::backend::scan_game_conflicts;

/// The `Inspect Game` tool's id in the Light domain.
pub const INSPECT_GAME_TOOL: ToolId = "inspect_game";

/// The `Inspect Game` tool tab's view.
pub const INSPECT_GAME_VIEW: ViewKindId = ViewKindId("light.inspect_game");

const SECTION_W: f32 = 680.0;
const WARN: Color = Color::srgb(0.95, 0.78, 0.35);

/// The `Inspect Game` tool tab's document.
pub struct InspectGameDoc {
    /// Manual Bevy lighting / camera-environment conflicts (`LW102`) found in the game's `.rs` files.
    pub conflicts: Vec<Diagnostic>,
    revision: u64,
}

impl InspectGameDoc {
    pub fn new(conflicts: Vec<Diagnostic>) -> Self {
        Self { conflicts, revision: 1 }
    }

    /// Re-scans the game workspace's `.rs` files and bumps `revision` when the conflict list changes.
    pub fn refresh(&mut self, project: Option<&Project>) {
        let conflicts = project.map(scan_game_conflicts).unwrap_or_default();
        self.set_conflicts(conflicts);
    }

    /// Updates the cached conflict list and bumps `revision` when it changes.
    pub fn set_conflicts(&mut self, conflicts: Vec<Diagnostic>) {
        if conflicts != self.conflicts {
            self.conflicts = conflicts;
            self.revision += 1;
        }
    }
}

/// Opens the `Inspect Game` tool tab with a fresh scan of the open project's `.rs` files.
pub(super) fn open(editor: &mut Editor) -> Box<dyn TabDoc> {
    let conflicts = editor.project.as_ref().map(scan_game_conflicts).unwrap_or_default();
    Box::new(InspectGameDoc::new(conflicts))
}

impl TabDoc for InspectGameDoc {
    fn apply(&mut self, _ctx: &mut DocCtx, _action: &TabAction) -> Handled {
        Handled::No
    }

    fn asset(&self) -> Option<&AssetRef> {
        None
    }

    fn view(&self) -> ViewKindId {
        INSPECT_GAME_VIEW
    }

    fn base_title(&self) -> String {
        "Inspect Game".into()
    }

    fn icon(&self) -> IconRef {
        super::LIGHT_ICON
    }

    fn watermark(&self) -> Option<&'static str> {
        Some("Inspect Game")
    }

    fn is_dirty(&self) -> bool {
        false
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn revision(&self) -> u64 {
        self.revision
    }

    fn save(&mut self, _ctx: &mut DocCtx) -> std::io::Result<()> {
        Ok(())
    }

    fn changed_on_disk(&self, project: &Project) -> bool {
        scan_game_conflicts(project) != self.conflicts
    }

    fn reload_from_disk(&mut self, ctx: &mut DocCtx) -> Result<(), String> {
        self.refresh(ctx.project);
        Ok(())
    }

    fn keep_mine(&mut self, _project: &Project) {}

    fn rename(&mut self, _project: &Project, _from: &AssetRef, _to: &AssetRef) {}

    fn config_key(&self) -> Option<String> {
        None
    }

    fn tool(&self) -> Option<ToolId> {
        Some(INSPECT_GAME_TOOL)
    }

    fn status(&self, ctx: &StatusCtx) -> Option<StatusInfo> {
        Some(StatusInfo::new(false, ctx, vec![plural(self.conflicts.len(), "conflict", "conflicts")]))
    }
}

/// The `Inspect Game` editor kind.
pub struct InspectGameEditor;

/// The editor topbar row of the `Inspect Game` tab.
#[derive(Component, Default, Clone)]
pub struct InspectGameBar;

/// The content column of the `Inspect Game` tab.
#[derive(Component, Default, Clone)]
pub struct InspectGameBody;

impl EditorAreas for InspectGameEditor {
    const VIEW: ViewKindId = INSPECT_GAME_VIEW;
    const SIDEBAR_TITLE: &'static str = "Inspect Game";

    fn topbar() -> Option<impl Scene> {
        Some(bsn! { @bar_row() InspectGameBar })
    }

    fn sidebar() -> Option<impl Scene> {
        let info = [
            "Scans .rs files in the game workspace (excluding crates/quillshade/ and target/) for manual Bevy \
             lighting and camera-environment types (DirectionalLight, GlobalAmbientLight, AmbientLight, \
             EnvironmentMapLight, Skybox, DistanceFog, Exposure, Tonemapping, DebandDither, ColorGrading, \
             ClearColor).",
            "Removing manual Bevy lighting setup lets quillshade::light::Light manage the scene without ECS \
             conflicts (LW102).",
        ]
        .map(|line| -> Box<dyn Scene> { Box::new(text(line, TEXT_DIM, 12.0)) });
        let pane = subpane_box("Light Usage Validator", info.into());
        Some(bsn! { @sidebar_body() Children [ @{pane} ] })
    }

    fn build(app: &mut App) {
        app.add_systems(Update, (redraw_bar, redraw_body).after(editor_core::apply_actions));
    }
}

fn active(editor: &Editor) -> Option<(usize, &InspectGameDoc)> {
    let i = editor.active?;
    Some((i, editor.tabs.get(i)?.get::<InspectGameDoc>()?))
}

/// The `Inspect Game` view in the editor content.
pub fn inspect_view() -> impl Scene {
    bsn! {
        ViewOf(INSPECT_GAME_VIEW)
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
            InspectGameBody
            Node { padding: UiRect::new(px(16), px(16), px(12), px(56)), row_gap: px(12) }
        ]
    }
}

fn redraw_bar(
    editor: Res<Editor>,
    bar: Query<Entity, With<InspectGameBar>>,
    mut last: Local<Option<(u64, usize, u64)>>,
    mut commands: Commands,
) {
    let Some((i, doc)) = active(&editor) else {
        *last = None;
        return;
    };
    let key = (editor.project_generation, i, doc.revision());
    if *last == Some(key) {
        return;
    }
    let Ok(bar) = bar.single() else { return };
    *last = Some(key);
    commands.entity(bar).despawn_children();
    let rescan = icon_tool_button(
        IconId::Regenerate,
        "Rescan",
        Action::Rescan,
        "Rescan game Rust source files for Bevy lighting conflicts",
    );
    let summary: Box<dyn Scene> = if doc.conflicts.is_empty() {
        Box::new(text("No conflicts in game Rust sources", TEXT_DIM, 12.0))
    } else {
        let count = plural(doc.conflicts.len(), "conflict", "conflicts");
        Box::new(text(format!("{count} in game Rust sources"), WARN, 12.0))
    };
    commands.spawn_scene(bsn! { @{rescan} ChildOf(bar) });
    commands.spawn_scene(bsn! { @{summary} ChildOf(bar) });
}

fn redraw_body(
    editor: Res<Editor>,
    body: Query<Entity, With<InspectGameBody>>,
    mut last: Local<Option<(u64, usize, u64)>>,
    mut commands: Commands,
) {
    let Some((i, doc)) = active(&editor) else {
        *last = None;
        return;
    };
    let key = (editor.project_generation, i, doc.revision());
    if *last == Some(key) {
        return;
    }
    let Ok(body) = body.single() else { return };
    *last = Some(key);
    commands.entity(body).despawn_children();
    let pane = if doc.conflicts.is_empty() {
        let rows: Vec<Box<dyn Scene>> = vec![Box::new(text(
            "No manual Bevy lighting or camera-environment conflicts (LW102) were found in the game's .rs files.",
            TEXT_DIM,
            12.0,
        ))];
        subpane_box("Game Rust Sources", rows)
    } else {
        let mut by_file: BTreeMap<&str, (Diagnostic, BTreeSet<usize>)> = BTreeMap::new();
        for diag in &doc.conflicts {
            let asset = diag.asset.as_deref().unwrap_or("");
            let entry = by_file.entry(asset).or_insert_with(|| (diag.clone(), BTreeSet::new()));
            if let Some(where_in) = diag.location.as_deref() {
                entry.1.extend(parse_highlights(where_in));
            }
        }
        let mut rows: Vec<Box<dyn Scene>> = Vec::with_capacity(by_file.len());
        for (rel_path, (first_diag, lines)) in by_file {
            let spec = format_highlights(&lines);
            let inner =
                spec.strip_prefix("highlight=[").and_then(|s| s.strip_suffix(']')).unwrap_or(&spec).replace(',', ", ");
            let lines_text = if inner.is_empty() { "Lines: -".to_string() } else { format!("Lines: {inner}") };
            let diag_with_all = if spec.is_empty() { first_diag } else { first_diag.at(spec) };
            let act = OnActivate(Action::FocusDiagnostic(diag_with_all));
            let tip = Tooltip(format!("Open {rel_path}"));
            let rel_str = rel_path.to_string();
            rows.push(Box::new(bsn! {
                @FeathersButton {
                    @variant: ButtonVariant::Plain,
                    @caption: bsn_list! {
                        @icon(IconId::File, 16.0, TEXT)
                        --
                        Node { flex_direction: FlexDirection::Column, min_width: px(0), row_gap: px(2) }
                        Children [
                            @text(rel_str, TEXT, 14.0)
                            --
                            @text(lines_text, TEXT_DIM, 12.0)
                        ]
                    },
                }
                Node {
                    width: percent(100),
                    height: Val::Auto,
                    justify_content: JustifyContent::Start,
                    column_gap: px(10),
                    padding: UiRect::axes(px(8), px(6)),
                }
                act
                tip
            }));
        }
        subpane_box("Conflicts (LW102)", rows)
    };
    commands.spawn_scene(bsn! {
        Node { width: percent(100), max_width: px(SECTION_W), flex_direction: FlexDirection::Column, row_gap: px(12) }
        Children [ @{pane} ]
        ChildOf(body)
    });
}
