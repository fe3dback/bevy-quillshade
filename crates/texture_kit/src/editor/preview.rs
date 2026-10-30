//! The image preview shared by the image and Texture tabs (design §6.2): the active tab's image ([`PreviewDoc`])
//! decoded through [`GameImageCache`], on a checkerboard. The mouse wheel zooms around the pointer, a drag pans, a
//! double click fits the image again; "Tile 3x3" repeats the image so the address modes show, R | G | B | All shows
//! one channel in grey or the image. Each kind's view holds a [`preview_area`] and puts [`preview_controls`] into its
//! editor topbar; the camera (zoom, pan) is kept per tab, keyed by `(view, image)`.
//!
//! Every component, resource, system and observer is `D`'s own: two texture domains each drive only their previews.

use std::marker::PhantomData;

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::input::mouse::MouseScrollPixelsPerLine;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::ui::ui_transform::UiGlobalTransform;
use editor_core::state::editor::Editor;
use editor_core::tab::{Tab, ViewKindId};
use editor_core::theme::{TEXT_DIM, text};
use editor_core::ui::props::checkbox;
use editor_core::ui::widgets::{Caption, GroupButton, button_group};

use super::{Channel, GameImageCache, PreviewChannel, TextureDomain, TextureProp, describe, preview, texture_key};
use crate::TextureSettings;

/// A tab kind with the shared image preview (design §6.2: image and Texture tabs): the image it shows and the
/// preview's view state (channel, tiling), which is editor-only: not saved, not undoable. [`super::preview`] gives it
/// for any tab of `D` that has one.
pub trait PreviewDoc {
    /// What [`GameImageCache::get_channel`] decodes (a path under `assets/` or an `@internal/…` id) and the settings
    /// to decode it with.
    fn preview_image(&self) -> (&str, TextureSettings);
    fn channel(&self) -> Channel;
    /// Shows `channel` only (`None`: the whole image).
    fn set_channel(&mut self, channel: Channel);
    /// The preview repeats the image 3x3, so the address modes show.
    fn tiled(&self) -> bool;
    fn set_tiled(&mut self, tiled: bool);
}

/// Space kept around the fitted image (logical px).
const PAD: f32 = 16.0;
const ERROR: Color = Color::srgb(0.95, 0.42, 0.42);
/// Zoom limits (image px per logical px).
const ZOOM_MIN: f32 = 1.0 / 16.0;
const ZOOM_MAX: f32 = 64.0;

/// Hotkeys of the preview, for the kinds' right sidebars.
pub const PREVIEW_HOTKEYS: [(&str, &str); 3] =
    [("Wheel", "Zoom the preview"), ("Drag", "Pan the preview"), ("Double-click", "Fit the preview")];

/// The preview area of one of `D`'s views (clips the image, takes wheel / drag / double click); the image of the
/// active tab is shown in the area of its kind.
#[derive(Component)]
pub struct PreviewArea<D>(pub ViewKindId, PhantomData<fn() -> D>);

impl<D> PreviewArea<D> {
    pub fn new(view: ViewKindId) -> Self {
        PreviewArea(view, PhantomData)
    }
}

impl<D> Clone for PreviewArea<D> {
    fn clone(&self) -> Self {
        PreviewArea::new(self.0)
    }
}

/// The shown image's frame (checkerboard background) and the image's size in pixels; positioned by [`place`].
#[derive(Component)]
pub struct PreviewPic<D>(pub UVec2, PhantomData<fn() -> D>);

marker! {
    /// The zoom (`Fit` or a percentage), in [`preview_controls`].
    PreviewZoomText
}

marker! {
    /// The image info (`WxH | format | mips` or `n/a`), wherever one of `D`'s kinds shows it; kept current by the
    /// preview.
    PreviewInfoText
}

/// 2×2 checkerboard tiled behind the image.
#[derive(Resource)]
struct Checker<D>(Handle<Image>, PhantomData<fn() -> D>);

impl<D> FromWorld for Checker<D> {
    fn from_world(world: &mut World) -> Self {
        let (a, b) = ([58, 59, 64, 255], [44, 45, 50, 255]);
        let mut image = Image::new(
            Extent3d { width: 2, height: 2, depth_or_array_layers: 1 },
            TextureDimension::D2,
            [a, b, b, a].concat(),
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::RENDER_WORLD,
        );
        image.sampler = ImageSampler::nearest();
        Checker(world.resource_mut::<Assets<Image>>().add(image), PhantomData)
    }
}

