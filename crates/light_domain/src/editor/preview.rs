//! 3D Render-to-Texture (RTT) preview studio for the Light domain (`light.preview_3d`).
//!
//! Managed via `editor_core::preview`'s RAII [`PreviewHandle`] (`start_preview`, `clean_preview`): opening an
//! [`EnvironmentDoc`] (or a future transition preview) starts an isolated 3D preview session on its own
//! [`RenderLayers`] layer with a calibrated PBR demo scene (matte ground plane, dielectric/chrome/glossy spheres,
//! and depth-cue boxes for fog and shadow evaluation) rendered to an offscreen [`Image`] target. Closing the tab or
//! cleaning the handle tears down the camera, directional lights, demo meshes, materials, and RTT texture.

use std::collections::{BTreeMap, BTreeSet};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::RenderLayers;
use bevy::camera::{Camera, Camera3d, ClearColorConfig, Exposure, Hdr, RenderTarget};
use bevy::color::LinearRgba;
use bevy::core_pipeline::Skybox;
use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::image::{CompressedImageFormatSupport, CompressedImageFormats, ImageType};
use bevy::input::mouse::MouseScrollPixelsPerLine;
use bevy::light::{DirectionalLight, EnvironmentMapLight, GlobalAmbientLight};
use bevy::pbr::{DistanceFog, FogFalloff, MeshMaterial3d, StandardMaterial};
use bevy::prelude::*;
use bevy::render::render_resource::{
    Extent3d, TextureDimension, TextureFormat, TextureUsages, TextureViewDescriptor, TextureViewDimension,
};
use bevy::render::view::{ColorGrading, ColorGradingGlobal, ColorGradingSection};
use editor_core::doc_kind::TabDoc;
use editor_core::preview::{PreviewCallbacks, PreviewEntity, PreviewKindId, sync_previews};
use editor_core::state::editor::Editor;
use editor_core::tab::ViewKindId;
use editor_core::theme::{ACCENT, TEXT_DIM, text};
use project_core::Project;
use texture_kit::{SamplerSettings, TextureAsset};

use super::{ENVIRONMENT_VIEW, EnvironmentDoc, LIGHT_PREVIEW_VIEW, LightPreviewDoc, LightState};
use crate::backend::{
    ColorGradingDef, ColorGradingGlobalDef, ColorGradingSectionDef, DirectionalLightDef, ENVIRONMENT_FORMAT_VERSION,
    Environment, FogDef, FogFalloffDef, LIGHT_TEXTURES, TonemappingMode, texture_ref,
};

/// Preview kind identifier for the Light domain's 3D RTT preview studio.
pub const LIGHT_PREVIEW_KIND: PreviewKindId = PreviewKindId("light.preview_3d");

const RTT_WIDTH: u32 = 640;
const RTT_HEIGHT: u32 = 420;

const DEFAULT_YAW: f32 = 0.35;
const DEFAULT_PITCH: f32 = 0.28;
const DEFAULT_DISTANCE: f32 = 6.5;
const MIN_DISTANCE: f32 = 2.5;
const MAX_DISTANCE: f32 = 25.0;
const MIN_PITCH: f32 = -0.15;
const MAX_PITCH: f32 = 1.45;
const ORBIT_TARGET: Vec3 = Vec3::new(0.0, -0.15, 0.0);

/// Returns the [`PreviewCallbacks`] registered with `editor_core` for [`LIGHT_PREVIEW_KIND`].
pub fn preview_callbacks() -> PreviewCallbacks {
    PreviewCallbacks { kind: LIGHT_PREVIEW_KIND, start: start_light_preview, clean: clean_light_preview }
}

/// Ensures the asset stores and preview resources needed by [`start_light_preview`] exist in both windowed and
/// headless test apps.
pub fn setup_preview_logic(app: &mut App) {
    if !app.world().contains_resource::<Assets<Image>>() {
        app.init_resource::<Assets<Image>>();
    }
    if !app.world().contains_resource::<Assets<Mesh>>() {
        app.init_resource::<Assets<Mesh>>();
    }
    if !app.world().contains_resource::<Assets<StandardMaterial>>() {
        app.init_resource::<Assets<StandardMaterial>>();
    }
    if !app.is_plugin_added::<BlendedSkyboxPlugin>() {
        app.add_plugins(BlendedSkyboxPlugin);
    }
    app.init_resource::<LightPreviewSessions>()
        .init_resource::<PreviewCubemapCache>()
        .add_systems(Update, update_light_previews.after(sync_previews));
}

/// Registers pointer orbit/zoom/reset observers for the 3D preview viewport and the `SHADER_EDITOR_SCREENSHOT` hook.
pub fn setup_preview_ui(app: &mut App) {
    app.add_observer(orbit_on_drag).add_observer(zoom_on_scroll).add_observer(reset_on_double_click);
    if std::env::var_os("SHADER_EDITOR_SCREENSHOT").is_some() {
        app.add_systems(Update, light_debug_screenshot.after(editor_core::apply_actions));
    }
}

fn light_debug_screenshot(mut frames: Local<u32>, mut actions: ResMut<editor_core::action::Actions>) {
    *frames += 1;
    if *frames == 4
        && let Ok(open) = std::env::var("SHADER_EDITOR_OPEN")
    {
        if let Some(tool) = open.strip_prefix("@tool/") {
            let id: &'static str = match tool {
                "light_preview" => super::LIGHT_PREVIEW_TOOL,
                "inspect_game" => super::INSPECT_GAME_TOOL,
                _ => return,
            };
            actions.push(editor_core::action::Action::OpenTool { domain: crate::backend::LIGHT, id, arg: None });
        } else {
            let rel = if open.starts_with("quillshade/light/") { open } else { format!("quillshade/light/{open}") };
            if let Some(r) = project_core::asset_ref::parse(
                &rel,
                crate::backend::LIGHT,
                &[crate::backend::ENVIRONMENT_SUFFIX, crate::backend::CURVE_SUFFIX, crate::backend::TEXTURE_SUFFIX],
            ) {
                actions.push(editor_core::action::Action::OpenAsset(r));
            }
        }
    }
}

/// Active 3D preview sessions keyed by [` editor_core::preview::PreviewHandle::id`].
#[derive(Resource, Default)]
pub struct LightPreviewSessions {
    pub sessions: BTreeMap<u64, LightPreviewSession>,
    used_layers: BTreeSet<usize>,
}

impl LightPreviewSessions {
    fn alloc_layer(&mut self) -> usize {
        let layer = (1..=31).find(|l| !self.used_layers.contains(l)).unwrap_or(1);
        self.used_layers.insert(layer);
        layer
    }

    fn free_layer(&mut self, layer: usize) {
        self.used_layers.remove(&layer);
    }
}

/// One spawned 3D preview studio instance associated with a [`editor_core::preview::PreviewHandle`].
pub struct LightPreviewSession {
    pub id: u64,
    pub layer: usize,
    pub camera: Entity,
    pub rtt_image: Option<Handle<Image>>,
    pub meshes: Vec<Handle<Mesh>>,
    pub materials: Vec<Handle<StandardMaterial>>,
    pub lights: Vec<Entity>,
    pub procedural_ibl: Option<([u32; 12], Handle<Image>)>,
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
    pub applied_key: Option<u64>,
    pub last_env: Option<Environment>,
    pub last_skybox_blend: Option<PreviewSkyboxBlend>,
}

impl LightPreviewSession {
    /// Computes the camera transform from the current spherical orbit coordinates around [`ORBIT_TARGET`].
    pub fn camera_transform(&self) -> Transform {
        let cos_p = self.pitch.cos();
        let offset = Vec3::new(
            self.distance * cos_p * self.yaw.sin(),
            self.distance * self.pitch.sin(),
            self.distance * cos_p * self.yaw.cos(),
        );
        Transform::from_translation(ORBIT_TARGET + offset).looking_at(ORBIT_TARGET, Vec3::Y)
    }
}

#[derive(Clone, PartialEq)]
struct CubemapCacheKey {
    file: PathBuf,
    is_srgb: bool,
    sampler: Option<SamplerSettings>,
}

/// Caches 6-face KTX2 cube-map [`Image`] handles decoded from the game's `assets/` directory for the 3D preview's
/// [`Skybox`].
#[derive(Resource, Default)]
pub struct PreviewCubemapCache {
    entries: Vec<(CubemapCacheKey, Option<Handle<Image>>)>,
    reload_generation: u64,
}

impl PreviewCubemapCache {
    /// Clears all cached cube-map handles so modified `.ktx2` files on disk are re-decoded on next use.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Resolves and decodes the Light Texture asset `cubemap_id` from `project` (using `in_memory_tex` when available
    /// to avoid reading `.texture.ron` from disk) into a cube-map [`Handle<Image>`].
    pub fn resolve(
        &mut self,
        project: &Project,
        cubemap_id: &str,
        in_memory_tex: Option<&TextureAsset>,
        formats: CompressedImageFormats,
        images: &mut Assets<Image>,
    ) -> Option<Handle<Image>> {
        let owned_tex;
        let tex_asset = match in_memory_tex {
            Some(t) => t,
            None => {
                let tref = texture_ref(cubemap_id);
                owned_tex = texture_kit::load(&project.root.join(tref.source_rel())).ok()?;
                &owned_tex
            }
        };
        TextureAsset::check_path(&tex_asset.path, &LIGHT_TEXTURES).ok()?;
        let file = project.assets_dir().join(&tex_asset.path);
        let key = CubemapCacheKey {
            file: file.clone(),
            is_srgb: tex_asset.settings.is_srgb,
            sampler: tex_asset.settings.sampler,
        };
        if let Some((_, cached)) = self.entries.iter().find(|(k, _)| *k == key)
            && cached.as_ref().is_none_or(|h| images.contains(h.id()))
        {
            return cached.clone();
        }
        self.entries.retain(|(k, _)| *k != key);

        let bytes = std::fs::read(&file).ok()?;
        if texture_kit::shape_problem(&bytes, LIGHT_TEXTURES.shape).is_some() {
            self.entries.push((key, None));
            return None;
        }
        let shown = texture_kit::editor::previewable(&tex_asset.settings, false).0;
        let mut image = Image::from_buffer(
            &bytes,
            ImageType::Extension("ktx2"),
            formats,
            shown.is_srgb,
            texture_kit::editor::sampler(&shown),
            RenderAssetUsages::default(),
        )
        .ok()?;
        if image.texture_descriptor.size.depth_or_array_layers == 6 {
            image.texture_view_descriptor =
                Some(TextureViewDescriptor { dimension: Some(TextureViewDimension::Cube), ..default() });
        }
        let handle = images.add(image);
        self.entries.push((key, Some(handle.clone())));
        Some(handle)
    }
}

