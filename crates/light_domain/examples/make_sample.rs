//! Writes the sample game's Light assets (design §9.2): the cube map `assets/textures/sky_cube.ktx2`, its Light
//! Texture asset `quillshade/light/sky.texture.ron`, the environments `quillshade/light/day.environment.ron` and
//! `quillshade/light/night.environment.ron`, and the transition curve `quillshade/light/smooth.curve.ron`.
//! Then regenerate with the CLI, which knows every domain (the umbrella crate lists them all):
//!
//! ```sh
//! cargo run -p light_domain --example make_sample -- examples/sample_game
//! cargo run -- regen examples/sample_game
//! ```
//!
//! The cube map is a tiny uncompressed KTX2 written here without external tools such as `toktx`: RGBA8 sRGB
//! (`VK_FORMAT_R8G8B8A8_SRGB`), 6 faces of 4x4, one mip level, no array layers, no supercompression. The faces form a
//! simple sky: a blue zenith, sides fading from blue to a pale horizon, a dark ground.

use curve_kit::Curve;
use light_domain::backend::{
    DirectionalLightDef, Environment, FogDef, FogFalloffDef, LightBackend, curve_ref, environment_ref, save_curve,
    save_environment, texture_ref,
};
use project_core::Project;
use project_core::regen::DomainBackend;
use texture_kit::TextureAsset;

/// KTX2 file identifier: `«KTX 20»\r\n\x1A\n`.
const IDENTIFIER: [u8; 12] = [0xAB, 0x4B, 0x54, 0x58, 0x20, 0x32, 0x30, 0xBB, 0x0D, 0x0A, 0x1A, 0x0A];
const VK_FORMAT_R8G8B8A8_SRGB: u32 = 43;
/// Identifier + 9 header `u32`s + the section index (4 `u32`s, 2 `u64`s).
const HEADER_LEN: usize = 12 + 9 * 4 + 4 * 4 + 2 * 8;
/// One level index entry (byteOffset, byteLength, uncompressedByteLength).
const LEVEL_INDEX_LEN: usize = 3 * 8;
const WRITER: &str = "shader-editor light_domain make_sample";

/// Side of a cube face, in pixels.
const SIDE: u32 = 4;
/// The game file of the cube map, under the game's `assets/`.
const CUBE_FILE: &str = "textures/sky_cube.ktx2";
/// The Light Texture asset of the cube map (domain-relative, no suffix).
const TEXTURE: &str = "sky";
/// The day environment (domain-relative, no suffix).
const ENVIRONMENT: &str = "day";
/// The night environment (domain-relative, no suffix).
const NIGHT_ENVIRONMENT: &str = "night";
/// The smooth transition curve (domain-relative, no suffix).
const SMOOTH_CURVE: &str = "smooth";

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| "examples/sample_game".into());
    let project = Project::open(&dir).unwrap_or_else(|e| panic!("{e}"));
    let cube = project.assets_dir().join(CUBE_FILE);
    std::fs::create_dir_all(cube.parent().expect("a parent")).expect("textures dir");
    std::fs::write(&cube, sky_cube()).expect("write the cube map");
    texture_kit::save(&project.root.join(texture_ref(TEXTURE).source_rel()), &TextureAsset::new(CUBE_FILE))
        .expect("write the Texture asset");
    save_environment(&project.root.join(environment_ref(ENVIRONMENT).source_rel()), &day_environment())
        .expect("write the day environment");
    save_environment(&project.root.join(environment_ref(NIGHT_ENVIRONMENT).source_rel()), &night_environment())
        .expect("write the night environment");
    save_curve(&project.root.join(curve_ref(SMOOTH_CURVE).source_rel()), &smooth_curve())
        .expect("write the smooth curve");
    let plan = LightBackend.plan(&project, false);
    for d in &plan.diagnostics {
        eprintln!("{d}");
    }
    println!("Light assets written ({} diagnostic(s)); now run `cargo run -- regen {dir}`", plan.diagnostics.len());
}

fn day_environment() -> Environment {
    Environment { cubemap: Some(TEXTURE.into()), ..Environment::default() }
}

fn night_environment() -> Environment {
    Environment {
        clear_color: [0.02, 0.03, 0.06, 1.0],
        cubemap: Some(TEXTURE.into()),
        skybox_brightness: 150.0,
        skybox_rotation: 180.0,
        env_intensity: 200.0,
        sky_top_color: [0.05, 0.08, 0.18, 1.0],
        sky_horizon_color: [0.10, 0.14, 0.25, 1.0],
        ground_color: [0.02, 0.02, 0.04, 1.0],
        ambient_color: [0.6, 0.7, 1.0, 1.0],
        ambient_brightness: 40.0,
        lights: vec![
            DirectionalLightDef {
                name: "Moon".into(),
                color: [0.65, 0.78, 1.0, 1.0],
                illuminance: 1_200.0,
                azimuth_deg: -45.0,
                elevation_deg: 35.0,
                ..DirectionalLightDef::default()
            },
            DirectionalLightDef {
                name: "Sky".into(),
                color: [0.4, 0.5, 0.8, 1.0],
                illuminance: 300.0,
                azimuth_deg: 180.0,
                elevation_deg: 80.0,
                ..DirectionalLightDef::default()
            },
        ],
        ev100: 11.0,
        fog: FogDef {
            enabled: true,
            color: [0.02, 0.03, 0.06, 1.0],
            falloff: FogFalloffDef::Linear { start: 30.0, end: 220.0 },
            ..FogDef::default()
        },
        ..Environment::default()
    }
}

