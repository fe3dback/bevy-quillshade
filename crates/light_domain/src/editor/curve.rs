//! Light's transition curves (design §9.2, §7.1): `curve_kit`'s curve editor for
//! `quillshade/light/**/*.curve.ron` ([`LightCurve`]). Each error-free curve becomes a variant of
//! `quillshade::light::Curve` for `Light::transition_to(preset, curve, duration)`.

use bevy::prelude::Scene;
use curve_kit::editor::CurveDomain;
use editor_core::tab::ViewKindId;
use editor_core::theme::{TEXT_DIM, text};
use editor_core::ui::widgets::subpane_box;
use project_core::DomainId;

use crate::backend::LIGHT;

/// Light's curve domain (`curve_kit::editor::register::<LightCurve>` in [`super::LightDomain`]).
pub struct LightCurve;

/// Light's curve editor kind's view.
pub const LIGHT_CURVE_VIEW: ViewKindId = ViewKindId("light.curve");

/// A Light curve tab's document.
pub type LightCurveDoc = curve_kit::editor::CurveDoc<LightCurve>;
/// Light's curve editor kind.
pub type LightCurveEditor = curve_kit::editor::CurveEditor<LightCurve>;
/// A curve edit for Light's curve tab (`curve_kit`'s, tagged with [`LightCurve`]).
pub type LightCurveEdit = curve_kit::editor::CurveEdit<LightCurve>;

impl CurveDomain for LightCurve {
    const DOMAIN: DomainId = LIGHT;
    const VIEW: ViewKindId = LIGHT_CURVE_VIEW;

    fn sidebar_extras(_doc: &LightCurveDoc) -> Vec<Box<dyn Scene>> {
        let usage = [
            "Each valid curve becomes a PascalCase variant of quillshade::light::Curve for Light::transition_to(preset, curve, duration).",
            "Keep the range at 0..1 and the endpoints at (0, 0) and (1, 1) so transitions start at the previous environment and finish at the target without jumping.",
        ]
        .map(|line| -> Box<dyn Scene> { Box::new(text(line, TEXT_DIM, 12.0)) });
        vec![subpane_box("Usage", usage.into())]
    }
}