/// Interactive 3D preview viewport container in the editor content (`{properties} | {preview}`).
#[derive(Component, Clone, Copy)]
pub struct LightPreviewViewport(pub ViewKindId);

/// The UI [`ImageNode`] displaying the active preview session's offscreen render target.
#[derive(Component, Clone, Copy)]
pub struct LightPreviewImage(pub ViewKindId);

/// Builds the right-hand 3D RTT preview pane for `view`.
pub fn preview_viewport(view: ViewKindId) -> impl Scene {
    let viewport = LightPreviewViewport(view);
    let pic = LightPreviewImage(view);
    bsn! {
        Node {
            flex_grow: 1.0,
            min_width: px(240),
            min_height: px(0),
            flex_direction: FlexDirection::Column,
            padding: UiRect::new(px(12), px(16), px(12), px(56)),
        }
        Children [
            viewport
            Node {
                flex_grow: 1.0,
                width: percent(100),
                min_height: px(0),
                border: UiRect::all(px(2)),
                border_radius: BorderRadius::all(px(8)),
                overflow: Overflow::clip(),
            }
            BorderColor::all(ACCENT)
            BackgroundColor(Color::srgb_u8(24, 25, 29))
            Children [
                ImageNode
                pic
                Node {
                    width: percent(100),
                    height: percent(100),
                    border_radius: BorderRadius::all(px(6)),
                }
                Pickable::IGNORE
                --
                Node {
                    position_type: PositionType::Absolute,
                    left: px(12),
                    top: px(10),
                }
                Pickable::IGNORE
                Children [
                    @text("3D Preview (drag: orbit, wheel: zoom, double-click: reset)", TEXT_DIM, 11.0)
                ]
            ]
        ]
    }
}

/// Spawns the RTT image, `Camera3d`, and calibrated PBR demo scene for preview session `id`.
pub fn start_light_preview(world: &mut World, id: u64) {
    if !world.contains_resource::<LightPreviewSessions>() {
        world.init_resource::<LightPreviewSessions>();
    }
    if !world.contains_resource::<PreviewCubemapCache>() {
        world.init_resource::<PreviewCubemapCache>();
    }

    let layer = world.resource_mut::<LightPreviewSessions>().alloc_layer();
    let render_layer = RenderLayers::layer(layer);

    let rtt_image = world.get_resource_mut::<Assets<Image>>().map(|mut images| {
        let mut img = Image::new_fill(
            Extent3d { width: RTT_WIDTH, height: RTT_HEIGHT, depth_or_array_layers: 1 },
            TextureDimension::D2,
            &[24, 25, 29, 255],
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::default(),
        );
        img.texture_descriptor.usage =
            TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST | TextureUsages::RENDER_ATTACHMENT;
        images.add(img)
    });

    let mut meshes_out = Vec::new();
    let mut materials_out = Vec::new();

    if world.contains_resource::<Assets<Mesh>>() && world.contains_resource::<Assets<StandardMaterial>>() {
        let (ground_mesh, center_mesh, side_mesh, box_mesh) = {
            let mut meshes = world.resource_mut::<Assets<Mesh>>();
            (
                meshes.add(Plane3d::default().mesh().size(80.0, 80.0).build()),
                meshes.add(Sphere::new(0.85).mesh().ico(5).unwrap()),
                meshes.add(Sphere::new(0.7).mesh().ico(5).unwrap()),
                meshes.add(Cuboid::new(1.2, 1.8, 1.2)),
            )
        };
        meshes_out.extend([ground_mesh.clone(), center_mesh.clone(), side_mesh.clone(), box_mesh.clone()]);

        let (ground_mat, dielectric_mat, chrome_mat, glossy_mat, box_mat) = {
            let mut materials = world.resource_mut::<Assets<StandardMaterial>>();
            (
                materials.add(StandardMaterial {
                    base_color: Color::LinearRgba(LinearRgba::rgb(0.35, 0.35, 0.35)),
                    perceptual_roughness: 0.85,
                    metallic: 0.0,
                    ..default()
                }),
                materials.add(StandardMaterial {
                    base_color: Color::LinearRgba(LinearRgba::rgb(0.8, 0.8, 0.8)),
                    perceptual_roughness: 0.45,
                    metallic: 0.0,
                    ..default()
                }),
                materials.add(StandardMaterial {
                    base_color: Color::WHITE,
                    perceptual_roughness: 0.05,
                    metallic: 1.0,
                    ..default()
                }),
                materials.add(StandardMaterial {
                    base_color: Color::srgb(0.22, 0.48, 0.88),
                    perceptual_roughness: 0.15,
                    metallic: 0.0,
                    ..default()
                }),
                materials.add(StandardMaterial {
                    base_color: Color::LinearRgba(LinearRgba::rgb(0.55, 0.55, 0.58)),
                    perceptual_roughness: 0.6,
                    metallic: 0.0,
                    ..default()
                }),
            )
        };
        materials_out.extend([
            ground_mat.clone(),
            dielectric_mat.clone(),
            chrome_mat.clone(),
            glossy_mat.clone(),
            box_mat.clone(),
        ]);

        // 1. Ground plane
        world.spawn((
            PreviewEntity(id),
            render_layer.clone(),
            Mesh3d(ground_mesh),
            MeshMaterial3d(ground_mat),
            Transform::from_xyz(0.0, -1.0, 0.0),
        ));
        // 2. Center neutral dielectric sphere
        world.spawn((
            PreviewEntity(id),
            render_layer.clone(),
            Mesh3d(center_mesh),
            MeshMaterial3d(dielectric_mat),
            Transform::from_xyz(0.0, -0.15, 0.0),
        ));
        // 3. Left chrome mirror sphere
        world.spawn((
            PreviewEntity(id),
            render_layer.clone(),
            Mesh3d(side_mesh.clone()),
            MeshMaterial3d(chrome_mat),
            Transform::from_xyz(-2.0, -0.3, 0.3),
        ));
        // 4. Right glossy dielectric sphere
        world.spawn((
            PreviewEntity(id),
            render_layer.clone(),
            Mesh3d(side_mesh),
            MeshMaterial3d(glossy_mat),
            Transform::from_xyz(2.0, -0.3, 0.3),
        ));
        // 5. Depth boxes for DistanceFog falloff and shadow evaluation
        for pos in [Vec3::new(-3.5, -0.1, -8.0), Vec3::new(3.5, 0.2, -18.0), Vec3::new(0.0, 1.0, -36.0)] {
            world.spawn((
                PreviewEntity(id),
                render_layer.clone(),
                Mesh3d(box_mesh.clone()),
                MeshMaterial3d(box_mat.clone()),
                Transform::from_translation(pos),
            ));
        }
    }

    let cam = Camera { order: -1, is_active: false, ..default() };
    let render_target = rtt_image.as_ref().map(|rtt| RenderTarget::Image(rtt.clone().into()));
    let def_env = Environment::default();
    let procedural_ibl = world.get_resource_mut::<Assets<Image>>().map(|mut images| {
        let handle = images.add(hemispherical_gradient_image(
            def_env.sky_top_color,
            def_env.sky_horizon_color,
            def_env.ground_color,
        ));
        (ibl_color_key(&def_env), handle)
    });

    let mut session = LightPreviewSession {
        id,
        layer,
        camera: Entity::PLACEHOLDER,
        rtt_image,
        meshes: meshes_out,
        materials: materials_out,
        lights: Vec::new(),
        procedural_ibl: procedural_ibl.clone(),
        yaw: DEFAULT_YAW,
        pitch: DEFAULT_PITCH,
        distance: DEFAULT_DISTANCE,
        applied_key: None,
        last_env: None,
        last_skybox_blend: None,
    };
    let cam_transform = session.camera_transform();
    let mut cam_cmd = world.spawn((
        PreviewEntity(id),
        render_layer,
        Camera3d::default(),
        cam,
        Hdr,
        Exposure { ev100: 13.0 },
        Tonemapping::TonyMcMapface,
        DebandDither::Enabled,
        ColorGrading::default(),
        cam_transform,
    ));
    if let Some((_, ibl_handle)) = procedural_ibl {
        cam_cmd.insert(EnvironmentMapLight {
            diffuse_map: ibl_handle.clone(),
            specular_map: ibl_handle,
            intensity: def_env.env_intensity,
            rotation: Quat::from_rotation_y(def_env.env_rotation.to_radians()),
            affects_lightmapped_mesh_diffuse: def_env.env_affects_lightmapped,
        });
    }
    if let Some(target) = render_target {
        cam_cmd.insert(target);
    }
    session.camera = cam_cmd.id();

    world.resource_mut::<LightPreviewSessions>().sessions.insert(id, session);
}