fn smooth_curve() -> Curve {
    Curve::new(SMOOTH_CURVE, vec![(0.0, 0.0), (0.25, 0.15), (0.75, 0.85), (1.0, 1.0)])
}

/// The sky's six faces in KTX2 order (+X, -X, +Y, -Y, +Z, -Z), each `SIDE` rows top to bottom of `SIDE` RGBA8 texels.
/// Rows of a side face run from up (+Y) to down; the top face is the zenith, the bottom one the ground.
fn sky_faces() -> Vec<u8> {
    const ZENITH: [u8; 3] = [46, 98, 184];
    const HORIZON: [u8; 3] = [196, 222, 244];
    const GROUND: [u8; 3] = [58, 54, 48];
    let side_row = |row: u32| -> [u8; 3] {
        let t = row as f32 / (SIDE - 1) as f32;
        std::array::from_fn(|c| (ZENITH[c] as f32 + (HORIZON[c] as f32 - ZENITH[c] as f32) * t).round() as u8)
    };
    let mut data = Vec::with_capacity((6 * SIDE * SIDE * 4) as usize);
    for face in 0..6 {
        for row in 0..SIDE {
            let [r, g, b] = match face {
                2 => ZENITH,
                3 => GROUND,
                _ => side_row(row),
            };
            for _ in 0..SIDE {
                data.extend_from_slice(&[r, g, b, 255]);
            }
        }
    }
    data
}

/// The committed `sky_cube.ktx2`.
fn sky_cube() -> Vec<u8> {
    encode_cube_ktx2(SIDE, &sky_faces())
}

/// Builds a KTX2 cube map: header (faceCount 6, layerCount 0, pixelDepth 0, one level), one level index entry, a basic
/// data format descriptor (RGBA, BT.709 primaries, sRGB transfer), a `KTXwriter` key/value entry, then the level data
/// (the six faces one after another, offset aligned to 4 bytes: the texel size and the spec's minimum).
fn encode_cube_ktx2(side: u32, faces: &[u8]) -> Vec<u8> {
    assert_eq!(faces.len(), 6 * side as usize * side as usize * 4, "six RGBA8 faces");
    let dfd = data_format_descriptor();
    let kvd = key_value(b"KTXwriter", WRITER.as_bytes());
    let dfd_offset = HEADER_LEN + LEVEL_INDEX_LEN;
    let kvd_offset = dfd_offset + dfd.len();
    // `kvd.len()` excludes the padding after the (only, so last) entry; the level data still starts aligned.
    let level_offset = (kvd_offset + kvd.len()).next_multiple_of(4);

    let mut out = Vec::with_capacity(level_offset + faces.len());
    out.extend_from_slice(&IDENTIFIER);
    // vkFormat, typeSize, pixelWidth, pixelHeight, pixelDepth, layerCount, faceCount, levelCount, supercompression.
    for v in [VK_FORMAT_R8G8B8A8_SRGB, 1, side, side, 0, 0, 6, 1, 0] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    for v in [dfd_offset, dfd.len(), kvd_offset, kvd.len()] {
        out.extend_from_slice(&(v as u32).to_le_bytes());
    }
    // sgdByteOffset, sgdByteLength: no supercompression global data.
    out.extend_from_slice(&[0; 16]);
    // Level 0: byteOffset, byteLength, uncompressedByteLength.
    for v in [level_offset, faces.len(), faces.len()] {
        out.extend_from_slice(&(v as u64).to_le_bytes());
    }
    debug_assert_eq!(out.len(), dfd_offset);
    out.extend_from_slice(&dfd);
    out.extend_from_slice(&kvd);
    out.resize(level_offset, 0);
    out.extend_from_slice(faces);
    out
}

/// `dfdTotalSize` + one Khronos basic descriptor block with four 8-bit samples (R, G, B, A), sRGB transfer.
fn data_format_descriptor() -> Vec<u8> {
    const SAMPLE_LEN: usize = 16;
    let block_len = 24 + 4 * SAMPLE_LEN;
    let mut d = Vec::with_capacity(4 + block_len);
    d.extend_from_slice(&((4 + block_len) as u32).to_le_bytes());
    d.extend_from_slice(&0u32.to_le_bytes()); // vendorId 0 (Khronos), descriptorType 0 (basic)
    d.extend_from_slice(&2u16.to_le_bytes()); // versionNumber (KDFS 1.3)
    d.extend_from_slice(&(block_len as u16).to_le_bytes());
    // colorModel RGBSDA, colorPrimaries BT709, transferFunction sRGB, flags: straight alpha.
    d.extend_from_slice(&[1, 1, 2, 0]);
    d.extend_from_slice(&[0; 4]); // texelBlockDimension 1x1x1x1 (stored minus one)
    d.extend_from_slice(&[4, 0, 0, 0, 0, 0, 0, 0]); // bytesPlane0..7
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
    d
}

