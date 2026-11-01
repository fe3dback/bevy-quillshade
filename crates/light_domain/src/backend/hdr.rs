//! Equirectangular Radiance `.hdr` decoding, 2D preview tonemapping, and HDR-to-KTX2 cubemap conversion.

use std::io::Cursor;
use std::path::Path;

use image::ImageDecoder;

/// KTX2 file identifier: `«KTX 20»\r\n\x1A\n`.
const KTX2_IDENTIFIER: [u8; 12] = [0xAB, 0x4B, 0x54, 0x58, 0x20, 0x32, 0x30, 0xBB, 0x0D, 0x0A, 0x1A, 0x0A];
const VK_FORMAT_R8G8B8A8_SRGB: u32 = 43;
const VK_FORMAT_R16G16B16A16_SFLOAT: u32 = 97;
/// Identifier + 9 header `u32`s + the section index (4 `u32`s, 2 `u64`s).
const KTX2_HEADER_LEN: usize = 12 + 9 * 4 + 4 * 4 + 2 * 8;
/// One level index entry (`byteOffset`, `byteLength`, `uncompressedByteLength`).
const KTX2_LEVEL_INDEX_ENTRY_LEN: usize = 3 * 8;
const KTX2_WRITER: &str = "shader-editor light_domain hdr_converter";

/// Maximum finite value representable in IEEE 754 binary16 (`f16::MAX`).
const MAX_HALF_FLOAT: f32 = 65_504.0;

/// Supported cube face resolutions in the HDR converter tool.
pub const FACE_SIZES: [u32; 4] = [256, 512, 1024, 2048];

/// Decoded linear floating-point Radiance `.hdr` panorama.
#[derive(Debug, Clone, PartialEq)]
pub struct HdrImage {
    pub width: u32,
    pub height: u32,
    /// Row-major top-to-bottom linear RGB pixels (`width * height`).
    pub pixels: Vec<[f32; 3]>,
}

