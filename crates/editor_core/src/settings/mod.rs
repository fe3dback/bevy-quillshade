//! Project Settings (design §14.2): the project domain's tool editing `quillshade/project.ron`. A General section
//! (project path, format version, `header_date`) and one section per real domain with its "Used in game" checkbox;
//! saving writes the file and rewrites the umbrella crate files. Typed per-domain settings come when a domain needs
//! one.

mod doc;
mod view;

pub use doc::{SettingsDoc, SettingsProp};
pub use view::{SETTINGS_VIEW, SettingsEditor};

use crate::doc_kind::TabDoc;
use crate::domain::{DomainRegistrar, ToolDef, ToolId};
use crate::menu::MainMenu;
use crate::state::editor::Editor;

/// The Project Settings tool's id in the project domain.
pub const SETTINGS_TOOL: ToolId = "settings";

/// Registers the tool (File → Project Settings...), its editor kind and its editor content view.
pub(crate) fn register(r: &mut DomainRegistrar) {
    r.tool(ToolDef { id: SETTINGS_TOOL, title: "Project Settings", open, menu: Some(MainMenu::File) })
        .editor_kind::<SettingsEditor>()
        .doc_view(|| Box::new(view::settings_view()));
}

/// The tab of the open project's settings (the last good ones when `project.ron` is broken).
fn open(editor: &mut Editor) -> Box<dyn TabDoc> {
    let settings = editor.project.as_ref().map(|p| p.settings.clone()).unwrap_or_default();
    Box::new(SettingsDoc::new(settings))
}
