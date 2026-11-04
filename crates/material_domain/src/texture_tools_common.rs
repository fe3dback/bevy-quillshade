//! Shared 2D image decoding, bilinear resizing, channel assembly, PNG/KTX2 encoding, and preview helpers for the
//! Material domain's standalone texture tools (`Texture Converter` and `Texture Channel Assembler`).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::image::{CompressedImageFormats, Image, ImageSampler, ImageType};
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use texture_kit::{KTX2_HEADER_LEN, TextureShape, read_ktx2_header, shape_problem};

/// KTX2 file identifier: `«KTX 20»\r\n\x1A\n`.
const KTX2_IDENTIFIER: [u8; 12] = [0xAB, 0x4B, 0x54, 0x58, 0x20, 0x32, 0x30, 0xBB, 0x0D, 0x0A, 0x1A, 0x0A];
/// Vulkan format code for 8-bit linear RGBA (`VK_FORMAT_R8G8B8A8_UNORM`).
pub const VK_FORMAT_R8G8B8A8_UNORM: u32 = 37;
/// Vulkan format code for 8-bit sRGB RGBA (`VK_FORMAT_R8G8B8A8_SRGB`).
pub const VK_FORMAT_R8G8B8A8_SRGB: u32 = 43;
/// Identifier (12 bytes) + 9 header `u32`s + section index (4 `u32`s, 2 `u64`s) = 80 bytes.
const KTX2_HEADER_BYTES: usize = 12 + 9 * 4 + 4 * 4 + 2 * 8;
/// One level index entry (`byteOffset`, `byteLength`, `uncompressedByteLength`).
const KTX2_LEVEL_INDEX_ENTRY_LEN: usize = 3 * 8;
const KTX2_WRITER: &str = "shader-editor material_domain texture_tools";

/// Decoded 2D RGBA8 image with source metadata for the Material domain's texture tools.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgbaImage2d {
    pub width: u32,
    pub height: u32,
    /// Row-major top-to-bottom RGBA8 pixels (`width * height`).
    pub pixels: Vec<[u8; 4]>,
    /// Human-readable description of the decoded source format (e.g. `"Rgba8UnormSrgb"`).
    pub source_format: String,
    /// Mip level count in the original file (`>= 1`).
    pub source_mips: u32,
    /// Whether the source texture used an sRGB color space.
    pub source_is_srgb: bool,
}

impl RgbaImage2d {
    /// Creates an [`RgbaImage2d`] from raw row-major `[u8; 4]` pixels with default sRGB metadata.
    pub fn new(width: u32, height: u32, pixels: Vec<[u8; 4]>) -> Self {
        assert_eq!(pixels.len(), width as usize * height as usize, "pixel count must match width * height");
        Self {
            width,
            height,
            pixels,
            source_format: "Rgba8UnormSrgb".to_string(),
            source_mips: 1,
            source_is_srgb: true,
        }
    }

    /// Decodes a 2D `.png`, `.jpg`/`.jpeg`, or `.ktx2` file from disk into level-0 RGBA8 pixels.
    pub fn from_file(path: &Path) -> Result<Self, String> {
        if let Some(header) = read_ktx2_header(path)
            && let Some(problem) = shape_problem(&header, TextureShape::Plain2d)
        {
            return Err(format!("{}: {problem}", path.display()));
        }
        let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        Self::from_bytes(&bytes, path)
    }

    /// Decodes a 2D `.png`, `.jpg`/`.jpeg`, or `.ktx2` byte buffer into level-0 RGBA8 pixels.
    ///
    /// - Rejects unsupported file extensions (only `.png`, `.jpg`, `.jpeg`, and `.ktx2` are accepted,
    ///   case-insensitively).
    /// - For `.ktx2`, validates `TextureShape::Plain2d` via [`shape_problem`] so cubemaps, texture arrays, and 3D
    ///   textures are rejected with a clear message, and reads the original mip count and `vkFormat` from the KTX2
    ///   header.
    /// - Uses Bevy's [`Image::from_buffer`] and [`Image::convert`] (truncating multi-mip images to mip 0 first) so any
    ///   uncompressed PNG/JPEG/KTX2 pixel format supported by Bevy converts cleanly to `[u8; 4]` RGBA8 pixels.
    pub fn from_bytes(bytes: &[u8], path: &Path) -> Result<Self, String> {
        let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
        if !matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "ktx2") {
            let shown = if ext.is_empty() { "<none>" } else { &ext };
            return Err(format!(
                "{}: unsupported image format `{shown}` (supported: png, jpg, jpeg, ktx2)",
                path.display()
            ));
        }

        let mut header_mips: Option<u32> = None;
        let mut header_vk_format: Option<u32> = None;
        if ext == "ktx2" {
            if let Some(problem) = shape_problem(bytes, TextureShape::Plain2d) {
                return Err(format!("{}: {problem}", path.display()));
            }
            if bytes.len() >= KTX2_HEADER_LEN && bytes[..12] == KTX2_IDENTIFIER {
                let vk_fmt = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
                let levels = u32::from_le_bytes([bytes[40], bytes[41], bytes[42], bytes[43]]).max(1);
                header_vk_format = Some(vk_fmt);
                header_mips = Some(levels);
            }
        }

        let mut image = Image::from_buffer(
            bytes,
            ImageType::Extension(&ext),
            CompressedImageFormats::NONE,
            true,
            ImageSampler::Default,
            RenderAssetUsages::default(),
        )
        .map_err(|e| format!("cannot decode {}: {e}", path.display()))?;

        let d = &image.texture_descriptor;
        if d.dimension != TextureDimension::D2 || d.size.depth_or_array_layers > 1 {
            return Err(format!("{}: only 2D single-layer textures are supported", path.display()));
        }
        let width = d.size.width;
        let height = d.size.height;
        if width == 0 || height == 0 {
            return Err(format!("{}: image has zero dimensions ({width}x{height})", path.display()));
        }

        let orig_format = d.format;
        let source_mips = header_mips.unwrap_or(d.mip_level_count).max(1);
        let source_is_srgb = match header_vk_format {
            Some(VK_FORMAT_R8G8B8A8_UNORM) => false,
            Some(VK_FORMAT_R8G8B8A8_SRGB) => true,
            _ => orig_format.is_srgb(),
        };
        let source_format = format!("{orig_format:?}");

