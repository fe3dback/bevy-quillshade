//! The per-kind strips around the editor content (design §1, §2.1, decision 0031): the editor topbar under the tabs
//! bar, the right sidebar and the editor statusbar under the view. The editor topbar and the right sidebar hold one
//! [`AreaOf`] container per kind that has the area (`EditorAreas`), spawned into the strip from [`EditorKinds`] when
//! the strip is; [`show_active_areas`] displays only the active kind's, and a strip with nothing for the active kind
//! is not displayed at all. With no tab open the right sidebar stays, titled "Properties", saying so.

use crate::config::Panel;
use crate::doc_kind::StatusInfo;
use crate::editors::{AreaOf, EditorKinds};
use crate::tab::ViewKindId;
use crate::theme::{BORDER, PANEL_BG, TEXT_DIM, TOPBAR_BG, text};
use crate::tooltip::Tooltip;
use crate::ui::widgets::{pane_box, pane_title};
use crate::util::text::plural;
use bevy::feathers::constants::fonts;
use bevy::feathers::controls::{ButtonVariant, FeathersToolButton};
use bevy::prelude::*;
use bevy::text::FontSource;

use super::OnActivate;
use crate::action::Action;
use crate::state::editor::Editor;

/// The editor topbar strip: the active kind's buttons and info.
#[derive(Component, Default, Clone)]
pub struct EditorTopbar;

/// The right sidebar strip: one pane, the active kind's `EditorAreas::SIDEBAR_TITLE` in its header, the kind's
/// subpanes in its body.
#[derive(Component, Default, Clone)]
pub struct RightSidebar;

/// The right sidebar's pane title.
#[derive(Component, Default, Clone)]
pub struct SidebarTitle;

/// The right sidebar's "Nothing open." shown while no tab is open.
#[derive(Component, Default, Clone)]
pub struct SidebarPlaceholder;

/// The editor statusbar strip: [`StatusInfo`] of the active tab.
#[derive(Component, Default, Clone)]
pub struct EditorStatusbar;

/// The column in the right sidebar's pane body that holds the kinds' areas (after [`SidebarPlaceholder`]).
#[derive(Component, Default, Clone)]
pub struct SidebarAreas;

/// Spawns the kinds' areas into the strips and keeps only the active kind's displayed. Needs [`EditorKinds`]: each
/// kind's `EditorAreas::build` is registered here.
pub struct AreasPlugin;

impl Plugin for AreasPlugin {
    fn build(&self, app: &mut App) {
        // Filled from the domain list by `EditorCorePlugin`, which must come first.
        let Some(kinds) = app.world().get_resource::<EditorKinds>().cloned() else {
            panic!("AreasPlugin needs EditorKinds: add EditorCorePlugin (with the Editor and its domains) first");
        };
        for kind in &kinds.0 {
            (kind.build)(app);
        }
        app.add_systems(Update, (show_active_areas, redraw_statusbar))
            .add_observer(spawn_topbar_areas)
            .add_observer(spawn_sidebar_areas);
    }
}

/// The editor topbar of every kind that has one, in [`EditorKinds`] order.
fn spawn_topbar_areas(add: On<Add<EditorTopbar>>, kinds: Res<EditorKinds>, mut commands: Commands) {
    for spawn in kinds.0.iter().filter_map(|k| k.spawn_topbar) {
        spawn(&mut commands, add.entity);
    }
}

/// The right sidebar body of every kind that has one, in [`EditorKinds`] order.
fn spawn_sidebar_areas(add: On<Add<SidebarAreas>>, kinds: Res<EditorKinds>, mut commands: Commands) {
    for spawn in kinds.0.iter().filter_map(|k| k.spawn_sidebar) {
        spawn(&mut commands, add.entity);
    }
}

/// The editor topbar strip; the kinds' areas are added when it spawns ([`AreasPlugin`]).
pub fn editor_topbar() -> impl Scene {
    bsn! {
        EditorTopbar
        Node {
            display: Display::None,
            flex_direction: FlexDirection::Row,
            min_width: px(0),
            overflow: Overflow::clip(),
            border: UiRect::top(px(1)),
        }
        BackgroundColor(TOPBAR_BG)
        BorderColor::all(BORDER)
    }
}

/// The right sidebar strip; the kinds' areas are added to its [`SidebarAreas`] column when it spawns
/// ([`AreasPlugin`]).
pub fn right_sidebar() -> impl Scene {
    let title = EditorKinds::NO_TAB_TITLE;
    bsn! {
        RightSidebar
        // `min_width: 0`: in a narrow window (the center at its minimum width) the sidebar gives way instead of being
        // pushed out of the window by its content's width.
        Node {
            width: px(330),
            min_width: px(0),
            flex_direction: FlexDirection::Column,
            padding: UiRect::all(px(4)),
            border: UiRect::left(px(1)),
        }
        BackgroundColor(PANEL_BG)
        BorderColor::all(BORDER)
        Children [
            @pane_box(
                bsn! { @pane_title(title) SidebarTitle },
                Vec::new(),
                bsn! {
                    SidebarAreas
                    Node { flex_grow: 1.0, min_height: px(0), flex_direction: FlexDirection::Column }
                    Children [
                        SidebarPlaceholder
                        Node { padding: UiRect::all(px(2)) }
                        Children [ @text("Nothing open.", TEXT_DIM, 13.0) ]
                    ]
                },
            )
            Node { flex_grow: 1.0, min_height: px(0) }
        ]
    }
}

