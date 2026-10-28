//! Regeneration pipeline (spec §10.1, 12): load → validate → IR → WESL + Rust → atomic write.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use shader_codegen_rust::MaterialEntry;
use shader_codegen_rust::manifest::{ManifestMaterial, ManifestTexture, manifest_ron, parse_manifest};
use shader_codegen_wesl::check::{self, BevySources, Files};
use shader_codegen_wesl::{GenCtx, Generated, paths};
use shader_graph_core::consts::GlobalConst;
use shader_graph_core::gradient::{GradientCollection, texture_id};
use shader_graph_core::ident::check_ident;
use shader_graph_core::ir::{Ir, IrResult, build_ir, declared_function_sig, find_cycle};
use shader_graph_core::model::{Asset, Preview};
use shader_graph_core::registry::{FUNCTION_PREFIX, const_refs, curve_refs, gradient_refs, texture_refs};
use shader_graph_core::texture::{INTERNAL_PREFIX, KTX2_HEADER_LEN, TextureAsset, ktx2_not_2d};
use shader_graph_core::{Code, Diagnostic, FunctionSig, Registry};
use shader_stdlib::internal::{DEFAULT_TEXTURE, INTERNAL_TEXTURES};

use crate::assets::{self, AssetId, AssetSet, AssetType};
use crate::gradient_png;
use crate::layout::{GRADIENTS_DIR, INTERNAL_DIR, INTERNAL_MARKER, Project};
use crate::writer::{Change, WriteBatch};

/// Which assets to regenerate.
#[derive(Debug, Clone, Default)]
pub enum Scope {
    /// Whole project, including orphan cleanup.
    #[default]
    All,
    /// The given assets and everything depending on them (no orphan cleanup).
    Affected(BTreeSet<AssetId>),
}

#[derive(Default)]
pub struct RegenOptions<'a> {
    pub scope: Scope,
    /// Plan only, do not touch the disk (`check`, `diff`).
    pub dry_run: bool,
    /// Compile generated WESL against Bevy sources (wesl + naga) before writing.
    pub compile_check: Option<&'a BevySources>,
}

/// Result of one asset.
#[derive(Debug, Clone, Default)]
pub struct AssetResult {
    pub diagnostics: Vec<Diagnostic>,
    pub ir: Option<Ir>,
    pub wesl: Option<Generated>,
    /// Generated and not blocked by errors. For a Texture asset: valid (no E011 / E012 / E013 / load error), so the
    /// game may load it.
    pub ok: bool,
}

#[derive(Debug, Default)]
pub struct RegenReport {
    /// Project-level diagnostics (W105, user nodes, crate conflicts).
    pub project: Vec<Diagnostic>,
    pub assets: BTreeMap<AssetId, AssetResult>,
    pub changes: Vec<Change>,
}

impl RegenReport {
    pub fn diagnostics(&self) -> impl Iterator<Item = &Diagnostic> {
        self.project.iter().chain(self.assets.values().flat_map(|a| a.diagnostics.iter()))
    }

    pub fn has_errors(&self) -> bool {
        self.diagnostics().any(Diagnostic::is_error)
    }
}

/// Dependencies between assets (function calls, sampled curves, global constants read, Texture assets referenced,
/// gradient collections sampled).
#[derive(Debug, Clone, Default)]
pub struct DepGraph {
    pub uses: BTreeMap<AssetId, BTreeSet<AssetId>>,
    pub used_by: BTreeMap<AssetId, BTreeSet<AssetId>>,
}

impl DepGraph {
    pub fn build(set: &AssetSet, registry: &Registry) -> DepGraph {
        let mut g = DepGraph::default();
        let (consts, _) = collect_consts(set);
        let mut add = |user: &AssetId, used: AssetId| {
            g.uses.entry(user.clone()).or_default().insert(used.clone());
            g.used_by.entry(used).or_default().insert(user.clone());
        };
        for (id, a) in &set.assets {
            let Ok(a) = a else { continue };
            for n in a.nodes.values() {
                if let Some(p) = n.kind.strip_prefix(FUNCTION_PREFIX) {
                    add(id, AssetId::new(AssetType::Function, p));
                }
            }
            for (_, _, path) in curve_refs(a, registry) {
                add(id, AssetId::new(AssetType::Curve, path));
            }
            for (_, _, name) in const_refs(a, registry) {
                if let Some((table, _)) = consts.get(&name) {
                    add(id, table.clone());
                }
            }
            // Internal textures are not project assets.
            for (_, _, texture) in texture_refs(a, registry) {
                if !texture.starts_with(INTERNAL_PREFIX) {
                    add(id, AssetId::new(AssetType::Texture, texture));
                }
            }
            // A collection's slot order is compiled into its users (the row literal, gradient design §7).
            for (_, _, collection) in gradient_refs(a, registry) {
                add(id, AssetId::new(AssetType::Gradients, collection));
            }
        }
        g
    }

