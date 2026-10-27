//! Text editor kind: a read-only generated WESL / Rust file ([`TextDoc`]).

use bevy::prelude::*;
use bevy_enhanced_input::prelude::Actions;
use shader_project::{AssetId, Project};

use super::main_input::{EditCopy, add_kind, ctrl, hotkey};
use super::{EditorAreas, EditorInput, SidebarKey, fill_sidebar_with};
use crate::doc_kind::TabDoc;
use crate::state::Editor;
use crate::tab::{Lang, Tab, TextDoc, ViewKind};
use crate::ui::icons::IconId;
use crate::ui::text_view::TextViewBar;
use crate::ui::widgets::subpane_box;
use crate::ui::{TEXT, TEXT_DIM, plural, text};

/// The text editor's window areas.
pub struct TextEditor;

impl EditorAreas for TextEditor {
    const VIEW: ViewKind = ViewKind::Text;
    const SIDEBAR_TITLE: &'static str = "Generated File";

    /// The bar row, filled by `ui::text_view::redraw`.
    fn topbar() -> Option<impl Scene> {
        Some(bsn! { @super::bar_row() TextViewBar })
    }

    /// The file: its path, language, source asset and size.
    fn sidebar() -> Option<impl Scene> {
        Some(bsn! { @super::sidebar_body() TextSidebarBody })
    }

    fn build(app: &mut App) {
        app.add_systems(Update, redraw_sidebar);
    }

    fn input(app: &mut App) {
        add_kind::<Self>(app);
    }
}

/// The text editor's input context: Ctrl+C copies the whole file.
#[derive(Component, Default, Clone)]
pub struct TextCtx;

impl EditorInput for TextEditor {
    type Ctx = TextCtx;

    fn bindings() -> impl Scene {
        bsn! {
            Actions<TextCtx> [
                @{hotkey::<EditCopy>(ctrl(KeyCode::KeyC))}
            ]
        }
    }
}

/// The text editor's right sidebar body.
#[derive(Component, Default, Clone)]
struct TextSidebarBody;

fn redraw_sidebar(
    editor: Res<Editor>,
    body: Query<Entity, With<TextSidebarBody>>,
    mut key: Local<SidebarKey>,
    mut commands: Commands,
) {
    let Some(Tab::Text(t)) = editor.tab() else { return key.forget() };
    fill_sidebar_with(&mut commands, &body, &mut key, SidebarKey::now(&editor), || text_rows(t));
}

/// A generated file: where it comes from (Source / Open externally are buttons of the editor topbar).
fn text_rows(t: &TextDoc) -> Vec<Box<dyn Scene>> {
    let mut out: Vec<Box<dyn Scene>> = Vec::new();
    let lang = match t.lang {
        Lang::Wesl => "WESL",
        Lang::Rust => "Rust",
    };
    out.push(Box::new(text(t.rel.clone(), TEXT, 15.0)));
    out.push(Box::new(text(format!("Generated {lang}, read-only"), TEXT_DIM, 12.0)));
    out.push(Box::new(text(format!("Source: {}", t.source.source_rel()), TEXT_DIM, 12.0)));
    if let Some(src) = &t.text {
        out.push(Box::new(text(plural(src.lines().count(), "line", "lines"), TEXT_DIM, 12.0)));
    }
    vec![subpane_box("File", out)]
}

impl TabDoc for TextDoc {
    fn asset(&self) -> Option<&AssetId> {
        None
    }

    fn view(&self) -> ViewKind {
        ViewKind::Text
    }

    fn base_title(&self) -> String {
        self.rel.clone()
    }

    fn icon(&self) -> IconId {
        IconId::Code
    }

    fn watermark(&self) -> Option<&'static str> {
        Some(self.lang.label())
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

    fn save(&mut self, _project: &Project) -> std::io::Result<()> {
        Ok(())
    }

    fn changed_on_disk(&self, _project: &Project) -> bool {
        false
    }

    fn reload_from_disk(&mut self, _project: &Project) -> Result<(), String> {
        self.reload();
        match self.text {
            Some(_) => Ok(()),
            None => Err(format!("{} does not exist", self.rel)),
        }
    }

    fn keep_mine(&mut self, _project: &Project) {}

    /// The generated file moves with its source (regeneration fills it).
    fn rename(&mut self, project: &Project, from: &AssetId, to: &AssetId) {
        if self.source != *from {
            return;
        }
        if let Some(moved) = TextDoc::open(project, to.clone(), self.lang) {
            *self = moved;
        }
    }

    /// `AssetId::source_rel` of the source plus `#wesl` / `#rust`.
    fn config_key(&self) -> Option<String> {
        Some(format!("{}#{}", self.source.source_rel(), self.lang.label()))
    }
}
