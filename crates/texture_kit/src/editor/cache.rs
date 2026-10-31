//! Images of the game decoded for the editor (design §4.4): the image tab, the Texture tab's preview and a domain's
//! own thumbnails (Material's texture nodes). The editor's `AssetServer` serves the editor's own assets, so a game
//! file is read as bytes and decoded by Bevy (`Image::from_buffer`, the decoder Bevy's `ImageLoader` uses) — no
//! format-specific code: what the editor cannot show, the game could not load either, and the error text says why.
//! A domain's built-in textures (`@internal/…`, design §2, [`crate::TexturePolicy::internal`]) are decoded from the
//! bytes embedded in the editor. Cube maps, arrays and 3D textures are shown as a 2D strip of their faces
//! ([`stack_layers`]): the UI binds a 2D view.

use std::marker::PhantomData;
use std::path::Path;
use std::sync::Arc;
use std::time::SystemTime;

use bevy::asset::RenderAssetUsages;
use bevy::image::{
    CompressedImageFormatSupport, CompressedImageFormats, ImageAddressMode, ImageFilterMode, ImageSampler,
    ImageSamplerDescriptor, ImageType,
};
use bevy::prelude::*;
use bevy::render::renderer::RenderDevice;
use bevy::render::settings::WgpuFeatures;
use project_core::Project;

use super::{TextureDomain, internal_path};
use crate::{AddressMode, FilterMode, INTERNAL_PREFIX, TextureAsset, TextureSettings};

/// Modification time and size of a file: a different stamp means the file changed (the size also catches two
/// writes within one coarse mtime tick).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileStamp {
    modified: Option<SystemTime>,
    len: u64,
}

impl FileStamp {
    /// `None` when `path` is not a readable file.
    pub fn of(path: &Path) -> Option<FileStamp> {
        let meta = std::fs::metadata(path).ok().filter(|m| m.is_file())?;
        Some(FileStamp { modified: meta.modified().ok(), len: meta.len() })
    }
}

/// A preview's channel: 0 / 1 / 2 (R / G / B) shown alone in grey, `None` the whole image.
pub type Channel = Option<usize>;

/// A decoded game image in [`GameImageCache`]: its 2D [`Image`] handle (with layers stacked top to bottom), whether
/// a single channel was rewritten to grey pixels, whether the source image is a 6-face square cube map, and its
/// `TextureViewDimension::Cube` [`Image`] handle for the 3D sphere / skybox preview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedImage {
    pub handle: Handle<Image>,
    pub grey: bool,
    pub is_cubemap: bool,
    pub cube_handle: Option<Handle<Image>>,
}

/// `D`'s decoded game images by (path under `assets/`, settings). An entry is decoded again when the file's
/// [`FileStamp`] changed (checked on every [`GameImageCache::get`]), and dropped by
/// [`GameImageCache::free_unused`] once nobody but the cache holds its handle (run every frame by
/// [`GameImagePlugin`]): callers keep the handle for as long as they show the image. Paths follow `D`'s policy.
#[derive(Resource)]
pub struct GameImageCache<D> {
    /// Compressed texture formats the GPU can sample, from the render device (`NONE` without one, e.g. in tests):
    /// the same filter Bevy's `ImageLoader` applies.
    pub formats: CompressedImageFormats,
    /// The render device can sample with `ClampToBorder` (wgpu feature); `false` without a device.
    pub clamp_to_border: bool,
    entries: Vec<Entry>,
    domain: PhantomData<fn() -> D>,
}

impl<D> Default for GameImageCache<D> {
    fn default() -> Self {
        GameImageCache {
            formats: CompressedImageFormats::default(),
            clamp_to_border: false,
            entries: Vec::new(),
            domain: PhantomData,
        }
    }
}

struct Entry {
    path: String,
    settings: TextureSettings,
    /// The image reduced to this channel in grey ([`GameImageCache::get_channel`]).
    channel: Option<usize>,
    stamp: Option<FileStamp>,
    result: Result<CachedImage, String>,
}

fn handle_in_use(h: &Handle<Image>) -> bool {
    matches!(h, Handle::Strong(s) if Arc::strong_count(s) > 1)
}

