//! The window shell (spec §9, `AGENTS.md` → "Editor"): mainbar (top: main menu and domain tabs), the secondbar across
//! the whole window under it (main actions and tabs, design §14.3), then left sidebar (project tree, assets tree),
//! center column (editor topbar, editor content, editor statusbar, console/logs) and right sidebar; the welcome screen
//! covers the secondbar and the panels while no project is open. The editor topbar / statusbar and the right sidebar
//! belong to the active editor kind ([`super::areas`]); the editor content's views come from the domains
//! (`DomainRegistrar::doc_view`, design §8.1).

use std::path::Path;

use crate::action::{Action, Actions};
use crate::config::Panel;
use crate::editors::{EditorKinds, ViewOf};
use crate::logo;
use crate::menu::MainMenu;
use crate::theme::{BORDER, PANEL_BG, TEXT, TEXT_DIM, text};
use crate::tooltip::{self, Tooltip};
use crate::ui::icons;
use crate::ui::widgets::{self, pane_box, pane_title, scroll_column};
use crate::util::text::plural;
use bevy::feathers::constants::fonts;
use bevy::feathers::controls::{ButtonVariant, FeathersMenu, FeathersMenuToolButton};
use bevy::feathers::display::caption;
use bevy::feathers::theme::ThemeBackgroundColor;
use bevy::feathers::tokens;
use bevy::input_focus::tab_navigation::TabGroup;
use bevy::prelude::*;
use bevy::text::FontSourceTemplate;
use bevy::ui::{InteractionDisabled, Pressed};
use bevy::ui_widgets::{Activate, MenuButton};

use super::browser::LeftPane;
use super::menu::{menu_entries, nav_enabled};
use super::{OnActivate, areas, browser, dialog, log, tabs, welcome};
use crate::domain::{DomainList, Highlighter};
use crate::state::editor::Editor;
use crate::ui::props::PropWidgetsPlugin;
use project_core::DomainId;

/// The area the active tab's view fills (below the editor topbar).
#[derive(Component, Default, Clone)]
pub struct DocArea;

/// The mainbar (design §14.3): logo, main menu, domain tabs, then the project name and error count.
#[derive(Component, Default, Clone)]
pub struct MainBar;

#[derive(Component, Default, Clone)]
pub struct EmptyHint;

/// The mainbar's project text (`name  |  N errors`, the full path as its tooltip).
#[derive(Component, Default, Clone)]
pub struct ProjectText;

/// Watermark in the main view's bottom-right corner naming the open document's type.
#[derive(Component, Default, Clone)]
pub struct DocKindText;

/// Background of the active row of a list (a tree, the palette).
pub const ROW_ACTIVE: Color = Color::srgba(0.35, 0.55, 0.95, 0.30);
pub const HEADER_BG: Color = Color::srgb_u8(40, 42, 48);
/// Background of document views (the window clear color the graph canvas shows through).
pub const VIEW_BG: Color = Color::srgb_u8(24, 25, 29);

/// Narrowest the center column (editor content and its bars) gets: in a smaller window the sidebars shrink.
const CENTER_MIN_W: f32 = 400.0;

/// The window's UI (design §8.1): the shell, the core panels and, in their place, every domain's window plugins and
/// systems (`DomainRegistrar::bevy`).
pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            browser::BrowserPlugin,
            tabs::TabsPlugin,
            log::LogPlugin,
            dialog::DialogPlugin,
            icons::IconsPlugin,
            tooltip::TooltipPlugin,
            areas::AreasPlugin,
            logo::LogoPlugin,
            welcome::WelcomePlugin,
            super::text_view::TextViewPlugin,
        ));
        app.add_plugins(DomainWindowPlugin);
        app.add_plugins(crate::main_input::InputContextsPlugin).add_systems(Startup, spawn_layout);
        app.add_systems(
            Update,
            (
                show_active_view,
                show_empty_hint,
                show_doc_kind,
                show_project,
                fill_main_menus,
                sync_nav_buttons,
                main_menu_barrier,
                show_left_sidebar,
                close_requested,
            ),
        )
        .add_observer(activate_action)
        .add_observer(crate::ui::row_hover_on)
        .add_observer(crate::ui::row_hover_off)
        .add_observer(tabs::tab_click);
    }
}

