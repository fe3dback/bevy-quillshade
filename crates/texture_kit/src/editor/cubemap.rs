//! Shared 3D cube-map preview (`CubeMapPreview`) for `texture_kit`: displays 6-face cube maps across all domains in
//! two selectable modes:
//! - [`CubeMapViewMode::Sphere`] (`"3D"`): a 3D sphere inside the cube-map skybox.
//! - [`CubeMapViewMode::Strip2d`] (`"2D"`): all 6 faces stacked top-to-bottom in the 2D strip view.

use std::collections::{BTreeMap, BTreeSet};

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::RenderLayers;
use bevy::camera::{
    Camera, Camera3d, ClearColorConfig, Exposure, Hdr, PerspectiveProjection, Projection, RenderTarget,
};
use bevy::core_pipeline::Skybox;
use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::input::mouse::MouseScrollPixelsPerLine;
use bevy::light::EnvironmentMapLight;
use bevy::pbr::{MeshMaterial3d, StandardMaterial};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages};
use editor_core::preview::{PreviewCallbacks, PreviewEntity, PreviewKindId};
use editor_core::state::editor::Editor;
use editor_core::theme::{TEXT_DIM, text};

use super::preview::{PreviewCubeControls, ShownState};
use super::{PreviewArea, PreviewPic, TextureDomain, preview};
use crate::TextureShape;

/// Preview kind identifier for `texture_kit`'s shared 3D cube-map preview studio.
pub const CUBEMAP_PREVIEW_KIND: PreviewKindId = PreviewKindId("texture_kit.cubemap_3d");

const RTT_SIZE: u32 = 512;
const DEFAULT_YAW: f32 = 0.4;
const DEFAULT_PITCH: f32 = 0.2;
const DEFAULT_DISTANCE: f32 = 3.2;
const MIN_DISTANCE: f32 = 1.8;
const MAX_DISTANCE: f32 = 8.0;
const DEFAULT_FOV_DEG: f32 = 55.0;
const MAX_PITCH: f32 = 1.48;

/// How a cube-map texture is displayed in the shared texture preview.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CubeMapViewMode {
    /// `"3D"`: 3D reflective sphere inside the cube-map skybox.
    #[default]
    Sphere,
    /// `"2D"`: all 6 faces stacked vertically in the 2D strip view.
    Strip2d,
}

impl CubeMapViewMode {
    pub const ALL: [CubeMapViewMode; 2] = [CubeMapViewMode::Sphere, CubeMapViewMode::Strip2d];

    pub fn label(self) -> &'static str {
        match self {
            CubeMapViewMode::Sphere => "3D",
            CubeMapViewMode::Strip2d => "2D",
        }
    }

    pub fn tip(self) -> &'static str {
        match self {
            CubeMapViewMode::Sphere => "3D sphere inside the cube-map skybox",
            CubeMapViewMode::Strip2d => "All 6 cube-map faces stacked as a 2D strip",
        }
    }

    pub fn caption(self) -> &'static str {
        match self {
            CubeMapViewMode::Sphere => "3D Preview (drag: orbit, wheel: zoom, double-click: reset)",
            CubeMapViewMode::Strip2d => "",
        }
    }
}

marker! {
    /// Shared 3D cube-map preview container in `D`'s [`PreviewArea`], visible when a cube map is viewed in
    /// [`CubeMapViewMode::Sphere`].
    CubeMapPreview
}

marker! {
    /// The UI [`ImageNode`] inside [`CubeMapPreview`] bound to the active session's offscreen 3D render target.
    CubeMapPreviewImage
}

marker! {
    /// Overlay caption inside [`CubeMapPreview`] describing the active 3D mode and mouse gestures.
    CubeMapPreviewCaption
}

/// Active 3D cube-map preview sessions keyed by [`editor_core::preview::PreviewHandle::id`].
#[derive(Resource, Default)]
pub struct CubeMapPreviewSessions {
    pub sessions: BTreeMap<u64, CubeMapPreviewSession>,
    used_layers: BTreeSet<usize>,
}

impl CubeMapPreviewSessions {
    fn alloc_layer(&mut self) -> usize {
        // Allocate from layer 16 upwards first so cube-map preview studios never collide with domain 3D studios.
        let layer = (16..=31).chain(1..16).find(|l| !self.used_layers.contains(l)).unwrap_or(16);
        self.used_layers.insert(layer);
        layer
    }

    fn free_layer(&mut self, layer: usize) {
        self.used_layers.remove(&layer);
    }
}