/// A preview's camera: `scale` = image px → logical px (`None`: fit the area), `offset` from the area's centre.
#[derive(Clone, Copy, Default)]
struct PreviewCamera {
    scale: Option<f32>,
    offset: Vec2,
    /// The scale [`place`] used last (the start of a wheel zoom from "fit").
    shown: f32,
}

/// Which preview a camera belongs to: the tab's kind and its image key (unique among open tabs of a kind: one
/// Texture asset per game file, one image tab per file).
type CameraKey = (ViewKindId, String);

/// The shown preview's camera, and those of the other open preview tabs of `D` (restored when shown again).
#[derive(Resource)]
struct Cameras<D> {
    /// The preview shown (compared every frame without allocating; cloned only when another one is shown).
    key: Option<CameraKey>,
    cam: PreviewCamera,
    others: Vec<(CameraKey, PreviewCamera)>,
    domain: PhantomData<fn() -> D>,
}

impl<D> Default for Cameras<D> {
    fn default() -> Self {
        Cameras { key: None, cam: PreviewCamera::default(), others: Vec::new(), domain: PhantomData }
    }
}

impl<D: TextureDomain> Cameras<D> {
    fn is(&self, kind: ViewKindId, image: &str) -> bool {
        self.key.as_ref().is_some_and(|(k, i)| *k == kind && i == image)
    }

    /// Shows the camera of `(kind, image)`, keeping the current one; cameras of closed tabs are dropped.
    fn switch(&mut self, kind: ViewKindId, image: &str, tabs: &[Tab]) {
        if let Some(key) = self.key.take() {
            self.others.push((key, self.cam));
        }
        let open = |(k, i): &CameraKey| {
            tabs.iter().any(|t| t.doc().view() == *k && preview::<D>(t).is_some_and(|p| p.preview_image().0 == i))
        };
        self.others.retain(|(key, _)| open(key));
        self.cam = match self.others.iter().position(|((k, i), _)| *k == kind && i == image) {
            Some(at) => self.others.swap_remove(at).1,
            None => PreviewCamera::default(),
        };
        self.key = Some((kind, image.to_string()));
    }
}

/// `D`'s image previews: the checkerboard, the cameras, the redraw and the zoom / pan / fit gestures.
pub struct ImagePreviewPlugin<D>(PhantomData<fn() -> D>);

impl<D> Default for ImagePreviewPlugin<D> {
    fn default() -> Self {
        ImagePreviewPlugin(PhantomData)
    }
}

impl<D: TextureDomain> Plugin for ImagePreviewPlugin<D> {
    fn build(&self, app: &mut App) {
        app.init_resource::<Checker<D>>()
            .init_resource::<Cameras<D>>()
            .add_systems(Update, (redraw::<D>, place::<D>).chain().after(editor_core::apply_actions))
            .add_observer(zoom::<D>)
            .add_observer(pan::<D>)
            .add_observer(fit_on_double_click::<D>);
    }
}

/// The preview area of `D`'s view `kind`; grows in its parent.
pub fn preview_area<D: TextureDomain>(kind: ViewKindId) -> impl Scene {
    let area = PreviewArea::<D>::new(kind);
    bsn! {
        // The bottom margin keeps the image clear of the file type watermark.
        area
        Node {
            flex_grow: 1.0,
            min_width: px(0),
            min_height: px(0),
            margin: UiRect::bottom(px(56)),
            overflow: Overflow::clip(),
        }
    }
}

/// The preview's editor topbar controls: R | G | B | All, Tile 3x3 and the zoom.
pub fn preview_controls<D: TextureDomain>(p: &dyn PreviewDoc) -> Vec<Box<dyn Scene>> {
    let channels = [(Some(0), "R"), (Some(1), "G"), (Some(2), "B"), (None, "All")].map(|(c, label)| {
        let tip = match c {
            Some(_) => format!("Preview only the {label} channel, in grey"),
            None => "Preview the image".to_string(),
        };
        let action = PreviewChannel::<D>::new(c).into();
        GroupButton { caption: Caption::Text(label.into()), action, selected: p.channel() == c, disabled: false, tip }
    });
    let zoom = PreviewZoomText::<D>::default();
    vec![
        Box::new(button_group(channels.into())),
        checkbox("Tile 3x3", texture_key::<D>(TextureProp::PreviewTiled), p.tiled()),
        Box::new(bsn! { @text("", TEXT_DIM, 12.0) zoom Node { min_width: px(48) } }),
    ]
}

