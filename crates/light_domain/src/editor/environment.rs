//! An open environment (design §9.2): the [`Environment`] (source of truth), its undo history and its [`TabDoc`]
//! side (saving, reloading, renames, the editor statusbar). Edits arrive as core property writes keyed by
//! [`EnvironmentProp`] or structural actions ([`EnvironmentAction`]).

use editor_core::action::{Action, Handled, TabAction};
use editor_core::doc_kind::{DocCtx, StatusCtx, StatusInfo, TabDoc};
use editor_core::dynval::{PropKey, PropTarget, PropValue};
use editor_core::history::{History, MergeKey};
use editor_core::preview::{PreviewHandle, start_preview};
use editor_core::state::editor::Editor;
use editor_core::tab::{Tab, ViewKindId};
use editor_core::ui::icons::IconRef;
use editor_core::util::text::plural;
use project_core::{AssetRef, Project};

use super::ENVIRONMENT_ICON;
use super::preview::LIGHT_PREVIEW_KIND;
use crate::backend::{
    ColorGradingDef, ColorGradingSectionDef, DirectionalLightDef, ENVIRONMENT_SUFFIX, Environment, FogFalloffDef,
    LIGHT, TonemappingMode, load_environment, save_environment,
};

/// The environment tab's view.
pub const ENVIRONMENT_VIEW: ViewKindId = ViewKindId("light.environment");

/// Luminance band of [`ColorGradingDef`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CgSection {
    Shadows,
    Midtones,
    Highlights,
}

impl CgSection {
    pub const ALL: [CgSection; 3] = [CgSection::Shadows, CgSection::Midtones, CgSection::Highlights];

    pub fn label(self) -> &'static str {
        match self {
            CgSection::Shadows => "Shadows",
            CgSection::Midtones => "Midtones",
            CgSection::Highlights => "Highlights",
        }
    }
}

/// Structural edits of an [`EnvironmentDoc`], sent as [`TabAction::Doc`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvironmentAction {
    AddLight,
    RemoveLight(usize),
    SetEasy(bool),
    ResetProp(EnvironmentProp),
}

/// Returns `true` when all advanced fields of `env` are at their default values.
pub fn fits_easy(env: &Environment) -> bool {
    env.skybox_rotation == 0.0
        && env.env_affects_lightmapped
        && env.ambient_affects_lightmapped
        && env
            .lights
            .iter()
            .all(|l| l.shadow_depth_bias == 0.02 && l.shadow_normal_bias == 1.8 && l.affects_lightmapped_mesh_diffuse)
        && env.tonemapping == TonemappingMode::TonyMcMapface
        && env.deband_dither
        && env.color_grading.global.midtones_range == [0.2, 0.7]
        && env.fog.directional_light_color == [0.0, 0.0, 0.0, 0.0]
}

/// Resets all advanced fields of `env` to their default values.
pub fn apply_easy_defaults(env: &mut Environment) {
    env.skybox_rotation = 0.0;
    env.env_affects_lightmapped = true;
    env.ambient_affects_lightmapped = true;
    for l in &mut env.lights {
        l.shadow_depth_bias = 0.02;
        l.shadow_normal_bias = 1.8;
        l.affects_lightmapped_mesh_diffuse = true;
    }
    env.tonemapping = TonemappingMode::TonyMcMapface;
    env.deband_dither = true;
    env.color_grading.global.midtones_range = [0.2, 0.7];
    env.fog.directional_light_color = [0.0, 0.0, 0.0, 0.0];
}

impl From<EnvironmentAction> for TabAction {
    fn from(a: EnvironmentAction) -> TabAction {
        TabAction::doc(a)
    }
}

impl From<EnvironmentAction> for Action {
    fn from(a: EnvironmentAction) -> Action {
        Action::Tab(TabAction::doc(a))
    }
}

/// A field of the environment form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvironmentProp {
    // Sky & IBL
    ClearColor,
    /// [`Environment::cubemap`]: a Light Texture asset's path, `""` for none ([`PropValue::Str`]).
    Cubemap,
    /// [`Environment::skybox_brightness`] ([`PropValue::Float`]).
    SkyboxBrightness,
    SkyboxRotation,
    /// [`Environment::env_intensity`] ([`PropValue::Float`]).
    EnvIntensity,
    SkyTopColor,
    SkyHorizonColor,
    GroundColor,
    EnvAffectsLightmapped,
    // Ambient Light
    AmbientColor,
    AmbientBrightness,
    AmbientAffectsLightmapped,
    // Directional Lights (by slot index)
    LightName(usize),
    LightColor(usize),
    LightIlluminance(usize),
    LightAzimuth(usize),
    LightElevation(usize),
    LightShadowMaps(usize),
    LightContactShadows(usize),
    LightShadowDepthBias(usize),
    LightShadowNormalBias(usize),
    LightAffectsLightmapped(usize),
    // Camera & Tonemapping
    Ev100,
    Tonemapping,
    DebandDither,
    // Color Grading
    CgGlobalExposure,
    CgGlobalTemperature,
    CgGlobalTint,
    CgGlobalHue,
    CgGlobalPostSaturation,
    CgGlobalMidtonesRange,
    CgSectionSaturation(CgSection),
    CgSectionContrast(CgSection),
    CgSectionGamma(CgSection),
    CgSectionGain(CgSection),
    CgSectionLift(CgSection),
    // Distance Fog
    FogEnabled,
    FogColor,
    FogDirectionalLightColor,
    FogDirectionalLightExponent,
    FogFalloffMode,
    FogLinearStart,
    FogLinearEnd,
    FogDensity,
    FogAtmosphericExtinction,
    FogAtmosphericInscattering,
}

