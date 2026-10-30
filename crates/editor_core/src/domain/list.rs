//! The registered domains (design §6.1): built once from `src/main.rs`'s list before the app runs, shared by the
//! [`Editor`] (an `Arc`) and the Bevy app (a resource).

use std::fmt;

use crate::editors::EditorKinds;
use crate::menu::MainMenu;
use crate::tab::TabKey;
use bevy::prelude::Resource;
use project_core::DomainId;
use project_core::umbrella::CrateContribution;

use super::registrar::{AssetKindDef, DomainDef, DomainRegistrar, EditorDomain, GameAssetsDef};
use super::{DomainState, DomainStates, ProjectDomain, ToolDef};
use crate::state::editor::Editor;

/// Every domain's registrations, in registration order; the project domain (design §14.2) is last.
#[derive(Resource, Clone, Default)]
pub struct DomainList {
    domains: Vec<DomainDef>,
    crates: Vec<CrateContribution>,
}

impl fmt::Debug for DomainList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.domains.iter().map(|d| d.id)).finish()
    }
}

impl DomainList {
    /// Registers each domain, then the built-in [`ProjectDomain`] last (design §14.2). Panics on a domain with the
    /// reserved id [`DomainId::PROJECT`], two domains with one id, or two claiming legacy tab keys: all are
    /// programming errors of the domain list.
    pub fn new(domains: Vec<Box<dyn EditorDomain>>) -> DomainList {
        DomainList::build(domains, &ProjectDomain)
    }

    /// [`DomainList::new`] with `project` in place of the built-in project domain (tests give it tools).
    #[cfg(test)]
    pub(crate) fn with_project_domain(domains: Vec<Box<dyn EditorDomain>>, project: &dyn EditorDomain) -> DomainList {
        DomainList::build(domains, project)
    }

    fn build(domains: Vec<Box<dyn EditorDomain>>, project: &dyn EditorDomain) -> DomainList {
        assert!(project.id() == DomainId::PROJECT, "the project domain must use the id `{}`", DomainId::PROJECT);
        for d in &domains {
            assert!(d.id() != DomainId::PROJECT, "the domain id `{}` is reserved", DomainId::PROJECT);
        }
        let mut list = DomainList::default();
        for d in domains.iter().map(Box::as_ref).chain([project]) {
            let id = d.id();
            assert!(list.get(id).is_none(), "domain `{id}` registered twice");
            let mut r = DomainRegistrar::new(id, d.info());
            d.register(&mut r);
            let def = r.finish();
            assert!(
                !(def.legacy_tab_keys && list.domains.iter().any(|o| o.legacy_tab_keys)),
                "two domains claim the legacy tab keys"
            );
            list.crates.extend(d.backend().map(|b| b.crate_contribution()));
            list.domains.push(def);
        }
        list.kind_defs(); // panics on two different editor kinds with one view id
        list
    }

    /// The registered domains, in registration order.
    pub fn domains(&self) -> &[DomainDef] {
        &self.domains
    }

