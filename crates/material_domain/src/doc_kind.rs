//! Tab documents (decision 0014): [`TabDoc`] and its contexts are core (`editor_core::doc_kind`); the impls live with
//! the rest of their kind in [`crate::editors`] (Texture and image tabs: `texture_kit`). Here: helpers of Material's
//! impls.

pub use editor_core::doc_kind::{DocCtx, StatusCtx, StatusInfo, TabDoc};
use shader_project::{AssetId, Project};

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

/// The project of a save / reload; an error when none is open (the editor saves only with one).
pub(crate) fn project_of<'a>(ctx: &DocCtx<'a>) -> std::io::Result<&'a Project> {
    ctx.project.ok_or_else(|| std::io::Error::other("no project is open"))
}
