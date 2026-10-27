//! The per-kind strips around the editor content (design §1, §2.1, decision 0031): the editor topbar under the tabs
//! bar, the right sidebar and the editor statusbar under the view. The editor topbar and the right sidebar hold one
//! [`AreaOf`] container per kind that has the area ([`EditorAreas`]); [`show_active_areas`] displays only the active
//! kind's, and a strip with nothing for the active kind is not displayed at all. With no tab open the right sidebar
//! stays, titled "Properties", saying so.

use bevy::feathers::constants::fonts;
use bevy::feathers::controls::{ButtonVariant, FeathersToolButton};
use bevy::prelude::*;
use bevy::text::FontSource;
use shader_graph_core::Diagnostic;

use super::tooltip::Tooltip;
use super::widgets::{pane_box, pane_title};
use super::{BORDER, OnActivate, PANEL_BG, TEXT_DIM, TOPBAR_BG, plural, scroll_column, text};
use crate::editors::{AreaOf, EditorAreas, KindVisitor, StatusInfo, for_each_kind};
use crate::state::{Action, Editor};
use crate::tab::ViewKind;

/// The editor topbar strip: the active kind's buttons and info.
#[derive(Component, Default, Clone)]
pub struct EditorTopbar;

/// The right sidebar strip: one pane, the active kind's [`EditorAreas::SIDEBAR_TITLE`] in its header, the kind's
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

pub struct AreasPlugin;

impl Plugin for AreasPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (show_active_areas, redraw_statusbar));
        for_each_kind(&mut BuildKinds(app));
    }
}

/// Registers every kind's area systems ([`EditorAreas::build`]).
struct BuildKinds<'a>(&'a mut App);

impl KindVisitor for BuildKinds<'_> {
    fn visit<E: EditorAreas>(&mut self) {
        E::build(self.0);
    }
}

/// Header of the right sidebar: the active kind's title, "Properties" while no tab is open.
pub fn sidebar_title(active: Option<ViewKind>) -> &'static str {
    struct Title(ViewKind, &'static str);
    impl KindVisitor for Title {
        fn visit<E: EditorAreas>(&mut self) {
            if E::VIEW == self.0 {
                self.1 = E::SIDEBAR_TITLE;
            }
        }
    }
    const NO_TAB: &str = "Properties";
    let Some(view) = active else { return NO_TAB };
    let mut title = Title(view, NO_TAB);
    for_each_kind(&mut title);
    title.1
}

/// Collects the topbar of every kind that has one, wrapped in its [`AreaOf`] container.
#[derive(Default)]
struct Topbars(Vec<Box<dyn Scene>>);

impl KindVisitor for Topbars {
    fn visit<E: EditorAreas>(&mut self) {
        let Some(bar) = E::topbar() else { return };
        let area = AreaOf(E::VIEW);
        self.0.push(Box::new(bsn! {
            area
            Node {
                display: Display::None,
                flex_grow: 1.0,
                min_width: px(0),
                min_height: px(30),
                align_items: AlignItems::Center,
            }
            Children [ @{bar} ]
        }));
    }
}

pub fn editor_topbar() -> impl Scene {
    let mut areas = Topbars::default();
    for_each_kind(&mut areas);
    let areas = areas.0;
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
        Children [ {areas} ]
    }
}

/// Collects the right sidebar body of every kind that has one, each in its own scroll column (a kind keeps its scroll
/// offset) tagged [`AreaOf`].
#[derive(Default)]
struct Sidebars(Vec<Box<dyn Scene>>);

impl KindVisitor for Sidebars {
    fn visit<E: EditorAreas>(&mut self) {
        let Some(body) = E::sidebar() else { return };
        let area = AreaOf(E::VIEW);
        self.0.push(Box::new(bsn! {
            @scroll_column()
            area
            Node { display: Display::None, padding: UiRect::all(px(0)) }
            Children [ @{body} ]
        }));
    }
}