        if image.texture_descriptor.mip_level_count > 1 {
            let (bw, bh) = orig_format.block_dimensions();
            let block = orig_format
                .block_copy_size(None)
                .ok_or_else(|| format!("{}: {orig_format:?} has no copyable blocks", path.display()))?;
            let level0 = (width.div_ceil(bw) * height.div_ceil(bh) * block) as usize;
            let data =
                image.data.as_mut().ok_or_else(|| format!("{}: decoded image has no pixel data", path.display()))?;
            if data.len() < level0 {
                return Err(format!("{}: {} bytes of data, mip 0 needs {level0}", path.display(), data.len()));
            }
            data.truncate(level0);
            image.texture_descriptor.mip_level_count = 1;
        }

        let target_format =
            if orig_format.is_srgb() { TextureFormat::Rgba8UnormSrgb } else { TextureFormat::Rgba8Unorm };
        let converted = if image.texture_descriptor.format == target_format {
            image
        } else {
            image
                .convert(target_format)
                .ok_or_else(|| format!("{}: cannot convert {orig_format:?} to RGBA8", path.display()))?
        };

        let raw =
            converted.data.as_ref().ok_or_else(|| format!("{}: converted image has no pixel data", path.display()))?;
        let expected_len = width as usize * height as usize * 4;
        if raw.len() < expected_len {
            return Err(format!(
                "{}: unexpected RGBA8 buffer length {} (expected {expected_len})",
                path.display(),
                raw.len()
            ));
        }
        let pixels: Vec<[u8; 4]> = raw[..expected_len].as_chunks::<4>().0.to_vec();

        Ok(Self { width, height, pixels, source_format, source_mips, source_is_srgb })
    }

    /// Flattens `self.pixels` into a contiguous `width * height * 4` RGBA8 byte vector.
    pub fn as_rgba_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.pixels.len() * 4);
        for px in &self.pixels {
            out.extend_from_slice(px);
        }
        out
    }

    /// Samples the image at normalized coordinates `(u, v)` in `[0.0, 1.0]` using bilinear filtering.
    pub fn sample_bilinear(&self, u: f32, v: f32) -> [u8; 4] {
        if self.width == 0 || self.height == 0 || self.pixels.is_empty() {
            return [0, 0, 0, 255];
        }
        let max_x = (self.width - 1) as f32;
        let max_y = (self.height - 1) as f32;
        let fx = (u.clamp(0.0, 1.0) * self.width as f32 - 0.5).clamp(0.0, max_x);
        let fy = (v.clamp(0.0, 1.0) * self.height as f32 - 0.5).clamp(0.0, max_y);

        let x0 = fx.floor() as usize;
        let y0 = fy.floor() as usize;
        let x1 = (x0 + 1).min(self.width as usize - 1);
        let y1 = (y0 + 1).min(self.height as usize - 1);
        let tx = fx - x0 as f32;
        let ty = fy - y0 as f32;

        let stride = self.width as usize;
        let c00 = self.pixels[y0 * stride + x0];
        let c10 = self.pixels[y0 * stride + x1];
        let c01 = self.pixels[y1 * stride + x0];
        let c11 = self.pixels[y1 * stride + x1];

        std::array::from_fn(|i| {
            let top = c00[i] as f32 + (c10[i] as f32 - c00[i] as f32) * tx;
            let bot = c01[i] as f32 + (c11[i] as f32 - c01[i] as f32) * tx;
            (top + (bot - top) * ty).round().clamp(0.0, 255.0) as u8
        })
    }
}

/// Target file format for standalone texture export.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutputFormat {
    #[default]
    Ktx2,
    Png,
}

impl OutputFormat {
    pub const ALL: [OutputFormat; 2] = [OutputFormat::Ktx2, OutputFormat::Png];

    pub fn extension(self) -> &'static str {
        match self {
            OutputFormat::Ktx2 => "ktx2",
            OutputFormat::Png => "png",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            OutputFormat::Ktx2 => "KTX2",
            OutputFormat::Png => "PNG",
        }
    }

    pub fn from_label(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.label().eq_ignore_ascii_case(s) || f.extension().eq_ignore_ascii_case(s))
    }
}

/// KTX2 header `vkFormat` and Data Format Descriptor transfer function for 8-bit RGBA output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Ktx2ColorSpace {
    /// `VK_FORMAT_R8G8B8A8_SRGB = 43` (color textures: albedo, emissive, UI).
    #[default]
    Srgb,
    /// `VK_FORMAT_R8G8B8A8_UNORM = 37` (linear data textures: normal maps, ORM/masks, roughness).
    Linear,
}

impl Ktx2ColorSpace {
    pub const ALL: [Ktx2ColorSpace; 2] = [Ktx2ColorSpace::Srgb, Ktx2ColorSpace::Linear];

    pub fn label(self) -> &'static str {
        match self {
            Ktx2ColorSpace::Srgb => "RGBA8 (sRGB)",
            Ktx2ColorSpace::Linear => "RGBA8 (Linear)",
        }
    }

    pub fn from_label(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.label() == s)
    }

    pub fn vk_format(self) -> u32 {
        match self {
            Ktx2ColorSpace::Srgb => VK_FORMAT_R8G8B8A8_SRGB,
            Ktx2ColorSpace::Linear => VK_FORMAT_R8G8B8A8_UNORM,
        }
    }

    pub fn is_srgb(self) -> bool {
        matches!(self, Ktx2ColorSpace::Srgb)
    }
}

/// Output resolution preset for texture conversion and channel assembly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ResizePreset {
    #[default]
    Original,
    Size256,
    Size512,
    Size1024,
    Size2048,
    Size4096,
}

