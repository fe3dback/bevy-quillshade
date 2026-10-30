//! tiny-skia pixmap helpers for the editor's CPU-drawn previews.

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use resvg::tiny_skia;

/// A pixmap with an opaque background as an image: its premultiplied pixels equal the straight ones Bevy expects.
pub fn opaque_image(pixmap: tiny_skia::Pixmap) -> Image {
    let (width, height) = (pixmap.width(), pixmap.height());
    Image::new(
        Extent3d { width, height, depth_or_array_layers: 1 },
        TextureDimension::D2,
        pixmap.take(),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

pub fn skia_color(c: Color, alpha: f32) -> tiny_skia::Color {
    let c = c.to_srgba();
    tiny_skia::Color::from_rgba(c.red, c.green, c.blue, c.alpha * alpha).unwrap_or(tiny_skia::Color::BLACK)
}