pub fn right_sidebar() -> impl Scene {
    let mut areas = Sidebars::default();
    for_each_kind(&mut areas);
    let areas = areas.0;
    let title = sidebar_title(None);
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
                    Node { flex_grow: 1.0, min_height: px(0), flex_direction: FlexDirection::Column }
                    Children [
                        SidebarPlaceholder
                        Node { padding: UiRect::all(px(2)) }
                        Children [ @text("Nothing open.", TEXT_DIM, 13.0) ]
                        --
                        {areas}
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

fn display(node: &mut Node, shown: bool) {
    let want = if shown { Display::Flex } else { Display::None };
    if node.display != want {
        node.display = want;
    }
}

/// Shows the active kind's areas only; a strip only when that kind has something in it (the right sidebar also
/// while no tab is open, with its placeholder, and never while View → Show properties is off: the editor content
/// takes its width). The right sidebar header follows the active kind. Runs when the active tab or its kind or
/// [`UserConfig::show_properties`](crate::config::UserConfig::show_properties) changed, or areas were spawned.
pub fn show_active_areas(
    editor: Res<Editor>,
    mut last: Local<Option<(Option<usize>, Option<ViewKind>, bool)>>,
    spawned: Query<(), Added<AreaOf>>,
    (mut areas, parents): (Query<(Entity, &AreaOf, &mut Node)>, Query<&ChildOf>),
    mut strips: Query<
        (Entity, &mut Node, Has<RightSidebar>),
        (Or<(With<EditorTopbar>, With<RightSidebar>)>, Without<AreaOf>),
    >,
    mut placeholder: Query<
        &mut Node,
        (With<SidebarPlaceholder>, Without<AreaOf>, Without<EditorTopbar>, Without<RightSidebar>),
    >,
    mut title: Query<&mut Text, With<SidebarTitle>>,
) {
    let active: Option<ViewKind> = editor.tab().map(|t| t.doc().view());
    let properties = editor.config.show_properties;
    let key = (editor.active, active, properties);
    if *last == Some(key) && spawned.is_empty() {
        return;
    }
    *last = Some(key);
    let mut used = Vec::new();
    for (entity, area, mut node) in &mut areas {
        let shown = Some(area.0) == active;
        if shown {
            // The strip may hold the area deeper down (the right sidebar's pane body).
            used.extend(parents.iter_ancestors(entity));
        }
        display(&mut node, shown);
    }
    for (strip, mut node, sidebar) in &mut strips {
        let content = used.contains(&strip) || (sidebar && active.is_none());
        display(&mut node, content && (properties || !sidebar));
    }
    for mut node in &mut placeholder {
        display(&mut node, active.is_none());
    }
    let want = sidebar_title(active);
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
    mut strip: Query<(Entity, &mut Node), With<EditorStatusbar>>,
    mut last: Local<Option<StatusKey>>,
    mut commands: Commands,
) {
    let doc = editor.tab().map(|t| t.doc());
    let key = (editor.ui.log, editor.active, editor.ui.tabs, doc.map_or(0, |d| d.revision()));
    if last.as_ref() == Some(&key) {
        return;
    }
    let Ok((strip, mut node)) = strip.single_mut() else { return };
    *last = Some(key);
    commands.entity(strip).despawn_children();
    let Some(status) = editor.status_info() else {
        display(&mut node, false);
        return;
    };
    display(&mut node, true);
    let font =
        TextFont { font: FontSource::Handle(assets.load(fonts::MONO)), font_size: FontSize::Px(12.0), ..default() };
    spawn_status(&mut commands, strip, &status, editor.active_first_diagnostic().cloned(), font);
}

/// `● unsaved` (when dirty), `N errors · M warnings` (a click focuses `first`, like a console/logs row), the counts.
/// The text is in the full Fira Mono: Bevy's default font is an ASCII subset without `●` / `·`.
fn spawn_status(
    commands: &mut Commands,
    strip: Entity,
    status: &StatusInfo,
    first: Option<Diagnostic>,
    font: TextFont,
) {
    let text = |s: String| (Text::new(s), font.clone(), TextColor(TEXT_DIM), TextLayout::no_wrap(), Pickable::IGNORE);
    if status.dirty {
        commands.spawn((text("● unsaved".into()), ChildOf(strip)));
    }
    let problems = text(format!(
        "{} · {}",
        plural(status.errors, "error", "errors"),
        plural(status.warnings, "warning", "warnings")
    ));
    match first {
        Some(d) => {
            // A plain Feathers tool button as tall as the strip's text.
            let (act, tip) = (OnActivate(Action::FocusDiagnostic(d)), Tooltip("Show the first problem".into()));
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
    use super::sidebar_title;
    use crate::tab::ViewKind;

    #[test]
    fn sidebar_title_follows_the_active_kind() {
        assert_eq!(sidebar_title(None), "Properties");
        let titles = [
            (ViewKind::Graph, "Properties"),
            (ViewKind::Text, "Generated File"),
            (ViewKind::Curve, "Curve"),
            (ViewKind::Consts, "Constants"),
            (ViewKind::Image, "Image"),
            (ViewKind::Texture, "Texture"),
        ];
        for (view, title) in titles {
            assert_eq!(sidebar_title(Some(view)), title, "{view:?}");
        }
    }
}