pub fn editor_statusbar() -> impl Scene {
    bsn! {
        EditorStatusbar
        Node {
            display: Display::None,
            height: px(22),
            flex_shrink: 0.0,
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: px(16),
            padding: UiRect::axes(px(10), px(0)),
            border: UiRect::top(px(1)),
        }
        BackgroundColor(TOPBAR_BG)
        BorderColor::all(BORDER)
    }
}

/// Shows or hides an area or strip: `Display::None` so it takes no space, and `Visibility::Hidden` because Bevy's tab
/// navigation skips only hidden entities (`InheritedVisibility`), not `Display::None` ones. Both written on change only.
pub fn display(node: &mut Node, visibility: &mut Visibility, shown: bool) {
    let want = if shown { Display::Flex } else { Display::None };
    if node.display != want {
        node.display = want;
    }
    let want = if shown { Visibility::Inherited } else { Visibility::Hidden };
    if *visibility != want {
        *visibility = want;
    }
}

/// Shows the active kind's areas only; a strip only when that kind has something in it (the right sidebar also
/// while no tab is open, with its placeholder, and never while View → Show properties is off or Zen mode is on:
/// the editor content takes its width). The right sidebar header follows the active kind. Runs when the active tab or
/// its kind or [`Panel::Properties`] changed, or areas were spawned.
pub fn show_active_areas(
    (editor, kinds): (Res<Editor>, Res<EditorKinds>),
    mut last: Local<Option<(Option<usize>, Option<ViewKindId>, bool)>>,
    spawned: Query<(), Added<AreaOf>>,
    (mut areas, parents): (Query<(Entity, &AreaOf, &mut Node, &mut Visibility)>, Query<&ChildOf>),
    mut strips: Query<
        (Entity, &mut Node, &mut Visibility, Has<RightSidebar>),
        (Or<(With<EditorTopbar>, With<RightSidebar>)>, Without<AreaOf>),
    >,
    mut placeholder: Query<
        (&mut Node, &mut Visibility),
        (With<SidebarPlaceholder>, Without<AreaOf>, Without<EditorTopbar>, Without<RightSidebar>),
    >,
    mut title: Query<&mut Text, With<SidebarTitle>>,
) {
    let active: Option<ViewKindId> = editor.tab().map(|t| t.doc().view());
    let properties = editor.config.panel_visible(Panel::Properties);
    let key = (editor.active, active, properties);
    if *last == Some(key) && spawned.is_empty() {
        return;
    }
    *last = Some(key);
    let mut used = Vec::new();
    for (entity, area, mut node, mut visibility) in &mut areas {
        let shown = Some(area.0) == active;
        if shown {
            // The strip may hold the area deeper down (the right sidebar's pane body).
            used.extend(parents.iter_ancestors(entity));
        }
        display(&mut node, &mut visibility, shown);
    }
    for (strip, mut node, mut visibility, sidebar) in &mut strips {
        let content = used.contains(&strip) || (sidebar && active.is_none());
        display(&mut node, &mut visibility, content && (properties || !sidebar));
    }
    for (mut node, mut visibility) in &mut placeholder {
        display(&mut node, &mut visibility, active.is_none());
    }
    let want = kinds.sidebar_title(active);
    for mut t in &mut title {
        if t.0 != want {
            t.0 = want.into();
        }
    }
}

/// What the statusbar shows depends on: the log revision (diagnostics, WESL line counts after a regeneration), the
/// active tab, the tabs revision (bumped whenever a tab is opened, closed, activated, renamed or saved, so another tab
/// at the same index is noticed) and the document's revision. The dirty state is not part of the key (a graph
/// document serialises its asset to tell): it changes only with an edit, an undo / redo or a reload (the revision) or
/// with a save, a rename or "keep mine" (the tabs revision).
type StatusKey = (u64, Option<usize>, u64, u64);

/// Rebuilds the editor statusbar from [`Editor::status_info`] when its key changed (not every frame: the status
/// clones the asset's diagnostics).
fn redraw_statusbar(
    editor: Res<Editor>,
    assets: Res<AssetServer>,
    mut strip: Query<(Entity, &mut Node, &mut Visibility), With<EditorStatusbar>>,
    mut last: Local<Option<StatusKey>>,
    mut commands: Commands,
) {
    let doc = editor.tab().map(|t| t.doc());
    let key = (editor.ui.log, editor.active, editor.ui.tabs, doc.map_or(0, |d| d.revision()));
    if last.as_ref() == Some(&key) {
        return;
    }
    let Ok((strip, mut node, mut visibility)) = strip.single_mut() else { return };
    *last = Some(key);
    commands.entity(strip).despawn_children();
    let Some(status) = editor.status_info() else {
        display(&mut node, &mut visibility, false);
        return;
    };
    display(&mut node, &mut visibility, true);
    let font =
        TextFont { font: FontSource::Handle(assets.load(fonts::MONO)), font_size: FontSize::Px(12.0), ..default() };
    let focus = editor.active_first_diagnostic().map(Action::FocusDiagnostic);
    spawn_status(&mut commands, strip, &status, focus, font);
}

