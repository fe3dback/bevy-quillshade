//! Editor kinds (design §2.1, §6.2, decision 0031): what the window asks of a kind of tab content besides its
//! editor content view.
//!
//! The window areas a kind owns (design §1, §2.1) are described by [`EditorAreas`], implemented by one zero-sized type
//! per kind: the editor topbar and the right sidebar. Each area is spawned once per kind inside its strip, tagged
//! [`AreaOf`], and only the active kind's is displayed; the kind's own systems ([`EditorAreas::build`]) fill it.
//! [`KindDef::of`] erases a kind into data, and [`EditorKinds`] is the one list of kinds the window is built from.
//!
//! Hotkeys: a kind with any implements [`EditorInput`] — its bevy_enhanced_input context, active while a tab of the
//! kind is (decision 0032).

pub mod text;

use bevy::prelude::*;

use crate::state::editor::Editor;
use crate::tab::ViewKindId;
use crate::ui::widgets::scroll_column;

/// The window areas an editor kind owns, besides its editor content. Each one is a persistent scene spawned once at
/// startup inside its strip and filled by the kind's own redraw systems (keyed on what they show, never every
/// frame); `None` means the kind has no such area and the strip is hidden for it.
pub trait EditorAreas {
    /// The view of the tabs this kind edits ([`crate::doc_kind::TabDoc::view`]).
    const VIEW: ViewKindId;
    /// Header of the right sidebar while this kind is active.
    const SIDEBAR_TITLE: &'static str;
    /// The editor topbar row (under the secondbar).
    fn topbar() -> Option<impl Scene>;
    /// The right sidebar body (under the header showing [`Self::SIDEBAR_TITLE`]).
    fn sidebar() -> Option<impl Scene>;
    /// Registers the systems that fill this kind's areas.
    fn build(_app: &mut App) {}
    /// Registers the kind's input context and its handlers (for a kind with an [`EditorInput`]). Called by the
    /// input contexts plugin (the UI side), not by [`EditorAreas::build`].
    fn input(_app: &mut App) {}
}

/// The hotkeys of an editor kind (design §2.2): a bevy_enhanced_input context on the input root entity, active only
/// while the active tab is of this kind.
pub trait EditorInput: EditorAreas {
    /// The context component.
    type Ctx: Component + Default;
    /// `Actions<Self::Ctx> [ … ]`: the context's actions.
    fn bindings() -> impl Scene;
}

/// One editor kind as data: its [`EditorAreas`] (and, through `input`, its [`EditorInput`]) with the types erased.
#[derive(Clone, Copy)]
pub struct KindDef {
    pub view: ViewKindId,
    /// The kind's type: two registrations of one view must be the same kind.
    pub type_id: std::any::TypeId,
    pub sidebar_title: &'static str,
    /// Spawns the kind's editor topbar area into the strip entity; `None` when the kind has none.
    pub spawn_topbar: Option<fn(&mut Commands, Entity)>,
    /// Spawns the kind's right sidebar area into the sidebar's body entity; `None` when the kind has none.
    pub spawn_sidebar: Option<fn(&mut Commands, Entity)>,
    /// [`EditorAreas::build`].
    pub build: fn(&mut App),
    /// [`EditorAreas::input`].
    pub input: fn(&mut App),
}

impl KindDef {
    pub fn of<E: EditorAreas + 'static>() -> KindDef {
        KindDef {
            view: E::VIEW,
            type_id: std::any::TypeId::of::<E>(),
            sidebar_title: E::SIDEBAR_TITLE,
            spawn_topbar: E::topbar().is_some().then_some(spawn_topbar::<E> as fn(&mut Commands, Entity)),
            spawn_sidebar: E::sidebar().is_some().then_some(spawn_sidebar::<E> as fn(&mut Commands, Entity)),
            build: E::build,
            input: E::input,
        }
    }
}