impl HdrImage {
    /// Decodes a Radiance `.hdr` file from disk.
    pub fn load(path: &Path) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        Self::from_hdr_bytes(&bytes)
    }

    /// Decodes a Radiance `.hdr` byte buffer into sanitized linear RGB `[f32; 3]` pixels.
    pub fn from_hdr_bytes(bytes: &[u8]) -> Result<Self, String> {
        let decoder = image::codecs::hdr::HdrDecoder::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
        let (width, height) = decoder.dimensions();
        if width == 0 || height == 0 {
            return Err("HDR image has zero dimensions".to_string());
        }
        let total_bytes = decoder.total_bytes();
        let expected_bytes = u64::from(width).saturating_mul(u64::from(height)).saturating_mul(12);
        if total_bytes != expected_bytes {
            return Err(format!("unexpected HDR buffer size: {total_bytes} (expected {expected_bytes})"));
        }
        let mut raw = vec![0u8; total_bytes as usize];
        decoder.read_image(&mut raw).map_err(|e| e.to_string())?;
        let pixels = raw
            .as_chunks::<12>()
            .0
            .iter()
            .map(|c| {
                [
                    sanitize_hdr_channel(f32::from_ne_bytes([c[0], c[1], c[2], c[3]])),
                    sanitize_hdr_channel(f32::from_ne_bytes([c[4], c[5], c[6], c[7]])),
                    sanitize_hdr_channel(f32::from_ne_bytes([c[8], c[9], c[10], c[11]])),
                ]
            })
            .collect();
        Ok(HdrImage { width, height, pixels })
    }

    /// Samples the equirectangular panorama using bilinear interpolation for a normalized 3D world direction.
    ///
    /// - `+Y` maps to the zenith (`v = 0.0`, top row) and `-Y` maps to the nadir (`v = 1.0`, bottom row).
    /// - Horizontal longitude wraps seamlessly (`rem_euclid(width)`), while vertical latitude clamps to `0..height - 1`.
    pub fn sample_equirect(&self, dir: [f32; 3]) -> [f32; 3] {
        if self.width == 0 || self.height == 0 || self.pixels.is_empty() {
            return [0.0; 3];
        }
        let [wx, wy, wz] = dir;
        let latitude = wy.clamp(-1.0, 1.0).asin();
        let v = (0.5 - latitude / std::f32::consts::PI).clamp(0.0, 1.0);
        let longitude = wx.atan2(-wz);
        let u = (longitude / std::f32::consts::TAU + 0.5).rem_euclid(1.0);

        let w = i64::from(self.width);
        let h = i64::from(self.height);
        let fx = u * self.width as f32 - 0.5;
        let fy = v * self.height as f32 - 0.5;
        let x0_floor = fx.floor();
        let y0_floor = fy.floor();
        let tx = fx - x0_floor;
        let ty = fy - y0_floor;

        let x0 = (x0_floor as i64).rem_euclid(w) as usize;
        let x1 = (x0_floor as i64 + 1).rem_euclid(w) as usize;
        let y0 = (y0_floor as i64).clamp(0, h - 1) as usize;
        let y1 = (y0_floor as i64 + 1).clamp(0, h - 1) as usize;

        let stride = self.width as usize;
        let c00 = self.pixels[y0 * stride + x0];
        let c10 = self.pixels[y0 * stride + x1];
        let c01 = self.pixels[y1 * stride + x0];
        let c11 = self.pixels[y1 * stride + x1];

        std::array::from_fn(|i| {
            let top = c00[i] + (c10[i] - c00[i]) * tx;
            let bot = c01[i] + (c11[i] - c01[i]) * tx;
            top + (bot - top) * ty
        })
    }

    /// Renders a downsampled RGBA8 sRGB 2D preview of the panorama with exposure/brightness scaling and Reinhard
    /// tonemapping (`c / (1 + c)`).
    pub fn render_2d_preview_rgba8(&self, max_width: u32, brightness: f32, exposure_ev: f32) -> (u32, u32, Vec<u8>) {
        if self.width == 0 || self.height == 0 || self.pixels.is_empty() {
            return (1, 1, vec![0, 0, 0, 255]);
        }
        let max_w = max_width.max(1);
        let out_w = self.width.min(max_w).max(1);
        let out_h = ((u64::from(self.height) * u64::from(out_w)) / u64::from(self.width)).max(1) as u32;
        let scale = brightness.max(0.0) * 2.0f32.powf(exposure_ev);
        let stride = self.width as usize;

        let mut rgba = Vec::with_capacity(out_w as usize * out_h as usize * 4);
        for y in 0..out_h {
            let src_y = (((y as f32 + 0.5) / out_h as f32) * self.height as f32)
                .floor()
                .clamp(0.0, (self.height - 1) as f32) as usize;
            for x in 0..out_w {
                let src_x = (((x as f32 + 0.5) / out_w as f32) * self.width as f32)
                    .floor()
                    .clamp(0.0, (self.width - 1) as f32) as usize;
                let rgb = self.pixels[src_y * stride + src_x];
                let [r, g, b] = rgb.map(|c| {
                    let v = (c * scale).max(0.0);
                    let mapped = v / (1.0 + v);
                    linear_to_srgb_u8(mapped)
                });
                rgba.extend_from_slice(&[r, g, b, 255]);
            }
        }
        (out_w, out_h, rgba)
    }
}

/// Encodes linear RGB pixels into a Radiance `.hdr` byte buffer.
pub fn encode_radiance_hdr(width: u32, height: u32, pixels: &[[f32; 3]]) -> Vec<u8> {
    assert_eq!(pixels.len(), width as usize * height as usize, "pixel count must match width * height");
    let rgb: Vec<image::Rgb<f32>> = pixels.iter().map(|&[r, g, b]| image::Rgb([r, g, b])).collect();
    let mut out = Vec::new();
    let encoder = image::codecs::hdr::HdrEncoder::new(&mut out);
    encoder.encode(&rgb, width as usize, height as usize).expect("encode Radiance HDR");
    out
}

