//! Curve editor kind: a curve asset's points (decision 0015, [`CurveDoc`]).

use bevy::prelude::*;
use bevy_enhanced_input::prelude::{Actions as InputActions, InputAction, ModKeys, Start};
use shader_graph_core::curve::Curve;
use shader_project::{AssetId, AssetSet, Project, assets, ops};

use super::main_input::{
    EditCopy, EditCut, EditDelete, EditDuplicate, EditPaste, EditSelectAll, add_kind, ctrl, hotkey, plain,
};
use super::{EditorAreas, EditorInput, SidebarKey, StatusCtx, StatusInfo, fill_sidebar_with};
use crate::curve_doc::CurveDoc;
use crate::doc_kind::{TabDoc, asset_title, io_error};
use crate::keys::{mods, no_command_mods};
use crate::state::{Action, Actions, CurveAction, Editor, PropTarget};
use crate::tab::ViewKind;
use crate::ui::curve_view::CurveViewBar;
use crate::ui::icons::IconId;
use crate::ui::props::{hotkey_row, labeled, number};
use crate::ui::widgets::subpane_box;
use crate::ui::{TEXT, TEXT_DIM, plural, text};

/// The curve editor's window areas.
pub struct CurveEditor;

impl EditorAreas for CurveEditor {
    const VIEW: ViewKind = ViewKind::Curve;
    const SIDEBAR_TITLE: &'static str = "Curve";

    /// The bar row, filled by `ui::curve_view::redraw_bar`.
    fn topbar() -> Option<impl Scene> {
        Some(bsn! { @super::bar_row() CurveViewBar })
    }

    /// The selected point and the hotkeys.
    fn sidebar() -> Option<impl Scene> {
        Some(bsn! { @super::sidebar_body() CurveSidebarBody })
    }

    fn build(app: &mut App) {
        app.add_systems(Update, redraw_sidebar);
    }

    fn input(app: &mut App) {
        add_kind::<Self>(app);
        app.add_observer(nudge::<-1, 0>)
            .add_observer(nudge::<1, 0>)
            .add_observer(nudge::<0, 1>)
            .add_observer(nudge::<0, -1>);
    }
}

/// The curve editor's input context: the edit keys (routed to the curve by `Editor`) and arrow nudging.
#[derive(Component, Default, Clone)]
pub struct CurveCtx;

impl EditorInput for CurveEditor {
    type Ctx = CurveCtx;

    fn bindings() -> impl Scene {
        use KeyCode::{ArrowDown, ArrowLeft, ArrowRight, ArrowUp, Backspace, Delete, KeyA, KeyC, KeyD, KeyV, KeyX};
        bsn! {
            InputActions<CurveCtx> [
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
                @{hotkey::<Nudge<-1, 0>>(plain([ArrowLeft]))}
                --
                @{hotkey::<Nudge<1, 0>>(plain([ArrowRight]))}
                --
                @{hotkey::<Nudge<0, 1>>(plain([ArrowUp]))}
                --
                @{hotkey::<Nudge<0, -1>>(plain([ArrowDown]))}
            ]
        }
    }
}

/// Arrow-key step in curve units (Shift: ten times).
const NUDGE: f32 = 0.01;

/// An arrow key: moves the selected points by `(X, Y)` steps.
pub struct Nudge<const X: i8, const Y: i8>;

impl<const X: i8, const Y: i8> InputAction for Nudge<X, Y> {
    type Output = bool;
}

/// Arrow keys move the selected points (Shift: ten times further).
fn nudge<const X: i8, const Y: i8>(
    _: On<Start<Nudge<X, Y>>>,
    keys: Res<ButtonInput<KeyCode>>,
    editor: Res<Editor>,
    mut actions: ResMut<Actions>,
) {
    if !no_command_mods(&keys) || editor.curve_doc().is_none_or(|c| c.selection.is_empty()) {
        return;
    }
    let step = if mods(&keys).contains(ModKeys::SHIFT) { NUDGE * 10.0 } else { NUDGE };
    actions.push(Action::Curve(CurveAction::Nudge((f32::from(X) * step, f32::from(Y) * step))));
}

/// The curve editor's right sidebar body.
#[derive(Component, Default, Clone)]
struct CurveSidebarBody;

fn redraw_sidebar(
    editor: Res<Editor>,
    body: Query<Entity, With<CurveSidebarBody>>,
    mut key: Local<SidebarKey>,
    mut commands: Commands,
) {
    let Some(c) = editor.curve_doc() else { return key.forget() };
    fill_sidebar_with(&mut commands, &body, &mut key, SidebarKey::now(&editor), || curve_rows(c));
}

