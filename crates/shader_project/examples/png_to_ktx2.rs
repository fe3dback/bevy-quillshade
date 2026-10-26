//! Converts a PNG into an uncompressed KTX2 texture (RGBA8, one mip level, no supercompression), so the sample game
//! exercises both image formats Texture assets accept (design §4.4) without external tools such as `toktx`:
//! `cargo run -p shader_project --example png_to_ktx2 -- [--linear] <in.png> [<out.ktx2>]`
//!
//! The vkFormat is `VK_FORMAT_R8G8B8A8_SRGB` by default and `VK_FORMAT_R8G8B8A8_UNORM` with `--linear` (data
//! textures such as normal maps). Bevy decides sRGB vs linear from the Texture asset's `is_srgb` setting, so the flag
//! only keeps the file self-describing. The output defaults to the input path with the `.ktx2` extension.
//!
//! The sample's `assets/textures/{rock_albedo,water_normal}.ktx2` were made with:
//! `png_to_ktx2 rock_albedo.png` and `png_to_ktx2 --linear water_normal.png`.

use std::path::{Path, PathBuf};

/// KTX2 file identifier: `«KTX 20»\r\n\x1A\n`.
const IDENTIFIER: [u8; 12] = [0xAB, 0x4B, 0x54, 0x58, 0x20, 0x32, 0x30, 0xBB, 0x0D, 0x0A, 0x1A, 0x0A];
const VK_FORMAT_R8G8B8A8_UNORM: u32 = 37;
const VK_FORMAT_R8G8B8A8_SRGB: u32 = 43;
/// Identifier + 9 header `u32`s + the section index (4 `u32`s, 2 `u64`s).
const HEADER_LEN: usize = 12 + 9 * 4 + 4 * 4 + 2 * 8;
const LEVEL_INDEX_LEN: usize = 3 * 8;
const WRITER: &str = "shader-editor png_to_ktx2";

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let linear = match args.iter().position(|a| a == "--linear") {
        Some(i) => {
            args.remove(i);
            true
        }
        None => false,
    };
    let (input, output) = match args.as_slice() {
        [input] => (PathBuf::from(input), Path::new(input).with_extension("ktx2")),
        [input, output] => (PathBuf::from(input), PathBuf::from(output)),
        _ => {
            eprintln!("usage: png_to_ktx2 [--linear] <in.png> [<out.ktx2>]");
            std::process::exit(2);
        }
    };
    let (width, height, rgba) = decode_png(&input).unwrap_or_else(|e| {
        eprintln!("{}: {e}", input.display());
        std::process::exit(1);
    });
    let bytes = encode_ktx2(width, height, &rgba, !linear);
    std::fs::write(&output, bytes).unwrap_or_else(|e| {
        eprintln!("{}: {e}", output.display());
        std::process::exit(1);
    });
    println!(
        "{} -> {} ({width}x{height}, {})",
        input.display(),
        output.display(),
        if linear { "UNORM" } else { "SRGB" }
    );
}

/// Decodes any PNG into 8-bit RGBA: palette / low bit depths are expanded, 16-bit is stripped, gray and missing alpha
/// are filled in.
fn decode_png(path: &Path) -> Result<(u32, u32, Vec<u8>), String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
    decoder.set_transformations(png::Transformations::normalize_to_color8() | png::Transformations::ALPHA);
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let mut buf = vec![0; reader.output_buffer_size().ok_or("image too large")?];
    let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    buf.truncate(info.buffer_size());
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::GrayscaleAlpha => buf.as_chunks::<2>().0.iter().flat_map(|&[g, a]| [g, g, g, a]).collect(),
        other => return Err(format!("unexpected decoded color type {other:?}")),
    };
    Ok((info.width, info.height, rgba))
}

/// Builds a KTX2 file: header, one level index entry, a basic data format descriptor (RGBA, BT.709 primaries, sRGB or
/// linear transfer), a `KTXwriter` key/value entry, then the level data (offset aligned to 4 bytes).
fn encode_ktx2(width: u32, height: u32, rgba: &[u8], srgb: bool) -> Vec<u8> {
    assert_eq!(rgba.len(), width as usize * height as usize * 4, "RGBA8 data size");
    let dfd = data_format_descriptor(srgb);
    let kvd = key_value(b"KTXwriter", WRITER.as_bytes());
    let dfd_offset = HEADER_LEN + LEVEL_INDEX_LEN;
    let kvd_offset = dfd_offset + dfd.len();
    let level_offset = (kvd_offset + kvd.len()).next_multiple_of(4);

    let mut out = Vec::with_capacity(level_offset + rgba.len());
    out.extend_from_slice(&IDENTIFIER);
    let format = if srgb { VK_FORMAT_R8G8B8A8_SRGB } else { VK_FORMAT_R8G8B8A8_UNORM };
    // vkFormat, typeSize, pixelWidth, pixelHeight, pixelDepth, layerCount, faceCount, levelCount, supercompression.
    for v in [format, 1, width, height, 0, 0, 1, 1, 0] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    for v in [dfd_offset, dfd.len(), kvd_offset, kvd.len()] {
        out.extend_from_slice(&(v as u32).to_le_bytes());
    }
    // sgdByteOffset, sgdByteLength: no supercompression global data.
    out.extend_from_slice(&[0; 16]);
    // Level 0: byteOffset, byteLength, uncompressedByteLength.
    for v in [level_offset, rgba.len(), rgba.len()] {
        out.extend_from_slice(&(v as u64).to_le_bytes());
    }
    debug_assert_eq!(out.len(), dfd_offset);
    out.extend_from_slice(&dfd);
    out.extend_from_slice(&kvd);
    out.resize(level_offset, 0);
    out.extend_from_slice(rgba);
    out
}

