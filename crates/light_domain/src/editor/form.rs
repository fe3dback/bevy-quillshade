//! The environment editor kind (design §9.2): the organized form in the editor content (`Sky & IBL`, `Ambient Light`,
//! `Directional Lights`, `Camera & Tonemapping`, `Color Grading`, and `Distance Fog`), an editor topbar opening the
//! cube map, and a right sidebar with what the environment is, its problems and the hotkeys. Every field is a core
//! property widget keyed by [`EnvironmentProp`].

use bevy::feathers::controls::{
    ButtonVariant, ColorInputValue, FeathersColorInput, FeathersToolButton, color_input_self_update,
};
use bevy::prelude::*;
use editor_core::action::{Action, Actions, TabAction};
use editor_core::doc_kind::{StatusCtx, TabDoc};
use editor_core::dynval::PropKey;
use editor_core::editors::{EditorAreas, SidebarKey, ViewOf, bar_row, fill_sidebar_with, sidebar_body};
use editor_core::state::editor::Editor;
use editor_core::tab::ViewKindId;
use editor_core::theme::{ACCENT, TEXT, TEXT_DIM, text};
use editor_core::tooltip::Tooltip;
use editor_core::ui::icons::{IconId, icon};
use editor_core::ui::props::{
    FieldEditEnded, PropField, checkbox, hotkey_row, labeled, linear_color, number_ranged, select_values, text_w,
    vector_fields,
};
use editor_core::ui::shell::VIEW_BG;
use editor_core::ui::widgets::{Caption, GroupButton, button_group, scroll_column, subpane_box};
use editor_core::ui::{OnActivate, icon_tool_button};
use texture_kit::editor::TEXTURE_ICON;

use super::{CgSection, ENVIRONMENT_VIEW, EnvironmentAction, EnvironmentDoc, EnvironmentProp, LightState};
use crate::backend::{ColorGradingSectionDef, FogFalloffDef, LIGHT, TEXTURE_SUFFIX, TonemappingMode, texture_ref};

/// Widest the form grows in the editor content (logical px).
const FORM_W: f32 = 430.0;
const WARN: Color = Color::srgb(0.95, 0.78, 0.35);
const ERROR: Color = Color::srgb(0.95, 0.42, 0.42);

/// Soft limits of the number fields: dragging stays inside, a typed value may leave them.
const BRIGHTNESS_RANGE: (f32, f32) = (0.0, 100_000.0);
const INTENSITY_RANGE: (f32, f32) = (0.0, 100_000.0);
const ILLUMINANCE_RANGE: (f32, f32) = (0.0, 150_000.0);
const YAW_RANGE: (f32, f32) = (-180.0, 180.0);
const ELEVATION_RANGE: (f32, f32) = (-90.0, 90.0);

/// The environment editor kind.
pub struct EnvironmentEditor;

/// The editor topbar row of the environment tab (opening the cube map).
#[derive(Component, Default, Clone)]
pub struct EnvironmentViewBar;

/// The form column in the editor content, rebuilt on [`EnvironmentDoc::layout`].
#[derive(Component, Default, Clone)]
pub struct EnvironmentForm;

/// The right sidebar body of the environment editor.
#[derive(Component, Default, Clone)]
pub struct EnvironmentSidebarBody;

/// Color input kept live-synced with an [`EnvironmentProp`] on undo/redo.
#[derive(Component, Clone, Copy)]
pub struct EnvironmentSwatch(pub EnvironmentProp);

/// Alias for [`EnvironmentSwatch`].
pub type LightSwatchOf = EnvironmentSwatch;

/// Per-property reset icon button tinted [`ACCENT`] when the property differs from its default value.
#[derive(Component, Clone, Copy)]
pub struct EnvironmentResetIcon(pub EnvironmentProp);

impl EditorAreas for EnvironmentEditor {
    const VIEW: ViewKindId = ENVIRONMENT_VIEW;
    const SIDEBAR_TITLE: &'static str = "Environment";

    fn topbar() -> Option<impl Scene> {
        Some(bsn! { @bar_row() EnvironmentViewBar })
    }

    fn sidebar() -> Option<impl Scene> {
        Some(bsn! { @sidebar_body() EnvironmentSidebarBody })
    }