impl PropTarget for EnvironmentProp {}

fn cg_section(cg: &ColorGradingDef, s: CgSection) -> &ColorGradingSectionDef {
    match s {
        CgSection::Shadows => &cg.shadows,
        CgSection::Midtones => &cg.midtones,
        CgSection::Highlights => &cg.highlights,
    }
}

fn cg_section_mut(cg: &mut ColorGradingDef, s: CgSection) -> &mut ColorGradingSectionDef {
    match s {
        CgSection::Shadows => &mut cg.shadows,
        CgSection::Midtones => &mut cg.midtones,
        CgSection::Highlights => &mut cg.highlights,
    }
}

fn as_vec2(v: &PropValue) -> [f32; 2] {
    let f = v.floats();
    [f.first().copied().unwrap_or(0.0), f.get(1).copied().unwrap_or(0.0)]
}

fn as_vec3(v: &PropValue) -> [f32; 3] {
    let f = v.floats();
    [f.first().copied().unwrap_or(0.0), f.get(1).copied().unwrap_or(0.0), f.get(2).copied().unwrap_or(0.0)]
}

fn as_vec4(v: &PropValue) -> [f32; 4] {
    let f = v.floats();
    [
        f.first().copied().unwrap_or(0.0),
        f.get(1).copied().unwrap_or(0.0),
        f.get(2).copied().unwrap_or(0.0),
        f.get(3).copied().unwrap_or(1.0),
    ]
}

/// An environment tab's document.
pub struct EnvironmentDoc {
    /// The asset; follows renames.
    pub aref: AssetRef,
    /// What the tab shows and edits.
    pub env: Environment,
    /// The environment last saved or loaded: dirty is `env != saved`.
    saved: Environment,
    history: History<Environment>,
    /// Easy mode hides advanced fields and resets them to their default values; off = Advanced mode showing all fields.
    pub easy: bool,
    /// Bumped whenever `env` changes.
    pub revision: u64,
    /// Bumped when the form must be rebuilt: any change but a dragged number field or typed text field (which shows
    /// its own value).
    pub layout: u64,
    /// RAII 3D preview studio session handle (`editor_core::preview::start_preview`).
    pub preview: PreviewHandle,
}

impl EnvironmentDoc {
    pub fn new(aref: AssetRef, env: Environment) -> EnvironmentDoc {
        let easy = fits_easy(&env);
        EnvironmentDoc {
            aref,
            saved: env.clone(),
            env,
            history: History::default(),
            easy,
            revision: 1,
            layout: 1,
            preview: start_preview(LIGHT_PREVIEW_KIND),
        }
    }