/// The topbar of `E` in its [`AreaOf`] container, a child of `strip`.
fn spawn_topbar<E: EditorAreas>(commands: &mut Commands, strip: Entity) {
    let Some(bar) = E::topbar() else { return };
    let area = AreaOf(E::VIEW);
    commands.spawn_scene(bsn! {
        area
        Node {
            display: Display::None,
            flex_grow: 1.0,
            min_width: px(0),
            min_height: px(30),
            align_items: AlignItems::Center,
        }
        ChildOf(strip)
        Children [ @{bar} ]
    });
}

/// The right sidebar body of `E` in its own scroll column (a kind keeps its scroll offset) tagged [`AreaOf`], a child
/// of `body`.
fn spawn_sidebar<E: EditorAreas>(commands: &mut Commands, body: Entity) {
    let Some(rows) = E::sidebar() else { return };
    let area = AreaOf(E::VIEW);
    commands.spawn_scene(bsn! {
        @scroll_column()
        area
        Node { display: Display::None, padding: UiRect::all(px(0)) }
        ChildOf(body)
        Children [ @{rows} ]
    });
}

/// Every editor kind, in the order their areas are spawned.
#[derive(Resource, Clone, Default)]
pub struct EditorKinds(pub Vec<KindDef>);

impl EditorKinds {
    /// Header of the right sidebar while no tab is open.
    pub const NO_TAB_TITLE: &'static str = "Properties";

    pub fn get(&self, view: ViewKindId) -> Option<&KindDef> {
        self.0.iter().find(|k| k.view == view)
    }

    /// Header of the right sidebar: the active kind's title, [`EditorKinds::NO_TAB_TITLE`] while no tab is open.
    pub fn sidebar_title(&self, active: Option<ViewKindId>) -> &'static str {
        active.and_then(|v| self.get(v)).map_or(Self::NO_TAB_TITLE, |k| k.sidebar_title)
    }
}

/// The row a kind's editor topbar is built on: a marker plus this, filled by the kind's redraw system. Items that
/// do not fit wrap onto another line (buttons never shrink).
pub fn bar_row() -> impl Scene {
    bsn! {
        Node {
            flex_grow: 1.0,
            min_width: px(0),
            flex_direction: FlexDirection::Row,
            flex_wrap: FlexWrap::Wrap,
            align_items: AlignItems::Center,
            column_gap: px(8),
            padding: UiRect::axes(px(10), px(2)),
        }
    }
}

/// The column a kind's right sidebar body is built on: a marker plus this, filled by the kind's redraw system.
pub fn sidebar_body() -> impl Scene {
    bsn! {
        Node { flex_direction: FlexDirection::Column, row_gap: px(6) }
    }
}

/// A per-kind area container: `Display::Flex` while the active tab is of this kind, `Display::None` otherwise (hidden
/// flow children must not take space), and `Visibility::Hidden` too (Bevy's tab navigation skips only hidden
/// entities).
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct AreaOf(pub ViewKindId);

impl Default for AreaOf {
    fn default() -> Self {
        AreaOf(ViewKindId(""))
    }
}

/// The editor content view of a kind (in the document area): visible while the active tab is of this kind.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewOf(pub ViewKindId);

impl Default for ViewOf {
    fn default() -> Self {
        ViewOf(ViewKindId(""))
    }
}

/// When a kind's right sidebar body is rebuilt: on a change of `(Editor::ui.props, Editor::active)` while the kind is
/// active (the key the Properties panel always had). It is forgotten while another kind is active, so the body is
/// rebuilt when its kind comes back.
#[derive(Default)]
pub struct SidebarKey(Option<(u64, Option<usize>)>);

impl SidebarKey {
    /// The key's current value: `(Editor::ui.props, Editor::active)`.
    pub fn now(editor: &Editor) -> (u64, Option<usize>) {
        (editor.ui.props, editor.active)
    }

    /// The kind is not active.
    pub fn forget(&mut self) {
        self.0 = None;
    }

    /// Whether the body must be rebuilt for `now` ([`SidebarKey::now`]): true when it differs from the stored key,
    /// which then becomes `now`.
    pub fn refresh(&mut self, now: (u64, Option<usize>)) -> bool {
        let changed = self.0 != Some(now);
        self.0 = Some(now);
        changed
    }
}

