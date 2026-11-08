//! Unit and headless Bevy integration tests for the generated `quillshade::light` runtime module.

use std::collections::BTreeMap;
use std::time::Duration;

use bevy::camera::{Camera3d, ClearColor, Exposure, Hdr};
use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::curve::Curve as BevyCurve;
use bevy::ecs::message::Messages;
use bevy::ecs::system::RunSystemOnce;
use bevy::light::{DirectionalLight, GeneratedEnvironmentMapLight, GlobalAmbientLight};
use bevy::pbr::DistanceFog;
use bevy::prelude::*;
use bevy::render::view::ColorGrading;
use bevy::state::prelude::{OnEnter, OnExit, State as BevyState};
use bevy::time::{TimePlugin, TimeUpdateStrategy, Virtual};

#[allow(dead_code)]
mod common {
    pub mod curve {
        include!("../../project_core/templates/common/curve.rs");
    }
    pub mod gradient {
        include!("../../project_core/templates/common/gradient.rs");
        include!("../../project_core/templates/common/gradient_bevy.rs");
    }
    pub use curve::{CurveData, Interp};
    pub use gradient::{GradientData, GradientStop};
}

#[allow(dead_code)]
mod generated_light {
    include!("../src/backend/templates/light_mod.rs");

    pub const BLENDED_SKYBOX_WESL: &str = include_str!("../src/backend/templates/blended_skybox.wesl");

    pub const ENVIRONMENTS: &[&str] = &["day", "night", "outdoor/sunset"];
    pub const CURVES: &[&str] = &["ease_in_out", "fast_start"];

    pub const COMPILED_MANIFEST: &str =
        "BakedLightManifest(\n  format_version: 1,\n  textures: {},\n  environments: {},\n  curves: {},\n)\n";

    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
    pub enum Preset {
        #[default]
        Day,
        Night,
        OutdoorSunset,
    }

    impl Preset {
        pub const ALL: &'static [Preset] = &[Self::Day, Self::Night, Self::OutdoorSunset];

        pub const fn as_str(self) -> &'static str {
            match self {
                Self::Day => "day",
                Self::Night => "night",
                Self::OutdoorSunset => "outdoor/sunset",
            }
        }

        pub fn from_path(path: &str) -> Option<Preset> {
            match path {
                "day" => Some(Self::Day),
                "night" => Some(Self::Night),
                "outdoor/sunset" => Some(Self::OutdoorSunset),
                _ => None,
            }
        }

        pub const fn state(self) -> State {
            match self {
                Self::Day => State::Day,
                Self::Night => State::Night,
                Self::OutdoorSunset => State::OutdoorSunset,
            }
        }

        pub fn default_values(self) -> EnvironmentValues {
            match self {
                Self::Day => EnvironmentValues {
                    clear_color: [0.6, 0.7, 0.9, 1.0],
                    cubemap: Some("textures/sky_day".into()),
                    skybox_brightness: 2500.0,
                    skybox_rotation: 10.0,
                    env_intensity: 2500.0,
                    ambient_brightness: 300.0,
                    ev100: 13.0,
                    tonemapping: TonemappingMode::TonyMcMapface,
                    deband_dither: true,
                    lights: default_lights(),
                    fog: FogDef {
                        enabled: true,
                        color: [0.6, 0.7, 0.9, 1.0],
                        directional_light_color: [1.0, 0.9, 0.8, 0.5],
                        directional_light_exponent: 8.0,
                        falloff: FogFalloffDef::Linear { start: 60.0, end: 400.0 },
                    },
                    ..EnvironmentValues::default()
                },
                Self::Night => EnvironmentValues {
                    clear_color: [0.02, 0.03, 0.06, 1.0],
                    cubemap: None,
                    skybox_brightness: 100.0,
                    skybox_rotation: 350.0,
                    env_intensity: 150.0,
                    ambient_brightness: 20.0,
                    ev100: 7.0,
                    tonemapping: TonemappingMode::AgX,
                    deband_dither: false,
                    lights: vec![DirectionalLightDef {
                        name: "Moon".into(),
                        color: [0.6, 0.7, 1.0, 1.0],
                        illuminance: 200.0,
                        azimuth_deg: 200.0,
                        elevation_deg: 40.0,
                        shadow_maps_enabled: true,
                        contact_shadows_enabled: false,
                        shadow_depth_bias: 0.02,
                        shadow_normal_bias: 1.8,
                        affects_lightmapped_mesh_diffuse: true,
                    }],
                    fog: FogDef {
                        enabled: false,
                        color: [0.02, 0.03, 0.06, 1.0],
                        directional_light_color: [0.0, 0.0, 0.0, 0.0],
                        directional_light_exponent: 8.0,
                        falloff: FogFalloffDef::Exponential { density: 0.02 },
                    },
                    ..EnvironmentValues::default()
                },
                Self::OutdoorSunset => EnvironmentValues {
                    clear_color: [0.9, 0.4, 0.2, 1.0],
                    cubemap: Some("textures/sky_sunset".into()),
                    skybox_brightness: 1200.0,
                    skybox_rotation: 90.0,
                    env_intensity: 1200.0,
                    ambient_brightness: 150.0,
                    ev100: 11.0,
                    tonemapping: TonemappingMode::AcesFitted,
                    deband_dither: true,
                    lights: vec![
                        DirectionalLightDef {
                            name: "Sun".into(),
                            color: [1.0, 0.5, 0.2, 1.0],
                            illuminance: 5_000.0,
                            azimuth_deg: 85.0,
                            elevation_deg: 10.0,
                            shadow_maps_enabled: true,
                            contact_shadows_enabled: true,
                            shadow_depth_bias: 0.03,
                            shadow_normal_bias: 2.0,
                            affects_lightmapped_mesh_diffuse: false,
                        },
                        DirectionalLightDef {
                            name: "Sky".into(),
                            color: [0.8, 0.4, 0.6, 1.0],
                            illuminance: 1_500.0,
                            azimuth_deg: 180.0,
                            elevation_deg: 75.0,
                            shadow_maps_enabled: false,
                            contact_shadows_enabled: false,
                            shadow_depth_bias: 0.02,
                            shadow_normal_bias: 1.8,
                            affects_lightmapped_mesh_diffuse: true,
                        },
                    ],
                    fog: FogDef {
                        enabled: true,
                        color: [0.9, 0.4, 0.2, 0.8],
                        directional_light_color: [1.0, 0.6, 0.3, 0.7],
                        directional_light_exponent: 12.0,
                        falloff: FogFalloffDef::Linear { start: 30.0, end: 250.0 },
                    },
                    ..EnvironmentValues::default()
                },
            }
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
    pub enum Curve {
        #[default]
        Linear,
        EaseInOut,
        FastStart,
    }

    impl Curve {
        pub const ALL: &'static [Curve] = &[Self::Linear, Self::EaseInOut, Self::FastStart];

        pub const fn as_str(self) -> &'static str {
            match self {
                Self::Linear => "linear",
                Self::EaseInOut => "ease_in_out",
                Self::FastStart => "fast_start",
            }
        }

        pub fn from_path(path: &str) -> Option<Curve> {
            match path {
                "linear" => Some(Self::Linear),
                "ease_in_out" => Some(Self::EaseInOut),
                "fast_start" => Some(Self::FastStart),
                _ => None,
            }
        }

        pub fn default_curve(self) -> CurveData {
            match self {
                Self::Linear => CurveData::linear(),
                Self::EaseInOut => CurveData {
                    interp: Interp::CatmullRom,
                    min: 0.0,
                    max: 1.0,
                    points: vec![(0.0, 0.0), (0.25, 0.1), (0.75, 0.9), (1.0, 1.0)],
                },
                Self::FastStart => CurveData {
                    interp: Interp::CatmullRom,
                    min: 0.0,
                    max: 1.0,
                    points: vec![(0.0, 0.0), (0.3, 0.8), (1.0, 1.0)],
                },
            }
        }

        pub fn sample(self, t: f32) -> f32 {
            match self {
                Self::Linear => t.clamp(0.0, 1.0),
                other => transition_sample(&other.default_curve(), t),
            }
        }
    }

