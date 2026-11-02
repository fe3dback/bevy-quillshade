//! Core's defaults for the domains' asset kinds (design §14.4): what every kind registered with
//! `DomainRegistrar::asset_kind` gets without code of its domain — its files as Project tree rows, its source path as a
//! tab key, renaming / moving and deleting its files, giving a file a new asset UUID. A domain overrides each through
//! its hooks (Material does: `DomainState::tree_items`, `rename_file`, `delete_file`, `set_asset_uuid`, its own
//! `parse_tab`).

use std::collections::BTreeSet;

use project_core::asset_index::rewrite_root_id;
use project_core::{AssetRef, AssetUuid, Diagnostic, DomainId, Project};

use super::editor::Editor;
use crate::domain::{DomainList, TreeAsset};
use crate::tab::TabKey;
use crate::ui::icons::{IconId, IconRef};

/// The files of `domain`'s registered asset kinds (`quillshade/<domain>/**/*.<suffix>.ron`, hidden folders skipped),
/// by kind in registration order, then by path.
pub fn scan(project: &Project, list: &DomainList, domain: DomainId) -> Vec<AssetRef> {
    let suffixes: Vec<&'static str> = list.asset_kinds(domain).iter().map(|k| k.suffix).collect();
    if suffixes.is_empty() {
        return Vec::new();
    }
    let dir = project.domain_dir(domain);
    let visible = |f: &std::path::Path| {
        let rel = f.strip_prefix(&dir).unwrap_or(f);
        !rel.components().any(|c| c.as_os_str().to_string_lossy().starts_with('.'))
    };
    let files = project_core::files::walk_files(&dir).into_iter().filter(|f| visible(f));
    let mut found: Vec<AssetRef> =
        files.filter_map(|f| project_core::asset_ref::parse(&project.rel(&f), domain, &suffixes)).collect();
    let rank = |r: &AssetRef| suffixes.iter().position(|s| *s == r.suffix);
    found.sort_by(|a, b| (rank(a), &a.path).cmp(&(rank(b), &b.path)));
    found
}

/// The default Project tree rows of `domain` ([`crate::domain::DomainState::tree_items`] returned `None`): the files of
/// its registered kinds as of the last rescan, each with its kind's icon, in the error colour while the domain has an
/// error against it ([`crate::domain::DomainState::diagnostics`]).
pub fn default_tree_items(editor: &Editor, domain: DomainId) -> Vec<TreeAsset> {
    let diagnostics = editor.domains.get_dyn(domain).map(|s| s.diagnostics()).unwrap_or_default();
    let errors: BTreeSet<String> =
        diagnostics.into_iter().filter(Diagnostic::is_error).filter_map(|d| d.asset).collect();
    let files = editor.kind_files.get(&domain).into_iter().flatten();
    files
        .map(|r| TreeAsset {
            asset: r.clone(),
            icon: editor.domain_list.asset_kind(domain, r.suffix).map_or(IconRef::from(IconId::File), |k| k.icon),
            error: errors.contains(&r.source_rel()),
        })
        .collect()
}

/// The asset a tab key names by core's default (design §14.4): a key equal to the source path of an asset of one of
/// `domain`'s registered kinds (`quillshade/<domain>/<path>.<suffix>.ron`).
pub fn kind_ref(list: &DomainList, domain: DomainId, key: &str) -> Option<AssetRef> {
    let suffixes: Vec<&'static str> = list.asset_kinds(domain).iter().map(|k| k.suffix).collect();
    project_core::asset_ref::parse(key, domain, &suffixes).filter(|r| r.source_rel() == key)
}

/// Whether `key` names an existing file of a registered kind of its domain ([`kind_ref`]).
pub fn kind_file_exists(editor: &Editor, key: &TabKey) -> bool {
    let (Some(project), Some(r)) = (&editor.project, kind_ref(&editor.domain_list, key.domain, &key.key)) else {
        return false;
    };
    project.root.join(r.source_rel()).is_file()
}

/// The default [`crate::domain::DomainState::rename_file`]: moves `r`'s file to the asset path `new_path` of the same
/// domain and kind, creating its folders. Refused for a path that is no project-wide identifier path or a file that
/// exists.
pub fn rename_file(editor: &mut Editor, r: &AssetRef, new_path: &str) -> Result<AssetRef, String> {
    let project = editor.project.as_ref().ok_or("no project is open")?;
    project_core::ident::check_path(new_path)?;
    let to = AssetRef { path: new_path.to_string(), ..r.clone() };
    if to == *r {
        return Ok(to);
    }
    let (from_file, to_file) = (project.root.join(r.source_rel()), project.root.join(to.source_rel()));
    if to_file.exists() {
        return Err(format!("`{}` already exists", to.source_rel()));
    }
    if let Some(dir) = to_file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::rename(from_file, to_file).map_err(|e| e.to_string())?;
    Ok(to)
}

/// The default [`crate::domain::DomainState::delete_file`]: removes `r`'s file.
pub fn delete_file(editor: &mut Editor, r: &AssetRef) -> Result<(), String> {
    let project = editor.project.as_ref().ok_or("no project is open")?;
    std::fs::remove_file(project.root.join(r.source_rel())).map_err(|e| e.to_string())
}

/// The default [`crate::domain::DomainState::set_asset_uuid`] (ADR 0047 §16): replaces the UUID of the root `id` line of
/// `r`'s file in place ([`rewrite_root_id`]), everything else byte for byte. Refused, with the file untouched, when it
/// has no such line.
pub fn set_asset_uuid(editor: &mut Editor, r: &AssetRef, id: AssetUuid) -> Result<(), String> {
    let project = editor.project.as_ref().ok_or("no project is open")?;
    let file = project.root.join(r.source_rel());
    let text = std::fs::read_to_string(&file).map_err(|e| e.to_string())?;
    let text = rewrite_root_id(&text, id)
        .ok_or_else(|| format!("`{r}` has no `id` line to rewrite; open and save it in the editor to assign one"))?;
    project_core::writer::write_atomic(&file, text).map_err(|e| e.to_string())
}