/// A kind's right sidebar redraw, called by its system once its guard found a tab of the kind active: when `now`
/// ([`SidebarKey::now`]) differs from `key`, replaces the rows of the body tagged `B` with `rows()` (built only then).
pub fn fill_sidebar_with<B: Component>(
    commands: &mut Commands,
    body: &Query<Entity, With<B>>,
    key: &mut SidebarKey,
    now: (u64, Option<usize>),
    rows: impl FnOnce() -> Vec<Box<dyn Scene>>,
) {
    let Ok(body) = body.single() else { return };
    if !key.refresh(now) {
        return;
    }
    commands.entity(body).despawn_children();
    for r in rows() {
        commands.spawn_scene(bsn! { @{r} ChildOf(body) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A kind with a right sidebar only.
    struct SidebarOnly;

    #[derive(Component, Default, Clone)]
    struct Body;

    impl EditorAreas for SidebarOnly {
        const VIEW: ViewKindId = ViewKindId("test");
        const SIDEBAR_TITLE: &'static str = "Test";

        fn topbar() -> Option<impl Scene> {
            None::<Box<dyn Scene>>
        }

        fn sidebar() -> Option<impl Scene> {
            Some(bsn! { @sidebar_body() Body })
        }
    }

    #[test]
    fn kind_def_without_topbar() {
        let def = KindDef::of::<SidebarOnly>();
        assert_eq!(def.view, ViewKindId("test"));
        assert_eq!(def.sidebar_title, "Test");
        assert!(def.spawn_topbar.is_none());
        assert!(def.spawn_sidebar.is_some());
        let kinds = EditorKinds(vec![def]);
        assert_eq!(kinds.sidebar_title(Some(ViewKindId("test"))), "Test");
        assert_eq!(kinds.sidebar_title(Some(ViewKindId("other"))), EditorKinds::NO_TAB_TITLE);
        assert_eq!(kinds.sidebar_title(None), EditorKinds::NO_TAB_TITLE);
    }

    /// `spawn_sidebar` puts the kind's body, tagged with its view, under the given entity.
    #[test]
    fn spawned_sidebar_area_is_tagged() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default(), bevy::scene::ScenePlugin));
        let parent = app.world_mut().spawn_empty().id();
        let spawn = KindDef::of::<SidebarOnly>().spawn_sidebar.expect("sidebar");
        let mut queue = bevy::ecs::world::CommandQueue::default();
        spawn(&mut Commands::new(&mut queue, app.world()), parent);
        queue.apply(app.world_mut());
        app.update();
        let w = app.world_mut();
        let areas: Vec<_> = w.query::<(&AreaOf, &ChildOf)>().iter(w).map(|(a, c)| (*a, c.parent())).collect();
        assert_eq!(areas, [(AreaOf(ViewKindId("test")), parent)]);
        assert_eq!(w.query_filtered::<(), With<Body>>().iter(w).count(), 1);
    }

    #[test]
    fn sidebar_key_rebuilds_on_props_or_tab_change() {
        let mut editor = Editor { active: Some(0), ..Default::default() };
        let mut key = SidebarKey::default();
        let rebuilds = |editor: &Editor, key: &mut SidebarKey| key.refresh(SidebarKey::now(editor));
        assert!(rebuilds(&editor, &mut key), "the first fill");
        assert!(!rebuilds(&editor, &mut key), "nothing changed");
        editor.ui.props += 1;
        assert!(rebuilds(&editor, &mut key), "properties revision");
        assert!(!rebuilds(&editor, &mut key));
        editor.active = Some(1);
        assert!(rebuilds(&editor, &mut key), "another tab");
        assert!(!rebuilds(&editor, &mut key));
        // Other revisions are not the sidebar's business.
        editor.ui.tabs += 1;
        editor.ui.log += 1;
        editor.ui.browser += 1;
        assert!(!rebuilds(&editor, &mut key));
        // Another kind was active meanwhile: the body is rebuilt when the kind comes back.
        key.forget();
        assert!(rebuilds(&editor, &mut key), "after forget");
    }
}