    fn build(app: &mut App) {
        super::preview::setup_preview_ui(app);
        app.add_observer(color_edit_ended).add_systems(
            Update,
            (redraw_bar, redraw_form, redraw_sidebar, update_swatches, update_reset_icons)
                .after(editor_core::apply_actions),
        );
    }
}

/// The active environment tab with its index.
fn active(editor: &Editor) -> Option<(usize, &EnvironmentDoc)> {
    let i = editor.active?;
    Some((i, editor.tabs.get(i)?.get::<EnvironmentDoc>()?))
}

/// The environment view in the editor content (`{properties} | {preview}`).
pub fn environment_view() -> impl Scene {
    let area: Box<dyn Scene> = Box::new(super::preview::preview_viewport(ENVIRONMENT_VIEW));
    bsn! {
        ViewOf(ENVIRONMENT_VIEW)
        Node {
            position_type: PositionType::Absolute,
            left: px(0),
            right: px(0),
            top: px(0),
            bottom: px(0),
            flex_direction: FlexDirection::Column,
        }
        BackgroundColor(VIEW_BG)
        Visibility::Hidden
        Children [
            Node { flex_grow: 1.0, min_height: px(0), flex_direction: FlexDirection::Row }
            Children [
                // The bottom padding keeps the form clear of the file type watermark.
                @scroll_column()
                EnvironmentForm
                Node {
                    width: px(FORM_W),
                    flex_grow: 0.0,
                    flex_shrink: 0.0,
                    padding: UiRect::new(px(16), px(16), px(12), px(56)),
                    row_gap: px(12),
                }
                --
                @{area}
            ]
        ]
    }
}

/// The domain's Light Texture assets (their paths), as of the last rescan.
fn light_textures(editor: &Editor) -> Vec<String> {
    let files = editor.kind_files.get(&LIGHT).into_iter().flatten();
    files.filter(|r| r.suffix == TEXTURE_SUFFIX).map(|r| r.path.clone()).collect()
}

/// The picker's labels and values: "None" (`""`), then every Light texture; a cube map that is none of them is
/// listed as missing, so the picker shows what the file says. The index of the current one.
pub(super) fn cubemap_options(textures: &[String], current: Option<&str>) -> (Vec<String>, Vec<String>, usize) {
    let (mut labels, mut values) = (vec!["None".to_string()], vec![String::new()]);
    for t in textures {
        labels.push(t.clone());
        values.push(t.clone());
    }
    if let Some(c) = current.filter(|c| !textures.iter().any(|t| t == c)) {
        labels.push(format!("{c} (missing)"));
        values.push(c.to_string());
    }
    let at = values.iter().position(|v| v == current.unwrap_or_default()).unwrap_or(0);
    (labels, values, at)
}

fn reset_icon_color(is_default: bool) -> Color {
    if is_default { TEXT_DIM.with_alpha(0.35) } else { ACCENT }
}

fn reset_button(doc: &EnvironmentDoc, prop: EnvironmentProp) -> Box<dyn Scene> {
    let color = reset_icon_color(doc.is_default(prop));
    let tag = EnvironmentResetIcon(prop);
    let act = OnActivate(EnvironmentAction::ResetProp(prop).into());
    let tip = Tooltip("Reset to default value".into());
    Box::new(bsn! {
        @FeathersToolButton {
            @variant: ButtonVariant::Plain,
            @caption: bsn! { @icon(IconId::Undo, 14.0, color) tag },
        }
        Node { flex_shrink: 0.0 }
        act
        tip
    })
}

fn labeled_prop(doc: &EnvironmentDoc, label: &str, prop: EnvironmentProp, widget: Box<dyn Scene>) -> Box<dyn Scene> {
    let reset = reset_button(doc, prop);
    let row = Box::new(bsn! {
        Node { flex_direction: FlexDirection::Row, column_gap: px(4), flex_grow: 1.0, align_items: AlignItems::Center }
        Children [ @{widget} -- @{reset} ]
    });
    Box::new(labeled(label.into(), row))
}

