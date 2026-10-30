//! The Texture and image tabs' edits (design §4.4, §6.2): the body of [`TabDoc::apply`] / [`TabDoc::current_value`] of
//! [`TextureDoc`] (and the preview's of [`super::ImageDoc`]), their kind-specific actions ([`TextureAction`] sent as
//! [`TextureEdit`], [`PreviewChannel`]), property keys ([`TextureProp`], keyed as `KitProp<D, TextureProp>`), all
//! carrying the domain (design §14.4), and the domain command [`CreateTextureForFile`]. A path change
//! and undo / redo depend on the other Texture assets (one file — one asset), so the document defers them to the
//! kit's step handler ([`steps`]), which sees the editor.
//!
//! [`TabDoc::apply`]: editor_core::doc_kind::TabDoc::apply
//! [`TabDoc::current_value`]: editor_core::doc_kind::TabDoc::current_value

use std::marker::PhantomData;

use editor_core::action::{Action, Handled, TabAction};
use editor_core::doc_kind::DocCtx;
use editor_core::dynval::{KitProp, Payload, PropKey, PropValue};
use editor_core::history::MergeKey;
use editor_core::state::editor::Editor;
use project_core::AssetRef;

use super::{Channel, PreviewDoc, TextureDoc, TextureDomain, active, active_mut, create_for_file, texture_owner};
use super::{FilterPreset, TextureField, fits_easy};
use crate::{TextureAsset, TextureSettings};

/// What a property widget of a Texture or image tab edits; widgets key it as `KitProp<D, TextureProp>`
/// ([`texture_key`]), so only `D`'s tabs take it (design §14.4).
#[derive(Debug, Clone, PartialEq)]
pub enum TextureProp {
    /// A settings field of the active Texture tab (design §4.4).
    Field(TextureField),
    /// The active image or Texture tab's preview repeats the image 3x3 (editor-only, design §6.2).
    PreviewTiled,
}

/// Edits of the active Texture tab (design §4.4); ignored on a built-in (read-only) texture.
#[derive(Debug, Clone, PartialEq)]
pub enum TextureAction {
    /// Points the asset at another image under `assets/`. Refused (log + dialog, model unchanged) when the path
    /// breaks the E012 rules of the domain's policy or another Texture asset already claims the file (on disk or in
    /// an open tab).
    SetPath(String),
    /// Replaces the settings (one undo step).
    SetSettings(TextureSettings),
    /// Applies an Easy mode preset's sampler (one undo step; nothing when it already matches).
    Preset(FilterPreset),
    /// Easy (presets) or Advanced (every sampler field) form; editor-only, not undoable. Refused for a domain without
    /// presets ([`crate::TexturePolicy::presets`]).
    SetEasy(bool),
    /// Undo (`undo`) or redo in the tab; refused like [`TextureAction::SetPath`] when the step would point the
    /// asset at a file another Texture asset claims.
    History { undo: bool },
}

/// What `D`'s image or Texture tab's preview shows: only channel 0 / 1 / 2 (R / G / B) in grey, or (`None`) the
/// image; editor-only, also on a built-in texture (design §6.2). A channel past B shows the image. Only a tab of `D`
/// applies it (design §14.4).
pub struct PreviewChannel<D>(pub Channel, PhantomData<fn() -> D>);

/// A [`TextureAction`] for `D`'s Texture tab (design §14.4): only a [`TextureDoc`] of `D` applies it, so an edit
/// queued for another domain's Texture tab is dropped when it reaches this one. Every sender in the kit builds this
/// form.
pub struct TextureEdit<D>(pub TextureAction, PhantomData<fn() -> D>);

/// Creates a Texture asset for a game image (path relative to `assets/`, design §4.3): `Action::Domain` of the
/// domain whose Game Assets menu or image tab offered it ([`super::create_for_file`]).
#[derive(Debug, Clone, PartialEq)]
pub struct CreateTextureForFile(pub String);

/// Texture edits of `D` reach the active tab as [`TabAction::Doc`] (design §6.2), compared by value.
macro_rules! doc_edit {
    ($($t:ident($inner:ty)),*) => {$(
        impl<D> $t<D> {
            pub fn new(inner: $inner) -> $t<D> {
                $t(inner, PhantomData)
            }
        }
        impl<D> Clone for $t<D> {
            fn clone(&self) -> Self {
                $t::new(self.0.clone())
            }
        }
        impl<D> PartialEq for $t<D> {
            fn eq(&self, other: &Self) -> bool {
                self.0 == other.0
            }
        }
        impl<D> std::fmt::Debug for $t<D> {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}({:?}) for {}", stringify!($t), self.0, std::any::type_name::<D>())
            }
        }
        impl<D: 'static> From<$t<D>> for TabAction {
            fn from(a: $t<D>) -> TabAction {
                TabAction::doc(a)
            }
        }
        impl<D: 'static> From<$t<D>> for Action {
            fn from(a: $t<D>) -> Action {
                Action::Tab(TabAction::doc(a))
            }
        }
    )*};
}
doc_edit!(TextureEdit(TextureAction), PreviewChannel(Channel));

