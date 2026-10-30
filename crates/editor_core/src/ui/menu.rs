//! What the mainbar's main menus hold (design §8.1, §14.2): core items plus the contributions of the active domain
//! and of the project domain. Assets lists the active domain's "New …" templates, then the domains' Assets items, then
//! New folder (if the active domain has asset kinds) and, after a separator, Generate; Help starts with Quick Start.
//! A menu's tool items (design §14.1) follow the domains' items, set off from core's by a separator; File's follow
//! "Close Project" instead (Project Settings…).

use crate::action::{Action, TabAction, Unsaved};
use crate::config::UserConfig;
use crate::menu::{MainMenu, MenuEntry, menu_separator};
use crate::ui::icons::{IconId, IconRef};
use project_core::DomainId;

use crate::state::editor::Editor;

/// The "New …" items of `domain`'s asset kinds (every domain's when `None`), in registration order.
pub fn new_asset_items(editor: &Editor, domain: Option<DomainId>) -> Vec<(Option<IconRef>, String, Action)> {
    let list = &editor.domain_list;
    let domains = list.domains().iter().filter(|d| domain.is_none_or(|id| d.id == id));
    let kinds = domains.flat_map(|d| d.asset_kinds.iter().map(move |k| (d.id, k)));
    kinds
        .flat_map(|(id, k)| {
            k.templates.iter().enumerate().map(move |(template, t)| {
                let action = Action::NewAssetDialog { kind: (id, k.suffix), template };
                (Some(t.icon), format!("New {}...", t.label), action)
            })
        })
        .collect()
}

/// The items opening the tools registered with `menu` (design §14.1): `"{title}..."`, the active domain's, then the
/// project domain's (reachable from every domain, design §14.2).
pub fn tool_items(editor: &Editor, menu: MainMenu) -> Vec<MenuEntry> {
    let active = editor.active_domain;
    let domains = editor.domain_list.shown(active);
    let tools = domains.flat_map(|d| d.tools.iter().filter(|t| t.menu == Some(menu)).map(move |t| (d.id, t)));
    tools
        .map(|(domain, t)| {
            let action = Action::OpenTool { domain, id: t.id, arg: None };
            MenuEntry::from((None, format!("{}...", t.title), action))
        })
        .collect()
}

/// The menu's entries: the domains' contributions and core's items ([`menu_items`]), the View toggles checked from
/// `editor`'s config, Back / Forward dimmed at the ends of the history (decision 0041; whether the entry still opens
/// is not checked).
pub fn menu_entries(menu: MainMenu, editor: &Editor) -> Vec<MenuEntry> {
    let [back, forward] = nav_enabled(editor);
    let active = editor.active_domain;
    // The active real domain's asset kinds (none for the project domain).
    let kinds = active.filter(|d| !editor.domain_list.asset_kinds(*d).is_empty());
    let mut entries: Vec<MenuEntry> = Vec::new();
    if menu == MainMenu::Assets
        && let Some(domain) = kinds
    {
        entries.extend(new_asset_items(editor, Some(domain)).into_iter().map(MenuEntry::from));
    }
    for items in editor.domain_list.menu_items(menu, active) {
        entries.extend(items(editor));
    }
    let tools = tool_items(editor, menu);
    let mut core: Vec<MenuEntry> = menu_items(menu, &editor.config).into_iter().map(MenuEntry::from).collect();
    match menu {
        // Right after Close Project (design §8.1: Project Settings...).
        MainMenu::File => {
            let at = core.iter().position(|e| matches!(e.action, Action::CloseProject(_))).map_or(0, |i| i + 1);
            core.splice(at..at, tools);
        }
        _ if !tools.is_empty() => {
            entries.extend(tools);
            entries.push(menu_separator().into());
        }
        _ => {}
    }
    if menu == MainMenu::Assets {
        if let Some(domain) = kinds {
            let folder = Action::NewFolderDialog(Some(domain));
            entries.push((Some(IconId::FolderNew.into()), "New folder...".to_string(), folder).into());
        }
        if !entries.is_empty() {
            entries.push(menu_separator().into());
        }
    }
    entries.extend(core);
    for entry in &mut entries {
        entry.disabled = match entry.action {
            Action::NavBack if !back => Some("Nothing to go back to".into()),
            Action::NavForward if !forward => Some("Nothing to go forward to".into()),
            _ => entry.disabled.take(),
        };
    }
    entries
}

/// Whether Back / Forward can go anywhere (decision 0041; whether the entry still opens is not checked): the View menu
/// items and the mainbar buttons are dimmed when not.
pub fn nav_enabled(editor: &Editor) -> [bool; 2] {
    [editor.nav.can_back(editor.active_key().as_ref()), editor.nav.can_forward()]
}

/// Core's items of a menu; `config` checks the View toggles that are on.
pub fn menu_items(menu: MainMenu, config: &UserConfig) -> Vec<(Option<IconRef>, String, Action)> {
    let check = |on: bool| on.then_some(IconId::Check.into());
    match menu {
        MainMenu::File => vec![
            (Some(IconId::FolderNew.into()), "New Project...".into(), Action::NewProjectDialog),
            (Some(IconId::FolderOpen.into()), "Open Project...\tCtrl+O".into(), Action::OpenProjectDialog),
            (Some(IconId::Close.into()), "Close Project".into(), Action::CloseProject(Unsaved::Ask)),
            menu_separator(),
            (Some(IconId::Save.into()), "Save\tCtrl+S".into(), Action::Save),
            (Some(IconId::SaveAll.into()), "Save All\tCtrl+Shift+S".into(), Action::SaveAll),
            menu_separator(),
            (None, "Quit\tCtrl+Q".into(), Action::Quit),
        ],
        MainMenu::Edit => vec![
            (Some(IconId::Undo.into()), "Undo\tCtrl+Z".into(), Action::Tab(TabAction::Undo)),
            (Some(IconId::Redo.into()), "Redo\tCtrl+Shift+Z".into(), Action::Tab(TabAction::Redo)),
        ],
        // Design §3: each panel's mark is its own flag, also while Zen mode hides the panel.
        MainMenu::View => vec![
            (check(config.zen_mode), "Zen mode\t`".into(), Action::ToggleZen),
            menu_separator(),
            (Some(IconId::Back.into()), "Back\tCtrl+[".into(), Action::NavBack),
            (Some(IconId::Forward.into()), "Forward\tCtrl+]".into(), Action::NavForward),
            menu_separator(),
            (check(config.show_left_sidebar), "Show project\tCtrl+1".into(), Action::ToggleLeftSidebar),
            (check(config.show_log), "Show console\tCtrl+2".into(), Action::ToggleLog),
            (check(config.show_properties), "Show properties\tCtrl+3".into(), Action::ToggleProperties),
        ],
        // The active domain's items and New folder come first ([`menu_entries`]); Generate regenerates every domain.
        MainMenu::Assets => vec![(Some(IconId::Regenerate.into()), "Generate".into(), Action::RegenAll)],
        // Quick Start: core's steps, then every domain's (design §8.2).
        MainMenu::Help => vec![
            (Some(IconId::Help.into()), "Quick Start".into(), Action::QuickStart(0)),
            (Some(IconId::Link.into()), "Project Page".into(), Action::OpenUrl(crate::PROJECT_URL.into())),
            menu_separator(),
            (Some(IconId::Info.into()), "About".into(), Action::About),
        ],
    }
}