    pub fn eval_transition_sample(c: &CurveData, t: f32) -> f32 {
        transition_sample(c, t)
    }

    impl bevy::curve::Curve<f32> for Curve {
        fn domain(&self) -> bevy::curve::Interval {
            bevy::curve::Interval::UNIT
        }

        fn sample_unchecked(&self, t: f32) -> f32 {
            (*self).sample(t)
        }
    }

    #[derive(States, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub enum State {
        #[default]
        None,
        Transitioning,
        Day,
        Night,
        OutdoorSunset,
    }

    impl From<Preset> for State {
        fn from(preset: Preset) -> Self {
            preset.state()
        }
    }

    impl State {
        pub const fn as_preset(self) -> Option<Preset> {
            match self {
                Self::None | Self::Transitioning => None,
                Self::Day => Some(Preset::Day),
                Self::Night => Some(Preset::Night),
                Self::OutdoorSunset => Some(Preset::OutdoorSunset),
            }
        }
    }
}

use common::{CurveData, Interp};
use generated_light::{
    BakedLightManifest, ColorGradingDef, ColorGradingGlobalDef, ColorGradingSectionDef, Curve, EnvironmentValues,
    FogDef, FogFalloffDef, Light, LightRuntime, LightTransitionFinished, LightTransitionStarted,
    ManagedDirectionalLight, Preset, QuillshadeLightInternalPlugin, State, TonemappingMode, eval_transition_sample,
    skybox::BlendedSkybox,
};

#[test]
fn codegen_emits_valid_manifest_and_module() {
    use light_domain::backend::{Environment, LightAssets, codegen};
    use texture_kit::TextureAsset;

    let mut assets = LightAssets::default();
    assets.textures.insert("textures/sky".into(), TextureAsset::new("textures/sky_cube.ktx2"));
    assets
        .environments
        .insert("day".into(), Environment { cubemap: Some("textures/sky".into()), ..Environment::default() });
    assets.environments.insert("outdoor/sunset".into(), Environment::default());
    assets.curves.insert(
        "ease_in_out".into(),
        curve_kit::Curve::new("ease_in_out", vec![(0.0, 0.0), (0.25, 0.1), (0.75, 0.9), (1.0, 1.0)]),
    );

    let manifest_ron = codegen::manifest(&assets);
    let parsed = BakedLightManifest::parse(&manifest_ron).expect("generated manifest parses");
    assert_eq!(parsed.format_version, 1);
    assert!(parsed.textures.contains_key("textures/sky"));
    assert!(parsed.environments.contains_key("day"));
    assert!(parsed.environments.contains_key("outdoor/sunset"));
    assert!(parsed.curves.contains_key("ease_in_out"));
    assert_eq!(parsed.curves["ease_in_out"].interp, Interp::CatmullRom);

    let module_rs = codegen::module(&assets);
    assert!(module_rs.contains("pub enum Preset {\n"));
    assert!(module_rs.contains("    #[default]\n    Day,\n"));
    assert!(module_rs.contains("    OutdoorSunset,\n"));
    assert!(module_rs.contains("pub enum Curve {\n"));
    assert!(module_rs.contains("    EaseInOut,\n"));
    assert!(module_rs.contains("pub fn default_curve(self) -> CurveData {\n"));
    assert!(module_rs.contains("Self::Linear => CurveData::linear(),\n"));
    assert!(module_rs.contains("CurveData { interp: Interp::CatmullRom, min: 0.0, max: 1.0, points: vec!["));
    assert!(module_rs.contains("other => transition_sample(&other.default_curve(), t),\n"));
    assert!(!module_rs.contains("BakedCurve"));
    assert!(module_rs.contains("pub enum State {\n"));
    assert!(module_rs.contains("    Transitioning,\n"));
}

