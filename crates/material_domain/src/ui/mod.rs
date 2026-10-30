//! Material's window parts: the editor content's views (consts, image, Texture, gradients;
//! `DomainRegistrar::doc_view`), node preview kinds, the palette's entries, the placement keys and the
//! Material property widgets (the canvas, its gestures and the palette popup are `node_graph_base`'s). The window shell and the core panels are in [`editor_core::ui`], re-exported here.

pub use editor_core::ui::{areas, browser, dialog, log, tabs, welcome};
pub mod const_preview;
pub mod consts_view;
pub mod curve_preview;
pub mod gradient_preview;
pub mod gradient_strip;
pub mod gradient_view;
pub use editor_core::ui::icons;
pub mod input;
pub use editor_core::logo;
pub mod node_preview;
pub mod palette;
pub mod props;
pub mod texture_preview;
pub use editor_core::tooltip;
pub mod tree;
pub mod widgets;

pub use editor_core::editors::ViewOf;
pub use editor_core::menu::MainMenu;
pub use editor_core::theme::{ACCENT, ACCENT_RGB};
pub use editor_core::theme::{BORDER, PANEL_BG, ROW_HOVER, TEXT, TEXT_DIM, TOPBAR_BG, text};
pub use editor_core::ui::HoverRow;
pub use editor_core::ui::OnActivate;
pub use editor_core::ui::shell::{
    DocArea, DocKindText, EmptyHint, HEADER_BG, LeftSidebar, MainMenuBarrier, MainMenuPopup, NavButton, ProjectText,
    ROW_ACTIVE, UiPlugin, VIEW_BG, activate_action, clipped_text, fill_main_menus, left_sidebar, main_menu,
    main_menu_barrier, project_name, show_left_sidebar,
};
pub use editor_core::ui::widgets::scroll_column;
pub use editor_core::ui::{button, icon_button, icon_tool_button, tool_button};

/// The graph canvas entity (`node_graph_base`'s; `register_graph` spawns Material's graph view with it).
pub use node_graph_base::EditorCanvasMarker;

#[cfg(test)]
mod tests {
    use shader_project::{AssetId, AssetType, Project, fixtures};

    use super::MainMenu;
    use crate::ops::{Action, Editor};
    use editor_core::ui::menu::menu_entries;

    /// Decision 0041: Back / Forward are dimmed (with the reason) at the ends of the history; validity is not checked.
    #[test]
    fn view_menu_back_forward_disabled_at_ends() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"g\"\n").unwrap();
        std::fs::create_dir_all(dir.path().join("assets")).unwrap();
        fixtures::save_all(&Project::open(dir.path()).unwrap()).unwrap();
        let mut e = crate::test_editor();
        e.open_project(dir.path()).unwrap();
        let disabled = |e: &Editor| {
            let entries = menu_entries(MainMenu::View, e);
            let of = |action: Action| entries.iter().find(|m| m.action == action).unwrap().disabled.clone();
            (of(Action::NavBack), of(Action::NavForward))
        };
        let back = Some("Nothing to go back to".to_string());
        let forward = Some("Nothing to go forward to".to_string());

        e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet").to_ref()));
        assert_eq!(disabled(&e), (back.clone(), forward.clone()), "one tab");
        e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "fx/hologram").to_ref()));
        assert_eq!(disabled(&e), (None, forward.clone()), "a second tab");
        e.apply(Action::NavBack);
        assert_eq!(disabled(&e), (back, None), "back at the first");
    }
}
