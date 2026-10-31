//! The Light domain in the editor (design §9.2): its registration ([`LightDomain`]), its state ([`LightState`]: the
//! loaded assets, the diagnostics, generation and the texture-reference follow-ups), the environment tab
//! ([`EnvironmentDoc`] edited through [`EnvironmentEditor`]'s form) and its cube-map Texture assets, which are
//! `texture_kit`'s texture editor under [`LightTexture`].
//!
//! Composition, not copies: Texture tabs, game image tabs, "Create Texture Asset" and the preview are the kit's;
//! tree rows, tab keys and the plain file rename / delete are core's asset-kind defaults (design §14.4).

mod curve;
mod environment;
mod form;
mod inspect;
pub mod preview;
pub mod preview_tool;
mod state;
mod texture;

use std::io;

use editor_core::domain::{AssetKindDef, AssetTemplate, DomainInfo, DomainRegistrar, EditorDomain, ToolDef};
use editor_core::menu::MainMenu;
use editor_core::ui::icons::IconRef;
use project_core::regen::DomainBackend;
use project_core::{AssetRef, DomainId, Project};

use crate::backend::{ENVIRONMENT_SUFFIX, Environment, LIGHT, LightBackend, save_environment};

pub use curve::{LIGHT_CURVE_VIEW, LightCurve, LightCurveDoc, LightCurveEdit, LightCurveEditor};
pub use environment::{
    CgSection, ENVIRONMENT_VIEW, EnvironmentAction, EnvironmentDoc, EnvironmentProp, apply_easy_defaults, fits_easy,
};
pub use form::{
    EnvironmentEditor, EnvironmentForm, EnvironmentResetIcon, EnvironmentSidebarBody, EnvironmentSwatch,
    EnvironmentViewBar, LightSwatchOf,
};
pub use inspect::{
    INSPECT_GAME_TOOL, INSPECT_GAME_VIEW, InspectGameBar, InspectGameBody, InspectGameDoc, InspectGameEditor,
};
pub use preview::{
    BlendedSkybox, BlendedSkyboxPlugin, LIGHT_PREVIEW_KIND, LightPreviewImage, LightPreviewSession,
    LightPreviewSessions, LightPreviewViewport, PreviewCubemapCache, PreviewSkyboxBlend, apply_environment_to_session,
    apply_environment_with_skybox_to_session, clean_light_preview, interpolate_environments, preview_callbacks,
    preview_viewport, start_light_preview,
};
pub use preview_tool::{
    LIGHT_PREVIEW_TOOL, LIGHT_PREVIEW_VIEW, LightPreviewBar, LightPreviewDoc, LightPreviewEditor, LightPreviewProp,
    LightPreviewSampleText, LightPreviewSidebarBody,
};
pub use state::LightState;
pub use texture::{LIGHT_IMAGE_VIEW, LIGHT_TEXTURE_VIEW, LightTexture};

/// The Light domain's icon (a sun).
pub const LIGHT_ICON: IconRef = IconRef::new(&icon_rs::THEME_LIGHT);

/// The icon of environments (Project tree, tabs, the "New Environment..." item): a globe, the sky around the scene.
pub const ENVIRONMENT_ICON: IconRef = IconRef::new(&icon_rs::PUBLIC);

/// Every icon of the domain, for the rasterize test.
pub const LIGHT_ICONS: &[IconRef] = &[LIGHT_ICON, ENVIRONMENT_ICON];

/// The Light domain (design §9.2): environments, transition curves, and cube-map Texture assets.
pub struct LightDomain;

impl EditorDomain for LightDomain {
    fn id(&self) -> DomainId {
        LIGHT
    }

    fn info(&self) -> DomainInfo {
        DomainInfo {
            title: "Light",
            icon: LIGHT_ICON,
            summary: "Light — environments, transition curves and cube-map textures in quillshade/light/.",
        }
    }

    fn backend(&self) -> Option<Box<dyn DomainBackend>> {
        Some(Box::new(LightBackend))
    }

    /// The asset kinds in the Assets menu's order: environments, then transition curves (`curve_kit`), then Texture
    /// assets (`texture_kit`, with its "New Texture..." template, the Light texture and image editor kinds and the
    /// `.ktx2` Game Assets).
    fn register(&self, r: &mut DomainRegistrar) {
        r.state(|| Box::new(LightState::default()))
            .preview(preview::preview_callbacks())
            .logic(preview::setup_preview_logic)
            .asset_kind(AssetKindDef {
                suffix: ENVIRONMENT_SUFFIX,
                title: "Environment",
                icon: ENVIRONMENT_ICON,
                templates: vec![AssetTemplate {
                    label: "Environment",
                    base_name: "environment",
                    icon: ENVIRONMENT_ICON,
                    create: create_environment,
                }],
                open: environment::open,
                row_menu: |_, _| Vec::new(),
            });
        curve_kit::editor::register::<LightCurve>(r);
        texture_kit::editor::register::<LightTexture>(r);
        r.editor_kind::<EnvironmentEditor>().doc_view(|| Box::new(form::environment_view()));
        r.tool(ToolDef {
            id: INSPECT_GAME_TOOL,
            title: "Inspect Game",
            open: inspect::open,
            menu: Some(MainMenu::Tools),
        })
        .editor_kind::<InspectGameEditor>()
        .doc_view(|| Box::new(inspect::inspect_view()));
        r.tool(ToolDef {
            id: LIGHT_PREVIEW_TOOL,
            title: "Light Transition Preview",
            open: preview_tool::open,
            menu: Some(MainMenu::Tools),
        })
        .editor_kind::<LightPreviewEditor>()
        .doc_view(|| Box::new(preview_tool::light_preview_view()));
    }
}

/// The "New Environment..." template: an environment with the default settings and no cube map.
fn create_environment(project: &Project, r: &AssetRef) -> io::Result<()> {
    project_core::ident::check_path(&r.path).map_err(io::Error::other)?;
    let file = project.root.join(r.source_rel());
    if file.exists() {
        return Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("`{}` already exists", r.source_rel())));
    }
    project.ensure_layout()?;
    save_environment(&file, &Environment::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn light_icons_rasterize() {
        editor_core::ui::icons::assert_icons_rasterize(LIGHT_ICONS);
    }

    #[test]
    fn domain_names_its_backend() {
        let d = LightDomain;
        assert_eq!(d.id(), LIGHT);
        assert_eq!(d.info().title, "Light");
        assert_eq!(d.backend().map(|b| b.id()), Some(LIGHT));
    }
}