/// The domains' window parts (design §6.1): the core property widgets' observers ([`PropWidgetsPlugin`]), then every
/// domain's own systems and plugins (`DomainRegistrar::bevy`, from the list `EditorCorePlugin` put in the app). Part of
/// [`UiPlugin`]; a headless UI test adds it to get the domains' window parts the way the window does.
pub struct DomainWindowPlugin;

impl Plugin for DomainWindowPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<PropWidgetsPlugin>() {
            app.add_plugins(PropWidgetsPlugin);
        }
        let domains = app.world().get_resource::<DomainList>().cloned().unwrap_or_default();
        for add in domains.domains().iter().flat_map(|d| d.bevy.clone()) {
            add(app);
        }
    }
}

/// The domains' views of the editor content (`DomainRegistrar::doc_view`), in registration order; the window spawns
/// them after core's own text viewer.
pub fn domain_doc_views(list: &DomainList) -> Vec<Box<dyn Scene>> {
    list.domains().iter().flat_map(|d| &d.doc_views).map(|f| f()).collect()
}

/// Closing the window asks to quit (`Action::Quit`: unsaved changes ask first).
fn close_requested(mut ev: MessageReader<bevy::window::WindowCloseRequested>, mut actions: ResMut<Actions>) {
    if ev.read().count() > 0 {
        actions.push(Action::Quit);
    }
}

/// Pushes the [`OnActivate`] action of an activated widget (not sent for an `InteractionDisabled` one).
pub fn activate_action(ev: On<Activate>, q: Query<&OnActivate>, mut actions: ResMut<Actions>) {
    if let Ok(a) = q.get(ev.event_target()) {
        actions.push(a.0.clone());
    }
}

/// One line of text that is clipped instead of wrapped when the row is too narrow (a path in an editor topbar).
pub fn clipped_text(s: impl Into<String>, color: Color, font_size: f32) -> impl Scene {
    let (s, layout) = (s.into(), TextLayout::no_wrap());
    bsn! {
        Node { min_width: px(0), overflow: Overflow::clip() }
        Children [ @text(s, color, font_size) layout ]
    }
}

/// The popup of a main menu, filled with its items by [`fill_main_menus`].
#[derive(Component, Default, Clone, Copy)]
pub struct MainMenuPopup(pub MainMenu);

/// A main menu: a `FeathersMenu` holding a plain menu tool button (no chevron) and its popup, which opens under the
/// button. Feathers opens and closes it; [`fill_main_menus`] fills it.
pub fn main_menu(menu: MainMenu) -> impl Scene {
    let (label, popup) = (menu.label(), MainMenuPopup(menu));
    bsn! {
        @FeathersMenu
        menu
        Children [
            @FeathersMenuToolButton { @caption: bsn! { @caption(label) }, @arrow: false }
            ButtonVariant::Plain
            --
            @dialog::menu_popup(Vec::new())
            popup
        ]
    }
}

/// The [`dialog::menu_barrier`] of an open main menu: a click outside the popup (the mainbar's buttons included) only
/// closes the menu, as with a context menu.
#[derive(Component, Default, Clone)]
pub struct MainMenuBarrier;

/// Spawns the [`MainMenuBarrier`] while a main menu's popup is shown and removes it once none is, and gives each main
/// menu button Feathers' active look (`Pressed`) exactly while its popup is shown.
///
/// The button gets `Pressed` from the press that opens the menu (`ActivateOnPress`) and loses it only on a release
/// over itself. That release lands on the barrier, spawned under the pointer in the meantime, so the button would keep
/// looking active after the menu closed and ignore its next press (design §1). Following the popup's visibility here
/// covers every way the menu closes: an outside click, Esc, an item, another menu.
pub fn main_menu_barrier(
    popups: Query<(&Visibility, &ChildOf), With<MainMenuPopup>>,
    children: Query<&Children>,
    buttons: Query<Has<Pressed>, With<MenuButton>>,
    barriers: Query<Entity, With<MainMenuBarrier>>,
    mut commands: Commands,
) {
    let mut open = false;
    for (visibility, menu) in &popups {
        let shown = *visibility == Visibility::Visible;
        open |= shown;
        for button in children.get(menu.parent()).into_iter().flatten() {
            match buttons.get(*button) {
                Ok(false) if shown => {
                    commands.entity(*button).try_insert(Pressed);
                }
                Ok(true) if !shown => {
                    commands.entity(*button).try_remove::<Pressed>();
                }
                _ => {}
            }
        }
    }
    if open && barriers.is_empty() {
        commands.spawn_scene(bsn! { @dialog::menu_barrier() MainMenuBarrier });
    } else if !open {
        for b in &barriers {
            commands.entity(b).despawn();
        }
    }
}

