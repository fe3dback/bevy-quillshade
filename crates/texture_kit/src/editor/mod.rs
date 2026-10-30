//! The texture editor (design §7.2, decisions 0024, 0028), shared by every domain with Texture assets: the open
//! Texture asset ([`TextureDoc`], with its undo history), its edits ([`TextureEdit`], [`TextureProp`]), the
//! settings form with Easy / Advanced presets, the editor topbar and right sidebar ([`TextureEditor`]); the read-only
//! image tab of a game file ([`ImageDoc`], [`ImageEditor`]); the image preview both share ([`PreviewDoc`],
//! [`ImagePreviewPlugin`]); the decoded game images ([`GameImageCache`]); "Create Texture Asset" for a game file
//! ([`CreateTextureForFile`]) and the virtual Internal folder of a domain's built-in textures.
//!
//! Composition, not copies: a domain implements [`TextureDomain`] on a zero-sized type and calls [`register`]. Every
//! type, system and observer the editor adds is generic over that type, so two domains each have their own texture
//! editor side by side; the domain folder decides which one a `*.texture.ron` file is (`quillshade/<domain>/…`), and
//! [`TextureDomain::POLICY`] what it accepts and offers.
//!
//! Depends on `editor_core` (and Bevy) only: never on a domain or a `shader_*` crate.

#![allow(clippy::type_complexity)] // Bevy system signatures

use std::io;
use std::marker::PhantomData;

use bevy::prelude::{App, Plugin, Scene};
use editor_core::domain::{AssetKindDef, AssetTemplate, DomainRegistrar, GameAssetsDef, TreeAsset, TreeFolder};
use editor_core::dynval::Payload;
use editor_core::menu::MenuEntry;
use editor_core::message::Level;
use editor_core::state::editor::Editor;
use editor_core::tab::{Tab, ViewKindId};
use editor_core::ui::icons::IconRef;
use project_core::{AssetRef, Diagnostic, DomainId, Project};

use crate::{INTERNAL_PREFIX, InternalTexture, TextureAsset, TexturePolicy, new_texture_path_for_file};

/// A marker component of `D`'s texture editor.
macro_rules! marker {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[derive(::bevy::prelude::Component)]
        pub struct $name<D>(::std::marker::PhantomData<fn() -> D>);

        impl<D> Default for $name<D> {
            fn default() -> Self {
                $name(::std::marker::PhantomData)
            }
        }

        impl<D> Clone for $name<D> {
            fn clone(&self) -> Self {
                $name::default()
            }
        }
    };
}

mod cache;
pub(crate) mod doc;
mod form;
mod image_tab;
mod ops;
mod preview;

pub use cache::{
    Channel, FileStamp, GameImageCache, GameImagePlugin, decode, decode_layers, describe, grey_channel, previewable,
    sampler, stack_layers,
};
pub use doc::{
    FilterPreset, MAX_ANISOTROPY, TextureDoc, TextureField, address_label, effective_sampler, filter_label, fits_easy,
};
pub use form::{
    TextureEditor, TexturePathField, TextureSidebarBody, TextureViewBar, TextureViewForm, TextureViewPlugin,
    texture_view,
};
pub use image_tab::{
    GAME_KEY_PREFIX, ImageDoc, ImageEditor, ImageSidebarBody, ImageViewBar, ImageViewPlugin, image_view,
};
pub use ops::{
    CreateTextureForFile, PreviewChannel, TextureAction, TextureEdit, TextureProp, path_problem, texture_key,
};
pub use preview::{
    ImagePreviewPlugin, PREVIEW_HOTKEYS, PreviewArea, PreviewDoc, PreviewInfoText, PreviewPic, PreviewZoomText,
    preview_area, preview_controls,
};

/// The icon of Texture assets (secondbar, Project tree, the Internal folder's rows); game images keep the image icon.
pub const TEXTURE_ICON: IconRef = IconRef::new(&icon_rs::COPPER_ZONE);

/// Key of the Project tree's virtual read-only `Internal` folder of built-in textures (design §2): the parent of their
/// `@internal/…` ids. `@` is not allowed in asset or folder names, so no real folder has this path.
pub const INTERNAL_FOLDER: &str = "@internal";