fn bool_prop_row(doc: &EnvironmentDoc, label: &str, prop: EnvironmentProp, checked: bool) -> Box<dyn Scene> {
    let cb = checkbox(label, prop, checked);
    let reset = reset_button(doc, prop);
    Box::new(bsn! {
        Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::SpaceBetween,
            min_height: px(26),
        }
        Children [ @{cb} -- @{reset} ]
    })
}

fn color_input_row(doc: &EnvironmentDoc, label: &str, prop: EnvironmentProp, vals: &[f32]) -> Box<dyn Scene> {
    let target = PropKey::new(prop);
    let field = PropField { target, ..Default::default() };
    let c = linear_color(vals);
    let sw = EnvironmentSwatch(prop);
    let widget = Box::new(bsn! {
        Node { flex_direction: FlexDirection::Row, column_gap: px(3), flex_grow: 1.0, align_items: AlignItems::Center }
        Children [
            @FeathersColorInput
            ColorInputValue(c)
            on(color_input_self_update)
            field
            sw
        ]
    });
    labeled_prop(doc, label, prop, widget)
}

fn color_row(doc: &EnvironmentDoc, label: &str, prop: EnvironmentProp, rgba: &[f32; 4]) -> Box<dyn Scene> {
    color_input_row(doc, label, prop, rgba)
}

fn vector_row(doc: &EnvironmentDoc, label: &str, prop: EnvironmentProp, vals: &[f32]) -> Box<dyn Scene> {
    if vals.len() >= 3 {
        return color_input_row(doc, label, prop, vals);
    }
    let target = PropKey::new(prop);
    let items = vector_fields(&target, vals, false);
    let widget = Box::new(bsn! {
        Node { flex_direction: FlexDirection::Row, column_gap: px(3), flex_grow: 1.0, align_items: AlignItems::Center }
        Children [ {items} ]
    });
    labeled_prop(doc, label, prop, widget)
}