/// (Re)fills the main menus' popups: when spawned, when a View toggle changed (its check mark), when Back / Forward
/// became possible or impossible (dimmed) and when the active domain changed (its items, design §14.2).
pub fn fill_main_menus(
    editor: Res<Editor>,
    popups: Query<(Entity, &MainMenuPopup)>,
    added: Query<(), Added<MainMenuPopup>>,
    mut last: Local<Option<([bool; 6], Option<DomainId>)>>,
    mut commands: Commands,
) {
    let c = &editor.config;
    let [back, forward] = nav_enabled(&editor);
    let key = ([c.zen_mode, c.show_left_sidebar, c.show_log, c.show_properties, back, forward], editor.active_domain);
    if *last == Some(key) && added.is_empty() {
        return;
    }
    *last = Some(key);
    for (popup, MainMenuPopup(menu)) in &popups {
        commands.entity(popup).despawn_children();
        for row in dialog::menu_rows(menu_entries(*menu, &editor)) {
            commands.spawn_scene(bsn! { @{row} ChildOf(popup) });
        }
    }
}

/// A secondbar Back / Forward button ([`sync_nav_buttons`]): `0` is Back, `1` Forward (the index into [`nav_enabled`]).
#[derive(Component, Default, Clone, Copy)]
pub struct NavButton(pub usize);

/// Dims the secondbar's Back / Forward buttons at the ends of the history: `InteractionDisabled` (no `Activate`,
/// Feathers' disabled look) and the icon's alpha, which Feathers does not dim. Written only on change.
fn sync_nav_buttons(
    editor: Res<Editor>,
    buttons: Query<(Entity, &NavButton, Has<InteractionDisabled>)>,
    children: Query<&Children>,
    mut images: Query<&mut ImageNode>,
    mut commands: Commands,
) {
    let enabled = nav_enabled(&editor);
    for (button, NavButton(i), disabled) in &buttons {
        let on = enabled.get(*i).copied().unwrap_or(false);
        if on != disabled {
            continue;
        }
        if on {
            commands.entity(button).remove::<InteractionDisabled>();
        } else {
            commands.entity(button).insert(InteractionDisabled);
        }
        let tint = if on { TEXT } else { TEXT.with_alpha(widgets::DISABLED_ICON_ALPHA) };
        for e in children.iter_descendants(button) {
            if let Ok(mut image) = images.get_mut(e) {
                image.color = tint;
            }
        }
    }
}

/// Logo, main menu, the domain tabs after a separator (design §14.3), then the project name and error count. Nothing
/// of one editor kind (that is the editor topbar's); Back / Forward are in the secondbar.
fn main_bar() -> impl Scene {
    let tip = Tooltip::default();
    let menus: Vec<Box<dyn Scene>> =
        MainMenu::ORDER.into_iter().map(|m| Box::new(main_menu(m)) as Box<dyn Scene>).collect();
    bsn! {
        MainBar
        Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: px(2),
            padding: UiRect::axes(px(6), px(3)),
            border: UiRect::bottom(px(1)),
        }
        BackgroundColor(HEADER_BG)
        BorderColor::all(BORDER)
        Children [
            @logo::logo(20.0) Node { margin: UiRect::axes(px(4), px(0)) }
            --
            {menus}
            --
            // The separator is the domain tabs' left border, as high as the menu buttons.
            @tabs::domain_tabs()
            Node {
                align_self: AlignSelf::Stretch,
                margin: UiRect::left(px(6)),
                padding: UiRect::left(px(8)),
                border: UiRect::left(px(1)),
            }
            BorderColor::all(BORDER)
            --
            Node { flex_grow: 1.0 }
            --
            ProjectText
            Text("")
            TextFont { font_size: FontSize::Px(12.0) }
            TextColor(TEXT_DIM)
            tip
            Node { margin: UiRect::right(px(6)) }
        ]
    }
}

/// The left sidebar (project tree and assets tree), shown by [`show_left_sidebar`].
#[derive(Component, Default, Clone)]
pub struct LeftSidebar;