#[test]
fn curve_sampling_matches_curve_kit_and_implements_bevy_curve() {
    assert_eq!(Curve::Linear.sample(0.0).to_bits(), 0.0_f32.to_bits());
    assert_eq!(Curve::Linear.sample(0.37).to_bits(), 0.37_f32.to_bits());
    assert_eq!(Curve::Linear.sample(1.0).to_bits(), 1.0_f32.to_bits());
    assert_eq!(BevyCurve::<f32>::sample_clamped(&Curve::Linear, 0.65).to_bits(), 0.65_f32.to_bits());

    let ref_curve = curve_kit::Curve::new("fast_start", vec![(0.0, 0.0), (0.3, 0.8), (1.0, 1.0)]);
    let curve_data = Curve::FastStart.default_curve();
    let (lo, hi) = (ref_curve.min.min(ref_curve.max), ref_curve.min.max(ref_curve.max));
    for i in -5..=25 {
        let t = i as f32 / 20.0;
        let curve_kit_sample = ref_curve.sample(t, curve_kit::Interp::CatmullRom);
        let expected_clamped = curve_kit_sample.max(lo).min(hi);
        let actual = Curve::FastStart.sample(t);
        assert_eq!(actual.to_bits(), expected_clamped.to_bits(), "t={t}: actual={actual}, expected={expected_clamped}");
        assert_eq!(actual.to_bits(), eval_transition_sample(&curve_data, t).to_bits(), "t={t}");
        if (0.0..=1.0).contains(&t) {
            let via_trait = BevyCurve::<f32>::sample_clamped(&Curve::FastStart, t);
            assert_eq!(via_trait.to_bits(), actual.to_bits(), "t={t}");
        }
    }
}

#[test]
fn environment_values_interpolation_covers_all_fields_and_modes() {
    let mut from = Preset::Day.default_values();
    from.env_affects_lightmapped = true;
    from.ambient_affects_lightmapped = true;
    from.color_grading = ColorGradingDef {
        global: ColorGradingGlobalDef {
            exposure: 0.0,
            temperature: -0.2,
            tint: 0.1,
            hue: 0.0,
            post_saturation: 1.0,
            midtones_range: [0.2, 0.7],
        },
        shadows: ColorGradingSectionDef { saturation: 1.0, contrast: 1.0, gamma: 1.0, gain: 1.0, lift: 0.0 },
        midtones: ColorGradingSectionDef::default(),
        highlights: ColorGradingSectionDef::default(),
    };

    let mut to = Preset::Night.default_values();
    to.env_affects_lightmapped = false;
    to.ambient_affects_lightmapped = false;
    to.color_grading = ColorGradingDef {
        global: ColorGradingGlobalDef {
            exposure: 1.0,
            temperature: 0.4,
            tint: -0.1,
            hue: 0.2,
            post_saturation: 0.8,
            midtones_range: [0.3, 0.8],
        },
        shadows: ColorGradingSectionDef { saturation: 0.5, contrast: 1.2, gamma: 0.9, gain: 0.8, lift: -0.1 },
        midtones: ColorGradingSectionDef::default(),
        highlights: ColorGradingSectionDef::default(),
    };

    // Mid-transition (raw_t = 0.5, s = 0.5):
    let mid = EnvironmentValues::interpolate(&from, &to, 0.5, 0.5);
    assert!((mid.ev100 - 10.0).abs() < 1e-5);
    assert!((mid.ambient_brightness - 160.0).abs() < 1e-5);
    // Shortest-angle wrap between 10 deg and 350 deg is 0 deg!
    assert!(mid.skybox_rotation.abs() < 1e-4 || (mid.skybox_rotation - 360.0).abs() < 1e-4);
    assert!((mid.color_grading.global.exposure - 0.5).abs() < 1e-5);
    assert!((mid.color_grading.shadows.saturation - 0.75).abs() < 1e-5);
    // Binary fields stay on `from` while raw_t < 1.0:
    assert_eq!(mid.tonemapping, TonemappingMode::TonyMcMapface);
    assert!(mid.deband_dither);
    assert!(mid.env_affects_lightmapped);
    assert!(mid.ambient_affects_lightmapped);
    // Directional lights: `from` has 3, `to` has 1 -> mid has 3 slots, slots 1 and 2 have halved illuminance:
    assert_eq!(mid.lights.len(), 3);
    assert!((mid.lights[1].illuminance - 1500.0).abs() < 1e-3);
    assert!((mid.lights[2].illuminance - 1250.0).abs() < 1e-3);
    // Skybox blend: Some("textures/sky_day") -> None blends cubemap_a toward to.clear_color with alpha = 1.0:
    let sky_mid = mid.effective_skybox_blend();
    assert_eq!(sky_mid.cubemap_a.as_deref(), Some("textures/sky_day"));
    assert_eq!(sky_mid.cubemap_b, None);
    assert_eq!(sky_mid.color_a, from.clear_color);
    assert_eq!(sky_mid.color_b, to.clear_color);
    assert!((sky_mid.blend - 0.5).abs() < 1e-5);
    assert!((sky_mid.alpha - 1.0).abs() < 1e-5);
    assert_eq!(mid.clear_color, to.clear_color);

    // Completion (raw_t = 1.0, s = 1.0):
    let done = EnvironmentValues::interpolate(&from, &to, 1.0, 1.0);
    assert_eq!(done.tonemapping, TonemappingMode::AgX);
    assert!(!done.deband_dither);
    assert!(!done.env_affects_lightmapped);
    assert!(!done.ambient_affects_lightmapped);
    assert_eq!(done.lights.len(), 1);
    assert_eq!(done.lights[0].name, "Moon");
    assert!(!done.fog.enabled);
    assert_eq!(done.effective_skybox_blend().alpha, 0.0);

    // Extra lights in `to` (1 -> 3 lights) ramp up from 0.0:
    let rev_start = EnvironmentValues::interpolate(&to, &from, 0.0, 0.0);
    assert_eq!(rev_start.lights.len(), 3);
    assert_eq!(rev_start.lights[1].illuminance, 0.0);
    assert_eq!(rev_start.lights[2].illuminance, 0.0);
}