/// Cleans up all GPU assets and session state for preview `id` (entities tagged with [`PreviewEntity`] are despawned
/// by [`sync_previews`]).
pub fn clean_light_preview(world: &mut World, id: u64) {
    let Some(session) = world.get_resource_mut::<LightPreviewSessions>().and_then(|mut s| {
        let removed = s.sessions.remove(&id);
        if let Some(ref sess) = removed {
            s.free_layer(sess.layer);
        }
        removed
    }) else {
        return;
    };

    if let Some(mut images) = world.get_resource_mut::<Assets<Image>>() {
        if let Some(rtt) = session.rtt_image {
            images.remove(rtt.id());
        }
        if let Some((_, ibl)) = session.procedural_ibl {
            images.remove(ibl.id());
        }
    }
    if let Some(mut meshes) = world.get_resource_mut::<Assets<Mesh>>() {
        for m in session.meshes {
            meshes.remove(m.id());
        }
    }
    if let Some(mut materials) = world.get_resource_mut::<Assets<StandardMaterial>>() {
        for mat in session.materials {
            materials.remove(mat.id());
        }
    }
}

/// Dual-cubemap / sky-color blend state for the editor's 3D preview camera.
#[derive(Debug, Clone, PartialEq)]
pub struct PreviewSkyboxBlend {
    pub cubemap_a: Option<String>,
    pub cubemap_b: Option<String>,
    pub brightness_a: f32,
    pub brightness_b: f32,
    pub rotation_a: f32,
    pub rotation_b: f32,
    pub color_a: [f32; 4],
    pub color_b: [f32; 4],
    pub blend: f32,
    pub alpha: f32,
}

impl PreviewSkyboxBlend {
    /// Skybox state for a single settled [`Environment`].
    pub fn from_single(env: &Environment) -> Self {
        match &env.cubemap {
            Some(c) => Self {
                cubemap_a: Some(c.clone()),
                cubemap_b: Some(c.clone()),
                brightness_a: env.skybox_brightness,
                brightness_b: env.skybox_brightness,
                rotation_a: env.skybox_rotation,
                rotation_b: env.skybox_rotation,
                color_a: env.clear_color,
                color_b: env.clear_color,
                blend: 0.0,
                alpha: 1.0,
            },
            None => Self {
                cubemap_a: None,
                cubemap_b: None,
                brightness_a: 0.0,
                brightness_b: 0.0,
                rotation_a: env.skybox_rotation,
                rotation_b: env.skybox_rotation,
                color_a: env.clear_color,
                color_b: env.clear_color,
                blend: 0.0,
                alpha: 0.0,
            },
        }
    }

    /// Interpolates skybox blend parameters between `from` and `to`.
    pub fn interpolate(from: &Environment, to: &Environment, raw_t: f32, s: f32) -> Self {
        let raw_t = raw_t.clamp(0.0, 1.0);
        if raw_t >= 1.0 {
            return Self::from_single(to);
        }
        let blend = s.clamp(0.0, 1.0);
        match (&from.cubemap, &to.cubemap) {
            (Some(a), Some(b)) => Self {
                cubemap_a: Some(a.clone()),
                cubemap_b: Some(b.clone()),
                brightness_a: from.skybox_brightness,
                brightness_b: to.skybox_brightness,
                rotation_a: from.skybox_rotation,
                rotation_b: to.skybox_rotation,
                color_a: from.clear_color,
                color_b: to.clear_color,
                blend,
                alpha: 1.0,
            },
            (None, Some(b)) => Self {
                cubemap_a: None,
                cubemap_b: Some(b.clone()),
                brightness_a: from.skybox_brightness,
                brightness_b: to.skybox_brightness,
                rotation_a: from.skybox_rotation,
                rotation_b: to.skybox_rotation,
                color_a: from.clear_color,
                color_b: to.clear_color,
                blend,
                alpha: 1.0,
            },
            (Some(a), None) => Self {
                cubemap_a: Some(a.clone()),
                cubemap_b: None,
                brightness_a: from.skybox_brightness,
                brightness_b: to.skybox_brightness,
                rotation_a: from.skybox_rotation,
                rotation_b: to.skybox_rotation,
                color_a: from.clear_color,
                color_b: to.clear_color,
                blend,
                alpha: 1.0,
            },
            (None, None) => Self {
                cubemap_a: None,
                cubemap_b: None,
                brightness_a: 0.0,
                brightness_b: 0.0,
                rotation_a: lerp_angle_deg(from.skybox_rotation, to.skybox_rotation, s),
                rotation_b: to.skybox_rotation,
                color_a: from.clear_color,
                color_b: to.clear_color,
                blend,
                alpha: 0.0,
            },
        }
    }
}

/// Applies `env` to preview session `id`'s camera, skybox, IBL, ambient light, fog, and directional lights.
pub fn apply_environment_to_session(world: &mut World, id: u64, env: &Environment) {
    let skybox_blend = PreviewSkyboxBlend::from_single(env);
    apply_environment_with_skybox_to_session(world, id, env, &skybox_blend);
}