impl ResizePreset {
    pub const ALL: [ResizePreset; 6] = [
        ResizePreset::Original,
        ResizePreset::Size256,
        ResizePreset::Size512,
        ResizePreset::Size1024,
        ResizePreset::Size2048,
        ResizePreset::Size4096,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ResizePreset::Original => "Original",
            ResizePreset::Size256 => "256x256",
            ResizePreset::Size512 => "512x512",
            ResizePreset::Size1024 => "1024x1024",
            ResizePreset::Size2048 => "2048x2048",
            ResizePreset::Size4096 => "4096x4096",
        }
    }

    pub fn from_label(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.label() == s)
    }

    /// Resolves the output `(width, height)` given a source or base `(src_w, src_h)`.
    pub fn resolve(self, src_w: u32, src_h: u32) -> (u32, u32) {
        match self {
            ResizePreset::Original => (src_w.max(1), src_h.max(1)),
            ResizePreset::Size256 => (256, 256),
            ResizePreset::Size512 => (512, 512),
            ResizePreset::Size1024 => (1024, 1024),
            ResizePreset::Size2048 => (2048, 2048),
            ResizePreset::Size4096 => (4096, 4096),
        }
    }
}

/// Export configuration shared by `Texture Converter` and `Texture Channel Assembler`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextureExportSettings {
    pub output_format: OutputFormat,
    pub resize: ResizePreset,
    pub flip_y: bool,
    pub ktx2_color_space: Ktx2ColorSpace,
    pub mipmaps: bool,
    pub zstd: bool,
}

impl Default for TextureExportSettings {
    fn default() -> Self {
        Self {
            output_format: OutputFormat::Ktx2,
            resize: ResizePreset::Original,
            flip_y: false,
            ktx2_color_space: Ktx2ColorSpace::Srgb,
            mipmaps: true,
            zstd: true,
        }
    }
}

impl TextureExportSettings {
    /// Number of mip levels that will be written for `(out_w, out_h)`.
    pub fn mip_count(&self, out_w: u32, out_h: u32) -> u32 {
        if self.output_format == OutputFormat::Ktx2 && self.mipmaps { out_w.max(out_h).max(1).ilog2() + 1 } else { 1 }
    }

    /// Human-readable summary of the target output (e.g. `"1024x1024 | KTX2 | RGBA8 (sRGB) | 11 mips | Zstd"` or
    /// `"1024x1024 | PNG | RGBA8"`).
    pub fn output_summary(&self, src_w: u32, src_h: u32) -> String {
        let (out_w, out_h) = self.resize.resolve(src_w, src_h);
        match self.output_format {
            OutputFormat::Ktx2 => {
                let mips = self.mip_count(out_w, out_h);
                let mip_word = if mips == 1 { "mip" } else { "mips" };
                let zstd = if self.zstd { " | Zstd" } else { "" };
                format!("{out_w}x{out_h} | KTX2 | {} | {mips} {mip_word}{zstd}", self.ktx2_color_space.label())
            }
            OutputFormat::Png => format!("{out_w}x{out_h} | PNG | RGBA8"),
        }
    }
}

/// Applies `resize` (bilinear resampling when dimensions differ) and `flip_y` (vertical row flip) to `src`.
pub fn prepare_output_image(src: &RgbaImage2d, resize: ResizePreset, flip_y: bool) -> RgbaImage2d {
    let (out_w, out_h) = resize.resolve(src.width, src.height);
    let mut pixels = if out_w == src.width && out_h == src.height {
        src.pixels.clone()
    } else {
        let mut resampled = Vec::with_capacity(out_w as usize * out_h as usize);
        for y in 0..out_h {
            let v = (y as f32 + 0.5) / out_h as f32;
            for x in 0..out_w {
                let u = (x as f32 + 0.5) / out_w as f32;
                resampled.push(src.sample_bilinear(u, v));
            }
        }
        resampled
    };

    if flip_y && out_h > 1 {
        let stride = out_w as usize;
        for y in 0..(out_h as usize / 2) {
            let top_start = y * stride;
            let bot_start = (out_h as usize - 1 - y) * stride;
            for x in 0..stride {
                pixels.swap(top_start + x, bot_start + x);
            }
        }
    }

    RgbaImage2d {
        width: out_w,
        height: out_h,
        pixels,
        source_format: src.source_format.clone(),
        source_mips: src.source_mips,
        source_is_srgb: src.source_is_srgb,
    }
}

/// Encodes `image` as an 8-bit RGBA `.png` byte buffer.
pub fn encode_png(image: &RgbaImage2d) -> Result<Vec<u8>, String> {
    if image.width == 0 || image.height == 0 {
        return Err("cannot encode PNG with zero dimensions".to_string());
    }
    let expected = image.width as usize * image.height as usize;
    if image.pixels.len() != expected {
        return Err(format!("pixel count {} does not match {}x{}", image.pixels.len(), image.width, image.height));
    }
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, image.width, image.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        encoder.set_filter(png::Filter::Adaptive);
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        writer.write_image_data(&image.as_rgba_bytes()).map_err(|e| e.to_string())?;
        writer.finish().map_err(|e| e.to_string())?;
    }
    Ok(out)
}

/// Downsamples a 2D RGBA8 image by 2x2 box filtering for the next mip level (`next_w = (w / 2).max(1)`,
/// `next_h = (h / 2).max(1)`).
pub fn downsample_2d_2x2(pixels: &[[u8; 4]], w: u32, h: u32) -> (Vec<[u8; 4]>, u32, u32) {
    let src_w = w.max(1) as usize;
    let src_h = h.max(1) as usize;
    let next_w = (w / 2).max(1);
    let next_h = (h / 2).max(1);
    let dst_w = next_w as usize;
    let dst_h = next_h as usize;

    let mut out = Vec::with_capacity(dst_w * dst_h);
    for y in 0..dst_h {
        let y0 = (y * 2).min(src_h - 1);
        let y1 = (y * 2 + 1).min(src_h - 1);
        for x in 0..dst_w {
            let x0 = (x * 2).min(src_w - 1);
            let x1 = (x * 2 + 1).min(src_w - 1);
            let p00 = pixels[y0 * src_w + x0];
            let p10 = pixels[y0 * src_w + x1];
            let p01 = pixels[y1 * src_w + x0];
            let p11 = pixels[y1 * src_w + x1];
            out.push(std::array::from_fn(|c| {
                ((u16::from(p00[c]) + u16::from(p10[c]) + u16::from(p01[c]) + u16::from(p11[c]) + 2) / 4) as u8
            }));
        }
    }
    (out, next_w, next_h)
}