    /// `changed` plus everything that (transitively) uses it.
    pub fn affected(&self, changed: &BTreeSet<AssetId>) -> BTreeSet<AssetId> {
        let mut out = changed.clone();
        let mut stack: Vec<AssetId> = changed.iter().cloned().collect();
        while let Some(id) = stack.pop() {
            for u in self.used_by.get(&id).into_iter().flatten() {
                if out.insert(u.clone()) {
                    stack.push(u.clone());
                }
            }
        }
        out
    }
}

/// Global constants nodes can read (decision 0017), by name with their table, and the problems of every table:
/// load errors, bad paths (the whole table is ignored), bad or repeated names (that constant is ignored; a name
/// defined again in a later table, in path order, is reported there and the first definition counts).
pub type ConstTables = (BTreeMap<String, (AssetId, GlobalConst)>, BTreeMap<AssetId, Vec<Diagnostic>>);

pub fn collect_consts(set: &AssetSet) -> ConstTables {
    let mut consts: BTreeMap<String, (AssetId, GlobalConst)> = BTreeMap::new();
    let mut diags: BTreeMap<AssetId, Vec<Diagnostic>> = BTreeMap::new();
    for (id, table) in &set.consts {
        let d = diags.entry(id.clone()).or_default();
        d.extend(assets::validate_path(id));
        let table = match table {
            Ok(t) if d.is_empty() => t,
            Ok(_) => continue,
            Err(e) => {
                d.push(Diagnostic::new(Code::E009, format!("cannot load: {e}")).in_asset(id.source_rel()));
                continue;
            }
        };
        for c in &table.consts {
            let problem = match (check_ident(&c.name), consts.get(&c.name)) {
                (Err(e), _) => format!("constant name: {e}"),
                (Ok(()), Some((other, _))) if other == id => format!("duplicate constant `{}`", c.name),
                (Ok(()), Some((other, _))) => format!("constant `{}` is already defined in {other}", c.name),
                (Ok(()), None) => {
                    consts.insert(c.name.clone(), (id.clone(), c.clone()));
                    continue;
                }
            };
            d.push(Diagnostic::new(Code::E005, problem).in_asset(id.source_rel()));
        }
    }
    (consts, diags)
}

/// Image paths of the loadable Texture assets, each with the assets pointing at it (sorted); more than one is E011.
pub fn texture_paths(set: &AssetSet) -> BTreeMap<String, Vec<AssetId>> {
    let mut out: BTreeMap<String, Vec<AssetId>> = BTreeMap::new();
    for (id, t) in &set.textures {
        if let Ok(t) = t {
            out.entry(t.path.clone()).or_default().push(id.clone());
        }
    }
    out
}

/// Texture ids nodes may refer to: every Texture asset of the set (one with errors still exists; its problems are
/// reported at the asset itself) and the internal textures (`@internal/*`).
pub fn texture_ids(set: &AssetSet) -> BTreeSet<String> {
    let internal = shader_stdlib::internal::INTERNAL_TEXTURES.iter().map(|t| t.id.to_string());
    set.textures.keys().map(|id| id.path.clone()).chain(internal).collect()
}

/// Problems of every Texture asset (design §1.1): bad asset path (E005), load error (E009), invalid image path
/// (E012), an image path shared with other Texture assets (E011, at each of them: neither may silently win Bevy's
/// "first load wins"), sampler settings wgpu cannot create (E013), a KTX2 cube map / array / 3D texture (E014), image
/// file missing under `assets/` (W109). An asset with an error is not loaded by the game (materials fall back to the
/// default texture).
pub fn texture_diagnostics(project: &Project, set: &AssetSet) -> BTreeMap<AssetId, Vec<Diagnostic>> {
    let paths = texture_paths(set);
    let mut out = BTreeMap::new();
    for (id, t) in &set.textures {
        let mut d = assets::validate_path(id);
        let at = |code, message: String| Diagnostic::new(code, message).in_asset(id.source_rel());
        if let Ok(Some(Err(e))) = t.as_ref().map(|t| t.settings.sampler.map(|s| s.check())) {
            d.push(at(Code::E013, format!("invalid sampler settings: {e}")));
        }
        match t {
            Err(e) => d.push(at(Code::E009, format!("cannot load: {e}"))),
            Ok(t) => match TextureAsset::check_path(&t.path) {
                Err(e) => d.push(at(Code::E012, format!("invalid texture path: {e}"))),
                Ok(()) => {
                    let others: Vec<String> =
                        paths[&t.path].iter().filter(|o| *o != id).map(AssetId::source_rel).collect();
                    if !others.is_empty() {
                        let message = format!(
                            "`{}` is also used by {}; a game file can have only one Texture asset",
                            t.path,
                            others.join(", ")
                        );
                        d.push(at(Code::E011, message));
                    }
                    let file = project.assets_dir().join(&t.path);
                    if !file.is_file() {
                        d.push(at(Code::W109, format!("`assets/{}` does not exist", t.path)));
                    } else if let Some(e) = ktx2_header(&file).as_deref().and_then(ktx2_not_2d) {
                        d.push(at(Code::E014, format!("`assets/{}`: {e}", t.path)));
                    }
                }
            },
        }
        out.insert(id.clone(), d);
    }
    out
}

