use super::*;
use shader_project::RegenReport;
use shader_project::writer::Change;

/// Material's validation and regeneration.
pub trait MaterialRegen {
    /// Assets as the open documents have them (unsaved edits included), the others as on disk.
    fn overridden_set(&self) -> AssetSet;

    /// Whether the in-memory state changed since the last validation.
    fn needs_validation(&self) -> bool;

    fn revisions(&self) -> Vec<(AssetId, u64)>;

    /// Live validation (dry run of the whole pipeline on the unsaved state).
    fn validate(&mut self);

    /// Full regeneration from what is saved on disk (with WESL compile check when available).
    fn regenerate(&mut self) -> bool;

    /// Regenerates the material of tab `i` from its in-memory document (other assets as on disk), without saving
    /// it (a preview change, spec §5 of the QA design / decision 0022). `false` when the asset has errors (its
    /// generated files are kept) or writing failed.
    fn regenerate_from_memory(&mut self, i: usize) -> bool;

    /// Regenerates one asset from what is saved on disk (an unsaved preview was discarded, decision 0022).
    fn regenerate_from_disk(&mut self, id: &AssetId);

    /// Writes the generated files of `id` (and its dependents) from `set`, through the usual writer.
    fn regenerate_one(&mut self, id: &AssetId, set: &AssetSet) -> bool;

    /// Remembers the line count of each `.wesl` the report generated without errors (the editor statusbar); an asset
    /// with errors keeps its previous file and count. `written_only`: count only files the run created or modified
    /// (a scoped run also generates assets it does not write).
    /// Warns when a regeneration rewrote the generated Rust crate: only a rebuild of the game applies it. Until then
    /// a running game keeps its compiled materials, and a changed one whose hot-reloaded shader fails the layout guard
    /// (decision 0029) is not drawn — reported only in the game's log otherwise.
    fn note_rust_changes(&mut self, report: &RegenReport);

    fn note_wesl_lines(&mut self, report: &RegenReport, written_only: bool);

    /// The diagnostics of asset `id`, in console/logs order: live ones, then regeneration-only codes.
    fn asset_diagnostics(&self, id: &AssetId) -> impl Iterator<Item = &Diagnostic>;

    /// All of Material's diagnostics, as the console/logs lists them (core's `Editor::console_diagnostics` gives them
    /// as generic diagnostics): live ones plus regeneration-only codes.
    fn all_diagnostics(&self) -> Vec<Diagnostic>;

    /// Diagnostics of the active document by node.
    fn node_diags(&self) -> BTreeMap<NodeId, Vec<&Diagnostic>>;
}

impl MaterialRegen for Editor {
    // ---- validation / generation ----

    fn overridden_set(&self) -> AssetSet {
        let mut set = self.domains.material().disk.clone();
        for t in &self.tabs {
            crate::tab::override_into(t, &mut set);
        }
        set
    }

    fn needs_validation(&self) -> bool {
        self.project.as_ref().is_some_and(|p| p.domain_enabled(MATERIAL))
            && self.domains.material().validated.as_ref() != Some(&self.revisions())
    }

    fn revisions(&self) -> Vec<(AssetId, u64)> {
        // Open tables too: a constant's type or name changes the diagnostics of the graphs reading it. Open Texture
        // assets: a path or sampler edit changes E011 / E012 / E013 / W109.
        let tables = self.tabs.iter().filter_map(Tab::get::<ConstsDoc>).map(|c| (c.id.clone(), c.revision));
        let textures = self.tabs.iter().filter_map(Tab::get::<TextureDoc>).filter(|t| !t.is_internal());
        let textures = textures.map(|t| (t.id(), t.revision));
        // Open gradient collections: a cleared or refilled slot changes W111 of the graphs sampling it.
        let gradients = self.tabs.iter().filter_map(Tab::get::<GradientsDoc>).map(|g| (g.id.clone(), g.revision));
        let features = self.tabs.iter().filter_map(Tab::get::<FeaturesDoc>).map(|f| (f.id.clone(), f.revision));
        self.docs()
            .map(|d| (d.id.clone(), d.validation_revision))
            .chain(tables)
            .chain(textures)
            .chain(gradients)
            .chain(features)
            .collect()
    }