/// `dfdTotalSize` + one Khronos basic descriptor block with four 8-bit samples (R, G, B, A).
fn data_format_descriptor(srgb: bool) -> Vec<u8> {
    const SAMPLE_LEN: usize = 16;
    let block_len = 24 + 4 * SAMPLE_LEN;
    let mut d = Vec::with_capacity(4 + block_len);
    d.extend_from_slice(&((4 + block_len) as u32).to_le_bytes());
    d.extend_from_slice(&0u32.to_le_bytes()); // vendorId 0 (Khronos), descriptorType 0 (basic)
    d.extend_from_slice(&2u16.to_le_bytes()); // versionNumber (KDFS 1.3)
    d.extend_from_slice(&(block_len as u16).to_le_bytes());
    // colorModel RGBSDA, colorPrimaries BT709, transferFunction sRGB / linear, flags: straight alpha.
    d.extend_from_slice(&[1, 1, if srgb { 2 } else { 1 }, 0]);
    d.extend_from_slice(&[0; 4]); // texelBlockDimension 1x1x1x1 (stored minus one)
    d.extend_from_slice(&[4, 0, 0, 0, 0, 0, 0, 0]); // bytesPlane0..7
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

/// One key/value entry: `keyAndValueByteLength`, `key\0value\0`, padded to 4 bytes.
fn key_value(key: &[u8], value: &[u8]) -> Vec<u8> {
    let len = key.len() + 1 + value.len() + 1;
    let mut kv = Vec::with_capacity(4 + len.next_multiple_of(4));
    kv.extend_from_slice(&(len as u32).to_le_bytes());
    kv.extend_from_slice(key);
    kv.push(0);
    kv.extend_from_slice(value);
    kv.push(0);
    kv.resize(4 + len.next_multiple_of(4), 0);
    kv
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u32_at(b: &[u8], at: usize) -> u32 {
        u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
    }

    #[test]
    fn ktx2_header_is_valid() {
        let rgba: Vec<u8> = (0..2 * 3 * 4).map(|i| i as u8).collect();
        for (srgb, format) in [(true, VK_FORMAT_R8G8B8A8_SRGB), (false, VK_FORMAT_R8G8B8A8_UNORM)] {
            let b = encode_ktx2(2, 3, &rgba, srgb);
            assert_eq!(&b[..12], b"\xABKTX 20\xBB\r\n\x1A\n");
            assert_eq!(u32_at(&b, 12), format, "vkFormat");
            assert_eq!(u32_at(&b, 16), 1, "typeSize");
            assert_eq!((u32_at(&b, 20), u32_at(&b, 24)), (2, 3), "width, height");
            assert_eq!(u32_at(&b, 40), 1, "levelCount");
            assert_eq!(u32_at(&b, 44), 0, "supercompressionScheme");

            // The `ktx2` crate Bevy 0.20 loads `.ktx2` with parses it back.
            let reader = ktx2::Reader::new(&b[..]).expect("valid KTX2");
            let header = reader.header();
            let want = if srgb { ktx2::Format::R8G8B8A8_SRGB } else { ktx2::Format::R8G8B8A8_UNORM };
            assert_eq!(header.format, Some(want));
            assert_eq!((header.pixel_width, header.pixel_height, header.level_count), (2, 3, 1));
            assert_eq!(header.supercompression_scheme, None);
            let levels: Vec<_> = reader.levels().collect();
            assert_eq!(levels.len(), 1);
            assert_eq!(levels[0].data, &rgba[..]);
            assert_eq!(levels[0].uncompressed_byte_length, rgba.len() as u64);
            let want_tf = if srgb { ktx2::TransferFunction::SRGB } else { ktx2::TransferFunction::Linear };
            assert_eq!(reader.transfer_function(), Some(want_tf));
            assert_eq!(reader.writer(), Some(&*format!("{WRITER}\0")));
        }
    }

    /// The committed sample textures are exactly what the generator makes from the committed PNGs.
    #[test]
    fn sample_ktx2_files_are_up_to_date() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/sample_game/assets/textures");
        for (name, srgb) in [("rock_albedo", true), ("water_normal", false)] {
            let (w, h, rgba) = decode_png(&dir.join(format!("{name}.png"))).unwrap();
            let committed = std::fs::read(dir.join(format!("{name}.ktx2"))).unwrap();
            assert!(committed == encode_ktx2(w, h, &rgba, srgb), "{name}.ktx2 is stale: rerun png_to_ktx2");
        }
    }
}