/// Applies `env` and `skybox_blend` to preview session `id`'s camera, blended skybox, IBL, ambient light, fog, and
/// directional lights.
pub fn apply_environment_with_skybox_to_session(
    world: &mut World,
    id: u64,
    env: &Environment,
    skybox_blend: &PreviewSkyboxBlend,
) {
    let (project, reload_generation, in_memory_tex_a, in_memory_tex_b) = world
        .get_resource::<Editor>()
        .map(|editor| {
            let project = editor.project.clone();
            let state = editor.domains.get::<LightState>();
            let reload_gen = state.map_or(0, |s| s.reload_generation);
            let lookup_tex = |cid: Option<&str>| {
                cid.and_then(|cid| {
                    editor
                        .tabs
                        .iter()
                        .filter(|t| t.domain == crate::backend::LIGHT)
                        .find_map(|t| {
                            t.get::<texture_kit::editor::TextureDoc<super::LightTexture>>()
                                .filter(|d| d.aref.path == cid)
                                .map(|d| d.asset.clone())
                        })
                        .or_else(|| state.and_then(|s| s.assets.textures.get(cid)).cloned())
                })
            };
            (
                project,
                reload_gen,
                lookup_tex(skybox_blend.cubemap_a.as_deref()),
                lookup_tex(skybox_blend.cubemap_b.as_deref()),
            )
        })
        .unwrap_or((None, 0, None, None));
    let formats = world.get_resource::<CompressedImageFormatSupport>().map_or(CompressedImageFormats::NONE, |s| s.0);

    if let Some(mut cache) = world.get_resource_mut::<PreviewCubemapCache>()
        && cache.reload_generation != reload_generation
    {
        cache.clear();
        cache.reload_generation = reload_generation;
    }

    let key = env_hash(
        env,
        skybox_blend,
        project.as_ref(),
        reload_generation,
        in_memory_tex_a.as_ref(),
        in_memory_tex_b.as_ref(),
    );
    let (
        camera_entity,
        layer,
        cam_transform,
        existing_ibl,
        existing_lights,
        already_applied,
        last_env,
        last_skybox_blend,
    ) = {
        let Some(sessions) = world.get_resource::<LightPreviewSessions>() else { return };
        let Some(s) = sessions.sessions.get(&id) else { return };
        (
            s.camera,
            s.layer,
            s.camera_transform(),
            s.procedural_ibl.clone(),
            s.lights.clone(),
            s.applied_key == Some(key),
            s.last_env.clone(),
            s.last_skybox_blend.clone(),
        )
    };

    if let Ok(mut cam_ent) = world.get_entity_mut(camera_entity) {
        if let Some(mut cam) = cam_ent.get_mut::<Camera>()
            && !cam.is_active
        {
            cam.is_active = true;
        }
        if let Some(mut tf) = cam_ent.get_mut::<Transform>()
            && *tf != cam_transform
        {
            *tf = cam_transform;
        }
    }

    if already_applied {
        return;
    }

    let prev = last_env.as_ref();

    if prev.is_none_or(|p| {
        p.ambient_color != env.ambient_color
            || p.ambient_brightness != env.ambient_brightness
            || p.ambient_affects_lightmapped != env.ambient_affects_lightmapped
    }) || !world.contains_resource::<GlobalAmbientLight>()
    {
        let want_ambient = GlobalAmbientLight {
            color: linear_rgba(env.ambient_color),
            brightness: env.ambient_brightness,
            affects_lightmapped_meshes: env.ambient_affects_lightmapped,
        };
        if let Some(mut amb) = world.get_resource_mut::<GlobalAmbientLight>() {
            *amb = want_ambient;
        } else {
            world.insert_resource(want_ambient);
        }
    }

    // Resolve procedural hemispherical IBL cubemap by updating the existing 6-face Rgba16Float Image in-place so its
    // AssetId<Image> never changes across frames (preventing a 1-frame IBL drop in Bevy's light probe extractor).
    let ibl_key = ibl_color_key(env);
    let mut new_ibl_pair = existing_ibl;
    let env_map_light = if let Some(mut images) = world.get_resource_mut::<Assets<Image>>() {
        match &mut new_ibl_pair {
            Some((k, h)) if images.contains(h.id()) => {
                if *k != ibl_key {
                    if let Some(mut img) = images.get_mut(h.id()) {
                        img.data = Some(hemispherical_gradient_bytes(
                            env.sky_top_color,
                            env.sky_horizon_color,
                            env.ground_color,
                        ));
                    }
                    *k = ibl_key;
                }
            }
            _ => {
                let handle = images.add(hemispherical_gradient_image(
                    env.sky_top_color,
                    env.sky_horizon_color,
                    env.ground_color,
                ));
                new_ibl_pair = Some((ibl_key, handle));
            }
        }
        new_ibl_pair.as_ref().map(|(_, h)| EnvironmentMapLight {
            diffuse_map: h.clone(),
            specular_map: h.clone(),
            intensity: env.env_intensity,
            rotation: Quat::from_rotation_y(env.env_rotation.to_radians()),
            affects_lightmapped_mesh_diffuse: env.env_affects_lightmapped,
        })
    } else {
        None
    };

    // Resolve dual-cubemap / sky-color BlendedSkybox component
    let (resolved_a, resolved_b) = if let Some(project) = project.as_ref() {
        world.resource_scope(|world, mut cache: Mut<PreviewCubemapCache>| {
            if let Some(mut images) = world.get_resource_mut::<Assets<Image>>() {
                let a = skybox_blend
                    .cubemap_a
                    .as_deref()
                    .and_then(|cid| cache.resolve(project, cid, in_memory_tex_a.as_ref(), formats, &mut images));
                let b = skybox_blend
                    .cubemap_b
                    .as_deref()
                    .and_then(|cid| cache.resolve(project, cid, in_memory_tex_b.as_ref(), formats, &mut images));
                (a, b)
            } else {
                (None, None)
            }
        })
    } else {
        (None, None)
    };
    let blended_skybox = BlendedSkybox {
        cubemap_a: resolved_a,
        cubemap_b: resolved_b,
        brightness_a: skybox_blend.brightness_a,
        brightness_b: skybox_blend.brightness_b,
        rotation_a: Quat::from_rotation_y(skybox_blend.rotation_a.to_radians()),
        rotation_b: Quat::from_rotation_y(skybox_blend.rotation_b.to_radians()),
        color_a: Vec4::from_array(skybox_blend.color_a),
        color_b: Vec4::from_array(skybox_blend.color_b),
        blend: skybox_blend.blend,
        alpha: skybox_blend.alpha,
    };

    if let Ok(mut cam_ent) = world.get_entity_mut(camera_entity) {
        if prev.is_none_or(|p| p.clear_color != env.clear_color)
            && let Some(mut cam) = cam_ent.get_mut::<Camera>()
        {
            cam.clear_color = ClearColorConfig::Custom(linear_rgba(env.clear_color));
        }
        if prev.is_none_or(|p| p.ev100 != env.ev100) {
            let exposure = Exposure { ev100: env.ev100 };
            if let Some(mut exp) = cam_ent.get_mut::<Exposure>() {
                *exp = exposure;
            } else {
                cam_ent.insert(exposure);
            }
        }
        if prev.is_none_or(|p| p.tonemapping != env.tonemapping) {
            let tonemapping = to_bevy_tonemapping(env.tonemapping);
            if let Some(mut tm) = cam_ent.get_mut::<Tonemapping>() {
                *tm = tonemapping;
            } else {
                cam_ent.insert(tonemapping);
            }
        }
        if prev.is_none_or(|p| p.deband_dither != env.deband_dither) {
            let deband = if env.deband_dither { DebandDither::Enabled } else { DebandDither::Disabled };
            if let Some(mut dd) = cam_ent.get_mut::<DebandDither>() {
                *dd = deband;
            } else {
                cam_ent.insert(deband);
            }
        }
        if prev.is_none_or(|p| p.color_grading != env.color_grading) {
            let color_grading = to_bevy_color_grading(&env.color_grading);
            if let Some(mut cg) = cam_ent.get_mut::<ColorGrading>() {
                *cg = color_grading;
            } else {
                cam_ent.insert(color_grading);
            }
        }
        if let Some(ibl) = env_map_light {
            if let Some(mut existing) = cam_ent.get_mut::<EnvironmentMapLight>() {
                if existing.diffuse_map != ibl.diffuse_map
                    || existing.specular_map != ibl.specular_map
                    || existing.intensity != ibl.intensity
                    || existing.rotation != ibl.rotation
                    || existing.affects_lightmapped_mesh_diffuse != ibl.affects_lightmapped_mesh_diffuse
                {
                    *existing = ibl;
                }
            } else {
                cam_ent.insert(ibl);
            }
        }
        if cam_ent.contains::<Skybox>() {
            cam_ent.remove::<Skybox>();
        }
        if last_skybox_blend.as_ref() != Some(skybox_blend) || !cam_ent.contains::<BlendedSkybox>() {
            if let Some(mut existing) = cam_ent.get_mut::<BlendedSkybox>() {
                if *existing != blended_skybox {
                    *existing = blended_skybox;
                }
            } else {
                cam_ent.insert(blended_skybox);
            }
        }
        if prev.is_none_or(|p| p.fog != env.fog) {
            if env.fog.enabled {
                let fog = to_bevy_fog(&env.fog);
                if let Some(mut existing) = cam_ent.get_mut::<DistanceFog>() {
                    *existing = fog;
                } else {
                    cam_ent.insert(fog);
                }
            } else if cam_ent.contains::<DistanceFog>() {
                cam_ent.remove::<DistanceFog>();
            }
        }
    }

    // Sync directional light entities for this session only when lights changed
    let lights_changed = prev.is_none_or(|p| p.lights != env.lights) || existing_lights.len() != env.lights.len();
    let mut updated_lights = existing_lights.clone();
    if lights_changed {
        let render_layer = RenderLayers::layer(layer);
        updated_lights = Vec::with_capacity(env.lights.len());
        for (idx, def) in env.lights.iter().enumerate() {
            let prev_def = prev.and_then(|p| p.lights.get(idx));
            let bevy_light = to_bevy_directional_light(def);
            let tf = Transform::from_rotation(light_rotation(def.azimuth_deg, def.elevation_deg));
            if let Some(&entity) = existing_lights.get(idx)
                && let Ok(mut ent) = world.get_entity_mut(entity)
            {
                if prev_def != Some(def) {
                    if let Some(mut l) = ent.get_mut::<DirectionalLight>() {
                        *l = bevy_light;
                    } else {
                        ent.insert(bevy_light);
                    }
                    if let Some(mut t) = ent.get_mut::<Transform>() {
                        *t = tf;
                    } else {
                        ent.insert(tf);
                    }
                    if let Some(mut n) = ent.get_mut::<Name>() {
                        if n.as_str() != def.name {
                            n.set(def.name.clone());
                        }
                    } else {
                        ent.insert(Name::new(def.name.clone()));
                    }
                }
                updated_lights.push(entity);
            } else {
                let entity = world
                    .spawn((PreviewEntity(id), render_layer.clone(), Name::new(def.name.clone()), bevy_light, tf))
                    .id();
                updated_lights.push(entity);
            }
        }
        for &extra in existing_lights.iter().skip(env.lights.len()) {
            if world.get_entity(extra).is_ok() {
                world.despawn(extra);
            }
        }
    }

    if let Some(mut sessions) = world.get_resource_mut::<LightPreviewSessions>()
        && let Some(s) = sessions.sessions.get_mut(&id)
    {
        s.procedural_ibl = new_ibl_pair;
        s.lights = updated_lights;
        s.applied_key = Some(key);
        s.last_env = Some(env.clone());
        s.last_skybox_blend = Some(skybox_blend.clone());
    }
}

/// Exclusive `Update` system that activates only the active tab's 3D preview camera, syncs the edited or interpolated
/// [`Environment`] into the preview scene, and binds the RTT image handle to the viewport [`ImageNode`].
fn update_light_previews(world: &mut World) {
    let active_env: Option<(u64, ViewKindId, Environment, PreviewSkyboxBlend)> =
        world.get_resource::<Editor>().and_then(|editor| {
            if let Some(doc) = editor.active_doc::<EnvironmentDoc>() {
                let handle = doc.preview_handle()?;
                let env = doc.env.clone();
                let skybox_blend = PreviewSkyboxBlend::from_single(&env);
                return handle.is_alive().then(|| (handle.id(), ENVIRONMENT_VIEW, env, skybox_blend));
            }
            if let Some(doc) = editor.active_doc::<LightPreviewDoc>() {
                let handle = doc.preview_handle()?;
                if !handle.is_alive() {
                    return None;
                }
                let shown_assets = editor
                    .domains
                    .get::<LightState>()
                    .map(|s| super::state::shown(&s.assets, &editor.tabs))
                    .unwrap_or_default();
                let from = shown_assets.environments.get(&doc.env_a).cloned().unwrap_or_default();
                let to = shown_assets.environments.get(&doc.env_b).cloned().unwrap_or_default();
                let raw_t = doc.t.clamp(0.0, 1.0);
                let s = if doc.curve.is_empty() {
                    raw_t
                } else {
                    shown_assets
                        .curves
                        .get(&doc.curve)
                        .map_or(raw_t, |c| c.sample(raw_t, curve_kit::Interp::CatmullRom))
                };
                let env = interpolate_environments(&from, &to, raw_t, s);
                let skybox_blend = PreviewSkyboxBlend::interpolate(&from, &to, raw_t, s);
                return Some((handle.id(), LIGHT_PREVIEW_VIEW, env, skybox_blend));
            }
            None
        });
    let active_id = active_env.as_ref().map(|(id, _, _, _)| *id);

    // Deactivate cameras of inactive sessions so only the visible preview renders.
    let inactive_cameras: Vec<Entity> = world
        .get_resource::<LightPreviewSessions>()
        .map(|s| s.sessions.values().filter(|sess| Some(sess.id) != active_id).map(|sess| sess.camera).collect())
        .unwrap_or_default();

    for cam_entity in inactive_cameras {
        if let Ok(mut e) = world.get_entity_mut(cam_entity)
            && let Some(mut cam) = e.get_mut::<Camera>()
            && cam.is_active
        {
            cam.is_active = false;
        }
    }

    let Some((id, view, env, skybox_blend)) = active_env else { return };

    // Resize the session's RTT image to match the UI viewport's computed size so the 3D preview fills the pane
    // without letterboxing or stretching.
    let viewport_size: Option<UVec2> = {
        let mut vp_q = world.query::<(&LightPreviewViewport, &ComputedNode)>();
        vp_q.iter(world).find_map(|(vp, node)| {
            if vp.0 == view {
                let sz = (node.size() * node.inverse_scale_factor()).round().as_uvec2();
                (sz.x >= 64 && sz.y >= 64).then_some(UVec2::new(sz.x.min(1920), sz.y.min(1440)))
            } else {
                None
            }
        })
    };

    let rtt = world
        .get_resource::<LightPreviewSessions>()
        .and_then(|s| s.sessions.get(&id))
        .and_then(|s| s.rtt_image.clone());
    if let (Some(rtt_handle), Some(want_size)) = (rtt.as_ref(), viewport_size)
        && let Some(mut images) = world.get_resource_mut::<Assets<Image>>()
        && images.get(rtt_handle.id()).is_some_and(|img| img.size() != want_size)
        && let Some(mut img) = images.get_mut(rtt_handle.id())
    {
        img.resize(Extent3d { width: want_size.x, height: want_size.y, depth_or_array_layers: 1 });
    }

    apply_environment_with_skybox_to_session(world, id, &env, &skybox_blend);

    if let Some(rtt) = rtt {
        let mut q = world.query::<(&LightPreviewImage, &mut ImageNode)>();
        for (tag, mut img_node) in q.iter_mut(world) {
            if tag.0 == view && img_node.image != rtt {
                img_node.image = rtt.clone();
            }
        }
    }
}