/// The form organized into 6 subpanes: Sky & IBL, Ambient Light, Directional Lights, Camera & Tonemapping,
/// Color Grading, and Distance Fog.
fn form(doc: &EnvironmentDoc, textures: &[String]) -> Vec<Box<dyn Scene>> {
    let env = &doc.env;
    let advanced = !doc.easy;

    // 1. Sky & IBL
    let (labels, values, at) = cubemap_options(textures, env.cubemap.as_deref());
    let picker = select_values(EnvironmentProp::Cubemap, labels, values, at);
    let mut sky_rows: Vec<Box<dyn Scene>> = vec![
        color_row(doc, "Clear color", EnvironmentProp::ClearColor, &env.clear_color),
        labeled_prop(doc, "Cube map", EnvironmentProp::Cubemap, picker),
        hint(
            "Optional Light Texture asset (.ktx2 cube map): the skybox and the source of the IBL reflections. When \
             None (or its faces are not square power-of-two), the IBL uses a procedural gradient from Sky top / Sky \
             horizon / Ground colors.",
        ),
        labeled_prop(
            doc,
            "Sky brightness",
            EnvironmentProp::SkyboxBrightness,
            number_ranged(EnvironmentProp::SkyboxBrightness, 0, env.skybox_brightness, Some(BRIGHTNESS_RANGE), None),
        ),
        hint("Brightness of the skybox in cd/m2 (Bevy's Skybox::brightness)."),
    ];
    if advanced {
        sky_rows.push(labeled_prop(
            doc,
            "Sky rotation",
            EnvironmentProp::SkyboxRotation,
            number_ranged(EnvironmentProp::SkyboxRotation, 0, env.skybox_rotation, Some(YAW_RANGE), None),
        ));
        sky_rows.push(hint("Yaw of the skybox in degrees; the IBL reflections turn with it."));
    }
    sky_rows.push(labeled_prop(
        doc,
        "Env intensity",
        EnvironmentProp::EnvIntensity,
        number_ranged(EnvironmentProp::EnvIntensity, 0, env.env_intensity, Some(INTENSITY_RANGE), None),
    ));
    sky_rows.push(hint("Scale of the diffuse/specular IBL light (Bevy's GeneratedEnvironmentMapLight::intensity)."));
    sky_rows.push(color_row(doc, "Sky top color", EnvironmentProp::SkyTopColor, &env.sky_top_color));
    sky_rows.push(color_row(doc, "Sky horizon", EnvironmentProp::SkyHorizonColor, &env.sky_horizon_color));
    sky_rows.push(color_row(doc, "Ground color", EnvironmentProp::GroundColor, &env.ground_color));
    if advanced {
        sky_rows.push(bool_prop_row(
            doc,
            "Affects lightmapped meshes",
            EnvironmentProp::EnvAffectsLightmapped,
            env.env_affects_lightmapped,
        ));
    }

    // 2. Ambient Light
    let mut ambient_rows: Vec<Box<dyn Scene>> = vec![
        color_row(doc, "Color", EnvironmentProp::AmbientColor, &env.ambient_color),
        labeled_prop(
            doc,
            "Brightness",
            EnvironmentProp::AmbientBrightness,
            number_ranged(EnvironmentProp::AmbientBrightness, 0, env.ambient_brightness, Some((0.0, 10_000.0)), None),
        ),
    ];
    if advanced {
        ambient_rows.push(bool_prop_row(
            doc,
            "Affects lightmapped meshes",
            EnvironmentProp::AmbientAffectsLightmapped,
            env.ambient_affects_lightmapped,
        ));
    }

    // 3. Directional Lights
    let add_btn = icon_tool_button(
        IconId::Plus,
        "Add light",
        EnvironmentAction::AddLight,
        "Add a directional light slot to this environment",
    );
    let mut light_rows: Vec<Box<dyn Scene>> = vec![Box::new(bsn! {
        Node { flex_direction: FlexDirection::Row, align_items: AlignItems::Center }
        Children [ @{add_btn} ]
    })];
    if env.lights.is_empty() {
        light_rows.push(Box::new(text("No directional lights in this environment.", TEXT_DIM, 12.0)));
    }
    for (i, l) in env.lights.iter().enumerate() {
        let slot_title = format!("Light #{}: {}", i + 1, l.name);
        let remove_btn = icon_tool_button(
            IconId::Delete,
            "Remove",
            EnvironmentAction::RemoveLight(i),
            format!("Remove directional light slot #{}", i + 1),
        );
        let name_input = text_w(EnvironmentProp::LightName(i), l.name.clone(), false);
        let name_widget = Box::new(bsn! {
            Node { flex_grow: 1.0, min_width: px(0) }
            Children [ @{name_input} ]
        });
        let mut slot_rows: Vec<Box<dyn Scene>> = vec![
            Box::new(bsn! {
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::FlexEnd,
                }
                Children [ @{remove_btn} ]
            }),
            labeled_prop(doc, "Name", EnvironmentProp::LightName(i), name_widget),
            color_row(doc, "Color", EnvironmentProp::LightColor(i), &l.color),
            labeled_prop(
                doc,
                "Illuminance (lux)",
                EnvironmentProp::LightIlluminance(i),
                number_ranged(EnvironmentProp::LightIlluminance(i), 0, l.illuminance, Some(ILLUMINANCE_RANGE), None),
            ),
            labeled_prop(
                doc,
                "Azimuth (deg)",
                EnvironmentProp::LightAzimuth(i),
                number_ranged(EnvironmentProp::LightAzimuth(i), 0, l.azimuth_deg, Some(YAW_RANGE), None),
            ),
            labeled_prop(
                doc,
                "Elevation (deg)",
                EnvironmentProp::LightElevation(i),
                number_ranged(
                    EnvironmentProp::LightElevation(i),
                    0,
                    l.elevation_deg,
                    Some(ELEVATION_RANGE),
                    Some(ELEVATION_RANGE),
                ),
            ),
            bool_prop_row(doc, "Shadow maps", EnvironmentProp::LightShadowMaps(i), l.shadow_maps_enabled),
            bool_prop_row(doc, "Contact shadows", EnvironmentProp::LightContactShadows(i), l.contact_shadows_enabled),
        ];
        if advanced {
            slot_rows.push(labeled_prop(
                doc,
                "Depth bias",
                EnvironmentProp::LightShadowDepthBias(i),
                number_ranged(EnvironmentProp::LightShadowDepthBias(i), 0, l.shadow_depth_bias, Some((0.0, 1.0)), None),
            ));
            slot_rows.push(labeled_prop(
                doc,
                "Normal bias",
                EnvironmentProp::LightShadowNormalBias(i),
                number_ranged(
                    EnvironmentProp::LightShadowNormalBias(i),
                    0,
                    l.shadow_normal_bias,
                    Some((0.0, 10.0)),
                    None,
                ),
            ));
            slot_rows.push(bool_prop_row(
                doc,
                "Affects lightmapped",
                EnvironmentProp::LightAffectsLightmapped(i),
                l.affects_lightmapped_mesh_diffuse,
            ));
        }
        light_rows.push(subpane_box(&slot_title, slot_rows));
    }

    // 4. Camera & Tonemapping
    let mut camera_rows: Vec<Box<dyn Scene>> = vec![labeled_prop(
        doc,
        "Exposure (EV100)",
        EnvironmentProp::Ev100,
        number_ranged(EnvironmentProp::Ev100, 0, env.ev100, Some((-5.0, 25.0)), None),
    )];
    if advanced {
        let tm_labels: Vec<String> = TonemappingMode::ALL.iter().map(|m| m.label().to_string()).collect();
        let tm_values: Vec<String> = TonemappingMode::ALL.iter().map(|m| m.as_str().to_string()).collect();
        let tm_at = TonemappingMode::ALL.iter().position(|&m| m == env.tonemapping).unwrap_or(0);
        let tm_picker = select_values(EnvironmentProp::Tonemapping, tm_labels, tm_values, tm_at);
        camera_rows.push(labeled_prop(doc, "Tonemapping", EnvironmentProp::Tonemapping, tm_picker));
        camera_rows.push(bool_prop_row(doc, "Deband dither", EnvironmentProp::DebandDither, env.deband_dither));
    }

    // 5. Color Grading
    let cg = &env.color_grading;
    let mut global_rows: Vec<Box<dyn Scene>> = vec![
        labeled_prop(
            doc,
            "Exposure",
            EnvironmentProp::CgGlobalExposure,
            number_ranged(EnvironmentProp::CgGlobalExposure, 0, cg.global.exposure, Some((-5.0, 5.0)), None),
        ),
        labeled_prop(
            doc,
            "Temperature",
            EnvironmentProp::CgGlobalTemperature,
            number_ranged(EnvironmentProp::CgGlobalTemperature, 0, cg.global.temperature, Some((-1.0, 1.0)), None),
        ),
        labeled_prop(
            doc,
            "Tint",
            EnvironmentProp::CgGlobalTint,
            number_ranged(EnvironmentProp::CgGlobalTint, 0, cg.global.tint, Some((-1.0, 1.0)), None),
        ),
        labeled_prop(
            doc,
            "Hue",
            EnvironmentProp::CgGlobalHue,
            number_ranged(
                EnvironmentProp::CgGlobalHue,
                0,
                cg.global.hue,
                Some((-std::f32::consts::PI, std::f32::consts::PI)),
                None,
            ),
        ),
        labeled_prop(
            doc,
            "Post saturation",
            EnvironmentProp::CgGlobalPostSaturation,
            number_ranged(
                EnvironmentProp::CgGlobalPostSaturation,
                0,
                cg.global.post_saturation,
                Some((0.0, 2.0)),
                None,
            ),
        ),
    ];
    if advanced {
        global_rows.push(vector_row(
            doc,
            "Midtones range",
            EnvironmentProp::CgGlobalMidtonesRange,
            &cg.global.midtones_range,
        ));
    }
    let section_rows = |sec: CgSection, s: &ColorGradingSectionDef| -> Vec<Box<dyn Scene>> {
        vec![
            labeled_prop(
                doc,
                "Saturation",
                EnvironmentProp::CgSectionSaturation(sec),
                number_ranged(EnvironmentProp::CgSectionSaturation(sec), 0, s.saturation, Some((0.0, 2.0)), None),
            ),
            labeled_prop(
                doc,
                "Contrast",
                EnvironmentProp::CgSectionContrast(sec),
                number_ranged(EnvironmentProp::CgSectionContrast(sec), 0, s.contrast, Some((0.0, 2.0)), None),
            ),
            labeled_prop(
                doc,
                "Gamma",
                EnvironmentProp::CgSectionGamma(sec),
                number_ranged(EnvironmentProp::CgSectionGamma(sec), 0, s.gamma, Some((0.1, 3.0)), None),
            ),
            labeled_prop(
                doc,
                "Gain",
                EnvironmentProp::CgSectionGain(sec),
                number_ranged(EnvironmentProp::CgSectionGain(sec), 0, s.gain, Some((0.0, 2.0)), None),
            ),
            labeled_prop(
                doc,
                "Lift",
                EnvironmentProp::CgSectionLift(sec),
                number_ranged(EnvironmentProp::CgSectionLift(sec), 0, s.lift, Some((-1.0, 1.0)), None),
            ),
        ]
    };
    let cg_rows: Vec<Box<dyn Scene>> = vec![
        subpane_box("Global", global_rows),
        subpane_box("Shadows", section_rows(CgSection::Shadows, &cg.shadows)),
        subpane_box("Midtones", section_rows(CgSection::Midtones, &cg.midtones)),
        subpane_box("Highlights", section_rows(CgSection::Highlights, &cg.highlights)),
    ];

    // 6. Distance Fog
    let fog = &env.fog;
    let fog_modes = ["Linear", "Exponential", "ExponentialSquared", "Atmospheric"];
    let fog_at = match &fog.falloff {
        FogFalloffDef::Linear { .. } => 0,
        FogFalloffDef::Exponential { .. } => 1,
        FogFalloffDef::ExponentialSquared { .. } => 2,
        FogFalloffDef::Atmospheric { .. } => 3,
    };
    let fog_picker = select_values(
        EnvironmentProp::FogFalloffMode,
        fog_modes.iter().map(|s| (*s).to_string()).collect(),
        fog_modes.iter().map(|s| (*s).to_string()).collect(),
        fog_at,
    );
    let mut fog_rows: Vec<Box<dyn Scene>> = vec![
        bool_prop_row(doc, "Enabled", EnvironmentProp::FogEnabled, fog.enabled),
        color_row(doc, "Fog color", EnvironmentProp::FogColor, &fog.color),
    ];
    if advanced {
        fog_rows.push(color_row(
            doc,
            "Inscattering color",
            EnvironmentProp::FogDirectionalLightColor,
            &fog.directional_light_color,
        ));
    }
    fog_rows.push(labeled_prop(
        doc,
        "Light exponent",
        EnvironmentProp::FogDirectionalLightExponent,
        number_ranged(
            EnvironmentProp::FogDirectionalLightExponent,
            0,
            fog.directional_light_exponent,
            Some((1.0, 64.0)),
            None,
        ),
    ));
    fog_rows.push(labeled_prop(doc, "Falloff mode", EnvironmentProp::FogFalloffMode, fog_picker));
    match &fog.falloff {
        FogFalloffDef::Linear { start, end } => {
            fog_rows.push(labeled_prop(
                doc,
                "Linear start",
                EnvironmentProp::FogLinearStart,
                number_ranged(EnvironmentProp::FogLinearStart, 0, *start, Some((0.0, 1000.0)), None),
            ));
            fog_rows.push(labeled_prop(
                doc,
                "Linear end",
                EnvironmentProp::FogLinearEnd,
                number_ranged(EnvironmentProp::FogLinearEnd, 0, *end, Some((0.0, 2000.0)), None),
            ));
        }
        FogFalloffDef::Exponential { density } | FogFalloffDef::ExponentialSquared { density } => {
            fog_rows.push(labeled_prop(
                doc,
                "Density",
                EnvironmentProp::FogDensity,
                number_ranged(EnvironmentProp::FogDensity, 0, *density, Some((0.0, 0.5)), None),
            ));
        }
        FogFalloffDef::Atmospheric { extinction, inscattering } => {
            fog_rows.push(vector_row(doc, "Extinction", EnvironmentProp::FogAtmosphericExtinction, extinction));
            fog_rows.push(vector_row(doc, "Inscattering", EnvironmentProp::FogAtmosphericInscattering, inscattering));
        }
    }

    let panes = vec![
        subpane_box("Sky & IBL", sky_rows),
        subpane_box("Ambient Light", ambient_rows),
        subpane_box("Directional Lights", light_rows),
        subpane_box("Camera & Tonemapping", camera_rows),
        subpane_box("Color Grading", cg_rows),
        subpane_box("Distance Fog", fog_rows),
    ];
    vec![Box::new(bsn! {
        Node { width: percent(100), max_width: px(FORM_W), flex_direction: FlexDirection::Column, row_gap: px(12) }
        Children [ {panes} ]
    })]
}

