//! The graph editor kind of a [`GraphDomain`] (decision 0031): its editor topbar tools (auto-layout, comment, fit),
//! the right sidebar's comment frame and multi-selection subpanes, and the base input context [`GraphCtx`]
//! (UE-like, spec §9, decision 0032). The domain adds its own topbar parts ([`GraphDomain::topbar_extras`]), the
//! sidebar of one node or of the asset ([`GraphDomain::sidebar`]) and extra bindings ([`GraphDomain::bindings`]).

use std::marker::PhantomData;

use bevy::prelude::*;
use bevy_enhanced_input::prelude::{Actions as InputActions, Fire, InputAction, Start};
use bevy_noodle::prelude::{CanvasView, GraphQuery};
use editor_core::action::{Action, Actions, TabAction, ViewRequest};
use editor_core::dynval::PropValue;
use editor_core::editors::{EditorAreas, EditorInput, SidebarKey, bar_row, fill_sidebar_with, sidebar_body};
use editor_core::keys::no_command_mods;
use editor_core::main_input::{
    EditCopy, EditCut, EditDelete, EditDuplicate, EditSelectAll, add_kind, ctrl, held_hotkey, hold_keyboard_while,
    hotkey, plain, shift,
};
use editor_core::state::editor::Editor;
use editor_core::tab::ViewKindId;
use editor_core::theme::{TEXT, TEXT_DIM, text};
use editor_core::ui::icons::IconId;
use editor_core::ui::props::{hue_input, labeled, text_w};
use editor_core::ui::widgets::subpane_box;
use editor_core::ui::{button, icon_tool_button};
use editor_core::util::text::plural;

use crate::canvas::{EditorCanvas, frame_color, view_center};
use crate::domain::{GraphCtxRef, GraphDomain, active};
use crate::edit::GraphEdit;
use crate::input::CanvasCursor;
use crate::palette::Palette;
use crate::view::{FrameField, GraphHotkey};

/// The graph editor's window areas for `G`.
pub struct GraphEditor<G>(PhantomData<fn() -> G>);

impl<G: GraphDomain> EditorAreas for GraphEditor<G> {
    const VIEW: ViewKindId = G::VIEW;
    const SIDEBAR_TITLE: &'static str = "Properties";

    /// The canvas tools, then the domain's [`GraphDomain::topbar_extras`] (design §5.2, §5.4).
    fn topbar() -> Option<impl Scene> {
        let extras = G::topbar_extras();
        Some(bsn! {
            @bar_row()
            Node { column_gap: px(2), padding: UiRect::axes(px(6), px(2)) }
            Children [
                @icon_tool_button(IconId::AutoLayout, "Auto-layout", GraphEdit::AutoLayout, "Auto-layout (Ctrl+L)")
                --
                @icon_tool_button(IconId::Comment, "Comment", GraphEdit::AddComment(None), COMMENT_TIP)
                --
                @icon_tool_button(IconId::Fit, "Fit", GraphEdit::View(ViewRequest::FitAll), "Fit the graph (Home)")
                --
                {extras}
            ]
        })
    }

    /// Properties of the selection (a node, a comment frame, several nodes) or of the asset.
    fn sidebar() -> Option<impl Scene> {
        let body = GraphSidebarBody::<G>::default();
        Some(bsn! { @sidebar_body() body })
    }

    fn build(app: &mut App) {
        app.add_systems(Update, (comment_at_view_centre::<G>.before(editor_core::apply_actions), redraw_sidebar::<G>));
        G::build(app);
    }

    fn input(app: &mut App) {
        add_kind::<Self>(app);
        // The open node palette takes the keyboard (its search field and list keys).
        hold_keyboard_while::<Palette<G>>(app, |p| p.open);
        // Shared by every graph domain: registered once.
        if !app.world().contains_resource::<CommentKey>() {
            app.init_resource::<CommentKey>().add_observer(comment_press);
        }
        app.add_observer(push_combo::<G, AutoLayout>(GraphEdit::AutoLayout))
            .add_observer(push_plain::<G, FitSelection>(GraphEdit::View(ViewRequest::FitSelection)))
            .add_observer(push_plain::<G, FitAll>(GraphEdit::View(ViewRequest::FitAll)))
            .add_observer(paste_at_cursor::<G>)
            .add_observer(comment_at_cursor::<G>)
            .add_observer(open_palette::<G>)
            .add_observer(node_hotkey::<G, ConvertPlus>)
            .add_observer(node_hotkey::<G, ConvertMinus>)
            .add_observer(node_hotkey::<G, ConvertStar>)
            .add_observer(node_hotkey::<G, ConvertSlash>)
            .add_observer(cancel_wire::<G>);
        G::input(app);
    }
}