/// Encodes `image` as a standalone 2D `.ktx2` byte buffer according to `settings` (`ktx2_color_space`, `mipmaps`, and
/// `zstd`). Note: `encode_ktx2` encodes the dimensions/pixels of `image` as given; use [`encode_texture`] to apply
/// `settings.resize` and `settings.flip_y` before encoding.
pub fn encode_ktx2(image: &RgbaImage2d, settings: &TextureExportSettings) -> Result<Vec<u8>, String> {
    if image.width == 0 || image.height == 0 {
        return Err("cannot encode KTX2 with zero dimensions".to_string());
    }
    let expected = image.width as usize * image.height as usize;
    if image.pixels.len() != expected {
        return Err(format!("pixel count {} does not match {}x{}", image.pixels.len(), image.width, image.height));
    }

    let level_count = if settings.mipmaps { image.width.max(image.height).max(1).ilog2() + 1 } else { 1 };
    let mut raw_levels: Vec<Vec<u8>> = Vec::with_capacity(level_count as usize);

    let mut cur_pixels = image.pixels.clone();
    let mut cur_w = image.width;
    let mut cur_h = image.height;
    raw_levels.push(flatten_rgba(&cur_pixels));
    for _ in 1..level_count {
        let (next_pixels, next_w, next_h) = downsample_2d_2x2(&cur_pixels, cur_w, cur_h);
        raw_levels.push(flatten_rgba(&next_pixels));
        cur_pixels = next_pixels;
        cur_w = next_w;
        cur_h = next_h;
    }

    let srgb = settings.ktx2_color_space.is_srgb();
    let dfd = data_format_descriptor_rgba8(srgb);
    let kvd = key_value_padded(b"KTXwriter", KTX2_WRITER.as_bytes());
    let level_index_bytes = raw_levels.len() * KTX2_LEVEL_INDEX_ENTRY_LEN;
    let dfd_offset = KTX2_HEADER_BYTES + level_index_bytes;
    let kvd_offset = dfd_offset + dfd.len();

    let encoded_levels: Vec<(Vec<u8>, u64)> = raw_levels
        .iter()
        .map(|raw| {
            let uncompressed_len = raw.len() as u64;
            let data = if settings.zstd {
                ruzstd::encoding::compress_to_vec(raw.as_slice(), ruzstd::encoding::CompressionLevel::Fastest)
            } else {
                raw.clone()
            };
            (data, uncompressed_len)
        })
        .collect();

    // KTX2 places smaller mip levels earlier in the file (smallest to largest) while the level index table lists levels
    // from largest (level 0) to smallest (level_count - 1). Each level is 8-byte aligned.
    let mut cursor = (kvd_offset + kvd.len()).next_multiple_of(8);
    let mut level_entries = vec![(0u64, 0u64, 0u64); encoded_levels.len()];
    for level in (0..encoded_levels.len()).rev() {
        cursor = cursor.next_multiple_of(8);
        let (ref data, uncompressed_len) = encoded_levels[level];
        let byte_offset = cursor as u64;
        let byte_length = data.len() as u64;
        level_entries[level] = (byte_offset, byte_length, uncompressed_len);
        cursor += data.len();
    }

    let supercompression = if settings.zstd { 2u32 } else { 0u32 };
    let mut out = Vec::with_capacity(cursor);
    out.extend_from_slice(&KTX2_IDENTIFIER);
    // vkFormat, typeSize, pixelWidth, pixelHeight, pixelDepth, layerCount, faceCount, levelCount, supercompression.
    for v in
        [settings.ktx2_color_space.vk_format(), 1, image.width, image.height, 0, 0, 1, level_count, supercompression]
    {
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

    for level in (0..encoded_levels.len()).rev() {
        let (byte_offset, _, _) = level_entries[level];
        out.resize(byte_offset as usize, 0);
        out.extend_from_slice(&encoded_levels[level].0);
    }

    Ok(out)
}

/// Applies `settings.resize` and `settings.flip_y` to `image` and encodes the result to `.ktx2` or `.png`.
pub fn encode_texture(image: &RgbaImage2d, settings: &TextureExportSettings) -> Result<Vec<u8>, String> {
    let prepared = prepare_output_image(image, settings.resize, settings.flip_y);
    match settings.output_format {
        OutputFormat::Ktx2 => encode_ktx2(&prepared, settings),
        OutputFormat::Png => encode_png(&prepared),
    }
}

fn flatten_rgba(pixels: &[[u8; 4]]) -> Vec<u8> {
    let mut out = Vec::with_capacity(pixels.len() * 4);
    for px in pixels {
        out.extend_from_slice(px);
    }
    out
}

/// `dfdTotalSize` + one Khronos basic descriptor block with four 8-bit samples (R, G, B, A).
fn data_format_descriptor_rgba8(srgb: bool) -> Vec<u8> {
    const SAMPLE_LEN: usize = 16;
    let block_len = 24 + 4 * SAMPLE_LEN;
    let mut d = Vec::with_capacity(4 + block_len);
    d.extend_from_slice(&((4 + block_len) as u32).to_le_bytes());
    d.extend_from_slice(&0u32.to_le_bytes()); // vendorId 0 (Khronos), descriptorType 0 (basic)
    d.extend_from_slice(&2u16.to_le_bytes()); // versionNumber (KDFS 1.3)
    d.extend_from_slice(&(block_len as u16).to_le_bytes());
    // colorModel RGBSDA (1), colorPrimaries BT709 (1), transferFunction sRGB (2) / Linear (1), flags 0 (straight alpha).
    d.extend_from_slice(&[1, 1, if srgb { 2 } else { 1 }, 0]);
    d.extend_from_slice(&[0; 4]); // texelBlockDimension 1x1x1x1 (stored minus one)
    d.extend_from_slice(&[4, 0, 0, 0, 0, 0, 0, 0]); // bytesPlane0 = 4
    for (i, channel) in [0u8, 1, 2, 15].into_iter().enumerate() {
        // Alpha is never sRGB-encoded: in an sRGB format it carries the LINEAR qualifier (0x10).
        let qualifiers = if srgb && channel == 15 { 0x10 } else { 0 };
        d.extend_from_slice(&((i * 8) as u16).to_le_bytes()); // bitOffset
        d.push(7); // bitLength - 1
        d.push(channel | qualifiers);
        d.extend_from_slice(&[0; 4]); // samplePosition
        d.extend_from_slice(&0u32.to_le_bytes()); // sampleLower
        d.extend_from_slice(&255u32.to_le_bytes()); // sampleUpper
    }
    d
}

/// One key/value entry: `keyAndValueByteLength`, `key\0value\0`, padded to 4-byte alignment.
fn key_value_padded(key: &[u8], value: &[u8]) -> Vec<u8> {
    let len = key.len() + 1 + value.len() + 1;
    let padded = len.next_multiple_of(4);
    let mut kv = Vec::with_capacity(4 + padded);
    kv.extend_from_slice(&(len as u32).to_le_bytes());
    kv.extend_from_slice(key);
    kv.push(0);
    kv.extend_from_slice(value);
    kv.push(0);
    kv.resize(4 + padded, 0);
    kv
}

/// Which channel to extract from a source image in `Texture Channel Assembler`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SourceChannel {
    #[default]
    R,
    G,
    B,
    A,
    Luminance,
}

