//! The Light domain (design §9.2): environments (a cube map and its brightness) in `quillshade/light/`, their
//! cube-map Texture assets through `texture_kit`, and the `light` module of the umbrella crate.
//!
//! [`backend`] is Bevy-free (the CLI); with the `editor` feature (default), [`editor`] is the domain the editor
//! registers ([`LightDomain`]). Depends on `editor_core` and `texture_kit` only: never on another domain or a
//! `shader_*` crate.

pub mod backend;

#[cfg(feature = "editor")]
pub mod editor;

#[cfg(feature = "editor")]
pub use editor::{
    BlendedSkybox, BlendedSkyboxPlugin, CgSection, ENVIRONMENT_ICON, EnvironmentAction, EnvironmentDoc,
    EnvironmentEditor, EnvironmentProp, EnvironmentResetIcon, EnvironmentSwatch, HDR_CONVERTER_TOOL,
    HDR_CONVERTER_VIEW, Hdr2dPreviewBox, Hdr2dPreviewImage, HdrConverterAction, HdrConverterBar, HdrConverterDoc,
    HdrConverterEditor, HdrConverterPreviewCache, HdrConverterProp, HdrConverterSidebarBody, HdrConverterSummaryText,
    HdrCubePreviewImage, HdrCubePreviewViewport, HdrCubeStripImage, HdrLeftPane, HdrRightPane, INSPECT_GAME_TOOL,
    INSPECT_GAME_VIEW, InspectGameBar, InspectGameBody, InspectGameDoc, InspectGameEditor, LIGHT_CURVE_VIEW,
    LIGHT_ICON, LIGHT_ICONS, LIGHT_PREVIEW_KIND, LIGHT_PREVIEW_TOOL, LIGHT_PREVIEW_VIEW, LightCurve, LightCurveDoc,
    LightCurveEdit, LightCurveEditor, LightDomain, LightPreviewBar, LightPreviewDoc, LightPreviewEditor,
    LightPreviewImage, LightPreviewProp, LightPreviewSampleText, LightPreviewSession, LightPreviewSessions,
    LightPreviewSidebarBody, LightPreviewViewport, LightState, LightSwatchOf, LightTexture, PreviewCubemapCache,
    PreviewSkyboxBlend, apply_easy_defaults, apply_environment_to_session, apply_environment_with_skybox_to_session,
    clean_light_preview, demo_sky_hdr, fits_easy, hdr_converter_view, interpolate_environments, preview_callbacks,
    preview_viewport, start_light_preview, update_hdr_converter_preview,
};
