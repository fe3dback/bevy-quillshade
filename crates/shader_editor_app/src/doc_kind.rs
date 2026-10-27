//! What every tab's content offers the editor, whatever its kind (decision 0014): identity, title, icon, dirty
//! state, saving, reloading after external changes, following a rename and the key it is remembered by in the user
//! config (decision 0018). [`crate::tab::Tab::doc`] gives it for any tab, so code that only needs one of these
//! operations does not match on the tab kind; kind-specific work (graph editing, curve edits) still does. The impls
//! live with the rest of their kind in [`crate::editors`].

use shader_project::{AssetId, AssetSet, Project};

use crate::editors::{StatusCtx, StatusInfo};
use crate::tab::ViewKind;
use crate::ui::icons::IconId;

pub trait TabDoc {
    /// The asset this document edits. `None` for read-only views of something that is not an asset (a generated
    /// file, a game image).
    fn asset(&self) -> Option<&AssetId>;
    fn view(&self) -> ViewKind;
    /// Full title without decorations (`shaders/env/rock_wet.material`, `assets/shaders/autogen/...wesl`).
    fn base_title(&self) -> String;
    /// Tab bar icon.
    fn icon(&self) -> IconId;
    /// Watermark in the editor content's corner: the asset type or the language of a generated file; `None` shows
    /// none.
    fn watermark(&self) -> Option<&'static str>;
    fn is_dirty(&self) -> bool;
    fn is_read_only(&self) -> bool;
    /// Bumped whenever the content the view shows changes.
    fn revision(&self) -> u64;
    /// Writes the document and marks it saved; a no-op for read-only documents.
    fn save(&mut self, project: &Project) -> std::io::Result<()>;
    /// Whether the file differs from what the document last saved or loaded (always `false` for read-only
    /// documents: they are re-read on activation and after regeneration instead).
    fn changed_on_disk(&self, project: &Project) -> bool;
    /// Replaces the content with the file's (history is cleared); `Err` when it does not load.
    fn reload_from_disk(&mut self, project: &Project) -> Result<(), String>;
    /// "Keep mine" after an external change: the file's version becomes the saved baseline, so the edits stay and
    /// the document stays dirty. Nothing happens when the file does not load; a no-op for read-only documents.
    fn keep_mine(&mut self, project: &Project);
    /// Follows the asset `from` renamed to `to` (already moved on disk); ignored by documents of other assets.
    fn rename(&mut self, project: &Project, from: &AssetId, to: &AssetId);
    /// Key in the user config's open tabs; parsed back by [`crate::tab::parse_config_key`].
    fn config_key(&self) -> Option<String>;
    /// Puts the unsaved content into `set` in place of the file's (what live validation and reference lookups
    /// see); nothing for read-only documents.
    fn override_into(&self, _set: &mut AssetSet) {}
    /// Content of the editor statusbar (design §2.3); `None` (the default) hides it.
    fn status(&self, _ctx: &StatusCtx) -> Option<StatusInfo> {
        None
    }
}

/// Title of an asset's document: its source path without `.ron`.
pub(crate) fn asset_title(id: &AssetId) -> String {
    id.source_rel().trim_end_matches(".ron").to_string()
}

/// A failed asset operation as an I/O error (same message).
pub(crate) fn io_error(e: shader_project::ops::OpError) -> std::io::Error {
    match e {
        shader_project::ops::OpError::Io(e) => e,
        e => std::io::Error::other(e),
    }
}