/// One spawned 3D cube-map preview studio instance (3D sphere inside the cube-map skybox).
pub struct CubeMapPreviewSession {
    pub id: u64,
    pub layer: usize,
    pub camera: Entity,
    pub sphere: Entity,
    pub rtt_image: Option<Handle<Image>>,
    pub sphere_mesh: Option<Handle<Mesh>>,
    pub sphere_material: Option<Handle<StandardMaterial>>,
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
    pub mode: CubeMapViewMode,
}

impl CubeMapPreviewSession {
    /// Camera transform orbiting around the sphere at `Vec3::ZERO` at `self.distance`.
    pub fn camera_transform(&self) -> Transform {
        let rot = Quat::from_euler(EulerRot::YXZ, self.yaw, -self.pitch, 0.0);
        let pos = rot * Vec3::new(0.0, 0.0, self.distance);
        Transform::from_translation(pos).looking_at(Vec3::ZERO, Vec3::Y)
    }
}

/// Returns the [`PreviewCallbacks`] registered with `editor_core` for [`CUBEMAP_PREVIEW_KIND`].
pub fn preview_callbacks() -> PreviewCallbacks {
    PreviewCallbacks { kind: CUBEMAP_PREVIEW_KIND, start: start_cubemap_preview, clean: clean_cubemap_preview }
}

/// Ensures the asset stores and session resource needed by [`start_cubemap_preview`] exist in both windowed and
/// headless test apps.
pub fn setup_cubemap_logic(app: &mut App) {
    if !app.world().contains_resource::<Assets<Image>>() {
        app.init_resource::<Assets<Image>>();
    }
    if !app.world().contains_resource::<Assets<Mesh>>() {
        app.init_resource::<Assets<Mesh>>();
    }
    if !app.world().contains_resource::<Assets<StandardMaterial>>() {
        app.init_resource::<Assets<StandardMaterial>>();
    }
    if !app.world().contains_resource::<CubeMapPreviewSessions>() {
        app.init_resource::<CubeMapPreviewSessions>();
    }
}

/// Spawns the offscreen RTT image, 3D sphere mesh, and `Camera3d` for cube-map preview session `id`.
pub fn start_cubemap_preview(world: &mut World, id: u64) {
    if !world.contains_resource::<CubeMapPreviewSessions>() {
        world.init_resource::<CubeMapPreviewSessions>();
    }
    if world.resource::<CubeMapPreviewSessions>().sessions.contains_key(&id) {
        return;
    }

    let layer = world.resource_mut::<CubeMapPreviewSessions>().alloc_layer();
    let render_layer = RenderLayers::layer(layer);

    let rtt_image = world.get_resource_mut::<Assets<Image>>().map(|mut images| {
        let mut img = Image::new_fill(
            Extent3d { width: RTT_SIZE, height: RTT_SIZE, depth_or_array_layers: 1 },
            TextureDimension::D2,
            &[20, 22, 26, 255],
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::default(),
        );
        img.texture_descriptor.usage =
            TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST | TextureUsages::RENDER_ATTACHMENT;
        images.add(img)
    });

    let sphere_mesh =
        world.get_resource_mut::<Assets<Mesh>>().map(|mut meshes| meshes.add(Sphere::new(1.15).mesh().ico(5).unwrap()));
    let sphere_material = world.get_resource_mut::<Assets<StandardMaterial>>().map(|mut materials| {
        materials.add(StandardMaterial {
            base_color: Color::srgb(0.5, 0.5, 0.5),
            metallic: 0.25,
            perceptual_roughness: 0.5,
            ..default()
        })
    });

    let mut sphere_cmd =
        world.spawn((PreviewEntity(id), render_layer.clone(), Transform::IDENTITY, Visibility::Inherited));
    if let (Some(mesh), Some(mat)) = (sphere_mesh.clone(), sphere_material.clone()) {
        sphere_cmd.insert((Mesh3d(mesh), MeshMaterial3d(mat)));
    }
    let sphere = sphere_cmd.id();

    let mut session = CubeMapPreviewSession {
        id,
        layer,
        camera: Entity::PLACEHOLDER,
        sphere,
        rtt_image: rtt_image.clone(),
        sphere_mesh,
        sphere_material,
        yaw: DEFAULT_YAW,
        pitch: DEFAULT_PITCH,
        distance: DEFAULT_DISTANCE,
        mode: CubeMapViewMode::Sphere,
    };
    let cam_transform = session.camera_transform();
    let mut cam_cmd = world.spawn((
        PreviewEntity(id),
        render_layer,
        Camera3d::default(),
        Camera {
            order: -2,
            is_active: false,
            clear_color: ClearColorConfig::Custom(Color::srgb_u8(20, 22, 26)),
            ..default()
        },
        Projection::Perspective(PerspectiveProjection { fov: DEFAULT_FOV_DEG.to_radians(), ..default() }),
        Hdr,
        Exposure { ev100: 10.0 },
        Tonemapping::TonyMcMapface,
        DebandDither::Enabled,
        cam_transform,
    ));
    if let Some(rtt) = rtt_image {
        cam_cmd.insert(RenderTarget::Image(rtt.into()));
    }
    session.camera = cam_cmd.id();

    world.resource_mut::<CubeMapPreviewSessions>().sessions.insert(id, session);
}

