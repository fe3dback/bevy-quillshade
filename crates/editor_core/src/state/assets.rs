//! Core flows on assets and folders (design §6.3): the path dialogs' answers, renaming / moving / deleting an asset
//! (the domain does the file work and what refers to it, through its [`DomainState`] hooks), the folders of each
//! domain's Project tree, focusing a diagnostic, the asset UUID quick fixes (ADR 0047 §11, §16) and Generate.

use std::path::PathBuf;

use crate::action::{Action, Dialog, Submit, TabAction};
use crate::dynval::PropValue;
use crate::tab::TabKey;
use project_core::{AssetRef, AssetUuid, Diagnostic, DomainId, Project};

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
    /// last tab follow (design §14.4), then the domain's own follow-up ([`DomainState::renamed`]: regeneration). No
    /// reference is rewritten (ADR 0047 §13).
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
        if self.copied_asset.as_ref() == Some(r) {
            self.copied_asset = Some(to.clone());
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
        if self.copied_asset.as_ref() == Some(r) {
            self.copied_asset = None;
        }
        self.info(format!("Deleted {}", r.source_rel()));
        self.rescan();
        self.with_domain(r.domain, |e, s| s.deleted(e, r));
        self.touch_all();
    }

    /// Whether `Action::PasteAsset` into `domain` is currently disabled, returning the reason when disabled.
    pub fn paste_disabled_reason(&self, domain: DomainId) -> Option<String> {
        let Some(project) = &self.project else {
            return Some("No project open".into());
        };
        let Some(copied) = &self.copied_asset else {
            return Some("Nothing copied".into());
        };
        if copied.domain != domain {
            return Some(format!("Copied asset is from the `{}` domain", copied.domain));
        }
        if !project.root.join(copied.source_rel()).is_file() {
            return Some(format!("`{}` no longer exists", copied.source_rel()));
        }
        None
    }

    /// `Action::CopyAsset`: stores `r` in [`Editor::copied_asset`].
    pub fn copy_asset(&mut self, r: &AssetRef) {
        let Some(project) = &self.project else { return };
        if !project.root.join(r.source_rel()).is_file() {
            return;
        }
        self.copied_asset = Some(r.clone());
        self.info(format!("Copied {}", r.source_rel()));
    }

    /// `Action::DuplicateAsset`: copies `r` in its own folder under `<name>_copy` / `<name>_copy_N` with a fresh
    /// UUIDv7, rescans, and opens the new asset.
    pub fn duplicate_asset(&mut self, r: &AssetRef) {
        let folder = r.path.rsplit_once('/').map_or("", |(dir, _)| dir);
        self.copy_asset_to_folder(r, folder, false, "Duplicated");
    }

    /// `Action::PasteAsset`: copies [`Editor::copied_asset`] into `folder` of `domain` (keeping its leaf name when
    /// unoccupied in `folder`, else `<name>_copy` / `<name>_copy_N`) with a fresh UUIDv7, rescans, and opens it.
    pub fn paste_asset(&mut self, domain: DomainId, folder: &str) {
        if self.paste_disabled_reason(domain).is_some() {
            return;
        }
        let Some(copied) = self.copied_asset.clone() else { return };
        let folder = folder.trim_matches('/');
        if !folder.is_empty()
            && let Err(e) = project_core::ident::check_path(folder)
        {
            return self.io_error("Cannot paste asset", e);
        }
        self.copy_asset_to_folder(&copied, folder, true, "Pasted");
    }

    fn copy_asset_to_folder(&mut self, r: &AssetRef, folder: &str, keep_name_if_free: bool, verb: &str) {
        let Some(project) = self.project.clone() else { return };
        if let Some(i) = self.asset_tab(r)
            && self.tabs[i].doc().is_dirty()
            && !self.save(i)
        {
            return;
        }
        let from_file = project.root.join(r.source_rel());
        if !from_file.is_file() {
            return self.io_error("Cannot copy asset", format!("`{}` does not exist", r.source_rel()));
        }
        let to = unique_copy_ref(&project, r, folder, keep_name_if_free);
        let to_file = project.root.join(to.source_rel());
        if let Some(parent) = to_file.parent()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            return self.io_error("Cannot create parent folder", e);
        }
        if let Err(e) = std::fs::copy(&from_file, &to_file) {
            return self.io_error("Cannot copy asset", e);
        }
        let new_id = AssetUuid::new_v7();
        let set = self.with_domain(to.domain, |e, s| s.set_asset_uuid(e, &to, new_id));
        if let Err(e) = set.unwrap_or_else(|| super::kinds::set_asset_uuid(self, &to, new_id))
            && self.domain_list.backends().iter().any(|b| b.id() == to.domain)
        {
            let _ = std::fs::remove_file(&to_file);
            return self.io_error("Cannot assign UUID to copied asset", e);
        }
        self.current_folders.insert(to.domain, folder.to_string());
        self.info(format!("{verb} {} -> {}", r.source_rel(), to.source_rel()));
        self.rescan();
        self.open_ref(&to);
        if let Some(i) = self.asset_tab(&to) {
            self.with_domain(to.domain, |e, s| s.after_save(e, &[i]));
        } else {
            self.with_domain(to.domain, |e, s| s.after_save(e, &[]));
        }
        self.touch_all();
    }

    // ---- folders (design §6.3) ----

    /// `Action::NewFolderDialog`: the path of a new folder of `domain`'s Project tree (`None`: the active real
    /// domain's, else the first enabled real domain's), proposed in its selected folder.
    pub fn new_folder_dialog(&mut self, domain: Option<DomainId>) {
        let real = |d: &DomainId| *d != DomainId::PROJECT && self.domain_enabled(*d);
        let Some(domain) = domain
            .or_else(|| self.active_domain.filter(real))
            .or_else(|| self.domain_list.real_ids().find(real))
            .filter(real)
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
    /// ([`DomainState::focus_diagnostic`]); a diagnostic of no domain goes to every enabled domain. The asset index's
    /// `AE001` and `AE004` open their quick fix's dialog instead ([`Editor::uuid_fix_dialog`]).
    pub fn focus_diagnostic(&mut self, diag: &Diagnostic) {
        if diag.domain.is_some_and(|id| !self.domain_enabled(id)) {
            return;
        }
        if let Some(dialog) = self.uuid_fix_dialog(diag) {
            self.dialog = Some(dialog);
            return;
        }
        let ids: Vec<DomainId> = match diag.domain {
            Some(id) => vec![id],
            None => self.domains.ids().filter(|&id| self.domain_enabled(id)).collect(),
        };
        for id in ids {
            self.with_domain(id, |e, s| s.focus_diagnostic(e, diag));
        }
    }

    /// The quick-fix dialog of an index diagnostic of the current index (ADR 0047 §11, §16): `AE001` at a duplicate
    /// loser asks to regenerate its UUID, `AE004` to assign one; both offer Open when the asset has a kind to open
    /// it. `None` for any other diagnostic, or one the index no longer reports.
    pub fn uuid_fix_dialog(&self, diag: &Diagnostic) -> Option<Dialog> {
        let names = |r: &AssetRef| diag.domain == Some(r.domain) && diag.asset.as_deref() == Some(&*r.source_rel());
        let (title, message, label, r) = match diag.code.as_str() {
            "AE001" => {
                let (loser, winner) = self.asset_index.duplicates().find_map(|(_, files)| {
                    files[1..].iter().find(|r| names(r)).map(|loser| (loser.clone(), &files[0]))
                })?;
                let message = format!(
                    "`{loser}` has the same UUID as `{winner}` (a copied file?). References keep pointing at \
                     `{winner}`."
                );
                ("Duplicate asset UUID", message, "Regenerate UUID", loser)
            }
            "AE004" => {
                let file = self.asset_index.missing_ids().map(|(r, _)| r).find(|r| names(r))?.clone();
                let message = format!(
                    "Assign a UUID to `{file}`? The file is rewritten in the canonical format (comments are not kept)."
                );
                ("Missing asset UUID", message, "Assign UUID", file)
            }
            _ => return None,
        };
        let mut buttons = vec![("Cancel".to_string(), Action::CloseDialog)];
        if self.domain_list.asset_kind(r.domain, r.suffix).is_some() {
            buttons.push(("Open".into(), Action::OpenAsset(r.clone())));
        }
        let fix = if diag.code == "AE001" { Action::RegenerateUuid(r) } else { Action::AssignUuid(r) };
        buttons.push((label.into(), fix));
        Some(Dialog::Confirm { title: title.into(), message, buttons })
    }

    /// `Action::RegenerateUuid` / `Action::AssignUuid` (ADR 0047 §11, §16): like a rename, an unsaved document of `r`
    /// is saved first; its domain writes a fresh UUIDv7 into the file ([`DomainState::set_asset_uuid`]; a domain
    /// without a state: core's default, [`super::kinds::set_asset_uuid`]), the domains rescan (index included), open
    /// clean documents reload the file and the domain follows as after a save (Material regenerates).
    pub fn set_uuid(&mut self, r: &AssetRef, fix: UuidFix) {
        self.dialog = None;
        if self.project.is_none() {
            return;
        }
        let mut saved = Vec::new();
        if let Some(i) = self.asset_tab(r)
            && self.tabs[i].doc().is_dirty()
        {
            if !self.save(i) {
                return;
            }
            saved.push(i);
        }
        let id = AssetUuid::new_v7();
        let set = self.with_domain(r.domain, |e, s| s.set_asset_uuid(e, r, id));
        if let Err(e) = set.unwrap_or_else(|| super::kinds::set_asset_uuid(self, r, id)) {
            return self.io_error(&format!("Cannot {} {r}", fix.verb()), e);
        }
        self.info(match fix {
            UuidFix::Regenerate => format!("Regenerated the UUID of {r}"),
            UuidFix::Assign => format!("Assigned a UUID to {r}"),
        });
        self.rescan();
        self.follow_disk_changes();
        self.with_domain(r.domain, |e, s| s.after_save(e, &saved));
        self.touch_all();
    }

    /// Generate (Assets menu, design §6.5): every enabled domain regenerates from what is saved
    /// ([`DomainState::generate`]).
    pub fn generate(&mut self) {
        let ids: Vec<DomainId> = self.domains.ids().filter(|&id| self.domain_enabled(id)).collect();
        for id in ids {
            self.with_domain(id, |e, s| s.generate(e));
        }
    }
}

