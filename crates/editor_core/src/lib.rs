//! Domain-neutral editor core (design §5, §6): the editor state ([`Editor`]) and its command dispatch, the domain
//! API ([`domain`]: what a domain registers and the state it keeps), the window ([`ui`], [`run`]), Bevy UI building
//! blocks, input helpers, user config, undo history, dynamic value types, open tabs ([`tab`], [`doc_kind`]) and the
//! editor kinds' window areas ([`editors`]). Depends on no kit, domain or `shader_*` crate.

#![allow(clippy::type_complexity)] // Bevy system signatures

pub mod action;
pub mod app;
pub mod clipboard;
pub mod config;
pub mod demo;
pub mod doc_kind;
pub mod domain;
pub mod dynval;
pub mod editors;
pub mod external;
pub mod fixes;
pub mod highlight;
pub mod history;
pub mod keys;
pub mod logo;
pub mod main_input;
pub mod menu;
pub mod message;
pub mod nav;
pub mod preview;
pub mod quick_start;
pub mod search;
pub mod settings;
pub mod state;
pub mod tab;
pub mod theme;
pub mod tooltip;
pub mod ui;
pub mod util;

pub use app::{EditorCorePlugin, EditorPlugins, apply_actions, run};
pub use state::editor::{Editor, UiRev};

/// Display name of the editor shown on the welcome screen and in Help → About.
pub const EDITOR_TITLE: &str = "Quillshade Editor";

/// Editor version (Help → About, `--version`): the release tag, which CI passes as `SHADER_EDITOR_VERSION` at build
/// time; the workspace version otherwise.
pub const EDITOR_VERSION: &str = match option_env!("SHADER_EDITOR_VERSION") {
    Some(v) => v,
    None => env!("CARGO_PKG_VERSION"),
};
pub const PROJECT_URL: &str = "https://github.com/fe3dback/bevy-quillshade";
pub const AUTHORS: &str = "AI / fe3dback";
pub const LICENSE: &str = "MIT";
/// Shown small and dimmed on the welcome screen and in About.
pub const DISCLAIMER: &str = "Unofficial, community-driven editor, not supported by the Bevy developers.";
