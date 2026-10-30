//! The curve editor (design §7.1, decisions 0015, 0038), shared by every domain with curve assets: the open curve
//! ([`CurveDoc`], with its undo history), its edits ([`CurveEdit`], [`CurveProp`], the curve clipboard), the plot
//! with its point handles and drags (the editor content), the right sidebar, the editor topbar and the input context
//! ([`CurveEditor`], [`CurveCtx`]).
//!
//! Composition, not copies: a domain implements [`CurveDomain`] on a zero-sized type and calls [`register`]. Every
//! type, system and observer the editor adds is generic over that type, so two domains each have their own curve
//! editor side by side; the domain folder decides which one a `*.curve.ron` file is (`quillshade/<domain>/…`).
//!
//! Depends on `editor_core` (and Bevy) only: never on a domain or a `shader_*` crate.

#![allow(clippy::type_complexity)] // Bevy system signatures

use std::io;

use bevy::prelude::Scene;
use editor_core::domain::{AssetKindDef, AssetTemplate, DomainRegistrar};
use editor_core::menu::MenuEntry;
use editor_core::state::editor::Editor;
use editor_core::tab::{Tab, ViewKindId};
use editor_core::ui::icons::IconRef;
use project_core::{AssetRef, Diagnostic, DomainId, Project};

use crate::Curve;

/// A marker component of `D`'s curve editor.
macro_rules! marker {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[derive(::bevy::prelude::Component)]
        pub struct $name<D>(::std::marker::PhantomData<fn() -> D>);

        impl<D> Default for $name<D> {
            fn default() -> Self {
                $name(::std::marker::PhantomData)
            }
        }

        impl<D> Clone for $name<D> {
            fn clone(&self) -> Self {
                $name::default()
            }
        }
    };
}

mod doc;
mod input;
mod ops;
mod sidebar;
mod view;

pub use doc::CurveDoc;
pub use input::{CurveCtx, CurveEditor, Nudge};
pub use ops::{CurveAction, CurveClipboard, CurveEdit, CurveProp, PASTE_STEP, curve_key};
pub use view::{CURVE_COLOR, CurveCanvas, CurveEditorPlugin, CurveHandle, CurvePlotImage, CurveViewBar, curve_view};

/// The icon of curve assets (secondbar, Project tree, "New Curve...").
pub const CURVE_ICON: IconRef = IconRef::new(&icon_rs::SPLINE);

/// A domain with curve assets (design §7.1): one zero-sized type per domain (Material: `MaterialCurve`). Its files
/// are `quillshade/<DOMAIN>/**/*.<SUFFIX>.ron`; its tabs show in the view [`CurveDomain::VIEW`].
pub trait CurveDomain: Sized + Send + Sync + 'static {
    const DOMAIN: DomainId;
    const SUFFIX: &'static str = "curve";
    /// The curve editor kind's view (`ViewKindId("material.curve")`): distinct per domain.
    const VIEW: ViewKindId;

    /// Scenes after the editor topbar's own tools (Material: "Show generated WESL"), rebuilt with the bar.
    fn topbar_extras(_doc: &CurveDoc<Self>) -> Vec<Box<dyn Scene>> {
        Vec::new()
    }

    /// Subpanes of the right sidebar between the selection's and the hotkeys, rebuilt with it.
    fn sidebar_extras(_doc: &CurveDoc<Self>) -> Vec<Box<dyn Scene>> {
        Vec::new()
    }

    /// Problems of a curve the editor statusbar counts besides the domain's own diagnostics of the asset.
    fn validate(_curve: &Curve) -> Vec<Diagnostic> {
        Vec::new()
    }

    /// Items the Project tree's row menu of a curve adds after Open / Rename / Move / Delete (Material: its generated
    /// WESL and its source).
    fn row_menu(_editor: &Editor, _r: &AssetRef) -> Vec<MenuEntry> {
        Vec::new()
    }
}

