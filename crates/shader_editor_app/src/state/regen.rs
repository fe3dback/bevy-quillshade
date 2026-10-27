use super::*;
use crate::editors::{StatusCtx, StatusInfo};
use shader_project::RegenReport;
use shader_project::writer::Change;

impl Editor {
    // ---- validation / generation ----

    /// Assets as the open documents have them (unsaved edits included), the others as on disk.
    pub(super) fn overridden_set(&self) -> AssetSet {
        let mut set = self.disk.clone();
        for t in &self.tabs {
            t.doc().override_into(&mut set);
        }
        set
    }

    /// Whether the in-memory state changed since the last validation.
    pub fn needs_validation(&self) -> bool {
        self.project.is_some() && self.validated.as_ref() != Some(&self.revisions())
    }

    pub(super) fn revisions(&self) -> Vec<(AssetId, u64)> {
        // Open tables too: a constant's type or name changes the diagnostics of the graphs reading it. Open Texture
        // assets: a path or sampler edit changes E011 / E012 / E013 / W109.
        let tables = self.tabs.iter().filter_map(Tab::consts).map(|c| (c.id.clone(), c.revision));
        let textures = self.tabs.iter().filter_map(Tab::texture).filter(|t| !t.is_internal());
        let textures = textures.map(|t| (t.id.clone(), t.revision));
        self.docs().map(|d| (d.id.clone(), d.revision)).chain(tables).chain(textures).collect()
    }

    /// Live validation (dry run of the whole pipeline on the unsaved state).
    pub fn validate(&mut self) {
        let Some(project) = self.project.clone() else { return };
        self.refresh_functions();
        let set = self.overridden_set();
        let opts = RegenOptions { dry_run: true, ..Default::default() };
        // A Texture tab's Properties list its problems and hold no text field: redraw them when those changed.
        let texture = self.texture_doc().filter(|t| !t.is_internal()).map(|t| t.id.clone());
        let before = texture.as_ref().and_then(|id| self.diags.get(id).cloned());
        if let Ok(report) = shader_project::regen::regenerate_set(&project, &set, &opts) {
            self.project_diags = report.project;
            self.diags = report.assets.into_iter().map(|(id, r)| (id, r.diagnostics)).collect();
        }
        if let Some(id) = &texture
            && self.diags.get(id) != before.as_ref()
        {
            self.ui.props += 1;
        }
        self.validated = Some(self.revisions());
        self.ui.log += 1;
    }

