//! The curve editor's right sidebar: the curve with its output range, the selected point, the domain's extras
//! ([`CurveDomain::sidebar_extras`]) and the hotkeys.

use bevy::prelude::*;
use editor_core::editors::{SidebarKey, fill_sidebar_with};
use editor_core::state::editor::Editor;
use editor_core::theme::{TEXT, TEXT_DIM, text};
use editor_core::ui::props::{hotkey_row, labeled, number, number_ranged};
use editor_core::ui::widgets::subpane_box;

use super::{CurveDoc, CurveDomain, CurveProp, active, curve_key};

marker! {
    /// The right sidebar body of `D`'s curve editor.
    CurveSidebarBody
}

pub(super) fn redraw_sidebar<D: CurveDomain>(
    editor: Res<Editor>,
    body: Query<Entity, With<CurveSidebarBody<D>>>,
    mut key: Local<SidebarKey>,
    mut commands: Commands,
) {
    let Some(c) = active::<D>(&editor) else { return key.forget() };
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

/// Drag range of a Min / Max field: `-10..10`, widened to hold a typed value outside it (a drag never jumps).
fn range_limit(v: f32) -> (f32, f32) {
    (v.min(-10.0), v.max(10.0))
}

/// A curve tab: the curve with its output range, the selected point, the domain's extras and the hotkeys (select
/// all / delete and the preview mode are in the editor topbar).
fn curve_rows<D: CurveDomain>(c: &CurveDoc<D>) -> Vec<Box<dyn Scene>> {
    // The point count is in the editor statusbar.
    let (min, max) = (c.curve.min, c.curve.max);
    let curve: Vec<Box<dyn Scene>> = vec![
        Box::new(text(c.aref.source_rel(), TEXT, 15.0)),
        Box::new(text("Type: Curve", TEXT_DIM, 12.0)),
        Box::new(labeled(
            "Min".into(),
            number_ranged(curve_key::<D>(CurveProp::Range), 0, min, Some(range_limit(min)), None),
        )),
        Box::new(labeled(
            "Max".into(),
            number_ranged(curve_key::<D>(CurveProp::Range), 1, max, Some(range_limit(max)), None),
        )),
    ];
    let mut out = vec![subpane_box("Curve", curve)];
    match c.selection.len() {
        0 => {}
        1 => {
            let i = *c.selection.first().expect("one");
            if let Some((t, v)) = c.curve.points.get(i) {
                let target = curve_key::<D>(CurveProp::Point(i));
                let rows: Vec<Box<dyn Scene>> = vec![
                    Box::new(labeled("T".into(), number(target.clone(), 0, *t, None))),
                    Box::new(labeled("Value".into(), number(target, 1, c.curve.to_real(*v), None))),
                ];
                out.push(subpane_box(&format!("Point {}", i + 1), rows));
            }
        }
        n => {
            let selected: Box<dyn Scene> = Box::new(text(format!("{n} points selected"), TEXT, 13.0));
            out.push(subpane_box("Selection", vec![selected]));
        }
    }
    out.extend(D::sidebar_extras(c));
    let hotkeys = CURVE_HOTKEYS.map(|(keys, what)| -> Box<dyn Scene> { Box::new(hotkey_row(keys, what)) });
    out.push(subpane_box("Hotkeys", hotkeys.into()));
    out
}
