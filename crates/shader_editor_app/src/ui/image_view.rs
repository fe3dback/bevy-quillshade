//! Read-only view of a game image (design §4.4): the image fitted to the view (aspect kept), a header with the path
//! and `WxH | format | mips` ([`describe`]); a load / decode error is shown as text instead of the image. Transparent parts show a
//! checkerboard.

use std::path::PathBuf;

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use shader_graph_core::texture::TextureSettings;

use super::{HEADER_BG, TEXT, TEXT_DIM, VIEW_BG, ViewOf, text};
use crate::doc_kind::TabDoc;
use crate::game_image::{GameImageCache, describe};
use crate::state::Editor;
use crate::tab::{Tab, ViewKind};

/// Space kept around the image (logical px).
const PAD: f32 = 16.0;
const ERROR: Color = Color::srgb(0.95, 0.42, 0.42);

/// Header row (path, info), rebuilt with the image.
#[derive(Component, Default, Clone)]
pub struct ImageViewBar;

/// The area the image is fitted into; holds the image or the error text.
#[derive(Component, Default, Clone)]
pub struct ImageViewArea;

/// The frame of the shown image (checkerboard background, sized by [`fit`]) and the image's size in pixels.
#[derive(Component)]
pub struct ImageViewPic(pub UVec2);

/// 2×2 checkerboard tiled behind the image.
#[derive(Resource)]
pub(crate) struct Checker(pub(crate) Handle<Image>);

impl FromWorld for Checker {
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
        Checker(world.resource_mut::<Assets<Image>>().add(image))
    }
}

pub struct ImageViewPlugin;

impl Plugin for ImageViewPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Checker>().add_systems(Update, (redraw, fit).chain());
    }
}

pub fn image_view() -> impl Scene {
    bsn! {
        ViewOf(ViewKind::Image)
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
            ImageViewBar
            Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: px(8),
                padding: UiRect::new(px(10), px(4), px(4), px(4)),
            }
            BackgroundColor(HEADER_BG)
            --
            // The bottom margin keeps the image clear of the file type watermark.
            ImageViewArea
            Node {
                flex_grow: 1.0,
                min_height: px(0),
                margin: UiRect::bottom(px(56)),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                overflow: Overflow::clip(),
            }
        ]
    }
}

/// Rebuilds the view when another image tab is active or its file changed; drops the image once its tab is closed
/// (so the cache can free it).
fn redraw(
    editor: Res<Editor>,
    mut cache: ResMut<GameImageCache>,
    mut images: ResMut<Assets<Image>>,
    checker: Res<Checker>,
    (bar, area): (Query<Entity, With<ImageViewBar>>, Query<Entity, With<ImageViewArea>>),
    mut last: Local<Option<(PathBuf, String, u64)>>,
    mut commands: Commands,
) {
    let (Ok(bar), Ok(area)) = (bar.single(), area.single()) else { return };
    let Some(Tab::Image(doc)) = editor.tab() else {
        if last.as_ref().is_some_and(|(_, path, _)| editor.image_index(path).is_none()) {
            *last = None;
            commands.entity(area).despawn_children();
        }
        return;
    };
    let Some(project) = &editor.project else { return };
    // The root too: another project's tab of the same path may be restored without a frame in between.
    let key = (project.root.clone(), doc.path.clone(), doc.revision);
    if last.as_ref() == Some(&key) {
        return;
    }
    *last = Some(key);
    commands.entity(bar).despawn_children();
    commands.entity(area).despawn_children();
    commands.spawn_scene(bsn! { @text(doc.base_title(), TEXT, 13.0) ChildOf(bar) });
    match cache.get(project, &doc.path, &TextureSettings::default(), &mut images) {
        Ok(handle) => {
            let Some(image) = images.get(&handle) else { return };
            let (size, info) = (image.size(), describe(image));
            commands.spawn_scene(bsn! { @text(info, TEXT_DIM, 12.0) ChildOf(bar) });
            let tiled = NodeImageMode::Tiled { tile_x: true, tile_y: true, stretch_value: 6.0 };
            let frame = commands
                .spawn((
                    ImageNode::new(checker.0.clone()).with_mode(tiled),
                    ImageViewPic(size),
                    Node { flex_shrink: 0.0, ..default() },
                    Pickable::IGNORE,
                    ChildOf(area),
                ))
                .id();
            commands.spawn((
                ImageNode::new(handle),
                Node { width: percent(100), height: percent(100), ..default() },
                Pickable::IGNORE,
                ChildOf(frame),
            ));
        }
        Err(e) => {
            commands.spawn_scene(bsn! { @text("n/a", TEXT_DIM, 12.0) ChildOf(bar) });
            commands.spawn_scene(bsn! { @text(e, ERROR, 14.0) Node { margin: UiRect::all(px(PAD)) } ChildOf(area) });
        }
    }
}

/// Scales the image to fit the area, keeping its aspect ratio (small images are enlarged).
fn fit(
    area: Query<&ComputedNode, With<ImageViewArea>>,
    mut pics: Query<(&ImageViewPic, &mut Node), Without<ImageViewArea>>,
) {
    let Ok(area) = area.single() else { return };
    let room = (area.size() * area.inverse_scale_factor() - Vec2::splat(2.0 * PAD)).max(Vec2::ONE);
    for (pic, mut node) in &mut pics {
        let px_size = pic.0.as_vec2().max(Vec2::ONE);
        let shown = px_size * (room / px_size).min_element();
        let (w, h) = (px(shown.x.floor()), px(shown.y.floor()));
        if node.width != w || node.height != h {
            node.width = w;
            node.height = h;
        }
    }
}