    /// Sets `prop` to `value`; `merge` edits of one property are one undo step. `false` when nothing changed.
    fn set(&mut self, prop: EnvironmentProp, value: &PropValue, merge: Option<MergeKey>) -> bool {
        let before = self.env.clone();
        match prop {
            EnvironmentProp::ClearColor => self.env.clear_color = as_vec4(value),
            EnvironmentProp::Cubemap => {
                let path = value.as_str().trim();
                self.env.cubemap = (!path.is_empty()).then(|| path.to_string());
            }
            EnvironmentProp::SkyboxBrightness => self.env.skybox_brightness = value.as_f32().max(0.0),
            EnvironmentProp::SkyboxRotation => self.env.skybox_rotation = value.as_f32(),
            EnvironmentProp::EnvIntensity => self.env.env_intensity = value.as_f32().max(0.0),
            EnvironmentProp::SkyTopColor => self.env.sky_top_color = as_vec4(value),
            EnvironmentProp::SkyHorizonColor => self.env.sky_horizon_color = as_vec4(value),
            EnvironmentProp::GroundColor => self.env.ground_color = as_vec4(value),
            EnvironmentProp::EnvAffectsLightmapped => self.env.env_affects_lightmapped = value.as_bool(),
            EnvironmentProp::AmbientColor => self.env.ambient_color = as_vec4(value),
            EnvironmentProp::AmbientBrightness => self.env.ambient_brightness = value.as_f32().max(0.0),
            EnvironmentProp::AmbientAffectsLightmapped => self.env.ambient_affects_lightmapped = value.as_bool(),
            EnvironmentProp::LightName(i) => {
                if let Some(l) = self.env.lights.get_mut(i) {
                    l.name = value.as_str().to_string();
                }
            }
            EnvironmentProp::LightColor(i) => {
                if let Some(l) = self.env.lights.get_mut(i) {
                    l.color = as_vec4(value);
                }
            }
            EnvironmentProp::LightIlluminance(i) => {
                if let Some(l) = self.env.lights.get_mut(i) {
                    l.illuminance = value.as_f32().max(0.0);
                }
            }
            EnvironmentProp::LightAzimuth(i) => {
                if let Some(l) = self.env.lights.get_mut(i) {
                    l.azimuth_deg = value.as_f32();
                }
            }
            EnvironmentProp::LightElevation(i) => {
                if let Some(l) = self.env.lights.get_mut(i) {
                    l.elevation_deg = value.as_f32().clamp(-90.0, 90.0);
                }
            }
            EnvironmentProp::LightShadowMaps(i) => {
                if let Some(l) = self.env.lights.get_mut(i) {
                    l.shadow_maps_enabled = value.as_bool();
                }
            }
            EnvironmentProp::LightContactShadows(i) => {
                if let Some(l) = self.env.lights.get_mut(i) {
                    l.contact_shadows_enabled = value.as_bool();
                }
            }
            EnvironmentProp::LightShadowDepthBias(i) => {
                if let Some(l) = self.env.lights.get_mut(i) {
                    l.shadow_depth_bias = value.as_f32().max(0.0);
                }
            }
            EnvironmentProp::LightShadowNormalBias(i) => {
                if let Some(l) = self.env.lights.get_mut(i) {
                    l.shadow_normal_bias = value.as_f32().max(0.0);
                }
            }
            EnvironmentProp::LightAffectsLightmapped(i) => {
                if let Some(l) = self.env.lights.get_mut(i) {
                    l.affects_lightmapped_mesh_diffuse = value.as_bool();
                }
            }
            EnvironmentProp::Ev100 => self.env.ev100 = value.as_f32(),
            EnvironmentProp::Tonemapping => {
                if let Some(m) = TonemappingMode::from_str_name(value.as_str()) {
                    self.env.tonemapping = m;
                }
            }
            EnvironmentProp::DebandDither => self.env.deband_dither = value.as_bool(),
            EnvironmentProp::CgGlobalExposure => self.env.color_grading.global.exposure = value.as_f32(),
            EnvironmentProp::CgGlobalTemperature => self.env.color_grading.global.temperature = value.as_f32(),
            EnvironmentProp::CgGlobalTint => self.env.color_grading.global.tint = value.as_f32(),
            EnvironmentProp::CgGlobalHue => self.env.color_grading.global.hue = value.as_f32(),
            EnvironmentProp::CgGlobalPostSaturation => {
                self.env.color_grading.global.post_saturation = value.as_f32().max(0.0);
            }
            EnvironmentProp::CgGlobalMidtonesRange => {
                let [a, b] = as_vec2(value);
                self.env.color_grading.global.midtones_range = [a.max(0.0), b.max(0.0)];
            }
            EnvironmentProp::CgSectionSaturation(s) => {
                cg_section_mut(&mut self.env.color_grading, s).saturation = value.as_f32().max(0.0);
            }
            EnvironmentProp::CgSectionContrast(s) => {
                cg_section_mut(&mut self.env.color_grading, s).contrast = value.as_f32().max(0.0);
            }
            EnvironmentProp::CgSectionGamma(s) => {
                cg_section_mut(&mut self.env.color_grading, s).gamma = value.as_f32().max(0.0);
            }
            EnvironmentProp::CgSectionGain(s) => {
                cg_section_mut(&mut self.env.color_grading, s).gain = value.as_f32().max(0.0);
            }
            EnvironmentProp::CgSectionLift(s) => {
                cg_section_mut(&mut self.env.color_grading, s).lift = value.as_f32();
            }
            EnvironmentProp::FogEnabled => self.env.fog.enabled = value.as_bool(),
            EnvironmentProp::FogColor => self.env.fog.color = as_vec4(value),
            EnvironmentProp::FogDirectionalLightColor => self.env.fog.directional_light_color = as_vec4(value),
            EnvironmentProp::FogDirectionalLightExponent => {
                self.env.fog.directional_light_exponent = value.as_f32().max(0.0);
            }
            EnvironmentProp::FogFalloffMode => {
                let next = match value.as_str() {
                    "Linear" => Some(FogFalloffDef::Linear { start: 60.0, end: 400.0 }),
                    "Exponential" => Some(FogFalloffDef::Exponential { density: 0.01 }),
                    "ExponentialSquared" | "Exponential Squared" => {
                        Some(FogFalloffDef::ExponentialSquared { density: 0.01 })
                    }
                    "Atmospheric" => Some(FogFalloffDef::Atmospheric {
                        extinction: [0.01, 0.01, 0.01],
                        inscattering: [0.01, 0.01, 0.01],
                    }),
                    _ => None,
                };
                if let Some(next) = next
                    && std::mem::discriminant(&self.env.fog.falloff) != std::mem::discriminant(&next)
                {
                    self.env.fog.falloff = next;
                }
            }
            EnvironmentProp::FogLinearStart => {
                if let FogFalloffDef::Linear { start, .. } = &mut self.env.fog.falloff {
                    *start = value.as_f32().max(0.0);
                }
            }
            EnvironmentProp::FogLinearEnd => {
                if let FogFalloffDef::Linear { end, .. } = &mut self.env.fog.falloff {
                    *end = value.as_f32().max(0.0);
                }
            }
            EnvironmentProp::FogDensity => match &mut self.env.fog.falloff {
                FogFalloffDef::Exponential { density } | FogFalloffDef::ExponentialSquared { density } => {
                    *density = value.as_f32().max(0.0);
                }
                _ => {}
            },
            EnvironmentProp::FogAtmosphericExtinction => {
                if let FogFalloffDef::Atmospheric { extinction, .. } = &mut self.env.fog.falloff {
                    let [r, g, b] = as_vec3(value);
                    *extinction = [r.max(0.0), g.max(0.0), b.max(0.0)];
                }
            }
            EnvironmentProp::FogAtmosphericInscattering => {
                if let FogFalloffDef::Atmospheric { inscattering, .. } = &mut self.env.fog.falloff {
                    let [r, g, b] = as_vec3(value);
                    *inscattering = [r.max(0.0), g.max(0.0), b.max(0.0)];
                }
            }
        }
        let changed = self.env != before;
        if changed {
            let merged = merge.is_some();
            self.history.record(Some(before), merge);
            self.revision += 1;
            if self.easy && !fits_easy(&self.env) {
                self.easy = false;
                self.layout += 1;
            } else if !merged {
                self.layout += 1;
            }
        }
        changed
    }

