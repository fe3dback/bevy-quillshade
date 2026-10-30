//! Core flows on assets and folders (design §6.3): the path dialogs' answers, renaming / moving / deleting an asset
//! (the domain does the file work and what refers to it, through its [`DomainState`] hooks), the folders of each
//! domain's Project tree, focusing a diagnostic and Generate.

use std::path::PathBuf;

use crate::action::{Action, Dialog, Submit, TabAction};
use crate::dynval::PropValue;
use crate::tab::TabKey;
use project_core::{AssetRef, Diagnostic, DomainId, Project};

use super::editor::Editor;
#[cfg(doc)]
use crate::domain::DomainState;

impl Editor {
    /// `Action::Submit`: the text of the open input dialog.
    pub fn submit(&mut self, text: &str) {
        let Some(Dialog::Input { submit, .. }) = self.dialog.take() else { return };
        self.pick_folder = false;
        match submit {
            Submit::OpenProject => return self.apply(Action::OpenProject(PathBuf::from(text))),
            Submit::NewProject => return self.apply(Action::NewProject(PathBuf::from(text))),
            // The sidebar's edit, as one undo step (an empty label clears it); no inline validation, diagnostics
            // report a bad name (design §5.5). The sidebar's field shows the new text.
            Submit::SetText(key) => {
                // `Submit` is handled inside `apply` (which only follows up `Action::Tab`), so the tab edit's
                // `after_apply` is called here.
                let domain = self.tab().map(|t| t.domain);
                self.apply_tab(TabAction::SetValue { key, value: PropValue::Str(text.to_string()), merge: false });
                if let Some(id) = domain {
                    self.after_apply(id);
                }
                self.ui.props += 1;
                return;
            }
            _ => {}
        }
        let Some(project) = self.project.clone() else { return };
        let text = text.trim_matches('/');
        match submit {
            Submit::Create { kind, template } => self.create_asset(kind, template, text),
            Submit::Folder(domain) => match create_folder(&project, domain, text) {
                Ok(()) => {
                    self.current_folders.insert(domain, text.to_string());
                    self.rescan();
                }
                Err(e) => self.io_error("Cannot create folder", e),
            },
            Submit::Rename(r) => self.rename(&r, text),
            Submit::MoveTo(r) => {
                self.apply(Action::MoveTo(r, text.to_string()));
            }
            Submit::OpenProject | Submit::NewProject | Submit::SetText(_) => {}
        }
    }

    // ---- assets ----

    /// `Action::DeleteDialog`: asks before deleting `r`, with its domain's question
    /// ([`DomainState::delete_message`]).
    pub fn delete_dialog(&mut self, r: &AssetRef) {
        let custom = self.domains.get_dyn(r.domain).and_then(|s| s.delete_message(self, r));
        let message = custom.unwrap_or_else(|| format!("Delete {}?", r.source_rel()));
        self.dialog = Some(Dialog::Confirm {
            title: "Delete asset".into(),
            message,
            buttons: vec![("Cancel".into(), Action::CloseDialog), ("Delete".into(), Action::Delete(r.clone()))],
        });
    }

    /// `Action::MoveTo`: the asset keeps its name in folder `dir` (empty = the root).
    pub fn move_to(&mut self, r: &AssetRef, dir: &str) {
        let name = r.path.rsplit('/').next().unwrap_or(&r.path).to_string();
        let new_path = if dir.is_empty() { name } else { format!("{dir}/{name}") };
        self.rename(r, &new_path);
    }

    /// Rename / move (the Rename and Move dialogs): an unsaved document of the asset is saved first, its domain
    /// renames the file ([`DomainState::rename_file`]; a domain without a state: core's default,
    /// [`super::kinds::rename_file`]), the domains rescan, open documents, the navigation history and the domain's
    /// last tab follow (design §14.4), then the domain's own follow-up ([`DomainState::renamed`]: references,
    /// regeneration).
    pub fn rename(&mut self, r: &AssetRef, new_path: &str) {
        self.dialog = None;
        let Some(project) = self.project.clone() else { return };
        if let Some(i) = self.asset_tab(r)
            && self.tabs[i].doc().is_dirty()
            && !self.save(i)
        {
            return;
        }
        let renamed = self.with_domain(r.domain, |e, s| s.rename_file(e, r, new_path));
        let to = match renamed.unwrap_or_else(|| super::kinds::rename_file(self, r, new_path)) {
            Ok(to) => to,
            Err(e) => return self.io_error("Cannot rename", e),
        };
        if to == *r {
            return;
        }
        self.rescan();
        // The asset's document and its generated files follow (the domain's regeneration fills the files).
        for t in &mut self.tabs {
            t.doc_mut().rename(&project, r, &to);
        }
        let (from_key, to_key) = (TabKey::new(r.domain, r.source_rel()), TabKey::new(to.domain, to.source_rel()));
        self.nav.rename(&from_key, &to_key);
        for key in self.last_tabs.values_mut() {
            if let Some(new) = key.renamed(&from_key, &to_key) {
                *key = new;
            }
        }
        self.info(format!("Renamed {} -> {}", r.source_rel(), to.source_rel()));
        self.with_domain(r.domain, |e, s| s.renamed(e, r, &to));
        self.touch_all();
        self.save_config();
    }