/// Interpolates between `from` and `to` environments at linear progress `raw_t` (`0.0..=1.0`) and curve-sampled
/// progress `s`.
pub fn interpolate_environments(from: &Environment, to: &Environment, raw_t: f32, s: f32) -> Environment {
    let raw_t = raw_t.clamp(0.0, 1.0);
    let done = raw_t >= 1.0;

    let (cubemap, skybox_brightness, skybox_rotation) = if done {
        (to.cubemap.clone(), to.skybox_brightness, to.skybox_rotation)
    } else {
        match (&from.cubemap, &to.cubemap) {
            (Some(a), Some(b)) if a == b => (
                Some(a.clone()),
                lerp_f32(from.skybox_brightness, to.skybox_brightness, s),
                lerp_angle_deg(from.skybox_rotation, to.skybox_rotation, s),
            ),
            (Some(a), Some(b)) => {
                if s < 0.5 {
                    (Some(a.clone()), from.skybox_brightness * (1.0 - s * 2.0), from.skybox_rotation)
                } else {
                    (Some(b.clone()), to.skybox_brightness * ((s - 0.5) * 2.0), to.skybox_rotation)
                }
            }
            (None, Some(b)) => (Some(b.clone()), to.skybox_brightness * s, to.skybox_rotation),
            (Some(a), None) => (Some(a.clone()), from.skybox_brightness * (1.0 - s), from.skybox_rotation),
            (None, None) => (
                None,
                lerp_f32(from.skybox_brightness, to.skybox_brightness, s),
                lerp_angle_deg(from.skybox_rotation, to.skybox_rotation, s),
            ),
        }
    };

    let lights = if done {
        to.lights.clone()
    } else {
        let max_len = from.lights.len().max(to.lights.len());
        (0..max_len)
            .map(|i| match (from.lights.get(i), to.lights.get(i)) {
                (Some(a), Some(b)) => {
                    let (azimuth_deg, elevation_deg) =
                        slerp_azimuth_elevation(a.azimuth_deg, a.elevation_deg, b.azimuth_deg, b.elevation_deg, s);
                    DirectionalLightDef {
                        name: a.name.clone(),
                        color: lerp_rgba(a.color, b.color, s),
                        illuminance: lerp_f32(a.illuminance, b.illuminance, s),
                        azimuth_deg,
                        elevation_deg,
                        shadow_maps_enabled: a.shadow_maps_enabled,
                        contact_shadows_enabled: a.contact_shadows_enabled,
                        shadow_depth_bias: lerp_f32(a.shadow_depth_bias, b.shadow_depth_bias, s),
                        shadow_normal_bias: lerp_f32(a.shadow_normal_bias, b.shadow_normal_bias, s),
                        affects_lightmapped_mesh_diffuse: a.affects_lightmapped_mesh_diffuse,
                    }
                }
                (Some(a), None) => DirectionalLightDef { illuminance: a.illuminance * (1.0 - s), ..a.clone() },
                (None, Some(b)) => DirectionalLightDef { illuminance: b.illuminance * s, ..b.clone() },
                (None, None) => unreachable!(),
            })
            .collect()
    };

    let clear_color = if done {
        to.clear_color
    } else if from.cubemap.is_none() && to.cubemap.is_some() {
        from.clear_color
    } else if from.cubemap.is_some() && to.cubemap.is_none() {
        to.clear_color
    } else {
        lerp_rgba(from.clear_color, to.clear_color, s)
    };

    Environment {
        format_version: ENVIRONMENT_FORMAT_VERSION,
        clear_color,
        cubemap,
        skybox_brightness,
        skybox_rotation,
        env_intensity: lerp_f32(from.env_intensity, to.env_intensity, s),
        env_rotation: lerp_angle_deg(from.env_rotation, to.env_rotation, s),
        sky_top_color: lerp_rgba(from.sky_top_color, to.sky_top_color, s),
        sky_horizon_color: lerp_rgba(from.sky_horizon_color, to.sky_horizon_color, s),
        ground_color: lerp_rgba(from.ground_color, to.ground_color, s),
        env_affects_lightmapped: if done { to.env_affects_lightmapped } else { from.env_affects_lightmapped },
        ambient_color: lerp_rgba(from.ambient_color, to.ambient_color, s),
        ambient_brightness: lerp_f32(from.ambient_brightness, to.ambient_brightness, s),
        ambient_affects_lightmapped: if done {
            to.ambient_affects_lightmapped
        } else {
            from.ambient_affects_lightmapped
        },
        lights,
        ev100: lerp_f32(from.ev100, to.ev100, s),
        tonemapping: if done { to.tonemapping } else { from.tonemapping },
        deband_dither: if done { to.deband_dither } else { from.deband_dither },
        color_grading: interpolate_color_grading(&from.color_grading, &to.color_grading, s),
        fog: interpolate_fog(&from.fog, &to.fog, raw_t, s),
    }
}

fn lerp_f32(a: f32, b: f32, s: f32) -> f32 {
    a + (b - a) * s
}

fn lerp_vec3(a: [f32; 3], b: [f32; 3], s: f32) -> [f32; 3] {
    [lerp_f32(a[0], b[0], s), lerp_f32(a[1], b[1], s), lerp_f32(a[2], b[2], s)]
}

fn lerp_rgba(a: [f32; 4], b: [f32; 4], s: f32) -> [f32; 4] {
    [lerp_f32(a[0], b[0], s), lerp_f32(a[1], b[1], s), lerp_f32(a[2], b[2], s), lerp_f32(a[3], b[3], s)]
}

fn lerp_angle_deg(a: f32, b: f32, s: f32) -> f32 {
    a + ((((b - a) % 360.0) + 540.0) % 360.0 - 180.0) * s
}

fn slerp_azimuth_elevation(az_a: f32, el_a: f32, az_b: f32, el_b: f32, s: f32) -> (f32, f32) {
    if s <= 0.0 {
        return (az_a, el_a);
    }
    if s >= 1.0 {
        return (az_b, el_b);
    }
    let qa = light_rotation(az_a, el_a);
    let qb = light_rotation(az_b, el_b);
    let q = qa.slerp(qb, s);
    let sky_dir = (q * Vec3::Z).normalize_or_zero();
    let elevation = sky_dir.y.clamp(-1.0, 1.0).asin().to_degrees();
    let horiz = sky_dir.x.hypot(sky_dir.z);
    let azimuth = if horiz < 1e-5 {
        lerp_angle_deg(az_a, az_b, s)
    } else {
        sky_dir.x.atan2(sky_dir.z).to_degrees().rem_euclid(360.0)
    };
    (azimuth, elevation)
}

fn interpolate_color_grading_section(
    from: &ColorGradingSectionDef,
    to: &ColorGradingSectionDef,
    s: f32,
) -> ColorGradingSectionDef {
    ColorGradingSectionDef {
        saturation: lerp_f32(from.saturation, to.saturation, s),
        contrast: lerp_f32(from.contrast, to.contrast, s),
        gamma: lerp_f32(from.gamma, to.gamma, s),
        gain: lerp_f32(from.gain, to.gain, s),
        lift: lerp_f32(from.lift, to.lift, s),
    }
}

