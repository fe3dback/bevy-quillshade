use super::*;

impl Editor {
    // ---- project ----

    pub fn open_project(&mut self, path: &Path) -> Result<(), String> {
        let project = Project::open(path).map_err(|e| e.to_string())?;
        if let Err(e) = project.ensure_shaders_dir() {
            return Err(format!("cannot create shaders/: {e}"));
        }
        self.save_tabs_to_config();
        let (registry, diags) = project_registry(&project);
        self.registry = registry;
        self.project_diags = diags;
        self.watcher = ShadersWatcher::new(&project).ok().map(std::sync::Mutex::new);
        self.config.touch_recent(&project.root);
        let tabs = self.config.tabs.get(&project.root).cloned().unwrap_or_default();
        self.project = Some(project);
        self.tabs.clear();
        self.active = None;
        self.diags.clear();
        self.regen_only.clear();
        self.wesl_lines.clear();
        self.current_folder.clear();
        self.rescan();
        self.refresh_game_files();
        self.nav.clear();
        for key in tabs.open.iter().filter_map(|k| parse_config_key(k)) {
            if self.key_exists(&key) {
                self.open_key(&key);
            }
        }
        self.active = tabs.active.filter(|i| *i < self.tabs.len()).or(self.active);
        self.follow_active();
        let root = self.project.as_ref().map(|p| p.root.display().to_string()).unwrap_or_default();
        self.info(format!("Opened project {root}"));
        self.save_config();
        self.touch_all();
        Ok(())
    }

    /// Whether what `key` names still exists: the asset on disk, the game file, the built-in texture.
    fn key_exists(&self, key: &TabKey) -> bool {
        match key {
            TabKey::Asset(id) | TabKey::Generated(id, _) => self.disk.contains(id),
            TabKey::GameImage(path) => self.game_files.contains(path),
            TabKey::Internal(id) => shader_stdlib::internal::internal_texture(id).is_some(),
        }
    }

    /// Opens (or activates) the tab `key` names; nothing may open (a load error is logged).
    fn open_key(&mut self, key: &TabKey) {
        match key {
            TabKey::Asset(id) => self.open_asset(id),
            TabKey::Generated(id, lang) => self.show_generated(id, *lang),
            TabKey::GameImage(path) => self.open_game_image(path),
            TabKey::Internal(id) => self.open_asset(&AssetId::new(AssetType::Texture, id)),
        }
    }

    /// Index of the open tab `key` names.
    fn key_index(&self, key: &TabKey) -> Option<usize> {
        self.tabs.iter().position(|t| t.doc().config_key().and_then(|k| parse_config_key(&k)).as_ref() == Some(key))
    }

    /// The active tab's identity ([`TabDoc::config_key`](crate::doc_kind::TabDoc::config_key) parsed back).
    pub fn active_key(&self) -> Option<TabKey> {
        parse_config_key(&self.tab()?.doc().config_key()?)
    }

    /// Back / Forward (decision 0041): walks the history to the next entry that opens, dropping entries whose
    /// target is gone and skipping (keeping) ones that fail to open, then restores the entry's graph view.
    /// An open tab is never gone, even if its file vanished; a load error is only logged (no dialog), and at an end
    /// of the list nothing happens. Ignored while a dialog is open.
    pub fn navigate(&mut self, dir: NavDir) {
        if self.dialog.is_some() || self.project.is_none() {
            return;
        }
        let mut nav = std::mem::take(&mut self.nav);
        let active = self.active_key();
        let reached = nav.step(dir, active.as_ref(), |entry| {
            if let Some(i) = self.key_index(&entry.tab) {
                self.activate(i);
                return Reach::Opened;
            }
            if !self.key_exists(&entry.tab) {
                return Reach::Gone;
            }
            // A load error is also shown in a dialog (`io_error`); the walk goes on, so keep only the console line.
            let had_dialog = self.dialog.is_some();
            self.open_key(&entry.tab);
            if !had_dialog {
                self.dialog = None;
            }
            if self.active_key().as_ref() == Some(&entry.tab) { Reach::Opened } else { Reach::Failed }
        });
        self.nav = nav;
        if let Some(NavEntry { view: Some(v), .. }) = reached {
            // Replaces the FitAll a reopened graph tab queued.
            self.view_requests.clear();
            self.view_requests.push(ViewRequest::Set { centre: v.centre, zoom: v.zoom });
        }
    }