impl SourceChannel {
    pub const ALL: [SourceChannel; 5] =
        [SourceChannel::R, SourceChannel::G, SourceChannel::B, SourceChannel::A, SourceChannel::Luminance];

    pub fn label(self) -> &'static str {
        match self {
            SourceChannel::R => "R",
            SourceChannel::G => "G",
            SourceChannel::B => "B",
            SourceChannel::A => "A",
            SourceChannel::Luminance => "Luminance",
        }
    }

    pub fn from_label(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.label().eq_ignore_ascii_case(s))
    }

    /// Extracts an 8-bit scalar value from an RGBA8 pixel.
    pub fn extract(self, px: [u8; 4]) -> u8 {
        match self {
            SourceChannel::R => px[0],
            SourceChannel::G => px[1],
            SourceChannel::B => px[2],
            SourceChannel::A => px[3],
            SourceChannel::Luminance => {
                (0.2126 * px[0] as f32 + 0.7152 * px[1] as f32 + 0.0722 * px[2] as f32).round().clamp(0.0, 255.0) as u8
            }
        }
    }
}

/// Configuration and loaded source image for one output channel (`R`, `G`, `B`, or `A`) in `Texture Channel Assembler`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelSlot {
    pub path: Option<PathBuf>,
    pub image: Option<Arc<RgbaImage2d>>,
    pub source_channel: SourceChannel,
    pub invert: bool,
    pub default_value: u8,
}

impl ChannelSlot {
    /// Creates an empty [`ChannelSlot`] with the given `default_value` (`0` for R/G/B, `255` for A).
    pub fn new(default_value: u8) -> Self {
        Self { path: None, image: None, source_channel: SourceChannel::R, invert: false, default_value }
    }

    /// Returns the default 4-channel slot array (`R = 0`, `G = 0`, `B = 0`, `A = 255`).
    pub fn default_slots() -> [Self; 4] {
        [Self::new(0), Self::new(0), Self::new(0), Self::new(255)]
    }

    /// Clears the loaded image and path while keeping `source_channel`, `invert`, and `default_value`.
    pub fn clear(&mut self) {
        self.path = None;
        self.image = None;
    }

    /// Sets the slot's loaded image and file path.
    pub fn set_image(&mut self, path: PathBuf, image: RgbaImage2d) {
        self.path = Some(path);
        self.image = Some(Arc::new(image));
    }
}

/// Assembles up to 4 [`ChannelSlot`]s into a single [`RgbaImage2d`], returning `(assembled_image, mixed_sizes)` or
/// `None` if all 4 slots are empty.
///
/// - The base resolution is taken from the first loaded slot's `(width, height)`, then resolved through `resize`.
/// - `mixed_sizes` is `true` when any two loaded slots have different `(width, height)`.
/// - Any loaded slot whose dimensions differ from `(out_w, out_h)` is bilinearly resampled.
/// - Empty slots are filled with their `default_value`.
pub fn assemble_channels(slots: &[ChannelSlot; 4], resize: ResizePreset, flip_y: bool) -> Option<(RgbaImage2d, bool)> {
    let mut base_dims: Option<(u32, u32)> = None;
    let mut mixed_sizes = false;
    for slot in slots {
        if let Some(img) = &slot.image {
            let dims = (img.width, img.height);
            if let Some(first) = base_dims {
                if first != dims {
                    mixed_sizes = true;
                }
            } else {
                base_dims = Some(dims);
            }
        }
    }
    let (base_w, base_h) = base_dims?;
    let (out_w, out_h) = resize.resolve(base_w, base_h);

    let mut pixels = Vec::with_capacity(out_w as usize * out_h as usize);
    for y in 0..out_h {
        let src_y = if flip_y { out_h - 1 - y } else { y };
        let v = (src_y as f32 + 0.5) / out_h as f32;
        for x in 0..out_w {
            let u = (x as f32 + 0.5) / out_w as f32;
            let rgba = std::array::from_fn(|c| {
                let slot = &slots[c];
                if let Some(img) = &slot.image {
                    let px = if img.width == out_w && img.height == out_h {
                        img.pixels[src_y as usize * out_w as usize + x as usize]
                    } else {
                        img.sample_bilinear(u, v)
                    };
                    let extracted = slot.source_channel.extract(px);
                    if slot.invert { 255 - extracted } else { extracted }
                } else {
                    slot.default_value
                }
            });
            pixels.push(rgba);
        }
    }

    let assembled = RgbaImage2d {
        width: out_w,
        height: out_h,
        pixels,
        source_format: "Assembled RGBA8".to_string(),
        source_mips: 1,
        source_is_srgb: true,
    };
    Some((assembled, mixed_sizes))
}

/// Active channel filter for 2D preview viewports in the texture tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PreviewChannel {
    #[default]
    All,
    R,
    G,
    B,
    A,
}

impl PreviewChannel {
    pub const ALL: [PreviewChannel; 5] =
        [PreviewChannel::All, PreviewChannel::R, PreviewChannel::G, PreviewChannel::B, PreviewChannel::A];