/// Which UUID quick fix [`Editor::set_uuid`] runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UuidFix {
    /// `AE001`: a duplicate loser gets a UUID of its own.
    Regenerate,
    /// `AE004`: a file without a valid id gets one.
    Assign,
}

impl UuidFix {
    /// How the error dialog names the fix ("Cannot <verb> <file>").
    fn verb(self) -> &'static str {
        match self {
            UuidFix::Regenerate => "regenerate the UUID of",
            UuidFix::Assign => "assign a UUID to",
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

fn copy_stem(leaf: &str) -> &str {
    if let Some(stem) = leaf.strip_suffix("_copy")
        && !stem.is_empty()
    {
        return stem;
    }
    if let Some((before, digits)) = leaf.rsplit_once("_copy_")
        && !before.is_empty()
        && !digits.is_empty()
        && digits.bytes().all(|b| b.is_ascii_digit())
    {
        return before;
    }
    leaf
}

fn unique_copy_ref(project: &Project, r: &AssetRef, folder: &str, keep_name_if_free: bool) -> AssetRef {
    let leaf = r.path.rsplit('/').next().unwrap_or(&r.path);
    let join = |name: &str| {
        let path = if folder.is_empty() { name.to_string() } else { format!("{folder}/{name}") };
        AssetRef { domain: r.domain, path, suffix: r.suffix }
    };
    if keep_name_if_free {
        let candidate = join(leaf);
        if !project.root.join(candidate.source_rel()).exists() {
            return candidate;
        }
    }
    let stem = copy_stem(leaf);
    let first = join(&format!("{stem}_copy"));
    if !project.root.join(first.source_rel()).exists() {
        return first;
    }
    let mut n = 2usize;
    loop {
        let candidate = join(&format!("{stem}_copy_{n}"));
        if !project.root.join(candidate.source_rel()).exists() {
            return candidate;
        }
        n += 1;
    }
}