/// The first [`KTX2_HEADER_LEN`] bytes of a `.ktx2` file (`None` for another extension or a shorter file).
fn ktx2_header(file: &std::path::Path) -> Option<Vec<u8>> {
    use std::io::Read;
    if !file.extension().is_some_and(|e| e.eq_ignore_ascii_case("ktx2")) {
        return None;
    }
    let mut header = vec![0; KTX2_HEADER_LEN];
    std::fs::File::open(file).ok()?.read_exact(&mut header).ok()?;
    Some(header)
}

/// Gradient collections nodes can sample (gradient design §6): the loadable ones with a valid asset path, by id.
/// Others leave E006 at their users; their own problems are reported by [`gradient_diagnostics`].
pub fn gradient_collections(set: &AssetSet) -> BTreeMap<String, GradientCollection> {
    set.gradients
        .iter()
        .filter(|(id, _)| assets::validate_path(id).is_empty())
        .filter_map(|(id, c)| Some((id.path.clone(), c.as_ref().ok()?.clone())))
        .collect()
}

/// Problems of every gradient collection: bad asset path (E005), load error (E009).
pub fn gradient_diagnostics(set: &AssetSet) -> BTreeMap<AssetId, Vec<Diagnostic>> {
    let mut out = BTreeMap::new();
    for (id, c) in &set.gradients {
        let mut d = assets::validate_path(id);
        if let Err(e) = c {
            d.push(Diagnostic::new(Code::E009, format!("cannot load: {e}")).in_asset(id.source_rel()));
        }
        out.insert(id.clone(), d);
    }
    out
}

/// Registry with stdlib + project user nodes.
pub fn project_registry(project: &Project) -> (Registry, Vec<Diagnostic>) {
    let mut r = shader_stdlib::registry();
    let d = assets::load_user_nodes(project, &mut r);
    (r, d)
}

/// Output paths of an asset (`None` for constant tables, Texture assets and gradient collections, which generate no
/// WESL).
pub fn wesl_file(project: &Project, id: &AssetId) -> Option<PathBuf> {
    Some(project.assets_dir().join(paths::module_file(&module_of(id)?)))
}

/// WESL module of an asset.
pub fn module_of(id: &AssetId) -> Option<Vec<String>> {
    Some(match id.ty {
        AssetType::Material => paths::material_module(&id.path),
        AssetType::Function => paths::function_module(&id.path),
        AssetType::Curve => paths::curve_module(&id.path),
        AssetType::Consts | AssetType::Texture | AssetType::Gradients => return None,
    })
}

/// Generated Rust module of a material (`src/materials/dir_env/mat_rock_wet.rs`, decision 0020).
pub fn rust_file(project: &Project, id: &AssetId) -> Option<PathBuf> {
    (id.ty == AssetType::Material).then(|| {
        let module = paths::material_components(&id.path).join("/");
        project.crate_dir().join("src/materials").join(format!("{module}.rs"))
    })
}

/// Baked PNG of a gradient collection (`assets/shaders/gradients/common/terrain.png`, gradient design §5).
pub fn gradient_png_file(project: &Project, id: &AssetId) -> Option<PathBuf> {
    (id.ty == AssetType::Gradients).then(|| project.gradients_dir().join(format!("{}.png", id.path)))
}

/// Whether a file's first line carries the AUTOGENERATED marker (`//` or `#` comment).
pub fn has_marker(text: &str) -> bool {
    let first = text.lines().next().unwrap_or("");
    let body = first.trim_start_matches("//").trim_start_matches('#').trim_start();
    body.starts_with("AUTOGENERATED by shader-editor v") && first.contains("do not edit")
}

