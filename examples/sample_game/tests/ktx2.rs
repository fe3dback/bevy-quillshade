//! The sample's `.ktx2` textures (made by `cargo run -p shader_project --example png_to_ktx2`, design §4.4) decode in
//! Bevy to exactly the pixels of the `.png` they were made from.

use bevy::asset::RenderAssetUsages;
use bevy::image::{CompressedImageFormats, ImageSampler, ImageType};
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;

fn decode(name: &str, is_srgb: bool) -> Image {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/textures").join(name);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let ext = path.extension().unwrap().to_str().unwrap();
    Image::from_buffer(
        &bytes,
        ImageType::Extension(ext),
        CompressedImageFormats::NONE,
        is_srgb,
        ImageSampler::Default,
        RenderAssetUsages::default(),
    )
    .unwrap_or_else(|e| panic!("{name}: {e}"))
}

#[test]
fn ktx2_textures_match_their_png() {
    // (file stem, `is_srgb` of its Texture asset)
    for (stem, is_srgb) in [("rock_albedo", true), ("water_normal", false)] {
        let ktx2 = decode(&format!("{stem}.ktx2"), is_srgb);
        let png = decode(&format!("{stem}.png"), is_srgb);
        let want = if is_srgb { TextureFormat::Rgba8UnormSrgb } else { TextureFormat::Rgba8Unorm };
        assert_eq!(ktx2.texture_descriptor.format, want, "{stem}");
        assert_eq!(png.texture_descriptor.format, want, "{stem}");
        assert_eq!(ktx2.texture_descriptor.size, png.texture_descriptor.size, "{stem}");
        assert_eq!(ktx2.texture_descriptor.mip_level_count, 1, "{stem}");
        assert!(ktx2.data.is_some() && ktx2.data == png.data, "{stem}: pixels differ");
    }
}