/// Hotkeys of the curve view, shown in its right sidebar.
const CURVE_HOTKEYS: [(&str, &str); 12] = [
    ("Double-click", "Add a point"),
    ("Drag", "Move selection"),
    ("X (held)", "Only along T"),
    ("Y (held)", "Only along value"),
    ("Shift (held)", "Snap to 0.1 grid"),
    ("Shift+click", "Toggle selection"),
    ("Arrows", "Nudge (Shift: x10)"),
    ("Del", "Delete selection"),
    ("Ctrl+A", "Select all"),
    ("Ctrl+C / V", "Copy / paste"),
    ("Ctrl+D", "Duplicate"),
    ("Right-click", "Menu"),
];

/// A curve tab: the curve, the selected point and the hotkeys (select all / delete, preview mode and generated WESL
/// are in the editor topbar).
fn curve_rows(c: &CurveDoc) -> Vec<Box<dyn Scene>> {
    let curve: Vec<Box<dyn Scene>> = vec![
        Box::new(text(c.id.source_rel(), TEXT, 15.0)),
        Box::new(text(format!("Type: Curve, {}", plural(c.curve.points.len(), "point", "points")), TEXT_DIM, 12.0)),
    ];
    let mut out = vec![subpane_box("Curve", curve)];
    match c.selection.len() {
        0 => {}
        1 => {
            let i = *c.selection.first().expect("one");
            if let Some((t, v)) = c.curve.points.get(i) {
                let target = PropTarget::CurvePoint(i);
                let rows: Vec<Box<dyn Scene>> = vec![
                    Box::new(labeled("T".into(), number(target.clone(), 0, *t, None))),
                    Box::new(labeled("Value".into(), number(target, 1, *v, None))),
                ];
                out.push(subpane_box(&format!("Point {}", i + 1), rows));
            }
        }
        n => {
            let selected: Box<dyn Scene> = Box::new(text(format!("{n} points selected"), TEXT, 13.0));
            out.push(subpane_box("Selection", vec![selected]));
        }
    }
    let hotkeys = CURVE_HOTKEYS.map(|(keys, what)| -> Box<dyn Scene> { Box::new(hotkey_row(keys, what)) });
    out.push(subpane_box("Hotkeys", hotkeys.into()));
    out
}

impl TabDoc for CurveDoc {
    fn asset(&self) -> Option<&AssetId> {
        Some(&self.id)
    }

    fn view(&self) -> ViewKind {
        ViewKind::Curve
    }

    fn base_title(&self) -> String {
        asset_title(&self.id)
    }

    fn icon(&self) -> IconId {
        IconId::Curve
    }

    fn watermark(&self) -> Option<&'static str> {
        Some("curve")
    }

    fn is_dirty(&self) -> bool {
        CurveDoc::is_dirty(self)
    }

    fn is_read_only(&self) -> bool {
        false
    }

    fn revision(&self) -> u64 {
        self.revision
    }

    fn save(&mut self, project: &Project) -> std::io::Result<()> {
        ops::save_curve(project, &self.id, &self.curve).map_err(io_error)?;
        self.mark_saved();
        Ok(())
    }

    fn changed_on_disk(&self, project: &Project) -> bool {
        assets::load_curve(project, &self.id).is_ok_and(|c| Curve::from_ron(&self.saved).ok() != Some(c))
    }

    fn reload_from_disk(&mut self, project: &Project) -> Result<(), String> {
        self.reload(assets::load_curve(project, &self.id)?);
        Ok(())
    }

    fn keep_mine(&mut self, project: &Project) {
        if let Ok(on_disk) = assets::load_curve(project, &self.id) {
            self.saved = on_disk.to_ron();
        }
    }

    fn rename(&mut self, _project: &Project, from: &AssetId, to: &AssetId) {
        if self.id != *from {
            return;
        }
        self.id = to.clone();
        self.curve.name = to.name().to_string();
        self.mark_saved();
        self.revision += 1;
    }

    fn config_key(&self) -> Option<String> {
        Some(self.id.source_rel())
    }

    fn override_into(&self, set: &mut AssetSet) {
        set.curves.insert(self.id.clone(), Ok(self.curve.clone()));
    }

    fn status(&self, ctx: &StatusCtx) -> Option<StatusInfo> {
        let counts = vec![plural(self.curve.points.len(), "point", "points")];
        Some(StatusInfo::new(CurveDoc::is_dirty(self), ctx, counts))
    }
}