#[test]
fn interpolate_skybox_blend_states() {
    let a_color = EnvironmentValues {
        clear_color: [0.8, 0.85, 0.9, 1.0],
        cubemap: None,
        skybox_brightness: 100.0,
        skybox_rotation: 15.0,
        ..EnvironmentValues::default()
    };
    let b_cubemap = EnvironmentValues {
        clear_color: [0.02, 0.02, 0.05, 1.0],
        cubemap: Some("sky_b".into()),
        skybox_brightness: 2000.0,
        skybox_rotation: 45.0,
        ..EnvironmentValues::default()
    };
    let a_cubemap = EnvironmentValues {
        clear_color: [0.05, 0.05, 0.1, 1.0],
        cubemap: Some("sky_a".into()),
        skybox_brightness: 1500.0,
        skybox_rotation: 20.0,
        ..EnvironmentValues::default()
    };

    // None -> Some("sky_b") at s = 0.3:
    let mid_none_to_some = EnvironmentValues::interpolate(&a_color, &b_cubemap, 0.3, 0.3);
    assert_eq!(mid_none_to_some.clear_color, a_color.clear_color);
    let sky = mid_none_to_some.effective_skybox_blend();
    assert_eq!(sky.cubemap_a, None);
    assert_eq!(sky.cubemap_b.as_deref(), Some("sky_b"));
    assert_eq!(sky.color_a, a_color.clear_color);
    assert_eq!(sky.color_b, b_cubemap.clear_color);
    assert!((sky.blend - 0.3).abs() < 1e-6);
    assert!((sky.alpha - 1.0).abs() < 1e-6);

    // None -> Some("sky_b") at raw_t = 1.0:
    let done_none_to_some = EnvironmentValues::interpolate(&a_color, &b_cubemap, 1.0, 1.0);
    assert_eq!(done_none_to_some.clear_color, b_cubemap.clear_color);
    let sky_done = done_none_to_some.effective_skybox_blend();
    assert_eq!(sky_done.cubemap_a.as_deref(), Some("sky_b"));
    assert_eq!(sky_done.cubemap_b.as_deref(), Some("sky_b"));
    assert!((sky_done.alpha - 1.0).abs() < 1e-6);

    // Some("sky_a") -> None at s = 0.3:
    let mid_some_to_none = EnvironmentValues::interpolate(&a_cubemap, &a_color, 0.3, 0.3);
    assert_eq!(mid_some_to_none.clear_color, a_color.clear_color);
    let sky = mid_some_to_none.effective_skybox_blend();
    assert_eq!(sky.cubemap_a.as_deref(), Some("sky_a"));
    assert_eq!(sky.cubemap_b, None);
    assert_eq!(sky.color_a, a_cubemap.clear_color);
    assert_eq!(sky.color_b, a_color.clear_color);
    assert!((sky.blend - 0.3).abs() < 1e-6);
    assert!((sky.alpha - 1.0).abs() < 1e-6);

    // Some("sky_a") -> None at raw_t = 1.0:
    let done_some_to_none = EnvironmentValues::interpolate(&a_cubemap, &a_color, 1.0, 1.0);
    assert_eq!(done_some_to_none.clear_color, a_color.clear_color);
    assert_eq!(done_some_to_none.effective_skybox_blend().alpha, 0.0);
}

#[test]
fn fog_cross_mode_interpolation() {
    let exp = FogDef {
        enabled: true,
        color: [0.5, 0.5, 0.5, 0.8],
        directional_light_color: [0.0; 4],
        directional_light_exponent: 8.0,
        falloff: FogFalloffDef::Exponential { density: 0.02 },
    };
    let atmo = FogDef {
        enabled: true,
        color: [0.7, 0.7, 0.7, 1.0],
        directional_light_color: [0.0; 4],
        directional_light_exponent: 8.0,
        falloff: FogFalloffDef::Atmospheric { extinction: [0.06, 0.06, 0.06], inscattering: [0.04, 0.04, 0.04] },
    };

    // Exponential <-> Atmospheric promotes Exponential to Atmospheric during 0 <= t < 1:
    let mid = FogDef::interpolate(&exp, &atmo, 0.5, 0.5);
    assert_eq!(
        mid.falloff,
        FogFalloffDef::Atmospheric { extinction: [0.04, 0.04, 0.04], inscattering: [0.03, 0.03, 0.03] }
    );
    let end = FogDef::interpolate(&atmo, &exp, 1.0, 1.0);
    assert_eq!(end.falloff, FogFalloffDef::Exponential { density: 0.02 });

    // Different curve families (Linear <-> ExponentialSquared) fade alpha to 0 at s = 0.5 and switch:
    let lin = FogDef {
        enabled: true,
        color: [0.5, 0.6, 0.7, 0.8],
        directional_light_color: [0.0; 4],
        directional_light_exponent: 8.0,
        falloff: FogFalloffDef::Linear { start: 50.0, end: 300.0 },
    };
    let exp2 = FogDef {
        enabled: true,
        color: [0.2, 0.3, 0.4, 0.6],
        directional_light_color: [0.0; 4],
        directional_light_exponent: 8.0,
        falloff: FogFalloffDef::ExponentialSquared { density: 0.05 },
    };
    let q1 = FogDef::interpolate(&lin, &exp2, 0.25, 0.25);
    assert!(matches!(q1.falloff, FogFalloffDef::Linear { .. }));
    assert!((q1.color[3] - 0.4).abs() < 1e-5);

    let half = FogDef::interpolate(&lin, &exp2, 0.5, 0.5);
    assert!(matches!(half.falloff, FogFalloffDef::ExponentialSquared { .. }));
    assert!(half.color[3].abs() < 1e-5);

    let q3 = FogDef::interpolate(&lin, &exp2, 0.75, 0.75);
    assert!(matches!(q3.falloff, FogFalloffDef::ExponentialSquared { .. }));
    assert!((q3.color[3] - 0.3).abs() < 1e-5);
}