/// `● unsaved` (when dirty), `N errors · M warnings` (a click runs `focus`, which shows the first problem like a
/// console/logs row), the counts. The text is in the full Fira Mono: Bevy's default font is an ASCII subset without
/// `●` / `·`.
fn spawn_status(commands: &mut Commands, strip: Entity, status: &StatusInfo, focus: Option<Action>, font: TextFont) {
    let text = |s: String| (Text::new(s), font.clone(), TextColor(TEXT_DIM), TextLayout::no_wrap(), Pickable::IGNORE);
    if status.dirty {
        commands.spawn((text("● unsaved".into()), ChildOf(strip)));
    }
    let problems = text(format!(
        "{} · {}",
        plural(status.errors, "error", "errors"),
        plural(status.warnings, "warning", "warnings")
    ));
    match focus {
        Some(action) => {
            // A plain Feathers tool button as tall as the strip's text.
            let (act, tip) = (OnActivate(action), Tooltip("Show the first problem".into()));
            let row = commands
                .spawn_scene(bsn! {
                    @FeathersToolButton { @variant: ButtonVariant::Plain }
                    Node { height: Val::Auto, min_width: px(0), padding: UiRect::axes(px(4), px(1)) }
                    act
                    tip
                    ChildOf(strip)
                })
                .id();
            commands.spawn((problems, ChildOf(row)));
        }
        None => {
            commands.spawn((problems, ChildOf(strip)));
        }
    }
    if !status.counts.is_empty() {
        commands.spawn((text(status.counts.join("  ·  ")), ChildOf(strip)));
    }
}

#[cfg(test)]
mod tests {
    use crate::editors::{EditorAreas, KindDef, sidebar_body};

    use super::*;

    /// A kind with a right sidebar but no editor topbar.
    struct NoTopbar;

    #[derive(Component, Default, Clone)]
    struct NoTopbarBody;

    impl EditorAreas for NoTopbar {
        const VIEW: ViewKindId = ViewKindId("test");
        const SIDEBAR_TITLE: &'static str = "Test";

        fn topbar() -> Option<impl Scene> {
            None::<Box<dyn Scene>>
        }

        fn sidebar() -> Option<impl Scene> {
            Some(bsn! { @sidebar_body() NoTopbarBody })
        }
    }

    /// A kind with both areas.
    struct Both;

    impl EditorAreas for Both {
        const VIEW: ViewKindId = ViewKindId("both");
        const SIDEBAR_TITLE: &'static str = "Both";

        fn topbar() -> Option<impl Scene> {
            Some(bsn! { Node })
        }

        fn sidebar() -> Option<impl Scene> {
            Some(bsn! { Node })
        }
    }

    /// The kinds in [`EditorKinds`] get one [`AreaOf`] per strip they have, in the strip they belong to; a kind
    /// whose `topbar()` is `None` gets no editor topbar area.
    #[test]
    fn kind_def_registers_areas() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default(), bevy::scene::ScenePlugin));
        app.init_asset::<Font>().init_asset::<Image>();
        app.insert_resource(Editor::default());
        app.insert_resource(EditorKinds(vec![KindDef::of::<NoTopbar>(), KindDef::of::<Both>()]));
        app.add_plugins(AreasPlugin);
        app.world_mut().spawn_scene(editor_topbar()).unwrap();
        app.world_mut().spawn_scene(right_sidebar()).unwrap();
        app.update();
        let w = app.world_mut();
        let topbar = w.query_filtered::<Entity, With<EditorTopbar>>().single(w).unwrap();
        let sidebar = w.query_filtered::<Entity, With<SidebarAreas>>().single(w).unwrap();
        let areas: Vec<(ViewKindId, Entity)> =
            w.query::<(&AreaOf, &ChildOf)>().iter(w).map(|(a, c)| (a.0, c.parent())).collect();
        let count = |view, strip| areas.iter().filter(|a| **a == (ViewKindId(view), strip)).count();
        assert_eq!(count("test", topbar), 0, "no topbar area: {areas:?}");
        assert_eq!(count("test", sidebar), 1, "{areas:?}");
        assert_eq!(count("both", topbar), 1, "{areas:?}");
        assert_eq!(count("both", sidebar), 1, "{areas:?}");
        assert_eq!(areas.len(), 3);
        assert_eq!(w.query_filtered::<(), With<NoTopbarBody>>().iter(w).count(), 1);
    }
}