/// The graph editor's input context of `G` (UE-like, spec §9). Held-key + click node placement and mouse modifiers
/// stay with the canvas gestures ([`crate::input`]).
#[derive(Component)]
pub struct GraphCtx<G>(PhantomData<fn() -> G>);

impl<G> Default for GraphCtx<G> {
    fn default() -> Self {
        GraphCtx(PhantomData)
    }
}

impl<G> Clone for GraphCtx<G> {
    fn clone(&self) -> Self {
        GraphCtx::default()
    }
}

impl<G: GraphDomain> EditorInput for GraphEditor<G> {
    type Ctx = GraphCtx<G>;

    fn bindings() -> impl Scene {
        use KeyCode::{
            Backspace, Delete, Digit8, Equal, Escape, Home, KeyA, KeyC, KeyD, KeyF, KeyL, KeyV, KeyX, Minus, NumpadAdd,
            NumpadDivide, NumpadMultiply, NumpadSubtract, Slash, Space,
        };
        let extras = G::bindings();
        bsn! {
            InputActions<GraphCtx<G>> [
                @{hotkey::<EditCopy>(ctrl(KeyC))}
                --
                @{hotkey::<EditCut>(ctrl(KeyX))}
                --
                @{hotkey::<GraphPaste>(ctrl(KeyV))}
                --
                @{hotkey::<EditDuplicate>(ctrl(KeyD))}
                --
                @{hotkey::<EditSelectAll>(ctrl(KeyA))}
                --
                @{hotkey::<AutoLayout>(ctrl(KeyL))}
                --
                @{hotkey::<EditDelete>(plain([Delete, Backspace]))}
                --
                @{hotkey::<FitSelection>(plain([KeyF]))}
                --
                @{hotkey::<FitAll>(plain([Home]))}
                --
                @{held_hotkey::<AddComment>(plain([KeyC]), COMMENT_HOLD_SECS)}
                --
                @{hotkey::<OpenPalette>(plain([Space]))}
                --
                // Node action hotkeys (decision 0021): Shift + the main-row key, or the numpad key alone.
                @{hotkey::<ConvertPlus>([shift(Equal), NumpadAdd.into()])}
                --
                @{hotkey::<ConvertMinus>([shift(Minus), NumpadSubtract.into()])}
                --
                @{hotkey::<ConvertStar>([shift(Digit8), NumpadMultiply.into()])}
                --
                @{hotkey::<ConvertSlash>([shift(Slash), NumpadDivide.into()])}
                --
                {extras}
                --
                @{hotkey::<CancelWire>(plain([Escape]))}
            ]
        }
    }
}

/// Ctrl+V: paste at the cursor.
#[derive(InputAction)]
#[action_output(bool)]
pub struct GraphPaste;

#[derive(InputAction)]
#[action_output(bool)]
pub struct AutoLayout;

#[derive(InputAction)]
#[action_output(bool)]
pub struct FitSelection;

#[derive(InputAction)]
#[action_output(bool)]
pub struct FitAll;

/// Esc: drops the wire being dragged, or puts the wires a Ctrl+drag lifted back (design §5.3); nothing is edited.
#[derive(InputAction)]
#[action_output(bool)]
pub struct CancelWire;

/// Hold C: a comment frame at the cursor (around the selection), design §6.2.
#[derive(InputAction)]
#[action_output(bool)]
pub struct AddComment;

/// How long C is held before [`AddComment`] fires (design §6.2).
pub const COMMENT_HOLD_SECS: f32 = 0.5;

/// The Comment button's tooltip (design §6.2).
const COMMENT_TIP: &str = "Add a comment frame (hold C)";

/// Space: the node palette at the cursor (design §4).
#[derive(InputAction)]
#[action_output(bool)]
pub struct OpenPalette;