#[derive(Resource, Default, Debug)]
struct TransitionLog {
    events: Vec<String>,
}

fn test_app() -> App {
    let mut app = App::new();
    app.add_plugins((TimePlugin, QuillshadeLightInternalPlugin))
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO))
        .init_resource::<TransitionLog>();
    app.world_mut().resource_mut::<Time<Virtual>>().set_max_delta(Duration::MAX);

    app.add_systems(OnExit(State::Day), |mut log: ResMut<TransitionLog>| {
        log.events.push("exit:day".into());
    })
    .add_systems(OnEnter(State::Day), |mut log: ResMut<TransitionLog>| {
        log.events.push("enter:day".into());
    })
    .add_systems(OnExit(State::OutdoorSunset), |mut log: ResMut<TransitionLog>| {
        log.events.push("exit:sunset".into());
    })
    .add_systems(OnEnter(State::OutdoorSunset), |mut log: ResMut<TransitionLog>| {
        log.events.push("enter:sunset".into());
    })
    .add_systems(OnEnter(State::Night), |mut log: ResMut<TransitionLog>| {
        log.events.push("enter:night".into());
    });

    app.world_mut().spawn(Camera3d::default());
    app.update();
    app
}

fn step(app: &mut App, dt: Duration) {
    app.insert_resource(TimeUpdateStrategy::ManualDuration(dt));
    app.update();
}

#[test]
fn headless_state_machine_and_world_synchronization() {
    let mut app = test_app();

    // Initial state is `State::None`; camera and lights untouched.
    assert_eq!(*app.world().resource::<BevyState<State>>().get(), State::None);
    assert!(app.world().get_resource::<ClearColor>().is_none());

    // `set_immediate(Preset::Day)` enters `State::Day` and populates camera + 3 directional lights.
    app.world_mut()
        .run_system_once(|mut light: Light| {
            light.set_immediate(Preset::Day);
        })
        .unwrap();
    step(&mut app, Duration::ZERO);

    assert_eq!(*app.world().resource::<BevyState<State>>().get(), State::Day);
    assert_eq!(app.world().resource::<TransitionLog>().events, vec!["enter:day"]);

    let mut cam_q = app.world_mut().query_filtered::<(
        &Exposure,
        &Tonemapping,
        &DebandDither,
        &ColorGrading,
        Option<&DistanceFog>,
        &GeneratedEnvironmentMapLight,
        &BlendedSkybox,
    ), (With<Camera3d>, With<Hdr>)>();
    let (exposure, tonemapping, deband, _cg, fog, ibl, skybox) = cam_q.single(app.world()).unwrap();
    assert_eq!(exposure.ev100, 13.0);
    assert_eq!(*tonemapping, Tonemapping::TonyMcMapface);
    assert_eq!(*deband, DebandDither::Enabled);
    assert!(fog.is_some());
    assert_eq!(ibl.intensity, 2500.0);
    assert_eq!(skybox.brightness_a, 2500.0);
    assert_eq!(ibl.rotation, skybox.rotation_a, "the IBL turns with the skybox");

    let light_count =
        app.world_mut().query::<(&ManagedDirectionalLight, &DirectionalLight)>().iter(app.world()).count();
    assert_eq!(light_count, 3);

    // Start transition Day -> OutdoorSunset over 2.0s:
    // Immediately at t = 0.0: OnExit(State::Day) fires, State becomes State::Transitioning!
    app.world_mut()
        .run_system_once(|mut light: Light| {
            light.transition_to(Preset::OutdoorSunset, Curve::EaseInOut, Duration::from_secs(2));
            assert_eq!(light.prev_preset(), Some(Preset::Day));
            assert_eq!(light.next_preset(), Some(Preset::OutdoorSunset));
            assert_eq!(light.current_preset(), None);
            assert!(light.is_transitioning());
            assert_eq!(light.progress(), 0.0);
        })
        .unwrap();
    step(&mut app, Duration::ZERO);

    assert_eq!(*app.world().resource::<BevyState<State>>().get(), State::Transitioning);
    assert_eq!(app.world().resource::<TransitionLog>().events, vec!["enter:day", "exit:day"]);

    let started: Vec<_> = app.world_mut().resource_mut::<Messages<LightTransitionStarted>>().drain().collect();
    assert_eq!(
        started,
        vec![LightTransitionStarted {
            from: Some(Preset::Day),
            to: Preset::OutdoorSunset,
            curve: Curve::EaseInOut,
            duration: Duration::from_secs(2),
        }]
    );

    // Advance 1.0s (t = 0.5): still transitioning, OnEnter(OutdoorSunset) has NOT fired yet.
    step(&mut app, Duration::from_secs(1));
    assert_eq!(*app.world().resource::<BevyState<State>>().get(), State::Transitioning);
    assert_eq!(app.world().resource::<TransitionLog>().events, vec!["enter:day", "exit:day"]);
    app.world_mut()
        .run_system_once(|light: Light| {
            assert!((light.progress() - 0.5).abs() < 1e-5);
            assert!((light.sampled_progress() - 0.5).abs() < 1e-2);
        })
        .unwrap();

    // Advance remaining 1.0s (t = 1.0): transition finishes, OnEnter(OutdoorSunset) fires, 3rd light despawned!
    step(&mut app, Duration::from_secs(1));
    assert_eq!(*app.world().resource::<BevyState<State>>().get(), State::OutdoorSunset);
    assert_eq!(app.world().resource::<TransitionLog>().events, vec!["enter:day", "exit:day", "enter:sunset"]);
    app.world_mut()
        .run_system_once(|light: Light| {
            assert_eq!(light.current_preset(), Some(Preset::OutdoorSunset));
            assert_eq!(light.prev_preset(), Some(Preset::OutdoorSunset));
            assert_eq!(light.next_preset(), None);
            assert!(!light.is_transitioning());
            assert_eq!(light.progress(), 1.0);
        })
        .unwrap();
    let light_count =
        app.world_mut().query::<(&ManagedDirectionalLight, &DirectionalLight)>().iter(app.world()).count();
    assert_eq!(light_count, 2);
}