    /// Returns the current [`PropValue`] of `prop` in `self.env`.
    pub fn value(&self, prop: EnvironmentProp) -> PropValue {
        match prop {
            EnvironmentProp::ClearColor => PropValue::Float4(self.env.clear_color),
            EnvironmentProp::Cubemap => PropValue::Str(self.env.cubemap.clone().unwrap_or_default()),
            EnvironmentProp::SkyboxBrightness => PropValue::Float(self.env.skybox_brightness),
            EnvironmentProp::SkyboxRotation => PropValue::Float(self.env.skybox_rotation),
            EnvironmentProp::EnvIntensity => PropValue::Float(self.env.env_intensity),
            EnvironmentProp::SkyTopColor => PropValue::Float4(self.env.sky_top_color),
            EnvironmentProp::SkyHorizonColor => PropValue::Float4(self.env.sky_horizon_color),
            EnvironmentProp::GroundColor => PropValue::Float4(self.env.ground_color),
            EnvironmentProp::EnvAffectsLightmapped => PropValue::Bool(self.env.env_affects_lightmapped),
            EnvironmentProp::AmbientColor => PropValue::Float4(self.env.ambient_color),
            EnvironmentProp::AmbientBrightness => PropValue::Float(self.env.ambient_brightness),
            EnvironmentProp::AmbientAffectsLightmapped => PropValue::Bool(self.env.ambient_affects_lightmapped),
            EnvironmentProp::LightName(i) => {
                PropValue::Str(self.env.lights.get(i).map(|l| l.name.clone()).unwrap_or_default())
            }
            EnvironmentProp::LightColor(i) => PropValue::Float4(self.env.lights.get(i).map_or([1.0; 4], |l| l.color)),
            EnvironmentProp::LightIlluminance(i) => {
                PropValue::Float(self.env.lights.get(i).map_or(0.0, |l| l.illuminance))
            }
            EnvironmentProp::LightAzimuth(i) => PropValue::Float(self.env.lights.get(i).map_or(0.0, |l| l.azimuth_deg)),
            EnvironmentProp::LightElevation(i) => {
                PropValue::Float(self.env.lights.get(i).map_or(0.0, |l| l.elevation_deg))
            }
            EnvironmentProp::LightShadowMaps(i) => {
                PropValue::Bool(self.env.lights.get(i).is_some_and(|l| l.shadow_maps_enabled))
            }
            EnvironmentProp::LightContactShadows(i) => {
                PropValue::Bool(self.env.lights.get(i).is_some_and(|l| l.contact_shadows_enabled))
            }
            EnvironmentProp::LightShadowDepthBias(i) => {
                PropValue::Float(self.env.lights.get(i).map_or(0.0, |l| l.shadow_depth_bias))
            }
            EnvironmentProp::LightShadowNormalBias(i) => {
                PropValue::Float(self.env.lights.get(i).map_or(0.0, |l| l.shadow_normal_bias))
            }
            EnvironmentProp::LightAffectsLightmapped(i) => {
                PropValue::Bool(self.env.lights.get(i).is_some_and(|l| l.affects_lightmapped_mesh_diffuse))
            }
            EnvironmentProp::Ev100 => PropValue::Float(self.env.ev100),
            EnvironmentProp::Tonemapping => PropValue::Str(self.env.tonemapping.as_str().to_string()),
            EnvironmentProp::DebandDither => PropValue::Bool(self.env.deband_dither),
            EnvironmentProp::CgGlobalExposure => PropValue::Float(self.env.color_grading.global.exposure),
            EnvironmentProp::CgGlobalTemperature => PropValue::Float(self.env.color_grading.global.temperature),
            EnvironmentProp::CgGlobalTint => PropValue::Float(self.env.color_grading.global.tint),
            EnvironmentProp::CgGlobalHue => PropValue::Float(self.env.color_grading.global.hue),
            EnvironmentProp::CgGlobalPostSaturation => PropValue::Float(self.env.color_grading.global.post_saturation),
            EnvironmentProp::CgGlobalMidtonesRange => PropValue::Float2(self.env.color_grading.global.midtones_range),
            EnvironmentProp::CgSectionSaturation(s) => {
                PropValue::Float(cg_section(&self.env.color_grading, s).saturation)
            }
            EnvironmentProp::CgSectionContrast(s) => PropValue::Float(cg_section(&self.env.color_grading, s).contrast),
            EnvironmentProp::CgSectionGamma(s) => PropValue::Float(cg_section(&self.env.color_grading, s).gamma),
            EnvironmentProp::CgSectionGain(s) => PropValue::Float(cg_section(&self.env.color_grading, s).gain),
            EnvironmentProp::CgSectionLift(s) => PropValue::Float(cg_section(&self.env.color_grading, s).lift),
            EnvironmentProp::FogEnabled => PropValue::Bool(self.env.fog.enabled),
            EnvironmentProp::FogColor => PropValue::Float4(self.env.fog.color),
            EnvironmentProp::FogDirectionalLightColor => PropValue::Float4(self.env.fog.directional_light_color),
            EnvironmentProp::FogDirectionalLightExponent => PropValue::Float(self.env.fog.directional_light_exponent),
            EnvironmentProp::FogFalloffMode => {
                let mode = match &self.env.fog.falloff {
                    FogFalloffDef::Linear { .. } => "Linear",
                    FogFalloffDef::Exponential { .. } => "Exponential",
                    FogFalloffDef::ExponentialSquared { .. } => "ExponentialSquared",
                    FogFalloffDef::Atmospheric { .. } => "Atmospheric",
                };
                PropValue::Str(mode.to_string())
            }
            EnvironmentProp::FogLinearStart => match &self.env.fog.falloff {
                FogFalloffDef::Linear { start, .. } => PropValue::Float(*start),
                _ => PropValue::Float(0.0),
            },
            EnvironmentProp::FogLinearEnd => match &self.env.fog.falloff {
                FogFalloffDef::Linear { end, .. } => PropValue::Float(*end),
                _ => PropValue::Float(0.0),
            },
            EnvironmentProp::FogDensity => match &self.env.fog.falloff {
                FogFalloffDef::Exponential { density } | FogFalloffDef::ExponentialSquared { density } => {
                    PropValue::Float(*density)
                }
                _ => PropValue::Float(0.0),
            },
            EnvironmentProp::FogAtmosphericExtinction => match &self.env.fog.falloff {
                FogFalloffDef::Atmospheric { extinction, .. } => PropValue::Float3(*extinction),
                _ => PropValue::Float3([0.0; 3]),
            },
            EnvironmentProp::FogAtmosphericInscattering => match &self.env.fog.falloff {
                FogFalloffDef::Atmospheric { inscattering, .. } => PropValue::Float3(*inscattering),
                _ => PropValue::Float3([0.0; 3]),
            },
        }
    }