/// A domain with Texture assets (design §7.2): one zero-sized type per domain (Material: `MaterialTexture`). Its files
/// are `quillshade/<DOMAIN>/**/*.<SUFFIX>.ron`; its Texture tabs show in the view [`TextureDomain::VIEW`], its game
/// image tabs in [`TextureDomain::IMAGE_VIEW`]; [`TextureDomain::POLICY`] says which images it takes, its built-in
/// textures and whether the Texture tab has Easy mode.
pub trait TextureDomain: Sized + Send + Sync + 'static {
    const DOMAIN: DomainId;
    const SUFFIX: &'static str = "texture";
    /// The Texture editor kind's view (`ViewKindId("material.texture")`): distinct per domain.
    const VIEW: ViewKindId;
    /// The image editor kind's view (`ViewKindId("material.image")`): distinct per domain.
    const IMAGE_VIEW: ViewKindId;
    const POLICY: TexturePolicy;
    /// A "New Texture..." template (Assets menu, folder menus) making a Texture asset without an image, which the
    /// form then points at a game file (Light, design §9.2). Off by default: Material makes Texture assets from game
    /// images only ("Create Texture Asset").
    const NEW_TEMPLATE: bool = false;
    /// The preview (Texture and image tabs) shows only the first face / layer / slice of a cube map, an array or a
    /// 3D image (Light's cube maps, design §7.2). Off by default: every layer, stacked top to bottom
    /// ([`stack_layers`]).
    const PREVIEW_FIRST_LAYER: bool = false;
    /// How the settings reach the game, under a Texture tab's description in the right sidebar. By default
    /// Material's: the baked manifest.
    const SETTINGS_NOTE: &'static str = "Settings reach the game through the baked manifest (hot-reloaded); sRGB and \
                                         grayscale expansion need a rebuild.";

    /// Scenes after the editor topbar's own tools, rebuilt with the bar.
    fn topbar_extras(_doc: &TextureDoc<Self>) -> Vec<Box<dyn Scene>> {
        Vec::new()
    }

    /// Items the Project tree's row menu of a Texture asset adds after Open / Rename / Move / Delete (Material: its
    /// source).
    fn row_menu(_editor: &Editor, _r: &AssetRef) -> Vec<MenuEntry> {
        Vec::new()
    }

    /// The domain's Texture assets on disk that point at the game file `file` (path under `assets/`), in path order:
    /// one file — one asset (design §1.1). By default the files are read now; a domain keeping its assets in its state
    /// answers from there (Material: its last scan).
    fn disk_owners(editor: &Editor, file: &str) -> Vec<AssetRef> {
        let Some(project) = &editor.project else { return Vec::new() };
        let dir = project.domain_dir(Self::DOMAIN);
        let mut found: Vec<AssetRef> = project_core::files::walk_files(&dir)
            .into_iter()
            .filter_map(|f| project_core::asset_ref::parse(&project.rel(&f), Self::DOMAIN, &[Self::SUFFIX]))
            .filter(|r| load(project, r).is_ok_and(|t| t.path == file))
            .collect();
        found.sort_by(|a, b| a.path.cmp(&b.path));
        found
    }

    /// The problems the right sidebar lists for the Texture asset `r`: by default the domain's diagnostics of it.
    fn problems(editor: &Editor, r: &AssetRef) -> Vec<Diagnostic> {
        let Some(state) = editor.domains.get_dyn(Self::DOMAIN) else { return Vec::new() };
        let rel = r.source_rel();
        state.diagnostics().into_iter().filter(|d| d.asset.as_deref() == Some(rel.as_str())).collect()
    }

    /// "Create Texture Asset" wrote `r` and the domains rescanned: by default the domain follows as after a save
    /// (Material regenerates).
    fn created(editor: &mut Editor, _r: &AssetRef) {
        editor.with_domain(Self::DOMAIN, |e, s| s.after_save(e, &[]));
    }
}

/// `D`'s texture editor in the window: the decoded images, the preview, the Texture and image tabs' systems.
pub struct TextureEditorPlugin<D>(PhantomData<fn() -> D>);

impl<D> Default for TextureEditorPlugin<D> {
    fn default() -> Self {
        TextureEditorPlugin(PhantomData)
    }
}

impl<D: TextureDomain> Plugin for TextureEditorPlugin<D> {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            GameImagePlugin::<D>::default(),
            ImagePreviewPlugin::<D>::default(),
            TextureViewPlugin::<D>::default(),
            ImageViewPlugin::<D>::default(),
        ));
    }
}

