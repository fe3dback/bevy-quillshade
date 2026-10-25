//! Open tabs (spec §9 "State"). A tab holds one kind of content and is shown by the view of that kind;
//! every view fills the document area (dark background, file type watermark in the bottom-right corner).
//! See decision 0014.

use std::path::PathBuf;

use shader_project::regen::{rust_file, wesl_file};
use shader_project::{AssetId, Project, assets};

use crate::doc::Doc;

/// The view that shows a tab's content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewKind {
    /// Node graph of a material or function (bevy_noodle canvas).
    Graph,
    /// Read-only text with syntax highlighting.
    Text,
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
    /// `None` when the asset has no such file (functions have no Rust module).
    pub fn open(project: &Project, source: AssetId, lang: Lang) -> Option<TextDoc> {
        let path = match lang {
            Lang::Wesl => wesl_file(project, &source),
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

#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)] // a handful of tabs, not worth the indirection
pub enum Tab {
    Graph(Doc),
    Text(TextDoc),
}

impl Tab {
    pub fn view(&self) -> ViewKind {
        match self {
            Tab::Graph(_) => ViewKind::Graph,
            Tab::Text(_) => ViewKind::Text,
        }
    }

    /// The asset the tab belongs to.
    pub fn asset(&self) -> &AssetId {
        match self {
            Tab::Graph(d) => &d.id,
            Tab::Text(t) => &t.source,
        }
    }

    pub fn title(&self) -> String {
        match self {
            Tab::Graph(d) => d.id.source_rel().trim_end_matches(".ron").to_string(),
            Tab::Text(t) => t.rel.clone(),
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.graph().is_some_and(Doc::is_dirty)
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

    /// Key in the user config: `AssetId::source_rel`, plus `#wesl` / `#rust` for generated files.
    pub fn config_key(&self) -> String {
        match self {
            Tab::Graph(d) => d.id.source_rel(),
            Tab::Text(t) => format!("{}#{}", t.source.source_rel(), t.lang.label()),
        }
    }
}

/// Parses [`Tab::config_key`]: the asset and, for a text tab, the generated file.
pub fn parse_config_key(key: &str) -> Option<(AssetId, Option<Lang>)> {
    match key.split_once('#') {
        Some((rel, lang)) => Some((assets::id_from_source_rel(rel)?, Some(Lang::from_label(lang)?))),
        None => Some((assets::id_from_source_rel(key)?, None)),
    }
}