    /// Returns the default [`PropValue`] for `prop`.
    pub fn default_value(&self, prop: EnvironmentProp) -> PropValue {
        let def = Environment::default();
        let light_def = |i: usize| -> DirectionalLightDef {
            crate::backend::default_lights().get(i).cloned().unwrap_or_else(|| DirectionalLightDef {
                name: format!("Light {}", i + 1),
                ..DirectionalLightDef::default()
            })
        };
        match prop {
            EnvironmentProp::ClearColor => PropValue::Float4(def.clear_color),
            EnvironmentProp::Cubemap => PropValue::Str(def.cubemap.unwrap_or_default()),
            EnvironmentProp::SkyboxBrightness => PropValue::Float(def.skybox_brightness),
            EnvironmentProp::SkyboxRotation => PropValue::Float(def.skybox_rotation),
            EnvironmentProp::EnvIntensity => PropValue::Float(def.env_intensity),
            EnvironmentProp::SkyTopColor => PropValue::Float4(def.sky_top_color),
            EnvironmentProp::SkyHorizonColor => PropValue::Float4(def.sky_horizon_color),
            EnvironmentProp::GroundColor => PropValue::Float4(def.ground_color),
            EnvironmentProp::EnvAffectsLightmapped => PropValue::Bool(def.env_affects_lightmapped),
            EnvironmentProp::AmbientColor => PropValue::Float4(def.ambient_color),
            EnvironmentProp::AmbientBrightness => PropValue::Float(def.ambient_brightness),
            EnvironmentProp::AmbientAffectsLightmapped => PropValue::Bool(def.ambient_affects_lightmapped),
            EnvironmentProp::LightName(i) => PropValue::Str(light_def(i).name),
            EnvironmentProp::LightColor(i) => PropValue::Float4(light_def(i).color),
            EnvironmentProp::LightIlluminance(i) => PropValue::Float(light_def(i).illuminance),
            EnvironmentProp::LightAzimuth(i) => PropValue::Float(light_def(i).azimuth_deg),
            EnvironmentProp::LightElevation(i) => PropValue::Float(light_def(i).elevation_deg),
            EnvironmentProp::LightShadowMaps(i) => PropValue::Bool(light_def(i).shadow_maps_enabled),
            EnvironmentProp::LightContactShadows(i) => PropValue::Bool(light_def(i).contact_shadows_enabled),
            EnvironmentProp::LightShadowDepthBias(i) => PropValue::Float(light_def(i).shadow_depth_bias),
            EnvironmentProp::LightShadowNormalBias(i) => PropValue::Float(light_def(i).shadow_normal_bias),
            EnvironmentProp::LightAffectsLightmapped(i) => {
                PropValue::Bool(light_def(i).affects_lightmapped_mesh_diffuse)
            }
            EnvironmentProp::Ev100 => PropValue::Float(def.ev100),
            EnvironmentProp::Tonemapping => PropValue::Str(def.tonemapping.as_str().to_string()),
            EnvironmentProp::DebandDither => PropValue::Bool(def.deband_dither),
            EnvironmentProp::CgGlobalExposure => PropValue::Float(def.color_grading.global.exposure),
            EnvironmentProp::CgGlobalTemperature => PropValue::Float(def.color_grading.global.temperature),
            EnvironmentProp::CgGlobalTint => PropValue::Float(def.color_grading.global.tint),
            EnvironmentProp::CgGlobalHue => PropValue::Float(def.color_grading.global.hue),
            EnvironmentProp::CgGlobalPostSaturation => PropValue::Float(def.color_grading.global.post_saturation),
            EnvironmentProp::CgGlobalMidtonesRange => PropValue::Float2(def.color_grading.global.midtones_range),
            EnvironmentProp::CgSectionSaturation(s) => PropValue::Float(cg_section(&def.color_grading, s).saturation),
            EnvironmentProp::CgSectionContrast(s) => PropValue::Float(cg_section(&def.color_grading, s).contrast),
            EnvironmentProp::CgSectionGamma(s) => PropValue::Float(cg_section(&def.color_grading, s).gamma),
            EnvironmentProp::CgSectionGain(s) => PropValue::Float(cg_section(&def.color_grading, s).gain),
            EnvironmentProp::CgSectionLift(s) => PropValue::Float(cg_section(&def.color_grading, s).lift),
            EnvironmentProp::FogEnabled => PropValue::Bool(def.fog.enabled),
            EnvironmentProp::FogColor => PropValue::Float4(def.fog.color),
            EnvironmentProp::FogDirectionalLightColor => PropValue::Float4(def.fog.directional_light_color),
            EnvironmentProp::FogDirectionalLightExponent => PropValue::Float(def.fog.directional_light_exponent),
            EnvironmentProp::FogFalloffMode => PropValue::Str("Linear".to_string()),
            EnvironmentProp::FogLinearStart => PropValue::Float(60.0),
            EnvironmentProp::FogLinearEnd => PropValue::Float(400.0),
            EnvironmentProp::FogDensity => PropValue::Float(0.01),
            EnvironmentProp::FogAtmosphericExtinction => PropValue::Float3([0.01, 0.01, 0.01]),
            EnvironmentProp::FogAtmosphericInscattering => PropValue::Float3([0.01, 0.01, 0.01]),
        }
    }

