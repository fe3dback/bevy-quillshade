//! Material's side of the tab paths: the tabs of its asset kinds and generated files (game images are
//! `texture_kit`'s), its tab keys (`parse_tab` / `tab_exists`, design §6.6) and the function signatures. The core
//! halves are in `editor_core`.

use super::*;

/// Material's side of the tab paths: generated files, function signatures.
pub trait MaterialTabs {
    fn refresh_functions(&mut self);

    fn open_asset(&mut self, id: &AssetId);

    /// Opens (or activates) a read-only tab with a file generated from `id`.
    fn show_generated(&mut self, id: &AssetId, lang: &'static str);

    /// Graph documents among the tabs `pick` selects whose generated files were written from unsaved edits
    /// (decision 0022): discarding them must regenerate those assets from disk.
    fn memory_regenerated(&self, pick: impl Fn(usize) -> bool) -> Vec<AssetId>;
}

impl MaterialTabs for Editor {
    fn refresh_functions(&mut self) {
        let mut sigs = BTreeMap::new();
        for (id, a) in &self.domains.material().disk.assets {
            if id.ty != AssetType::Function {
                continue;
            }
            let asset = self.docs().find(|d| d.id == *id).map(|d| &d.asset).or(a.as_ref().ok());
            if let Some(asset) = asset {
                sigs.insert(id.path.clone(), declared_function_sig(asset, &id.path, &self.domains.material().registry));
            }
        }
        self.domains.material_mut().functions = sigs;
    }

    fn open_asset(&mut self, id: &AssetId) {
        self.open_ref(&id.to_ref());
    }

    fn show_generated(&mut self, id: &AssetId, lang: &'static str) {
        if let Some(i) = self.text_index(id, lang) {
            return self.activate(i);
        }
        if let Some(tab) = generated_tab(self, id, lang) {
            self.push_tab(tab);
        }
    }

    fn memory_regenerated(&self, pick: impl Fn(usize) -> bool) -> Vec<AssetId> {
        (0..self.tabs.len())
            .filter(|i| pick(*i))
            .filter_map(|i| self.tab_of::<Doc>(i))
            .filter(|d| d.regen_from_memory && d.is_dirty())
            .map(|d| d.id.clone())
            .collect()
    }
}

/// The tab of a Material asset (the asset kinds' `open`, design §6.1): its document loaded from disk, or the read-only
/// Texture tab of a built-in `@internal/…` texture (activated when already open). `None` with an error dialog when
/// the file does not load.
pub(crate) fn asset_tab(e: &mut Editor, r: &AssetRef) -> Option<Tab> {
    let id = AssetId::from_ref(r)?;
    let project = e.project.clone()?;
    let project = &project;
    let tab = match id.ty {
        // `texture_kit`'s and `curve_kit`'s asset kinds open through the kits' own `open` (their registration); these
        // arms only hand such a reference over to it.
        AssetType::Texture => return texture_kit::editor::open::<MaterialTexture>(e, r),
        AssetType::Consts => assets::load_consts(project, &id).map(|c| material_tab(ConstsDoc::new(id.clone(), c))),
        AssetType::Gradients => {
            assets::load_gradients(project, &id).map(|c| material_tab(GradientsDoc::new(id.clone(), c)))
        }
        AssetType::Curve => return curve_kit::editor::open::<crate::curve::MaterialCurve>(e, r),
        AssetType::Material | AssetType::Function => assets::load(project, &id).map(|a| {
            // The statusbar's WESL line count before this session regenerates the asset.
            if let Some(text) = wesl_file(project, &id).and_then(|f| std::fs::read_to_string(f).ok()) {
                e.domains.material_mut().wesl_lines.insert(id.clone(), text.lines().count());
            }
            e.view_requests.push(ViewRequest::FitAll);
            material_tab(Doc::new(id.clone(), a))
        }),
    };
    match tab {
        Ok(tab) => Some(tab),
        Err(err) => {
            e.io_error(&format!("cannot open {}", id.source_rel()), err);
            None
        }
    }
}

/// The read-only tab of a file generated from `id`; `None` (with a warning) while it does not exist.
fn generated_tab(e: &mut Editor, id: &AssetId, lang: &'static str) -> Option<Tab> {
    let project = e.project.as_ref()?;
    match crate::tab::generated_doc(project, id, lang) {
        Some(t) if t.text.is_some() => return Some(material_tab(t)),
        Some(t) => {
            let msg = format!("{} does not exist yet (save the asset to generate it)", t.rel);
            e.push_msg(Level::Warning, msg);
        }
        None if !id.ty.has_wesl() => e.push_msg(Level::Warning, "constant tables generate no files".into()),
        None => e.push_msg(Level::Warning, "only materials have a Rust module".into()),
    }
    None
}

/// Material's `parse_tab` (design §6.6): the tab of a generated file (`…ron#wesl` / `#rust`). The other
/// [`MaterialKey`]s open elsewhere, each exactly once: an asset's source path through core's default (its kind's
/// `open`, design §14.4), a game image's and a built-in texture's through `texture_kit`'s registration.
pub(crate) fn parse_tab(e: &mut Editor, key: &str) -> Option<Tab> {
    match parse_config_key(key)? {
        MaterialKey::Generated(id, lang) => generated_tab(e, &id, lang),
        MaterialKey::Asset(_) | MaterialKey::GameImage(_) | MaterialKey::Internal(_) => None,
    }
}

/// Material's `tab_exists`: the asset on disk, the game file, the built-in texture.
pub(crate) fn tab_exists(e: &Editor, key: &str) -> bool {
    match parse_config_key(key) {
        Some(MaterialKey::Asset(id) | MaterialKey::Generated(id, _)) => e.domains.material().disk.contains(&id),
        Some(MaterialKey::GameImage(path)) => e.game_files.contains(&path),
        Some(MaterialKey::Internal(id)) => MaterialTexture::POLICY.internal_texture(&id).is_some(),
        None => false,
    }
}