/// Adds `D`'s Texture assets to its domain (design §7.2): the asset kind (a "New Texture..." template only with
/// [`TextureDomain::NEW_TEMPLATE`]: Texture assets are made from game files), the image and Texture editor kinds
/// with their views and window systems ([`TextureEditorPlugin`]), the Game Assets images of `D::POLICY.extensions` (open, "Create Texture Asset"),
/// the kit's step handler (path edits, undo / redo, creation), the tab keys of game images (`@game/<path>`) and built-in
/// textures (`@internal/…`) and, with built-in textures, the virtual Internal folder. Core's defaults of an asset
/// kind cover the rest (design §14.4): a Texture tab's key is its source path, the Project tree lists the files,
/// rename / delete move / remove them, unless the domain overrides them.
pub fn register<D: TextureDomain>(r: &mut DomainRegistrar) {
    r.asset_kind(AssetKindDef {
        suffix: D::SUFFIX,
        title: "Texture",
        icon: TEXTURE_ICON,
        templates: if D::NEW_TEMPLATE { vec![new_texture_template()] } else { Vec::new() },
        open: open::<D>,
        row_menu: D::row_menu,
    })
    .editor_kind::<ImageEditor<D>>()
    .editor_kind::<TextureEditor<D>>()
    .game_assets(GameAssetsDef { extensions: D::POLICY.extensions, open: open_image::<D>, menu: game_file_menu::<D> })
    .steps(ops::steps::<D>)
    .parse_tab(parse_key::<D>)
    .tab_exists(key_exists::<D>)
    .bevy(|app| {
        app.add_plugins(TextureEditorPlugin::<D>::default());
    })
    .doc_view(|| Box::new(image_view::<D>()))
    .doc_view(|| Box::new(texture_view::<D>()));
    if D::POLICY.internal.is_some() {
        r.tree_extras(internal_folder::<D>);
    }
}

/// The "New Texture..." template of a domain with [`TextureDomain::NEW_TEMPLATE`]: a Texture asset with no image yet
/// (its empty path is a problem until the form sets one) and default settings.
fn new_texture_template() -> AssetTemplate {
    AssetTemplate { label: "Texture", base_name: "texture", icon: TEXTURE_ICON, create: create_empty }
}

/// Writes the new Texture asset `r` without an image; refused for a bad path or an existing file.
fn create_empty(project: &Project, r: &AssetRef) -> io::Result<()> {
    project_core::ident::check_path(&r.path).map_err(io::Error::other)?;
    if file(project, r).exists() {
        let what = format!("`{}` already exists", r.source_rel());
        return Err(io::Error::new(io::ErrorKind::AlreadyExists, what));
    }
    save(project, r, &TextureAsset::new(""))
}

/// The active tab's Texture asset (or built-in texture) of `D` (`None` while another tab is active).
pub fn active<D: TextureDomain>(editor: &Editor) -> Option<&TextureDoc<D>> {
    editor.active_doc()
}

pub fn active_mut<D: TextureDomain>(editor: &mut Editor) -> Option<&mut TextureDoc<D>> {
    editor.active_doc_mut()
}

/// The tab of `D`'s Texture asset (or built-in texture: `r.path` is its `@internal/…` id).
pub fn index<D: TextureDomain>(editor: &Editor, r: &AssetRef) -> Option<usize> {
    editor.tabs.iter().position(|t| t.get::<TextureDoc<D>>().is_some_and(|d| d.aref == *r))
}

/// The tab of `D`'s game image `path`.
pub fn image_index<D: TextureDomain>(editor: &Editor, path: &str) -> Option<usize> {
    editor.tabs.iter().position(|t| t.get::<ImageDoc<D>>().is_some_and(|i| i.path == path))
}

/// The content as a [`PreviewDoc`], on a tab of `D` with the shared image preview (image, Texture).
pub fn preview<D: TextureDomain>(tab: &Tab) -> Option<&dyn PreviewDoc> {
    match (tab.get::<ImageDoc<D>>(), tab.get::<TextureDoc<D>>()) {
        (Some(i), _) => Some(i),
        (_, Some(t)) => Some(t),
        _ => None,
    }
}

/// The asset kind's `open`: `D`'s Texture asset `r` loaded from disk, or the read-only tab of a built-in texture
/// (`r.path` an `@internal/…` id; activated when already open). `None` with an error dialog when the file does not
/// load.
pub fn open<D: TextureDomain>(e: &mut Editor, r: &AssetRef) -> Option<Tab> {
    if let Some(i) = index::<D>(e, r) {
        e.activate(i);
        return None;
    }
    if let Some(t) = D::POLICY.internal_texture(&r.path) {
        return Some(Tab::new(D::DOMAIN, TextureDoc::<D>::internal(t)));
    }
    let project = e.project.as_ref()?;
    match load(project, r) {
        Ok(t) => Some(Tab::new(D::DOMAIN, TextureDoc::<D>::new(r.clone(), t))),
        Err(err) => {
            e.io_error(&format!("cannot open {}", r.source_rel()), err);
            None
        }
    }
}

