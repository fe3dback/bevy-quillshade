//! The Project Settings editor kind (design §14.2): its editor content lists the General section and one section per
//! real domain; the editor topbar names the file, the right sidebar says what saving does.

use bevy::prelude::*;

use super::{SettingsDoc, SettingsProp};
use crate::editors::{EditorAreas, ViewOf, bar_row, sidebar_body};
use crate::state::editor::Editor;
use crate::tab::ViewKindId;
use crate::theme::{TEXT, TEXT_DIM, text};
use crate::tooltip::Tooltip;
use crate::ui::props::{checkbox_row, hotkey_row, labeled};
use crate::ui::shell::{VIEW_BG, clipped_text};
use crate::ui::widgets::{scroll_column, subpane_box};
use project_core::Project;
use project_core::layout::SETTINGS_FILE;

/// The Project Settings tab's view.
pub const SETTINGS_VIEW: ViewKindId = ViewKindId("project.settings");

/// Widest a section grows in the editor content (logical px).
const SECTION_W: f32 = 560.0;

/// The Project Settings editor kind's window areas.
pub struct SettingsEditor;

impl EditorAreas for SettingsEditor {
    const VIEW: ViewKindId = SETTINGS_VIEW;
    const SIDEBAR_TITLE: &'static str = "Project Settings";

    /// The file the tab edits.
    fn topbar() -> Option<impl Scene> {
        let path = format!("{}/{SETTINGS_FILE}", project_core::layout::QUILLSHADE_DIR);
        Some(bsn! { @bar_row() Children [ @text(path, TEXT, 13.0) ] })
    }

    /// What saving writes, the hotkeys.
    fn sidebar() -> Option<impl Scene> {
        let saving = [
            "Saving writes quillshade/project.ron and rewrites the umbrella crate files crates/quillshade/Cargo.toml \
             and src/lib.rs.",
            "A domain not used in game removes its plugin from QuillshadePlugin, deletes \
             crates/quillshade/src/<domain>/, and disables the domain in the editor while keeping \
             quillshade/<domain>/.",
            "Header date reaches every other generated file on its domain's next generation.",
        ]
        .map(|line| -> Box<dyn Scene> { Box::new(text(line, TEXT_DIM, 12.0)) });
        let hotkeys = [("Ctrl+Z / Y", "Undo / redo"), ("Ctrl+S", "Save")]
            .map(|(keys, what)| -> Box<dyn Scene> { Box::new(hotkey_row(keys, what)) });
        let saving = subpane_box("Saving", saving.into());
        let hotkeys = subpane_box("Hotkeys", hotkeys.into());
        Some(bsn! { @sidebar_body() Children [ @{saving} -- @{hotkeys} ] })
    }

    fn build(app: &mut App) {
        app.add_systems(Update, redraw.after(crate::apply_actions));
    }
}

/// The column the sections are rebuilt into.
#[derive(Component, Default, Clone)]
struct SettingsBody;

/// The editor content view of the Project Settings tab, in `ui::DocArea`.
pub fn settings_view() -> impl Scene {
    bsn! {
        ViewOf(SETTINGS_VIEW)
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
            // The bottom margin keeps the sections clear of the file type watermark.
            @scroll_column()
            SettingsBody
            Node { padding: UiRect::new(px(16), px(16), px(12), px(56)), row_gap: px(12) }
        ]
    }
}

/// Rebuilds the sections when the settings tab becomes active or its settings change (a checkbox toggles itself
/// first, so the rebuild after its edit shows what it already shows).
fn redraw(
    editor: Res<Editor>,
    body: Query<Entity, With<SettingsBody>>,
    mut last: Local<Option<(u64, usize, u64)>>,
    mut commands: Commands,
) {
    let active = editor.active.and_then(|i| Some((i, editor.tabs.get(i)?.get::<SettingsDoc>()?)));
    let (Some((i, doc)), Some(project)) = (active, &editor.project) else {
        *last = None;
        return;
    };
    let key = (editor.project_generation, i, crate::doc_kind::TabDoc::revision(doc));
    let Ok(body) = body.single() else { return };
    if *last == Some(key) {
        return;
    }
    *last = Some(key);
    commands.entity(body).despawn_children();
    for s in sections(&editor, project, doc) {
        commands.spawn_scene(bsn! { @{s} ChildOf(body) });
    }
}

/// General, then one section per real domain in registration order.
fn sections(editor: &Editor, project: &Project, doc: &SettingsDoc) -> Vec<Box<dyn Scene>> {
    let root = project.root.display().to_string();
    let tip = Tooltip(root.clone());
    // A `labeled` row whose value may shrink below the text: a long path is clipped at the section's edge, the
    // tooltip has it all.
    let path: Box<dyn Scene> = Box::new(bsn! {
        Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: px(6),
            min_height: px(26),
        }
        Children [
            Node { width: px(110), flex_shrink: 0.0 } Children [ @text("Project", TEXT, 13.0) ]
            --
            Node { flex_grow: 1.0, min_width: px(0) } tip Children [ @clipped_text(root, TEXT, 13.0) ]
        ]
    });
    let version: Box<dyn Scene> = Box::new(text(doc.settings.format_version.to_string(), TEXT_DIM, 13.0));
    let general = vec![
        path,
        Box::new(labeled("Format version".into(), version)),
        checkbox_row("Header date", SettingsProp::HeaderDate, doc.settings.header_date),
        hint("Adds the generation date to the header of every generated file (the files then change every day)."),
    ];
    let mut out = vec![section("General", general)];
    for id in editor.domain_list.real_ids() {
        let Some(d) = editor.domain_list.get(id) else { continue };
        let rows = vec![
            checkbox_row("Used in game", SettingsProp::Enabled(d.id), doc.enabled(d.id)),
            hint(
                "Off: removes the domain's plugin from QuillshadePlugin, deletes crates/quillshade/src/<domain>/, and \
                 disables the domain in the editor (quillshade/<domain>/ is kept).",
            ),
        ];
        out.push(section(d.info.title, rows));
    }
    out
}

/// A section: a Feathers subpane as wide as the editor content allows, up to [`SECTION_W`].
fn section(title: &str, rows: Vec<Box<dyn Scene>>) -> Box<dyn Scene> {
    let pane = subpane_box(title, rows);
    Box::new(bsn! {
        Node { width: percent(100), max_width: px(SECTION_W), flex_direction: FlexDirection::Column }
        Children [ @{pane} ]
    })
}

fn hint(line: &str) -> Box<dyn Scene> {
    Box::new(text(line.to_string(), TEXT_DIM, 12.0))
}
