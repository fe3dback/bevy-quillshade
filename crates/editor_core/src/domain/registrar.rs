//! What a domain plugs into the editor (design §6.1): the [`EditorDomain`] trait and the [`DomainRegistrar`] its
//! `register` fills, once at startup before the Bevy app runs.

use std::io;

use crate::action::Handled;
use crate::dynval::Payload;
use crate::editors::{EditorAreas, KindDef};
use crate::menu::{MainMenu, MenuEntry};
use crate::preview::PreviewCallbacks;
use crate::tab::Tab;
use crate::ui::icons::IconRef;
use bevy::prelude::{App, Scene};
use project_core::regen::DomainBackend;
use project_core::{AssetRef, DomainId, Project};

use super::{DomainState, ToolDef};
use crate::state::editor::Editor;

/// A domain of the editor (design §6.1): Material, Light, …. `src/main.rs` lists them once for the GUI and the CLI.
pub trait EditorDomain: Send + Sync + 'static {
    fn id(&self) -> DomainId;
    fn info(&self) -> DomainInfo;
    /// Its generator (`None` for a domain without output); the CLI runs it, the editor reads its crate contribution.
    fn backend(&self) -> Option<Box<dyn DomainBackend>>;
    /// Registers everything the editor needs of the domain.
    fn register(&self, r: &mut DomainRegistrar);
}

/// How the editor shows a domain.
#[derive(Clone, Copy, Debug)]
pub struct DomainInfo {
    pub title: &'static str,
    pub icon: IconRef,
    /// One line on the welcome screen: what the domain makes and where its files go (Material: "Material — node-based
    /// materials in quillshade/material/, …").
    pub summary: &'static str,
}

/// A "New …" item of an asset kind: the Assets menu and the Project tree's folder menus list every template.
#[derive(Clone, Copy, Debug)]
pub struct AssetTemplate {
    /// `Material (Pbr3d)`: the item reads `New Material (Pbr3d)...`, the dialog `New Material (Pbr3d)`.
    pub label: &'static str,
    /// The name the path dialog proposes (`new_material`).
    pub base_name: &'static str,
    /// The item's icon (a kind with several templates gives each its own, Material's Pbr3d / Unlit3d / Sprite2d).
    pub icon: IconRef,
    /// Writes the new asset's file; an error (bad name, file exists…) is shown as is.
    pub create: fn(&Project, &AssetRef) -> io::Result<()>,
}

/// A kind of asset file of a domain (design §6.1): `quillshade/<domain>/**/*.<suffix>.ron`.
#[derive(Clone, Debug)]
pub struct AssetKindDef {
    pub suffix: &'static str,
    pub title: &'static str,
    pub icon: IconRef,
    pub templates: Vec<AssetTemplate>,
    /// The tab of an asset of this kind (`None`: nothing opens; the function reports why, or activates a tab it
    /// found itself). Core first activates an open tab whose document is the asset.
    pub open: fn(&mut Editor, &AssetRef) -> Option<Tab>,
    /// Items the Project tree's row menu adds after Open / Rename / Move / Delete.
    pub row_menu: fn(&Editor, &AssetRef) -> Vec<MenuEntry>,
}

/// The game files a domain shows in the Game Assets tree (design §8.1).
#[derive(Clone, Copy, Debug)]
pub struct GameAssetsDef {
    /// File extensions, without the dot, compared ignoring case.
    pub extensions: &'static [&'static str],
    /// A double click on such a file (path relative to `assets/`).
    pub open: fn(&mut Editor, &str),
    /// Its row menu.
    pub menu: fn(&Editor, &str) -> Vec<MenuEntry>,
}

impl GameAssetsDef {
    /// Whether `path` is one of this domain's game files: a file with a non-empty stem and one of the extensions.
    pub fn accepts(&self, path: &str) -> bool {
        let file = path.rsplit('/').next().unwrap_or(path);
        file.rsplit_once('.')
            .is_some_and(|(stem, ext)| !stem.is_empty() && self.extensions.iter().any(|e| e.eq_ignore_ascii_case(ext)))
    }
}

/// An asset row of a domain's Project tree ([`DomainState::tree_items`]).
#[derive(Clone, Debug, PartialEq)]
pub struct TreeAsset {
    pub asset: AssetRef,
    pub icon: IconRef,
    /// Drawn in the error colour.
    pub error: bool,
}

/// A virtual read-only folder at the end of a domain's Project tree (`tree_extras`; Material: Internal).
#[derive(Clone, Debug, PartialEq)]
pub struct TreeFolder {
    /// Its key in the tree (collapse state); starts with `@`, which no real folder name has.
    pub key: &'static str,
    pub label: &'static str,
    /// Its rows, opened as assets on a double click; never renamed, moved or deleted.
    pub items: Vec<TreeAsset>,
}

/// A text highlighter: the styled pieces of a text (filled by the text viewer, design §6.2).
pub type Highlighter = fn(&str) -> Vec<(crate::highlight::Style, &str)>;

