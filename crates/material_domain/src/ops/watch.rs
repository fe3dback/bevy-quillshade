use super::*;

/// Material's reaction to external changes.
pub trait MaterialWatch {
    /// Files under `quillshade/` changed outside the editor ([`MaterialState`]'s `files_changed` hook, after the
    /// rescan): a change of the user nodes (`quillshade/material/.nodes/`) reloads the registry, and the graphs redraw.
    fn material_files_changed(&mut self, changed: &[PathBuf]);
}

impl MaterialWatch for Editor {
    // ---- external changes ----

    fn material_files_changed(&mut self, changed: &[PathBuf]) {
        let Some(project) = &self.project else { return };
        let user_nodes_dir = project.material_dir().join(USER_NODES_DIR);
        let user_nodes = changed.iter().any(|p| p.starts_with(&user_nodes_dir));
        if user_nodes {
            let (r, d) = project_registry(project);
            self.domains.material_mut().registry = r;
            self.domains.material_mut().project_diags = d;
            for doc in self.tabs.iter_mut().filter_map(Tab::get_mut::<Doc>) {
                doc.revision += 1;
            }
        }
    }
}