    fn validate(&mut self) {
        let Some(project) = self.project.clone().filter(|p| p.domain_enabled(MATERIAL)) else { return };
        let current_revs = self.revisions();
        let index_gen = self.asset_index.generation();
        let incremental_mats: Option<Vec<AssetId>> = {
            let m = self.domains.material();
            if let (Some(prev_revs), Some((cached_gen, cached_non_mat, _))) = (&m.validated, &m.validation_cache)
                && *cached_gen == index_gen
                && prev_revs.len() == current_revs.len()
                && prev_revs.iter().zip(&current_revs).all(|((a, _), (b, _))| a == b)
            {
                let non_mat: Vec<(AssetId, u64)> =
                    current_revs.iter().filter(|(id, _)| id.ty != AssetType::Material).cloned().collect();
                if &non_mat == cached_non_mat {
                    let changed: Vec<AssetId> = prev_revs
                        .iter()
                        .zip(&current_revs)
                        .filter(|((_, r1), (_, r2))| r1 != r2)
                        .map(|((_, _), (id, _))| id.clone())
                        .collect();
                    (!changed.is_empty()).then_some(changed)
                } else {
                    None
                }
            } else {
                None
            }
        };
        if let Some(changed_ids) = incremental_mats {
            let updates: Vec<(AssetId, Vec<Diagnostic>)> = {
                let m = self.domains.material();
                let (_, _, env) = m.validation_cache.as_ref().expect("checked above");
                changed_ids
                    .into_iter()
                    .filter_map(|id| {
                        let d = self.docs().find(|d| d.id == id)?;
                        let view = d.resolved(&self.asset_index, &m.registry);
                        let res = env.validate_material(&id, &view, &m.registry);
                        Some((id, res.diagnostics))
                    })
                    .collect()
            };
            let m = self.domains.material_mut();
            for (id, diags) in updates {
                m.diags.insert(id, diags);
            }
            m.validated = Some(current_revs);
            self.ui.log += 1;
            return;
        }

        self.refresh_functions();
        let set = self.overridden_set();
        // A Texture tab's Properties list its problems and hold no text field: redraw them when those changed.
        let texture = self.texture_doc().filter(|t| !t.is_internal()).map(|t| t.id());
        let before = texture.as_ref().and_then(|id| self.domains.material().diags.get(id).cloned());
        // UUID references resolve against the editor's index (ADR 0047 §4).
        let (report, env) = {
            let m = self.domains.material();
            shader_project::regen::validate_set(
                &project,
                &set,
                Some(&self.asset_index),
                Some((&m.registry, &m.user_node_diags)),
            )
        };
        let non_mat: Vec<(AssetId, u64)> =
            current_revs.iter().filter(|(id, _)| id.ty != AssetType::Material).cloned().collect();
        let m = self.domains.material_mut();
        m.project_diags = report.project;
        m.diags = report.assets.into_iter().map(|(id, r)| (id, r.diagnostics)).collect();
        m.validation_cache = Some((index_gen, non_mat, env));
        if let Some(id) = &texture
            && self.domains.material().diags.get(id) != before.as_ref()
        {
            self.ui.props += 1;
        }
        self.domains.material_mut().validated = Some(current_revs);
        self.ui.log += 1;
    }

    fn regenerate(&mut self) -> bool {
        let Some(project) = self.project.clone().filter(|p| p.domain_enabled(MATERIAL)) else { return false };
        self.rescan();
        // Unsaved documents already written from memory (a preview, decision 0022) keep that output.
        let mut set = self.domains.material().disk.clone();
        for d in self.docs().filter(|d| d.regen_from_memory && d.is_dirty()) {
            set.assets.insert(d.id.clone(), Ok(d.asset.clone()));
        }
        // The umbrella crate lists every registered domain, as the CLI's (design §6.5).
        let list = self.domain_list.clone();
        let opts = RegenOptions {
            scope: Scope::All,
            compile_check: self.domains.material().bevy.as_ref(),
            crates: list.crate_contributions(),
            index: Some(&self.asset_index),
            dry_run: false,
        };
        match shader_project::regen::regenerate_set(&project, &set, &opts) {
            Ok(report) => {
                let errors = report.diagnostics().filter(|d| d.is_error()).count();
                self.domains.material_mut().regen_only = report
                    .assets
                    .iter()
                    .map(|(id, r)| {
                        let v = r
                            .diagnostics
                            .iter()
                            .filter(|d| matches!(d.code, Code::E008 | Code::E010))
                            .cloned()
                            .collect();
                        (id.clone(), v)
                    })
                    .collect();
                let n = report.changes.len();
                let msg = if n == 0 {
                    "Regenerated: no changes".to_string()
                } else {
                    format!("Regenerated: {} written", plural(n, "file", "files"))
                };
                if errors > 0 {
                    self.push_msg(
                        Level::Warning,
                        format!(
                            "{msg}; {} - affected assets kept their previous output",
                            plural(errors, "error", "errors")
                        ),
                    );
                } else {
                    self.info(msg);
                }
                self.note_rust_changes(&report);
                self.note_wesl_lines(&report, false);
                self.validate();
                self.reload_read_only_tabs();
                true
            }
            Err(e) => {
                self.io_error("Regeneration failed", e);
                false
            }
        }
    }

