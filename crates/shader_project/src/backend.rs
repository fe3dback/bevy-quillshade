//! The Material domain's backend (design §4.2, §6.5): what it contributes to the umbrella crate `crates/quillshade`
//! and its [`DomainBackend`] for `project_core::regen::run`.

use std::collections::BTreeSet;

use project_core::asset_index::{AssetIndex, DomainIndex, IndexedAsset, IndexedRef, SubItem};
use project_core::layout::Project;
use project_core::regen::{DomainBackend, DomainPlan};
use project_core::umbrella::CrateContribution;
use project_core::{AssetRef, AssetUuid, DomainId};
use shader_codegen_wesl::check::BevySources;
use shader_graph_core::Registry;
use shader_graph_core::model::Asset;
use shader_graph_core::registry::{FUNCTION_PREFIX, ParamDef, ParamKind};
use shader_stdlib::user::USER_PREFIX;

use crate::assets::{AssetId, AssetSet, UserNodeFile};
use crate::layout::MATERIAL;
use crate::regen::{RegenOptions, Scope, plan_set, project_nodes};
use crate::resolve::ref_type;

/// Bevy features the generated code needs; kept with `workspace = true` too (features are additive). `png` / `ktx2`
/// / `zstd_rust`: the image formats of Texture assets (design §3.4); `bevy_log`: warnings of the baked loader.
const BEVY_FEATURES: &[&str] = &[
    "std",
    "bevy_pbr",
    "bevy_sprite_render",
    "bevy_asset",
    "bevy_color",
    "bevy_mesh",
    "bevy_log",
    "png",
    "ktx2",
    "zstd_rust",
];