fn civil_today() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let z = (secs / 86400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Runs the pipeline. Errors only on I/O failure of the final write.
pub fn regenerate(project: &Project, opts: &RegenOptions) -> std::io::Result<RegenReport> {
    let set = AssetSet::load_all(project);
    regenerate_set(project, &set, opts)
}

pub fn regenerate_set(project: &Project, set: &AssetSet, opts: &RegenOptions) -> std::io::Result<RegenReport> {
    let (registry, user_diags) = project_registry(project);
    let mut report = RegenReport { project: user_diags, ..Default::default() };
    if let Some(w) = project.workspace_warning() {
        report.project.push(w);
    }
    let date = project.settings.header_date.then(civil_today);
    let deps = DepGraph::build(set, &registry);
    let mut results: BTreeMap<AssetId, AssetResult> = BTreeMap::new();

    // Load errors and path checks.
    let mut loaded: BTreeMap<&AssetId, &Asset> = BTreeMap::new();
    for (id, a) in &set.assets {
        let r = results.entry(id.clone()).or_default();
        r.diagnostics.extend(assets::validate_path(id));
        match a {
            Ok(a) => {
                loaded.insert(id, a);
            }
            Err(e) => {
                r.diagnostics.push(Diagnostic::new(Code::E009, format!("cannot load: {e}")).in_asset(id.source_rel()))
            }
        }
    }
    // Curves: loadable ones can be sampled (others leave E006 at their users).
    let mut curves = BTreeMap::new();
    for (id, c) in &set.curves {
        let r = results.entry(id.clone()).or_default();
        r.diagnostics.extend(assets::validate_path(id));
        match c {
            Ok(c) if r.diagnostics.is_empty() => {
                curves.insert(id, c);
            }
            Ok(_) => {}
            Err(e) => {
                r.diagnostics.push(Diagnostic::new(Code::E009, format!("cannot load: {e}")).in_asset(id.source_rel()))
            }
        }
    }
    let curve_paths: BTreeSet<String> = curves.keys().map(|id| id.path.clone()).collect();
    // Global constants: values are inlined by the nodes reading them, tables generate nothing.
    let (consts, const_diags) = collect_consts(set);
    for (id, d) in const_diags {
        results.entry(id).or_default().diagnostics.extend(d);
    }
    let consts: BTreeMap<String, GlobalConst> = consts.into_iter().map(|(name, (_, c))| (name, c)).collect();
    // Texture assets: validated only, they generate nothing.
    for (id, d) in texture_diagnostics(project, set) {
        let r = results.entry(id).or_default();
        r.ok = !d.iter().any(Diagnostic::is_error);
        r.diagnostics.extend(d);
    }
    let textures = texture_ids(set);
    // Gradient collections: validated only here; their users read the slot positions.
    for (id, d) in gradient_diagnostics(set) {
        let r = results.entry(id).or_default();
        r.ok = !d.iter().any(Diagnostic::is_error);
        r.diagnostics.extend(d);
    }
    let gradients = gradient_collections(set);

    // Functions: declared signatures, recursion check, topological build.
    let fn_ids: Vec<&AssetId> = loaded.keys().copied().filter(|i| i.ty == AssetType::Function).collect();
    let mut sigs: BTreeMap<String, FunctionSig> = BTreeMap::new();
    for id in &fn_ids {
        sigs.insert(id.path.clone(), declared_function_sig(loaded[id], &id.path, &registry));
    }
    let fn_paths: Vec<&str> = fn_ids.iter().map(|i| i.path.as_str()).collect();
    let mut succ: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for id in &fn_ids {
        for u in deps.uses.get(*id).into_iter().flatten().filter(|u| u.ty == AssetType::Function) {
            if let Some(p) = fn_paths.iter().find(|p| **p == u.path) {
                succ.entry(id.path.as_str()).or_default().insert(p);
            }
        }
    }
    let mut failed_fns: BTreeSet<String> = BTreeSet::new();
    let mut remaining: BTreeSet<&str> = fn_paths.iter().copied().collect();
    while let Some(cycle) = find_cycle(&remaining.iter().copied().collect::<Vec<_>>(), &succ) {
        for p in &cycle {
            let id = AssetId::new(AssetType::Function, *p);
            results.entry(id.clone()).or_default().diagnostics.push(
                Diagnostic::new(Code::E006, format!("recursive function call: {}", cycle.join(" → ")))
                    .in_asset(id.source_rel()),
            );
            failed_fns.insert(p.to_string());
            remaining.remove(p);
        }
    }
    // Dependencies first.
    let mut order: Vec<&str> = Vec::new();
    let mut visited = BTreeSet::new();
    fn visit<'a>(
        p: &'a str,
        succ: &BTreeMap<&'a str, BTreeSet<&'a str>>,
        rem: &BTreeSet<&'a str>,
        seen: &mut BTreeSet<&'a str>,
        out: &mut Vec<&'a str>,
    ) {
        if !rem.contains(p) || !seen.insert(p) {
            return;
        }
        for d in succ.get(p).into_iter().flatten() {
            visit(d, succ, rem, seen, out);
        }
        out.push(p);
    }
    for p in &remaining {
        visit(p, &succ, &remaining, &mut visited, &mut order);
    }

    let gen_ctx_file = |id: &AssetId| id.source_rel();
    for p in order {
        let id = AssetId::new(AssetType::Function, p);
        let asset = loaded[&id];
        let r = build_ir(asset, p, &registry, &sigs, &curve_paths, &consts, &textures, &gradients);
        let entry = results.entry(id.clone()).or_default();
        entry.diagnostics.extend(r.diagnostics.into_iter().map(|d| d.in_asset(id.source_rel())));
        for u in deps.uses.get(&id).into_iter().flatten().filter(|u| u.ty == AssetType::Function) {
            if failed_fns.contains(&u.path) {
                entry.diagnostics.push(
                    Diagnostic::new(Code::E006, format!("depends on function `{}` which has errors", u.path))
                        .in_asset(id.source_rel()),
                );
            }
        }
        if let Some(ir) = &r.ir
            && let Some(sig) = &ir.function_sig
        {
            sigs.insert(p.to_string(), sig.clone());
        }
        entry.ir = r.ir;
        if entry.diagnostics.iter().any(Diagnostic::is_error) {
            failed_fns.insert(p.to_string());
        }
    }
    for id in &fn_ids {
        if results.get(*id).is_some_and(|r| r.diagnostics.iter().any(Diagnostic::is_error)) {
            failed_fns.insert(id.path.clone());
        }
    }

    // Materials.
    for id in loaded.keys().copied().filter(|i| i.ty == AssetType::Material) {
        let build = |a: &Asset| build_ir(a, &id.path, &registry, &sigs, &curve_paths, &consts, &textures, &gradients);
        let mut r = build(loaded[id]);
        if let Some(w) = preview_changes_rust(loaded[id], &r, &id.source_rel(), build) {
            r.diagnostics.push(w);
        }
        let entry = results.entry(id.clone()).or_default();
        entry.diagnostics.extend(r.diagnostics.into_iter().map(|d| d.in_asset(id.source_rel())));
        for u in deps.uses.get(id).into_iter().flatten().filter(|u| u.ty == AssetType::Function) {
            if failed_fns.contains(&u.path) && sigs.contains_key(&u.path) {
                entry.diagnostics.push(
                    Diagnostic::new(Code::E006, format!("depends on function `{}` which has errors", u.path))
                        .in_asset(id.source_rel()),
                );
            }
        }
        entry.ir = r.ir;
    }

    // Generate WESL; Custom node parse check (E007).
    for (id, r) in results.iter_mut() {
        if r.diagnostics.iter().any(Diagnostic::is_error) {
            continue;
        }
        let Some(ir) = &r.ir else { continue };
        for (name, code) in &ir.custom_fns {
            if let Some(e) = check::parse_error(code) {
                let node = name.strip_prefix("sg_custom_").or_else(|| name.strip_prefix("sg_user_")).unwrap_or(name);
                r.diagnostics.push(
                    Diagnostic::new(Code::E007, format!("Custom WESL does not parse: {e}"))
                        .at_node(node)
                        .in_asset(id.source_rel()),
                );
            }
        }
        if r.diagnostics.iter().any(Diagnostic::is_error) {
            continue;
        }
        let source = gen_ctx_file(id);
        let ctx = GenCtx { functions: &sigs, source_file: &source };
        let mut g = match id.ty {
            AssetType::Material => shader_codegen_wesl::material(ir, &ctx),
            AssetType::Function => shader_codegen_wesl::function(ir, &ctx),
            AssetType::Curve | AssetType::Consts | AssetType::Texture | AssetType::Gradients => continue,
        };
        if let Some(d) = &date {
            insert_date(&mut g.text, d);
            for (l, _) in g.line_nodes.iter_mut() {
                *l += 1;
            }
        }
        r.wesl = Some(g);
        r.ok = true;
    }

    for (id, c) in &curves {
        let mut text = shader_codegen_wesl::curve(c, &id.path, &id.source_rel());
        if let Some(d) = &date {
            insert_date(&mut text, d);
        }
        let r = results.get_mut(*id).expect("curve result");
        r.wesl = Some(Generated { text, line_nodes: Vec::new() });
        r.ok = true;
    }

    // Optional compile check against Bevy sources.
    if let Some(bevy) = opts.compile_check {
        let mut files = Files::default();
        for (name, src) in shader_stdlib::MODULES {
            files.add(&paths::stdlib_module(name), shader_codegen_wesl::stdlib_module(name, src));
        }
        for (id, r) in &results {
            let (Some(module), Some(file)) = (module_of(id), wesl_file(project, id)) else { continue };
            if let Some(g) = &r.wesl {
                files.add(&module, g.text.clone());
                if let Some(ir) = r.ir.as_ref().filter(|i| i.kind.material().is_some()) {
                    files.enable(ir.layout_feature());
                }
            } else if let Ok(old) = std::fs::read_to_string(file) {
                files.add(&module, old);
            }
        }
        for (id, r) in results.iter_mut() {
            if !r.ok || id.ty != AssetType::Material {
                continue;
            }
            let kind = *r.ir.as_ref().and_then(|i| i.kind.material()).expect("material IR");
            for v in check::variants_for(&kind) {
                if let Err(e) = check::compile(bevy, &files, &paths::material_module(&id.path), *v) {
                    r.diagnostics.push(
                        Diagnostic::new(Code::E010, format!("generated WESL failed to compile: {e}"))
                            .in_asset(id.source_rel()),
                    );
                    r.ok = false;
                    break;
                }
            }
        }
    }

    // Scope.
    let in_scope: Option<BTreeSet<AssetId>> = match &opts.scope {
        Scope::All => None,
        Scope::Affected(ids) => Some(deps.affected(ids)),
    };
    let scoped = |id: &AssetId| in_scope.as_ref().is_none_or(|s| s.contains(id));

    // Desired files, in write order: stdlib, curves, functions, materials, Rust crate, manifest, internal textures,
    // gradient collections.
    let mut desired: Vec<(PathBuf, Vec<u8>)> = Vec::new();
    let autogen = project.autogen_dir();
    for (name, src) in shader_stdlib::MODULES {
        let mut text = shader_codegen_wesl::stdlib_module(name, src);
        if let Some(d) = &date {
            insert_date(&mut text, d);
        }
        desired.push((autogen.join("stdlib").join(format!("{name}.wesl")), text.into_bytes()));
    }
    for ty in [AssetType::Curve, AssetType::Function, AssetType::Material] {
        for (id, r) in &results {
            if id.ty == ty && r.ok && scoped(id) {
                desired.push((wesl_file(project, id).unwrap(), r.wesl.as_ref().unwrap().text.clone().into_bytes()));
            }
        }
    }
    // Directories of binary files (internal textures, design §2; baked gradient collections, gradient design §5):
    // binary files cannot carry the marker line, so each directory is owned through its marker file. A foreign one
    // blocks only its own files (E008).
    let internal = project.internal_dir();
    let internal_owned = owned_binary_dir(&internal);
    let gradients_dir = project.gradients_dir();
    let gradients_owned = owned_binary_dir(&gradients_dir);
    // Baked manifest (design §3.2.1): failed materials keep their previous entry, read back from the old file.
    let manifest_path = autogen.join("baked.ron");
    let old_manifest: BTreeMap<String, ManifestMaterial> = std::fs::read_to_string(&manifest_path)
        .ok()
        .filter(|t| has_marker(t))
        .and_then(|t| parse_manifest(&t).ok())
        .map(|(_, m)| m.into_iter().map(|m| (m.path.clone(), m)).collect())
        .unwrap_or_default();
    let manifest_textures = manifest_textures(set, &results, gradients_owned);
    let loadable: BTreeSet<&str> = manifest_textures.iter().map(|t| t.id.as_str()).collect();
    let mut manifest_materials: Vec<ManifestMaterial> = Vec::new();
    // Rust: modules of OK materials, entries also for failed materials that keep their old module.
    let mut entries = Vec::new();
    let mut modules = BTreeMap::new();
    let mut keep: BTreeSet<PathBuf> = BTreeSet::new();
    for (id, r) in &results {
        if id.ty != AssetType::Material {
            continue;
        }
        let rs = rust_file(project, id).unwrap();
        if r.ok {
            let ir = r.ir.as_ref().unwrap();
            entries.push(MaterialEntry::from_ir(ir));
            manifest_materials.push(ManifestMaterial::from_ir(ir));
            if scoped(id) {
                let mut text = shader_codegen_rust::material_module(ir, &id.source_rel());
                if let Some(d) = &date {
                    insert_date(&mut text, d);
                }
                modules.insert(id.path.clone(), text);
            } else {
                keep.insert(rs);
                keep.extend(wesl_file(project, id));
            }
        } else {
            keep.extend(wesl_file(project, id));
            if let Ok(old) = std::fs::read_to_string(&rs)
                && has_marker(&old)
            {
                keep.insert(rs);
                entries.push(entry_from_old(id, &old));
                // Without an old entry (manifest deleted): an empty one, the game keeps the compiled defaults.
                manifest_materials.push(
                    old_manifest
                        .get(&id.path)
                        .cloned()
                        .unwrap_or_else(|| ManifestMaterial { path: id.path.clone(), ..Default::default() }),
                );
            }
        }
    }
    for (id, r) in &results {
        if id.ty != AssetType::Material && (!r.ok || !scoped(id)) {
            keep.extend(wesl_file(project, id));
        }
    }
    // A Texture asset the game does not load (E011 / E012 / E013 / load error, or gone since an old entry was
    // written) → the default texture.
    for t in manifest_materials.iter_mut().flat_map(|m| m.textures.values_mut()) {
        if !loadable.contains(t.as_str()) {
            *t = DEFAULT_TEXTURE.to_string();
        }
    }
    let crate_dir = project.crate_dir();
    let texture_ids = texture_field_ids(set);
    for (rel, text) in shader_codegen_rust::crate_files(&entries, &modules, project.bevy_dep(), &texture_ids) {
        desired.push((crate_dir.join(rel), text.into_bytes()));
    }
    let mut manifest = manifest_ron(&manifest_textures, &manifest_materials);
    if let Some(d) = &date {
        insert_date(&mut manifest, d);
    }
    desired.push((manifest_path, manifest.into_bytes()));
    let not_owned = |dir: &std::path::Path, what: &str| {
        Diagnostic::new(
            Code::E008,
            format!(
                "`{}` is not a directory with a {INTERNAL_MARKER} marker file (or cannot be read); not writing {what}",
                project.rel(dir)
            ),
        )
    };
    // Internal textures (design §2), always written.
    if internal_owned {
        let marker = format!("{}\n", shader_codegen_wesl::header("internal textures"));
        desired.push((internal.join(INTERNAL_MARKER), marker.into_bytes()));
        for t in INTERNAL_TEXTURES {
            desired.push((internal.join(t.file), t.bytes.to_vec()));
        }
    } else {
        report.project.push(not_owned(&internal, "internal textures"));
    }
    // Baked gradient collections (gradient design §5), written by every regeneration, a scoped one too: a color edit
    // changes only the PNG. A failed collection keeps its old PNG (not in the manifest, so the game does not use it).
    // Without collections nothing is written (a full regen deletes the last PNG and the marker as orphans) and a
    // foreign `gradients/` is no error.
    let has_gradients = !set.gradients.is_empty();
    if has_gradients && gradients_owned {
        let marker = format!("{}\n", shader_codegen_wesl::header("baked gradient collections"));
        desired.push((gradients_dir.join(INTERNAL_MARKER), marker.into_bytes()));
        for (id, c) in &set.gradients {
            let png = gradient_png_file(project, id).expect("collection PNG");
            match c {
                Ok(c) if results.get(id).is_some_and(|r| r.ok) => desired.push((png, gradient_png::encode(c))),
                _ => {
                    keep.insert(png);
                }
            }
        }
    } else if has_gradients {
        report.project.push(not_owned(&gradients_dir, "gradient collections"));
    }

    // Compare with disk, refuse to overwrite foreign files (E008), binary ones included. Files in the binary
    // directories are owned through the directory marker (only desired when it is present).
    let mut batch = WriteBatch::default();
    let binary_dirs = [internal, gradients_dir];
    let desired_paths: BTreeSet<PathBuf> = desired.iter().map(|(p, _)| p.clone()).collect();
    for (path, bytes) in desired {
        let blocked = match std::fs::read(&path) {
            Ok(old) if old == bytes => continue,
            Ok(old)
                if !binary_dirs.iter().any(|d| path.starts_with(d))
                    && !std::str::from_utf8(&old).is_ok_and(has_marker) =>
            {
                format!("`{}` has no AUTOGENERATED marker; not overwriting", project.rel(&path))
            }
            Ok(_) => {
                batch.writes.push((path, bytes));
                continue;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                batch.writes.push((path, bytes));
                continue;
            }
            // Unreadable (permissions, a directory in the way, …): it may be a foreign file, never replace it.
            Err(e) => format!("`{}` cannot be read ({e}); not overwriting", project.rel(&path)),
        };
        let d = Diagnostic::new(Code::E008, blocked);
        match results.iter_mut().find(|(id, _)| {
            [wesl_file(project, id), rust_file(project, id), gradient_png_file(project, id)]
                .contains(&Some(path.clone()))
        }) {
            Some((id, r)) => {
                r.diagnostics.push(d.in_asset(id.source_rel()));
                r.ok = false;
            }
            None => report.project.push(d),
        }
    }

    // Orphans (full regen only): marker files that no source produces any more; in an owned binary directory every
    // file regen does not produce.
    let [internal, gradients_dir] = binary_dirs;
    let owned_dirs: Vec<PathBuf> = [(internal, internal_owned), (gradients_dir, gradients_owned)]
        .into_iter()
        .filter(|(_, o)| *o)
        .map(|(d, _)| d)
        .collect();
    let mut roots = vec![autogen, crate_dir];
    roots.extend(owned_dirs.iter().cloned());
    if in_scope.is_none() {
        for root in &roots {
            for f in walk_files(root) {
                if desired_paths.contains(&f) || keep.contains(&f) {
                    continue;
                }
                if f.file_name().is_some_and(|n| n.to_string_lossy().contains(".tmp-")) {
                    continue;
                }
                if owned_dirs.contains(root) || std::fs::read_to_string(&f).is_ok_and(|t| has_marker(&t)) {
                    batch.deletes.push(f);
                }
            }
        }
    }

    report.changes = batch.changes();
    if !opts.dry_run && !batch.is_empty() {
        batch.apply(&roots)?;
    }
    report.assets = results;
    Ok(report)
}