/// A handler of a domain's payloads besides its state ([`DomainRegistrar::steps`]): [`Handled::No`] for a payload
/// it does not know.
pub type StepFn = fn(&mut Editor, &Payload) -> Handled;

/// A domain's parser of tab keys ([`DomainRegistrar::parse_tab`]).
pub type ParseTabFn = fn(&mut Editor, &str) -> Option<Tab>;

/// Whether what a tab key names still exists ([`DomainRegistrar::tab_exists`]).
pub type TabExistsFn = fn(&Editor, &str) -> bool;

/// A scene a domain adds to the window (its views in the editor content, design §8.1).
pub type SceneFn = fn() -> Box<dyn Scene>;

/// A domain's steps of Help → Quick Start (design §8.2), after core's own ([`crate::quick_start`]): its steps as
/// `(title, optional)` (the dialog prefixes the domain's name), and the lines of step `i` below its heading (`i` is a
/// valid step).
#[derive(Clone, Copy, Debug)]
pub struct QuickStartDef {
    pub steps: &'static [(&'static str, bool)],
    pub body: fn(usize) -> Vec<Box<dyn Scene>>,
}

/// Everything one domain registered.
#[derive(Clone)]
pub struct DomainDef {
    pub id: DomainId,
    pub info: DomainInfo,
    pub state: Option<fn() -> Box<dyn DomainState>>,
    pub asset_kinds: Vec<AssetKindDef>,
    pub editor_kinds: Vec<KindDef>,
    pub game_assets: Option<GameAssetsDef>,
    pub tree_extras: Vec<fn(&Editor) -> Vec<TreeFolder>>,
    pub menu_items: Vec<(MainMenu, fn(&Editor) -> Vec<MenuEntry>)>,
    pub parse_tabs: Vec<ParseTabFn>,
    pub steps: Vec<StepFn>,
    pub tab_exists: Vec<TabExistsFn>,
    pub legacy_tab_keys: bool,
    pub highlighters: Vec<(&'static str, Highlighter)>,
    pub input_contexts: Vec<fn(&mut App)>,
    pub logic: Vec<fn(&mut App)>,
    pub bevy: Vec<fn(&mut App)>,
    pub doc_views: Vec<SceneFn>,
    pub doc_overlays: Vec<SceneFn>,
    pub quick_start: Option<QuickStartDef>,
    pub tools: Vec<ToolDef>,
    pub previews: Vec<PreviewCallbacks>,
}

/// Collects one domain's registrations (design §6.1); [`super::DomainList`] keeps them per domain.
pub struct DomainRegistrar {
    def: DomainDef,
}

impl DomainRegistrar {
    pub(super) fn new(id: DomainId, info: DomainInfo) -> DomainRegistrar {
        DomainRegistrar {
            def: DomainDef {
                id,
                info,
                state: None,
                asset_kinds: Vec::new(),
                editor_kinds: Vec::new(),
                game_assets: None,
                tree_extras: Vec::new(),
                menu_items: Vec::new(),
                parse_tabs: Vec::new(),
                steps: Vec::new(),
                tab_exists: Vec::new(),
                legacy_tab_keys: false,
                highlighters: Vec::new(),
                input_contexts: Vec::new(),
                logic: Vec::new(),
                bevy: Vec::new(),
                doc_views: Vec::new(),
                doc_overlays: Vec::new(),
                quick_start: None,
                tools: Vec::new(),
                previews: Vec::new(),
            },
        }
    }

    pub(super) fn finish(self) -> DomainDef {
        self.def
    }

    /// The domain's empty state, created with the editor; loading a project goes through its `rescan` hook.
    pub fn state(&mut self, make: fn() -> Box<dyn DomainState>) -> &mut Self {
        self.def.state = Some(make);
        self
    }

    pub fn asset_kind(&mut self, kind: AssetKindDef) -> &mut Self {
        self.def.asset_kinds.push(kind);
        self
    }

    /// An editor kind (decision 0031): its window areas and, through `EditorAreas::input`, its input context.
    pub fn editor_kind<E: EditorAreas + 'static>(&mut self) -> &mut Self {
        self.def.editor_kinds.push(KindDef::of::<E>());
        self
    }

    pub fn game_assets(&mut self, def: GameAssetsDef) -> &mut Self {
        self.def.game_assets = Some(def);
        self
    }

    /// Virtual folders after the domain's assets in its Project tree.
    pub fn tree_extras(&mut self, extras: fn(&Editor) -> Vec<TreeFolder>) -> &mut Self {
        self.def.tree_extras.push(extras);
        self
    }

    /// Items the domain adds to a main menu, before core's own (Assets: after the "New …" templates), built each
    /// time the menu is filled. They show only while the domain is active; the project domain's always (design
    /// §14.2).
    pub fn menu_items(&mut self, menu: MainMenu, items: fn(&Editor) -> Vec<MenuEntry>) -> &mut Self {
        self.def.menu_items.push((menu, items));
        self
    }