    pub fn save_tabs_to_config(&mut self) {
        let Some(p) = &self.project else { return };
        // A tab without a key is not restored; the active index counts only the remembered ones.
        let keys: Vec<Option<String>> = self.tabs.iter().map(|t| t.doc().config_key()).collect();
        let active = self.active.filter(|a| keys.get(*a).is_some_and(Option::is_some));
        let active = active.map(|a| keys[..a].iter().flatten().count());
        let open = keys.into_iter().flatten().collect();
        self.config.tabs.insert(p.root.clone(), ProjectTabs { open, active });
    }

    pub fn save_config(&mut self) {
        self.save_tabs_to_config();
        if let Some(path) = &self.config_path
            && let Err(e) = self.config.save(path)
        {
            let msg = format!("cannot save user config: {e}");
            self.push_msg(Level::Warning, msg);
        }
    }

    /// Reloads the asset list and declared function signatures from disk.
    pub fn rescan(&mut self) {
        let Some(project) = &self.project else { return };
        self.disk = AssetSet::load_all(project);
        self.folders = assets::scan_dirs(project);
        self.refresh_functions();
        self.ui.browser += 1;
        self.validated = None;
    }

    /// Re-lists the game's images (Game Assets tree); validation reruns, as W109 depends on them.
    pub fn refresh_game_files(&mut self) {
        let Some(project) = &self.project else { return };
        let files = game_assets::scan_images(project);
        if files != self.game_files {
            self.game_files = files;
            self.ui.browser += 1;
            self.validated = None;
        }
    }

    pub(super) fn refresh_functions(&mut self) {
        let mut sigs = BTreeMap::new();
        for (id, a) in &self.disk.assets {
            if id.ty != AssetType::Function {
                continue;
            }
            let asset = self.docs().find(|d| d.id == *id).map(|d| &d.asset).or(a.as_ref().ok());
            if let Some(asset) = asset {
                sigs.insert(id.path.clone(), declared_function_sig(asset, &id.path, &self.registry));
            }
        }
        self.functions = sigs;
    }

    pub fn open_asset(&mut self, id: &AssetId) {
        if let Some(i) = self.asset_index(id) {
            self.activate(i);
            return;
        }
        if let Some(i) = self.texture_index(id) {
            self.activate(i);
            return;
        }
        if id.ty == AssetType::Texture
            && let Some(t) = shader_stdlib::internal::internal_texture(&id.path)
        {
            self.tabs.push(Tab::Texture(TextureDoc::internal(t)));
            self.activate(self.tabs.len() - 1);
            return;
        }
        let Some(project) = &self.project else { return };
        if id.ty == AssetType::Texture {
            match assets::load_texture(project, id) {
                Ok(t) => {
                    self.tabs.push(Tab::Texture(TextureDoc::new(id.clone(), t)));
                    self.activate(self.tabs.len() - 1);
                }
                Err(e) => self.io_error(&format!("cannot open {}", id.source_rel()), e),
            }
            return;
        }
        if id.ty == AssetType::Consts {
            match assets::load_consts(project, id) {
                Ok(c) => {
                    self.tabs.push(Tab::Consts(ConstsDoc::new(id.clone(), c)));
                    self.activate(self.tabs.len() - 1);
                }
                Err(e) => self.io_error(&format!("cannot open {}", id.source_rel()), e),
            }
            return;
        }
        if id.ty == AssetType::Gradients {
            match assets::load_gradients(project, id) {
                Ok(c) => {
                    self.tabs.push(Tab::Gradients(GradientsDoc::new(id.clone(), c)));
                    self.activate(self.tabs.len() - 1);
                }
                Err(e) => self.io_error(&format!("cannot open {}", id.source_rel()), e),
            }
            return;
        }
        if id.ty == AssetType::Curve {
            match assets::load_curve(project, id) {
                Ok(c) => {
                    self.tabs.push(Tab::Curve(CurveDoc::new(id.clone(), c)));
                    self.activate(self.tabs.len() - 1);
                }
                Err(e) => self.io_error(&format!("cannot open {}", id.source_rel()), e),
            }
            return;
        }
        match assets::load(project, id) {
            Ok(a) => {
                // The statusbar's WESL line count before this session regenerates the asset.
                if let Some(text) = wesl_file(project, id).and_then(|f| std::fs::read_to_string(f).ok()) {
                    self.wesl_lines.insert(id.clone(), text.lines().count());
                }
                self.tabs.push(Tab::Graph(Doc::new(id.clone(), a)));
                self.activate(self.tabs.len() - 1);
                self.view_requests.push(ViewRequest::FitAll);
            }
            Err(e) => self.io_error(&format!("cannot open {}", id.source_rel()), e),
        }
    }