/// Cleans up all GPU assets and session state for cube-map preview `id` (entities tagged with [`PreviewEntity`] are
/// despawned by [`editor_core::preview::sync_previews`]).
pub fn clean_cubemap_preview(world: &mut World, id: u64) {
    let Some(session) = world.get_resource_mut::<CubeMapPreviewSessions>().and_then(|mut s| {
        let removed = s.sessions.remove(&id);
        if let Some(ref sess) = removed {
            s.free_layer(sess.layer);
        }
        removed
    }) else {
        return;
    };

    if let Some(mut images) = world.get_resource_mut::<Assets<Image>>()
        && let Some(rtt) = session.rtt_image
    {
        images.remove(rtt.id());
    }
    if let Some(mut meshes) = world.get_resource_mut::<Assets<Mesh>>()
        && let Some(mesh) = session.sphere_mesh
    {
        meshes.remove(mesh.id());
    }
    if let Some(mut materials) = world.get_resource_mut::<Assets<StandardMaterial>>()
        && let Some(mat) = session.sphere_material
    {
        materials.remove(mat.id());
    }
}

/// Builds the [`CubeMapPreview`] UI scene spawned inside `D`'s [`PreviewArea`].
pub(super) fn cubemap_preview_scene<D: TextureDomain>(mode: CubeMapViewMode, display: Display) -> impl Scene {
    let preview = CubeMapPreview::<D>::default();
    let img = CubeMapPreviewImage::<D>::default();
    let caption = CubeMapPreviewCaption::<D>::default();
    let label = mode.caption();
    bsn! {
        preview
        Node {
            width: percent(100),
            height: percent(100),
            display: (display),
        }
        Children [
            ImageNode
            img
            Node {
                width: percent(100),
                height: percent(100),
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
                @text(label, TEXT_DIM, 11.0)
                caption
            ]
        ]
    }
}