fn interpolate_color_grading(from: &ColorGradingDef, to: &ColorGradingDef, s: f32) -> ColorGradingDef {
    ColorGradingDef {
        global: ColorGradingGlobalDef {
            exposure: lerp_f32(from.global.exposure, to.global.exposure, s),
            temperature: lerp_f32(from.global.temperature, to.global.temperature, s),
            tint: lerp_f32(from.global.tint, to.global.tint, s),
            hue: lerp_f32(from.global.hue, to.global.hue, s),
            post_saturation: lerp_f32(from.global.post_saturation, to.global.post_saturation, s),
            midtones_range: [
                lerp_f32(from.global.midtones_range[0], to.global.midtones_range[0], s),
                lerp_f32(from.global.midtones_range[1], to.global.midtones_range[1], s),
            ],
        },
        shadows: interpolate_color_grading_section(&from.shadows, &to.shadows, s),
        midtones: interpolate_color_grading_section(&from.midtones, &to.midtones, s),
        highlights: interpolate_color_grading_section(&from.highlights, &to.highlights, s),
    }
}

fn interpolate_fog(from: &FogDef, to: &FogDef, raw_t: f32, s: f32) -> FogDef {
    if raw_t >= 1.0 {
        return to.clone();
    }
    if !from.enabled && !to.enabled {
        return FogDef { enabled: false, ..to.clone() };
    }
    if !from.enabled && to.enabled {
        return FogDef {
            enabled: true,
            color: [to.color[0], to.color[1], to.color[2], lerp_f32(0.0, to.color[3], s)],
            directional_light_color: to.directional_light_color,
            directional_light_exponent: to.directional_light_exponent,
            falloff: to.falloff.clone(),
        };
    }
    if from.enabled && !to.enabled {
        return FogDef {
            enabled: true,
            color: [from.color[0], from.color[1], from.color[2], lerp_f32(from.color[3], 0.0, s)],
            directional_light_color: from.directional_light_color,
            directional_light_exponent: from.directional_light_exponent,
            falloff: from.falloff.clone(),
        };
    }

    let same_or_promoted = match (&from.falloff, &to.falloff) {
        (FogFalloffDef::Linear { start: s0, end: e0 }, FogFalloffDef::Linear { start: s1, end: e1 }) => {
            Some(FogFalloffDef::Linear { start: lerp_f32(*s0, *s1, s), end: lerp_f32(*e0, *e1, s) })
        }
        (FogFalloffDef::Exponential { density: d0 }, FogFalloffDef::Exponential { density: d1 }) => {
            Some(FogFalloffDef::Exponential { density: lerp_f32(*d0, *d1, s) })
        }
        (FogFalloffDef::ExponentialSquared { density: d0 }, FogFalloffDef::ExponentialSquared { density: d1 }) => {
            Some(FogFalloffDef::ExponentialSquared { density: lerp_f32(*d0, *d1, s) })
        }
        (
            FogFalloffDef::Atmospheric { extinction: ex0, inscattering: in0 },
            FogFalloffDef::Atmospheric { extinction: ex1, inscattering: in1 },
        ) => Some(FogFalloffDef::Atmospheric {
            extinction: lerp_vec3(*ex0, *ex1, s),
            inscattering: lerp_vec3(*in0, *in1, s),
        }),
        (
            FogFalloffDef::Exponential { density: d0 },
            FogFalloffDef::Atmospheric { extinction: ex1, inscattering: in1 },
        ) => {
            let d = [*d0, *d0, *d0];
            Some(FogFalloffDef::Atmospheric { extinction: lerp_vec3(d, *ex1, s), inscattering: lerp_vec3(d, *in1, s) })
        }
        (
            FogFalloffDef::Atmospheric { extinction: ex0, inscattering: in0 },
            FogFalloffDef::Exponential { density: d1 },
        ) => {
            let d = [*d1, *d1, *d1];
            Some(FogFalloffDef::Atmospheric { extinction: lerp_vec3(*ex0, d, s), inscattering: lerp_vec3(*in0, d, s) })
        }
        _ => None,
    };

    if let Some(falloff) = same_or_promoted {
        FogDef {
            enabled: true,
            color: lerp_rgba(from.color, to.color, s),
            directional_light_color: lerp_rgba(from.directional_light_color, to.directional_light_color, s),
            directional_light_exponent: lerp_f32(from.directional_light_exponent, to.directional_light_exponent, s),
            falloff,
        }
    } else if s < 0.5 {
        let u = (s * 2.0).clamp(0.0, 1.0);
        FogDef {
            enabled: true,
            color: [from.color[0], from.color[1], from.color[2], lerp_f32(from.color[3], 0.0, u)],
            directional_light_color: from.directional_light_color,
            directional_light_exponent: from.directional_light_exponent,
            falloff: from.falloff.clone(),
        }
    } else {
        let u = ((s - 0.5) * 2.0).clamp(0.0, 1.0);
        FogDef {
            enabled: true,
            color: [to.color[0], to.color[1], to.color[2], lerp_f32(0.0, to.color[3], u)],
            directional_light_color: to.directional_light_color,
            directional_light_exponent: to.directional_light_exponent,
            falloff: to.falloff.clone(),
        }
    }
}

fn active_session_id(editor: &Editor, view: ViewKindId) -> Option<u64> {
    let tab = editor.tab()?;
    if tab.doc().view() != view {
        return None;
    }
    tab.doc().preview_handle().filter(|h| h.is_alive()).map(|h| h.id())
}

fn orbit_on_drag(
    mut ev: On<PointerDrag>,
    viewports: Query<&LightPreviewViewport>,
    editor: Res<Editor>,
    mut sessions: ResMut<LightPreviewSessions>,
) {
    let Ok(vp) = viewports.get(ev.entity) else { return };
    let Some(id) = active_session_id(&editor, vp.0) else { return };
    let Some(sess) = sessions.sessions.get_mut(&id) else { return };
    ev.propagate(false);
    sess.yaw -= ev.delta.x * 0.008;
    sess.pitch = (sess.pitch + ev.delta.y * 0.008).clamp(MIN_PITCH, MAX_PITCH);
}

fn zoom_on_scroll(
    mut ev: On<PointerScroll>,
    viewports: Query<&LightPreviewViewport>,
    ratio: Res<MouseScrollPixelsPerLine>,
    editor: Res<Editor>,
    mut sessions: ResMut<LightPreviewSessions>,
) {
    let Ok(vp) = viewports.get(ev.entity) else { return };
    let Some(id) = active_session_id(&editor, vp.0) else { return };
    let Some(sess) = sessions.sessions.get_mut(&id) else { return };
    ev.propagate(false);
    let notches = ev.to_lines(&ratio).y;
    if notches != 0.0 {
        sess.distance = (sess.distance * 0.9f32.powf(notches)).clamp(MIN_DISTANCE, MAX_DISTANCE);
    }
}

fn reset_on_double_click(
    ev: On<PointerClick>,
    viewports: Query<&LightPreviewViewport>,
    editor: Res<Editor>,
    mut sessions: ResMut<LightPreviewSessions>,
) {
    let Ok(vp) = viewports.get(ev.entity) else { return };
    if ev.button != PointerButton::Primary || ev.count < 2 {
        return;
    }
    let Some(id) = active_session_id(&editor, vp.0) else { return };
    let Some(sess) = sessions.sessions.get_mut(&id) else { return };
    sess.yaw = DEFAULT_YAW;
    sess.pitch = DEFAULT_PITCH;
    sess.distance = DEFAULT_DISTANCE;
}

fn linear_rgba(c: [f32; 4]) -> Color {
    Color::LinearRgba(LinearRgba::from_f32_array(c))
}

fn ibl_color_key(env: &Environment) -> [u32; 12] {
    [
        env.sky_top_color[0].to_bits(),
        env.sky_top_color[1].to_bits(),
        env.sky_top_color[2].to_bits(),
        env.sky_top_color[3].to_bits(),
        env.sky_horizon_color[0].to_bits(),
        env.sky_horizon_color[1].to_bits(),
        env.sky_horizon_color[2].to_bits(),
        env.sky_horizon_color[3].to_bits(),
        env.ground_color[0].to_bits(),
        env.ground_color[1].to_bits(),
        env.ground_color[2].to_bits(),
        env.ground_color[3].to_bits(),
    ]
}

fn f32_to_f16_bits(v: f32) -> u16 {
    let bits = v.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exp = ((bits >> 23) & 0xff) as i32;
    let mant = bits & 0x007f_ffff;
    if exp == 255 {
        return if mant != 0 { sign | 0x7e00 } else { sign | 0x7c00 };
    }
    let half_exp = exp - 127 + 15;
    if half_exp >= 31 {
        return sign | 0x7c00;
    }
    if half_exp <= 0 {
        if half_exp < -10 {
            return sign;
        }
        let m = (mant | 0x0080_0000) >> (1 - half_exp);
        return sign | ((m + 0x0000_1000) >> 13) as u16;
    }
    let rounded = mant + 0x0000_1000;
    if (rounded & 0x0080_0000) != 0 {
        let next_exp = half_exp + 1;
        if next_exp >= 31 {
            return sign | 0x7c00;
        }
        return sign | ((next_exp as u16) << 10);
    }
    sign | ((half_exp as u16) << 10) | ((rounded >> 13) as u16)
}

fn hemispherical_gradient_bytes(top: [f32; 4], mid: [f32; 4], bottom: [f32; 4]) -> Vec<u8> {
    let mut data = Vec::with_capacity(48);
    for color in [mid, mid, top, bottom, mid, mid] {
        for c in color {
            data.extend_from_slice(&f32_to_f16_bits(c).to_le_bytes());
        }
    }
    data
}

fn hemispherical_gradient_image(top: [f32; 4], mid: [f32; 4], bottom: [f32; 4]) -> Image {
    let mut img = Image::new(
        Extent3d { width: 1, height: 1, depth_or_array_layers: 6 },
        TextureDimension::D2,
        hemispherical_gradient_bytes(top, mid, bottom),
        TextureFormat::Rgba16Float,
        RenderAssetUsages::default(),
    );
    img.texture_view_descriptor =
        Some(TextureViewDescriptor { dimension: Some(TextureViewDimension::Cube), ..default() });
    img
}