#[test]
fn mid_transition_interruption_and_virtual_time_scaling() {
    let mut app = test_app();

    app.world_mut()
        .run_system_once(|mut light: Light| {
            light.set_immediate(Preset::Day);
        })
        .unwrap();
    step(&mut app, Duration::ZERO);

    // Start Day -> OutdoorSunset (2.0s linear) and advance 1.0s (t = 0.5, ev100 = 12.0):
    app.world_mut()
        .run_system_once(|mut light: Light| {
            light.transition_linear_to(Preset::OutdoorSunset, Duration::from_secs(2));
        })
        .unwrap();
    step(&mut app, Duration::from_secs(1));

    let mid_ev100 = app.world_mut().run_system_once(|light: Light| light.current_values().unwrap().ev100).unwrap();
    assert!((mid_ev100 - 12.0).abs() < 1e-5);

    // Interrupt mid-transition toward Preset::Night (ev100 = 7.0) over 4.0s:
    // `prev_preset()` must remain `Some(Preset::Day)` (the last settled preset), and `current_values().ev100` at t = 0
    // must seamlessly start from 12.0!
    app.world_mut()
        .run_system_once(|mut light: Light| {
            light.transition_linear_to(Preset::Night, Duration::from_secs(4));
            assert_eq!(light.prev_preset(), Some(Preset::Day));
            assert_eq!(light.next_preset(), Some(Preset::Night));
            assert_eq!(light.progress(), 0.0);
            assert!((light.current_values().unwrap().ev100 - 12.0).abs() < 1e-5);
        })
        .unwrap();

    // Pause virtual time (`relative_speed = 0.0`): advancing 5.0s wall-clock time does not advance the transition!
    app.world_mut().resource_mut::<Time<Virtual>>().set_relative_speed(0.0);
    step(&mut app, Duration::from_secs(5));
    app.world_mut()
        .run_system_once(|light: Light| {
            assert_eq!(light.progress(), 0.0);
            assert!(light.is_transitioning());
        })
        .unwrap();

    // Speed up virtual time (`relative_speed = 2.0`): 1.0s wall-clock advances 2.0s virtual time -> t = 0.5!
    app.world_mut().resource_mut::<Time<Virtual>>().set_relative_speed(2.0);
    step(&mut app, Duration::from_secs(1));
    app.world_mut()
        .run_system_once(|light: Light| {
            assert!((light.progress() - 0.5).abs() < 1e-5);
            // Halfway from 12.0 to 7.0 is 9.5:
            assert!((light.current_values().unwrap().ev100 - 9.5).abs() < 1e-5);
        })
        .unwrap();

    // Another 1.0s wall-clock (2.0s virtual) completes the 4.0s transition -> enters State::Night!
    step(&mut app, Duration::from_secs(1));
    assert_eq!(*app.world().resource::<BevyState<State>>().get(), State::Night);
    let finished: Vec<_> = app.world_mut().resource_mut::<Messages<LightTransitionFinished>>().drain().collect();
    assert_eq!(finished.last(), Some(&LightTransitionFinished { preset: Preset::Night }));
}

#[test]
fn manifest_hot_reload_updates_active_environment_in_place() {
    let mut app = test_app();

    app.world_mut()
        .run_system_once(|mut light: Light| {
            light.set_immediate(Preset::Day);
        })
        .unwrap();
    step(&mut app, Duration::ZERO);

    let mut updated_day = Preset::Day.default_values();
    updated_day.ev100 = 15.5;
    updated_day.ambient_brightness = 777.0;

    let manifest = BakedLightManifest {
        format_version: 1,
        textures: BTreeMap::new(),
        environments: BTreeMap::from([("day".to_string(), updated_day)]),
        curves: BTreeMap::from([(
            "ease_in_out".to_string(),
            CurveData { interp: Interp::CatmullRom, min: 0.0, max: 1.0, points: vec![(0.0, 0.0), (1.0, 1.0)] },
        )]),
    };

    app.world_mut().resource_mut::<LightRuntime>().apply_manifest_data(&manifest, None);
    step(&mut app, Duration::ZERO);

    assert_eq!(app.world().resource::<GlobalAmbientLight>().brightness, 777.0);
    let mut cam_q = app.world_mut().query_filtered::<&Exposure, With<Camera3d>>();
    assert_eq!(cam_q.single(app.world()).unwrap().ev100, 15.5);
}