    /// Reopens a tab from the key its document gave ([`crate::doc_kind::TabDoc::config_key`]): `None` when the
    /// key is not one it parses or the tab does not open. May be called several times (the domain's own, a kit's):
    /// core tries them in registration order, then its default, which opens a key equal to the source path of a
    /// file of a registered asset kind through [`Editor::open_ref`] (design §14.4). Each handler should parse only
    /// keys no other handler or the default takes, or a key that fails to open is tried (and reported) twice.
    pub fn parse_tab(&mut self, parse: ParseTabFn) -> &mut Self {
        self.def.parse_tabs.push(parse);
        self
    }

    /// A handler of the domain's payloads next to its state (design §6.2): an [`crate::action::Action::Domain`]
    /// command or a step one of its documents deferred ([`crate::doc_kind::DocCtx::defer`]) goes to the handlers in
    /// registration order, and to the state's [`DomainState::apply`] when none handled it. A kit's commands and
    /// deferred steps work this way, with or without a state of the domain; the handler runs with the domain's state
    /// in place (not taken).
    pub fn steps(&mut self, handler: StepFn) -> &mut Self {
        self.def.steps.push(handler);
        self
    }

    /// Whether what a tab key names still exists. May be called several times (the domain's own, a kit's): a key
    /// exists when one of them says so, or when it is the source path of an existing file of a registered asset kind
    /// (core's default, design §14.4). Restoring tabs skips a missing one silently; Back / Forward drops its entry,
    /// while one that exists but does not open is kept (decision 0041).
    pub fn tab_exists(&mut self, exists: TabExistsFn) -> &mut Self {
        self.def.tab_exists.push(exists);
        self
    }

    /// Tab keys saved without a `<domain>:` prefix (user configs written before domains existed, design §6.6) are
    /// this domain's. At most one domain says so (Material).
    pub fn legacy_tab_keys(&mut self) -> &mut Self {
        self.def.legacy_tab_keys = true;
        self
    }

    /// A highlighter of the core text viewer for language `lang` (design §6.2).
    pub fn highlighter(&mut self, lang: &'static str, highlight: Highlighter) -> &mut Self {
        self.def.highlighters.push((lang, highlight));
        self
    }

    /// A domain-level input context (design §6.7), registered with the input contexts.
    pub fn input_context(&mut self, add: fn(&mut App)) -> &mut Self {
        self.def.input_contexts.push(add);
        self
    }

    /// Systems the editor logic needs, also without a window (headless tests): added by `EditorCorePlugin`.
    pub fn logic(&mut self, add: fn(&mut App)) -> &mut Self {
        self.def.logic.push(add);
        self
    }

    /// Anything else the window app needs: systems, resources, plugins (added by `UiPlugin`).
    pub fn bevy(&mut self, add: fn(&mut App)) -> &mut Self {
        self.def.bevy.push(add);
        self
    }

    /// A view in the editor content (a kind's main view, tagged `ViewOf(kind)`), spawned once with the window layout
    /// under the empty hint and the watermark.
    pub fn doc_view(&mut self, view: SceneFn) -> &mut Self {
        self.def.doc_views.push(view);
        self
    }

    /// A scene over the editor content (above the empty hint and the watermark), spawned once with the window
    /// layout.
    pub fn doc_overlay(&mut self, overlay: SceneFn) -> &mut Self {
        self.def.doc_overlays.push(overlay);
        self
    }

    /// The domain's steps of the Help → Quick Start dialog (design §8.2), shown after core's steps and the steps of
    /// the domains registered before it.
    pub fn quick_start(&mut self, def: QuickStartDef) -> &mut Self {
        self.def.quick_start = Some(def);
        self
    }

    /// A tool of the domain (design §14.1): a singleton tab without an asset, opened by
    /// [`crate::action::Action::OpenTool`] and, with [`ToolDef::menu`], from a main menu. Panics on two tools with one
    /// id, a programming error of the domain.
    pub fn tool(&mut self, def: ToolDef) -> &mut Self {
        let (id, domain) = (def.id, self.def.id);
        assert!(self.def.tools.iter().all(|t| t.id != id), "tool `{id}` registered twice by `{domain}`");
        self.def.tools.push(def);
        self
    }

    /// Registers start and cleanup callbacks for a preview kind ([`crate::preview::PreviewCallbacks`]). Panics if the
    /// same [`crate::preview::PreviewKindId`] is registered twice by the domain.
    pub fn preview(&mut self, callbacks: PreviewCallbacks) -> &mut Self {
        let (kind, domain) = (callbacks.kind, self.def.id);
        assert!(self.def.previews.iter().all(|p| p.kind != kind), "preview `{kind}` registered twice by `{domain}`");
        self.def.previews.push(callbacks);
        self
    }
}
