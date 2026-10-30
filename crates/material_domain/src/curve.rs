//! Material's curves (decision 0015, design §7.1): `curve_kit`'s curve editor for `quillshade/material/**/*.curve.ron`
//! ([`MaterialCurve`]), plus what only Material has: the editor topbar's "Show generated WESL" (each curve is a WESL
//! module) and the Project tree row's generated-file items. Codegen and the reference follow-ups (`curve_refs`) stay
//! in Material's backend and ops.

use bevy::prelude::Scene;
use curve_kit::editor::CurveDomain;
use editor_core::menu::MenuEntry;
use editor_core::state::editor::Editor;
use editor_core::tab::ViewKindId;
use editor_core::ui::icon_button;
use editor_core::ui::icons::IconId;
use project_core::{AssetRef, DomainId};
use shader_project::AssetId;
use shader_project::layout::MATERIAL;

use crate::ops::MaterialAction;
use crate::tab::WESL;

pub use curve_kit::editor::{CurveAction, CurveClipboard, CurveProp};

/// A curve edit for Material's curve tab (`curve_kit`'s, tagged with [`MaterialCurve`], design §14.4).
pub type CurveEdit = curve_kit::editor::CurveEdit<MaterialCurve>;
/// A property target of Material's curve tab (`curve_kit`'s [`CurveProp`], tagged with [`MaterialCurve`]).
pub type CurveKey = editor_core::dynval::KitProp<MaterialCurve, CurveProp>;

/// Material's curve domain (`curve_kit::editor::register::<MaterialCurve>` in [`crate::MaterialDomain`]).
pub struct MaterialCurve;

/// Material's curve editor kind's view.
pub const CURVE: ViewKindId = ViewKindId("material.curve");

/// A Material curve tab's document.
pub type CurveDoc = curve_kit::editor::CurveDoc<MaterialCurve>;
/// Material's curve editor kind.
pub type CurveEditor = curve_kit::editor::CurveEditor<MaterialCurve>;
/// Material's curve editor input context.
pub type CurveCtx = curve_kit::editor::CurveCtx<MaterialCurve>;

impl CurveDomain for MaterialCurve {
    const DOMAIN: DomainId = MATERIAL;
    const VIEW: ViewKindId = CURVE;

    /// "Show generated WESL": the curve's module.
    fn topbar_extras(doc: &CurveDoc) -> Vec<Box<dyn Scene>> {
        let Some(id) = AssetId::from_ref(&doc.aref) else { return Vec::new() };
        let wesl = MaterialAction::ShowGenerated(id, WESL);
        vec![Box::new(icon_button(IconId::Code, wesl, "Show generated WESL"))]
    }

    fn row_menu(editor: &Editor, r: &AssetRef) -> Vec<MenuEntry> {
        crate::domain::asset_row_menu(editor, r)
    }
}