/// One key/value entry: `keyAndValueByteLength`, `key\0value\0`. Not padded: the padding after the last entry is not
/// part of `kvdByteLength` (the caller aligns what follows).
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

    fn sample_game() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/sample_game")
    }

    /// The level 0 byteOffset (the first level index entry follows the 80-byte header).
    fn levels_offset(bytes: &[u8]) -> usize {
        u64::from_le_bytes(bytes[80..88].try_into().unwrap()) as usize
    }

    /// The `ktx2` crate (Bevy's KTX2 parser) reads the cube map back: format, size, six faces, one level, the data.
    #[test]
    fn cube_ktx2_parses() {
        let bytes = sky_cube();
        assert_eq!(shape_problem(&bytes, TextureShape::CubeMap), None, "the texture kit's cube check");
        assert!(shape_problem(&bytes, TextureShape::Plain2d).is_some());
        let reader = ktx2::Reader::new(&bytes[..]).expect("valid KTX2");
        let h = reader.header();
        assert_eq!(h.format, Some(ktx2::Format::R8G8B8A8_SRGB));
        assert_eq!((h.pixel_width, h.pixel_height, h.pixel_depth), (SIDE, SIDE, 0));
        assert_eq!((h.layer_count, h.face_count, h.level_count), (0, 6, 1));
        assert_eq!(h.supercompression_scheme, None);
        let levels: Vec<_> = reader.levels().collect();
        assert_eq!(levels.len(), 1);
        assert_eq!(levels[0].data, &sky_faces()[..]);
        assert_eq!(levels[0].uncompressed_byte_length, 6 * 4 * 4 * 4);
        assert_eq!(reader.transfer_function(), Some(ktx2::TransferFunction::SRGB));
        assert_eq!(reader.writer(), Some(&*format!("{WRITER}\0")));
        // kvdByteLength (header bytes 60..64) is the unpadded entry; the level data starts 4-aligned after it.
        let kvd_len = u32::from_le_bytes(bytes[60..64].try_into().unwrap()) as usize;
        assert_eq!(kvd_len, 4 + b"KTXwriter".len() + 1 + WRITER.len() + 1);
        assert_eq!(levels_offset(&bytes) % 4, 0);
    }

    /// Bevy's own decoder (what the game's `ImageLoader` runs) gives a 4x4 cube of six layers with a cube view.
    #[test]
    fn bevy_loads_the_cube_map() {
        let image = Image::from_buffer(
            &sky_cube(),
            ImageType::Extension("ktx2"),
            CompressedImageFormats::NONE,
            true,
            ImageSampler::Default,
            RenderAssetUsages::default(),
        )
        .expect("Bevy decodes the cube map");
        let size = image.texture_descriptor.size;
        assert_eq!((size.width, size.height, size.depth_or_array_layers), (SIDE, SIDE, 6));
        assert_eq!(image.texture_descriptor.format, TextureFormat::Rgba8UnormSrgb);
        assert_eq!(image.texture_descriptor.mip_level_count, 1);
        let view = image.texture_view_descriptor.as_ref().and_then(|v| v.dimension);
        assert_eq!(view, Some(TextureViewDimension::Cube));
        assert_eq!(image.data.as_deref(), Some(&sky_faces()[..]));
    }

    /// The committed sample files are exactly what this generator writes.
    #[test]
    fn sample_files_are_up_to_date() {
        let p = Project::open(sample_game()).unwrap();
        let stale = "stale: rerun `cargo run -p light_domain --example make_sample`";
        assert!(std::fs::read(p.assets_dir().join(CUBE_FILE)).unwrap() == sky_cube(), "{CUBE_FILE} is {stale}");
        let texture = texture_kit::load(&p.root.join(texture_ref(TEXTURE).source_rel())).unwrap();
        assert_eq!(texture, TextureAsset::new(CUBE_FILE), "{stale}");
        let day = light_domain::backend::load_environment(&p.root.join(environment_ref(ENVIRONMENT).source_rel()));
        assert_eq!(day.unwrap(), day_environment(), "{stale}");
        let night =
            light_domain::backend::load_environment(&p.root.join(environment_ref(NIGHT_ENVIRONMENT).source_rel()));
        assert_eq!(night.unwrap(), night_environment(), "{stale}");
        let smooth = light_domain::backend::load_curve(&p.root.join(curve_ref(SMOOTH_CURVE).source_rel()));
        assert_eq!(smooth.unwrap(), smooth_curve(), "{stale}");
    }
}