/// Syncs `D`'s [`CubeMapPreview`] widget, topbar mode controls, and 3D session camera / sphere for the active tab.
pub(super) fn update_cubemap_preview<D: TextureDomain>(world: &mut World) {
    let active_info = world.get_resource::<Editor>().and_then(|editor| {
        let tab = editor.tab()?;
        let p = preview::<D>(tab)?;
        let mode = p.cube_mode();
        let handle_id = p.preview_handle().filter(|h| h.is_alive()).map(|h| h.id());
        Some((mode, handle_id))
    });

    let domain_tab_ids: Vec<u64> = world
        .get_resource::<Editor>()
        .map(|editor| {
            editor
                .tabs
                .iter()
                .filter(|t| preview::<D>(t).is_some())
                .filter_map(|t| t.doc().preview_handle().map(|h| h.id()))
                .collect()
        })
        .unwrap_or_default();

    let Some((mode, active_id)) = active_info else {
        // Deactivate any 3D cameras belonging to `D`'s inactive tabs.
        if let Some(sessions) = world.get_resource::<CubeMapPreviewSessions>() {
            let cams: Vec<Entity> =
                domain_tab_ids.iter().filter_map(|id| sessions.sessions.get(id).map(|s| s.camera)).collect();
            for cam_entity in cams {
                if let Ok(mut e) = world.get_entity_mut(cam_entity)
                    && let Some(mut cam) = e.get_mut::<Camera>()
                    && cam.is_active
                {
                    cam.is_active = false;
                }
            }
        }
        return;
    };

    let (is_cubemap, cube_handle) = world
        .get_resource::<ShownState<D>>()
        .and_then(|s| s.0.as_ref())
        .map_or((false, None), |s| (s.is_cubemap, s.cube_handle.clone()));

    let controls_display =
        if is_cubemap || D::POLICY.shape == TextureShape::CubeMap { Display::Flex } else { Display::None };
    {
        let mut q = world.query_filtered::<&mut Node, With<PreviewCubeControls<D>>>();
        for mut node in q.iter_mut(world) {
            if node.display != controls_display {
                node.display = controls_display;
            }
        }
    }

    let show_3d = is_cubemap && mode != CubeMapViewMode::Strip2d && cube_handle.is_some();
    let preview_display = if show_3d { Display::Flex } else { Display::None };
    let pic_display = if show_3d { Display::None } else { Display::Flex };

    {
        let mut q = world.query_filtered::<&mut Node, With<CubeMapPreview<D>>>();
        for mut node in q.iter_mut(world) {
            if node.display != preview_display {
                node.display = preview_display;
            }
        }
    }
    {
        let mut q = world.query_filtered::<&mut Node, With<PreviewPic<D>>>();
        for mut node in q.iter_mut(world) {
            if node.display != pic_display {
                node.display = pic_display;
            }
        }
    }
    {
        let want_caption = mode.caption();
        let mut q = world.query_filtered::<&mut Text, With<CubeMapPreviewCaption<D>>>();
        for mut text in q.iter_mut(world) {
            if text.0 != want_caption {
                text.0 = want_caption.to_string();
            }
        }
    }

    // Deactivate cameras of all other tabs of `D` (or the active tab if it switched to 2D strip mode).
    if let Some(sessions) = world.get_resource::<CubeMapPreviewSessions>() {
        let inactive_cams: Vec<Entity> = domain_tab_ids
            .iter()
            .filter(|&&id| Some(id) != active_id || !show_3d)
            .filter_map(|id| sessions.sessions.get(id).map(|s| s.camera))
            .collect();
        for cam_entity in inactive_cams {
            if let Ok(mut e) = world.get_entity_mut(cam_entity)
                && let Some(mut cam) = e.get_mut::<Camera>()
                && cam.is_active
            {
                cam.is_active = false;
            }
        }
    }

    let (Some(id), Some(cube_img)) = (active_id, cube_handle) else { return };
    if !show_3d {
        if let Some(mut sessions) = world.get_resource_mut::<CubeMapPreviewSessions>()
            && let Some(sess) = sessions.sessions.get_mut(&id)
        {
            sess.mode = mode;
        }
        return;
    }

    if !world.contains_resource::<CubeMapPreviewSessions>()
        || !world.resource::<CubeMapPreviewSessions>().sessions.contains_key(&id)
    {
        start_cubemap_preview(world, id);
    }

    let viewport_size: Option<UVec2> = {
        let mut q = world.query_filtered::<&ComputedNode, With<CubeMapPreview<D>>>();
        q.iter(world).find_map(|node| {
            let sz = (node.size() * node.inverse_scale_factor()).round().as_uvec2();
            (sz.x >= 64 && sz.y >= 64).then_some(UVec2::new(sz.x.min(1920), sz.y.min(1440)))
        })
    };

    let (camera_entity, sphere_entity, rtt_handle, cam_tf, fov_rad) = {
        let mut sessions = world.resource_mut::<CubeMapPreviewSessions>();
        let Some(sess) = sessions.sessions.get_mut(&id) else { return };
        sess.mode = mode;
        let tf = sess.camera_transform();
        let fov = DEFAULT_FOV_DEG.to_radians();
        (sess.camera, sess.sphere, sess.rtt_image.clone(), tf, fov)
    };

    if let (Some(rtt), Some(want_size)) = (rtt_handle.as_ref(), viewport_size)
        && let Some(mut images) = world.get_resource_mut::<Assets<Image>>()
        && images.get(rtt.id()).is_some_and(|img| img.size() != want_size)
        && let Some(mut img) = images.get_mut(rtt.id())
    {
        img.resize(Extent3d { width: want_size.x, height: want_size.y, depth_or_array_layers: 1 });
    }

    if let Ok(mut sphere_ent) = world.get_entity_mut(sphere_entity)
        && let Some(mut vis) = sphere_ent.get_mut::<Visibility>()
        && *vis != Visibility::Inherited
    {
        *vis = Visibility::Inherited;
    }

    if let Ok(mut cam_ent) = world.get_entity_mut(camera_entity) {
        if let Some(mut cam) = cam_ent.get_mut::<Camera>()
            && !cam.is_active
        {
            cam.is_active = true;
        }
        if let Some(mut tf) = cam_ent.get_mut::<Transform>()
            && *tf != cam_tf
        {
            *tf = cam_tf;
        }
        if let Some(mut proj) = cam_ent.get_mut::<Projection>()
            && let Projection::Perspective(ref mut p) = *proj
            && (p.fov - fov_rad).abs() > 1e-5
        {
            p.fov = fov_rad;
        }
        let skybox = Skybox { image: Some(cube_img.clone()), brightness: 1000.0, rotation: Quat::IDENTITY };
        if let Some(mut existing) = cam_ent.get_mut::<Skybox>() {
            if existing.image != skybox.image || existing.brightness != skybox.brightness {
                *existing = skybox;
            }
        } else {
            cam_ent.insert(skybox);
        }
        let ibl = EnvironmentMapLight {
            diffuse_map: cube_img.clone(),
            specular_map: cube_img,
            intensity: 1000.0,
            rotation: Quat::IDENTITY,
            affects_lightmapped_mesh_diffuse: true,
        };
        if let Some(mut existing) = cam_ent.get_mut::<EnvironmentMapLight>() {
            if existing.diffuse_map != ibl.diffuse_map || existing.specular_map != ibl.specular_map {
                *existing = ibl;
            }
        } else {
            cam_ent.insert(ibl);
        }
    }

    if let Some(rtt) = rtt_handle {
        let mut q = world.query_filtered::<&mut ImageNode, With<CubeMapPreviewImage<D>>>();
        for mut img_node in q.iter_mut(world) {
            if img_node.image != rtt {
                img_node.image = rtt.clone();
            }
        }
    }
}