/// A note under a [`labeled`] row, in its widget column.
fn hint(s: &str) -> Box<dyn Scene> {
    let s = s.to_string();
    Box::new(bsn! { Node { margin: UiRect::left(px(116)) } Children [ @text(s, TEXT_DIM, 11.0) ] })
}

/// What the form shows: the project generation, the active tab, its [`EnvironmentDoc::layout`], the Light textures.
type FormKey = (u64, usize, u64, Vec<String>);

/// Rebuilds the form when another environment tab is shown, its form must follow the model (not during a drag) or
/// the Light textures changed.
fn redraw_form(
    editor: Res<Editor>,
    body: Query<Entity, With<EnvironmentForm>>,
    mut last: Local<Option<FormKey>>,
    mut commands: Commands,
) {
    let Some((i, doc)) = active(&editor) else {
        *last = None;
        return;
    };
    let textures = light_textures(&editor);
    let key = (editor.project_generation, i, doc.layout, textures);
    if last.as_ref() == Some(&key) {
        return;
    }
    let Ok(body) = body.single() else { return };
    commands.entity(body).despawn_children();
    for s in form(doc, &key.3) {
        commands.spawn_scene(bsn! { @{s} ChildOf(body) });
    }
    *last = Some(key);
}

/// Keeps [`EnvironmentSwatch`] color inputs synced with the active [`EnvironmentDoc`] on undo/redo.
fn update_swatches(
    editor: Res<Editor>,
    q: Query<(Entity, &EnvironmentSwatch, &ColorInputValue)>,
    mut commands: Commands,
) {
    if !editor.is_changed() {
        return;
    }
    let Some((_, doc)) = active(&editor) else { return };
    let ctx = StatusCtx::default();
    for (e, sw, v) in &q {
        if let Some(val) = doc.current_value(&PropKey::new(sw.0), &ctx) {
            let floats = val.floats();
            if floats.len() >= 3 {
                let c = linear_color(&floats);
                if v.0 != c {
                    commands.entity(e).try_insert(ColorInputValue(c));
                }
            }
        }
    }
}