/// Target KTX2 texel format for a converted cubemap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CubemapFormat {
    /// 16-bit half-float linear RGBA (`VK_FORMAT_R16G16B16A16_SFLOAT = 97`, 8 bytes/texel).
    #[default]
    Rgba16Float,
    /// 8-bit sRGB RGBA (`VK_FORMAT_R8G8B8A8_SRGB = 43`, 4 bytes/texel).
    Rgba8Srgb,
}

impl CubemapFormat {
    pub const ALL: [CubemapFormat; 2] = [CubemapFormat::Rgba16Float, CubemapFormat::Rgba8Srgb];

    pub fn label(self) -> &'static str {
        match self {
            CubemapFormat::Rgba16Float => "RGBA16Float (HDR)",
            CubemapFormat::Rgba8Srgb => "RGBA8 (sRGB)",
        }
    }

    pub fn from_label(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.label() == s)
    }

    pub fn vk_format(self) -> u32 {
        match self {
            CubemapFormat::Rgba16Float => VK_FORMAT_R16G16B16A16_SFLOAT,
            CubemapFormat::Rgba8Srgb => VK_FORMAT_R8G8B8A8_SRGB,
        }
    }

    pub fn type_size(self) -> u32 {
        match self {
            CubemapFormat::Rgba16Float => 2,
            CubemapFormat::Rgba8Srgb => 1,
        }
    }

    pub fn bytes_per_texel(self) -> usize {
        match self {
            CubemapFormat::Rgba16Float => 8,
            CubemapFormat::Rgba8Srgb => 4,
        }
    }
}

/// Parameters controlling equirectangular HDR to KTX2 cubemap conversion.
#[derive(Debug, Clone, PartialEq)]
pub struct HdrConvertSettings {
    /// Resolution (`face_size x face_size`) of each of the 6 cube faces.
    pub face_size: u32,
    /// Linear brightness multiplier (`>= 0`).
    pub brightness: f32,
    /// Exposure compensation in stops (`2^exposure_ev`).
    pub exposure_ev: f32,
    /// Output texel format.
    pub format: CubemapFormat,
    /// Whether to generate a full mipmap pyramid down to `1x1`.
    pub mipmaps: bool,
    /// Whether to supercompress mip levels with Zstandard (`supercompressionScheme = 2`).
    pub zstd: bool,
}

impl Default for HdrConvertSettings {
    fn default() -> Self {
        HdrConvertSettings {
            face_size: 512,
            brightness: 1.0,
            exposure_ev: 0.0,
            format: CubemapFormat::Rgba16Float,
            mipmaps: true,
            zstd: false,
        }
    }
}

impl HdrConvertSettings {
    pub const FACE_SIZES: [u32; 4] = FACE_SIZES;

    /// Combined linear radiance multiplier `brightness.max(0.0) * 2^exposure_ev`.
    pub fn scale(&self) -> f32 {
        self.brightness.max(0.0) * 2.0f32.powf(self.exposure_ev)
    }
}

/// Encoded cubemap mip pyramid ready for KTX2 serialization or GPU preview upload.
#[derive(Debug, Clone, PartialEq)]
pub struct ConvertedCubemap {
    pub face_size: u32,
    pub mip_count: u32,
    pub format: CubemapFormat,
    /// Uncompressed mip level payloads from level `0` (`face_size x face_size`) down to `mip_count - 1`.
    /// Each level stores all 6 faces concatenated in KTX2 order (`+X, -X, +Y, -Y, +Z, -Z`).
    pub levels: Vec<Vec<u8>>,
}

