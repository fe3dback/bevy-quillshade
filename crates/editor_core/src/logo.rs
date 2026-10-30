//! The editor logo (welcome screen, About, main menu). The PNGs in the repository's `assets/` are compiled in, so
//! the binary needs no asset directory. The big one is box-filtered down to the shown size (512 → 256, 128, …):
//! sampling a 4x larger texture without mipmaps would alias.

use std::collections::HashMap;

use bevy::asset::RenderAssetUsages;
use bevy::image::{CompressedImageFormats, ImageSampler, ImageType};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::window::PrimaryWindow;

const LOGO: &[u8] = include_bytes!("../../../assets/logo.png");
const LOGO_SMALL: &[u8] = include_bytes!("../../../assets/logo_sm.png");

/// A square logo `size` logical px wide.
#[derive(Component, Clone, Default)]
#[require(Node, ImageNode)]
pub struct Logo {
    pub size: f32,
}

pub fn logo(size: f32) -> impl Scene {
    let l = Logo { size };
    bsn! {
        Node { width: px(size), height: px(size), flex_shrink: 0.0 }
        l
        Pickable::IGNORE
    }
}

pub struct LogoPlugin;

impl Plugin for LogoPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LogoCache>().add_systems(PostUpdate, resolve_logos.before(bevy::ui::UiSystems::Prepare));
    }
}

/// Images by physical px (the small logo under key 0).
#[derive(Resource, Default)]
struct LogoCache(HashMap<u32, Handle<Image>>);

fn resolve_logos(
    mut cache: ResMut<LogoCache>,
    mut images: ResMut<Assets<Image>>,
    window: Query<&Window, With<PrimaryWindow>>,
    mut q: Query<(&Logo, &mut ImageNode), Added<Logo>>,
) {
    let scale = window.single().map_or(1.0, |w| w.scale_factor());
    for (logo, mut image) in &mut q {
        let want = (logo.size * scale).ceil() as u32;
        let key = if want <= 32 { 0 } else { want };
        image.image = cache
            .0
            .entry(key)
            .or_insert_with(|| images.add(if key == 0 { decode(LOGO_SMALL) } else { downsample(decode(LOGO), want) }))
            .clone();
    }
}

/// The compiled-in PNGs are part of the build, so a decode error is a bug (caught by the tests).
fn decode(png: &[u8]) -> Image {
    Image::from_buffer(
        png,
        ImageType::Extension("png"),
        CompressedImageFormats::NONE,
        true,
        ImageSampler::Default,
        RenderAssetUsages::RENDER_WORLD,
    )
    .expect("logo PNG decodes")
}

/// Halves a square RGBA8 image (alpha-weighted 2x2 box filter) while it stays at least `want` px.
fn downsample(image: Image, want: u32) -> Image {
    let mut size = image.width();
    let mut rgba = image.data.expect("decoded image has data");
    while size.is_multiple_of(2) && size / 2 >= want.max(1) {
        let half = size / 2;
        let mut out = vec![0u8; (half * half * 4) as usize];
        for y in 0..half {
            for x in 0..half {
                let mut sum = [0u32; 4];
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let i = (((y * 2 + dy) * size + x * 2 + dx) * 4) as usize;
                    let a = rgba[i + 3] as u32;
                    for c in 0..3 {
                        sum[c] += rgba[i + c] as u32 * a;
                    }
                    sum[3] += a;
                }
                let o = ((y * half + x) * 4) as usize;
                for c in 0..3 {
                    out[o + c] = sum[c].checked_div(sum[3]).unwrap_or(0) as u8;
                }
                out[o + 3] = (sum[3] / 4) as u8;
            }
        }
        (rgba, size) = (out, half);
    }
    Image::new(
        Extent3d { width: size, height: size, depth_or_array_layers: 1 },
        TextureDimension::D2,
        rgba,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logos_decode_and_downsample() {
        let small = decode(LOGO_SMALL);
        assert_eq!((small.width(), small.height()), (32, 32));
        let big = decode(LOGO);
        assert_eq!(big.texture_descriptor.format, TextureFormat::Rgba8UnormSrgb);
        assert_eq!(downsample(big.clone(), 128).width(), 128);
        assert_eq!(downsample(big.clone(), 200).width(), 256);
        assert_eq!(downsample(big, 1024).width(), 512);
    }
}