    /// Full regeneration from what is saved on disk (with WESL compile check when available).
    pub fn regenerate(&mut self) -> bool {
        let Some(project) = self.project.clone() else { return false };
        self.rescan();
        // Unsaved documents already written from memory (a preview, decision 0022) keep that output.
        let mut set = self.disk.clone();
        for d in self.docs().filter(|d| d.regen_from_memory && d.is_dirty()) {
            set.assets.insert(d.id.clone(), Ok(d.asset.clone()));
        }
        let opts = RegenOptions { scope: Scope::All, dry_run: false, compile_check: self.bevy.as_ref() };
        match shader_project::regen::regenerate_set(&project, &set, &opts) {
            Ok(report) => {
                let errors = report.diagnostics().filter(|d| d.is_error()).count();
                self.regen_only = report
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
                    format!("Regenerated: {n} file(s) written")
                };
                if errors > 0 {
                    self.push_msg(
                        Level::Warning,
                        format!("{msg}; {errors} error(s) - affected assets kept their previous output"),
                    );
                } else {
                    self.info(msg);
                }
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

    /// Regenerates the material of tab `i` from its in-memory document (other assets as on disk), without saving
    /// it (a preview change, spec §5 of the QA design / decision 0022). `false` when the asset has errors (its
    /// generated files are kept) or writing failed.
    pub(super) fn regenerate_from_memory(&mut self, i: usize) -> bool {
        let Some(d) = self.tabs.get_mut(i).and_then(Tab::graph_mut) else { return false };
        // A clean document generates what the file does: nothing to restore later.
        d.regen_from_memory = d.is_dirty();
        let (id, asset) = (d.id.clone(), d.asset.clone());
        let mut set = self.disk.clone();
        set.assets.insert(id.clone(), Ok(asset));
        self.regenerate_one(&id, &set)
    }

    /// Regenerates one asset from what is saved on disk (an unsaved preview was discarded, decision 0022).
    pub(super) fn regenerate_from_disk(&mut self, id: &AssetId) {
        let set = self.disk.clone();
        self.regenerate_one(id, &set);
    }

    /// Writes the generated files of `id` (and its dependents) from `set`, through the usual writer.
    pub(super) fn regenerate_one(&mut self, id: &AssetId, set: &AssetSet) -> bool {
        let Some(project) = self.project.clone() else { return false };
        let opts = RegenOptions {
            scope: Scope::Affected([id.clone()].into()),
            dry_run: false,
            compile_check: self.bevy.as_ref(),
        };
        let ok = match shader_project::regen::regenerate_set(&project, set, &opts) {
            Ok(report) => {
                let diags = report.assets.get(id).map(|r| r.diagnostics.as_slice()).unwrap_or_default();
                let errors = diags.iter().filter(|d| d.is_error()).count();
                let regen_only = diags.iter().filter(|d| matches!(d.code, Code::E008 | Code::E010)).cloned().collect();
                self.regen_only.insert(id.clone(), regen_only);
                self.note_wesl_lines(&report, true);
                if errors > 0 {
                    let msg =
                        format!("Preview: {errors} error(s) in {} - its generated files were kept", id.source_rel());
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

    /// Remembers the line count of each `.wesl` the report generated without errors (the editor statusbar); an asset
    /// with errors keeps its previous file and count. `written_only`: count only files the run created or modified
    /// (a scoped run also generates assets it does not write).
    fn note_wesl_lines(&mut self, report: &RegenReport, written_only: bool) {
        let Some(project) = &self.project else { return };
        let written: BTreeSet<&Path> =
            report.changes.iter().filter(|c| !matches!(c, Change::Delete(_))).map(Change::path).collect();
        for (id, r) in &report.assets {
            let (true, Some(g), Some(file)) = (r.ok, &r.wesl, wesl_file(project, id)) else { continue };
            if !written_only || written.contains(file.as_path()) {
                self.wesl_lines.insert(id.clone(), g.text.lines().count());
            }
        }
    }

    /// The editor statusbar's content for the active tab (design §2.3); `None` without a tab or for a kind that has
    /// no statusbar.
    pub fn status_info(&self) -> Option<StatusInfo> {
        let doc = self.tab()?.doc();
        let id = doc.asset();
        let diagnostics: Vec<Diagnostic> = id
            .into_iter()
            .flat_map(|id| self.diags.get(id).into_iter().chain(self.regen_only.get(id)).flatten())
            .cloned()
            .collect();
        let wesl_lines = id.and_then(|id| self.wesl_lines.get(id).copied());
        let ctx = StatusCtx { diagnostics: &diagnostics, wesl_lines };
        doc.status(&ctx)
    }

    /// The first diagnostic of the active tab's asset, in console/logs order (live ones, then regeneration-only
    /// codes): what a click on the editor statusbar's problem count focuses.
    pub fn active_first_diagnostic(&self) -> Option<&Diagnostic> {
        let id = self.tab()?.doc().asset()?;
        self.diags.get(id).into_iter().chain(self.regen_only.get(id)).flatten().next()
    }

    /// All diagnostics for display: live ones plus regeneration-only codes.
    pub fn all_diagnostics(&self) -> Vec<Diagnostic> {
        let mut out: Vec<Diagnostic> = self.project_diags.clone();
        let active = self.doc().map(|d| d.id.clone());
        let mut ids: Vec<&AssetId> = self.diags.keys().chain(self.regen_only.keys()).collect();
        ids.sort_by_key(|id| (Some(*id) != active.as_ref(), (*id).clone()));
        ids.dedup();
        for id in ids {
            out.extend(self.diags.get(id).into_iter().flatten().cloned());
            out.extend(self.regen_only.get(id).into_iter().flatten().cloned());
        }
        out
    }

    /// Diagnostics of the active document by node.
    pub fn node_diags(&self) -> BTreeMap<NodeId, Vec<&Diagnostic>> {
        let mut m: BTreeMap<NodeId, Vec<&Diagnostic>> = BTreeMap::new();
        let Some(d) = self.doc() else { return m };
        for diag in self.diags.get(&d.id).into_iter().flatten().chain(self.regen_only.get(&d.id).into_iter().flatten())
        {
            if let Some(n) = &diag.node {
                m.entry(n.clone()).or_default().push(diag);
            }
        }
        m
    }

    pub fn save(&mut self, i: usize) -> bool {
        let Some(project) = &self.project else { return false };
        let Some(doc) = self.tabs.get_mut(i).map(Tab::doc_mut).filter(|d| !d.is_read_only()) else { return false };
        let name = doc.asset().map_or_else(|| doc.base_title(), AssetId::source_rel);
        if let Err(e) = doc.save(project) {
            self.io_error(&format!("Cannot save {name}"), e);
            return false;
        }
        self.info(format!("Saved {name}"));
        self.ui.tabs += 1;
        true
    }

    pub fn save_all(&mut self) -> bool {
        let mut ok = true;
        for i in 0..self.tabs.len() {
            if self.tabs[i].doc().is_dirty() {
                ok &= self.save(i);
            }
        }
        ok
    }
}