/// The property key of `target` on `D`'s Texture or image tab (design §14.4).
pub fn texture_key<D: 'static>(target: TextureProp) -> PropKey {
    PropKey::new(KitProp::<D, _>::new(target))
}

/// `key` as `D`'s [`TextureProp`]; `None` for another domain's or another kind's key.
pub(super) fn prop_of_key<D: 'static>(key: &PropKey) -> Option<&TextureProp> {
    key.downcast::<KitProp<D, TextureProp>>().map(|k| &k.0)
}

impl<D: TextureDomain> TextureDoc<D> {
    pub(super) fn apply_edit(&mut self, ctx: &mut DocCtx, action: &TabAction) -> Handled {
        match action {
            TabAction::Undo => ctx.defer(TextureEdit::<D>::new(TextureAction::History { undo: true })),
            TabAction::Redo => ctx.defer(TextureEdit::<D>::new(TextureAction::History { undo: false })),
            TabAction::CommitInline => {
                self.end_merge();
                ctx.redraw_props = true;
            }
            TabAction::SetValue { key, value, merge } => {
                if let Some(TextureProp::PreviewTiled) = prop_of_key::<D>(key) {
                    return preview_edit::<D>(self, action);
                }
                let Some(TextureProp::Field(f)) = prop_of_key::<D>(key) else { return Handled::No };
                ctx.redraw_props |= self.set_field(key, *f, value, *merge);
            }
            TabAction::SetComponent { key, comp, value, is_final } => {
                let Some(TextureProp::Field(f)) = prop_of_key::<D>(key) else { return Handled::No };
                // `None`: a field sent its shown (rounded) text back on focus loss, which is not an edit.
                let Some(new) = f.get(&self.asset.settings).with_component(*comp, *value) else {
                    return Handled::Yes;
                };
                ctx.redraw_props |= self.set_field(key, *f, &new, true);
                if *is_final {
                    self.end_merge();
                }
            }
            TabAction::Doc(p) if p.downcast::<PreviewChannel<D>>().is_some() => return preview_edit::<D>(self, action),
            TabAction::Doc(p) => {
                let Some(TextureEdit(a, _)) = p.downcast::<TextureEdit<D>>() else { return Handled::No };
                match a {
                    TextureAction::SetPath(_) | TextureAction::History { .. } => {
                        ctx.defer(TextureEdit::<D>::new(a.clone()))
                    }
                    _ => ctx.redraw_props |= self.texture_action(a),
                }
            }
            _ => return Handled::No,
        }
        Handled::Yes
    }

    /// Value a property widget shows.
    pub fn prop_of(&self, target: &TextureProp) -> Option<PropValue> {
        match target {
            TextureProp::Field(f) => Some(f.get(&self.asset.settings)),
            TextureProp::PreviewTiled => Some(PropValue::Bool(self.tiled)),
        }
    }

    /// Sets a settings field; a `merge`d edit (a dragged number field) is one step and keeps the form. Returns whether
    /// the right sidebar must redraw.
    fn set_field(&mut self, key: &PropKey, f: TextureField, value: &PropValue, merge: bool) -> bool {
        if f.get(&self.asset.settings) == *value {
            return false;
        }
        let settings = f.set(&self.asset.settings, value);
        if merge {
            self.set_settings(settings, Some(MergeKey::Prop(key.clone())));
            return false;
        }
        self.texture_action(&TextureAction::SetSettings(settings))
    }

    /// The edits that need nothing but the document (ignored on a built-in texture); returns whether the right
    /// sidebar must redraw.
    fn texture_action(&mut self, action: &TextureAction) -> bool {
        if self.is_internal() {
            return false;
        }
        match action {
            TextureAction::Preset(p) => {
                let settings = p.apply(&self.asset.settings);
                self.set_settings(settings, None);
            }
            TextureAction::SetEasy(easy) => {
                // Easy shows only the presets: refused while the sampler is none of them (the button is disabled,
                // design §6.3), and in a domain without presets. Switching never rewrites the settings.
                if *easy && !(D::POLICY.presets && fits_easy(&self.asset.settings)) {
                    return false;
                }
                self.easy = *easy;
                self.relayout();
            }
            TextureAction::SetSettings(settings) => self.set_settings(settings.clone(), None),
            TextureAction::SetPath(_) | TextureAction::History { .. } => return false,
        }
        true
    }
}