impl<D: TextureDomain> GameImageCache<D> {
    /// The image `assets/<path>` decoded with `settings`, or why it cannot be shown. `path` must pass
    /// [`TextureAsset::check_path`] with `D`'s policy (relative, no `..`, its extensions): nothing outside `assets/`
    /// is read. An `@internal/…` id instead decodes that built-in texture of `D`'s embedded bytes (no file is
    /// read).
    /// `settings.expand_grayscale` is not applied: `Image::from_buffer` always expands grayscale images. Sampler
    /// settings wgpu cannot create (E013) are an error (nothing decoded: they must not take the editor down);
    /// `ClampToBorder` without the device feature is previewed as `ClampToEdge` ([`previewable`]).
    pub fn get(
        &mut self,
        project: &Project,
        path: &str,
        settings: &TextureSettings,
        images: &mut Assets<Image>,
    ) -> Result<Handle<Image>, String> {
        self.get_cached(project, path, settings, None, images).map(|c| c.handle)
    }

    /// [`GameImageCache::get`], with channel `channel` (0 / 1 / 2 = R / G / B) copied into R, G and B (opaque), so it
    /// shows in grey. The pixels exist only while decoding (images live in the render world), hence a cache entry of
    /// its own. `true` when the channel is shown so; an image whose format has no per-pixel bytes to rewrite
    /// (compressed, not 8-bit RGBA / BGRA) comes back unchanged with `false`.
    pub fn get_channel(
        &mut self,
        project: &Project,
        path: &str,
        settings: &TextureSettings,
        channel: Channel,
        images: &mut Assets<Image>,
    ) -> Result<(Handle<Image>, bool), String> {
        self.get_cached(project, path, settings, channel, images).map(|c| (c.handle, c.grey))
    }

    /// [`GameImageCache::get_channel`], returning the full [`CachedImage`] (including `is_cubemap` and `cube_handle`
    /// with `TextureViewDimension::Cube` when the source image is a 6-face square cube map).
    pub fn get_cached(
        &mut self,
        project: &Project,
        path: &str,
        settings: &TextureSettings,
        channel: Channel,
        images: &mut Assets<Image>,
    ) -> Result<CachedImage, String> {
        if let Some(Err(e)) = settings.sampler.map(|s| s.check()) {
            return Err(format!("E013 invalid sampler settings: {e}"));
        }
        let internal = match path.starts_with(INTERNAL_PREFIX) {
            true => Some(D::POLICY.internal_texture(path).ok_or_else(|| format!("unknown built-in texture `{path}`"))?),
            false => None,
        };
        if internal.is_none() {
            TextureAsset::check_path(path, &D::POLICY)?;
        }
        let file = project.assets_dir().join(path);
        // Embedded bytes never change: no stamp.
        let stamp = if internal.is_some() { None } else { FileStamp::of(&file) };
        let same = |e: &Entry| e.path == path && e.settings == *settings && e.channel == channel;
        if let Some(e) = self.entries.iter().find(|e| same(e) && e.stamp == stamp) {
            return e.result.clone();
        }
        self.entries.retain(|e| !same(e));
        let shown = previewable(settings, self.clamp_to_border).0;
        let first = D::PREVIEW_FIRST_LAYER;
        let decoded = match (internal, stamp) {
            (Some(t), _) => decode_entry(t.bytes, &internal_path::<D>(t), &shown, self.formats, first),
            (None, None) => Err(format!("assets/{path} does not exist")),
            (None, Some(_)) => std::fs::read(&file)
                .map_err(|e| format!("cannot read assets/{path}: {e}"))
                .and_then(|bytes| decode_entry(&bytes, path, &shown, self.formats, first)),
        };
        let result = decoded.map(|(mut image, mut cube)| {
            let mut grey = false;
            if let Some(c) = channel {
                grey = grey_channel(&mut image, c);
                if let Some(cube_img) = &mut cube {
                    grey_channel(cube_img, c);
                }
            }
            let is_cubemap = cube.is_some();
            let cube_handle = cube.map(|c| images.add(c));
            let handle = images.add(image);
            CachedImage { handle, grey, is_cubemap, cube_handle }
        });
        let (path, settings) = (path.to_string(), settings.clone());
        self.entries.push(Entry { path, settings, channel, stamp, result: result.clone() });
        result
    }
}

impl<D> GameImageCache<D> {
    /// Drops the images only the cache still holds (failures stay: they hold no image and are retried when the
    /// file changes).
    pub fn free_unused(&mut self) {
        self.entries.retain(|e| match &e.result {
            Ok(cached) => handle_in_use(&cached.handle) || cached.cube_handle.as_ref().is_some_and(handle_in_use),
            Err(_) => true,
        });
    }