impl ConvertedCubemap {
    /// Serializes this cubemap into a standalone `.ktx2` file, optionally compressing level payloads with Zstandard.
    pub fn to_ktx2_bytes(&self, zstd: bool) -> Vec<u8> {
        let level_count = self.levels.len() as u32;
        assert_eq!(level_count, self.mip_count, "mip_count must match levels.len()");

        let dfd = data_format_descriptor(self.format);
        let kvd = key_value(b"KTXwriter", KTX2_WRITER.as_bytes());
        let level_index_bytes = self.levels.len() * KTX2_LEVEL_INDEX_ENTRY_LEN;
        let dfd_offset = KTX2_HEADER_LEN + level_index_bytes;
        let kvd_offset = dfd_offset + dfd.len();

        let encoded_levels: Vec<(Vec<u8>, u64)> = self
            .levels
            .iter()
            .map(|raw| {
                let uncompressed_len = raw.len() as u64;
                let data = if zstd {
                    ruzstd::encoding::compress_to_vec(raw.as_slice(), ruzstd::encoding::CompressionLevel::Fastest)
                } else {
                    raw.clone()
                };
                (data, uncompressed_len)
            })
            .collect();

        let mut cursor = (kvd_offset + kvd.len()).next_multiple_of(8);
        let mut level_entries = Vec::with_capacity(encoded_levels.len());
        for (data, uncompressed_len) in &encoded_levels {
            cursor = cursor.next_multiple_of(8);
            let byte_offset = cursor as u64;
            let byte_length = data.len() as u64;
            level_entries.push((byte_offset, byte_length, *uncompressed_len));
            cursor += data.len();
        }

        let supercompression = if zstd { 2u32 } else { 0u32 };
        let mut out = Vec::with_capacity(cursor);
        out.extend_from_slice(&KTX2_IDENTIFIER);
        // vkFormat, typeSize, pixelWidth, pixelHeight, pixelDepth, layerCount, faceCount, levelCount, supercompression.
        for v in [
            self.format.vk_format(),
            self.format.type_size(),
            self.face_size,
            self.face_size,
            0,
            0,
            6,
            level_count,
            supercompression,
        ] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        for v in [dfd_offset, dfd.len(), kvd_offset, kvd.len()] {
            out.extend_from_slice(&(v as u32).to_le_bytes());
        }
        // sgdByteOffset, sgdByteLength: 0 for uncompressed and Zstandard.
        out.extend_from_slice(&[0u8; 16]);
        for &(byte_offset, byte_length, uncompressed_len) in &level_entries {
            out.extend_from_slice(&byte_offset.to_le_bytes());
            out.extend_from_slice(&byte_length.to_le_bytes());
            out.extend_from_slice(&uncompressed_len.to_le_bytes());
        }
        debug_assert_eq!(out.len(), dfd_offset);
        out.extend_from_slice(&dfd);
        out.extend_from_slice(&kvd);
        for ((data, _), &(byte_offset, _, _)) in encoded_levels.iter().zip(&level_entries) {
            out.resize(byte_offset as usize, 0);
            out.extend_from_slice(data);
        }
        out
    }
}

/// Projects an equirectangular [`HdrImage`] onto a 6-face cubemap (with optional box-filtered mipmaps).
pub fn convert_hdr_to_cubemap(hdr: &HdrImage, settings: &HdrConvertSettings) -> ConvertedCubemap {
    let face_size = settings.face_size.clamp(1, 4096);
    let mip_count = if settings.mipmaps { face_size.ilog2() + 1 } else { 1 };
    let scale = settings.scale();

    let mut base_faces: Vec<Vec<[f32; 3]>> = Vec::with_capacity(6);
    let inv_size = 1.0 / face_size as f32;
    for face in 0..6u8 {
        let mut pixels = Vec::with_capacity(face_size as usize * face_size as usize);
        for row in 0..face_size {
            let v = (row as f32 + 0.5) * inv_size * 2.0 - 1.0;
            for col in 0..face_size {
                let u = (col as f32 + 0.5) * inv_size * 2.0 - 1.0;
                let dir = cube_face_world_dir(face, u, v);
                let sample = hdr.sample_equirect(dir);
                pixels.push(sample.map(|c| sanitize_hdr_channel(c * scale)));
            }
        }
        base_faces.push(pixels);
    }

    let mut levels = Vec::with_capacity(mip_count as usize);
    levels.push(encode_faces(&base_faces, settings.format));

    let mut current_faces = base_faces;
    let mut current_size = face_size;
    for _ in 1..mip_count {
        let next_size = (current_size / 2).max(1);
        let next_faces: Vec<Vec<[f32; 3]>> =
            current_faces.iter().map(|face| downsample_face_2x2(face, current_size, next_size)).collect();
        levels.push(encode_faces(&next_faces, settings.format));
        current_faces = next_faces;
        current_size = next_size;
    }

    ConvertedCubemap { face_size, mip_count, format: settings.format, levels }
}

