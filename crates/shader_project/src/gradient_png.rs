//! Baked texture of a gradient collection (gradient design §5): a [`SAMPLES`]×[`SLOTS`] RGBA8 PNG of
//! [`GradientCollection::bake_rgba8`] (sRGB-encoded bytes, row = slot), written to
//! `assets/shaders/gradients/<collection>.png`.

use shader_graph_core::gradient::{GradientCollection, SAMPLES, SLOTS};
use shader_graph_core::texture::{FilterMode, SamplerSettings, TextureSettings};

/// Loader settings of a baked collection (its manifest entry): sRGB (a sample returns linear values), `ClampToEdge`,
/// `Linear` mag / min filters, `Nearest` mipmap filter (the PNG has no mips; the shader samples level 0).
pub fn texture_settings() -> TextureSettings {
    TextureSettings {
        is_srgb: true,
        sampler: Some(SamplerSettings {
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// The PNG of collection `c`. Byte-deterministic on every platform (the writer compares bytes, CI fails on a diff
/// after `regen`): fixed encoder settings and the pure-Rust `fdeflate` backend, never `flate2`, whose output depends
/// on the backend feature-unification picks. No `sRGB` / `gAMA` chunk: the game reads `is_srgb` from the loader
/// settings of the manifest entry.
pub fn encode(c: &GradientCollection) -> Vec<u8> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, SAMPLES as u32, SLOTS as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_deflate_compression(png::DeflateCompression::FdeflateUltraFast);
    encoder.set_filter(png::Filter::Adaptive);
    // Writing into a `Vec` with a valid header and exactly `width * height * 4` bytes cannot fail.
    let mut writer = encoder.write_header().expect("PNG header");
    writer.write_image_data(&c.bake_rgba8()).expect("PNG data");
    writer.finish().expect("PNG end");
    out
}

#[cfg(test)]
mod tests {
    use shader_graph_core::gradient::{SAMPLES, SLOTS, Stop};

    use super::*;

    #[test]
    fn encode_is_deterministic_and_decodes() {
        let mut next = 0;
        let mut c = GradientCollection::new(|| {
            next += 1;
            next
        });
        c.slots[0].stops.push(Stop { t: 0.0, color: [1.0, 0.0, 0.0, 1.0], mid: 0.5 });
        c.slots[0].stops.push(Stop { t: 1.0, color: [0.0, 0.0, 1.0, 0.5], mid: 0.3 });
        c.slots[7].stops.push(Stop { t: 0.4, color: [0.2, 0.7, 0.1, 1.0], mid: 0.5 });
        let png = encode(&c);
        assert_eq!(png, encode(&c.clone()), "encoding is byte-deterministic");

        let decoder = png::Decoder::new(std::io::Cursor::new(png.as_slice()));
        let mut reader = decoder.read_info().expect("PNG header");
        let info = reader.info();
        assert_eq!((info.width as usize, info.height as usize), (SAMPLES, SLOTS));
        assert_eq!((info.color_type, info.bit_depth), (png::ColorType::Rgba, png::BitDepth::Eight));
        let mut pixels = vec![0; reader.output_buffer_size().expect("buffer size")];
        let frame = reader.next_frame(&mut pixels).expect("PNG data");
        pixels.truncate(frame.buffer_size());
        assert_eq!(pixels, c.bake_rgba8());
    }
}