#[test]
fn manifest_hot_reload_resamples_in_flight_transition() {
    let mut app = test_app();

    app.world_mut()
        .run_system_once(|mut light: Light| {
            light.set_immediate(Preset::Day);
        })
        .unwrap();
    step(&mut app, Duration::ZERO);

    // Start Day -> Night (2.0 s) on Curve::EaseInOut and step 0.6 s (raw_t = 0.3).
    app.world_mut()
        .run_system_once(|mut light: Light| {
            light.transition_to(Preset::Night, Curve::EaseInOut, Duration::from_secs(2));
        })
        .unwrap();
    step(&mut app, Duration::from_millis(600));

    let initial_s = Curve::EaseInOut.sample(0.3);
    let (raw_t_before, s_before) =
        app.world_mut().run_system_once(|light: Light| (light.progress(), light.sampled_progress())).unwrap();
    assert!((raw_t_before - 0.3).abs() < 1e-6);
    assert_eq!(s_before.to_bits(), initial_s.to_bits());
    let ambient_before = app.world().resource::<GlobalAmbientLight>().brightness;
    let mut cam_q = app.world_mut().query_filtered::<&Exposure, With<Camera3d>>();
    let ev100_before = cam_q.single(app.world()).unwrap().ev100;

    // Hot-reload `ease_in_out` with a steep curve that evaluates much closer to 1.0 at t = 0.3.
    let updated_curve = CurveData {
        interp: Interp::CatmullRom,
        min: 0.0,
        max: 1.0,
        points: vec![(0.0, 0.0), (0.25, 0.9), (0.75, 0.95), (1.0, 1.0)],
    };
    let expected_s = eval_transition_sample(&updated_curve, raw_t_before);
    assert!((expected_s - initial_s).abs() > 0.5, "expected_s={expected_s}, initial_s={initial_s}");

    let manifest = BakedLightManifest {
        format_version: 1,
        textures: BTreeMap::new(),
        environments: BTreeMap::new(),
        curves: BTreeMap::from([("ease_in_out".to_string(), updated_curve)]),
    };
    app.world_mut().resource_mut::<LightRuntime>().apply_manifest_data(&manifest, None);

    // `apply_manifest_data` immediately re-samples the in-flight transition in `LightRuntime`.
    let s_after = app.world_mut().run_system_once(|light: Light| light.sampled_progress()).unwrap();
    assert_eq!(s_after.to_bits(), expected_s.to_bits());

    // Flushing world synchronization updates the scene resources/components to the new interpolated values.
    step(&mut app, Duration::ZERO);
    let expected_values = EnvironmentValues::interpolate(
        &Preset::Day.default_values(),
        &Preset::Night.default_values(),
        raw_t_before,
        expected_s,
    );
    let ambient_after = app.world().resource::<GlobalAmbientLight>().brightness;
    let ev100_after = cam_q.single(app.world()).unwrap().ev100;
    assert_ne!(ambient_after.to_bits(), ambient_before.to_bits());
    assert_ne!(ev100_after.to_bits(), ev100_before.to_bits());
    assert_eq!(ambient_after.to_bits(), expected_values.ambient_brightness.to_bits());
    assert_eq!(ev100_after.to_bits(), expected_values.ev100.to_bits());

    // A subsequent manifest that omits `ease_in_out` preserves the previously loaded curve.
    let empty_manifest = BakedLightManifest {
        format_version: 1,
        textures: BTreeMap::new(),
        environments: BTreeMap::new(),
        curves: BTreeMap::new(),
    };
    app.world_mut().resource_mut::<LightRuntime>().apply_manifest_data(&empty_manifest, None);
    let s_kept = app.world_mut().run_system_once(|light: Light| light.sampled_progress()).unwrap();
    assert_eq!(s_kept.to_bits(), expected_s.to_bits());
}

#[test]
fn baked_manifest_curves_parse_as_curve_data() {
    let raw = include_str!("../../../examples/sample_game/assets/quillshade/light/autogen/baked.ron");
    let manifest = BakedLightManifest::parse(raw).expect("sample game's baked.ron parses as BakedLightManifest");
    assert!(!manifest.curves.is_empty(), "expected at least one curve in sample baked.ron");
    let smooth = manifest.curves.get("smooth").expect("sample baked.ron contains `smooth` curve");
    assert_eq!(smooth.interp, Interp::CatmullRom);
    assert_eq!(smooth.min, 0.0);
    assert_eq!(smooth.max, 1.0);
    assert_eq!(smooth.points, vec![(0.0, 0.0), (0.25, 0.15), (0.75, 0.85), (1.0, 1.0)]);
    for (name, c) in &manifest.curves {
        assert_eq!(c.interp, Interp::CatmullRom, "curve `{name}` must default to CatmullRom");
    }
}

#[test]
fn common_curve_data_bitwise_matches_project_core() {
    use project_core::common::curve::{CurveData as CoreCurveData, Interp as CoreInterp};

    let pulse = curve_kit::Curve::new("pulse", vec![(0.0, 0.0), (0.25, 1.0), (0.5, 0.3), (0.75, 0.1), (1.0, 0.0)]);
    let steps = curve_kit::Curve::new("steps", vec![(0.0, 0.2), (0.33, 0.2), (0.34, 1.0), (0.66, 1.0), (0.67, 0.5)]);
    let smooth = curve_kit::Curve::new("smooth", vec![(0.0, 0.0), (0.25, 0.15), (0.75, 0.85), (1.0, 1.0)]);
    let ramp = curve_kit::Curve::ramp("ramp");
    let one = curve_kit::Curve::new("one", vec![(0.35, 0.7)]);
    let dup = curve_kit::Curve::new("dup", vec![(0.0, 0.0), (0.5, 0.1), (0.5, 0.9), (1.0, 1.0)]);
    let ranged = curve_kit::Curve { min: -2.0, max: 5.0, ..pulse.clone() };
    let curves = [pulse, steps, smooth, ramp, one, dup, ranged];

    let mut ts = vec![-1.0_f32, -0.01, 0.0, 0.001];
    for k in 1..=20 {
        ts.push(k as f32 * 0.05);
    }
    ts.extend([0.999, 1.0, 1.5, f32::NEG_INFINITY, f32::INFINITY, f32::NAN]);

    for c in &curves {
        for (c_interp, core_interp) in
            [(common::Interp::Simple, CoreInterp::Simple), (common::Interp::CatmullRom, CoreInterp::CatmullRom)]
        {
            let composed = common::CurveData::new(c_interp, c.min, c.max, c.points.clone());
            let core = CoreCurveData::new(core_interp, c.min, c.max, c.points.clone());
            for &t in &ts {
                assert_eq!(
                    composed.sample(t).to_bits(),
                    core.sample(t).to_bits(),
                    "CurveData::sample mismatch on {} ({c_interp:?}, t={t})",
                    c.name
                );
                assert_eq!(
                    composed.sample_normalized(t).to_bits(),
                    core.sample_normalized(t).to_bits(),
                    "CurveData::sample_normalized mismatch on {} ({c_interp:?}, t={t})",
                    c.name
                );
            }
        }
    }
}