    /// Number of cached entries (decoded images and failures).
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Decodes the bytes of `path` (its extension names the format) as Bevy's image loader would with `settings`; the
/// layers of a cube map, an array or a 3D image are stacked ([`stack_layers`]).
pub fn decode(
    bytes: &[u8],
    path: &str,
    settings: &TextureSettings,
    formats: CompressedImageFormats,
) -> Result<Image, String> {
    decode_layers(bytes, path, settings, formats, false)
}

/// [`decode`]; with `first_layer`, only the first face / layer / slice is kept
/// ([`super::TextureDomain::PREVIEW_FIRST_LAYER`]).
pub fn decode_layers(
    bytes: &[u8],
    path: &str,
    settings: &TextureSettings,
    formats: CompressedImageFormats,
    first_layer: bool,
) -> Result<Image, String> {
    decode_entry(bytes, path, settings, formats, first_layer).map(|(image, _)| image)
}

/// Decodes `bytes` into the 2D stacked [`Image`] plus an optional 6-layer `TextureViewDimension::Cube` [`Image`] when
/// the source texture is a 6-face square cube map and `!first_layer`.
fn decode_entry(
    bytes: &[u8],
    path: &str,
    settings: &TextureSettings,
    formats: CompressedImageFormats,
    first_layer: bool,
) -> Result<(Image, Option<Image>), String> {
    use bevy::render::render_resource::{TextureDimension, TextureViewDescriptor, TextureViewDimension};
    let ext = Path::new(path).extension().and_then(|e| e.to_str()).unwrap_or_default();
    let mut image = Image::from_buffer(
        bytes,
        ImageType::Extension(ext),
        formats,
        settings.is_srgb,
        sampler(settings),
        RenderAssetUsages::RENDER_WORLD,
    )
    .map_err(|e| format!("cannot decode assets/{path}: {e}"))?;
    let is_cubemap = !first_layer
        && image.texture_descriptor.dimension == TextureDimension::D2
        && image.texture_descriptor.array_layer_count() == 6
        && image.width() == image.height();
    let cube = is_cubemap.then(|| {
        let mut cube = image.clone();
        cube.texture_view_descriptor =
            Some(TextureViewDescriptor { dimension: Some(TextureViewDimension::Cube), ..default() });
        cube
    });
    if first_layer {
        // The first face / layer / slice is the first bytes of mip 0 in either data order: the stacking keeps it.
        image.texture_descriptor.size.depth_or_array_layers = 1;
    }
    stack_layers(&mut image).map_err(|e| format!("cannot show assets/{path}: {e}"))?;
    Ok((image, cube))
}

/// Turns a cube map, an array or a 3D (or 1D) image into one 2D image, mip 0 only: its faces / layers / slices
/// stacked top to bottom. Every UI image is bound as a 2D view; a cube view there fails bind group creation, which
/// quits the app (Bevy's default render error policy). A plain 2D image is left as it is.
pub fn stack_layers(image: &mut Image) -> Result<(), String> {
    use bevy::render::render_resource::{Extent3d, TextureDataOrder, TextureDimension, TextureViewDimension};
    let d = &image.texture_descriptor;
    let layers = d.size.depth_or_array_layers;
    let view_2d = image
        .texture_view_descriptor
        .as_ref()
        .is_none_or(|v| v.dimension.is_none_or(|x| x == TextureViewDimension::D2));
    if layers == 1 && d.dimension == TextureDimension::D2 && view_2d {
        return Ok(());
    }
    let (w, h, format) = (d.size.width, d.size.height, d.format);
    let (bw, bh) = format.block_dimensions();
    if layers > 1 && !h.is_multiple_of(bh) {
        return Err(format!("{layers} layers of height {h} do not stack in {format:?} blocks"));
    }
    if d.mip_level_count > 1 && image.data_order == TextureDataOrder::LayerMajor && layers > 1 {
        return Err("layer-major data with mipmaps".into());
    }
    let block = format.block_copy_size(None).ok_or_else(|| format!("{format:?} has no copyable blocks"))?;
    // Mip-major data starts with mip 0 of every layer, one after the other: exactly the stacked image.
    let level0 = (w.div_ceil(bw) * h.div_ceil(bh) * block * layers) as usize;
    if let Some(data) = image.data.as_mut() {
        if data.len() < level0 {
            return Err(format!("{} bytes of data, mip 0 needs {level0}", data.len()));
        }
        data.truncate(level0);
    }
    let d = &mut image.texture_descriptor;
    d.size = Extent3d { width: w, height: h * layers, depth_or_array_layers: 1 };
    d.dimension = TextureDimension::D2;
    d.mip_level_count = 1;
    image.texture_view_descriptor = None;
    Ok(())
}

/// `settings` as the editor's GPU can preview them, plus what was changed for that (shown next to the preview):
/// `ClampToBorder` needs the device feature `ADDRESS_MODE_CLAMP_TO_BORDER` and is previewed as `ClampToEdge` without
/// it. Settings no GPU can create (E013) are not substituted: [`GameImageCache::get`] refuses them.
pub fn previewable(settings: &TextureSettings, clamp_to_border: bool) -> (TextureSettings, Vec<&'static str>) {
    let mut out = settings.clone();
    let mut notes = Vec::new();
    if let Some(s) = &mut out.sampler {
        let modes = [&mut s.address_mode_u, &mut s.address_mode_v, &mut s.address_mode_w];
        if !clamp_to_border && modes.iter().any(|m| **m == AddressMode::ClampToBorder) {
            for m in modes.into_iter().filter(|m| **m == AddressMode::ClampToBorder) {
                *m = AddressMode::ClampToEdge;
            }
            notes.push(
                "Clamp to border needs the GPU feature ADDRESS_MODE_CLAMP_TO_BORDER in the game; this GPU lacks it, so \
                 the preview uses Clamp to edge.",
            );
        }
    }
    (out, notes)
}

/// Bevy's sampler for `settings` (`None` → the app's default sampler).
pub fn sampler(settings: &TextureSettings) -> ImageSampler {
    let Some(s) = &settings.sampler else { return ImageSampler::Default };
    let address = |m: AddressMode| match m {
        AddressMode::ClampToEdge => ImageAddressMode::ClampToEdge,
        AddressMode::Repeat => ImageAddressMode::Repeat,
        AddressMode::MirrorRepeat => ImageAddressMode::MirrorRepeat,
        AddressMode::ClampToBorder => ImageAddressMode::ClampToBorder,
    };
    let filter = |f: FilterMode| match f {
        FilterMode::Nearest => ImageFilterMode::Nearest,
        FilterMode::Linear => ImageFilterMode::Linear,
    };
    ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: address(s.address_mode_u),
        address_mode_v: address(s.address_mode_v),
        address_mode_w: address(s.address_mode_w),
        mag_filter: filter(s.mag_filter),
        min_filter: filter(s.min_filter),
        mipmap_filter: filter(s.mipmap_filter),
        lod_min_clamp: s.lod_min_clamp,
        lod_max_clamp: s.lod_max_clamp,
        anisotropy_clamp: s.anisotropy_clamp,
        ..default()
    })
}

