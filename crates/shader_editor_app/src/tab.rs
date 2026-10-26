//! Open tabs (spec §9 "State"). A tab holds one kind of content and is shown by the view of that kind;
//! every view fills the document area (dark background, file type watermark in the bottom-right corner).
//! See decision 0014.

use std::path::PathBuf;

use shader_graph_core::model::AssetKind;
use shader_graph_core::texture::INTERNAL_PREFIX;
use shader_project::regen::{rust_file, wesl_file};
use shader_project::{AssetId, Project, assets};

use crate::consts_doc::ConstsDoc;
use crate::curve_doc::CurveDoc;
use crate::doc::Doc;
use crate::doc_kind::TabDoc;
use crate::game_image::FileStamp;
use crate::texture_doc::TextureDoc;
use crate::ui::icons::IconId;

/// The view that shows a tab's content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewKind {
    /// Node graph of a material or function (bevy_noodle canvas).
    Graph,
    /// Read-only text with syntax highlighting.
    Text,
    /// Curve asset editor (decision 0015).
    Curve,
    /// Global constant table: add form + table (decision 0017).
    Consts,
    /// Read-only game image (design §4.4).
    Image,
    /// Texture asset: settings form + preview (design §4.4).
    Texture,
}

/// Generated file shown in a text tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Wesl,
    Rust,
}

impl Lang {
    /// Suffix of the tab's key in the user config, also the watermark.
    pub fn label(self) -> &'static str {
        match self {
            Lang::Wesl => "wesl",
            Lang::Rust => "rust",
        }
    }

    pub fn from_label(s: &str) -> Option<Lang> {
        [Lang::Wesl, Lang::Rust].into_iter().find(|l| l.label() == s)
    }
}

/// Read-only view of a file generated from an asset ("Show generated WESL/Rust").
#[derive(Debug, Clone)]
pub struct TextDoc {
    pub source: AssetId,
    pub lang: Lang,
    pub path: PathBuf,
    /// Project-relative path (tab title).
    pub rel: String,
    /// File contents; `None` while the file does not exist.
    pub text: Option<String>,
    /// Bumped whenever `text` changes; the view rebuilds when it differs.
    pub revision: u64,
}

impl TextDoc {
    /// `None` when the asset has no such file (functions have no Rust module, constant tables nothing at all).
    pub fn open(project: &Project, source: AssetId, lang: Lang) -> Option<TextDoc> {
        let path = match lang {
            Lang::Wesl => wesl_file(project, &source)?,
            Lang::Rust => rust_file(project, &source)?,
        };
        let mut doc = TextDoc { source, lang, rel: project.rel(&path), path, text: None, revision: 0 };
        doc.reload();
        Some(doc)
    }

    /// Re-reads the file; returns whether the contents changed.
    pub fn reload(&mut self) -> bool {
        let text = std::fs::read_to_string(&self.path).ok();
        if text == self.text && self.revision > 0 {
            return false;
        }
        self.text = text;
        self.revision += 1;
        true
    }
}

impl TabDoc for TextDoc {
    fn asset(&self) -> Option<&AssetId> {
        None
    }

    fn view(&self) -> ViewKind {
        ViewKind::Text
    }

    fn base_title(&self) -> String {
        self.rel.clone()
    }

    fn icon(&self) -> IconId {
        IconId::Code
    }

    fn kind_label(&self) -> &'static str {
        self.lang.label()
    }

    fn is_dirty(&self) -> bool {
        false
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn revision(&self) -> u64 {
        self.revision
    }

    fn save(&mut self, _project: &Project) -> std::io::Result<()> {
        Ok(())
    }

    fn changed_on_disk(&self, _project: &Project) -> bool {
        false
    }

    fn reload_from_disk(&mut self, _project: &Project) -> Result<(), String> {
        self.reload();
        match self.text {
            Some(_) => Ok(()),
            None => Err(format!("{} does not exist", self.rel)),
        }
    }

    fn keep_mine(&mut self, _project: &Project) {}

    /// The generated file moves with its source (regeneration fills it).
    fn rename(&mut self, project: &Project, from: &AssetId, to: &AssetId) {
        if self.source != *from {
            return;
        }
        if let Some(moved) = TextDoc::open(project, to.clone(), self.lang) {
            *self = TextDoc { revision: self.revision + 1, ..moved };
        }
    }

    /// `AssetId::source_rel` of the source plus `#wesl` / `#rust`.
    fn config_key(&self) -> Option<String> {
        Some(format!("{}#{}", self.source.source_rel(), self.lang.label()))
    }
}