#[test]
fn common_gradient_data_bitwise_matches_project_core_and_roundtrips() {
    use bevy::color::{ColorToComponents, LinearRgba};
    use project_core::common::gradient::{RawGradient as CoreRawGradient, RawStop as CoreRawStop};

    const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
    const GREEN: [f32; 4] = [0.0, 1.0, 0.0, 1.0];
    const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

    let make_core = |stops: &[(f32, [f32; 4], f32)]| CoreRawGradient {
        stops: stops.iter().map(|&(t, color, mid)| CoreRawStop { t, color, mid }).collect(),
    };

    let mut raw_gradients: Vec<CoreRawGradient> = shader_project::fixtures::gradients()
        .into_iter()
        .flat_map(|(_, col)| col.slots.into_iter().map(|s| CoreRawGradient::from(&s)))
        .collect();
    raw_gradients.extend([
        make_core(&[]),
        make_core(&[(0.35, GREEN, 0.5)]),
        make_core(&[(0.0, RED, 0.5), (0.5, RED, 0.5), (0.5, BLUE, 0.5), (1.0, BLUE, 0.5)]),
        make_core(&[(0.0, RED, 0.15), (0.6, GREEN, 0.85), (1.0, BLUE, 0.5)]),
        make_core(&[(0.0, [1.0, 0.5, 0.2, 1.0], 0.5), (1.0, [0.1, 0.8, 0.9, 0.0], 0.5)]),
        make_core(&[(0.0, [0.3, 0.6, 0.9, 0.2], 0.35), (1.0, [0.3, 0.6, 0.9, 0.8], 0.5)]),
    ]);

    let mut ts = vec![-1.0_f32, -0.01, 0.0, 0.001];
    for k in 1..=19 {
        ts.push(k as f32 * 0.05);
    }
    ts.extend([0.999, 1.0, 1.5, f32::NEG_INFINITY, f32::INFINITY, f32::NAN]);

    for (idx, core_raw) in raw_gradients.iter().enumerate() {
        let composed_raw = common::gradient::RawGradient {
            stops: core_raw
                .stops
                .iter()
                .map(|s| common::gradient::RawStop { t: s.t, color: s.color, mid: s.mid })
                .collect(),
        };
        let grad_data = common::GradientData::from(composed_raw.clone());

        // Exact round-trip between GradientData and RawGradient for linear colors.
        assert_eq!(common::gradient::RawGradient::from(&grad_data), composed_raw);
        assert_eq!(common::gradient::RawGradient::from(grad_data.clone()), composed_raw);
        assert_eq!(common::GradientData::from(common::gradient::RawGradient::from(&grad_data)), grad_data);

        // RON wire format of GradientData is identical to RawGradient.
        let ron_grad = ron::to_string(&grad_data).expect("serialize GradientData");
        let ron_raw = ron::to_string(core_raw).expect("serialize CoreRawGradient");
        assert_eq!(ron_grad, ron_raw, "wire format mismatch at index {idx}");
        let decoded: common::GradientData = ron::from_str(&ron_raw).expect("deserialize GradientData from raw RON");
        assert_eq!(decoded, grad_data);

        for &t in &ts {
            let actual_color = grad_data.sample(t);
            match core_raw.sample(t) {
                Some(expected) => {
                    let actual = actual_color.to_linear().to_f32_array();
                    assert_eq!(
                        actual.map(f32::to_bits),
                        expected.map(f32::to_bits),
                        "GradientData::sample mismatch at index {idx}, t={t}: {actual:?} vs {expected:?}"
                    );
                }
                None => {
                    assert_eq!(actual_color, Color::WHITE, "empty gradient at index {idx}, t={t} must return WHITE");
                }
            }
        }
    }

    // Empty default returns WHITE.
    assert_eq!(common::GradientData::default().sample(0.3), Color::WHITE);

    // `mid: None` is bitwise equivalent to `mid: Some(0.5)`.
    let red_c = Color::LinearRgba(LinearRgba::from_f32_array(RED));
    let blue_c = Color::LinearRgba(LinearRgba::from_f32_array(BLUE));
    let with_none = common::GradientData {
        stops: vec![
            common::GradientStop { t: 0.0, color: red_c, mid: None },
            common::GradientStop { t: 1.0, color: blue_c, mid: None },
        ],
    };
    let with_some = common::GradientData {
        stops: vec![
            common::GradientStop { t: 0.0, color: red_c, mid: Some(0.5) },
            common::GradientStop { t: 1.0, color: blue_c, mid: Some(0.5) },
        ],
    };
    assert_eq!(common::gradient::RawGradient::from(&with_none), common::gradient::RawGradient::from(&with_some));
    for &t in &ts {
        assert_eq!(
            with_none.sample(t).to_linear().to_f32_array().map(f32::to_bits),
            with_some.sample(t).to_linear().to_f32_array().map(f32::to_bits),
            "mid: None vs Some(0.5) mismatch at t={t}"
        );
    }
}