/// Copies channel `c` (0 / 1 / 2 = R / G / B) of every pixel into R, G and B and makes it opaque; `false` (image
/// unchanged) unless the image is 8-bit RGBA / BGRA with its pixels at hand. Every mip level is rewritten.
pub fn grey_channel(image: &mut Image, c: usize) -> bool {
    use bevy::render::render_resource::TextureFormat as F;
    let c = match image.texture_descriptor.format {
        F::Rgba8Unorm | F::Rgba8UnormSrgb if c < 3 => c,
        F::Bgra8Unorm | F::Bgra8UnormSrgb if c < 3 => 2 - c,
        _ => return false,
    };
    let Some(data) = image.data.as_mut() else { return false };
    for px in data.as_chunks_mut::<4>().0 {
        let v = px[c];
        *px = [v, v, v, 255];
    }
    true
}

/// `WxH | format | mips` (`256x256 | Rgba8UnormSrgb | 9 mips`); ASCII only, the UI font has no `×` / `·`.
pub fn describe(image: &Image) -> String {
    let d = &image.texture_descriptor;
    let mips = d.mip_level_count;
    let layers = match d.size.depth_or_array_layers {
        1 => String::new(),
        n => format!(" | {n} layers"),
    };
    let s = if mips == 1 { "" } else { "s" };
    format!("{}x{} | {:?} | {mips} mip{s}{layers}", d.size.width, d.size.height, d.format)
}

/// `D`'s [`GameImageCache`]: following the render device, freeing unused images.
pub struct GameImagePlugin<D>(PhantomData<fn() -> D>);

impl<D> Default for GameImagePlugin<D> {
    fn default() -> Self {
        GameImagePlugin(PhantomData)
    }
}

impl<D: TextureDomain> Plugin for GameImagePlugin<D> {
    fn build(&self, app: &mut App) {
        app.init_resource::<GameImageCache<D>>().add_systems(Last, (sync_formats::<D>, free_unused::<D>));
    }
}