/// Prefix of a game image's key in the user config (`@game/textures/rock.png`); `@` never starts an asset's
/// `shaders/…` key.
pub const GAME_KEY_PREFIX: &str = "@game/";

/// A game image (design §4.4) in a read-only tab. Pure data: the view decodes the file through
/// [`crate::game_image::GameImageCache`] whenever [`ImageDoc::revision`] changes.
#[derive(Debug, Clone)]
pub struct ImageDoc {
    /// Path relative to the game's `assets/` dir, `/`-separated.
    pub path: String,
    /// The file's modification time and size when last checked (`None`: missing).
    stamp: Option<FileStamp>,
    /// Bumped whenever the file changes.
    pub revision: u64,
}

impl ImageDoc {
    pub fn open(project: &Project, path: &str) -> ImageDoc {
        let mut doc = ImageDoc { path: path.to_string(), stamp: None, revision: 0 };
        doc.reload(project);
        doc
    }

    /// Re-checks the file; returns whether it changed (the view then decodes it again).
    pub fn reload(&mut self, project: &Project) -> bool {
        let stamp = FileStamp::of(&project.assets_dir().join(&self.path));
        if stamp == self.stamp && self.revision > 0 {
            return false;
        }
        self.stamp = stamp;
        self.revision += 1;
        true
    }
}

impl TabDoc for ImageDoc {
    fn asset(&self) -> Option<&AssetId> {
        None
    }

    fn view(&self) -> ViewKind {
        ViewKind::Image
    }

    fn base_title(&self) -> String {
        format!("assets/{}", self.path)
    }

    fn icon(&self) -> IconId {
        IconId::Image
    }

    fn kind_label(&self) -> &'static str {
        "image"
    }

    fn is_dirty(&self) -> bool {
        false
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn revision(&self) -> u64 {
        self.revision
    }

    fn save(&mut self, _project: &Project) -> std::io::Result<()> {
        Ok(())
    }

    fn changed_on_disk(&self, _project: &Project) -> bool {
        false
    }

    fn reload_from_disk(&mut self, project: &Project) -> Result<(), String> {
        self.reload(project);
        match self.stamp {
            Some(_) => Ok(()),
            None => Err(format!("assets/{} does not exist", self.path)),
        }
    }

    fn keep_mine(&mut self, _project: &Project) {}

    /// Game files are not assets: nothing to follow.
    fn rename(&mut self, _project: &Project, _from: &AssetId, _to: &AssetId) {}

    fn config_key(&self) -> Option<String> {
        Some(format!("{GAME_KEY_PREFIX}{}", self.path))
    }
}

#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)] // a handful of tabs, not worth the indirection
pub enum Tab {
    Graph(Doc),
    Text(TextDoc),
    Curve(CurveDoc),
    Consts(ConstsDoc),
    Image(ImageDoc),
    Texture(TextureDoc),
}

impl Tab {
    /// The content as a [`TabDoc`], whatever its kind.
    pub fn doc(&self) -> &dyn TabDoc {
        match self {
            Tab::Graph(d) => d,
            Tab::Text(t) => t,
            Tab::Curve(c) => c,
            Tab::Consts(c) => c,
            Tab::Image(i) => i,
            Tab::Texture(t) => t,
        }
    }

    pub fn doc_mut(&mut self) -> &mut dyn TabDoc {
        match self {
            Tab::Graph(d) => d,
            Tab::Text(t) => t,
            Tab::Curve(c) => c,
            Tab::Consts(c) => c,
            Tab::Image(i) => i,
            Tab::Texture(t) => t,
        }
    }

    /// The asset the tab belongs to: the one it edits, or the source of a generated file (closed with it on
    /// delete, highlighted in the browser).
    pub fn owner(&self) -> Option<&AssetId> {
        self.doc().asset().or_else(|| self.text().map(|t| &t.source))
    }

    pub fn title(&self) -> String {
        format!("{}{}", self.doc().base_title(), self.suffix())
    }