    /// Every registered id, the project domain's last.
    pub fn ids(&self) -> impl Iterator<Item = DomainId> + '_ {
        self.domains.iter().map(|d| d.id)
    }

    /// Every id but [`DomainId::PROJECT`]: the domains with a folder of their own under `quillshade/`.
    pub fn real_ids(&self) -> impl Iterator<Item = DomainId> + '_ {
        self.ids().filter(|id| *id != DomainId::PROJECT)
    }

    pub fn get(&self, id: DomainId) -> Option<&DomainDef> {
        self.domains.iter().find(|d| d.id == id)
    }

    /// The asset kinds of `domain` (none for an unknown one).
    pub fn asset_kinds(&self, domain: DomainId) -> &[AssetKindDef] {
        self.get(domain).map_or(&[], |d| &d.asset_kinds)
    }

    /// The asset kind `suffix` of `domain`.
    pub fn asset_kind(&self, domain: DomainId, suffix: &str) -> Option<&AssetKindDef> {
        self.asset_kinds(domain).iter().find(|k| k.suffix == suffix)
    }

    /// The tools of `domain` (none for an unknown one), in registration order.
    pub fn tools(&self, domain: DomainId) -> &[ToolDef] {
        self.get(domain).map_or(&[], |d| &d.tools)
    }

    /// The tool `id` of `domain`.
    pub fn tool(&self, domain: DomainId, id: &str) -> Option<&ToolDef> {
        self.tools(domain).iter().find(|t| t.id == id)
    }

    /// Core's own editor kinds (the text viewer), then every domain's in registration order; a kind two domains
    /// register (a kit's) is listed once.
    pub fn kind_defs(&self) -> EditorKinds {
        let mut kinds = EditorKinds(vec![crate::editors::KindDef::of::<crate::editors::text::TextEditor>()]);
        for k in self.domains.iter().flat_map(|d| &d.editor_kinds) {
            match kinds.get(k.view) {
                None => kinds.0.push(*k),
                Some(first) => assert!(
                    first.type_id == k.type_id,
                    "two different editor kinds share the view id `{}`: namespace it (`<domain>.<kind>`)",
                    k.view.0
                ),
            }
        }
        kinds
    }

    /// The umbrella crate's modules (design §6.5): one per domain with a backend, in registration order (never the
    /// project domain, which has none). The editor and the CLI build `crates/quillshade` from this same list.
    pub fn crate_contributions(&self) -> &[CrateContribution] {
        &self.crates
    }

    /// The Game Assets definitions, with their domains.
    pub fn game_assets(&self) -> impl Iterator<Item = (DomainId, &GameAssetsDef)> {
        self.domains.iter().filter_map(|d| d.game_assets.as_ref().map(|g| (d.id, g)))
    }

    /// The Game Assets definition of `domain` (`None`: it shows no game files).
    pub fn game_assets_of(&self, domain: DomainId) -> Option<&GameAssetsDef> {
        self.get(domain).and_then(|d| d.game_assets.as_ref())
    }

    /// Whether some domain shows `path` (relative to `assets/`) in the Game Assets tree: the files scanned for every
    /// domain, of which the tree lists the active domain's.
    pub fn is_game_file(&self, path: &str) -> bool {
        self.game_assets().any(|(_, g)| g.accepts(path))
    }

    /// The domains shown while `active` is the active domain (design §14.2): the project domain and the active one.
    pub fn shown(&self, active: Option<DomainId>) -> impl Iterator<Item = &DomainDef> + '_ {
        self.domains.iter().filter(move |d| d.id == DomainId::PROJECT || Some(d.id) == active)
    }

    /// The items domains add to `menu` while `active` is the active domain (design §14.2): the active domain's, then
    /// the project domain's (always shown).
    pub fn menu_items(
        &self,
        menu: MainMenu,
        active: Option<DomainId>,
    ) -> impl Iterator<Item = fn(&Editor) -> Vec<crate::menu::MenuEntry>> {
        self.shown(active).flat_map(|d| &d.menu_items).filter(move |(m, _)| *m == menu).map(|(_, f)| *f)
    }

    /// A user config entry as a tab key (design §6.6): `"<domain>:<key>"` of a registered domain; an entry with no
    /// domain prefix (written before domains existed) belongs to the domain claiming legacy keys. `None` for an
    /// unknown domain.
    pub fn parse_tab_key(&self, entry: &str) -> Option<TabKey> {
        match TabKey::split(entry) {
            Some((id, key)) => self.domains.iter().find(|d| d.id.as_str() == id).map(|d| TabKey::new(d.id, key)),
            None => self.domains.iter().find(|d| d.legacy_tab_keys).map(|d| TabKey::new(d.id, entry)),
        }
    }

    /// The highlighter for language `lang`: the first domain's that registered it, else core's own (Rust).
    pub fn highlighter(&self, lang: &str) -> Option<super::Highlighter> {
        let domain = self.domains.iter().flat_map(|d| &d.highlighters).find(|(l, _)| *l == lang).map(|(_, h)| *h);
        domain.or_else(|| crate::highlight::builtin(lang))
    }

    /// The domains' Quick Start steps (design §8.2), in registration order; core's own steps come before them
    /// ([`crate::quick_start::steps`]).
    pub fn quick_starts(&self) -> impl Iterator<Item = (DomainId, super::QuickStartDef)> + '_ {
        self.domains.iter().filter_map(|d| d.quick_start.map(|q| (d.id, q)))
    }

    /// One empty state per domain that registered one.
    pub fn states(&self) -> DomainStates {
        let mut states = DomainStates::default();
        for make in self.domains.iter().filter_map(|d| d.state) {
            let state: Box<dyn DomainState> = make();
            states.insert(state);
        }
        states
    }
}