/// Takes the render device's compressed formats and sampler features once the render plugin has set them up.
fn sync_formats<D: TextureDomain>(
    support: Option<Res<CompressedImageFormatSupport>>,
    device: Option<Res<RenderDevice>>,
    mut cache: ResMut<GameImageCache<D>>,
) {
    if let Some(s) = support
        && cache.formats != s.0
    {
        cache.formats = s.0;
        // Decoded against the old filter.
        cache.entries.clear();
    }
    let border = device.is_some_and(|d| d.features().contains(WgpuFeatures::ADDRESS_MODE_CLAMP_TO_BORDER));
    if cache.clamp_to_border != border {
        cache.clamp_to_border = border;
        cache.entries.clear();
    }
}

fn free_unused<D: TextureDomain>(mut cache: ResMut<GameImageCache<D>>) {
    cache.free_unused();
}

#[cfg(test)]
mod tests {
    use bevy::render::render_resource::TextureFormat;
    use editor_core::tab::ViewKindId;
    use project_core::DomainId;

    use super::*;
    use crate::editor::doc::tests::TestTexture;
    use crate::{InternalTexture, SamplerSettings, TexturePolicy};

    /// [`TestTexture`] with one built-in texture: the sample game's 64 px sprite, filtered linearly.
    struct WithInternal;

    static INTERNAL: [InternalTexture; 1] = [InternalTexture {
        id: "@internal/sprite",
        file: "sprite.png",
        bytes: include_bytes!("../../../../examples/sample_game/assets/textures/sprite.png"),
        settings: linear,
    }];

    fn linear() -> TextureSettings {
        TextureSettings {
            sampler: Some(SamplerSettings { mag_filter: FilterMode::Linear, ..SamplerSettings::default() }),
            ..TextureSettings::default()
        }
    }

    impl TextureDomain for WithInternal {
        const DOMAIN: DomainId = DomainId::new("test");
        const VIEW: ViewKindId = ViewKindId("test.texture");
        const IMAGE_VIEW: ViewKindId = ViewKindId("test.image");
        const POLICY: TexturePolicy = TexturePolicy { internal: Some(|| &INTERNAL), ..TestTexture::POLICY };

        fn disk_owners(_editor: &editor_core::state::editor::Editor, _file: &str) -> Vec<project_core::AssetRef> {
            Vec::new()
        }
    }

    type Cache = GameImageCache<TestTexture>;

    const SAMPLE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/sample_game/assets/textures");

