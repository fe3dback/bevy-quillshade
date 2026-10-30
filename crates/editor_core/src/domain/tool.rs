//! Tools (design §14.1): tabs that are not assets. A tool is a singleton per `(domain, tool id)`: opening it again
//! activates its open tab and hands it the argument. Tool tabs take part in Back / Forward under the key
//! `"@tool/<id>"` and are never written to the user config.

use crate::doc_kind::TabDoc;
use crate::menu::MainMenu;
use crate::state::editor::Editor;

/// A tool's id, unique within its domain.
pub type ToolId = &'static str;

/// The [`crate::tab::TabKey`] key of a tool tab is this prefix and the tool id; no asset key starts with `@tool/`.
pub const TOOL_KEY_PREFIX: &str = "@tool/";

/// A tool a domain registers ([`super::DomainRegistrar::tool`]).
#[derive(Clone, Copy, Debug)]
pub struct ToolDef {
    pub id: ToolId,
    pub title: &'static str,
    /// Its document, made when the tool opens (with its domain's console/logs tag).
    pub open: fn(&mut Editor) -> Box<dyn TabDoc>,
    /// The main menu with an item `"{title}..."` that opens it (`None`: no item).
    pub menu: Option<MainMenu>,
}