/// Returns `preview_id` when `D`'s active tab is currently showing the 3D [`CubeMapPreview`].
pub(super) fn active_3d_cubemap<D: TextureDomain>(editor: &Editor, shown: &ShownState<D>) -> Option<u64> {
    let tab = editor.tab()?;
    let p = preview::<D>(tab)?;
    let s = shown.0.as_ref()?;
    if !s.is_cubemap || s.cube_handle.is_none() || p.cube_mode() == CubeMapViewMode::Strip2d {
        return None;
    }
    Some(p.preview_handle()?.id())
}

pub(super) fn cubemap_drag<D: TextureDomain>(
    mut ev: On<PointerDrag>,
    previews: Query<(), Or<(With<CubeMapPreview<D>>, With<PreviewArea<D>>)>>,
    editor: Res<Editor>,
    shown: Res<ShownState<D>>,
    mut sessions: ResMut<CubeMapPreviewSessions>,
) {
    if !previews.contains(ev.entity) {
        return;
    }
    let Some(id) = active_3d_cubemap::<D>(&editor, &shown) else { return };
    let Some(sess) = sessions.sessions.get_mut(&id) else { return };
    ev.propagate(false);
    sess.yaw -= ev.delta.x * 0.008;
    sess.pitch = (sess.pitch + ev.delta.y * 0.008).clamp(-MAX_PITCH, MAX_PITCH);
}

pub(super) fn cubemap_scroll<D: TextureDomain>(
    mut ev: On<PointerScroll>,
    previews: Query<(), Or<(With<CubeMapPreview<D>>, With<PreviewArea<D>>)>>,
    ratio: Res<MouseScrollPixelsPerLine>,
    editor: Res<Editor>,
    shown: Res<ShownState<D>>,
    mut sessions: ResMut<CubeMapPreviewSessions>,
) {
    if !previews.contains(ev.entity) {
        return;
    }
    let Some(id) = active_3d_cubemap::<D>(&editor, &shown) else { return };
    let Some(sess) = sessions.sessions.get_mut(&id) else { return };
    ev.propagate(false);
    let notches = ev.to_lines(&ratio).y;
    if notches == 0.0 {
        return;
    }
    sess.distance = (sess.distance * 0.9f32.powf(notches)).clamp(MIN_DISTANCE, MAX_DISTANCE);
}

pub(super) fn cubemap_double_click<D: TextureDomain>(
    ev: On<PointerClick>,
    previews: Query<(), Or<(With<CubeMapPreview<D>>, With<PreviewArea<D>>)>>,
    editor: Res<Editor>,
    shown: Res<ShownState<D>>,
    mut sessions: ResMut<CubeMapPreviewSessions>,
) {
    if !previews.contains(ev.entity) || ev.button != PointerButton::Primary || ev.count < 2 {
        return;
    }
    let Some(id) = active_3d_cubemap::<D>(&editor, &shown) else { return };
    let Some(sess) = sessions.sessions.get_mut(&id) else { return };
    sess.yaw = DEFAULT_YAW;
    sess.pitch = DEFAULT_PITCH;
    sess.distance = DEFAULT_DISTANCE;
}