/// Adds `D`'s curves to its domain (design §7.1): the asset kind with its "Curve" template, the curve editor kind
/// (editor topbar, right sidebar, input context) and its view with the window systems ([`CurveEditorPlugin`]).
/// Core's defaults of an asset kind cover the rest (design §14.4): a curve tab's key is its source path, the Project
/// tree lists the files, rename / delete move / remove them, unless the domain overrides them.
pub fn register<D: CurveDomain>(r: &mut DomainRegistrar) {
    r.asset_kind(AssetKindDef {
        suffix: D::SUFFIX,
        title: "Curve",
        icon: CURVE_ICON,
        templates: vec![AssetTemplate { label: "Curve", base_name: "new_curve", icon: CURVE_ICON, create }],
        open: open::<D>,
        row_menu: D::row_menu,
    })
    .editor_kind::<CurveEditor<D>>()
    .bevy(|app| {
        app.add_plugins(CurveEditorPlugin::<D>::default());
    })
    .doc_view(|| Box::new(curve_view::<D>()));
}

/// The active tab's curve of `D` (`None` while another tab, or another domain's curve, is active).
pub fn active<D: CurveDomain>(editor: &Editor) -> Option<&CurveDoc<D>> {
    editor.active_doc()
}

pub fn active_mut<D: CurveDomain>(editor: &mut Editor) -> Option<&mut CurveDoc<D>> {
    editor.active_doc_mut()
}

/// The tab of `D`'s curve `r`.
pub fn index<D: CurveDomain>(editor: &Editor, r: &AssetRef) -> Option<usize> {
    editor.tabs.iter().position(|t| t.get::<CurveDoc<D>>().is_some_and(|c| c.aref == *r))
}

/// The curve asset kind's `open`: `D`'s curve `r` loaded from disk; `None` with an error dialog when it does not
/// load.
pub fn open<D: CurveDomain>(e: &mut Editor, r: &AssetRef) -> Option<Tab> {
    let project = e.project.as_ref()?;
    match load(project, r) {
        Ok(curve) => Some(Tab::new(D::DOMAIN, CurveDoc::<D>::new(r.clone(), curve))),
        Err(err) => {
            e.io_error(&format!("cannot open {}", r.source_rel()), err);
            None
        }
    }
}

/// The file of asset `r`.
fn file(project: &Project, r: &AssetRef) -> std::path::PathBuf {
    project.root.join(r.source_rel())
}

/// Loads a curve file; its name is the file name (the `name` stored inside does not count).
pub fn load(project: &Project, r: &AssetRef) -> Result<Curve, String> {
    let text = std::fs::read_to_string(file(project, r)).map_err(|e| e.to_string())?;
    let mut curve = Curve::from_ron(&text).map_err(|e| e.to_string())?;
    curve.name = name_of(r).to_string();
    Ok(curve)
}

/// Writes a curve file (creating `quillshade/<domain>/` and the parent directories), through a temporary file.
pub fn save(project: &Project, r: &AssetRef, curve: &Curve) -> io::Result<()> {
    project.ensure_layout()?;
    let file = file(project, r);
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = file.with_extension("ron.tmp");
    std::fs::write(&tmp, curve.to_ron())?;
    std::fs::rename(&tmp, &file)
}

/// The "Curve" template: a straight ramp from (0, 0) to (1, 1) at `r`, which must not exist yet.
fn create(project: &Project, r: &AssetRef) -> io::Result<()> {
    project_core::ident::check_path(&r.path).map_err(io::Error::other)?;
    if file(project, r).exists() {
        return Err(io::Error::other(format!("`{}` already exists", r.source_rel())));
    }
    save(project, r, &Curve::ramp(name_of(r)))
}

/// The asset's name: the last component of its path.
fn name_of(r: &AssetRef) -> &str {
    r.path.rsplit('/').next().unwrap_or(&r.path)
}