    /// Returns `true` when `prop` currently holds its default value.
    pub fn is_default(&self, prop: EnvironmentProp) -> bool {
        self.value(prop) == self.default_value(prop)
    }

    /// Replaces the environment with the file's: saved, history cleared.
    fn replace(&mut self, env: Environment) {
        self.saved = env.clone();
        self.env = env;
        if !fits_easy(&self.env) {
            self.easy = false;
        }
        self.history.clear();
        self.changed();
    }

    fn changed(&mut self) {
        self.revision += 1;
        self.layout += 1;
    }

    /// The Light Texture asset `from` was renamed to `to` (`None`: deleted) and the environment files on disk were
    /// rewritten: the edited and the saved environment follow, so the unsaved edits stay and the tab is not dirty for
    /// it, and so do the undo / redo states, so an undo never brings `from` back. `false` when neither the edited nor
    /// the saved environment showed `from`.
    pub fn retarget(&mut self, from: &str, to: Option<&str>) -> bool {
        let follow = |env: &mut Environment| {
            let hit = env.cubemap.as_deref() == Some(from);
            if hit {
                env.cubemap = to.map(str::to_string);
            }
            hit
        };
        self.history.for_each_snapshot(|env| {
            follow(env);
        });
        let hit = [follow(&mut self.env), follow(&mut self.saved)].contains(&true);
        if hit {
            self.changed();
        }
        hit
    }

    /// Whether the edited or the saved environment shows the Light Texture asset `path`.
    pub fn shows(&self, path: &str) -> bool {
        [&self.env, &self.saved].iter().any(|e| e.cubemap.as_deref() == Some(path))
    }
}

/// The asset kind's `open`: the environment `r` loaded from disk; `None` with an error dialog when it does not load.
pub(super) fn open(e: &mut Editor, r: &AssetRef) -> Option<Tab> {
    let project = e.project.as_ref()?;
    match load_environment(&project.root.join(r.source_rel())) {
        Ok(env) => Some(Tab::new(LIGHT, EnvironmentDoc::new(r.clone(), env))),
        Err(err) => {
            e.io_error(&format!("cannot open {}", r.source_rel()), err);
            None
        }
    }
}

fn load(project: &Project, r: &AssetRef) -> Result<Environment, String> {
    load_environment(&project.root.join(r.source_rel()))
}