/// W108 (decision 0022): the preview IR `r` of `asset` generates a different Rust module than the asset without the
/// preview — the previewed upstream uses a parameter or builtin the material output does not, so the game has to be
/// rebuilt (spec §5 wants only `.wesl` to change). Checked only when both IRs are free of errors.
fn preview_changes_rust(
    asset: &Asset,
    r: &IrResult,
    source: &str,
    build: impl Fn(&Asset) -> IrResult,
) -> Option<Diagnostic> {
    let ir = r.ir.as_ref().filter(|ir| ir.preview.is_some() && ir.kind.material().is_some())?;
    if r.diagnostics.iter().any(Diagnostic::is_error) {
        return None;
    }
    let mut normal = asset.clone();
    normal.preview = Preview::default();
    let n = build(&normal);
    let normal_ir = n.ir.as_ref().filter(|_| !n.diagnostics.iter().any(Diagnostic::is_error))?;
    (shader_codegen_rust::material_module(ir, source) != shader_codegen_rust::material_module(normal_ir, source)).then(
        || {
            Diagnostic::new(
                Code::W108,
                "preview changes the generated Rust crate (extra parameters or builtins); rebuild the game, and \
                 again after turning preview off",
            )
        },
    )
}

fn insert_date(text: &mut String, date: &str) {
    if let Some(pos) = text.find('\n') {
        text.insert_str(pos + 1, &format!("// generated: {date}\n"));
    }
}