/// The left sidebar: the active domain's project tree and assets tree, a pane each, or the project domain's tools
/// ([`browser::LeftPane`], design §14.4). `min_width: 0`: below `CENTER_MIN_W` it shrinks past its content (the rows
/// clip) instead of pushing the window wider.
pub fn left_sidebar() -> impl Scene {
    bsn! {
        LeftSidebar
        Node {
            width: px(270),
            min_width: px(0),
            flex_direction: FlexDirection::Column,
            row_gap: px(4),
            padding: UiRect::all(px(4)),
            border: UiRect::right(px(1)),
        }
        BackgroundColor(PANEL_BG)
        BorderColor::all(BORDER)
        Children [
            @pane_box(pane_title("Project"), Vec::new(), bsn! { @scroll_column() browser::BrowserBody })
            Node { flex_grow: 3.0, flex_basis: px(0), min_height: px(0) }
            LeftPane::Project
            --
            @pane_box(pane_title("Game Assets"), Vec::new(), bsn! { @scroll_column() browser::GameAssetsBody })
            Node { flex_grow: 2.0, flex_basis: px(0), min_height: px(0) }
            LeftPane::GameAssets
            --
            // The project domain's tools, in place of both (design §14.2); hidden until it is active.
            @pane_box(pane_title("Tools"), Vec::new(), bsn! { @scroll_column() browser::ProjectToolsBody })
            Node { flex_grow: 1.0, flex_basis: px(0), min_height: px(0), display: Display::None }
            LeftPane::Tools
            Visibility::Hidden
        ]
    }
}

/// Follows [`Panel::LeftSidebar`] (View → Show project, Zen mode): hidden, the left sidebar takes no space and Tab
/// skips it ([`areas::display`]). Runs when the flag changed or the sidebar was spawned.
pub fn show_left_sidebar(
    editor: Res<Editor>,
    mut last: Local<Option<bool>>,
    spawned: Query<(), Added<LeftSidebar>>,
    mut sidebars: Query<(&mut Node, &mut Visibility), With<LeftSidebar>>,
) {
    let shown = editor.config.panel_visible(Panel::LeftSidebar);
    if *last == Some(shown) && spawned.is_empty() {
        return;
    }
    *last = Some(shown);
    for (mut node, mut visibility) in &mut sidebars {
        areas::display(&mut node, &mut visibility, shown);
    }
}

/// The project's name: the root directory's name (the whole path when it has none, e.g. `/`).
pub fn project_name(root: &Path) -> String {
    root.file_name().map_or_else(|| root.display().to_string(), |n| n.to_string_lossy().into_owned())
}

fn spawn_layout(mut commands: Commands, list: Res<DomainList>) {
    commands.spawn(Camera2d);
    // Core's own kind (the generated-file text viewer) first, then the domains'.
    let views: Vec<Box<dyn Scene>> = std::iter::once(Box::new(super::text_view::text_view()) as Box<dyn Scene>)
        .chain(domain_doc_views(&list))
        .collect();
    let overlays: Vec<Box<dyn Scene>> = list.domains().iter().flat_map(|d| &d.doc_overlays).map(|f| f()).collect();
    commands.spawn_scene(shell(views, overlays, list.highlighter("rust")));
}