    pub fn label(self) -> &'static str {
        match self {
            PreviewChannel::All => "All",
            PreviewChannel::R => "R",
            PreviewChannel::G => "G",
            PreviewChannel::B => "B",
            PreviewChannel::A => "A",
        }
    }

    pub fn tip(self) -> &'static str {
        match self {
            PreviewChannel::All => "Show full RGBA image",
            PreviewChannel::R => "Preview Red channel as grayscale",
            PreviewChannel::G => "Preview Green channel as grayscale",
            PreviewChannel::B => "Preview Blue channel as grayscale",
            PreviewChannel::A => "Preview Alpha channel as opaque grayscale",
        }
    }
}

/// Converts an [`RgbaImage2d`] into a 2D Bevy [`Image`] (`TextureFormat::Rgba8UnormSrgb`) for UI display.
/// When `channel` is `R`, `G`, `B`, or `A`, that single channel is replicated into `[v, v, v, 255]` so even the alpha
/// channel renders as opaque grayscale.
pub fn rgba_to_bevy_image(img: &RgbaImage2d, channel: PreviewChannel) -> Image {
    let w = img.width.max(1);
    let h = img.height.max(1);
    let mut data = Vec::with_capacity(img.pixels.len() * 4);
    for &[r, g, b, a] in &img.pixels {
        let px = match channel {
            PreviewChannel::All => [r, g, b, a],
            PreviewChannel::R => [r, r, r, 255],
            PreviewChannel::G => [g, g, g, 255],
            PreviewChannel::B => [b, b, b, 255],
            PreviewChannel::A => [a, a, a, 255],
        };
        data.extend_from_slice(&px);
    }
    Image::new(
        Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    )
}

/// Generates a 128x128 procedural texture with color gradients, geometric rings, and a distinct alpha ring for
/// `SHADER_EDITOR_OPEN="@tool/texture_converter:demo"` and unit tests.
pub fn demo_converter_image() -> RgbaImage2d {
    let (w, h) = (128u32, 128u32);
    let mut pixels = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        let v = (y as f32 + 0.5) / h as f32;
        let dy = v * 2.0 - 1.0;
        for x in 0..w {
            let u = (x as f32 + 0.5) / w as f32;
            let dx = u * 2.0 - 1.0;
            let dist = (dx * dx + dy * dy).sqrt();
            let angle = dy.atan2(dx);

            // Warm-to-cool gradient modulated by concentric rings and angular sectors.
            let ring = ((dist * 14.0).cos() * 0.5 + 0.5).clamp(0.0, 1.0);
            let spokes = ((angle * 6.0).sin() * 0.5 + 0.5).clamp(0.0, 1.0);
            let grid = if (x / 16 + y / 16) % 2 == 0 { 1.0 } else { 0.82 };

            let r = ((0.18 + 0.72 * u * grid + 0.10 * ring).clamp(0.0, 1.0) * 255.0).round() as u8;
            let g = ((0.15 + 0.68 * (1.0 - v) * grid + 0.17 * spokes).clamp(0.0, 1.0) * 255.0).round() as u8;
            let b = ((0.25 + 0.55 * ring + 0.20 * (1.0 - u)).clamp(0.0, 1.0) * 255.0).round() as u8;

            // Smooth alpha vignette with a semi-transparent outer ring so Alpha preview is clearly distinct.
            let alpha_f = if dist < 0.55 {
                1.0
            } else if dist < 0.82 {
                0.45 + 0.35 * ((dist - 0.55) / 0.27 * std::f32::consts::TAU).cos()
            } else {
                (1.0 - (dist - 0.82) / 0.35).clamp(0.15, 1.0)
            };
            let a = (alpha_f.clamp(0.0, 1.0) * 255.0).round() as u8;
            pixels.push([r, g, b, a]);
        }
    }
    RgbaImage2d {
        width: w,
        height: h,
        pixels,
        source_format: "Rgba8UnormSrgb".to_string(),
        source_mips: 1,
        source_is_srgb: true,
    }
}