/// What the preview shows: the tab and preview state that determine the decoded image, plus the info line for any
/// newly spawned [`PreviewInfoText`].
#[derive(PartialEq)]
struct ShownKey {
    kind: ViewKindId,
    root: std::path::PathBuf,
    image: String,
    settings: TextureSettings,
    tiled: bool,
    channel: Option<usize>,
    revision: u64,
    formats: bevy::image::CompressedImageFormats,
    clamp_to_border: bool,
}

struct Shown {
    key: ShownKey,
    line: String,
}

/// Decodes the active tab's image when the tab or preview state changes and rebuilds the preview; drops the image once
/// no preview tab is active.
fn redraw<D: TextureDomain>(
    editor: Res<Editor>,
    (mut cache, mut images, checker): (ResMut<GameImageCache<D>>, ResMut<Assets<Image>>, Res<Checker<D>>),
    (areas, mut info): (Query<(Entity, &PreviewArea<D>)>, Query<&mut Text, With<PreviewInfoText<D>>>),
    mut cams: ResMut<Cameras<D>>,
    mut last: Local<Option<Shown>>,
    mut commands: Commands,
) {
    let clear = |commands: &mut Commands| {
        for (e, _) in &areas {
            commands.entity(e).despawn_children();
        }
    };
    let (Some(tab), Some(project)) = (editor.tab(), &editor.project) else {
        if last.take().is_some() {
            clear(&mut commands);
        }
        return;
    };
    let kind = tab.doc().view();
    let (Some(p), Some((area, _))) = (preview::<D>(tab), areas.iter().find(|(_, a)| a.0 == kind)) else {
        if last.take().is_some() {
            clear(&mut commands);
        }
        return;
    };
    let (image, settings) = p.preview_image();
    if !cams.is(kind, image) {
        cams.switch(kind, image, &editor.tabs);
    }
    let key = ShownKey {
        kind,
        root: project.root.clone(),
        image: image.to_string(),
        settings,
        tiled: p.tiled(),
        channel: p.channel(),
        revision: tab.doc().revision(),
        formats: cache.formats,
        clamp_to_border: cache.clamp_to_border,
    };
    if let Some(shown) = last.as_ref().filter(|s| s.key == key) {
        for mut text in &mut info {
            if text.0 != shown.line {
                text.0 = shown.line.clone();
            }
        }
        return;
    }
    let result = cache.get_channel(project, &key.image, &key.settings, key.channel, &mut images);
    let line = match &result {
        Ok((h, _)) => images.get(h).map_or_else(|| "n/a".to_string(), describe),
        Err(_) => "n/a".to_string(),
    };
    for mut text in &mut info {
        if text.0 != line {
            text.0 = line.clone();
        }
    }
    *last = Some(Shown { key, line });
    clear(&mut commands);
    match result {
        Ok((handle, grey)) => {
            let Some(size) = images.get(&handle).map(Image::size) else { return };
            let tiled = NodeImageMode::Tiled { tile_x: true, tile_y: true, stretch_value: 6.0 };
            let frame = commands
                .spawn((
                    ImageNode::new(checker.0.clone()).with_mode(tiled),
                    PreviewPic::<D>(size, PhantomData),
                    Node { position_type: PositionType::Absolute, ..default() },
                    Pickable::IGNORE,
                    ChildOf(area),
                ))
                .id();
            // A channel the cache could not turn grey (compressed formats) is shown tinted instead.
            let tint = match p.channel() {
                Some(c) if !grey => Color::srgb((c == 0) as u8 as f32, (c == 1) as u8 as f32, (c == 2) as u8 as f32),
                _ => Color::WHITE,
            };
            let mut image = ImageNode::new(handle).with_color(tint);
            if p.tiled() {
                // UVs -1..2: the sampler's address modes decide what lies around the image.
                let s = size.as_vec2();
                image.rect = Some(Rect::from_corners(-s, 2.0 * s));
            }
            commands.spawn((
                image,
                Node { width: percent(100), height: percent(100), ..default() },
                Pickable::IGNORE,
                ChildOf(frame),
            ));
        }
        Err(e) => {
            commands.spawn_scene(bsn! { @text(e, ERROR, 14.0) Node { margin: UiRect::all(px(PAD)) } ChildOf(area) });
        }
    }
}