fn light_rotation(azimuth_deg: f32, elevation_deg: f32) -> Quat {
    Quat::from_euler(EulerRot::YXZ, azimuth_deg.to_radians(), -elevation_deg.to_radians(), 0.0)
}

fn to_bevy_directional_light(def: &DirectionalLightDef) -> DirectionalLight {
    DirectionalLight {
        color: linear_rgba(def.color),
        illuminance: def.illuminance,
        shadow_maps_enabled: def.shadow_maps_enabled,
        contact_shadows_enabled: def.contact_shadows_enabled,
        affects_lightmapped_mesh_diffuse: def.affects_lightmapped_mesh_diffuse,
        shadow_depth_bias: def.shadow_depth_bias,
        shadow_normal_bias: def.shadow_normal_bias,
    }
}

fn to_bevy_tonemapping(mode: TonemappingMode) -> Tonemapping {
    match mode {
        TonemappingMode::None => Tonemapping::None,
        TonemappingMode::Reinhard => Tonemapping::Reinhard,
        TonemappingMode::ReinhardLuminance => Tonemapping::ReinhardLuminance,
        TonemappingMode::AcesFitted => Tonemapping::AcesFitted,
        TonemappingMode::AgX => Tonemapping::AgX,
        TonemappingMode::SomewhatBoringDisplayTransform => Tonemapping::SomewhatBoringDisplayTransform,
        TonemappingMode::TonyMcMapface => Tonemapping::TonyMcMapface,
        TonemappingMode::BlenderFilmic => Tonemapping::BlenderFilmic,
    }
}

fn to_bevy_color_grading_global(g: &ColorGradingGlobalDef) -> ColorGradingGlobal {
    ColorGradingGlobal {
        exposure: g.exposure,
        temperature: g.temperature,
        tint: g.tint,
        hue: g.hue,
        post_saturation: g.post_saturation,
        midtones_range: g.midtones_range[0]..g.midtones_range[1],
    }
}

fn to_bevy_color_grading_section(s: &ColorGradingSectionDef) -> ColorGradingSection {
    ColorGradingSection { saturation: s.saturation, contrast: s.contrast, gamma: s.gamma, gain: s.gain, lift: s.lift }
}

fn to_bevy_color_grading(cg: &ColorGradingDef) -> ColorGrading {
    ColorGrading {
        global: to_bevy_color_grading_global(&cg.global),
        shadows: to_bevy_color_grading_section(&cg.shadows),
        midtones: to_bevy_color_grading_section(&cg.midtones),
        highlights: to_bevy_color_grading_section(&cg.highlights),
    }
}

fn to_bevy_fog(fog: &FogDef) -> DistanceFog {
    let falloff = match fog.falloff {
        FogFalloffDef::Linear { start, end } => FogFalloff::Linear { start, end },
        FogFalloffDef::Exponential { density } => FogFalloff::Exponential { density },
        FogFalloffDef::ExponentialSquared { density } => FogFalloff::ExponentialSquared { density },
        FogFalloffDef::Atmospheric { extinction, inscattering } => FogFalloff::Atmospheric {
            extinction: Vec3::from_array(extinction),
            inscattering: Vec3::from_array(inscattering),
        },
    };
    DistanceFog {
        color: linear_rgba(fog.color),
        directional_light_color: linear_rgba(fog.directional_light_color),
        directional_light_exponent: fog.directional_light_exponent,
        falloff,
    }
}

fn hash_f32_slice(state: &mut impl Hasher, vals: &[f32]) {
    for v in vals {
        v.to_bits().hash(state);
    }
}

fn env_hash(
    env: &Environment,
    skybox_blend: &PreviewSkyboxBlend,
    project: Option<&Project>,
    reload_generation: u64,
    in_memory_tex_a: Option<&TextureAsset>,
    in_memory_tex_b: Option<&TextureAsset>,
) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    project.map(|p| p.root.as_path()).hash(&mut h);
    reload_generation.hash(&mut h);
    hash_f32_slice(&mut h, &env.clear_color);
    env.cubemap.hash(&mut h);
    skybox_blend.cubemap_a.hash(&mut h);
    skybox_blend.cubemap_b.hash(&mut h);
    hash_f32_slice(
        &mut h,
        &[
            skybox_blend.brightness_a,
            skybox_blend.brightness_b,
            skybox_blend.rotation_a,
            skybox_blend.rotation_b,
            skybox_blend.blend,
            skybox_blend.alpha,
        ],
    );
    hash_f32_slice(&mut h, &skybox_blend.color_a);
    hash_f32_slice(&mut h, &skybox_blend.color_b);
    for tex in [in_memory_tex_a, in_memory_tex_b].into_iter().flatten() {
        tex.path.hash(&mut h);
        tex.settings.is_srgb.hash(&mut h);
        format!("{:?}", tex.settings.sampler).hash(&mut h);
    }
    hash_f32_slice(
        &mut h,
        &[
            env.skybox_brightness,
            env.skybox_rotation,
            env.env_intensity,
            env.env_rotation,
            env.ambient_brightness,
            env.ev100,
        ],
    );
    hash_f32_slice(&mut h, &env.sky_top_color);
    hash_f32_slice(&mut h, &env.sky_horizon_color);
    hash_f32_slice(&mut h, &env.ground_color);
    hash_f32_slice(&mut h, &env.ambient_color);
    env.env_affects_lightmapped.hash(&mut h);
    env.ambient_affects_lightmapped.hash(&mut h);
    env.tonemapping.as_str().hash(&mut h);
    env.deband_dither.hash(&mut h);
    env.lights.len().hash(&mut h);
    for l in &env.lights {
        l.name.hash(&mut h);
        hash_f32_slice(&mut h, &l.color);
        hash_f32_slice(
            &mut h,
            &[l.illuminance, l.azimuth_deg, l.elevation_deg, l.shadow_depth_bias, l.shadow_normal_bias],
        );
        l.shadow_maps_enabled.hash(&mut h);
        l.contact_shadows_enabled.hash(&mut h);
        l.affects_lightmapped_mesh_diffuse.hash(&mut h);
    }
    let cg = &env.color_grading;
    hash_f32_slice(
        &mut h,
        &[
            cg.global.exposure,
            cg.global.temperature,
            cg.global.tint,
            cg.global.hue,
            cg.global.post_saturation,
            cg.global.midtones_range[0],
            cg.global.midtones_range[1],
        ],
    );
    for s in [&cg.shadows, &cg.midtones, &cg.highlights] {
        hash_f32_slice(&mut h, &[s.saturation, s.contrast, s.gamma, s.gain, s.lift]);
    }
    env.fog.enabled.hash(&mut h);
    hash_f32_slice(&mut h, &env.fog.color);
    hash_f32_slice(&mut h, &env.fog.directional_light_color);
    env.fog.directional_light_exponent.to_bits().hash(&mut h);
    match &env.fog.falloff {
        FogFalloffDef::Linear { start, end } => {
            0u8.hash(&mut h);
            hash_f32_slice(&mut h, &[*start, *end]);
        }
        FogFalloffDef::Exponential { density } => {
            1u8.hash(&mut h);
            density.to_bits().hash(&mut h);
        }
        FogFalloffDef::ExponentialSquared { density } => {
            2u8.hash(&mut h);
            density.to_bits().hash(&mut h);
        }
        FogFalloffDef::Atmospheric { extinction, inscattering } => {
            3u8.hash(&mut h);
            hash_f32_slice(&mut h, extinction);
            hash_f32_slice(&mut h, inscattering);
        }
    }
    h.finish()
}

const BLENDED_SKYBOX_WESL: &str = include_str!("../backend/templates/blended_skybox.wesl");

/// Camera component driving the dual-cubemap / sky-color blended skybox shader on 3D preview cameras.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct BlendedSkybox {
    pub cubemap_a: Option<Handle<Image>>,
    pub cubemap_b: Option<Handle<Image>>,
    pub brightness_a: f32,
    pub brightness_b: f32,
    pub rotation_a: Quat,
    pub rotation_b: Quat,
    pub color_a: Vec4,
    pub color_b: Vec4,
    pub blend: f32,
    pub alpha: f32,
}

/// GPU uniform buffer for [`BlendedSkybox`].
#[derive(Component, bevy::render::render_resource::ShaderType, Clone, Copy, Debug)]
pub struct BlendedSkyboxUniforms {
    pub transform_a: Mat4,
    pub transform_b: Mat4,
    pub color_a: Vec4,
    pub color_b: Vec4,
    pub brightness_a: f32,
    pub brightness_b: f32,
    pub blend: f32,
    pub alpha: f32,
    pub has_a: u32,
    pub has_b: u32,
    pub _pad0: u32,
    pub _pad1: u32,
}

#[derive(Resource, Clone)]
struct BlendedSkyboxShader(Handle<bevy::shader::Shader>);

/// Plugin registering the blended skybox shader and render pipeline for the editor's 3D preview studio.
#[derive(Default)]
pub struct BlendedSkyboxPlugin;