    fn project() -> (tempfile::TempDir, Project) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"g\"\n").unwrap();
        let textures = dir.path().join("assets/textures");
        std::fs::create_dir_all(&textures).unwrap();
        for f in ["rock_albedo.ktx2", "rock_albedo.png"] {
            std::fs::copy(Path::new(SAMPLE).join(f), textures.join(f)).unwrap();
        }
        let p = Project::open(dir.path()).unwrap();
        (dir, p)
    }

    #[test]
    fn decodes_png_and_ktx2_from_the_game_dir() {
        let (_dir, p) = project();
        let mut images = Assets::<Image>::default();
        let mut cache = Cache::default();
        let settings = TextureSettings::default();
        for f in ["textures/rock_albedo.png", "textures/rock_albedo.ktx2"] {
            let h = cache.get(&p, f, &settings, &mut images).unwrap_or_else(|e| panic!("{f}: {e}"));
            let image = images.get(&h).unwrap();
            assert!(image.width() > 0 && image.height() > 0, "{f}");
            assert_eq!(image.texture_descriptor.format, TextureFormat::Rgba8UnormSrgb, "{f}");
            assert!(describe(image).starts_with(&format!("{}x{} | Rgba8UnormSrgb | ", image.width(), image.height())));
            // Cached: the same handle.
            assert_eq!(cache.get(&p, f, &settings, &mut images), Ok(h));
        }
        // Linear data and a sampler of its own: a separate image.
        let linear = TextureSettings {
            is_srgb: false,
            sampler: Some(SamplerSettings { mag_filter: FilterMode::Linear, ..default() }),
            ..default()
        };
        let h = cache.get(&p, "textures/rock_albedo.png", &linear, &mut images).unwrap();
        let image = images.get(&h).unwrap();
        assert_eq!(image.texture_descriptor.format, TextureFormat::Rgba8Unorm);
        assert!(matches!(&image.sampler, ImageSampler::Descriptor(d) if d.mag_filter == ImageFilterMode::Linear));
        assert_eq!(cache.len(), 3);
    }

    #[test]
    fn errors_are_text_and_changed_files_reload() {
        let (dir, p) = project();
        let mut images = Assets::<Image>::default();
        let mut cache = Cache::default();
        let settings = TextureSettings::default();
        let file = dir.path().join("assets/textures/x.png");

        let missing = cache.get(&p, "textures/x.png", &settings, &mut images).unwrap_err();
        assert!(missing.contains("assets/textures/x.png does not exist"), "{missing}");
        std::fs::write(&file, b"not a png").unwrap();
        let corrupt = cache.get(&p, "textures/x.png", &settings, &mut images).unwrap_err();
        assert!(corrupt.starts_with("cannot decode assets/textures/x.png: "), "{corrupt}");

        // The file is replaced by a real image: decoded again.
        std::fs::copy(Path::new(SAMPLE).join("rock_albedo.png"), &file).unwrap();
        let first = cache.get(&p, "textures/x.png", &settings, &mut images).unwrap();
        std::fs::copy(Path::new(SAMPLE).join("rock_albedo.ktx2"), &file).unwrap();
        let second = cache.get(&p, "textures/x.png", &settings, &mut images);
        assert!(second.is_err(), "KTX2 bytes behind a .png name: the extension decides, as in Bevy");
        std::fs::copy(Path::new(SAMPLE).join("rock_albedo.png"), &file).unwrap();
        let third = cache.get(&p, "textures/x.png", &settings, &mut images).unwrap();
        assert_ne!(first, third);
        assert_eq!(cache.len(), 1, "one entry per (path, settings)");
    }

    /// Only relative `.ktx2` / `.png` paths inside `assets/` are read (E012 rules); nothing is cached for the others.
    #[test]
    fn paths_outside_assets_are_refused() {
        let (dir, p) = project();
        let mut images = Assets::<Image>::default();
        let mut cache = Cache::default();
        let settings = TextureSettings::default();
        // A real image right next to `assets/`, reachable by `..` or its absolute path.
        let outside = dir.path().join("outside.png");
        std::fs::copy(Path::new(SAMPLE).join("rock_albedo.png"), &outside).unwrap();
        std::fs::copy(Path::new(SAMPLE).join("rock_albedo.png"), dir.path().join("assets/textures/x.jpg")).unwrap();
        let absolute = outside.to_string_lossy().replace('\\', "/");
        for (path, why) in [
            ("../outside.png", "must not contain empty, `.` or `..` components"),
            ("textures/../../outside.png", "must not contain empty, `.` or `..` components"),
            (absolute.as_str(), "must be relative to the game's assets/ directory"),
            ("textures/x.jpg", "unsupported image format"),
            ("", "empty texture path"),
        ] {
            let err = cache.get(&p, path, &settings, &mut images).unwrap_err();
            assert!(err.contains(why), "{path}: {err}");
        }
        assert!(cache.is_empty());
        assert_eq!(images.len(), 0);
    }

    #[test]
    fn unused_images_are_freed() {
        let (_dir, p) = project();
        let mut images = Assets::<Image>::default();
        let mut cache = Cache::default();
        let settings = TextureSettings::default();
        let kept = cache.get(&p, "textures/rock_albedo.png", &settings, &mut images).unwrap();
        drop(cache.get(&p, "textures/rock_albedo.ktx2", &settings, &mut images).unwrap());
        let _ = cache.get(&p, "textures/missing.png", &settings, &mut images).unwrap_err();
        cache.free_unused();
        assert_eq!(cache.len(), 2, "the held image and the failure stay");
        assert_eq!(cache.get(&p, "textures/rock_albedo.png", &settings, &mut images), Ok(kept.clone()));
        drop(kept);
        cache.free_unused();
        assert_eq!(cache.len(), 1);
    }

    /// Built-in textures decode from the embedded bytes (no file under `assets/` needed), cached per settings; a domain
    /// without them knows no `@internal/…` id.
    #[test]
    fn internal_textures_decode_from_embedded_bytes() {
        let (_dir, p) = project();
        let mut images = Assets::<Image>::default();
        let mut cache = GameImageCache::<WithInternal>::default();
        let settings = (INTERNAL[0].settings)();
        let h = cache.get(&p, "@internal/sprite", &settings, &mut images).unwrap();
        assert_eq!(images.get(&h).unwrap().size(), UVec2::new(64, 64));
        assert!(
            matches!(&images.get(&h).unwrap().sampler, ImageSampler::Descriptor(d) if d.mag_filter == ImageFilterMode::Linear)
        );
        assert_eq!(cache.get(&p, "@internal/sprite", &settings, &mut images), Ok(h));
        let plain = cache.get(&p, "@internal/sprite", &TextureSettings::default(), &mut images).unwrap();
        assert!(matches!(images.get(&plain).unwrap().sampler, ImageSampler::Default));
        let unknown = cache.get(&p, "@internal/nope", &settings, &mut images).unwrap_err();
        assert!(unknown.contains("unknown built-in texture"), "{unknown}");
        assert_eq!(internal_path::<WithInternal>(&INTERNAL[0]), "quillshade/test/internal/sprite.png");
        let none = Cache::default().get(&p, "@internal/sprite", &settings, &mut images).unwrap_err();
        assert!(none.contains("unknown built-in texture"), "{none}");
    }

    /// Clamp to border without the GPU feature is previewed as Clamp to edge (with a note); sampler settings no GPU
    /// can create (E013) decode nothing.
    #[test]
    fn previewable_samplers() {
        let plain = TextureSettings::default();
        assert_eq!(previewable(&plain, false), (plain.clone(), Vec::new()));
        let border = TextureSettings {
            sampler: Some(SamplerSettings { address_mode_u: AddressMode::ClampToBorder, ..default() }),
            ..default()
        };
        let (shown, notes) = previewable(&border, false);
        assert_eq!(shown.sampler.unwrap().address_mode_u, AddressMode::ClampToEdge);
        assert_eq!(notes.len(), 1);
        assert!(notes[0].contains("ADDRESS_MODE_CLAMP_TO_BORDER"));
        assert_eq!(previewable(&border, true), (border.clone(), Vec::new()));

        let (_dir, p) = project();
        let mut images = Assets::<Image>::default();
        let mut cache = Cache::default();
        let aniso =
            TextureSettings { sampler: Some(SamplerSettings { anisotropy_clamp: 8, ..default() }), ..default() };
        assert_eq!(previewable(&aniso, false), (aniso.clone(), Vec::new()), "not substituted");
        let err = cache.get(&p, "textures/rock_albedo.png", &aniso, &mut images).unwrap_err();
        assert!(err.starts_with("E013 invalid sampler settings: "), "{err}");
        assert_eq!(images.len(), 0);
        let linear = SamplerSettings {
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            mipmap_filter: FilterMode::Linear,
            anisotropy_clamp: 16,
            ..default()
        };
        let ok = TextureSettings { sampler: Some(linear), ..default() };
        assert!(cache.get(&p, "textures/rock_albedo.png", &ok, &mut images).is_ok());
    }

    /// Uncompressed RGBA8 KTX2 cube map of `side`-pixel faces, one mip; face `i` is filled with byte `i * 40`.
    fn cube_ktx2(side: u32) -> Vec<u8> {
        let face = (side * side * 4) as usize;
        let data_at = 80 + 24 + 4;
        let mut f = vec![0xAB, b'K', b'T', b'X', b' ', b'2', b'0', 0xBB, b'\r', b'\n', 0x1A, b'\n'];
        // vkFormat R8G8B8A8_UNORM, typeSize, width, height, depth, layers, faces, levels, supercompression.
        for v in [37, 1, side, side, 0, 0, 6, 1, 0] {
            f.extend(u32::to_le_bytes(v));
        }
        // DFD (an empty one: total size 4), no key/value or supercompression data.
        for v in [104, 4, 0, 0] {
            f.extend(u32::to_le_bytes(v));
        }
        f.extend([0u8; 16]);
        for v in [data_at, 6 * face, 6 * face] {
            f.extend(u64::to_le_bytes(v as u64));
        }
        f.extend(u32::to_le_bytes(4));
        for i in 0..6u8 {
            f.extend(std::iter::repeat_n(i * 40, face));
        }
        f
    }

    #[test]
    fn cube_map_shows_as_a_strip() {
        let image = decode(&cube_ktx2(4), "sky.ktx2", &TextureSettings::default(), CompressedImageFormats::NONE);
        let image = image.unwrap();
        assert_eq!((image.width(), image.height()), (4, 24));
        assert_eq!(image.texture_descriptor.size.depth_or_array_layers, 1);
        assert!(image.texture_view_descriptor.is_none(), "a 2D view");
        let data = image.data.as_ref().unwrap();
        assert_eq!(data.len(), 4 * 24 * 4);
        assert_eq!((data[0], data[4 * 4 * 4 * 5]), (0, 200), "faces top to bottom");
        assert_eq!(describe(&image), "4x24 | Rgba8UnormSrgb | 1 mip");
    }

    /// A domain previewing only the first layer (design §7.2) shows a cube map's first face, not the strip.
    #[test]
    fn first_layer_only_shows_the_first_face() {
        let image =
            decode_layers(&cube_ktx2(4), "sky.ktx2", &TextureSettings::default(), CompressedImageFormats::NONE, true);
        let image = image.unwrap();
        assert_eq!((image.width(), image.height()), (4, 4));
        assert!(image.texture_view_descriptor.is_none(), "a 2D view");
        let data = image.data.as_ref().unwrap();
        assert_eq!(data.len(), 4 * 4 * 4);
        assert!(data.iter().all(|b| *b == 0), "face 0");
        // Through the cache of such a domain.
        let (_dir, p) = project();
        std::fs::write(p.assets_dir().join("textures/sky.ktx2"), cube_ktx2(4)).unwrap();
        let mut images = Assets::<Image>::default();
        let settings = TextureSettings::default();
        let first = GameImageCache::<FirstLayer>::default().get(&p, "textures/sky.ktx2", &settings, &mut images);
        assert_eq!(images.get(&first.unwrap()).unwrap().size(), UVec2::new(4, 4));
        let mut cache = Cache::default();
        let cached = cache.get_cached(&p, "textures/sky.ktx2", &settings, None, &mut images).unwrap();
        assert!(cached.is_cubemap);
        assert_eq!(images.get(&cached.handle).unwrap().size(), UVec2::new(4, 24), "all 6 faces stacked in 2D");
        let cube_img = images.get(cached.cube_handle.as_ref().unwrap()).unwrap();
        assert_eq!(cube_img.size(), UVec2::new(4, 4));
        assert_eq!(cube_img.texture_descriptor.array_layer_count(), 6);
        assert_eq!(
            cube_img.texture_view_descriptor.as_ref().and_then(|v| v.dimension),
            Some(bevy::render::render_resource::TextureViewDimension::Cube)
        );
    }

    /// [`TestTexture`] previewing only the first layer.
    struct FirstLayer;

    impl TextureDomain for FirstLayer {
        const DOMAIN: DomainId = DomainId::new("test");
        const VIEW: ViewKindId = ViewKindId("test.texture");
        const IMAGE_VIEW: ViewKindId = ViewKindId("test.image");
        const POLICY: TexturePolicy = TestTexture::POLICY;
        const PREVIEW_FIRST_LAYER: bool = true;

        fn disk_owners(_editor: &editor_core::state::editor::Editor, _file: &str) -> Vec<project_core::AssetRef> {
            Vec::new()
        }
    }

    #[test]
    fn describe_counts_mips() {
        let mut image = Image::default();
        image.texture_descriptor.size.width = 4;
        image.texture_descriptor.size.height = 2;
        image.texture_descriptor.mip_level_count = 3;
        assert_eq!(describe(&image), format!("4x2 | {:?} | 3 mips", image.texture_descriptor.format));
        image.texture_descriptor.mip_level_count = 1;
        assert!(describe(&image).ends_with(" | 1 mip"));
    }

    #[test]
    fn channel_shows_in_grey() {
        let (_dir, p) = project();
        let mut images = Assets::<Image>::default();
        let mut cache = Cache::default();
        let settings = TextureSettings::default();
        let f = "textures/rock_albedo.png";
        let full = cache.get(&p, f, &settings, &mut images).unwrap();
        let (green, grey) = cache.get_channel(&p, f, &settings, Some(1), &mut images).unwrap();
        assert!(grey);
        assert_ne!(full, green, "an entry of its own");
        assert_eq!(cache.get_channel(&p, f, &settings, Some(1), &mut images), Ok((green.clone(), true)), "cached");
        let full = images.get(&full).unwrap().data.clone().unwrap();
        let green = images.get(&green).unwrap().data.clone().unwrap();
        for (a, b) in full.as_chunks::<4>().0.iter().zip(green.as_chunks::<4>().0) {
            assert_eq!(*b, [a[1], a[1], a[1], 255]);
        }
        // A format without per-pixel bytes stays as it is.
        let mut image = Image::default();
        image.texture_descriptor.format = TextureFormat::Bc1RgbaUnorm;
        assert!(!grey_channel(&mut image, 0));
    }
}