    /// Opens (or activates) a read-only tab with a file generated from `id`.
    pub fn show_generated(&mut self, id: &AssetId, lang: Lang) {
        if let Some(i) = self.text_index(id, lang) {
            self.activate(i);
            return;
        }
        let Some(project) = &self.project else { return };
        match TextDoc::open(project, id.clone(), lang) {
            Some(t) if t.text.is_some() => {
                self.tabs.push(Tab::Text(t));
                self.activate(self.tabs.len() - 1);
            }
            Some(t) => {
                let msg = format!("{} does not exist yet (save the asset to generate it)", t.rel);
                self.push_msg(Level::Warning, msg);
            }
            None if !id.ty.has_wesl() => self.push_msg(Level::Warning, "constant tables generate no files".into()),
            None => self.push_msg(Level::Warning, "only materials have a Rust module".into()),
        }
    }

    /// Opens (or activates) the read-only tab of the game image `assets/<path>` (design §4.4).
    pub fn open_game_image(&mut self, path: &str) {
        if let Some(i) = self.image_index(path) {
            self.activate(i);
            return;
        }
        let Some(project) = &self.project else { return };
        if let Err(e) = TextureAsset::check_path(path) {
            self.push_msg(Level::Warning, format!("cannot open game image: {e}"));
            return;
        }
        if !project.assets_dir().join(path).is_file() {
            self.push_msg(Level::Warning, format!("assets/{path} does not exist"));
            return;
        }
        self.tabs.push(Tab::Image(ImageDoc::open(project, path)));
        self.activate(self.tabs.len() - 1);
    }

    /// Re-reads the files of read-only tabs (after regeneration; each one also on activation).
    pub(super) fn reload_read_only_tabs(&mut self) {
        let Some(project) = &self.project else { return };
        let (mut changed, mut active_changed) = (false, false);
        for (i, t) in self.tabs.iter_mut().map(Tab::doc_mut).enumerate().filter(|(_, t)| t.is_read_only()) {
            let revision = t.revision();
            // A missing file is shown as such; nothing to report here.
            let _ = t.reload_from_disk(project);
            changed |= t.revision() != revision;
            active_changed |= t.revision() != revision && self.active == Some(i);
        }
        if changed {
            self.ui.tabs += 1;
        }
        // The right sidebar of the active tab shows the file (a text tab's line count).
        if active_changed {
            self.ui.props += 1;
        }
        self.note_wesl_text();
    }

    /// Line counts of the generated WESL that text tabs show (the file as last read).
    fn note_wesl_text(&mut self) {
        for t in self.tabs.iter().filter_map(Tab::text).filter(|t| t.lang == Lang::Wesl) {
            if let Some(text) = &t.text {
                self.wesl_lines.insert(t.source.clone(), text.lines().count());
            }
        }
    }

    pub(super) fn activate(&mut self, i: usize) {
        if i < self.tabs.len() {
            if let Some(project) = &self.project
                && self.tabs[i].doc().is_read_only()
            {
                let _ = self.tabs[i].doc_mut().reload_from_disk(project);
                self.note_wesl_text();
            }
            self.active = Some(i);
            self.ui.tabs += 1;
            self.ui.props += 1;
            self.ui.log += 1;
        }
    }

    /// Closes a tab; asks first when it has unsaved changes.
    pub fn close_tab(&mut self, i: usize, force: bool) {
        // `usize::MAX` = the active tab (Ctrl+W).
        let i = if i == usize::MAX {
            match self.active {
                Some(a) => a,
                None => return,
            }
        } else {
            i
        };
        let Some(t) = self.tabs.get(i) else { return };
        if t.doc().is_dirty() && !force {
            let name = t.doc().asset().map_or_else(|| t.doc().base_title(), AssetId::source_rel);
            self.dialog = Some(Dialog::Confirm {
                title: "Unsaved changes".into(),
                message: format!("Save changes to {name} before closing?"),
                buttons: vec![
                    ("Cancel".into(), Action::CloseDialog),
                    ("Discard".into(), Action::DiscardTab(i)),
                    ("Save".into(), Action::SaveAndCloseTab(i)),
                ],
            });
            return;
        }
        let close: Vec<bool> = (0..self.tabs.len()).map(|j| j == i).collect();
        self.remove_tabs(&close);
    }