/// The window's UI root: mainbar, then the secondbar, then left sidebar | centre column | right sidebar (design
/// §14.3); `views` are the editor content's views (under the empty hint and the watermark), `overlays` go above them,
/// `rust` highlights the welcome screen's snippet. Headless UI tests spawn it without views.
pub fn shell(views: Vec<Box<dyn Scene>>, overlays: Vec<Box<dyn Scene>>, rust: Option<Highlighter>) -> impl Scene {
    bsn! {
        // `min_width` / `min_height: 0`: Bevy lays a UI root out in an implicit grid cell, which would otherwise grow
        // to the root's min-content size (the editor topbar's buttons do not wrap; the sidebars' panes) and push the
        // right sidebar and the console/logs out of the window.
        Node {
            width: percent(100),
            height: percent(100),
            min_width: px(0),
            min_height: px(0),
            flex_direction: FlexDirection::Column,
        }
        ThemeBackgroundColor(tokens::WINDOW_BG)
        TabGroup
        Children [
            @main_bar()
            --
            // Below the mainbar: the secondbar over the panels' row; the welcome screen covers both.
            Node { flex_grow: 1.0, flex_basis: px(0), flex_direction: FlexDirection::Column, min_height: px(0) }
            Children [
                @tabs::second_bar()
                --
                Node { flex_grow: 1.0, flex_basis: px(0), flex_direction: FlexDirection::Row, min_height: px(0) }
                Children [
                    @left_sidebar()
                    --
                    // Center: editor topbar, editor content, editor statusbar, console/logs. `flex_basis: 0` here
                    // and on the editor content: they take what is left, so the sidebars keep their width and the
                    // log its height (with a content-sized basis every sibling shrinks with them). Below
                    // `CENTER_MIN_W` the sidebars give way instead.
                    Node {
                        flex_grow: 1.0,
                        flex_basis: px(0),
                        flex_direction: FlexDirection::Column,
                        min_width: px(CENTER_MIN_W),
                    }
                    Children [
                        @areas::editor_topbar()
                        --
                        DocArea
                        Node { flex_grow: 1.0, flex_basis: px(0), min_height: px(0) }
                        Children [
                            {views}
                            --
                            EmptyHint
                            // `right` bounds the width: the hint wraps instead of running past the editor content.
                            Node { position_type: PositionType::Absolute, left: px(24), right: px(24), top: px(24) }
                            Pickable::IGNORE
                            Children [ @empty_hint_text() ]
                            --
                            // Over the active view, whatever kind it is.
                            DocKindText
                            Text("")
                            TextFont { font_size: FontSize::Px(72.0), weight: FontWeight::BOLD }
                            TextColor(Color::srgba(1.0, 1.0, 1.0, 0.1))
                            Node { position_type: PositionType::Absolute, right: px(20), bottom: px(8) }
                            Pickable::IGNORE
                            --
                            {overlays}
                        ]
                        --
                        @areas::editor_statusbar()
                        --
                        @log::console()
                    ]
                    --
                    @areas::right_sidebar()
                ]
                --
                @welcome::welcome(rust)
            ]
        ]
    }
}

/// Shows the active tab's view; without a tab, the first registered kind's (Material: the empty graph canvas).
fn show_active_view(editor: Res<Editor>, kinds: Res<EditorKinds>, mut q: Query<(&ViewOf, &mut Visibility)>) {
    let Some(active) = editor.tab().map(|t| t.doc().view()).or_else(|| kinds.0.first().map(|k| k.view)) else {
        return;
    };
    for (view, mut v) in &mut q {
        v.set_if_neq(if view.0 == active { Visibility::Inherited } else { Visibility::Hidden });
    }
}

/// The text of the [`EmptyHint`], which [`show_empty_hint`] sets for the active domain.
#[derive(Component, Default, Clone)]
pub struct EmptyHintText;

/// The empty hint of a real domain without an active tab (design §14.3).
pub const ASSET_HINT: &str = "No open tab — open an asset from the project tree";
/// The empty hint of the project domain without an active tab: its left sidebar lists the tools.
pub const TOOL_HINT: &str = "Open a tool from the list";

/// The [`EmptyHint`]'s text, in the full Fira Mono: the default font's subset has no em dash.
fn empty_hint_text() -> impl Scene {
    bsn! {
        @text(ASSET_HINT, TEXT_DIM, 14.0)
        TextFont { font: FontSourceTemplate::Handle(fonts::MONO) }
        EmptyHintText
    }
}

/// The empty hint of the active domain.
pub fn empty_hint(editor: &Editor) -> &'static str {
    if editor.active_domain == Some(DomainId::PROJECT) { TOOL_HINT } else { ASSET_HINT }
}

/// Shows the [`EmptyHint`] while no tab is active, with the active domain's text.
fn show_empty_hint(
    editor: Res<Editor>,
    mut q: Query<&mut Visibility, With<EmptyHint>>,
    mut texts: Query<&mut Text, With<EmptyHintText>>,
) {
    let want = if editor.tab().is_none() { Visibility::Inherited } else { Visibility::Hidden };
    for mut v in &mut q {
        v.set_if_neq(want);
    }
    let hint = empty_hint(&editor);
    for mut t in &mut texts {
        if t.0 != hint {
            t.0 = hint.into();
        }
    }
}

/// Watermark text: the asset type of a graph, the language of a generated file
/// ([`crate::doc_kind::TabDoc::watermark`]).
fn show_doc_kind(editor: Res<Editor>, mut q: Query<&mut Text, With<DocKindText>>) {
    let label = editor.tab().and_then(|t| t.doc().watermark()).unwrap_or("");
    for mut t in &mut q {
        if t.0 != label {
            t.0 = label.into();
        }
    }
}

