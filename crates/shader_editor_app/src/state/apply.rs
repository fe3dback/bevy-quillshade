use super::*;

impl Editor {
    // ---- commands ----

    pub fn apply(&mut self, action: Action) {
        // A preview change (toggle, mode, undo, deleting the previewed node) regenerates the material from the
        // in-memory document (decision 0022).
        let before = self.doc().map(|d| (d.id.clone(), d.asset.preview.clone()));
        self.apply_action(action);
        if let Some((id, preview)) = before
            && id.ty == AssetType::Material
            && let Some(i) = self.doc_index(&id)
            && self.tabs[i].graph().is_some_and(|d| d.asset.preview != preview)
        {
            self.ui.tabs += 1;
            // The mode alone, with no node previewed before or after, generates nothing different.
            let previewed =
                preview.node.is_some() || self.tabs[i].graph().is_some_and(|d| d.asset.preview.node.is_some());
            if previewed {
                self.regenerate_from_memory(i);
            }
        }
        self.follow_active();
    }

    /// Records a change of the active tab, by whatever path it came, in the navigation history (decision 0041).
    pub(super) fn follow_active(&mut self) {
        if let Some(k) = self.active_key() {
            self.nav.visit(k);
        }
    }

    pub(super) fn apply_action(&mut self, action: Action) {
        match action {
            Action::None => {}
            Action::OpenProjectDialog => {
                let initial = self.project.as_ref().map(|p| p.root.display().to_string()).unwrap_or_default();
                self.dialog = Some(Dialog::Input {
                    title: "Open project".into(),
                    label: "Project directory (with Cargo.toml and assets/)".into(),
                    initial,
                    submit: Submit::OpenProject,
                });
                self.pick_folder = true;
            }
            Action::OpenProject(p) => {
                if self.is_dirty() {
                    self.save_all();
                }
                if let Err(e) = self.open_project(&p) {
                    self.io_error("Cannot open project", e);
                }
            }
            Action::NewProjectDialog => {
                self.dialog = Some(Dialog::Input {
                    title: "New project".into(),
                    label: "Game directory (a Cargo crate; assets/ and shaders/ are created in it)".into(),
                    initial: String::new(),
                    submit: Submit::NewProject,
                });
                self.pick_folder = true;
            }
            Action::BrowseFolder => self.pick_folder = true,
            Action::FolderPicked(p) => {
                let next = match &self.dialog {
                    Some(Dialog::Input { submit: Submit::OpenProject, .. }) => Action::OpenProject(p),
                    Some(Dialog::Input { submit: Submit::NewProject, .. }) => Action::NewProject(p),
                    _ => return,
                };
                self.apply(Action::CloseDialog);
                self.apply(next);
            }
            Action::NewProject(p) => match Project::create(&p) {
                Ok(_) => self.apply(Action::OpenProject(p)),
                Err(e) => self.io_error("Cannot create project", e),
            },
            Action::About => self.dialog = Some(Dialog::About),
            Action::QuickStart(step) => self.dialog = Some(Dialog::QuickStart(step)),
            Action::PreviewChannel(c) => {
                if let Some(p) = self.preview_doc_mut() {
                    p.set_channel(c.filter(|c| *c < 3));
                }
            }
            Action::OpenUrl(url) => self.external.push(PathBuf::from(url)),
            Action::OpenAsset(id) => self.open_asset(&id),
            Action::OpenGameImage(path) => self.open_game_image(&path),
            Action::ActivateTab(i) => self.activate(i),
            Action::CloseTab(i) => self.close_tab(i, false),
            Action::CloseTabs { which, at, unsaved } => {
                self.dialog = None;
                self.close_tabs(which, at, unsaved);
            }
            Action::CloseProject(unsaved) => {
                self.dialog = None;
                self.close_project(unsaved);
            }
            Action::DiscardTab(i) => {
                self.dialog = None;
                self.close_tab(i, true);
            }
            Action::SaveAndCloseTab(i) => {
                self.dialog = None;
                if self.save(i) {
                    self.regenerate();
                    self.close_tab(i, true);
                }
            }
            Action::NextTab => {
                if !self.tabs.is_empty() {
                    let n = self.active.map_or(0, |a| (a + 1) % self.tabs.len());
                    self.activate(n);
                }
            }
            // The layout follows the flags themselves (`ui::log`, `ui::areas`, `ui::show_left_sidebar`), no revision to
            // bump. In Zen mode a panel's flag still flips; the panel shows once Zen is off (design §3).
            Action::ToggleLeftSidebar => {
                self.config.show_left_sidebar = !self.config.show_left_sidebar;
                self.save_config();
            }
            Action::ToggleLog => {
                self.config.show_log = !self.config.show_log;
                self.save_config();
            }
            Action::ToggleProperties => {
                self.config.show_properties = !self.config.show_properties;
                self.save_config();
            }
            Action::ToggleZen => {
                self.config.zen_mode = !self.config.zen_mode;
                self.save_config();
            }
            Action::NavBack => self.navigate(NavDir::Back),
            Action::NavForward => self.navigate(NavDir::Forward),
            Action::Save => {
                if let Some(i) = self.active
                    && self.save(i)
                {
                    self.regenerate();
                }
            }
            Action::SaveAll => {
                if self.save_all() {
                    self.regenerate();
                }
            }
            Action::RegenAll => {
                self.regenerate();
            }
            Action::Undo => self.history(true),
            Action::Redo => self.history(false),
            Action::Copy => self.copy(),
            Action::Cut => {
                self.copy();
                // The clipboard holds no frames: a cut keeps the enclosed ones (only Delete removes them, design §4.5).
                if let Some(d) = self.doc_mut() {
                    d.enclosed_frames.clear();
                }
                self.delete_selection();
            }
            Action::Paste(at) => self.paste(at),
            Action::Duplicate if self.gradients_doc().is_some() => {
                self.gradients_action(GradientsAction::DuplicateSlot);
            }
            Action::Duplicate if self.curve_doc().is_some() => {
                if let Some(c) = self.curve_doc_mut() {
                    let copies: Vec<(f32, f32)> =
                        c.selected_points().into_iter().map(|(t, v)| (t + CURVE_PASTE_STEP, v)).collect();
                    if !copies.is_empty() {
                        c.insert_points(&copies);
                    }
                }
                self.ui.props += 1;
            }
            Action::Duplicate => {
                let saved = self.clipboard.take();
                self.copy();
                self.paste(None);
                self.clipboard = saved;
            }
            Action::DeleteSelection => self.delete_selection(),
            Action::SelectAll => {
                if let Some(c) = self.curve_doc_mut() {
                    c.select((0..c.curve.points.len()).collect());
                } else if let Some(d) = self.doc() {
                    let all = d.asset.nodes.keys().cloned().collect();
                    self.apply(Action::Select(all));
                    if let Some(d) = self.doc_mut() {
                        d.revision += 1;
                    }
                }
                self.ui.props += 1;
            }
            Action::Select(sel) => {
                if let Some(d) = self.doc_mut()
                    && (d.selection != sel || d.selected_frame.is_some() || !d.enclosed_frames.is_empty())
                {
                    d.set_selection(sel);
                    d.selected_frame = None;
                    self.ui.props += 1;
                }
            }
            Action::SelectFrame(f) => {
                if let Some(d) = self.doc_mut()
                    && d.selected_frame != f
                {
                    d.selected_frame = f;
                    d.enclosed_frames.clear();
                    self.ui.props += 1;
                }
            }
            // The sidebar keeps showing the node selection (design §4.5): no props refresh.
            Action::SelectFrames(frames) => {
                if let Some(d) = self.doc_mut() {
                    d.enclosed_frames = d.enclosed(&frames, &d.selection);
                }
            }
            Action::AddNode { kind, pos, link, params } => {
                self.add_node(&kind, pos, link, &params);
            }
            Action::AddNodeAndRename { kind, pos, link, params } => {
                if let Some(target) = self.add_node(&kind, pos, link, &params).and_then(|id| self.rename_target(&id)) {
                    self.apply_action(Action::TextDialog(target));
                }
            }
            Action::Drop { item, pos, node } => self.drop_item(item, pos, node),
            Action::Connect { from, to } => self.connect(from, to),
            Action::Disconnect { to } => {
                if let Some(d) = self.doc_mut() {
                    d.edit(|a| a.disconnect_input((&to.0, &to.1)));
                }
            }
            Action::DisconnectPin { node, pin, output } => {
                let at_pin = |e: &shader_graph_core::model::Edge| {
                    let end = if output { &e.from } else { &e.to };
                    end.0 == node && end.1 == pin
                };
                if let Some(d) = self.doc_mut() {
                    d.edit(|a| a.edges.retain(|e| !at_pin(e)));
                }
            }
            Action::MoveWires { node, pin, output, to } => self.move_wires((node, pin), output, to),
            Action::Moved { nodes, frames } => {
                if let Some(d) = self.doc_mut() {
                    d.edit(|a| {
                        for (id, p) in &nodes {
                            if let Some(n) = a.nodes.get_mut(id) {
                                n.pos = *p;
                            }
                        }
                        for (id, p) in &frames {
                            if let Some(f) = a.comments.iter_mut().find(|f| f.id == *id) {
                                f.rect.0 = p.0;
                                f.rect.1 = p.1;
                            }
                        }
                    });
                }
            }
            Action::SetValue { target, value, merge } => self.set_value(target, value, merge),
            Action::SetComponent { target, comp, value, is_final } => self.set_component(target, comp, value, is_final),
            Action::CommitInline => {
                if let Some(d) = self.doc_mut() {
                    d.end_merge();
                }
                if let Some(c) = self.curve_doc_mut() {
                    c.end_merge();
                }
                if let Some(c) = self.consts_doc_mut() {
                    c.end_merge();
                }
                if let Some(t) = self.texture_doc_mut() {
                    t.end_merge();
                }
                if let Some(g) = self.gradients_doc_mut() {
                    g.end_merge();
                }
                self.ui.props += 1;
            }
            Action::Curve(a) => self.curve_action(a),
            Action::Gradients(a) => self.gradients_action(a),
            Action::Consts(a) => self.consts_action(a),
            Action::Texture(a) => self.texture_action(a),
            Action::SetPinType { node, ty } => self.set_pin_type(&node, ty),
            Action::ResetInput { node, pin } => {
                if let Some(d) = self.doc_mut() {
                    d.edit(|a| a.nodes.get_mut(&node).map(|n| n.inputs.remove(&pin)));
                }
                self.ui.props += 1;
            }
            Action::ConvertNodes { nodes, conv } => self.convert_nodes(&nodes, &conv),
            Action::TogglePreview(node) => self.toggle_preview(node),
            Action::AutoLayout => self.auto_layout(),
            // Around the selection; with none, a default-size frame at `at` on the grid (the graph editor resolves
            // `None` to the visible canvas centre; headless it is the origin).
            Action::AddComment(at) => {
                if let Some(d) = self.doc_mut() {
                    let sel = d.selection.clone();
                    d.add_frame("Comment", &sel, crate::doc::snap(at.unwrap_or((0.0, 0.0))));
                }
            }
            Action::AddReroute { from, to, pos } => {
                if let Some(d) = self.doc_mut() {
                    let r = shader_graph_core::model::new_node_id();
                    d.edit(|a| {
                        a.nodes
                            .insert(r.clone(), shader_graph_core::Node::new("stdlib::Reroute", crate::doc::snap(pos)));
                        a.connect((&from.0, &from.1), (&r, "in"));
                        a.connect((&r, "out"), (&to.0, &to.1));
                    });
                }
            }
            Action::View(v) => self.view_requests.push(v),
            Action::NewAssetDialog(kind) => {
                let dir = self.current_folder.clone();
                let base = kind.base_name();
                self.dialog = Some(Dialog::Input {
                    title: format!("New {}", kind.label()),
                    label: "Path inside shaders/ (snake_case, `/` for folders)".into(),
                    initial: if dir.is_empty() { base.into() } else { format!("{dir}/{base}") },
                    submit: Submit::Create(kind),
                });
            }
            Action::NewFolderDialog => {
                let dir = self.current_folder.clone();
                self.dialog = Some(Dialog::Input {
                    title: "New folder".into(),
                    label: "Folder path inside shaders/".into(),
                    initial: if dir.is_empty() { "new_folder".into() } else { format!("{dir}/new_folder") },
                    submit: Submit::Folder,
                });
            }
            Action::TextDialog(target) => {
                let (title, label) = match target {
                    PropTarget::FrameTitle(_) => ("Comment", "Title"),
                    _ => ("Rename node", "Name"),
                };
                let initial = self.current_value(&target).map(|v| v.as_str().to_string()).unwrap_or_default();
                self.dialog = Some(Dialog::Input {
                    title: title.into(),
                    label: label.into(),
                    initial,
                    submit: Submit::SetText(target),
                });
            }
            Action::RenameDialog(id) => {
                self.dialog = Some(Dialog::Input {
                    title: format!("Rename {}", id.source_rel()),
                    label: "New path inside shaders/".into(),
                    initial: id.path.clone(),
                    submit: Submit::Rename(id),
                });
            }
            Action::MoveDialog(id) => {
                let dir = id.path.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default();
                self.dialog = Some(Dialog::Input {
                    title: format!("Move {}", id.source_rel()),
                    label: "Target folder inside shaders/ (empty = root)".into(),
                    initial: dir,
                    submit: Submit::MoveTo(id),
                });
            }
            Action::DeleteDialog(id) => {
                let message = if id.ty.has_wesl() {
                    format!("Delete {} and its generated files?", id.source_rel())
                } else {
                    let names = self.owned_consts(&id, self.global_consts().into_keys());
                    match self.const_usages(&names) {
                        u if u.is_empty() => format!("Delete {}?", id.source_rel()),
                        u => usage_message(&format!("Deleting {}", id.source_rel()), &names, &u),
                    }
                };
                self.dialog = Some(Dialog::Confirm {
                    title: "Delete asset".into(),
                    message,
                    buttons: vec![("Cancel".into(), Action::CloseDialog), ("Delete".into(), Action::Delete(id))],
                });
            }
            Action::DeleteFolderDialog(path) => {
                self.dialog = Some(Dialog::Confirm {
                    title: "Delete folder".into(),
                    message: format!("Delete empty folder {path}?"),
                    buttons: vec![
                        ("Cancel".into(), Action::CloseDialog),
                        ("Delete".into(), Action::DeleteFolder(path)),
                    ],
                });
            }
            Action::Submit(text) => self.submit(text.trim()),
            Action::MoveTo(id, dir) => {
                let name = id.name().to_string();
                let new_path = if dir.is_empty() { name } else { format!("{dir}/{name}") };
                self.rename(&id, &new_path);
            }
            Action::Delete(id) => self.delete(&id),
            Action::DeleteFolder(path) => {
                self.dialog = None;
                if let Some(p) = &self.project {
                    match ops::delete_folder(p, &path) {
                        Ok(()) => self.info(format!("Deleted folder {path}")),
                        Err(e) => self.io_error("Cannot delete folder (is it empty?)", e),
                    }
                }
                self.rescan();
            }
            Action::ShowFile(id, which) => self.show_file(&id, which),
            Action::ShowGenerated(id, lang) => self.show_generated(&id, lang),
            Action::ToggleFolder(tree, f) => {
                let collapsed = self.collapsed.entry(tree).or_default();
                if !collapsed.remove(&f) {
                    collapsed.insert(f);
                }
                self.ui.browser += 1;
            }
            Action::SelectFolder(f) => {
                self.current_folder = f;
                self.ui.browser += 1;
            }
            Action::CreateTextureForFile(path) => {
                self.create_texture_for_file(&path);
            }
            Action::FocusDiagnostic(diag) => self.focus_diagnostic(&diag),
            Action::Reload(id) => {
                self.dialog = None;
                if let Some(i) = self.asset_index(&id) {
                    // Unsaved edits written to the generated files are discarded too (decision 0022).
                    let restore = self.memory_regenerated(|j| j == i);
                    if self.reload_tab(i) {
                        self.info(format!("Reloaded {}", id.source_rel()));
                        for id in &restore {
                            self.regenerate_from_disk(id);
                        }
                    }
                }
                self.touch_all();
            }
            Action::KeepMine(id) => {
                self.dialog = None;
                // The disk version becomes the "saved" baseline, so the document stays dirty.
                if let (Some(i), Some(project)) = (self.asset_index(&id), &self.project) {
                    self.tabs[i].doc_mut().keep_mine(project);
                }
                self.ui.tabs += 1;
            }
            Action::CloseDialog => {
                self.dialog = None;
                self.pick_folder = false;
            }
            Action::Notify(text) => self.push_msg(Level::Warning, text),
            Action::ClearMessages => {
                self.messages.clear();
                self.ui.log += 1;
            }
            Action::Quit => {
                if self.is_dirty() {
                    self.dialog = Some(Dialog::Confirm {
                        title: "Unsaved changes".into(),
                        message: "Some assets have unsaved changes.".into(),
                        buttons: vec![
                            ("Cancel".into(), Action::CloseDialog),
                            ("Quit without saving".into(), Action::ForceQuit),
                            ("Save all and quit".into(), Action::SaveAllAndQuit),
                        ],
                    });
                } else {
                    self.save_config();
                    self.quit = true;
                }
            }
            Action::ForceQuit => {
                for id in self.memory_regenerated(|_| true) {
                    self.regenerate_from_disk(&id);
                }
                self.save_config();
                self.quit = true;
            }
            Action::SaveAllAndQuit => {
                self.dialog = None;
                if self.save_all() {
                    self.regenerate();
                    self.save_config();
                    self.quit = true;
                }
            }
        }
    }

    pub(super) fn history(&mut self, undo: bool) {
        if self.consts_doc().is_some() {
            return self.consts_action(ConstsAction::History { undo, confirmed: false });
        }
        if self.texture_doc().is_some() {
            return self.texture_history(undo);
        }
        let done = match self.active.and_then(|i| self.tabs.get_mut(i)) {
            Some(Tab::Graph(d)) => {
                if undo {
                    d.undo()
                } else {
                    d.redo()
                }
            }
            Some(Tab::Curve(c)) => {
                if undo {
                    c.undo()
                } else {
                    c.redo()
                }
            }
            Some(Tab::Gradients(g)) => {
                if undo {
                    g.undo()
                } else {
                    g.redo()
                }
            }
            _ => false,
        };
        if done {
            self.ui.props += 1;
        }
    }
}