/// Image size shown with tiling (3x3 when tiled).
fn content_px<D>(pic: &PreviewPic<D>, tiled: bool) -> Vec2 {
    pic.0.as_vec2().max(Vec2::ONE) * if tiled { 3.0 } else { 1.0 }
}

/// The scale that fits `content` into the area (small images are enlarged).
fn fit_scale(area: Vec2, content: Vec2) -> f32 {
    let room = (area - Vec2::splat(2.0 * PAD)).max(Vec2::ONE);
    (room / content).min_element()
}

/// Positions the image in the active kind's area: fitted, or at the camera's scale and offset from the centre.
fn place<D: TextureDomain>(
    editor: Res<Editor>,
    areas: Query<(&PreviewArea<D>, &ComputedNode)>,
    mut pics: Query<(&PreviewPic<D>, &mut Node)>,
    mut zoom_text: Query<&mut Text, With<PreviewZoomText<D>>>,
    mut cams: ResMut<Cameras<D>>,
) {
    let Some(tab) = editor.tab() else { return };
    let (Some(p), kind) = (preview::<D>(tab), tab.doc().view()) else { return };
    let Some((_, area)) = areas.iter().find(|(a, _)| a.0 == kind) else { return };
    let size = area.size() * area.inverse_scale_factor();
    let cam = &mut cams.cam;
    for (pic, mut node) in &mut pics {
        let content = content_px(pic, p.tiled());
        let scale = cam.scale.unwrap_or_else(|| fit_scale(size, content));
        cam.shown = scale;
        let shown = content * scale;
        let pos = (size - shown) / 2.0 + cam.offset;
        let (w, h, l, top) = (px(shown.x.round()), px(shown.y.round()), px(pos.x.round()), px(pos.y.round()));
        if node.width != w || node.height != h || node.left != l || node.top != top {
            node.width = w;
            node.height = h;
            node.left = l;
            node.top = top;
        }
    }
    let label = match cam.scale {
        None => "Fit".to_string(),
        Some(s) => format!("{:.0}%", s * 100.0),
    };
    for mut text in &mut zoom_text {
        if text.0 != label {
            text.0 = label.clone();
        }
    }
}

/// The wheel zooms around the pointer (one notch: x1.25).
fn zoom<D: TextureDomain>(
    mut ev: On<PointerScroll>,
    areas: Query<(&ComputedNode, &UiGlobalTransform), With<PreviewArea<D>>>,
    ratio: Res<MouseScrollPixelsPerLine>,
    mut cams: ResMut<Cameras<D>>,
) {
    let Ok((node, transform)) = areas.get(ev.entity) else { return };
    ev.propagate(false);
    let notches = ev.to_lines(&ratio).y;
    if notches == 0.0 {
        return;
    }
    let cam = &mut cams.cam;
    let from = cam.scale.unwrap_or(cam.shown).max(ZOOM_MIN);
    let to = (from * 1.25f32.powf(notches)).clamp(ZOOM_MIN, ZOOM_MAX);
    // The image point under the pointer stays there: offset' = p - (p - offset) * to / from (from the centre).
    let center = transform.translation * node.inverse_scale_factor();
    let p = ev.pointer.position - center;
    cam.offset = p - (p - cam.offset) * (to / from);
    cam.scale = Some(to);
}

fn pan<D: TextureDomain>(
    mut ev: On<PointerDrag>,
    areas: Query<(), With<PreviewArea<D>>>,
    mut cams: ResMut<Cameras<D>>,
) {
    if !areas.contains(ev.entity) {
        return;
    }
    ev.propagate(false);
    let cam = &mut cams.cam;
    if cam.scale.is_none() {
        cam.scale = Some(cam.shown);
    }
    cam.offset += ev.delta;
}

fn fit_on_double_click<D: TextureDomain>(
    ev: On<PointerClick>,
    areas: Query<(), With<PreviewArea<D>>>,
    mut cams: ResMut<Cameras<D>>,
) {
    if areas.contains(ev.entity) && ev.button == PointerButton::Primary && ev.count >= 2 {
        cams.cam.scale = None;
        cams.cam.offset = Vec2::ZERO;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_scale_keeps_aspect_and_enlarges() {
        let area = Vec2::new(400.0, 300.0) + Vec2::splat(2.0 * PAD);
        assert_eq!(fit_scale(area, Vec2::new(100.0, 100.0)), 3.0);
        assert_eq!(fit_scale(area, Vec2::new(800.0, 300.0)), 0.5);
    }
}