/// Textures of the baked manifest (design §3.2.1): every Texture asset the game may load (no error: E011 / E012 /
/// E013 / load error exclude it, so neither of two assets on one file wins Bevy's "first load wins"), used or not,
/// the internal textures and the baked PNG of every collection without errors (gradient design §5; none when regen
/// does not own `gradients/`, which then holds no PNG of ours).
fn manifest_textures(
    set: &AssetSet,
    results: &BTreeMap<AssetId, AssetResult>,
    gradients_owned: bool,
) -> Vec<ManifestTexture> {
    let internal_rel = INTERNAL_DIR.strip_prefix("assets/").expect("internal dir is under assets/");
    let project = set.textures.iter().filter_map(|(id, t)| {
        let t = t.as_ref().ok().filter(|_| results.get(id).is_some_and(|r| r.ok))?;
        Some(ManifestTexture { id: id.path.clone(), path: t.path.clone(), settings: t.settings.clone() })
    });
    let internal = INTERNAL_TEXTURES.iter().map(|t| ManifestTexture {
        id: t.id.to_string(),
        path: format!("{internal_rel}/{}", t.file),
        settings: (t.settings)(),
    });
    let gradients_rel = GRADIENTS_DIR.strip_prefix("assets/").expect("gradients dir is under assets/");
    let gradients = set
        .gradients
        .iter()
        .filter(|(id, c)| gradients_owned && c.is_ok() && results.get(*id).is_some_and(|r| r.ok))
        .map(|(id, _)| ManifestTexture {
            id: texture_id(&id.path),
            path: format!("{gradients_rel}/{}.png", id.path),
            settings: gradient_png::texture_settings(),
        });
    project.chain(internal).chain(gradients).collect()
}