/// The Material module of the umbrella crate: `material::QuillshadeMaterialInternalPlugin`, plus serde / ron for the
/// baked manifest.
pub const MATERIAL_CRATE: CrateContribution = CrateContribution {
    domain: MATERIAL,
    plugin: "QuillshadeMaterialInternalPlugin",
    loader: Some("BakedMaterialsLoader"),
    bevy_features: BEVY_FEATURES,
    dependencies: &[("serde", r#"{ version = "1", features = ["derive"] }"#), ("ron", r#""0.12""#)],
};

/// The Material domain as a [`DomainBackend`]: the whole project, orphan cleanup included.
#[derive(Default)]
pub struct MaterialBackend {
    /// Bevy WESL sources for the compile check (wesl + naga); `None` skips the check even when it is asked for,
    /// unless [`MaterialBackend::discover`] is set.
    pub compile_check: Option<BevySources>,
    /// Without [`MaterialBackend::compile_check`], a plan asked to compile finds the sources in the cargo registry
    /// itself (once), with a warning on stderr when they are missing: the CLI's `--compile`.
    pub discover: bool,
    found: std::sync::OnceLock<Option<BevySources>>,
}

impl MaterialBackend {
    /// A backend that finds the Bevy sources when a compile check is asked for (the CLI's, design §10).
    pub fn discovering() -> MaterialBackend {
        MaterialBackend { discover: true, ..MaterialBackend::default() }
    }

    fn sources(&self) -> Option<&BevySources> {
        if self.compile_check.is_some() || !self.discover {
            return self.compile_check.as_ref();
        }
        let found = self.found.get_or_init(|| {
            let s = BevySources::discover();
            if s.is_none() {
                eprintln!("warning: Bevy 0.20 WESL sources not found in the cargo registry; skipping compile check");
            }
            s
        });
        found.as_ref()
    }

    /// The whole project's plan with only `scope`'s files regenerated (and orphan-scanned for [`Scope::All`]).
    fn plan_scope(&self, project: &Project, index: &AssetIndex, scope: Scope, compile: bool) -> DomainPlan {
        let set = AssetSet::load_all(project);
        let compile_check = if compile { self.sources() } else { None };
        let opts = RegenOptions { scope, compile_check, index: Some(index), ..Default::default() };
        let (report, output) = plan_set(project, &set, &opts);
        DomainPlan { diagnostics: report.diagnostics().map(|d| d.to_generic(MATERIAL)).collect(), output }
    }
}

impl DomainBackend for MaterialBackend {
    fn id(&self) -> DomainId {
        MATERIAL
    }

    fn crate_contribution(&self) -> CrateContribution {
        MATERIAL_CRATE
    }

    /// [`index_part`] of the whole project and its user nodes (stdlib and project user nodes recognize the
    /// references).
    fn index(&self, project: &Project) -> DomainIndex {
        let (registry, user_nodes, _) = project_nodes(project);
        index_part(&AssetSet::load_all(project), &user_nodes, &registry)
    }

    fn plan(&self, project: &Project, index: &AssetIndex, compile: bool) -> DomainPlan {
        self.plan_scope(project, index, Scope::All, compile)
    }

    /// [`Scope::Affected`] of the Material assets in `affected` (techdoc §4.3); what uses them inside the domain
    /// follows through the domain's own dependency graph. A user node
    /// is no generated asset: its users are in `affected` already (the index's edges).
    fn plan_affected(
        &self,
        project: &Project,
        index: &AssetIndex,
        affected: &BTreeSet<AssetRef>,
        compile: bool,
    ) -> DomainPlan {
        let ids = affected.iter().filter_map(AssetId::from_ref).collect();
        self.plan_scope(project, index, Scope::Affected(ids), compile)
    }
}

/// The Material domain's part of the `AssetIndex` (ADR 0047 §7): every loaded asset of `set` and every loaded user
/// node file of `user_nodes` (one that does not load is not listed) with its `id`, its sub-items (constants of a
/// table, gradient slots) and, for graphs, its UUID references found through `registry`: function and user node
/// kinds and the reference params, located at their node, with the sub-item id of a constant / gradient slot param.
/// A value that is no UUID (a hand-written path) is not an edge.
pub fn index_part(set: &AssetSet, user_nodes: &[UserNodeFile], registry: &Registry) -> DomainIndex {
    let mut assets = Vec::new();
    let mut add = |id: &AssetId, uuid: AssetUuid, sub_items: Vec<SubItem>, refs: Vec<IndexedRef>| {
        assets.push(IndexedAsset { asset: id.to_ref(), id: Ok(uuid), sub_items, refs });
    };
    for (id, a) in &set.assets {
        if let Ok(a) = a {
            add(id, a.id, Vec::new(), graph_refs(a, registry));
        }
    }
    for (id, c) in &set.curves {
        if let Ok(c) = c {
            add(id, c.id, Vec::new(), Vec::new());
        }
    }
    for (id, t) in &set.consts {
        if let Ok(t) = t {
            let subs = t.consts.iter().map(|c| SubItem::new(u64::from(c.id), c.name.clone())).collect();
            add(id, t.id, subs, Vec::new());
        }
    }
    for (id, t) in &set.features {
        if let Ok(t) = t {
            let mut subs = Vec::new();
            for e in &t.enums {
                subs.push(SubItem::with_uuid(e.id, e.name.clone()));
                for v in &e.variants {
                    subs.push(SubItem::with_uuid(v.id, format!("{}::{}", e.name, v.name)));
                }
            }
            add(id, t.id, subs, Vec::new());
        }
    }
    for (id, t) in &set.textures {
        if let Ok(t) = t {
            add(id, t.id, Vec::new(), Vec::new());
        }
    }
    for (id, g) in &set.gradients {
        if let Ok(g) = g {
            let subs = g.slots.iter().map(|s| SubItem::new(u64::from(s.id), s.name.clone())).collect();
            add(id, g.id, subs, Vec::new());
        }
    }
    for f in user_nodes {
        if let Ok(n) = &f.node {
            assets.push(IndexedAsset { asset: f.asset.clone(), id: Ok(n.id), sub_items: Vec::new(), refs: Vec::new() });
        }
    }
    DomainIndex { domain: MATERIAL, assets }
}

/// The UUID references stored in a graph (see [`index_part`]).
fn graph_refs(asset: &Asset, registry: &Registry) -> Vec<IndexedRef> {
    let mut out = Vec::new();
    for (node, n) in &asset.nodes {
        let at = || Some(node.clone());
        if let Some(to) =
            [FUNCTION_PREFIX, USER_PREFIX].iter().find_map(|p| n.kind.strip_prefix(p)).and_then(AssetUuid::parse)
        {
            out.push(IndexedRef { to, sub: None, location: at() });
        }
        let Some(def) = registry.get(&n.kind) else { continue };
        let params = def.params();
        let value = |p: &ParamDef| n.params.get(&p.id).unwrap_or(&p.default).clone();
        for p in params.iter().filter(|p| ref_type(&p.kind).is_some()) {
            let Some(to) = AssetUuid::parse(value(p).as_str()) else { continue };
            // The sub-item params belonging to this reference: the constant of a table, the slot of a collection.
            let is_sub = |k: &ParamKind| match p.kind {
                ParamKind::GlobalConsts => matches!(k, ParamKind::GlobalConst(_)),
                ParamKind::GradientCollection => *k == ParamKind::GradientSlot,
                _ => false,
            };
            let subs: Vec<u64> = params
                .iter()
                .filter(|q| is_sub(&q.kind))
                .filter_map(|q| u64::try_from(value(q).as_i32()).ok())
                .filter(|&s| s != 0)
                .collect();
            if subs.is_empty() {
                out.push(IndexedRef { to, sub: None, location: at() });
            }
            out.extend(subs.into_iter().map(|s| IndexedRef { to, sub: Some(s), location: at() }));
        }
    }
    out
}