/// Opens (or activates) the read-only tab of `D`'s game image `assets/<path>` (design §4.4): the Game Assets open.
pub fn open_image<D: TextureDomain>(e: &mut Editor, path: &str) {
    if let Some(i) = image_index::<D>(e, path) {
        return e.activate(i);
    }
    if let Some(tab) = image_tab::<D>(e, path) {
        e.push_tab(tab);
    }
}

/// The read-only tab of the game image `assets/<path>`; `None` (with a warning) for a path that is no image of `D`'s
/// policy or a missing file.
pub fn image_tab<D: TextureDomain>(e: &mut Editor, path: &str) -> Option<Tab> {
    let project = e.project.as_ref()?;
    if let Err(err) = TextureAsset::check_path(path, &D::POLICY) {
        e.push_msg(Level::Warning, format!("cannot open game image: {err}"));
        return None;
    }
    if !project.assets_dir().join(path).is_file() {
        e.push_msg(Level::Warning, format!("assets/{path} does not exist"));
        return None;
    }
    Some(Tab::new(D::DOMAIN, ImageDoc::<D>::open(project, path)))
}

/// The tab of a key only the kit gives ([`editor_core::doc_kind::TabDoc::config_key`]), registered by [`register`]:
/// a game image's `@game/<path>` or a built-in texture's `@internal/…` id. A Texture asset's key is its source path,
/// which core's default opens (design §14.4); `None` for it and any other key.
fn parse_key<D: TextureDomain>(e: &mut Editor, key: &str) -> Option<Tab> {
    if let Some(path) = key.strip_prefix(GAME_KEY_PREFIX) {
        return image_tab::<D>(e, path);
    }
    if key.starts_with(INTERNAL_PREFIX) {
        return open::<D>(e, &internal_ref::<D>(key));
    }
    None
}

/// Whether what a key of [`parse_key`] names still exists: the game file, the built-in texture (a Texture asset's
/// file is core's default, design §14.4).
fn key_exists<D: TextureDomain>(e: &Editor, key: &str) -> bool {
    if let Some(path) = key.strip_prefix(GAME_KEY_PREFIX) {
        return e.game_files.iter().any(|f| f == path);
    }
    key.starts_with(INTERNAL_PREFIX) && D::POLICY.internal_texture(key).is_some()
}

/// The Texture asset reference of `D`'s built-in texture `id` (`@internal/…`).
pub fn internal_ref<D: TextureDomain>(id: &str) -> AssetRef {
    AssetRef { domain: D::DOMAIN, path: id.to_string(), suffix: D::SUFFIX }
}

/// Where `D`'s regeneration copies a built-in texture, relative to `assets/`
/// (`quillshade/<domain>/internal/default.png`).
pub fn internal_path<D: TextureDomain>(t: &InternalTexture) -> String {
    format!("quillshade/{}/internal/{}", D::DOMAIN, t.file)
}

/// The file of asset `r`.
fn file(project: &Project, r: &AssetRef) -> std::path::PathBuf {
    project.root.join(r.source_rel())
}

/// Loads the Texture asset file of `r` ([`crate::load`]). The image path is not checked here (validation reports it).
pub fn load(project: &Project, r: &AssetRef) -> Result<TextureAsset, String> {
    crate::load(&file(project, r))
}

/// Writes the Texture asset file of `r` ([`crate::save`]), creating `quillshade/` with its settings first.
pub fn save(project: &Project, r: &AssetRef, texture: &TextureAsset) -> io::Result<()> {
    project.ensure_layout()?;
    crate::save(&file(project, r), texture)
}

/// `D`'s Texture asset other than `except` that claims the game file `path`: it points there **on disk or in an open
/// tab** (unsaved). Both count until the tab is saved: saving it, or regenerating from disk, must never produce two
/// assets on one file (E011), so a file an unsaved tab moved away from stays taken until then.
pub fn texture_owner<D: TextureDomain>(e: &Editor, path: &str, except: Option<&AssetRef>) -> Option<AssetRef> {
    let on_disk = D::disk_owners(e, path);
    let open = e.tabs.iter().filter_map(Tab::get::<TextureDoc<D>>).filter(|t| !t.is_internal() && t.asset.path == path);
    let open = open.map(|t| t.aref.clone());
    on_disk.into_iter().chain(open).find(|r| Some(r) != except)
}

