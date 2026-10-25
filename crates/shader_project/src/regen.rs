//! Regeneration pipeline (spec §10.1, 12): load → validate → IR → WESL + Rust → atomic write.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use shader_codegen_rust::MaterialEntry;
use shader_codegen_wesl::check::{self, BevySources, Files};
use shader_codegen_wesl::{GenCtx, Generated, paths};
use shader_graph_core::consts::GlobalConst;
use shader_graph_core::ident::check_ident;
use shader_graph_core::ir::{Ir, IrResult, build_ir, declared_function_sig, find_cycle};
use shader_graph_core::model::{Asset, Preview};
use shader_graph_core::registry::{FUNCTION_PREFIX, const_refs, curve_refs};
use shader_graph_core::{Code, Diagnostic, FunctionSig, Registry};

use crate::assets::{self, AssetId, AssetSet, AssetType};
use crate::layout::Project;
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
    /// Generated and not blocked by errors.
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

/// Dependencies between assets (function calls, sampled curves, global constants read).
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

/// Registry with stdlib + project user nodes.
pub fn project_registry(project: &Project) -> (Registry, Vec<Diagnostic>) {
    let mut r = shader_stdlib::registry();
    let d = assets::load_user_nodes(project, &mut r);
    (r, d)
}

/// Output paths of an asset (`None` for constant tables, which generate nothing).
pub fn wesl_file(project: &Project, id: &AssetId) -> Option<PathBuf> {
    Some(project.root.join("assets").join(paths::module_file(&module_of(id)?)))
}

/// WESL module of an asset.
pub fn module_of(id: &AssetId) -> Option<Vec<String>> {
    Some(match id.ty {
        AssetType::Material => paths::material_module(&id.path),
        AssetType::Function => paths::function_module(&id.path),
        AssetType::Curve => paths::curve_module(&id.path),
        AssetType::Consts => return None,
    })
}

/// Generated Rust module of a material (`src/materials/dir_env/mat_rock_wet.rs`, decision 0020).
pub fn rust_file(project: &Project, id: &AssetId) -> Option<PathBuf> {
    (id.ty == AssetType::Material).then(|| {
        let module = paths::material_components(&id.path).join("/");
        project.crate_dir().join("src/materials").join(format!("{module}.rs"))
    })
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
        let r = build_ir(asset, p, &registry, &sigs, &curve_paths, &consts);
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
        let build = |a: &Asset| build_ir(a, &id.path, &registry, &sigs, &curve_paths, &consts);
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
            AssetType::Curve | AssetType::Consts => continue,
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

    // Desired files, in write order: stdlib, curves, functions, materials, Rust crate.
    let mut desired: Vec<(PathBuf, String)> = Vec::new();
    let autogen = project.autogen_dir();
    for (name, src) in shader_stdlib::MODULES {
        let mut text = shader_codegen_wesl::stdlib_module(name, src);
        if let Some(d) = &date {
            insert_date(&mut text, d);
        }
        desired.push((autogen.join("stdlib").join(format!("{name}.wesl")), text));
    }
    for ty in [AssetType::Curve, AssetType::Function, AssetType::Material] {
        for (id, r) in &results {
            if id.ty == ty && r.ok && scoped(id) {
                desired.push((wesl_file(project, id).unwrap(), r.wesl.as_ref().unwrap().text.clone()));
            }
        }
    }
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
            }
        }
    }
    for (id, r) in &results {
        if id.ty != AssetType::Material && (!r.ok || !scoped(id)) {
            keep.extend(wesl_file(project, id));
        }
    }
    let crate_dir = project.crate_dir();
    for (rel, text) in shader_codegen_rust::crate_files(&entries, &modules, project.bevy_dep()) {
        desired.push((crate_dir.join(rel), text));
    }

    // Compare with disk, refuse to overwrite foreign files (E008).
    let mut batch = WriteBatch::default();
    let desired_paths: BTreeSet<PathBuf> = desired.iter().map(|(p, _)| p.clone()).collect();
    for (path, text) in desired {
        match std::fs::read_to_string(&path) {
            Ok(old) if old == text => {}
            Ok(old) if !has_marker(&old) => {
                let d = Diagnostic::new(
                    Code::E008,
                    format!("`{}` has no AUTOGENERATED marker; not overwriting", project.rel(&path)),
                );
                match results.iter_mut().find(|(id, _)| {
                    wesl_file(project, id).as_ref() == Some(&path) || rust_file(project, id).as_ref() == Some(&path)
                }) {
                    Some((id, r)) => {
                        r.diagnostics.push(d.in_asset(id.source_rel()));
                        r.ok = false;
                    }
                    None => report.project.push(d),
                }
            }
            _ => batch.writes.push((path, text)),
        }
    }

    // Orphans (full regen only): marker files that no source produces any more.
    if in_scope.is_none() {
        for root in [&autogen, &crate_dir] {
            for f in walk_files(root) {
                if desired_paths.contains(&f) || keep.contains(&f) {
                    continue;
                }
                if f.file_name().is_some_and(|n| n.to_string_lossy().contains(".tmp-")) {
                    continue;
                }
                if std::fs::read_to_string(&f).is_ok_and(|t| has_marker(&t)) {
                    batch.deletes.push(f);
                }
            }
        }
    }

    report.changes = batch.changes();
    if !opts.dry_run && !batch.is_empty() {
        batch.apply(&[autogen, crate_dir])?;
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

fn walk_files(root: &std::path::Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
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
