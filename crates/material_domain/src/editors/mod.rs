//! Material's editor kinds (design §2.1, decision 0014): one module per kind of tab content — graph, consts,
//! gradients — holding every trait impl of that kind ([`crate::doc_kind::TabDoc`], the kind's [`EditorAreas`] and
//! [`EditorInput`]) and its [`ViewKindId`] (`graph::GRAPH`, …). The document types themselves stay in their own files
//! (`doc.rs`, `consts_doc.rs`, …). The text kind is `editor_core`'s, the curve kind `curve_kit`'s ([`crate::curve`]),
//! the image and Texture kinds `texture_kit`'s ([`crate::texture`]).
//!
//! The traits and the kind list's types are in `editor_core::editors` (design §6.2); Material registers its kinds
//! ([`crate::domain::MaterialDomain`]) and `editor_core::EditorCorePlugin` puts the domains' kinds in the
//! [`EditorKinds`] resource the window is built from.

pub use editor_core::doc_kind::{StatusCtx, StatusInfo};
pub use editor_core::editors::{AreaOf, EditorAreas, EditorInput, EditorKinds, KindDef, ViewOf, bar_row, sidebar_body};
pub use editor_core::tab::ViewKindId;

pub use editor_core::editors::{SidebarKey, fill_sidebar_with};
pub use editor_core::main_input;

pub mod consts;
pub mod gradients;
pub mod graph;

pub use crate::curve::{CurveCtx, CurveEditor};
pub use crate::texture::{ImageEditor, TextureEditor};
pub use consts::ConstsEditor;
pub use editor_core::editors::text::{self, TextCtx, TextEditor};
pub use gradients::GradientsEditor;
pub use graph::{GraphCtx, GraphEditor};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidebar_title_follows_the_active_kind() {
        let kinds = crate::domain::material_only().kind_defs();
        assert_eq!(kinds.sidebar_title(None), "Properties");
        let titles = [
            (graph::GRAPH, "Properties"),
            (text::TEXT, "Generated File"),
            (crate::curve::CURVE, "Curve"),
            (consts::CONSTS, "Constants"),
            (crate::texture::IMAGE, "Image"),
            (crate::texture::TEXTURE, "Texture"),
            (gradients::GRADIENTS, "Gradients"),
            // The built-in project domain's.
            (editor_core::settings::SETTINGS_VIEW, "Project Settings"),
        ];
        for (view, title) in titles {
            assert_eq!(kinds.sidebar_title(Some(view)), title, "{view:?}");
        }
        assert_eq!(kinds.0.len(), titles.len(), "one entry per kind");
    }
}
