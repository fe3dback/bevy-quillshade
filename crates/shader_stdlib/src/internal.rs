//! Internal textures shipped with the editor (design §2): referenced by `@internal/<name>` ids
//! ([`shader_graph_core::texture::INTERNAL_PREFIX`]) like Texture assets, but not part of any project.

use shader_graph_core::texture::{AddressMode, FilterMode, SamplerSettings, TextureSettings};

/// The texture of texture nodes without a texture (design §1.2, W110).
pub const DEFAULT_TEXTURE: &str = "@internal/default";

/// A built-in texture: embedded image bytes plus loader settings.
pub struct InternalTexture {
    /// `@internal/<name>`.
    pub id: &'static str,
    /// File name inside the game's internal textures directory (`default.png`).
    pub file: &'static str,
    pub bytes: &'static [u8],
    pub settings: fn() -> TextureSettings,
}

/// Every internal texture.
pub const INTERNAL_TEXTURES: &[InternalTexture] = &[InternalTexture {
    id: DEFAULT_TEXTURE,
    file: "default.png",
    bytes: include_bytes!("../textures/default.png"),
    settings: default_settings,
}];

/// The internal texture with this id.
pub fn internal_texture(id: &str) -> Option<&'static InternalTexture> {
    INTERNAL_TEXTURES.iter().find(|t| t.id == id)
}

/// The default texture tiles (Repeat) and is filtered linearly, like UE's default texture.
fn default_settings() -> TextureSettings {
    TextureSettings {
        sampler: Some(SamplerSettings {
            address_mode_u: AddressMode::Repeat,
            address_mode_v: AddressMode::Repeat,
            address_mode_w: AddressMode::Repeat,
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            mipmap_filter: FilterMode::Linear,
            ..Default::default()
        }),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use shader_graph_core::texture::INTERNAL_PREFIX;

    use super::*;

    #[test]
    fn internal_ids_use_the_internal_prefix() {
        assert!(INTERNAL_TEXTURES.iter().all(|t| t.id.starts_with(INTERNAL_PREFIX)));
        assert!(internal_texture(DEFAULT_TEXTURE).is_some());
        assert!(INTERNAL_TEXTURES.iter().all(|t| t.file.ends_with(".png") && !t.file.contains('/')));
    }

    /// `textures/default.png` is exactly what [`default_png`] produces. To rewrite it after changing the generator:
    /// `UPDATE_DEFAULT_PNG=1 cargo test -p shader_stdlib default_png`.
    #[test]
    fn default_png_matches_its_generator() {
        let png = default_png();
        if std::env::var_os("UPDATE_DEFAULT_PNG").is_some() {
            std::fs::write(concat!(env!("CARGO_MANIFEST_DIR"), "/textures/default.png"), &png).unwrap();
        }
        assert_eq!(INTERNAL_TEXTURES[0].bytes, png.as_slice(), "textures/default.png is stale");
    }

    #[test]
    fn default_png_is_a_64px_grey_white_checker() {
        let pixels = checker_pixels();
        let px = |x: usize, y: usize| &pixels[(y * SIZE + x) * 4..][..4];
        assert_eq!(px(0, 0), [0x80, 0x80, 0x80, 0xff]);
        assert_eq!(px(7, 7), [0x80, 0x80, 0x80, 0xff]);
        assert_eq!(px(8, 0), [0xff; 4]);
        assert_eq!(px(0, 8), [0xff; 4]);
        assert_eq!(px(8, 8), [0x80, 0x80, 0x80, 0xff]);
        assert_eq!(px(63, 63), [0x80, 0x80, 0x80, 0xff]);
        let png = INTERNAL_TEXTURES[0].bytes;
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        // IHDR: width, height, bit depth 8, color type 6 (RGBA).
        assert_eq!(&png[12..16], b"IHDR");
        assert_eq!(&png[16..26], [0, 0, 0, 64, 0, 0, 0, 64, 8, 6]);
    }

    const SIZE: usize = 64;
    const SQUARE: usize = 8;

    /// RGBA pixels, row by row: 8×8 px squares, grey `#808080` at the top left, white next to it.
    fn checker_pixels() -> Vec<u8> {
        let mut out = Vec::with_capacity(SIZE * SIZE * 4);
        for y in 0..SIZE {
            for x in 0..SIZE {
                let grey = (x / SQUARE + y / SQUARE).is_multiple_of(2);
                out.extend_from_slice(if grey { &[0x80, 0x80, 0x80, 0xff] } else { &[0xff; 4] });
            }
        }
        out
    }

    /// Minimal PNG encoder: one IDAT with a zlib stream of stored (uncompressed) deflate blocks.
    fn default_png() -> Vec<u8> {
        let pixels = checker_pixels();
        let mut raw = Vec::new();
        for row in pixels.chunks(SIZE * 4) {
            raw.push(0); // filter type None
            raw.extend_from_slice(row);
        }
        let mut zlib = vec![0x78, 0x01];
        let blocks: Vec<&[u8]> = raw.chunks(0xffff).collect();
        for (i, block) in blocks.iter().enumerate() {
            zlib.push(u8::from(i + 1 == blocks.len()));
            let len = block.len() as u16;
            zlib.extend_from_slice(&len.to_le_bytes());
            zlib.extend_from_slice(&(!len).to_le_bytes());
            zlib.extend_from_slice(block);
        }
        zlib.extend_from_slice(&adler32(&raw).to_be_bytes());

        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&(SIZE as u32).to_be_bytes());
        ihdr.extend_from_slice(&(SIZE as u32).to_be_bytes());
        ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        for (ty, data) in [(b"IHDR", ihdr.as_slice()), (b"IDAT", &zlib), (b"IEND", &[])] {
            png.extend_from_slice(&(data.len() as u32).to_be_bytes());
            let start = png.len();
            png.extend_from_slice(ty);
            png.extend_from_slice(data);
            let crc = crc32(&png[start..]);
            png.extend_from_slice(&crc.to_be_bytes());
        }
        png
    }

    fn crc32(data: &[u8]) -> u32 {
        let mut crc = !0u32;
        for b in data {
            crc ^= u32::from(*b);
            for _ in 0..8 {
                crc = if crc & 1 != 0 { (crc >> 1) ^ 0xedb8_8320 } else { crc >> 1 };
            }
        }
        !crc
    }

    fn adler32(data: &[u8]) -> u32 {
        let (mut a, mut b) = (1u32, 0u32);
        for x in data {
            a = (a + u32::from(*x)) % 65521;
            b = (b + a) % 65521;
        }
        (b << 16) | a
    }
}