/// Ids of the `BakedTextures` fields (design §3.1 "all of them"): every Texture asset that parses and has a valid
/// name — also one with an error (E011 / E012 / E013), so such an error never changes the public shape of the
/// generated crate (the runtime points that field at the default texture) — and the internal textures. A Texture
/// asset whose name is no identifier (E005) has no field (it cannot name a Rust field); one whose RON does not parse
/// (E009) is left out too, per the final review ruling: its content is unknown, so nothing says it is a usable
/// Texture asset. Gradient collections follow the same rule (`@gradients/<path>`, gradient design §5).
fn texture_field_ids(set: &AssetSet) -> Vec<String> {
    let usable = |id: &AssetId, ok: bool| ok && assets::validate_path(id).is_empty();
    let project = set.textures.iter().filter(|(id, t)| usable(id, t.is_ok())).map(|(id, _)| id.path.clone());
    let gradients = set.gradients.iter().filter(|(id, c)| usable(id, c.is_ok())).map(|(id, _)| texture_id(&id.path));
    project.chain(INTERNAL_TEXTURES.iter().map(|t| t.id.to_string())).chain(gradients).collect()
}

/// Rebuilds a crate entry of a material whose current graph is broken from its previous module.
fn entry_from_old(id: &AssetId, old: &str) -> MaterialEntry {
    let base = shader_graph_core::ident::to_pascal_case(id.name());
    let pbr = old.contains("impl MaterialExtension for");
    let params = format!("{base}Params");
    MaterialEntry {
        path: id.path.clone(),
        main: if pbr { format!("{base}Ext") } else { format!("{base}Material") },
        material: format!("{base}Material"),
        params: old.contains(&format!("pub struct {params}")).then_some(params),
    }
}