/// A node action hotkey (decision 0021): runs the selected nodes' actions bound to it
/// ([`GraphDomain::hotkey_actions`]).
pub trait NodeHotkey: InputAction {
    const KEY: GraphHotkey;
}

macro_rules! node_hotkeys {
    ($($name:ident => $key:ident),* $(,)?) => {$(
        #[derive(InputAction)]
        #[action_output(bool)]
        pub struct $name;

        impl NodeHotkey for $name {
            const KEY: GraphHotkey = GraphHotkey::$key;
        }
    )*};
}

node_hotkeys!(ConvertPlus => Plus, ConvertMinus => Minus, ConvertStar => Star, ConvertSlash => Slash);

/// Pushes `edit` when `A` (bound to Ctrl / Super combinations) starts on a `G` graph.
fn push_combo<G: GraphDomain, A: InputAction>(
    edit: GraphEdit,
) -> impl FnMut(On<Start<A>>, Res<Editor>, ResMut<Actions>) {
    move |_, editor, mut actions| {
        if active::<G>(&editor).is_some() {
            actions.push(edit.clone());
        }
    }
}

/// Pushes `edit` when `A` (bound to plain keys) starts on a `G` graph, unless Ctrl, Alt or Super is held.
fn push_plain<G: GraphDomain, A: InputAction>(
    edit: GraphEdit,
) -> impl FnMut(On<Start<A>>, Res<ButtonInput<KeyCode>>, Res<Editor>, ResMut<Actions>) {
    move |_, keys, editor, mut actions| {
        if no_command_mods(&keys) && active::<G>(&editor).is_some() {
            actions.push(edit.clone());
        }
    }
}

fn paste_at_cursor<G: GraphDomain>(
    _: On<Start<GraphPaste>>,
    cursor: CanvasCursor<G>,
    editor: Res<Editor>,
    mut actions: ResMut<Actions>,
) {
    if active::<G>(&editor).is_some() {
        actions.push(Action::Tab(TabAction::Paste(cursor.graph_pos().map(Vec2::from))));
    }
}

/// Hold C (design §6.2): only C pressed with no Ctrl, Alt or Super arms the hold. A C still down when Ctrl is let go
/// (a Ctrl+C: the copy consumed the C until then) starts the action too, but does not arm it.
#[derive(Resource, Default)]
struct CommentKey {
    armed: bool,
}

fn comment_press(_: On<Start<AddComment>>, keys: Res<ButtonInput<KeyCode>>, mut key: ResMut<CommentKey>) {
    key.armed = keys.just_pressed(KeyCode::KeyC) && no_command_mods(&keys);
}

/// C held long enough: a comment at the cursor; with the cursor off the canvas `None`, the visible canvas centre
/// like the Comment button (controller ruling over design §6.1's origin).
fn comment_at_cursor<G: GraphDomain>(
    _: On<Fire<AddComment>>,
    keys: Res<ButtonInput<KeyCode>>,
    cursor: CanvasCursor<G>,
    editor: Res<Editor>,
    mut key: ResMut<CommentKey>,
    mut actions: ResMut<Actions>,
) {
    if active::<G>(&editor).is_none() {
        return;
    }
    if std::mem::take(&mut key.armed) && no_command_mods(&keys) {
        actions.push(GraphEdit::AddComment(cursor.graph_pos()));
    }
}

/// The editor topbar's Comment button and hold C off the canvas push `AddComment(None)`: with no selection the frame
/// goes at the visible canvas centre (design §6.1), filled in here before the action is applied.
fn comment_at_view_centre<G: GraphDomain>(
    mut actions: ResMut<Actions>,
    editor: Res<Editor>,
    ec: Option<Res<EditorCanvas<G>>>,
    views: Query<(&CanvasView, &ComputedNode)>,
) {
    let at_centre: Action = GraphEdit::AddComment(None).into();
    if !actions.0.contains(&at_centre) || active::<G>(&editor).is_none() {
        return;
    }
    let Some(centre) = ec.and_then(|ec| views.get(ec.canvas).ok()).map(|(v, c)| view_center(v, c)) else { return };
    for a in actions.0.iter_mut().filter(|a| **a == at_centre) {
        *a = GraphEdit::AddComment(Some((centre.x, centre.y))).into();
    }
}

