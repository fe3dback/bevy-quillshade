//! `D`'s curve editor kind (decision 0031): its window areas ([`EditorAreas`]) and its input context
//! ([`EditorInput`], decision 0032): the edit keys, routed to the active curve by `Editor`, and arrow nudging.

use std::marker::PhantomData;

use bevy::prelude::*;
use bevy_enhanced_input::prelude::{Actions as InputActions, InputAction, ModKeys, Start};
use editor_core::action::{Action, Actions};
use editor_core::editors::{EditorAreas, EditorInput, bar_row, sidebar_body};
use editor_core::keys::{mods, no_command_mods};
use editor_core::main_input::{
    EditCopy, EditCut, EditDelete, EditDuplicate, EditPaste, EditSelectAll, add_kind, ctrl, hotkey, plain,
};
use editor_core::state::editor::Editor;
use editor_core::tab::ViewKindId;

use super::sidebar::{CurveSidebarBody, redraw_sidebar};
use super::{CurveAction, CurveDomain, CurveEdit, CurveViewBar, active};

/// `D`'s curve editor kind.
pub struct CurveEditor<D>(PhantomData<fn() -> D>);

impl<D: CurveDomain> EditorAreas for CurveEditor<D> {
    const VIEW: ViewKindId = D::VIEW;
    const SIDEBAR_TITLE: &'static str = "Curve";

    /// The bar row, filled by the view's `redraw_bar`.
    fn topbar() -> Option<impl Scene> {
        let bar = CurveViewBar::<D>::default();
        Some(bsn! { @bar_row() bar })
    }

    /// The output range, the selected point and the hotkeys.
    fn sidebar() -> Option<impl Scene> {
        let body = CurveSidebarBody::<D>::default();
        Some(bsn! { @sidebar_body() body })
    }

    fn build(app: &mut App) {
        app.add_systems(Update, redraw_sidebar::<D>);
    }

    fn input(app: &mut App) {
        add_kind::<Self>(app);
        app.add_observer(nudge::<D, -1, 0>)
            .add_observer(nudge::<D, 1, 0>)
            .add_observer(nudge::<D, 0, 1>)
            .add_observer(nudge::<D, 0, -1>);
    }
}

/// `D`'s curve editor input context: one type per domain, so two curve domains' contexts never clash.
#[derive(Component)]
pub struct CurveCtx<D>(PhantomData<fn() -> D>);

impl<D> Default for CurveCtx<D> {
    fn default() -> Self {
        CurveCtx(PhantomData)
    }
}

impl<D> Clone for CurveCtx<D> {
    fn clone(&self) -> Self {
        CurveCtx::default()
    }
}

impl<D: CurveDomain> EditorInput for CurveEditor<D> {
    type Ctx = CurveCtx<D>;

    fn bindings() -> impl Scene {
        use KeyCode::{ArrowDown, ArrowLeft, ArrowRight, ArrowUp, Backspace, Delete, KeyA, KeyC, KeyD, KeyV, KeyX};
        bsn! {
            InputActions<CurveCtx<D>> [
                @{hotkey::<EditSelectAll>(ctrl(KeyA))}
                --
                @{hotkey::<EditDuplicate>(ctrl(KeyD))}
                --
                @{hotkey::<EditCopy>(ctrl(KeyC))}
                --
                @{hotkey::<EditCut>(ctrl(KeyX))}
                --
                @{hotkey::<EditPaste>(ctrl(KeyV))}
                --
                @{hotkey::<EditDelete>(plain([Delete, Backspace]))}
                --
                @{hotkey::<Nudge<D, -1, 0>>(plain([ArrowLeft]))}
                --
                @{hotkey::<Nudge<D, 1, 0>>(plain([ArrowRight]))}
                --
                @{hotkey::<Nudge<D, 0, 1>>(plain([ArrowUp]))}
                --
                @{hotkey::<Nudge<D, 0, -1>>(plain([ArrowDown]))}
            ]
        }
    }
}

/// Arrow-key step in curve units (Shift: ten times).
const NUDGE: f32 = 0.01;

/// An arrow key of `D`'s curve editor: moves the selected points by `(X, Y)` steps.
pub struct Nudge<D, const X: i8, const Y: i8>(PhantomData<fn() -> D>);

impl<D: CurveDomain, const X: i8, const Y: i8> InputAction for Nudge<D, X, Y> {
    type Output = bool;
}

/// Arrow keys move the selected points of `D`'s active curve (Shift: ten times further).
fn nudge<D: CurveDomain, const X: i8, const Y: i8>(
    _: On<Start<Nudge<D, X, Y>>>,
    keys: Res<ButtonInput<KeyCode>>,
    editor: Res<Editor>,
    mut actions: ResMut<Actions>,
) {
    if !no_command_mods(&keys) || active::<D>(&editor).is_none_or(|c| c.selection.is_empty()) {
        return;
    }
    let step = if mods(&keys).contains(ModKeys::SHIFT) { NUDGE * 10.0 } else { NUDGE };
    actions.push(Action::from(CurveEdit::<D>::new(CurveAction::Nudge((f32::from(X) * step, f32::from(Y) * step)))));
}