/// The mainbar's project text and tooltip, and the window title. Keyed like the editor statusbar (`areas`): the log
/// revision (diagnostics; opening or closing a project logs), the tabs revision (tab titles, saves), the active tab
/// and its document's revision (the dirty mark). Not on every change of `Editor`: the error count clones all
/// diagnostics and the dirty mark serialises a graph.
fn show_project(
    editor: Res<Editor>,
    mut q: Query<(&mut Text, &mut Tooltip), With<ProjectText>>,
    spawned: Query<(), Added<ProjectText>>,
    mut windows: Query<&mut Window>,
    mut last: Local<Option<(u64, u64, Option<usize>, u64)>>,
) {
    let key = (editor.ui.log, editor.ui.tabs, editor.active, editor.tab().map_or(0, |t| t.doc().revision()));
    if *last == Some(key) && spawned.is_empty() {
        return;
    }
    *last = Some(key);
    let (status, path) = match &editor.project {
        Some(p) => {
            let errors = editor.error_count();
            (
                format!("{}  |  {}", project_name(&p.root), plural(errors, "error", "errors")),
                p.root.display().to_string(),
            )
        }
        None => ("No project".into(), "No project open".into()),
    };
    for (mut t, mut tip) in &mut q {
        if t.0 != status {
            t.0 = status.clone();
        }
        if tip.0 != path {
            tip.0 = path.clone();
        }
    }
    let title = editor.window_title();
    for mut w in &mut windows {
        if w.title != title {
            w.title = title.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::action::{Action, TabAction};
    use crate::config::UserConfig;
    use crate::menu::MainMenu;
    use crate::ui::icons::{IconId, IconRef};

    use super::project_name;
    use crate::ui::menu::menu_items;

    /// Design §3: the View menu lists Zen mode, a separator, Back / Forward and a separator (decision 0041), and the
    /// three panels with their hotkeys; each check mark is its own flag (Zen on does not clear the panels' marks); Edit
    /// sits between File and View.
    #[test]
    fn view_menu_check_marks_follow_config() {
        let marks = |zen_mode, show_left_sidebar, show_log, show_properties| {
            let config = UserConfig { zen_mode, show_left_sidebar, show_log, show_properties, ..UserConfig::default() };
            menu_items(MainMenu::View, &config)
        };
        let check = Some(IconId::Check.into());
        assert_eq!(
            marks(false, true, true, true),
            [
                (None, "Zen mode\t`".to_string(), Action::ToggleZen),
                (None, String::new(), Action::None),
                (Some(IconId::Back.into()), "Back\tCtrl+[".to_string(), Action::NavBack),
                (Some(IconId::Forward.into()), "Forward\tCtrl+]".to_string(), Action::NavForward),
                (None, String::new(), Action::None),
                (check, "Show project\tCtrl+1".to_string(), Action::ToggleLeftSidebar),
                (check, "Show console\tCtrl+2".to_string(), Action::ToggleLog),
                (check, "Show properties\tCtrl+3".to_string(), Action::ToggleProperties),
            ]
        );
        let icons = |m: Vec<(Option<IconRef>, String, Action)>| m.into_iter().map(|i| i.0).collect::<Vec<_>>();
        assert_eq!(
            icons(marks(true, true, true, true)),
            [check, None, Some(IconId::Back.into()), Some(IconId::Forward.into()), None, check, check, check]
        );
        assert_eq!(
            icons(marks(true, false, true, false)),
            [check, None, Some(IconId::Back.into()), Some(IconId::Forward.into()), None, None, check, None]
        );
        assert_eq!(
            icons(marks(false, true, false, true)),
            [None, None, Some(IconId::Back.into()), Some(IconId::Forward.into()), None, check, None, check]
        );

        let edit = menu_items(MainMenu::Edit, &UserConfig::default());
        let edit: Vec<_> = edit.into_iter().map(|(_, label, action)| (label, action)).collect();
        assert_eq!(
            edit,
            [
                ("Undo\tCtrl+Z".to_string(), Action::Tab(TabAction::Undo)),
                ("Redo\tCtrl+Shift+Z".to_string(), Action::Tab(TabAction::Redo))
            ]
        );
        assert_eq!(&MainMenu::ORDER[..3], [MainMenu::File, MainMenu::Edit, MainMenu::View]);
    }

    #[test]
    fn project_name_is_last_component() {
        assert_eq!(project_name(Path::new("/home/u/games/my_game")), "my_game");
        assert_eq!(project_name(Path::new("/")), "/");
    }
}