/// Whether regen may write a directory of binary files (`internal/`, `gradients/`): it carries the marker file, or
/// it is an empty directory, or it does not exist. Anything else (a regular file in the way, an unreadable directory)
/// is not owned (E008).
fn owned_binary_dir(dir: &std::path::Path) -> bool {
    let not_found = |e: &std::io::Error| e.kind() == std::io::ErrorKind::NotFound;
    match std::fs::read(dir.join(INTERNAL_MARKER)) {
        Ok(b) => std::str::from_utf8(&b).is_ok_and(has_marker),
        Err(e) if not_found(&e) => match std::fs::read_dir(dir) {
            Ok(mut entries) => entries.next().is_none(),
            Err(e) => not_found(&e),
        },
        Err(_) => false,
    }
}

/// Regular files below `root` (the orphan scan). Symlinks are skipped entirely: regen only ever writes regular files,
/// so a link is never its own orphan, and following a linked directory could loop forever or delete files outside
/// the project (in an owned binary directory every unproduced file is deleted without a marker check).
fn walk_files(root: &std::path::Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let Ok(ty) = e.file_type() else { continue };
            if ty.is_symlink() {
                continue;
            }
            let p = e.path();
            if ty.is_dir() {
                // Never descend into build output of the generated crate.
                if p.file_name().is_some_and(|n| n == "target") {
                    continue;
                }
                stack.push(p);
            } else {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}