impl Plugin for BlendedSkyboxPlugin {
    fn build(&self, app: &mut App) {
        use bevy::render::extract_component::UniformComponentPlugin;
        use bevy::render::render_resource::SpecializedRenderPipelines;
        use bevy::render::sync_component::SyncComponentPlugin;
        use bevy::render::{ExtractSchedule, GpuResourceAppExt, Render, RenderApp, RenderStartup, RenderSystems};
        use bevy::shader::Shader;

        if !app.world().contains_resource::<Assets<Shader>>() {
            return;
        }
        let shader = app
            .world_mut()
            .resource_mut::<Assets<Shader>>()
            .add(Shader::from_wesl(BLENDED_SKYBOX_WESL, "embedded://quillshade/light_editor/blended_skybox.wesl"));
        app.insert_resource(BlendedSkyboxShader(shader.clone()));

        if app.get_sub_app_mut(RenderApp).is_none() {
            return;
        }
        app.add_plugins((
            SyncComponentPlugin::<BlendedSkybox, Self>::default(),
            UniformComponentPlugin::<BlendedSkyboxUniforms>::default(),
        ));

        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .insert_resource(BlendedSkyboxShader(shader))
            .init_gpu_resource::<SpecializedRenderPipelines<BlendedSkyboxPipeline>>()
            .add_systems(ExtractSchedule, extract_blended_skybox)
            .add_systems(RenderStartup, init_blended_skybox_pipeline)
            .add_systems(
                Render,
                (
                    prepare_blended_skybox_pipelines.in_set(RenderSystems::Prepare),
                    prepare_blended_skybox_bind_groups.in_set(RenderSystems::PrepareBindGroups),
                ),
            );
    }
}

impl bevy::render::sync_component::SyncComponent<bevy::render::RenderApp, BlendedSkyboxPlugin> for BlendedSkybox {
    type Target = (
        Self,
        BlendedSkyboxUniforms,
        bevy::core_pipeline::skybox::SkyboxPipelineId,
        bevy::core_pipeline::skybox::SkyboxBindGroup,
    );
}

fn extract_blended_skybox(
    mut commands: Commands,
    mut previous_len: Local<usize>,
    query: bevy::render::Extract<Query<(bevy::render::sync_world::RenderEntity, &BlendedSkybox, Option<&Exposure>)>>,
) {
    let mut values = Vec::with_capacity(*previous_len);
    for (entity, skybox, exposure) in &query {
        let exposure = exposure.map(Exposure::exposure).unwrap_or_else(|| Exposure::default().exposure());
        let uniforms = BlendedSkyboxUniforms {
            transform_a: Transform::from_rotation(skybox.rotation_a.inverse()).to_matrix(),
            transform_b: Transform::from_rotation(skybox.rotation_b.inverse()).to_matrix(),
            color_a: skybox.color_a,
            color_b: skybox.color_b,
            brightness_a: skybox.brightness_a * exposure,
            brightness_b: skybox.brightness_b * exposure,
            blend: skybox.blend,
            alpha: skybox.alpha,
            has_a: u32::from(skybox.cubemap_a.is_some()),
            has_b: u32::from(skybox.cubemap_b.is_some()),
            _pad0: 0,
            _pad1: 0,
        };
        values.push((entity, (skybox.clone(), uniforms)));
    }
    *previous_len = values.len();
    commands.try_insert_batch(values);
}

#[derive(Resource)]
struct BlendedSkyboxPipeline {
    bind_group_layout: bevy::render::render_resource::BindGroupLayoutDescriptor,
    shader: Handle<bevy::shader::Shader>,
}

fn init_blended_skybox_pipeline(mut commands: Commands, shader: Res<BlendedSkyboxShader>) {
    use bevy::render::render_resource::binding_types::{sampler, texture_cube, uniform_buffer};
    use bevy::render::render_resource::{
        BindGroupLayoutDescriptor, BindGroupLayoutEntries, SamplerBindingType, ShaderStages, TextureSampleType,
    };
    use bevy::render::view::ViewUniform;

    let bind_group_layout = BindGroupLayoutDescriptor::new(
        "editor_blended_skybox_bind_group_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_cube(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                texture_cube(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                uniform_buffer::<ViewUniform>(true).visibility(ShaderStages::VERTEX_FRAGMENT),
                uniform_buffer::<BlendedSkyboxUniforms>(true),
            ),
        ),
    );
    commands.insert_resource(BlendedSkyboxPipeline { bind_group_layout, shader: shader.0.clone() });
}

#[derive(PartialEq, Eq, Hash, Clone, Copy)]
struct BlendedSkyboxPipelineKey {
    target_format: TextureFormat,
    samples: u32,
    depth_format: TextureFormat,
}

impl bevy::render::render_resource::SpecializedRenderPipeline for BlendedSkyboxPipeline {
    type Key = BlendedSkyboxPipelineKey;

    fn specialize(&self, key: Self::Key) -> bevy::render::render_resource::RenderPipelineDescriptor {
        use bevy::render::render_resource::{
            BlendState, ColorTargetState, ColorWrites, CompareFunction, DepthBiasState, DepthStencilState,
            FragmentState, MultisampleState, RenderPipelineDescriptor, StencilFaceState, StencilState, VertexState,
        };

        RenderPipelineDescriptor {
            label: Some("editor_blended_skybox_pipeline".into()),
            layout: vec![self.bind_group_layout.clone()],
            vertex: VertexState { shader: self.shader.clone(), entry_point: Some("skybox_vertex".into()), ..default() },
            depth_stencil: Some(DepthStencilState {
                format: key.depth_format,
                depth_write_enabled: Some(false),
                depth_compare: Some(CompareFunction::GreaterEqual),
                stencil: StencilState {
                    front: StencilFaceState::IGNORE,
                    back: StencilFaceState::IGNORE,
                    read_mask: 0,
                    write_mask: 0,
                },
                bias: DepthBiasState { constant: 0, slope_scale: 0.0, clamp: 0.0 },
            }),
            multisample: MultisampleState { count: key.samples, mask: !0, alpha_to_coverage_enabled: false },
            fragment: Some(FragmentState {
                shader: self.shader.clone(),
                entry_point: Some("skybox_fragment".into()),
                targets: vec![Some(ColorTargetState {
                    format: key.target_format,
                    blend: Some(BlendState::ALPHA_BLENDING),
                    write_mask: ColorWrites::ALL,
                })],
                ..default()
            }),
            ..default()
        }
    }
}

fn prepare_blended_skybox_pipelines(
    mut commands: Commands,
    pipeline_cache: Res<bevy::render::render_resource::PipelineCache>,
    mut pipelines: ResMut<bevy::render::render_resource::SpecializedRenderPipelines<BlendedSkyboxPipeline>>,
    pipeline: Res<BlendedSkyboxPipeline>,
    cameras: Query<(Entity, &bevy::render::view::ExtractedView, &bevy::render::view::Msaa), With<BlendedSkybox>>,
) {
    use bevy::core_pipeline::core_3d::CORE_3D_DEPTH_FORMAT;
    use bevy::core_pipeline::skybox::SkyboxPipelineId;

    for (entity, view, msaa) in &cameras {
        let pipeline_id = pipelines.specialize(
            &pipeline_cache,
            &pipeline,
            BlendedSkyboxPipelineKey {
                target_format: view.target_format,
                samples: msaa.samples(),
                depth_format: CORE_3D_DEPTH_FORMAT,
            },
        );
        commands.entity(entity).insert(SkyboxPipelineId(pipeline_id));
    }
}

#[expect(clippy::too_many_arguments)]
fn prepare_blended_skybox_bind_groups(
    mut commands: Commands,
    pipeline: Res<BlendedSkyboxPipeline>,
    view_uniforms: Res<bevy::render::view::ViewUniforms>,
    skybox_uniforms: Res<bevy::render::extract_component::ComponentUniforms<BlendedSkyboxUniforms>>,
    images: Res<bevy::render::render_asset::RenderAssets<bevy::render::texture::GpuImage>>,
    render_device: Res<bevy::render::renderer::RenderDevice>,
    pipeline_cache: Res<bevy::render::render_resource::PipelineCache>,
    views: Query<(
        Entity,
        &BlendedSkybox,
        &bevy::render::extract_component::DynamicUniformIndex<BlendedSkyboxUniforms>,
    )>,
) {
    use bevy::core_pipeline::skybox::SkyboxBindGroup;
    use bevy::render::render_resource::BindGroupEntries;

    for (entity, skybox, skybox_uniform_index) in &views {
        if skybox.alpha <= 0.0 || (skybox.cubemap_a.is_none() && skybox.cubemap_b.is_none()) {
            commands.entity(entity).remove::<SkyboxBindGroup>();
            continue;
        }
        let handle_a = skybox.cubemap_a.as_ref().or(skybox.cubemap_b.as_ref());
        let handle_b = skybox.cubemap_b.as_ref().or(skybox.cubemap_a.as_ref());
        if let (Some(handle_a), Some(handle_b), Some(view_uniforms), Some(skybox_uniforms)) =
            (handle_a, handle_b, view_uniforms.uniforms.binding(), skybox_uniforms.binding())
            && let (Some(image_a), Some(image_b)) = (images.get(handle_a), images.get(handle_b))
            && is_cubemap(image_a)
            && is_cubemap(image_b)
        {
            let bind_group = render_device.create_bind_group(
                "editor_blended_skybox_bind_group",
                &pipeline_cache.get_bind_group_layout(&pipeline.bind_group_layout),
                &BindGroupEntries::sequential((
                    &image_a.texture_view,
                    &image_a.sampler,
                    &image_b.texture_view,
                    &image_b.sampler,
                    view_uniforms,
                    skybox_uniforms,
                )),
            );
            commands.entity(entity).insert(SkyboxBindGroup((bind_group, skybox_uniform_index.index())));
        } else {
            commands.entity(entity).remove::<SkyboxBindGroup>();
        }
    }
}

fn is_cubemap(image: &bevy::render::texture::GpuImage) -> bool {
    image.texture_view_descriptor.as_ref().and_then(|desc| desc.dimension) == Some(TextureViewDimension::Cube)
}