    fn regenerate_from_memory(&mut self, i: usize) -> bool {
        let Some(d) = self.tabs.get_mut(i).and_then(Tab::get_mut::<Doc>) else { return false };
        // A clean document generates what the file does: nothing to restore later.
        d.regen_from_memory = d.is_dirty();
        let (id, asset) = (d.id.clone(), d.asset.clone());
        let mut set = self.domains.material().disk.clone();
        set.assets.insert(id.clone(), Ok(asset));
        self.regenerate_one(&id, &set)
    }

    fn regenerate_from_disk(&mut self, id: &AssetId) {
        let set = self.domains.material().disk.clone();
        self.regenerate_one(id, &set);
    }

    fn regenerate_one(&mut self, id: &AssetId, set: &AssetSet) -> bool {
        let Some(project) = self.project.clone().filter(|p| p.domain_enabled(MATERIAL)) else { return false };
        let list = self.domain_list.clone();
        let opts = RegenOptions {
            scope: Scope::Affected([id.clone()].into()),
            compile_check: self.domains.material().bevy.as_ref(),
            crates: list.crate_contributions(),
            index: Some(&self.asset_index),
            dry_run: false,
        };
        let ok = match shader_project::regen::regenerate_set(&project, set, &opts) {
            Ok(report) => {
                let diags = report.assets.get(id).map(|r| r.diagnostics.as_slice()).unwrap_or_default();
                let errors = diags.iter().filter(|d| d.is_error()).count();
                let regen_only = diags.iter().filter(|d| matches!(d.code, Code::E008 | Code::E010)).cloned().collect();
                self.domains.material_mut().regen_only.insert(id.clone(), regen_only);
                self.note_rust_changes(&report);
                self.note_wesl_lines(&report, true);
                if errors > 0 {
                    let errors = plural(errors, "error", "errors");
                    let msg = format!("Preview: {errors} in {} - its generated files were kept", id.source_rel());
                    self.push_msg(Level::Warning, msg);
                } else {
                    self.info(format!("Preview: regenerated {}", id.source_rel()));
                }
                errors == 0
            }
            Err(e) => {
                self.io_error("Regeneration failed", e);
                false
            }
        };
        self.validate();
        self.reload_read_only_tabs();
        ok
    }

    fn note_rust_changes(&mut self, report: &RegenReport) {
        let Some(project) = &self.project else { return };
        let crate_dir = project.crate_dir();
        let changed: BTreeSet<&Path> = report
            .changes
            .iter()
            .map(Change::path)
            .filter(|p| p.starts_with(&crate_dir) && p.extension().is_some_and(|e| e == "rs"))
            .collect();
        if changed.is_empty() {
            return;
        }
        let materials: Vec<String> = report
            .assets
            .keys()
            .filter(|id| rust_file(project, id).is_some_and(|f| changed.contains(f.as_path())))
            .map(|id| id.path.clone())
            .collect();
        let what = if materials.is_empty() {
            "The generated Rust crate changed".to_string()
        } else {
            format!("The generated Rust of {} changed", materials.join(", "))
        };
        self.push_msg(
            Level::Warning,
            format!("{what}: rebuild the game to apply it (a running game may not draw the changed materials)"),
        );
    }

    fn note_wesl_lines(&mut self, report: &RegenReport, written_only: bool) {
        let Some(project) = &self.project else { return };
        let written: BTreeSet<&Path> =
            report.changes.iter().filter(|c| !matches!(c, Change::Delete(_))).map(Change::path).collect();
        for (id, r) in &report.assets {
            let (true, Some(g), Some(file)) = (r.ok, &r.wesl, wesl_file(project, id)) else { continue };
            if !written_only || written.contains(file.as_path()) {
                self.domains.material_mut().wesl_lines.insert(id.clone(), g.text.lines().count());
            }
        }
    }

    fn asset_diagnostics(&self, id: &AssetId) -> impl Iterator<Item = &Diagnostic> {
        self.domains.material().diags.get(id).into_iter().chain(self.domains.material().regen_only.get(id)).flatten()
    }

    fn all_diagnostics(&self) -> Vec<Diagnostic> {
        let mut out: Vec<Diagnostic> = self.domains.material().project_diags.clone();
        let active = self.doc().map(|d| d.id.clone());
        let mut ids: Vec<&AssetId> =
            self.domains.material().diags.keys().chain(self.domains.material().regen_only.keys()).collect();
        ids.sort_by_key(|id| (Some(*id) != active.as_ref(), (*id).clone()));
        ids.dedup();
        for id in ids {
            out.extend(self.asset_diagnostics(id).cloned());
        }
        out
    }

    fn node_diags(&self) -> BTreeMap<NodeId, Vec<&Diagnostic>> {
        let mut m: BTreeMap<NodeId, Vec<&Diagnostic>> = BTreeMap::new();
        let Some(d) = self.doc() else { return m };
        for diag in self.asset_diagnostics(&d.id) {
            if let Some(n) = &diag.node {
                m.entry(n.clone()).or_default().push(diag);
            }
        }
        m
    }
}