    /// Closes the tabs `which` names relative to tab `at` (tab context menu).
    pub fn close_tabs(&mut self, which: CloseTabs, at: usize, unsaved: Unsaved) {
        if at >= self.tabs.len() {
            return;
        }
        let close: Vec<bool> = (0..self.tabs.len())
            .map(|i| match which {
                CloseTabs::All => true,
                CloseTabs::Others => i != at,
                CloseTabs::Left => i < at,
                CloseTabs::Right => i > at,
                CloseTabs::Unmodified => !self.tabs[i].doc().is_dirty(),
            })
            .collect();
        let dirty: Vec<usize> = (0..self.tabs.len()).filter(|i| close[*i] && self.tabs[*i].doc().is_dirty()).collect();
        if !dirty.is_empty() {
            match unsaved {
                Unsaved::Ask => {
                    let action = |unsaved| Action::CloseTabs { which, at, unsaved };
                    self.dialog = Some(Dialog::Confirm {
                        title: "Unsaved changes".into(),
                        message: format!("{} of the tabs being closed have unsaved changes.", dirty.len()),
                        buttons: vec![
                            ("Cancel".into(), Action::CloseDialog),
                            ("Discard".into(), action(Unsaved::Discard)),
                            ("Save".into(), action(Unsaved::Save)),
                        ],
                    });
                    return;
                }
                Unsaved::Discard => {}
                Unsaved::Save => {
                    let mut saved = true;
                    for i in dirty {
                        saved &= self.save(i);
                    }
                    if !saved {
                        return;
                    }
                    self.regenerate();
                }
            }
        }
        self.remove_tabs(&close);
    }

    /// Removes the tabs marked in `close`. The active tab stays active; when it goes, its right neighbour that
    /// stays takes over (the last tab when there is none).
    pub(super) fn remove_tabs(&mut self, close: &[bool]) {
        if !close.contains(&true) {
            return;
        }
        let restore = self.memory_regenerated(|i| close[i]);
        let kept_before = |i: usize| (0..i).filter(|j| !close[*j]).count();
        let kept = close.iter().filter(|c| !**c).count();
        self.active = match self.active {
            _ if kept == 0 => None,
            Some(a) if !close[a] => Some(kept_before(a)),
            Some(a) => Some(kept_before(a).min(kept - 1)),
            None => None,
        };
        let mut i = 0;
        self.tabs.retain(|_| {
            i += 1;
            !close[i - 1]
        });
        self.refresh_functions();
        self.validated = None;
        for id in &restore {
            self.regenerate_from_disk(id);
        }
        self.touch_all();
        self.save_config();
    }

    /// Graph documents among the tabs `pick` selects whose generated files were written from unsaved edits
    /// (decision 0022): discarding them must regenerate those assets from disk.
    pub(super) fn memory_regenerated(&self, pick: impl Fn(usize) -> bool) -> Vec<AssetId> {
        (0..self.tabs.len())
            .filter(|i| pick(*i))
            .filter_map(|i| self.tabs[i].graph())
            .filter(|d| d.regen_from_memory && d.is_dirty())
            .map(|d| d.id.clone())
            .collect()
    }

    /// Closes the project (its open tabs are remembered for the next time it opens). Asks first when tabs have
    /// unsaved changes.
    pub fn close_project(&mut self, unsaved: Unsaved) {
        if self.project.is_none() {
            return;
        }
        if self.is_dirty() {
            match unsaved {
                Unsaved::Ask => {
                    self.dialog = Some(Dialog::Confirm {
                        title: "Unsaved changes".into(),
                        message: "Some assets have unsaved changes.".into(),
                        buttons: vec![
                            ("Cancel".into(), Action::CloseDialog),
                            ("Close without saving".into(), Action::CloseProject(Unsaved::Discard)),
                            ("Save all and close".into(), Action::CloseProject(Unsaved::Save)),
                        ],
                    });
                    return;
                }
                Unsaved::Discard => {
                    for id in self.memory_regenerated(|_| true) {
                        self.regenerate_from_disk(&id);
                    }
                }
                Unsaved::Save => {
                    if !self.save_all() {
                        return;
                    }
                    self.regenerate();
                }
            }
        }
        self.save_config();
        let root = self.project.take().map(|p| p.root.display().to_string()).unwrap_or_default();
        self.watcher = None;
        self.registry = shader_stdlib::registry();
        self.disk = AssetSet::default();
        self.folders.clear();
        self.game_files.clear();
        self.tabs.clear();
        self.active = None;
        self.nav.clear();
        self.functions.clear();
        self.diags.clear();
        self.project_diags.clear();
        self.regen_only.clear();
        self.validated = None;
        self.current_folder.clear();
        self.info(format!("Closed project {root}"));
        self.touch_all();
    }
}