/// Keeps [`EnvironmentResetIcon`] colors synced with the active [`EnvironmentDoc`] during live drags and edits.
fn update_reset_icons(editor: Res<Editor>, mut q: Query<(&EnvironmentResetIcon, &mut ImageNode)>) {
    if !editor.is_changed() {
        return;
    }
    let Some((_, doc)) = active(&editor) else { return };
    for (reset, mut img) in &mut q {
        let want = reset_icon_color(doc.is_default(reset.0));
        if img.color != want {
            img.color = want;
        }
    }
}

fn color_edit_ended(ev: On<FieldEditEnded>, swatches: Query<&EnvironmentSwatch>, mut actions: ResMut<Actions>) {
    if swatches.contains(ev.source) {
        actions.push(Action::Tab(TabAction::CommitInline));
    }
}

type BarKey = (usize, bool, Option<String>, bool);

/// Rebuilds the editor topbar when another environment tab is shown, its `Easy / Advanced` mode changed, or its cube
/// map changed: `Easy / Advanced` toggle on the left, then "Open cube map" while it names a Light texture.
fn redraw_bar(
    editor: Res<Editor>,
    bar: Query<Entity, With<EnvironmentViewBar>>,
    mut last: Local<Option<BarKey>>,
    mut commands: Commands,
) {
    let Some((i, doc)) = active(&editor) else {
        *last = None;
        return;
    };
    let cubemap = doc.env.cubemap.clone();
    let known = cubemap.as_ref().is_some_and(|c| light_textures(&editor).contains(c));
    let key = (i, doc.easy, cubemap, known);
    if last.as_ref() == Some(&key) {
        return;
    }
    let Ok(bar) = bar.single() else { return };
    commands.entity(bar).despawn_children();
    let modes = [
        (
            true,
            "Easy",
            "Essential lighting, camera, color grading, and fog settings (resets advanced fields to default)",
        ),
        (
            false,
            "Advanced",
            "All environment settings including rotations, shadow biases, tonemapping, and lightmap flags",
        ),
    ]
    .map(|(easy, label, tip)| GroupButton {
        caption: Caption::Text(label.into()),
        action: EnvironmentAction::SetEasy(easy).into(),
        selected: doc.easy == easy,
        disabled: false,
        tip: tip.into(),
    });
    let mode_group = button_group(modes.into());
    commands.spawn_scene(bsn! { @{mode_group} ChildOf(bar) });
    let item: Box<dyn Scene> = match &key.2 {
        Some(path) if known => Box::new(icon_tool_button(
            TEXTURE_ICON,
            "Open cube map",
            editor_core::action::Action::OpenAsset(texture_ref(path)),
            format!("Open the Light Texture asset {path}"),
        )),
        Some(path) => Box::new(text(format!("Cube map `{path}` is no Light Texture asset"), ERROR, 12.0)),
        None => Box::new(text("No cube map set", TEXT_DIM, 12.0)),
    };
    commands.spawn_scene(bsn! { @{item} ChildOf(bar) });
    *last = Some(key);
}