/// Projects an equirectangular [`HdrImage`] onto a cubemap and serializes it to `.ktx2` bytes.
pub fn convert_hdr_to_ktx2(hdr: &HdrImage, settings: &HdrConvertSettings) -> Vec<u8> {
    convert_hdr_to_cubemap(hdr, settings).to_ktx2_bytes(settings.zstd)
}

/// Maps cube face index (`0..6` for `+X, -X, +Y, -Y, +Z, -Z`) and normalized face coordinates `u, v in -1..1` to a
/// normalized world-space sampling direction, accounting for Bevy's left-handed cube-map Z negation (`-cz`).
fn cube_face_world_dir(face: u8, u: f32, v: f32) -> [f32; 3] {
    let (cx, cy, cz) = match face {
        0 => (1.0, -v, -u),  // +X
        1 => (-1.0, -v, u),  // -X
        2 => (u, 1.0, v),    // +Y
        3 => (u, -1.0, -v),  // -Y
        4 => (u, -v, 1.0),   // +Z
        _ => (-u, -v, -1.0), // -Z
    };
    let (wx, wy, wz) = (cx, cy, -cz);
    let len = (wx * wx + wy * wy + wz * wz).sqrt().max(1e-12);
    [wx / len, wy / len, wz / len]
}

fn downsample_face_2x2(src: &[[f32; 3]], src_size: u32, dst_size: u32) -> Vec<[f32; 3]> {
    let s = src_size as usize;
    let d = dst_size as usize;
    let mut out = Vec::with_capacity(d * d);
    for y in 0..d {
        let y0 = (y * 2).min(s - 1);
        let y1 = (y * 2 + 1).min(s - 1);
        for x in 0..d {
            let x0 = (x * 2).min(s - 1);
            let x1 = (x * 2 + 1).min(s - 1);
            let p00 = src[y0 * s + x0];
            let p10 = src[y0 * s + x1];
            let p01 = src[y1 * s + x0];
            let p11 = src[y1 * s + x1];
            out.push(std::array::from_fn(|i| 0.25 * (p00[i] + p10[i] + p01[i] + p11[i])));
        }
    }
    out
}

fn encode_faces(faces: &[Vec<[f32; 3]>], format: CubemapFormat) -> Vec<u8> {
    let texels: usize = faces.iter().map(Vec::len).sum();
    let mut bytes = Vec::with_capacity(texels * format.bytes_per_texel());
    match format {
        CubemapFormat::Rgba16Float => {
            const ONE_F16: [u8; 2] = 0x3c00u16.to_le_bytes();
            for face in faces {
                for &[r, g, b] in face {
                    bytes.extend_from_slice(&f32_to_f16(r).to_le_bytes());
                    bytes.extend_from_slice(&f32_to_f16(g).to_le_bytes());
                    bytes.extend_from_slice(&f32_to_f16(b).to_le_bytes());
                    bytes.extend_from_slice(&ONE_F16);
                }
            }
        }
        CubemapFormat::Rgba8Srgb => {
            for face in faces {
                for &[r, g, b] in face {
                    bytes.extend_from_slice(&[linear_to_srgb_u8(r), linear_to_srgb_u8(g), linear_to_srgb_u8(b), 255]);
                }
            }
        }
    }
    bytes
}

fn sanitize_hdr_channel(v: f32) -> f32 {
    if v.is_nan() || v <= 0.0 { 0.0 } else { v.min(MAX_HALF_FLOAT) }
}