impl TabDoc for EnvironmentDoc {
    fn apply(&mut self, ctx: &mut DocCtx, action: &TabAction) -> Handled {
        match action {
            TabAction::Undo | TabAction::Redo => {
                let done = match action {
                    TabAction::Undo => self.history.undo(&mut self.env),
                    _ => self.history.redo(&mut self.env),
                };
                if done {
                    if !fits_easy(&self.env) {
                        self.easy = false;
                    }
                    self.changed();
                    ctx.redraw_props = true;
                }
            }
            TabAction::SetValue { key, value, merge } => {
                let Some(prop) = key.downcast::<EnvironmentProp>() else { return Handled::No };
                let merge = merge.then(|| MergeKey::Prop(key.clone()));
                ctx.redraw_props |= self.set(*prop, value, merge);
            }
            TabAction::SetComponent { key, comp, value, is_final } => {
                let Some(prop) = key.downcast::<EnvironmentProp>() else { return Handled::No };
                let was_dirty = self.is_dirty();
                // `None`: a field sent its shown (rounded) text back on focus loss, which is not an edit.
                if let Some(new) = self.value(*prop).with_component(*comp, *value) {
                    let changed = self.set(*prop, &new, Some(MergeKey::Prop(key.clone())));
                    if *is_final || (changed && self.is_dirty() != was_dirty) {
                        ctx.redraw_props = true;
                    }
                    // A clamped value (a typed negative number): the field must show the stored one.
                    if *is_final && self.value(*prop) != new {
                        self.layout += 1;
                    }
                }
                if *is_final {
                    self.history.end_merge();
                }
            }
            TabAction::CommitInline => {
                self.history.end_merge();
                ctx.redraw_props = true;
            }
            TabAction::Doc(payload) => {
                let Some(action) = payload.downcast::<EnvironmentAction>() else { return Handled::No };
                match action {
                    EnvironmentAction::AddLight => {
                        self.history.record(Some(self.env.clone()), None);
                        let name = format!("Light {}", self.env.lights.len() + 1);
                        self.env.lights.push(DirectionalLightDef { name, ..DirectionalLightDef::default() });
                        self.changed();
                        ctx.redraw_props = true;
                    }
                    EnvironmentAction::RemoveLight(idx) => {
                        if *idx < self.env.lights.len() {
                            self.history.record(Some(self.env.clone()), None);
                            self.env.lights.remove(*idx);
                            self.changed();
                            ctx.redraw_props = true;
                        }
                    }
                    EnvironmentAction::SetEasy(true) => {
                        let before = self.env.clone();
                        apply_easy_defaults(&mut self.env);
                        let env_changed = self.env != before;
                        if env_changed {
                            self.history.record(Some(before), None);
                            self.revision += 1;
                        }
                        if !self.easy || env_changed {
                            self.easy = true;
                            self.layout += 1;
                            ctx.redraw_props = true;
                        }
                    }
                    EnvironmentAction::SetEasy(false) => {
                        if self.easy {
                            apply_easy_defaults(&mut self.env);
                            self.easy = false;
                            self.layout += 1;
                            ctx.redraw_props = true;
                        }
                    }
                    EnvironmentAction::ResetProp(prop) => {
                        let def = self.default_value(*prop);
                        if self.set(*prop, &def, None) {
                            ctx.redraw_props = true;
                        }
                    }
                }
            }
            _ => return Handled::No,
        }
        Handled::Yes
    }

    fn current_value(&self, key: &PropKey, _ctx: &StatusCtx) -> Option<PropValue> {
        Some(self.value(*key.downcast::<EnvironmentProp>()?))
    }

    fn asset(&self) -> Option<&AssetRef> {
        Some(&self.aref)
    }

    fn view(&self) -> ViewKindId {
        ENVIRONMENT_VIEW
    }

    /// The source path without `.ron`.
    fn base_title(&self) -> String {
        self.aref.source_rel().trim_end_matches(".ron").to_string()
    }

    fn icon(&self) -> IconRef {
        ENVIRONMENT_ICON
    }