fn redraw_sidebar(
    editor: Res<Editor>,
    body: Query<Entity, With<EnvironmentSidebarBody>>,
    mut key: Local<SidebarKey>,
    mut commands: Commands,
) {
    let Some((_, doc)) = active(&editor) else { return key.forget() };
    fill_sidebar_with(&mut commands, &body, &mut key, SidebarKey::now(&editor), || sidebar_rows(&editor, doc));
}

/// Hotkeys of the environment tab.
const HOTKEYS: [(&str, &str); 2] = [("Ctrl+Z / Y", "Undo / redo"), ("Ctrl+S", "Save and regenerate")];

/// What the environment is, its problems (the Light domain's diagnostics of the asset), the hotkeys.
fn sidebar_rows(editor: &Editor, doc: &EnvironmentDoc) -> Vec<Box<dyn Scene>> {
    let about: Vec<Box<dyn Scene>> = vec![
        Box::new(text(doc.base_title(), TEXT, 15.0)),
        Box::new(text("Type: Environment", TEXT_DIM, 12.0)),
        Box::new(text(
            "Camera skybox, IBL, ambient light, directional lights, tonemapping, color grading, and distance fog. \
             Saving regenerates crates/quillshade/src/light/mod.rs and assets/quillshade/light/autogen/baked.ron.",
            TEXT_DIM,
            12.0,
        )),
    ];
    let mut out = vec![subpane_box("Environment", about)];
    let rel = doc.aref.source_rel();
    let problems: Vec<Box<dyn Scene>> = editor
        .domains
        .get::<LightState>()
        .into_iter()
        .flat_map(|s| &s.diagnostics)
        .filter(|d| d.asset.as_deref() == Some(rel.as_str()))
        .map(|d| -> Box<dyn Scene> {
            let c = if d.is_error() { ERROR } else { WARN };
            Box::new(text(format!("{}: {}", d.code, d.message), c, 12.0))
        })
        .collect();
    if !problems.is_empty() {
        out.push(subpane_box("Problems", problems));
    }
    let hotkeys = HOTKEYS.map(|(keys, what)| -> Box<dyn Scene> { Box::new(hotkey_row(keys, what)) });
    out.push(subpane_box("Hotkeys", hotkeys.into()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picker_lists_none_the_textures_and_a_missing_one() {
        let textures = vec!["sky".to_string(), "skies/blue".to_string()];
        let (labels, values, at) = cubemap_options(&textures, None);
        assert_eq!(
            (labels, values, at),
            (
                vec!["None".into(), "sky".into(), "skies/blue".into()],
                vec![String::new(), "sky".into(), "skies/blue".into()],
                0
            )
        );
        assert_eq!(cubemap_options(&textures, Some("skies/blue")).2, 2);
        let (labels, values, at) = cubemap_options(&textures, Some("gone"));
        assert_eq!((labels[3].as_str(), values[3].as_str(), at), ("gone (missing)", "gone", 3));
    }
}