/// Generates 4 distinct 128x128 procedural channel slots (`R`: radial dome, `G`: checker grid, `B`: diagonal waves,
/// `A`: circular mask) for `SHADER_EDITOR_OPEN="@tool/texture_assembler:demo"` and unit tests.
pub fn demo_assembler_slots() -> [ChannelSlot; 4] {
    let (w, h) = (128u32, 128u32);
    let make_pattern = |path: &str, f: fn(u32, u32, f32, f32) -> u8| -> (PathBuf, RgbaImage2d) {
        let mut pixels = Vec::with_capacity((w * h) as usize);
        for y in 0..h {
            let v = (y as f32 + 0.5) / h as f32;
            for x in 0..w {
                let u = (x as f32 + 0.5) / w as f32;
                let val = f(x, y, u, v);
                pixels.push([val, val, val, 255]);
            }
        }
        (PathBuf::from(path), RgbaImage2d::new(w, h, pixels))
    };

    let (r_path, r_img) = make_pattern("ao_dome.png", |_, _, u, v| {
        let dx = u * 2.0 - 1.0;
        let dy = v * 2.0 - 1.0;
        let d = (1.0 - (dx * dx + dy * dy).sqrt().clamp(0.0, 1.0)).powf(0.75);
        (d * 255.0).round() as u8
    });
    let (g_path, g_img) = make_pattern("roughness_grid.png", |x, y, u, _| {
        let cell = if ((x / 16) + (y / 16)) % 2 == 0 { 0.78 } else { 0.28 };
        let grad = 0.85 + 0.15 * u;
        ((cell * grad).clamp(0.0, 1.0) * 255.0).round() as u8
    });
    let (b_path, b_img) = make_pattern("metallic_stripes.png", |_, _, u, v| {
        let wave = ((u * 5.0 + v * 5.0) * std::f32::consts::TAU).sin() * 0.5 + 0.5;
        (wave.clamp(0.0, 1.0) * 255.0).round() as u8
    });
    let (a_path, a_img) = make_pattern("opacity_mask.png", |_, _, u, v| {
        let dx = u * 2.0 - 1.0;
        let dy = v * 2.0 - 1.0;
        let r = (dx * dx + dy * dy).sqrt();
        let mask = 1.0 - ((r - 0.62) / 0.18).clamp(0.0, 1.0);
        (mask * 255.0).round() as u8
    });

    let mut slots = ChannelSlot::default_slots();
    slots[0].set_image(r_path, r_img);
    slots[1].set_image(g_path, g_img);
    slots[2].set_image(b_path, b_img);
    slots[3].set_image(a_path, a_img);
    slots
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::render::render_resource::TextureViewDimension;

    fn sample_test_image(width: u32, height: u32) -> RgbaImage2d {
        let mut pixels = Vec::with_capacity((width * height) as usize);
        for y in 0..height {
            for x in 0..width {
                let r = ((x * 37 + y * 13) & 0xFF) as u8;
                let g = ((x * 19 + y * 53) & 0xFF) as u8;
                let b = ((x * 71 + y * 29) & 0xFF) as u8;
                let a = 200u8.wrapping_add(((x + y) & 0x1F) as u8);
                pixels.push([r, g, b, a]);
            }
        }
        RgbaImage2d::new(width, height, pixels)
    }

    /// Minimal 6-face KTX2 header + payload for verifying that cubemaps are rejected by `RgbaImage2d::from_bytes`.
    fn make_cubemap_ktx2(side: u32) -> Vec<u8> {
        let face_bytes = (side * side * 4) as usize;
        let data_at = 80 + 24 + 4;
        let mut bytes = Vec::from(KTX2_IDENTIFIER);
        for v in [VK_FORMAT_R8G8B8A8_UNORM, 1, side, side, 0, 0, 6, 1, 0] {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        for v in [104u32, 4, 0, 0] {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        bytes.extend_from_slice(&[0u8; 16]);
        for v in [data_at as u64, (6 * face_bytes) as u64, (6 * face_bytes) as u64] {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        bytes.extend_from_slice(&4u32.to_le_bytes());
        bytes.extend(std::iter::repeat_n(128u8, 6 * face_bytes));
        bytes
    }

    #[test]
    fn png_encode_and_decode_roundtrip() {
        let src = sample_test_image(12, 8);
        let png_bytes = encode_png(&src).expect("encode PNG");
        let decoded = RgbaImage2d::from_bytes(&png_bytes, Path::new("test.png")).expect("decode PNG");
        assert_eq!((decoded.width, decoded.height), (12, 8));
        assert_eq!(decoded.pixels, src.pixels);
        assert_eq!(decoded.source_mips, 1);
        assert!(decoded.source_is_srgb);

        // Also verify Bevy's Image::from_buffer decodes the PNG directly.
        let bevy_img = Image::from_buffer(
            &png_bytes,
            ImageType::Extension("png"),
            CompressedImageFormats::NONE,
            true,
            ImageSampler::Default,
            RenderAssetUsages::default(),
        )
        .expect("Bevy decode PNG");
        assert_eq!((bevy_img.width(), bevy_img.height()), (12, 8));
    }

    #[test]
    fn ktx2_encode_and_decode_all_combinations() {
        let src = sample_test_image(16, 8);
        let expected_mips = 16u32.ilog2() + 1; // 5 mips: 16x8, 8x4, 4x2, 2x1, 1x1

        for color_space in Ktx2ColorSpace::ALL {
            for mipmaps in [false, true] {
                for zstd in [false, true] {
                    let settings = TextureExportSettings {
                        output_format: OutputFormat::Ktx2,
                        resize: ResizePreset::Original,
                        flip_y: false,
                        ktx2_color_space: color_space,
                        mipmaps,
                        zstd,
                    };
                    let ktx2_bytes = encode_texture(&src, &settings).expect("encode KTX2");

                    // 1. Accepted as Plain2d by texture_kit.
                    assert_eq!(shape_problem(&ktx2_bytes, TextureShape::Plain2d), None);

                    // 2. Parsed cleanly by ktx2::Reader.
                    let reader = ktx2::Reader::new(&ktx2_bytes[..]).expect("valid KTX2");
                    let header = reader.header();
                    let want_mips = if mipmaps { expected_mips } else { 1 };
                    assert_eq!((header.pixel_width, header.pixel_height), (16, 8));
                    assert_eq!((header.layer_count, header.face_count, header.level_count), (0, 1, want_mips));
                    assert_eq!(
                        header.supercompression_scheme,
                        if zstd { Some(ktx2::SupercompressionScheme::Zstandard) } else { None }
                    );

                    // 3. Decoded cleanly by Bevy's Image::from_buffer with expected mips and format.
                    let bevy_img = Image::from_buffer(
                        &ktx2_bytes,
                        ImageType::Extension("ktx2"),
                        CompressedImageFormats::NONE,
                        color_space.is_srgb(),
                        ImageSampler::Default,
                        RenderAssetUsages::default(),
                    )
                    .expect("Bevy decode KTX2");
                    assert_eq!((bevy_img.width(), bevy_img.height()), (16, 8));
                    assert_eq!(bevy_img.texture_descriptor.mip_level_count, want_mips);
                    let want_fmt = match color_space {
                        Ktx2ColorSpace::Srgb => TextureFormat::Rgba8UnormSrgb,
                        Ktx2ColorSpace::Linear => TextureFormat::Rgba8Unorm,
                    };
                    assert_eq!(bevy_img.texture_descriptor.format, want_fmt);
                    let view_dim = bevy_img.texture_view_descriptor.as_ref().and_then(|v| v.dimension);
                    assert!(view_dim.is_none_or(|d| d == TextureViewDimension::D2));

                    // 4. Decoded back into RgbaImage2d (including multi-mip truncation and Zstd decompression).
                    let roundtrip =
                        RgbaImage2d::from_bytes(&ktx2_bytes, Path::new("roundtrip.ktx2")).expect("from_bytes KTX2");
                    assert_eq!((roundtrip.width, roundtrip.height), (16, 8));
                    assert_eq!(roundtrip.source_mips, want_mips);
                    assert_eq!(roundtrip.source_is_srgb, color_space.is_srgb());
                    assert_eq!(roundtrip.pixels, src.pixels);
                }
            }
        }
    }

    #[test]
    fn jpg_and_jpeg_decode_roundtrip() {
        let (w, h) = (8u32, 8u32);
        let rgb = [180u8, 90, 45].repeat((w * h) as usize);
        let mut jpg_bytes = Vec::new();
        {
            let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpg_bytes, 100);
            enc.encode(&rgb, w, h, image::ExtendedColorType::Rgb8).expect("encode JPEG");
        }

        for ext_name in ["photo.jpg", "PHOTO.JPEG"] {
            let decoded = RgbaImage2d::from_bytes(&jpg_bytes, Path::new(ext_name)).expect("decode JPEG");
            assert_eq!((decoded.width, decoded.height), (8, 8));
            assert_eq!(decoded.source_mips, 1);
            assert!(decoded.source_is_srgb);
            assert_eq!(decoded.pixels.len(), 64);
            for &[r, g, b, a] in &decoded.pixels {
                assert!(r.abs_diff(180) <= 3, "r={r}");
                assert!(g.abs_diff(90) <= 3, "g={g}");
                assert!(b.abs_diff(45) <= 3, "b={b}");
                assert_eq!(a, 255);
            }
        }
    }

    #[test]
    fn rejects_unsupported_extensions_and_cubemap_ktx2() {
        let err_ext = RgbaImage2d::from_bytes(b"1234", Path::new("photo.bmp")).unwrap_err();
        assert!(err_ext.contains("unsupported image format `bmp` (supported: png, jpg, jpeg, ktx2)"), "{err_ext}");

        let cube_bytes = make_cubemap_ktx2(4);
        let err_cube = RgbaImage2d::from_bytes(&cube_bytes, Path::new("sky.ktx2")).unwrap_err();
        assert!(err_cube.contains("cube map"), "{err_cube}");

        let dir = tempfile::tempdir().unwrap();
        let cube_path = dir.path().join("sky.ktx2");
        std::fs::write(&cube_path, &cube_bytes).unwrap();
        let err_file = RgbaImage2d::from_file(&cube_path).unwrap_err();
        assert!(err_file.contains("cube map"), "{err_file}");
    }

    #[test]
    fn resize_and_flip_y_transform_pixels_and_summaries() {
        let mut src = RgbaImage2d::new(2, 2, vec![[10, 0, 0, 255], [20, 0, 0, 255], [30, 0, 0, 255], [40, 0, 0, 255]]);
        src.source_format = "Rgba8UnormSrgb".to_string();

        let flipped = prepare_output_image(&src, ResizePreset::Original, true);
        assert_eq!(flipped.pixels, vec![[30, 0, 0, 255], [40, 0, 0, 255], [10, 0, 0, 255], [20, 0, 0, 255]]);

        let resized = prepare_output_image(&src, ResizePreset::Size256, false);
        assert_eq!((resized.width, resized.height), (256, 256));
        assert_eq!(resized.pixels.len(), 256 * 256);

        let ktx_settings = TextureExportSettings {
            output_format: OutputFormat::Ktx2,
            resize: ResizePreset::Size1024,
            flip_y: false,
            ktx2_color_space: Ktx2ColorSpace::Srgb,
            mipmaps: true,
            zstd: true,
        };
        assert_eq!(ktx_settings.output_summary(64, 64), "1024x1024 | KTX2 | RGBA8 (sRGB) | 11 mips | Zstd");

        let png_settings = TextureExportSettings {
            output_format: OutputFormat::Png,
            resize: ResizePreset::Original,
            ..TextureExportSettings::default()
        };
        assert_eq!(png_settings.output_summary(512, 256), "512x256 | PNG | RGBA8");
        assert_eq!(png_settings.mip_count(512, 256), 1);
    }

    #[test]
    fn assemble_channels_supports_partial_slots_extraction_invert_and_mixed_sizes() {
        let mut slots = ChannelSlot::default_slots();
        assert!(assemble_channels(&slots, ResizePreset::Original, false).is_none());

        // Load only R and B with different dimensions; leave G (default 0) and A (default 255) empty.
        let r_img = RgbaImage2d::new(2, 2, vec![[100, 50, 25, 200]; 4]);
        let b_img = RgbaImage2d::new(4, 4, vec![[10, 80, 200, 128]; 16]);

        slots[0].set_image(PathBuf::from("r.png"), r_img);
        slots[0].source_channel = SourceChannel::G; // extract G = 50
        slots[0].invert = true; // 255 - 50 = 205

        slots[2].set_image(PathBuf::from("b.png"), b_img);
        slots[2].source_channel = SourceChannel::B; // extract B = 200

        let (assembled, mixed) = assemble_channels(&slots, ResizePreset::Original, false).expect("assembled");
        assert!(mixed, "2x2 and 4x4 should report mixed_sizes = true");
        assert_eq!((assembled.width, assembled.height), (2, 2));
        for px in &assembled.pixels {
            assert_eq!(*px, [205, 0, 200, 255]);
        }

        // Check Luminance extraction.
        let white = SourceChannel::Luminance.extract([255, 255, 255, 0]);
        let black = SourceChannel::Luminance.extract([0, 0, 0, 255]);
        let green = SourceChannel::Luminance.extract([0, 255, 0, 255]);
        assert_eq!(white, 255);
        assert_eq!(black, 0);
        assert!(green > 170 && green < 190, "Rec.709 green luminance ~182, got {green}");
    }

    #[test]
    fn preview_channel_and_demos_produce_expected_images() {
        let demo = demo_converter_image();
        assert_eq!((demo.width, demo.height), (128, 128));
        for ch in PreviewChannel::ALL {
            let img = rgba_to_bevy_image(&demo, ch);
            assert_eq!((img.width(), img.height()), (128, 128));
            let data = img.data.as_ref().unwrap();
            assert_eq!(data.len(), 128 * 128 * 4);
            if ch != PreviewChannel::All {
                assert!(data.as_chunks::<4>().0.iter().all(|px| px[0] == px[1] && px[1] == px[2] && px[3] == 255));
            }
        }

        let slots = demo_assembler_slots();
        let (assembled, mixed) = assemble_channels(&slots, ResizePreset::Original, false).expect("demo assemble");
        assert!(!mixed);
        assert_eq!((assembled.width, assembled.height), (128, 128));
    }
}