    fn watermark(&self) -> Option<&'static str> {
        Some(ENVIRONMENT_SUFFIX)
    }

    fn is_dirty(&self) -> bool {
        self.env != self.saved
    }

    fn is_read_only(&self) -> bool {
        false
    }

    fn revision(&self) -> u64 {
        self.revision
    }

    fn save(&mut self, ctx: &mut DocCtx) -> std::io::Result<()> {
        let project = ctx.project.ok_or_else(|| std::io::Error::other("no project is open"))?;
        project.ensure_layout()?;
        save_environment(&project.root.join(self.aref.source_rel()), &self.env)?;
        self.saved = self.env.clone();
        self.revision += 1;
        Ok(())
    }

    fn changed_on_disk(&self, project: &Project) -> bool {
        load(project, &self.aref).is_ok_and(|env| env != self.saved)
    }

    fn reload_from_disk(&mut self, ctx: &mut DocCtx) -> Result<(), String> {
        let project = ctx.project.ok_or("no project is open")?;
        self.replace(load(project, &self.aref)?);
        Ok(())
    }

    fn keep_mine(&mut self, project: &Project) {
        if let Ok(on_disk) = load(project, &self.aref) {
            self.saved = on_disk;
        }
    }

    fn rename(&mut self, _project: &Project, from: &AssetRef, to: &AssetRef) {
        if self.aref == *from {
            self.aref = to.clone();
            self.changed();
        }
    }

    fn config_key(&self) -> Option<String> {
        Some(self.aref.source_rel())
    }

    fn status(&self, ctx: &StatusCtx) -> Option<StatusInfo> {
        let counts = vec![plural(self.env.lights.len(), "directional light", "directional lights")];
        Some(StatusInfo::new(self.is_dirty(), ctx, counts))
    }

    fn preview_handle(&self) -> Option<&PreviewHandle> {
        Some(&self.preview)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc() -> EnvironmentDoc {
        EnvironmentDoc::new(crate::backend::environment_ref("day"), Environment::default())
    }

    fn apply(doc: &mut EnvironmentDoc, action: TabAction) -> Handled {
        let (mut state, mut messages) = ((), Vec::new());
        doc.apply(&mut DocCtx::new(None, &mut state, &mut messages), &action)
    }

    fn drag(prop: EnvironmentProp, value: f32, is_final: bool) -> TabAction {
        TabAction::SetComponent { key: PropKey::new(prop), comp: 0, value, is_final }
    }

    /// A drag is one undo step and keeps the form; the end of it, undo and a picked cube map rebuild it.
    #[test]
    fn drags_merge_and_keep_the_form() {
        let mut d = doc();
        let layout = d.layout;
        for v in [1.0, 2.0, 3.0] {
            assert_eq!(apply(&mut d, drag(EnvironmentProp::EnvIntensity, v, false)), Handled::Yes);
        }
        apply(&mut d, drag(EnvironmentProp::EnvIntensity, 4.0, true));
        assert_eq!((d.env.env_intensity, d.layout), (4.0, layout));
        apply(&mut d, TabAction::Undo);
        assert_eq!(d.env.env_intensity, 2500.0);
        assert!(d.layout > layout && !d.is_dirty());
        let pick = TabAction::SetValue {
            key: PropKey::new(EnvironmentProp::Cubemap),
            value: PropValue::Str("sky".into()),
            merge: false,
        };
        apply(&mut d, pick);
        assert_eq!(d.env.cubemap.as_deref(), Some("sky"));
        assert_eq!(
            d.current_value(&PropKey::new(EnvironmentProp::Cubemap), &StatusCtx::default()),
            Some(PropValue::Str("sky".into()))
        );
        // Another kind's key is not this document's.
        let other = TabAction::SetValue { key: PropKey::default(), value: PropValue::Float(1.0), merge: false };
        assert_eq!(apply(&mut d, other), Handled::No);
    }

    /// A typed negative number is stored as 0 and the form is rebuilt to show it.
    #[test]
    fn clamped_value_rebuilds_the_form() {
        let mut d = doc();
        let layout = d.layout;
        apply(&mut d, drag(EnvironmentProp::EnvIntensity, -2.0, true));
        assert_eq!(d.env.env_intensity, 0.0);
        assert!(d.layout > layout, "the field shows 0, not -2");
        let layout = d.layout;
        apply(&mut d, drag(EnvironmentProp::EnvIntensity, 3.0, true));
        assert_eq!((d.env.env_intensity, d.layout), (3.0, layout), "an accepted value keeps the form");
    }

    /// A renamed cube map is followed in the edited and the saved environment: the tab stays as dirty as it was.
    #[test]
    fn retarget_keeps_the_dirty_state() {
        let mut d = EnvironmentDoc::new(
            crate::backend::environment_ref("day"),
            Environment { cubemap: Some("sky".into()), ..Environment::default() },
        );
        assert!(d.retarget("sky", Some("blue")));
        assert!(!d.is_dirty() && d.shows("blue"));
        apply(&mut d, drag(EnvironmentProp::SkyboxBrightness, 5.0, true));
        assert!(d.retarget("blue", None));
        assert_eq!(d.env.cubemap, None);
        assert!(d.is_dirty(), "the brightness edit stays");
        assert!(!d.retarget("blue", None));
        apply(&mut d, TabAction::Undo);
        assert_eq!((d.env.skybox_brightness, d.env.cubemap.as_deref()), (2500.0, None), "the undo state follows too");
        assert!(!d.is_dirty());
    }

    /// Adding and removing directional lights are undoable steps, and all new property groups edit and clamp properly.
    #[test]
    fn directional_lights_and_fog_modes_edit_and_undo() {
        let mut d = doc();
        assert_eq!(d.env.lights.len(), 3);
        assert_eq!(apply(&mut d, EnvironmentAction::AddLight.into()), Handled::Yes);
        assert_eq!(d.env.lights.len(), 4);
        assert_eq!(d.env.lights[3].name, "Light 4");
        apply(&mut d, drag(EnvironmentProp::LightElevation(3), 120.0, true));
        assert_eq!(d.env.lights[3].elevation_deg, 90.0);
        assert_eq!(apply(&mut d, EnvironmentAction::RemoveLight(1).into()), Handled::Yes);
        assert_eq!(d.env.lights.len(), 3);
        apply(&mut d, TabAction::Undo);
        assert_eq!(d.env.lights.len(), 4);
        apply(&mut d, TabAction::Undo);
        assert_eq!(d.env.lights[3].elevation_deg, 50.0);
        apply(&mut d, TabAction::Undo);
        assert_eq!(d.env.lights.len(), 3);

        // Fog mode switch and parameter edits
        apply(
            &mut d,
            TabAction::SetValue {
                key: PropKey::new(EnvironmentProp::FogFalloffMode),
                value: PropValue::Str("Atmospheric".into()),
                merge: false,
            },
        );
        assert!(matches!(d.env.fog.falloff, FogFalloffDef::Atmospheric { .. }));
        apply(
            &mut d,
            TabAction::SetComponent {
                key: PropKey::new(EnvironmentProp::FogAtmosphericExtinction),
                comp: 1,
                value: 0.05,
                is_final: true,
            },
        );
        assert_eq!(
            d.current_value(&PropKey::new(EnvironmentProp::FogAtmosphericExtinction), &StatusCtx::default()),
            Some(PropValue::Float3([0.01, 0.05, 0.01]))
        );
    }
}