/// `D`'s Texture asset claiming the game file `path` (on disk or in an open tab): "Create Texture Asset" is offered
/// only when there is none.
pub fn texture_for_file<D: TextureDomain>(e: &Editor, path: &str) -> Option<AssetRef> {
    texture_owner::<D>(e, path, None)
}

/// Creates `D`'s Texture asset with default settings for the game image `path` (relative to `assets/`) at
/// `quillshade/<domain>/textures/<folders>/<stem>.texture.ron` ([`new_texture_path_for_file`], design §4.3); the
/// domain follows ([`TextureDomain::created`]). Refused (an error dialog) when a Texture asset already points at the
/// file, on disk or in an open tab, or `D`'s policy does not take it. Not undoable: asset creation is not part of any
/// document's history.
pub fn create_for_file<D: TextureDomain>(e: &mut Editor, path: &str) -> Option<AssetRef> {
    try_create_for_file::<D>(e, path).map_err(|err| e.io_error("Cannot create Texture asset", err)).ok()
}

/// [`create_for_file`] without the error dialog: the error text is returned instead (a drop onto a graph reports it
/// itself).
pub fn try_create_for_file<D: TextureDomain>(e: &mut Editor, path: &str) -> Result<AssetRef, String> {
    let project = e.project.clone().ok_or("no project is open")?;
    // One game file, one Texture asset: claimed on disk or by an open tab (unsaved) is refused. The claim is read
    // from the last scan, so rescan first: a Texture asset written outside the editor a moment ago counts. A
    // user action, not a hot path.
    e.rescan();
    if let Some(other) = texture_for_file::<D>(e, path) {
        return Err(format!("`{path}` already has a Texture asset: {}", other.source_rel()));
    }
    let suffixes: Vec<&str> = e.domain_list.asset_kinds(D::DOMAIN).iter().map(|k| k.suffix).collect();
    // A name another asset of the domain has, of any kind, gets a number.
    let taken = |p: &str| {
        suffixes.iter().any(|&suffix| file(&project, &AssetRef { domain: D::DOMAIN, path: p.into(), suffix }).exists())
    };
    let r = AssetRef { domain: D::DOMAIN, path: new_texture_path_for_file(path, taken), suffix: D::SUFFIX };
    create(&project, &r, &TextureAsset::new(path), &D::POLICY)?;
    e.info(format!("Created {} for assets/{path}", r.source_rel()));
    e.rescan();
    // An image tab of the file offers the creation in its right sidebar.
    e.ui.props += 1;
    D::created(e, &r);
    Ok(r)
}

/// Writes the new Texture asset `r` (which must not exist yet) for `texture`, whose image path `policy` must take.
fn create(project: &Project, r: &AssetRef, texture: &TextureAsset, policy: &TexturePolicy) -> Result<(), String> {
    project_core::ident::check_path(&r.path)?;
    if file(project, r).exists() {
        return Err(format!("`{}` already exists", r.source_rel()));
    }
    TextureAsset::check_path(&texture.path, policy)?;
    save(project, r, texture).map_err(|e| e.to_string())
}

/// A game image's row menu: "Create Texture Asset", disabled once a Texture asset of `D` points at the file.
fn game_file_menu<D: TextureDomain>(editor: &Editor, path: &str) -> Vec<MenuEntry> {
    let disabled =
        texture_for_file::<D>(editor, path).map(|other| format!("{} already points at this file", other.source_rel()));
    vec![MenuEntry {
        icon: Some(TEXTURE_ICON),
        label: "Create Texture Asset".into(),
        action: editor_core::action::Action::Domain(D::DOMAIN, Payload::new(CreateTextureForFile(path.to_string()))),
        disabled,
    }]
}

/// The virtual `Internal` folder of `D`'s built-in textures, each opened as a read-only Texture tab (design §2).
fn internal_folder<D: TextureDomain>(_editor: &Editor) -> Vec<TreeFolder> {
    let item = |t: &InternalTexture| TreeAsset { asset: internal_ref::<D>(t.id), icon: TEXTURE_ICON, error: false };
    vec![TreeFolder {
        key: INTERNAL_FOLDER,
        label: "Internal",
        items: D::POLICY.internal_textures().iter().map(item).collect(),
    }]
}