/// The preview's view state on a tab with the shared image preview (image, Texture; editor-only, design §6.2):
/// [`TextureProp::PreviewTiled`] and [`PreviewChannel`] of `D`.
pub(super) fn preview_edit<D: 'static>(p: &mut dyn PreviewDoc, action: &TabAction) -> Handled {
    match action {
        TabAction::SetValue { key, value, .. } if prop_of_key::<D>(key) == Some(&TextureProp::PreviewTiled) => {
            if p.tiled() != value.as_bool() {
                p.set_tiled(value.as_bool());
            }
            Handled::Yes
        }
        TabAction::Doc(payload) => match payload.downcast::<PreviewChannel<D>>() {
            Some(PreviewChannel(c, _)) => {
                p.set_channel(c.filter(|c| *c < 3));
                Handled::Yes
            }
            None => Handled::No,
        },
        _ => Handled::No,
    }
}

/// `D`'s step handler (`DomainRegistrar::steps`): the Texture tab's deferred path edits and undo / redo, and
/// [`CreateTextureForFile`].
pub(super) fn steps<D: TextureDomain>(e: &mut Editor, payload: &Payload) -> Handled {
    if let Some(CreateTextureForFile(path)) = payload.downcast() {
        create_for_file::<D>(e, path);
        return Handled::Yes;
    }
    // A step deferred by `D`'s Texture tab, which is the active one.
    match payload.downcast::<TextureEdit<D>>().map(|edit| &edit.0) {
        Some(_) if active::<D>(e).is_none() => Handled::No,
        Some(TextureAction::SetPath(path)) => {
            texture_path::<D>(e, path.clone());
            Handled::Yes
        }
        Some(TextureAction::History { undo }) => {
            texture_history::<D>(e, *undo);
            Handled::Yes
        }
        _ => Handled::No,
    }
}

/// Points the active Texture asset at another image under `assets/`. Refused (log + dialog, model unchanged) when
/// the path breaks the E012 rules or another Texture asset already claims the file (on disk or in an open tab).
fn texture_path<D: TextureDomain>(e: &mut Editor, path: String) {
    let Some(t) = active::<D>(e).filter(|t| !t.is_internal()) else { return };
    let (r, path) = (t.aref.clone(), path.trim().to_string());
    if path == t.asset.path {
        // The field may show surrounding spaces the model never had.
        if let Some(t) = active_mut::<D>(e) {
            t.relayout();
        }
        return;
    }
    match path_problem::<D>(e, &r, &path) {
        Some(why) => refuse::<D>(e, "Cannot change the texture path", &why),
        None => {
            if let Some(t) = active_mut::<D>(e) {
                t.set_path(&path);
            }
        }
    }
    e.ui.props += 1;
}

/// Undo / redo in the active Texture tab; refused (like a path edit) when the step would point the asset at a file
/// another Texture asset now points at (design §1.1).
fn texture_history<D: TextureDomain>(e: &mut Editor, undo: bool) {
    let Some(t) = active::<D>(e).filter(|t| !t.is_internal()) else { return };
    let target = if undo { t.peek_undo() } else { t.peek_redo() };
    let Some(target) = target else { return };
    if target.path != t.asset.path
        && let Some(why) = path_problem::<D>(e, &t.aref, &target.path)
    {
        return refuse::<D>(e, if undo { "Cannot undo" } else { "Cannot redo" }, &why);
    }
    if let Some(t) = active_mut::<D>(e) {
        if undo {
            t.undo();
        } else {
            t.redo();
        }
    }
    e.ui.props += 1;
}

/// Why `D`'s Texture asset `r` may not point at the game file `path`: the E012 rules of `D`'s policy, or another
/// Texture asset already claims it on disk or in an open tab ([`texture_owner`]; E011 would follow, design §1.1).
pub fn path_problem<D: TextureDomain>(e: &Editor, r: &AssetRef, path: &str) -> Option<String> {
    if let Err(err) = TextureAsset::check_path(path, &D::POLICY) {
        return Some(err);
    }
    let other = texture_owner::<D>(e, path, Some(r))?;
    Some(format!("`{path}` already has a Texture asset: {}", other.source_rel()))
}

/// Reports a refused Texture edit (log + dialog); the form shows the unchanged model again.
fn refuse<D: TextureDomain>(e: &mut Editor, what: &str, why: &str) {
    e.io_error(what, why);
    if let Some(t) = active_mut::<D>(e) {
        t.relayout();
    }
    e.ui.props += 1;
}