/// Esc ends the canvas's wire gesture the way bevy_noodle's own cancel does: by despawning its dragged wire or
/// lifted edges (`MovingEdges`). Any modifier may be held (a Ctrl+drag holds Ctrl). While typing, the keyboard gate
/// hides Esc from this context, so a text field, the palette or a dialog keeps it.
fn cancel_wire<G: GraphDomain>(
    _: On<Start<CancelWire>>,
    ec: Option<Res<EditorCanvas<G>>>,
    graph: GraphQuery,
    mut commands: Commands,
) {
    if let Some(wire) = ec.and_then(|ec| graph.wire_of(ec.canvas)) {
        commands.entity(wire).try_despawn();
    }
}

fn open_palette<G: GraphDomain>(
    _: On<Start<OpenPalette>>,
    keys: Res<ButtonInput<KeyCode>>,
    cursor: CanvasCursor<G>,
    editor: Res<Editor>,
    palette: Option<ResMut<Palette<G>>>,
) {
    if active::<G>(&editor).is_none() {
        return;
    }
    if let (true, Some((screen, graph)), Some(mut palette)) = (no_command_mods(&keys), cursor.at(), palette) {
        palette.show(screen, graph, None);
    }
}

fn node_hotkey<G: GraphDomain, A: NodeHotkey>(
    _: On<Start<A>>,
    keys: Res<ButtonInput<KeyCode>>,
    editor: Res<Editor>,
    mut actions: ResMut<Actions>,
) {
    let Some(d) = active::<G>(&editor) else { return };
    if no_command_mods(&keys) {
        for a in G::hotkey_actions(&GraphCtxRef { editor: &editor }, d, A::KEY) {
            actions.push(a);
        }
    }
}

/// The graph editor's right sidebar body ("Properties").
#[derive(Component)]
struct GraphSidebarBody<G>(PhantomData<fn() -> G>);

impl<G> Default for GraphSidebarBody<G> {
    fn default() -> Self {
        GraphSidebarBody(PhantomData)
    }
}

impl<G> Clone for GraphSidebarBody<G> {
    fn clone(&self) -> Self {
        GraphSidebarBody::default()
    }
}

fn redraw_sidebar<G: GraphDomain>(
    editor: Res<Editor>,
    body: Query<Entity, With<GraphSidebarBody<G>>>,
    mut key: Local<SidebarKey>,
    mut commands: Commands,
) {
    let Some(d) = active::<G>(&editor) else { return key.forget() };
    fill_sidebar_with(&mut commands, &body, &mut key, SidebarKey::now(&editor), || sidebar_rows::<G>(&editor, d));
}

/// The selected comment frame, several nodes, or (one node or none, [`GraphDomain::sidebar`]) the domain's: one
/// subpane per kind of content.
fn sidebar_rows<G: GraphDomain>(editor: &Editor, d: &G::Doc) -> Vec<Box<dyn Scene>> {
    let view = G::view(d);
    if let Some(f) = view.selected_frame.and_then(|f| view.frames.iter().find(|c| c.id == f)) {
        let mut rows: Vec<Box<dyn Scene>> = Vec::new();
        let (title, hue_key) = (G::frame_prop(&f.id, FrameField::Title), G::frame_prop(&f.id, FrameField::Hue));
        rows.push(Box::new(labeled("Title".into(), text_w(title, f.title.clone(), false))));
        // Unset: the field starts at the default colour's hue (design §4.6).
        let hue = f.hue.unwrap_or_else(|| Hsla::from(frame_color(f)).hue);
        rows.push(Box::new(labeled("Hue".into(), hue_input(hue_key.clone(), hue))));
        let reset =
            Action::Tab(TabAction::SetValue { key: hue_key, value: PropValue::Str(String::new()), merge: false });
        rows.push(Box::new(labeled(String::new(), Box::new(button("Default colour", reset, false)))));
        vec![subpane_box("Comment", rows)]
    } else if view.selection.len() > 1 {
        let selected = format!("{} selected", plural(view.selection.len(), "node", "nodes"));
        let rows: Vec<Box<dyn Scene>> = vec![
            Box::new(text(selected, TEXT, 14.0)),
            Box::new(text("Hold C: comment around them | Auto-layout arranges them.", TEXT_DIM, 12.0)),
        ];
        vec![subpane_box("Selection", rows)]
    } else {
        G::sidebar(&GraphCtxRef { editor }, d)
    }
}