    /// `Action::Delete`: the asset's tabs close (unsaved edits dropped), its domain deletes the file
    /// ([`DomainState::delete_file`]; a domain without a state: core's default, [`super::kinds::delete_file`]), the
    /// domains rescan, then the domain's own follow-up ([`DomainState::deleted`]).
    pub fn delete(&mut self, r: &AssetRef) {
        self.dialog = None;
        if self.project.is_none() {
            return;
        }
        self.with_domain(r.domain, |e, s| s.before_delete(e, r));
        while let Some(i) = self.tabs.iter().position(|t| t.owner() == Some(r)) {
            self.close_tab(i, true);
        }
        let deleted = self.with_domain(r.domain, |e, s| s.delete_file(e, r));
        if let Err(e) = deleted.unwrap_or_else(|| super::kinds::delete_file(self, r)) {
            return self.io_error("Cannot delete", e);
        }
        self.info(format!("Deleted {}", r.source_rel()));
        self.rescan();
        self.with_domain(r.domain, |e, s| s.deleted(e, r));
        self.touch_all();
    }

    // ---- folders (design §6.3) ----

    /// `Action::NewFolderDialog`: the path of a new folder of `domain`'s Project tree (`None`: the active real
    /// domain's, else the first real domain's), proposed in its selected folder.
    pub fn new_folder_dialog(&mut self, domain: Option<DomainId>) {
        let real = |d: &DomainId| *d != DomainId::PROJECT;
        let Some(domain) = domain.or(self.active_domain.filter(real)).or_else(|| self.domain_list.real_ids().next())
        else {
            return;
        };
        let dir = self.current_folder(domain).to_string();
        self.dialog = Some(Dialog::Input {
            title: "New folder".into(),
            label: format!("Folder path inside quillshade/{domain}/"),
            initial: if dir.is_empty() { "new_folder".into() } else { format!("{dir}/new_folder") },
            submit: Submit::Folder(domain),
        });
    }

    /// `Action::DeleteFolder`: an empty folder of `domain`'s Project tree.
    pub fn delete_folder(&mut self, domain: DomainId, path: &str) {
        self.dialog = None;
        if let Some(p) = &self.project {
            let dir = p.domain_dir(domain).join(path);
            project_core::writer::remove_empty_dirs(&dir);
            match std::fs::remove_dir(&dir) {
                Ok(()) => self.info(format!("Deleted folder {path}")),
                Err(e) => self.io_error("Cannot delete folder (is it empty?)", e),
            }
        }
        self.rescan();
    }

    // ---- diagnostics, Generate ----

    /// Opens the asset of a diagnostic (a console/logs row, the editor statusbar's count) through its domain
    /// ([`DomainState::focus_diagnostic`]); a diagnostic of no domain goes to every domain.
    pub fn focus_diagnostic(&mut self, diag: &Diagnostic) {
        let ids: Vec<DomainId> = match diag.domain {
            Some(id) => vec![id],
            None => self.domains.ids().collect(),
        };
        for id in ids {
            self.with_domain(id, |e, s| s.focus_diagnostic(e, diag));
        }
    }

    /// Generate (Assets menu, design §6.5): every domain regenerates from what is saved
    /// ([`DomainState::generate`]).
    pub fn generate(&mut self) {
        let ids: Vec<DomainId> = self.domains.ids().collect();
        for id in ids {
            self.with_domain(id, |e, s| s.generate(e));
        }
    }
}

/// A new folder `path` of `domain`'s Project tree: each component a project-wide identifier
/// ([`project_core::ident::check_path`]).
fn create_folder(project: &Project, domain: DomainId, path: &str) -> Result<(), String> {
    project_core::ident::check_path(path)?;
    project.ensure_layout().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(project.domain_dir(domain).join(path)).map_err(|e| e.to_string())
}