/// Converts a linear `[0.0, 1.0]` channel to an 8-bit sRGB value using the IEC 61966-2-1 transfer function.
pub fn linear_to_srgb_u8(c: f32) -> u8 {
    let x = if c.is_nan() { 0.0 } else { c.clamp(0.0, 1.0) };
    let srgb = if x <= 0.003_130_8 { x * 12.92 } else { 1.055 * x.powf(1.0 / 2.4) - 0.055 };
    (srgb.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Converts an `f32` to IEEE 754 binary16 (`f16`) bits, clamping finite overflow to `f16::MAX` (`0x7bff`).
pub fn f32_to_f16(val: f32) -> u16 {
    let bits = val.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exp = ((bits >> 23) & 0xff) as i32;
    let mant = bits & 0x007f_ffff;
    if exp == 0xff {
        return if mant != 0 { sign | 0x7e00 } else { sign | 0x7c00 };
    }
    let half_exp = exp - 127 + 15;
    if half_exp >= 31 {
        return sign | 0x7bff;
    }
    if half_exp <= 0 {
        if half_exp < -10 {
            return sign;
        }
        let full_mant = mant | 0x0080_0000;
        let shift = 14 - half_exp;
        let half_mant = (full_mant >> shift) as u16;
        let round_bit = (full_mant >> (shift - 1)) & 1;
        let sticky = full_mant & ((1 << (shift - 1)) - 1);
        let rounded = if round_bit != 0 && (sticky != 0 || (half_mant & 1) != 0) { half_mant + 1 } else { half_mant };
        return sign | rounded;
    }
    let half_mant = (mant >> 13) as u16;
    let round_bit = (mant >> 12) & 1;
    let sticky = mant & 0x0fff;
    let mut result = sign | ((half_exp as u16) << 10) | half_mant;
    if round_bit != 0 && (sticky != 0 || (half_mant & 1) != 0) {
        result += 1;
    }
    if (result & 0x7fff) >= 0x7c00 {
        return sign | 0x7bff;
    }
    result
}

/// Builds the Khronos basic Data Format Descriptor (DFD) block for `format`.
fn data_format_descriptor(format: CubemapFormat) -> Vec<u8> {
    const SAMPLE_LEN: usize = 16;
    let block_len = 24 + 4 * SAMPLE_LEN;
    let mut d = Vec::with_capacity(4 + block_len);
    d.extend_from_slice(&((4 + block_len) as u32).to_le_bytes());
    d.extend_from_slice(&0u32.to_le_bytes()); // vendorId 0 (Khronos), descriptorType 0 (basic)
    d.extend_from_slice(&2u16.to_le_bytes()); // versionNumber (KDFS 1.3)
    d.extend_from_slice(&(block_len as u16).to_le_bytes());

    match format {
        CubemapFormat::Rgba16Float => {
            // colorModel RGBSDA (1), colorPrimaries BT709 (1), transferFunction Linear (1), flags 0 (straight alpha).
            d.extend_from_slice(&[1, 1, 1, 0]);
            d.extend_from_slice(&[0; 4]); // texelBlockDimension 1x1x1x1 (stored minus one)
            d.extend_from_slice(&[8, 0, 0, 0, 0, 0, 0, 0]); // bytesPlane0 = 8
            for (i, channel) in [0u8, 1, 2, 15].into_iter().enumerate() {
                // Sample qualifiers: FLOAT (0x80) | SIGNED (0x40).
                let qualifiers = 0x80 | 0x40;
                d.extend_from_slice(&((i * 16) as u16).to_le_bytes()); // bitOffset
                d.push(15); // bitLength - 1
                d.push(channel | qualifiers);
                d.extend_from_slice(&[0; 4]); // samplePosition
                d.extend_from_slice(&0xbf80_0000u32.to_le_bytes()); // sampleLower (-1.0f32)
                d.extend_from_slice(&0x3f80_0000u32.to_le_bytes()); // sampleUpper (+1.0f32)
            }
        }
        CubemapFormat::Rgba8Srgb => {
            // colorModel RGBSDA (1), colorPrimaries BT709 (1), transferFunction sRGB (2), flags 0 (straight alpha).
            d.extend_from_slice(&[1, 1, 2, 0]);
            d.extend_from_slice(&[0; 4]); // texelBlockDimension 1x1x1x1 (stored minus one)
            d.extend_from_slice(&[4, 0, 0, 0, 0, 0, 0, 0]); // bytesPlane0 = 4
            for (i, channel) in [0u8, 1, 2, 15].into_iter().enumerate() {
                // Alpha is never sRGB-encoded: it carries the LINEAR qualifier (0x10).
                let qualifiers = if channel == 15 { 0x10 } else { 0 };
                d.extend_from_slice(&((i * 8) as u16).to_le_bytes()); // bitOffset
                d.push(7); // bitLength - 1
                d.push(channel | qualifiers);
                d.extend_from_slice(&[0; 4]); // samplePosition
                d.extend_from_slice(&0u32.to_le_bytes()); // sampleLower
                d.extend_from_slice(&255u32.to_le_bytes()); // sampleUpper
            }
        }
    }
    d
}

/// One key/value entry: `keyAndValueByteLength`, `key\0value\0`.
fn key_value(key: &[u8], value: &[u8]) -> Vec<u8> {
    let len = key.len() + 1 + value.len() + 1;
    let mut kv = Vec::with_capacity(4 + len);
    kv.extend_from_slice(&(len as u32).to_le_bytes());
    kv.extend_from_slice(key);
    kv.push(0);
    kv.extend_from_slice(value);
    kv.push(0);
    kv
}

#[cfg(test)]
mod tests {
    use super::*;

    use bevy::asset::RenderAssetUsages;
    use bevy::image::{CompressedImageFormats, Image, ImageSampler, ImageType};
    use bevy::render::render_resource::{TextureFormat, TextureViewDimension};
    use texture_kit::{TextureShape, shape_problem};

    fn synthetic_hdr(width: u32, height: u32) -> HdrImage {
        let mut pixels = Vec::with_capacity(width as usize * height as usize);
        for y in 0..height {
            let v = y as f32 / height.max(1) as f32;
            for x in 0..width {
                let u = x as f32 / width.max(1) as f32;
                // Include HDR values > 1.0 to exercise float precision and tonemapping.
                pixels.push([u * 4.0, (1.0 - v) * 2.5, 0.5 + 0.5 * u]);
            }
        }
        HdrImage { width, height, pixels }
    }

    #[test]
    fn encode_and_decode_radiance_hdr_roundtrip() {
        let src = synthetic_hdr(16, 8);
        let bytes = encode_radiance_hdr(src.width, src.height, &src.pixels);
        let decoded = HdrImage::from_hdr_bytes(&bytes).expect("decode Radiance HDR");
        assert_eq!((decoded.width, decoded.height), (16, 8));
        assert_eq!(decoded.pixels.len(), src.pixels.len());
        // Radiance RGBE has 8-bit mantissa per channel (~1% relative tolerance).
        for (a, b) in src.pixels.iter().zip(&decoded.pixels) {
            for i in 0..3 {
                let diff = (a[i] - b[i]).abs();
                let tol = a[i].abs() * 0.03 + 0.02;
                assert!(diff <= tol, "channel {i}: expected {}, got {} (diff {diff})", a[i], b[i]);
            }
        }
    }

    #[test]
    fn load_hdr_from_disk_and_render_2d_preview() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test_sky.hdr");
        let src = synthetic_hdr(32, 16);
        std::fs::write(&path, encode_radiance_hdr(src.width, src.height, &src.pixels)).unwrap();

        let loaded = HdrImage::load(&path).expect("load HDR file");
        assert_eq!((loaded.width, loaded.height), (32, 16));

        let (pw, ph, rgba) = loaded.render_2d_preview_rgba8(16, 1.2, 0.5);
        assert_eq!((pw, ph), (16, 8));
        assert_eq!(rgba.len(), 16 * 8 * 4);
        assert!(rgba.as_chunks::<4>().0.iter().all(|px| px[3] == 255));
    }

    #[test]
    fn sample_equirect_poles_and_wrap() {
        // Top half bright blue, bottom half warm ground.
        let mut pixels = Vec::with_capacity(8 * 4);
        for y in 0..4 {
            let color = if y < 2 { [0.2, 0.5, 3.0] } else { [0.8, 0.3, 0.1] };
            for _ in 0..8 {
                pixels.push(color);
            }
        }
        let hdr = HdrImage { width: 8, height: 4, pixels };
        let zenith = hdr.sample_equirect([0.0, 1.0, 0.0]);
        let nadir = hdr.sample_equirect([0.0, -1.0, 0.0]);
        assert!(zenith[2] > 2.5, "zenith should sample top rows: {zenith:?}");
        assert!(nadir[0] > 0.7 && nadir[2] < 0.2, "nadir should sample bottom rows: {nadir:?}");
    }

    #[test]
    fn cubemap_format_labels_roundtrip() {
        for fmt in CubemapFormat::ALL {
            assert_eq!(CubemapFormat::from_label(fmt.label()), Some(fmt));
        }
        assert_eq!(CubemapFormat::from_label("unknown"), None);
    }

    #[test]
    fn convert_all_combinations_valid_in_ktx2_and_bevy() {
        let hdr = synthetic_hdr(16, 8);
        let side = 8u32;
        let expected_mips = side.ilog2() + 1;

        for format in CubemapFormat::ALL {
            for mipmaps in [false, true] {
                for zstd in [false, true] {
                    let settings = HdrConvertSettings {
                        face_size: side,
                        brightness: 1.5,
                        exposure_ev: -0.5,
                        format,
                        mipmaps,
                        zstd,
                    };
                    let bytes = convert_hdr_to_ktx2(&hdr, &settings);

                    // 1. `texture_kit` accepts the KTX2 header as a valid CubeMap and rejects it as Plain2d.
                    assert_eq!(
                        shape_problem(&bytes, TextureShape::CubeMap),
                        None,
                        "CubeMap check failed for {format:?} mipmaps={mipmaps} zstd={zstd}"
                    );
                    assert!(shape_problem(&bytes, TextureShape::Plain2d).is_some());

                    // 2. `ktx2::Reader` parses header, DFD, and level index cleanly.
                    let reader = ktx2::Reader::new(&bytes[..]).expect("valid KTX2");
                    let h = reader.header();
                    let want_ktx_fmt = match format {
                        CubemapFormat::Rgba16Float => ktx2::Format::R16G16B16A16_SFLOAT,
                        CubemapFormat::Rgba8Srgb => ktx2::Format::R8G8B8A8_SRGB,
                    };
                    let want_levels = if mipmaps { expected_mips } else { 1 };
                    assert_eq!(h.format, Some(want_ktx_fmt));
                    assert_eq!((h.pixel_width, h.pixel_height, h.pixel_depth), (side, side, 0));
                    assert_eq!((h.layer_count, h.face_count, h.level_count), (0, 6, want_levels));
                    assert_eq!(
                        h.supercompression_scheme,
                        if zstd { Some(ktx2::SupercompressionScheme::Zstandard) } else { None }
                    );
                    let levels: Vec<_> = reader.levels().collect();
                    assert_eq!(levels.len(), want_levels as usize);

                    // 3. Bevy's `Image::from_buffer` decodes the KTX2 into a 6-layer Cube texture with all mip levels.
                    let image = Image::from_buffer(
                        &bytes,
                        ImageType::Extension("ktx2"),
                        CompressedImageFormats::NONE,
                        format == CubemapFormat::Rgba8Srgb,
                        ImageSampler::Default,
                        RenderAssetUsages::default(),
                    )
                    .unwrap_or_else(|e| panic!("Bevy failed to decode {format:?} mipmaps={mipmaps} zstd={zstd}: {e}"));

                    let size = image.texture_descriptor.size;
                    assert_eq!((size.width, size.height, size.depth_or_array_layers), (side, side, 6));
                    assert_eq!(image.texture_descriptor.mip_level_count, want_levels);
                    let want_bevy_fmt = match format {
                        CubemapFormat::Rgba16Float => TextureFormat::Rgba16Float,
                        CubemapFormat::Rgba8Srgb => TextureFormat::Rgba8UnormSrgb,
                    };
                    assert_eq!(image.texture_descriptor.format, want_bevy_fmt);
                    let view_dim = image.texture_view_descriptor.as_ref().and_then(|v| v.dimension);
                    assert_eq!(view_dim, Some(TextureViewDimension::Cube));
                    assert!(image.data.as_ref().is_some_and(|d| !d.is_empty()));
                }
            }
        }
    }
}