    /// ` (preview)` on a material with a previewed node (decision 0022).
    fn suffix(&self) -> &'static str {
        match self {
            Tab::Graph(d) if matches!(d.asset.kind, AssetKind::Material(_)) && d.asset.preview.node.is_some() => {
                " (preview)"
            }
            _ => "",
        }
    }

    /// File name without directories and extensions (`rock_wet`); the tab bar falls back to [`Tab::title`] when
    /// several tabs share one.
    pub fn short_title(&self) -> String {
        format!("{}{}", self.short_name(), self.suffix())
    }

    /// [`Tab::short_title`] without the preview suffix: what tab labels compare to find shared names.
    fn short_name(&self) -> String {
        let title = self.doc().base_title();
        let name = title.rsplit('/').next().unwrap_or(&title);
        name.split('.').next().unwrap_or(name).to_string()
    }

    pub fn graph(&self) -> Option<&Doc> {
        match self {
            Tab::Graph(d) => Some(d),
            _ => None,
        }
    }

    pub fn graph_mut(&mut self) -> Option<&mut Doc> {
        match self {
            Tab::Graph(d) => Some(d),
            _ => None,
        }
    }

    pub fn text(&self) -> Option<&TextDoc> {
        match self {
            Tab::Text(t) => Some(t),
            _ => None,
        }
    }

    pub fn curve(&self) -> Option<&CurveDoc> {
        match self {
            Tab::Curve(c) => Some(c),
            _ => None,
        }
    }

    pub fn curve_mut(&mut self) -> Option<&mut CurveDoc> {
        match self {
            Tab::Curve(c) => Some(c),
            _ => None,
        }
    }

    pub fn consts(&self) -> Option<&ConstsDoc> {
        match self {
            Tab::Consts(c) => Some(c),
            _ => None,
        }
    }

    pub fn consts_mut(&mut self) -> Option<&mut ConstsDoc> {
        match self {
            Tab::Consts(c) => Some(c),
            _ => None,
        }
    }

    pub fn image(&self) -> Option<&ImageDoc> {
        match self {
            Tab::Image(i) => Some(i),
            _ => None,
        }
    }

    pub fn texture(&self) -> Option<&TextureDoc> {
        match self {
            Tab::Texture(t) => Some(t),
            _ => None,
        }
    }

    pub fn texture_mut(&mut self) -> Option<&mut TextureDoc> {
        match self {
            Tab::Texture(t) => Some(t),
            _ => None,
        }
    }
}

/// What a tab's [`TabDoc::config_key`] names.
#[derive(Debug, Clone, PartialEq)]
pub enum TabKey {
    /// An asset's own document (`shaders/env/rock.material.ron`).
    Asset(AssetId),
    /// A file generated from the asset (`…ron#wesl` / `#rust`).
    Generated(AssetId, Lang),
    /// A game image (`@game/<path under assets/>`).
    GameImage(String),
    /// A built-in texture's read-only Texture tab (`@internal/default`, design §2).
    Internal(String),
}

/// Parses [`TabDoc::config_key`].
pub fn parse_config_key(key: &str) -> Option<TabKey> {
    if let Some(path) = key.strip_prefix(GAME_KEY_PREFIX) {
        return (!path.is_empty()).then(|| TabKey::GameImage(path.to_string()));
    }
    if key.starts_with(INTERNAL_PREFIX) {
        return Some(TabKey::Internal(key.to_string()));
    }
    match key.split_once('#') {
        Some((rel, lang)) => Some(TabKey::Generated(assets::id_from_source_rel(rel)?, Lang::from_label(lang)?)),
        None => Some(TabKey::Asset(assets::id_from_source_rel(key)?)),
    }
}

/// Tab bar labels: [`Tab::short_title`], or the full [`Tab::title`] for every tab whose short name is shared
/// (the ` (preview)` suffix does not tell two `rock` tabs apart).
pub fn labels(tabs: &[Tab]) -> Vec<String> {
    let short: Vec<String> = tabs.iter().map(Tab::short_name).collect();
    tabs.iter()
        .zip(&short)
        .map(|(t, s)| if short.iter().filter(|o| *o == s).count() > 1 { t.title() } else { t.short_title() })
        .collect()
}
